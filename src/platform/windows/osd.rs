//! On-Screen Display (OSD) implementation for Windows.
//!
//! ## Layout (Bidirectional Bar)
//!
//! ```text
//! ┌──────────────────────────────────────────────────────────┐
//! │  pct 🕶 ░░░░░░░░██████████ ██████████░░░░░░░░ 🔆 pct     │
//! │       └─ overlay (left) ─┘ └─ hardware (right) ─┘        │
//! └──────────────────────────────────────────────────────────┘
//! ```
//!
//! - Left half: overlay dimming (fills right-to-left, purple)
//! - Right half: hardware brightness (fills left-to-right, gold)
//! - Small gap separates the two halves
//! - Compact height (50px) normally; expands to 75px for error messages

use std::cell::RefCell;
use std::sync::OnceLock;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateSolidBrush, EndPaint, GetMonitorInfoW, HDC, HMONITOR, InvalidateRect,
    MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow, PAINTSTRUCT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, GetClientRect,
    HWND_TOPMOST, KillTimer, LWA_ALPHA, RegisterClassExW, SW_HIDE, SW_SHOW, SWP_NOACTIVATE,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, WM_PAINT, WM_TIMER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::{SafeHwnd, hmonitor_from_isize, last_error_as_brightness_error, osd_render};
use crate::core::state::MonitorState;
use crate::error::{BrightnessError, Result};

/// The class name for the OSD window.
const OSD_CLASS_NAME: PCWSTR = w!("DarkBrightOSDClass");

/// Base OSD window width in pixels (wider for bidirectional bar with two labels).
const BASE_OSD_WIDTH: i32 = 360;
/// Base OSD window height in pixels (compact single-row layout).
const BASE_OSD_HEIGHT: i32 = 50;
/// Base OSD window height when displaying an error message (expanded).
const BASE_OSD_HEIGHT_WITH_ERROR: i32 = 75;
/// Base margin from the bottom of the monitor in pixels.
const BASE_OSD_BOTTOM_MARGIN: i32 = 100;

/// Base height reserved for the error message row (when expanded).
const BASE_ERROR_ROW_HEIGHT: i32 = 25;

/// OSD background color (dark gray, semi-transparent look).
pub(super) const OSD_BACKGROUND_COLOR: u32 = 0x0030_3030; // RGB: 48, 48, 48

/// Base padding inside the OSD window.
const BASE_OSD_PADDING: i32 = 10;
/// Base height of the progress bar.
const BASE_BAR_HEIGHT: i32 = 20;
/// Base gap between the left (overlay) and right (hardware) bar sections.
const BASE_BAR_GAP: i32 = 4;

/// Base font size for percentage text.
const BASE_FONT_SIZE: i32 = 18;

/// Base width reserved for each icon.
const BASE_ICON_WIDTH: i32 = 25;
/// Base width reserved for percentage text (e.g., "100%").
const BASE_PERCENT_TEXT_WIDTH: i32 = 45;

/// Metrics for the OSD window, scaled for a specific DPI.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OsdMetrics {
    /// OSD window width in pixels.
    pub(crate) width: i32,
    /// OSD window height in pixels (compact mode).
    pub(crate) height: i32,
    /// OSD window height when displaying an error message (expanded mode).
    pub(crate) height_with_error: i32,
    /// Margin from the bottom of the monitor in pixels.
    pub(crate) bottom_margin: i32,
    /// Height reserved for the error message row.
    pub(crate) error_row_height: i32,
    /// Padding inside the OSD window.
    pub(crate) padding: i32,
    /// Height of the progress bar.
    pub(crate) bar_height: i32,
    /// Gap between the left (overlay) and right (hardware) bar sections.
    pub(crate) bar_gap: i32,
    /// Font size for text elements.
    pub(crate) font_size: i32,
    /// Width reserved for each icon.
    pub(crate) icon_width: i32,
    /// Width reserved for percentage text (e.g., "100%").
    pub(crate) percent_text_width: i32,
}

impl Default for OsdMetrics {
    fn default() -> Self {
        Self::for_dpi(96)
    }
}

