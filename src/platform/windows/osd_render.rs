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
    SetBkMode, SetTextAlign, SetTextColor, TA_LEFT, TA_RIGHT, TRANSPARENT, TextOutW,
};
use windows::core::w;

use super::osd::{OsdMetrics, OsdRenderState};

/// Error message displayed when DDC communication fails.
const ERROR_MESSAGE: &str = "DDC Error - Adjustment failed";

/// Hardware brightness bar fill color (golden/orange).
const BAR_FILL_COLOR: u32 = 0x00D0_A030; // BGR: 48, 160, 208
/// Overlay dimming bar fill color (purple/violet).
const OVERLAY_FILL_COLOR: u32 = 0x00CC_6699; // BGR: 153, 102, 204
/// Progress bar background color (dark gray).
const BAR_BACKGROUND_COLOR: u32 = 0x0050_5050; // BGR: 80, 80, 80
/// Text color (white).
const TEXT_COLOR: u32 = 0x00FF_FFFF; // BGR: 255, 255, 255
/// Error bar color (red).
const BAR_ERROR_COLOR: u32 = 0x0000_00CC; // BGR: 204, 0, 0
/// Error text color (light red for visibility on dark background).
const ERROR_TEXT_COLOR: u32 = 0x0050_50FF; // BGR: 255, 80, 80

/// Icon for hardware brightness (sun symbol).
const ICON_HARDWARE: &str = "🔆";
/// Icon for overlay/dimming (sunglasses symbol).
const ICON_OVERLAY: &str = "🕶";

/// Fills `rect` with a solid `color` (BGR). The brush never escapes this call,
/// so no RAII wrapper is needed.
fn fill_rect(hdc: HDC, rect: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(COLORREF(color));
        FillRect(hdc, &raw const *rect, brush);
        let _ = DeleteObject(brush);
    }
}

