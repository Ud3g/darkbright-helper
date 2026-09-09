//! Text and system-metric measurement for the settings window's layout
//! planner.
//!
//! Everything that needs a device context lives here; `plan` consumes the
//! numbers and does no FFI of its own. Two rules the whole module exists to
//! enforce. Measurement excludes the string's terminating NUL — the
//! `DrawTextW` binding sends a slice's whole length as the character count,
//! so a NUL-terminated buffer measures one extra glyph. And every
//! DPI-sensitive query takes an explicit DPI: a test binary does not link
//! `main`'s `SetProcessDpiAwarenessContext`, so it runs DPI-unaware and a
//! plain `GetSystemMetrics` would silently answer for 96 at every scale.

use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DT_CALCRECT, DT_SINGLELINE, DT_WORDBREAK, DeleteDC, DeleteObject,
    DrawTextW, FW_BOLD, FW_NORMAL, HDC, HFONT, HGDIOBJ, SelectObject,
};
use windows::Win32::UI::Controls::{
    BP_CHECKBOX, CBS_UNCHECKEDNORMAL, CloseThemeData, GetThemePartSize, TS_DRAW,
};
use windows::Win32::UI::HiDpi::{GetSystemMetricsForDpi, OpenThemeDataForDpi};
use windows::Win32::UI::WindowsAndMessaging::SM_CXVSCROLL;
use windows::core::w;

use super::window::build_font;

/// Checkbox indicator width to assume when the theme cannot answer, matching
/// what the dark-mode painter already falls back to.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no non-test caller until the layout planner lands"
    )
)]
const CHECKBOX_INDICATOR_FALLBACK: i32 = 13;

/// What the layout planner needs to know about text and system metrics.
///
/// A trait rather than a concrete type so the planner can be exercised on
/// any host with a fake whose numbers are chosen to make an assertion
/// legible, instead of only against whatever font the machine happens to
/// have.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no non-test caller until the layout planner lands"
    )
)]
pub(super) trait TextMeasure {
    /// Width of `text` drawn on one line, in physical pixels.
    fn text_width(&mut self, text: &str, bold: bool) -> i32;

    /// Height of `text` wrapped into `width` physical pixels.
    fn wrapped_height(&mut self, text: &str, bold: bool, width: i32) -> i32;

    /// Width of the checkbox indicator glyph the dark-mode painter draws at
    /// this DPI — the width a caption has to sit beside.
    fn checkbox_indicator(&mut self) -> i32;

    /// Width of a combo box's dropdown arrow at this DPI.
    fn combo_arrow(&mut self) -> i32;
}

/// GDI-backed [`TextMeasure`] for one DPI.
///
/// Owns a memory device context and the two dialog fonts, which is why it
/// needs no window: a plan can therefore be computed before the settings
/// window is created, and the CI gate can run without one at all. Measured
/// widths from a memory DC match a screen DC's.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no non-test caller until the layout planner lands"
    )
)]
pub(super) struct GdiMeasure {
    dc: HDC,
    regular: HFONT,
    bold: HFONT,
    /// The object `SelectObject` displaced on first use, restored before the
    /// fonts are deleted: GDI refuses to delete a font still selected into a
    /// DC.
    restore: Option<HGDIOBJ>,
    dpi: u32,
}

#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no non-test caller until the layout planner lands"
    )
)]
impl GdiMeasure {
    /// Builds a measurer for `dpi`, or `None` if the device context or
    /// either font could not be created.
    pub(super) fn new(dpi: u32) -> Option<Self> {
        // SAFETY: a null reference DC asks for a memory DC compatible with
        // the screen, which is what the layout is measured against.
        let dc = unsafe { CreateCompatibleDC(None) };
        if dc.is_invalid() {
            log::warn!(dpi; "CreateCompatibleDC failed; settings layout cannot measure text");
            return None;
        }
        let regular = build_font(dpi, FW_NORMAL);
        let bold = build_font(dpi, FW_BOLD);
        if regular.is_invalid() || bold.is_invalid() {
            // Each `build_font` call makes its own font, so the one that did
            // succeed has to be freed here or it leaks.
            // SAFETY: every handle was created just above, none is selected
            // into any DC, and each is released once.
            unsafe {
                if !regular.is_invalid() {
                    let _ = DeleteObject(regular.into());
                }
                if !bold.is_invalid() {
                    let _ = DeleteObject(bold.into());
                }
                let _ = DeleteDC(dc);
            }
            return None;
        }
        Some(Self {
            dc,
            regular,
            bold,
            restore: None,
            dpi,
        })
    }

    /// Selects the requested weight, remembering the displaced object once so
    /// [`Drop`] can put it back.
    fn select(&mut self, bold: bool) {
        let font = if bold { self.bold } else { self.regular };
        // SAFETY: `self.dc` and both fonts were created in `new` and live as
        // long as `self`; neither font is selected into any other DC.
        let previous = unsafe { SelectObject(self.dc, font.into()) };
        // An invalid handle is not something `Drop` can put back, and storing
        // it would make `Drop` believe the stock font was already restored.
        if self.restore.is_none() && !previous.is_invalid() {
            self.restore = Some(previous);
        }
    }