impl OsdMetrics {
    /// Calculates metrics for the given DPI.
    ///
    /// Base values are designed for 96 DPI (100% scaling).
    #[must_use]
    pub(crate) fn for_dpi(dpi: u32) -> Self {
        #[expect(clippy::cast_precision_loss)]
        let scale = dpi as f32 / 96.0;

        // Helper to scale and round
        let s = |val: i32| {
            #[expect(clippy::cast_precision_loss, clippy::cast_possible_truncation)]
            let scaled = (val as f32 * scale).round() as i32;
            scaled
        };

        Self {
            width: s(BASE_OSD_WIDTH),
            height: s(BASE_OSD_HEIGHT),
            height_with_error: s(BASE_OSD_HEIGHT_WITH_ERROR),
            bottom_margin: s(BASE_OSD_BOTTOM_MARGIN),
            error_row_height: s(BASE_ERROR_ROW_HEIGHT),
            padding: s(BASE_OSD_PADDING),
            bar_height: s(BASE_BAR_HEIGHT),
            bar_gap: s(BASE_BAR_GAP),
            font_size: s(BASE_FONT_SIZE),
            icon_width: s(BASE_ICON_WIDTH),
            percent_text_width: s(BASE_PERCENT_TEXT_WIDTH),
        }
    }
}

/// Timer ID for the auto-hide functionality.
const HIDE_TIMER_ID: usize = 1;

thread_local! {
    /// What the OSD currently shows. Thread-local rather than passed in
    /// because `WM_PAINT` arrives at the window procedure with no room for
    /// an argument; the main thread owns the OSD and is the only one that
    /// touches this.
    static OSD_STATE: RefCell<OsdRenderState> = RefCell::new(OsdRenderState::default());
    /// DPI-derived sizes for the monitor the OSD is on, recomputed on every
    /// repositioning. Split from [`OSD_STATE`] because it changes on a
    /// different occasion: display geometry, not brightness.
    static OSD_METRICS: RefCell<OsdMetrics> = RefCell::new(OsdMetrics::default());
}

/// State used for rendering the OSD.
#[derive(Debug, Clone, Default)]
pub(super) struct OsdRenderState {
    /// Hardware brightness (0-100).
    pub(super) hardware_brightness: u8,
    /// Overlay opacity (0-100, 0 = inactive).
    pub(super) overlay_opacity: u8,
    /// Whether an error occurred.
    pub(super) is_error: bool,
}

/// Ensures the window class is registered exactly once.
static REGISTER_CLASS_ONCE: OnceLock<Result<()>> = OnceLock::new();