/// Selects which font face the OSD draws with.
#[derive(Clone, Copy)]
enum FontFace {
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
struct SelectedFont {
    hdc: HDC,
    font: HFONT,
    old: HGDIOBJ,
}

impl SelectedFont {
    /// Creates a font of the given size/face and selects it into `hdc`.
    fn new(hdc: HDC, font_size: i32, face: FontFace) -> Self {
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
struct BackBuffer {
    mem_dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
}

impl BackBuffer {
    /// Allocates a memory DC + compatible bitmap sized `width` x `height`,
    /// compatible with `target`. Returns `None` if any GDI allocation fails.
    fn new(target: HDC, width: i32, height: i32) -> Option<Self> {
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
    fn dc(&self) -> HDC {
        self.mem_dc
    }

    /// Copies the back buffer onto `target` at the origin.
    fn blit_to(&self, target: HDC) {
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

/// Paints the OSD content into `hdc` using double-buffering.
///
/// `client_rect` is the window's client rectangle; `state` and `metrics` are
/// snapshots taken by the caller — this function reads no thread-local state.
///
/// # Safety
///
/// `hdc` must be a valid device context (e.g. from `BeginPaint`).
pub(super) unsafe fn paint(
    hdc: HDC,
    client_rect: &RECT,
    state: &OsdRenderState,
    metrics: &OsdMetrics,
) {
    let width = client_rect.right - client_rect.left;
    let height = client_rect.bottom - client_rect.top;

    // Double-buffer to avoid flicker; the guard cleans up on drop.
    let Some(buffer) = BackBuffer::new(hdc, width, height) else {
        return;
    };
    let mem_dc = buffer.dc();

    // Fill background
    fill_rect(mem_dc, client_rect, super::osd::OSD_BACKGROUND_COLOR);

    // Draw progress bar(s)
    draw_brightness_bars(mem_dc, client_rect, state, metrics);

    // Draw error message if in error state
    if state.is_error {
        draw_error_message(mem_dc, client_rect, ERROR_MESSAGE, metrics);
    }

    // Copy to screen
    buffer.blit_to(hdc);
}

/// Draws the bidirectional brightness bar.
///
/// Layout: `|pad|pct 🕶 ░░░░██████|gap|██████░░░░ 🔆 pct|pad|`
///
/// - Left half: overlay dimming (fills right-to-left)
/// - Right half: hardware brightness (fills left-to-right)
/// - Small gap separates the two halves
fn draw_brightness_bars(
    hdc: HDC,
    client_rect: &RECT,
    state: &OsdRenderState,
    metrics: &OsdMetrics,
) {
    log::trace!(
        hardware_brightness = state.hardware_brightness,
        overlay_opacity = state.overlay_opacity,
        is_error = state.is_error;
        "Drawing bidirectional brightness bar"
    );

    // Use fixed bar position based on compact height (OSD_HEIGHT).
    // This keeps the bar in the same position whether or not the error row is visible.
    // When expanded for errors, the extra space is at the bottom for the error message.
    let bar_top = (metrics.height - metrics.bar_height) / 2;

    // Draw left side (overlay section)
    draw_overlay_section(hdc, client_rect, bar_top, state.overlay_opacity, metrics);

    // Draw right side (hardware section)
    draw_hardware_section(
        hdc,
        client_rect,
        bar_top,
        state.hardware_brightness,
        state.is_error,
        metrics,
    );
}

/// Draws the hardware brightness section (right half of the bidirectional bar).
///
/// Layout: `|...|gap|████████░░░░░░░░░░ 🔆 100%|pad|`
///
/// The bar fills from left to right based on hardware brightness percentage.
fn draw_hardware_section(
    hdc: HDC,
    client_rect: &RECT,
    bar_top: i32,
    hardware_brightness: u8,
    is_error: bool,
    metrics: &OsdMetrics,
) {
    let width = client_rect.right - client_rect.left;

    // Calculate layout positions
    let padding = metrics.padding;
    let percent_text_width = metrics.percent_text_width;
    let icon_width = metrics.icon_width;
    let bar_gap = metrics.bar_gap;
    let bar_height = metrics.bar_height;

    // Full layout: |pad|pct|ico|left_bar|gap|right_bar|ico|pct|pad|
    let content_width = width - (padding * 2);
    let total_bar_width = content_width - (percent_text_width * 2) - (icon_width * 2) - bar_gap;
    let single_bar_width = total_bar_width / 2;

    // Right bar starts after: pad + pct + ico + left_bar + gap
    let bar_left = padding + percent_text_width + icon_width + single_bar_width + bar_gap;

    // Draw the bar (fills left-to-right)
    draw_single_bar(
        hdc,
        bar_left,
        bar_top,
        single_bar_width,
        bar_height,
        hardware_brightness,
        is_error,
    );

    // Icon position: right of bar with small padding
    let icon_x = bar_left + single_bar_width + 2;
    draw_icon(hdc, icon_x, bar_top, ICON_HARDWARE, metrics);

    // Percentage text position: right of icon (left-aligned)
    let percent_x = icon_x + icon_width;
    draw_percentage_text(hdc, percent_x, bar_top, hardware_brightness, false, metrics);
}

/// Draws the overlay dimming section (left half of the bidirectional bar).
///
/// Layout: `|pad|30% 🕶 ░░░░░░░░░░░░░░██████|gap|...|`
///
/// The bar fills from right to left based on overlay opacity percentage.
fn draw_overlay_section(
    hdc: HDC,
    client_rect: &RECT,
    bar_top: i32,
    overlay_opacity: u8,
    metrics: &OsdMetrics,
) {
    let width = client_rect.right - client_rect.left;

    // Calculate layout positions
    let padding = metrics.padding;
    let percent_text_width = metrics.percent_text_width;
    let icon_width = metrics.icon_width;
    let bar_gap = metrics.bar_gap;
    let bar_height = metrics.bar_height;

    // Full layout: |pad|pct|ico|left_bar|gap|right_bar|ico|pct|pad|
    let content_width = width - (padding * 2);
    let total_bar_width = content_width - (percent_text_width * 2) - (icon_width * 2) - bar_gap;
    let single_bar_width = total_bar_width / 2;

    // Left bar starts after: pad + pct + ico
    let bar_left = padding + percent_text_width + icon_width;

    // Draw the bar background
    let bg_rect = RECT {
        left: bar_left,
        top: bar_top,
        right: bar_left + single_bar_width,
        bottom: bar_top + bar_height,
    };
    fill_rect(hdc, &bg_rect, BAR_BACKGROUND_COLOR);

    // Draw filled portion (fills from right to left)
    let fill_width =
        i32::try_from(i64::from(single_bar_width) * i64::from(overlay_opacity) / 100).unwrap_or(0);
    if fill_width > 0 {
        let filled = RECT {
            left: bar_left + single_bar_width - fill_width, // Start from right side
            top: bar_top,
            right: bar_left + single_bar_width,
            bottom: bar_top + bar_height,
        };
        fill_rect(hdc, &filled, OVERLAY_FILL_COLOR);
    }

    // Percentage text position: right-aligned so "%" stays at fixed position
    let percent_right_x = padding + percent_text_width;
    draw_percentage_text(
        hdc,
        percent_right_x,
        bar_top,
        overlay_opacity,
        true,
        metrics,
    );

    // Icon position: left of bar with small padding
    let icon_x = padding + percent_text_width;
    draw_icon(hdc, icon_x, bar_top, ICON_OVERLAY, metrics);
}

/// Draws an icon (emoji) at the specified position.
fn draw_icon(hdc: HDC, x: i32, y: i32, icon: &str, metrics: &OsdMetrics) {
    let font_size = metrics.font_size;
    let bar_height = metrics.bar_height;

    // Select an emoji font; the guard restores + deletes on drop.
    let _font = SelectedFont::new(hdc, font_size, FontFace::Emoji);

    // Set text properties
    unsafe {
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(TEXT_COLOR));
    }

    // Draw icon
    let wide_text: Vec<u16> = icon.encode_utf16().collect();
    let text_y = y + (bar_height - font_size) / 2;
    unsafe {
        TextOutW(hdc, x, text_y, &wide_text);
    }
}

/// Draws a single progress bar.
fn draw_single_bar(hdc: HDC, x: i32, y: i32, width: i32, height: i32, percent: u8, is_error: bool) {
    // Draw background
    let bg_rect = RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    };
    fill_rect(hdc, &bg_rect, BAR_BACKGROUND_COLOR);

    // Draw filled portion
    let fill_width = i32::try_from(i64::from(width) * i64::from(percent) / 100).unwrap_or(0);
    if fill_width > 0 {
        let filled = RECT {
            left: x,
            top: y,
            right: x + fill_width,
            bottom: y + height,
        };
        let fill_color = if is_error {
            BAR_ERROR_COLOR
        } else {
            BAR_FILL_COLOR
        };
        fill_rect(hdc, &filled, fill_color);
    }
}

/// Draws the percentage text next to a bar.
///
/// # Arguments
///
/// * `hdc` - Device context to draw on.
/// * `x` - X coordinate (left edge if `right_align` is false, right edge if true).
/// * `y` - Y coordinate (top of bar area, text is centered vertically).
/// * `percent` - Percentage value to display.
/// * `right_align` - If true, text is right-aligned (x is the right edge).
fn draw_percentage_text(
    hdc: HDC,
    x: i32,
    y: i32,
    percent: u8,
    right_align: bool,
    metrics: &OsdMetrics,
) {
    let font_size = metrics.font_size;
    let bar_height = metrics.bar_height;

    // Select the text font; the guard restores + deletes on drop.
    let _font = SelectedFont::new(hdc, font_size, FontFace::Text);

    unsafe {
        // Set text properties
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(TEXT_COLOR));

        // Set text alignment
        let align = if right_align { TA_RIGHT } else { TA_LEFT };
        SetTextAlign(hdc, align);
    }

    // Format and draw text
    let text = format!("{percent}%");
    let wide_text: Vec<u16> = text.encode_utf16().collect();

    // Center text vertically with the bar
    let text_y = y + (bar_height - font_size).saturating_div(2);
    unsafe {
        TextOutW(hdc, x, text_y, &wide_text);

        // Restore default alignment (draw state, not a resource).
        SetTextAlign(hdc, TA_LEFT);
    }
}

/// Draws the error message centered in the error row at the bottom of the OSD.
///
/// The error row occupies the bottom `error_row_height` pixels of the expanded window.
/// This function should only be called when the window is in expanded (error) mode.
fn draw_error_message(hdc: HDC, client_rect: &RECT, message: &str, metrics: &OsdMetrics) {
    let font_size = metrics.font_size;
    let error_row_height = metrics.error_row_height;

    // Select the text font; the guard restores + deletes on drop.
    let _font = SelectedFont::new(hdc, font_size, FontFace::Text);

    // Convert message to wide string
    let wide_text: Vec<u16> = message.encode_utf16().collect();

    // Calculate position - centered horizontally, in footer area at bottom
    let width = client_rect.right - client_rect.left;
    // Approximate text width (~7 pixels per character at this font size)
    // Scaling approximation: original was 7px for 18pt font. Ratio ~0.38
    #[allow(clippy::cast_possible_truncation)]
    let approx_char_width = (f64::from(font_size) * 0.38).round() as i32;
    let approx_text_width = i32::try_from(wide_text.len()).unwrap_or(0) * approx_char_width;

    let x = (width - approx_text_width) / 2;
    // Position in error row area: bottom of window minus row height, centered vertically
    let y = client_rect.bottom - error_row_height + (error_row_height - font_size) / 2;

    unsafe {
        // Set text properties - use red color for error visibility
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(ERROR_TEXT_COLOR));
        TextOutW(hdc, x, y, &wide_text);
    }
}