    fn calc(&mut self, text: &str, bold: bool, wrap_width: Option<i32>) -> RECT {
        // No terminating NUL: the binding sends the slice's whole length, so
        // one would be measured as a glyph.
        let mut buf: Vec<u16> = text.encode_utf16().collect();
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: wrap_width.unwrap_or(0),
            bottom: 0,
        };
        let flags = match wrap_width {
            Some(_) => DT_CALCRECT | DT_WORDBREAK,
            None => DT_CALCRECT | DT_SINGLELINE,
        };
        self.select(bold);
        // SAFETY: `self.dc` is the memory DC created in `new` and valid for
        // this type's whole life; the font it needs is selected by `select`
        // just above; `buf` outlives the call and `DrawTextW` reads only the
        // code units its own length reports.
        unsafe {
            DrawTextW(self.dc, &mut buf, &raw mut rect, flags);
        }
        rect
    }
}

impl TextMeasure for GdiMeasure {
    fn text_width(&mut self, text: &str, bold: bool) -> i32 {
        if text.is_empty() {
            return 0;
        }
        let rect = self.calc(text, bold, None);
        rect.right - rect.left
    }

    fn wrapped_height(&mut self, text: &str, bold: bool, width: i32) -> i32 {
        if text.is_empty() || width <= 0 {
            return 0;
        }
        let rect = self.calc(text, bold, Some(width));
        rect.bottom - rect.top
    }

    fn checkbox_indicator(&mut self) -> i32 {
        // SAFETY: a null window handle asks for the class's own theme data,
        // which needs no window; the handle is closed below on the one path
        // that opens it.
        let theme = unsafe { OpenThemeDataForDpi(None, w!("BUTTON"), self.dpi) };
        if theme.is_invalid() {
            return CHECKBOX_INDICATOR_FALLBACK;
        }
        // TS_DRAW, not TS_TRUE: the dark-mode painter draws the glyph at the
        // drawing size, which at 192 DPI is 10 px wider than the true size,
        // and a caption has to clear what is actually drawn.
        // SAFETY: `theme` was just opened and checked valid; passing no DC
        // and no bounding rect asks the part for its own size.
        let size = unsafe {
            GetThemePartSize(
                theme,
                None,
                BP_CHECKBOX.0,
                CBS_UNCHECKEDNORMAL.0,
                None,
                TS_DRAW,
            )
        };
        // SAFETY: `theme` is the handle opened above, closed exactly once.
        unsafe {
            let _ = CloseThemeData(theme);
        }
        size.map_or(CHECKBOX_INDICATOR_FALLBACK, |size| size.cx)
    }

    fn combo_arrow(&mut self) -> i32 {
        unsafe { GetSystemMetricsForDpi(SM_CXVSCROLL, self.dpi) }
    }
}

impl Drop for GdiMeasure {
    fn drop(&mut self) {
        // SAFETY: every handle here was created in `new` and is dropped
        // exactly once. The displaced object goes back into the DC first —
        // GDI refuses to delete a font that is still selected, which would
        // leak both fonts silently.
        unsafe {
            if let Some(previous) = self.restore.take() {
                SelectObject(self.dc, previous);
            }
            let _ = DeleteObject(self.regular.into());
            let _ = DeleteObject(self.bold.into());
            let _ = DeleteDC(self.dc);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn widths_are_positive_monotonic_and_free_of_the_terminating_nul() {
        let mut m = GdiMeasure::new(96).expect("measure");
        // The properties the planner relies on from a memory DC.
        let short = m.text_width("Close", false);
        let long = m.text_width("Restore defaults", false);
        assert!(short > 0 && long > short, "short={short} long={long}");
        assert_eq!(
            m.text_width("debug", false),
            34,
            "the NUL must not be measured; 34 is the value recorded beside ID_LOG_LEVEL"
        );
    }

    #[test]
    fn bold_is_wider_than_regular_for_the_same_text() {
        let mut m = GdiMeasure::new(96).expect("measure");
        assert!(m.text_width("Advanced", true) > m.text_width("Advanced", false));
    }

    #[test]
    fn wrapping_a_long_hint_into_a_narrow_width_takes_more_than_one_line() {
        let mut m = GdiMeasure::new(96).expect("measure");
        let one_line = m.wrapped_height("Opacity", false, 400);
        let wrapped = m.wrapped_height(
            "(may not work with all keyboards; some antivirus software flags low-level hooks)",
            false,
            328,
        );
        assert!(wrapped > one_line, "one_line={one_line} wrapped={wrapped}");
    }

    #[test]
    fn system_metrics_come_from_the_requested_dpi_not_the_process() {
        // The test process is DPI-unaware, so a plain GetSystemMetrics would
        // return the same value at every DPI. These must differ.
        let mut low = GdiMeasure::new(96).expect("measure 96");
        let mut high = GdiMeasure::new(192).expect("measure 192");
        assert!(
            high.combo_arrow() > low.combo_arrow(),
            "arrow 96={} 192={}",
            low.combo_arrow(),
            high.combo_arrow()
        );
        // Strict, so a theme that never opens — every call answering with
        // the fallback — cannot pass.
        assert!(
            high.checkbox_indicator() > low.checkbox_indicator(),
            "checkbox 96={} 192={}",
            low.checkbox_indicator(),
            high.checkbox_indicator()
        );
    }
}