/// Window procedure for the OSD window.
///
/// Handles `WM_PAINT` for custom rendering of the brightness indicator.
///
/// # Safety
///
/// This is a system callback. The caller must ensure `hwnd` is a valid window handle.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // SAFETY: Windows is the caller, so `hwnd` is a live OSD window and
    // `wparam` means what `msg` documents.
    unsafe {
        match msg {
            WM_PAINT => {
                log::trace!("WM_PAINT received");
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &raw mut ps);

                if hdc.is_invalid() {
                    log::warn!("BeginPaint returned null HDC");
                } else {
                    paint_osd(hwnd, hdc);
                    let _ = EndPaint(hwnd, &raw const ps);
                }

                LRESULT(0)
            }
            WM_TIMER => {
                log::trace!(timer_id = wparam.0; "WM_TIMER received");
                if wparam.0 == HIDE_TIMER_ID {
                    log::debug!("Auto-hiding after timeout");
                    let _ = KillTimer(Some(hwnd), HIDE_TIMER_ID);
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Snapshots render state and metrics, then delegates drawing to `osd_render`.
///
/// # Safety
///
/// Must be called from within a `BeginPaint`/`EndPaint` block.
unsafe fn paint_osd(hwnd: HWND, hdc: HDC) {
    // Kept here (before the GetClientRect guard) so the trace order on a
    // GetClientRect failure is identical to the pre-refactor behavior.
    log::trace!("Painting OSD");
    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &raw mut rect) }.is_err() {
        log::warn!("GetClientRect failed");
        return;
    }

    let state = OSD_STATE.with(|s| s.borrow().clone());
    let metrics = OSD_METRICS.with(|m| *m.borrow());
    // SAFETY: `osd_render::paint` requires a live paint DC; this function's
    // own contract requires the same of `hdc` and the only caller satisfies
    // it, so forwarding the handle discharges the precondition.
    unsafe { osd_render::paint(hdc, &rect, &state, &metrics) };
}

/// Updates the OSD render state from a `MonitorState`.
fn update_osd_state(state: &MonitorState, is_error: bool) {
    let hw = state.effective_brightness();
    let overlay = state.overlay_opacity;
    log::trace!(
        hardware_brightness = hw,
        overlay_opacity = overlay,
        is_error = is_error;
        "Updating OSD state"
    );
    OSD_STATE.with(|s| {
        let mut render_state = s.borrow_mut();
        render_state.hardware_brightness = hw;
        render_state.overlay_opacity = overlay;
        render_state.is_error = is_error;
    });
}

/// Updates the OSD metrics for a new DPI.
fn update_osd_metrics(dpi: u32) {
    OSD_METRICS.with(|m| {
        *m.borrow_mut() = OsdMetrics::for_dpi(dpi);
    });
}

/// Executes a closure with the current OSD metrics.
#[inline]
fn with_metrics<R>(f: impl FnOnce(&OsdMetrics) -> R) -> R {
    OSD_METRICS.with(|m| f(&m.borrow()))
}

/// Registers the window class for the OSD window if not already registered.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if `GetModuleHandleW` or `RegisterClassExW` fails.
pub(crate) fn ensure_osd_class_registered() -> Result<PCWSTR> {
    REGISTER_CLASS_ONCE
        .get_or_init(|| {
            unsafe {
                let hinstance = GetModuleHandleW(None).map_err(|e| {
                    BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
                })?;

                let background_brush = CreateSolidBrush(COLORREF(OSD_BACKGROUND_COLOR));

                let wnd_class = WNDCLASSEXW {
                    cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(0),
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: hinstance.into(),
                    hCursor: windows::Win32::UI::WindowsAndMessaging::HCURSOR::default(),
                    hbrBackground: background_brush,
                    lpszClassName: OSD_CLASS_NAME,
                    ..Default::default()
                };

                if RegisterClassExW(&raw const wnd_class) == 0 {
                    return Err(last_error_as_brightness_error("RegisterClassExW"));
                }
            }
            Ok(())
        })
        .as_ref()
        .map_err(|_| BrightnessError::windows_api("ensure_osd_class_registered", 0))?;

    Ok(OSD_CLASS_NAME)
}

/// Creates a new OSD window.
///
/// The window is created as:
/// - Layered (for transparency)
/// - Transparent (click-through)
/// - Tool window (no taskbar)
/// - Topmost
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if window class registration or
/// `CreateWindowExW` fails.
pub(crate) fn create_osd_window() -> Result<SafeHwnd> {
    let class_name = ensure_osd_class_registered()?;

    unsafe {
        let ex_style = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;
        let style = WS_POPUP;

        let hwnd = CreateWindowExW(
            ex_style,
            class_name,
            w!("DarkBrightOSD"),
            style,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0, // Size will be set by position_osd_window()
            0,
            None,
            None,
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None,
        )
        .map_err(|e| BrightnessError::windows_api("CreateWindowExW", e.code().0.cast_unsigned()))?;

        apply_rounded_corners(hwnd);

        Ok(SafeHwnd::new_owned(hwnd))
    }
}

/// Positions the OSD window at the bottom-center of the specified monitor.
///
/// # Arguments
///
/// * `hwnd` - The `HWND` of the OSD window.
/// * `hmonitor` - The `HMONITOR` to position the OSD on.
/// * `with_error` - If true, use expanded height for error message row.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if `GetMonitorInfoW` or `SetWindowPos` fails.
pub(crate) fn position_osd_window(hwnd: HWND, hmonitor: HMONITOR, with_error: bool) -> Result<()> {
    let dpi = get_monitor_dpi(hmonitor);
    update_osd_metrics(dpi);

    let (width, height, bottom_margin) = with_metrics(|m| {
        let height = if with_error {
            m.height_with_error
        } else {
            m.height
        };
        (m.width, height, m.bottom_margin)
    });

    log::trace!(dpi, width, height; "Positioning OSD window with scaled metrics");

    let mut mi = MONITORINFO {
        cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).unwrap_or(0),
        ..Default::default()
    };

    unsafe {
        if !GetMonitorInfoW(hmonitor, &raw mut mi).as_bool() {
            return Err(last_error_as_brightness_error("GetMonitorInfoW"));
        }

        let rect = mi.rcMonitor;
        let monitor_width = rect.right - rect.left;

        let x = rect.left + (monitor_width - width) / 2;
        let y = rect.bottom - height - bottom_margin;

        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            width,
            height,
            SWP_NOACTIVATE,
        )
        .map_err(|e| BrightnessError::windows_api("SetWindowPos", e.code().0.cast_unsigned()))?;
    }

    Ok(())
}

