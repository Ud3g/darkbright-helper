//! GDI drawing primitives for the OSD window.
//!
//! This module owns the pure rendering layer: it draws into a device context
//! it is handed and knows nothing about window lifecycle or thread-local state.
//! GDI resource cleanup is handled by RAII guards (`SelectedFont`, `BackBuffer`)
//! and the `fill_rect` helper, so callers never balance `DeleteObject` by hand.

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
    CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH, DeleteDC, DeleteObject, FF_DONTCARE,
    FW_NORMAL, FillRect, HBITMAP, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, SRCCOPY, SelectObject,
};
use windows::core::w;

/// Fills `rect` with a solid `color` (BGR). The brush never escapes this call,
/// so no RAII wrapper is needed.
pub(super) fn fill_rect(hdc: HDC, rect: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(COLORREF(color));
        FillRect(hdc, &raw const *rect, brush);
        let _ = DeleteObject(brush);
    }
}

/// Selects which font face the OSD draws with.
#[derive(Clone, Copy)]
pub(super) enum FontFace {
    /// Regular UI text ("Segoe UI").
    Text,
    /// Emoji glyphs ("Segoe UI Emoji").
    Emoji,
}

/// Creates an OSD font of the given pixel height and face.
fn create_font(font_size: i32, face: FontFace) -> HFONT {
    let face_name = match face {
        FontFace::Text => w!("Segoe UI"),
        FontFace::Emoji => w!("Segoe UI Emoji"),
    };
    unsafe {
        CreateFontW(
            font_size,
            0,
            0,
            0,
            FW_NORMAL.0.cast_signed(),
            0,
            0,
            0,
            u32::from(DEFAULT_CHARSET.0),
            u32::from(OUT_DEFAULT_PRECIS.0),
            u32::from(CLIP_DEFAULT_PRECIS.0),
            0,
            u32::from(DEFAULT_PITCH.0 | FF_DONTCARE.0),
            face_name,
        )
    }
}

/// RAII guard that selects a font into a device context and, on drop, restores
/// the previously selected object and deletes the font.
pub(super) struct SelectedFont {
    hdc: HDC,
    font: HFONT,
    old: HGDIOBJ,
}

impl SelectedFont {
    /// Creates a font of the given size/face and selects it into `hdc`.
    pub(super) fn new(hdc: HDC, font_size: i32, face: FontFace) -> Self {
        let font = create_font(font_size, face);
        let old = unsafe { SelectObject(hdc, font) };
        Self { hdc, font, old }
    }
}

impl Drop for SelectedFont {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.hdc, self.old);
            let _ = DeleteObject(self.font);
        }
    }
}

/// RAII double-buffer: a memory DC with a compatible bitmap selected into it.
/// On drop it restores the old bitmap, deletes the bitmap, then deletes the DC.
pub(super) struct BackBuffer {
    mem_dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
}

impl BackBuffer {
    /// Allocates a memory DC + compatible bitmap sized `width` x `height`,
    /// compatible with `target`. Returns `None` if any GDI allocation fails.
    pub(super) fn new(target: HDC, width: i32, height: i32) -> Option<Self> {
        unsafe {
            let mem_dc = CreateCompatibleDC(target);
            if mem_dc.0 == 0 {
                return None;
            }
            let bitmap = CreateCompatibleBitmap(target, width, height);
            if bitmap.0 == 0 {
                let _ = DeleteDC(mem_dc);
                return None;
            }
            let old_bitmap = SelectObject(mem_dc, bitmap);
            Some(Self {
                mem_dc,
                bitmap,
                old_bitmap,
                width,
                height,
            })
        }
    }

    /// The memory device context to draw into.
    pub(super) fn dc(&self) -> HDC {
        self.mem_dc
    }

    /// Copies the back buffer onto `target` at the origin.
    pub(super) fn blit_to(&self, target: HDC) {
        unsafe {
            let _ = BitBlt(
                target,
                0,
                0,
                self.width,
                self.height,
                self.mem_dc,
                0,
                0,
                SRCCOPY,
            );
        }
    }
}

impl Drop for BackBuffer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem_dc, self.old_bitmap);
            let _ = DeleteObject(self.bitmap);
            let _ = DeleteDC(self.mem_dc);
        }
    }
}