/// Resizes the OSD window for compact or expanded (error) mode.
///
/// Uses `MonitorFromWindow` to determine the current monitor.
///
/// # Arguments
///
/// * `hwnd` - The `HWND` of the OSD window.
/// * `with_error` - If true, use expanded height for error message row.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if positioning fails.
fn resize_osd_window(hwnd: HWND, with_error: bool) -> Result<()> {
    let hmonitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    position_osd_window(hwnd, hmonitor, with_error)
}

/// Sets the overall opacity of the OSD window.
///
/// # Arguments
///
/// * `hwnd` - The OSD window handle.
/// * `opacity` - Opacity value from 0.0 (invisible) to 1.0 (fully opaque).
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if `SetLayeredWindowAttributes` fails.
pub(crate) fn set_osd_opacity(hwnd: HWND, opacity: f32) -> Result<()> {
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    unsafe {
        SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA).map_err(|e| {
            BrightnessError::windows_api("SetLayeredWindowAttributes", e.code().0.cast_unsigned())
        })?;
    }
    Ok(())
}

/// Applies rounded corners to the window using DWM (Windows 11+).
///
/// This is a best-effort operation. On Windows 10 or if DWM is unavailable,
/// the call will fail silently and the window will have square corners.
///
/// # Arguments
///
/// * `hwnd` - The window handle to apply rounded corners to.
fn apply_rounded_corners(hwnd: HWND) {
    let preference = DWMWCP_ROUND;
    // SAFETY: the attribute value is passed as an untyped pointer with its
    // size as a separate argument; both are taken from the same local, so
    // DWM reads exactly the bytes that exist, and the local outlives the
    // call. `DWMWA_WINDOW_CORNER_PREFERENCE` is the attribute whose value
    // type matches that of `preference`.
    let result = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const preference).cast(),
            u32::try_from(std::mem::size_of_val(&preference)).unwrap_or(0),
        )
    };

    match result {
        Ok(()) => log::debug!("Applied rounded corners to OSD window"),
        Err(e) => log::debug!(
            error_code = e.code().0;
            "Rounded corners not available (expected on Windows 10)"
        ),
    }
}

/// Returns the effective DPI for the specified monitor.
///
/// Falls back to the default system DPI (96) if the API fails.
fn get_monitor_dpi(hmonitor: HMONITOR) -> u32 {
    let mut dpi_x = 96;
    let mut dpi_y = 96;

    unsafe {
        // MDT_EFFECTIVE_DPI returns the effective DPI for the monitor, which takes into account
        // the user's scaling settings and any system overrides.
        if GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y).is_ok() {
            dpi_x
        } else {
            log::warn!("GetDpiForMonitor failed, falling back to 96 DPI");
            96
        }
    }
}

/// Manages the On-Screen Display (OSD) window.
///
/// Provides methods to show, hide, and update the brightness indicator
/// with automatic timeout-based hiding.
pub struct OsdWindow {
    hwnd: SafeHwnd,
    timeout_ms: u32,
}

impl OsdWindow {
    /// Creates a new `OsdWindow`.
    ///
    /// # Arguments
    ///
    /// * `opacity` - Window opacity from 0.0 to 1.0.
    /// * `timeout_ms` - Auto-hide timeout in milliseconds.
    ///
    /// # Errors
    ///
    /// Returns an error if window creation or opacity setting fails.
    pub fn new(opacity: f32, timeout_ms: u32) -> Result<Self> {
        let hwnd = create_osd_window()?;
        set_osd_opacity(hwnd.as_raw(), opacity)?;

        Ok(Self { hwnd, timeout_ms })
    }

    /// Shows the OSD for a specific monitor with the given state.
    ///
    /// Uses compact height (single bar row).
    ///
    /// # Arguments
    ///
    /// * `hmonitor` - The monitor to display the OSD on.
    /// * `state` - The current monitor brightness state.
    ///
    /// # Errors
    ///
    /// Returns an error if window positioning fails.
    pub(crate) fn show(&mut self, hmonitor: HMONITOR, state: &MonitorState) -> Result<()> {
        position_osd_window(self.hwnd.as_raw(), hmonitor, false)?;
        update_osd_state(state, false);

        unsafe {
            let _ = InvalidateRect(Some(self.hwnd.as_raw()), None, true);
            let _ = ShowWindow(self.hwnd.as_raw(), SW_SHOW);
            self.reset_timer();
        }

        log::debug!(
            hardware = state.effective_brightness(),
            overlay = state.overlay_opacity;
            "OSD shown"
        );

        Ok(())
    }

    /// Shows the OSD with an error indicator (red progress bar + error message).
    ///
    /// Uses expanded height to show error message row.
    ///
    /// # Arguments
    ///
    /// * `hmonitor` - The monitor to display the OSD on.
    /// * `state` - The current monitor brightness state.
    ///
    /// # Errors
    ///
    /// Returns an error if window positioning fails.
    /// Triggers a redraw of the OSD with updated state.
    ///
    /// Resizes to compact height (no error row) and resets the auto-hide timer.
    ///
    /// # Arguments
    ///
    /// * `state` - The current monitor brightness state.
    ///
    /// # Errors
    ///
    /// Returns an error if window resizing fails.
    pub(crate) fn update(&mut self, state: &MonitorState) -> Result<()> {
        update_osd_state(state, false);
        // The window may still be at the taller error height from an earlier update.
        resize_osd_window(self.hwnd.as_raw(), false)?;
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd.as_raw()), None, true);
            self.reset_timer();
        }
        Ok(())
    }

    /// Triggers a redraw of the OSD with error state (red progress bar + message).
    ///
    /// Resizes to expanded height (with error row) and resets the auto-hide timer.
    ///
    /// # Arguments
    ///
    /// * `state` - The current monitor brightness state.
    ///
    /// # Errors
    ///
    /// Returns an error if window resizing fails.
    pub(crate) fn update_error(&mut self, state: &MonitorState) -> Result<()> {
        update_osd_state(state, true);
        resize_osd_window(self.hwnd.as_raw(), true)?;
        unsafe {
            let _ = InvalidateRect(Some(self.hwnd.as_raw()), None, true);
            self.reset_timer();
        }
        Ok(())
    }

    /// Resets the auto-hide timer.
    fn reset_timer(&self) {
        unsafe {
            let _ = SetTimer(
                Some(self.hwnd.as_raw()),
                HIDE_TIMER_ID,
                self.timeout_ms,
                None,
            );
        }
    }

    /// Returns `true` if the OSD window is currently visible.
    #[must_use]
    pub(crate) fn is_visible(&self) -> bool {
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
            IsWindowVisible(self.hwnd.as_raw()).as_bool()
        }
    }

    /// Applies a live opacity/timeout preview from the settings dialog.
    ///
    /// The auto-hide timer picks up the new `timeout_ms` on its next reset;
    /// a failure to set the window's alpha channel is logged and otherwise
    /// ignored, since this is a preview rather than adjustment feedback.
    pub(crate) fn set_appearance(&mut self, opacity: f32, timeout_ms: u32) {
        self.timeout_ms = timeout_ms;
        if let Err(e) = set_osd_opacity(self.hwnd.as_raw(), opacity) {
            log::warn!(error:% = e; "Failed to apply OSD opacity from settings dialog");
        }
    }
}

impl crate::core::controller::OsdSink for OsdWindow {
    fn show(
        &mut self,
        handle: crate::core::controller::MonitorHandle,
        state: &MonitorState,
    ) -> Result<()> {
        OsdWindow::show(self, hmonitor_from_isize(handle.0), state)
    }
    fn update(&mut self, state: &MonitorState) -> Result<()> {
        OsdWindow::update(self, state)
    }
    fn update_error(&mut self, state: &MonitorState) -> Result<()> {
        OsdWindow::update_error(self, state)
    }
    fn is_visible(&self) -> bool {
        OsdWindow::is_visible(self)
    }
    fn set_appearance(&mut self, opacity: f32, timeout_ms: u32) {
        OsdWindow::set_appearance(self, opacity, timeout_ms);
    }
}
