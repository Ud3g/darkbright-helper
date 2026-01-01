//! On-Screen Display (OSD) implementation for Windows.

use std::cell::RefCell;
use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW, CreateSolidBrush,
    DeleteDC, DeleteObject, EndPaint, FillRect, GetMonitorInfoW, InvalidateRect, SelectObject,
    SetBkMode, SetTextColor, TextOutW, CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH,
    FF_DONTCARE, FW_NORMAL, HDC, HFONT, HMONITOR, MONITORINFO, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
    SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, GetClientRect, KillTimer, RegisterClassExW,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, CS_HREDRAW, CS_VREDRAW,
    CW_USEDEFAULT, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE, SW_HIDE, SW_SHOW, WM_PAINT, WM_TIMER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::core::state::MonitorState;
use crate::error::{BrightnessError, Result};
use super::{last_error_as_brightness_error, SafeHwnd};

/// The class name for the OSD window.
const OSD_CLASS_NAME: PCWSTR = w!("DarkBrightOSDClass");

/// OSD window width in pixels.
const OSD_WIDTH: i32 = 300;
/// OSD window height in pixels.
const OSD_HEIGHT: i32 = 80;
/// Margin from the bottom of the monitor in pixels.
const OSD_BOTTOM_MARGIN: i32 = 100;

/// OSD background color (dark gray, semi-transparent look).
const OSD_BACKGROUND_COLOR: u32 = 0x00303030; // RGB: 48, 48, 48

/// Padding inside the OSD window.
const OSD_PADDING: i32 = 10;
/// Height of the progress bar.
const BAR_HEIGHT: i32 = 20;
/// Spacing between bars (when two bars are shown).
const BAR_SPACING: i32 = 8;

/// Progress bar fill color (bright blue).
const BAR_FILL_COLOR: u32 = 0x00D0A030; // BGR: 48, 160, 208 (golden/orange)
/// Progress bar background color (dark gray).
const BAR_BACKGROUND_COLOR: u32 = 0x00505050; // BGR: 80, 80, 80
/// Text color (white).
const TEXT_COLOR: u32 = 0x00FFFFFF; // BGR: 255, 255, 255
/// Error bar color (red).
const BAR_ERROR_COLOR: u32 = 0x000000CC; // BGR: 204, 0, 0

/// Font size for percentage text.
const FONT_SIZE: i32 = 18;

/// Icon for hardware brightness (sun symbol).
const ICON_HARDWARE: &str = "🔆";
/// Icon for overlay/dimming (sunglasses symbol).
const ICON_OVERLAY: &str = "🕶";
/// Width reserved for the icon on the left side.
const ICON_WIDTH: i32 = 30;

/// Timer ID for the auto-hide functionality.
const HIDE_TIMER_ID: usize = 1;

// Thread-local storage for OSD render state.
// This allows the window procedure to access the current brightness values.
thread_local! {
    static OSD_STATE: RefCell<OsdRenderState> = RefCell::new(OsdRenderState::default());
}

/// State used for rendering the OSD.
#[derive(Debug, Clone, Default)]
struct OsdRenderState {
    /// Hardware brightness (0-100).
    hardware_brightness: u8,
    /// Overlay opacity (0-100, 0 = inactive).
    overlay_opacity: u8,
    /// Whether an error occurred.
    is_error: bool,
}

/// Ensures the window class is registered exactly once.
static REGISTER_CLASS_ONCE: OnceLock<Result<()>> = OnceLock::new();

/// Window procedure for the OSD window.
///
/// Handles `WM_PAINT` for custom rendering of the brightness indicator.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &mut ps);

                if hdc.0 != 0 {
                    paint_osd(hwnd, hdc);
                    EndPaint(hwnd, &ps);
                }

                LRESULT(0)
            }
            WM_TIMER => {
                if wparam.0 == HIDE_TIMER_ID {
                    let _ = KillTimer(hwnd, HIDE_TIMER_ID);
                    let _ = ShowWindow(hwnd, SW_HIDE);
                }
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Paints the OSD content using double-buffering.
///
/// # Safety
///
/// Must be called from within a `BeginPaint`/`EndPaint` block.
unsafe fn paint_osd(hwnd: HWND, hdc: HDC) {
    unsafe {
        let mut rect = RECT::default();
        if GetClientRect(hwnd, &mut rect).is_err() {
            return;
        }

        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        // Create memory DC for double-buffering
        let mem_dc = CreateCompatibleDC(hdc);
        if mem_dc.0 == 0 {
            return;
        }

        let mem_bitmap = CreateCompatibleBitmap(hdc, width, height);
        if mem_bitmap.0 == 0 {
            DeleteDC(mem_dc);
            return;
        }

        let old_bitmap = SelectObject(mem_dc, mem_bitmap);

        // Fill background
        let bg_brush = CreateSolidBrush(COLORREF(OSD_BACKGROUND_COLOR));
        FillRect(mem_dc, &rect, bg_brush);
        DeleteObject(bg_brush);

        // Get current state from thread-local storage
        let state = OSD_STATE.with(|s| s.borrow().clone());

        // Draw progress bar(s)
        draw_brightness_bars(mem_dc, &rect, &state);

        // Copy to screen
        let _ = BitBlt(hdc, 0, 0, width, height, mem_dc, 0, 0, SRCCOPY);

        // Cleanup
        SelectObject(mem_dc, old_bitmap);
        DeleteObject(mem_bitmap);
        DeleteDC(mem_dc);
    }
}

/// Draws the brightness progress bar(s).
///
/// # Safety
///
/// Must be called with a valid device context.
unsafe fn draw_brightness_bars(hdc: HDC, client_rect: &RECT, state: &OsdRenderState) {
    unsafe {
        let width = client_rect.right - client_rect.left;

        // Calculate bar dimensions (leave space for icon on left and percentage on right)
        let bar_width = width - (OSD_PADDING * 2) - ICON_WIDTH - 50;
        let bar_left = OSD_PADDING + ICON_WIDTH;

        // Determine if we show one or two bars
        let show_overlay_bar = state.overlay_opacity > 0;

        if show_overlay_bar {
            // Two-bar mode: hardware bar on top, overlay bar below
            let bar1_top = OSD_PADDING;
            let bar2_top = OSD_PADDING + BAR_HEIGHT + BAR_SPACING;

            // Hardware brightness bar with icon
            draw_icon(hdc, OSD_PADDING, bar1_top, ICON_HARDWARE);
            draw_single_bar(
                hdc,
                bar_left,
                bar1_top,
                bar_width,
                BAR_HEIGHT,
                state.hardware_brightness,
                state.is_error,
            );
            draw_percentage_text(hdc, bar_left + bar_width + 8, bar1_top, state.hardware_brightness);

            // Overlay bar with icon
            draw_icon(hdc, OSD_PADDING, bar2_top, ICON_OVERLAY);
            draw_single_bar(
                hdc,
                bar_left,
                bar2_top,
                bar_width,
                BAR_HEIGHT,
                state.overlay_opacity,
                false, // Overlay doesn't show error state
            );
            draw_percentage_text(hdc, bar_left + bar_width + 8, bar2_top, state.overlay_opacity);
        } else {
            // Single-bar mode: centered vertically
            let bar_top = (client_rect.bottom - client_rect.top - BAR_HEIGHT) / 2;

            draw_icon(hdc, OSD_PADDING, bar_top, ICON_HARDWARE);
            draw_single_bar(
                hdc,
                bar_left,
                bar_top,
                bar_width,
                BAR_HEIGHT,
                state.hardware_brightness,
                state.is_error,
            );
            draw_percentage_text(hdc, bar_left + bar_width + 8, bar_top, state.hardware_brightness);
        }
    }
}

/// Draws an icon (emoji) at the specified position.
///
/// # Safety
///
/// Must be called with a valid device context.
unsafe fn draw_icon(hdc: HDC, x: i32, y: i32, icon: &str) {
    unsafe {
        // Create font (use Segoe UI Emoji for proper emoji rendering)
        let font = create_icon_font();
        let old_font = SelectObject(hdc, font);

        // Set text properties
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(TEXT_COLOR));

        // Draw icon
        let wide_text: Vec<u16> = icon.encode_utf16().collect();
        let text_y = y + (BAR_HEIGHT - FONT_SIZE) / 2;
        TextOutW(hdc, x, text_y, &wide_text);

        // Cleanup
        SelectObject(hdc, old_font);
        DeleteObject(font);
    }
}

/// Creates the font used for OSD icons (emoji).
///
/// # Safety
///
/// The returned HFONT must be deleted with DeleteObject when no longer needed.
unsafe fn create_icon_font() -> HFONT {
    unsafe {
        CreateFontW(
            FONT_SIZE,           // Height
            0,                   // Width (0 = auto)
            0,                   // Escapement
            0,                   // Orientation
            FW_NORMAL.0 as i32,  // Weight
            0,                   // Italic
            0,                   // Underline
            0,                   // StrikeOut
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            0,                   // Quality (DEFAULT_QUALITY)
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            w!("Segoe UI Emoji"), // Font face for emoji support
        )
    }
}

/// Draws a single progress bar.
///
/// # Safety
///
/// Must be called with a valid device context.
unsafe fn draw_single_bar(
    hdc: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    percent: u8,
    is_error: bool,
) {
    unsafe {
        // Draw background
        let bg_rect = RECT {
            left: x,
            top: y,
            right: x + width,
            bottom: y + height,
        };
        let bg_brush = CreateSolidBrush(COLORREF(BAR_BACKGROUND_COLOR));
        FillRect(hdc, &bg_rect, bg_brush);
        DeleteObject(bg_brush);

        // Draw filled portion
        let fill_width = (width as i64 * percent as i64 / 100) as i32;
        if fill_width > 0 {
            let fill_rect = RECT {
                left: x,
                top: y,
                right: x + fill_width,
                bottom: y + height,
            };
            let fill_color = if is_error { BAR_ERROR_COLOR } else { BAR_FILL_COLOR };
            let fill_brush = CreateSolidBrush(COLORREF(fill_color));
            FillRect(hdc, &fill_rect, fill_brush);
            DeleteObject(fill_brush);
        }
    }
}

/// Draws the percentage text next to a bar.
///
/// # Safety
///
/// Must be called with a valid device context.
unsafe fn draw_percentage_text(hdc: HDC, x: i32, y: i32, percent: u8) {
    unsafe {
        // Create font
        let font = create_osd_font();
        let old_font = SelectObject(hdc, font);

        // Set text properties
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(TEXT_COLOR));

        // Format and draw text
        let text = format!("{}%", percent);
        let wide_text: Vec<u16> = text.encode_utf16().collect();

        // Center text vertically with the bar
        let text_y = y + (BAR_HEIGHT - FONT_SIZE) / 2;
        TextOutW(hdc, x, text_y, &wide_text);

        // Cleanup
        SelectObject(hdc, old_font);
        DeleteObject(font);
    }
}

/// Creates the font used for OSD text.
///
/// # Safety
///
/// The returned HFONT must be deleted with DeleteObject when no longer needed.
unsafe fn create_osd_font() -> HFONT {
    unsafe {
        CreateFontW(
            FONT_SIZE,           // Height
            0,                   // Width (0 = auto)
            0,                   // Escapement
            0,                   // Orientation
            FW_NORMAL.0 as i32,  // Weight
            0,                   // Italic
            0,                   // Underline
            0,                   // StrikeOut
            DEFAULT_CHARSET.0 as u32,
            OUT_DEFAULT_PRECIS.0 as u32,
            CLIP_DEFAULT_PRECIS.0 as u32,
            0,                   // Quality (DEFAULT_QUALITY)
            (DEFAULT_PITCH.0 | FF_DONTCARE.0) as u32,
            w!("Segoe UI"),      // Font face
        )
    }
}

/// Updates the OSD render state from a MonitorState.
fn update_osd_state(state: &MonitorState, is_error: bool) {
    OSD_STATE.with(|s| {
        let mut render_state = s.borrow_mut();
        render_state.hardware_brightness = state.effective_brightness();
        render_state.overlay_opacity = state.overlay_opacity;
        render_state.is_error = is_error;
    });
}

/// Registers the window class for the OSD window if not already registered.
pub fn ensure_osd_class_registered() -> Result<PCWSTR> {
    REGISTER_CLASS_ONCE
        .get_or_init(|| {
            unsafe {
                let hinstance = GetModuleHandleW(None).map_err(|e| {
                    BrightnessError::windows_api("GetModuleHandleW", e.code().0 as u32)
                })?;

                // Ein grauer Brush als Standardhintergrund.
                let background_brush = CreateSolidBrush(COLORREF(OSD_BACKGROUND_COLOR));

                let wnd_class = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: hinstance.into(),
                    hCursor: Default::default(),
                    hbrBackground: background_brush,
                    lpszClassName: OSD_CLASS_NAME,
                    ..Default::default()
                };

                if RegisterClassExW(&wnd_class) == 0 {
                    return Err(last_error_as_brightness_error("RegisterClassExW"));
                }
            }
            Ok(())
        })
        .as_ref()
        .map_err(|_| {
            BrightnessError::windows_api("ensure_osd_class_registered", 0)
        })?;

    Ok(OSD_CLASS_NAME)
}

/// Creates a new OSD window.
///
/// The window is created as:
/// - Layered (for transparency)
/// - Transparent (click-through)
/// - Tool window (no taskbar)
/// - Topmost
pub fn create_osd_window() -> Result<SafeHwnd> {
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
            0, // Größe wird in Schritt 26 berechnet
            0,
            None,
            None,
            GetModuleHandleW(None).unwrap_or_default(),
            None,
        );

        if hwnd.0 == 0 {
            return Err(last_error_as_brightness_error("CreateWindowExW"));
        }

        Ok(SafeHwnd::new_owned(hwnd))
    }
}

/// Positions the OSD window at the bottom-center of the specified monitor.
/// (Step #26)
pub fn position_osd_window(hwnd: HWND, hmonitor: HMONITOR) -> Result<()> {
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };

    unsafe {
        if !GetMonitorInfoW(hmonitor, &mut mi).as_bool() {
            return Err(last_error_as_brightness_error("GetMonitorInfoW"));
        }

        let rect = mi.rcMonitor;
        let monitor_width = rect.right - rect.left;

        // Calculate center-x and bottom-y position
        let x = rect.left + (monitor_width - OSD_WIDTH) / 2;
        let y = rect.bottom - OSD_HEIGHT - OSD_BOTTOM_MARGIN;

        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            OSD_WIDTH,
            OSD_HEIGHT,
            SWP_NOACTIVATE,
        )
        .map_err(|e| BrightnessError::windows_api("SetWindowPos", e.code().0 as u32))?;
    }

    Ok(())
}

/// Sets the overall opacity of the OSD window.
pub fn set_osd_opacity(hwnd: HWND, opacity: f32) -> Result<()> {
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    unsafe {
        SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA).map_err(|e| {
            BrightnessError::windows_api("SetLayeredWindowAttributes", e.code().0 as u32)
        })?;
    }
    Ok(())
}

/// Manages the On-Screen Display (OSD) window.
/// (Step #33)
pub struct OsdWindow {
    hwnd: SafeHwnd,
    timeout_ms: u32,
}

impl OsdWindow {
    /// Creates a new OsdWindow.
    pub fn new(opacity: f32, timeout_ms: u32) -> Result<Self> {
        let hwnd = create_osd_window()?;
        set_osd_opacity(hwnd.as_raw(), opacity)?;

        Ok(Self { hwnd, timeout_ms })
    }

    /// Shows the OSD for a specific monitor with the given state.
    pub fn show(&mut self, hmonitor: HMONITOR, state: &MonitorState) -> Result<()> {
        position_osd_window(self.hwnd.as_raw(), hmonitor)?;
        update_osd_state(state, false);

        unsafe {
            let _ = InvalidateRect(self.hwnd.as_raw(), None, true);
            let _ = ShowWindow(self.hwnd.as_raw(), SW_SHOW);
            self.reset_timer();
        }

        Ok(())
    }

    /// Shows the OSD with an error indicator.
    pub fn show_error(&mut self, hmonitor: HMONITOR, state: &MonitorState) -> Result<()> {
        position_osd_window(self.hwnd.as_raw(), hmonitor)?;
        update_osd_state(state, true);

        unsafe {
            let _ = InvalidateRect(self.hwnd.as_raw(), None, true);
            let _ = ShowWindow(self.hwnd.as_raw(), SW_SHOW);
            self.reset_timer();
        }

        Ok(())
    }

    /// Hides the OSD window.
    pub fn hide(&mut self) -> Result<()> {
        unsafe {
            let _ = ShowWindow(self.hwnd.as_raw(), SW_HIDE);
        }
        Ok(())
    }

    /// Triggers a redraw of the OSD with updated state.
    pub fn update(&mut self, state: &MonitorState) -> Result<()> {
        update_osd_state(state, false);
        unsafe {
            // Invalidate the entire window to trigger WM_PAINT
            let _ = InvalidateRect(self.hwnd.as_raw(), None, true);
            self.reset_timer();
        }
        Ok(())
    }

    /// Triggers a redraw of the OSD with error state.
    pub fn update_error(&mut self, state: &MonitorState) -> Result<()> {
        update_osd_state(state, true);
        unsafe {
            let _ = InvalidateRect(self.hwnd.as_raw(), None, true);
            self.reset_timer();
        }
        Ok(())
    }

    /// Resets the auto-hide timer.
    fn reset_timer(&self) {
        unsafe {
            let _ = SetTimer(self.hwnd.as_raw(), HIDE_TIMER_ID, self.timeout_ms, None);
        }
    }

    /// Returns true if the OSD window is currently visible.
    pub fn is_visible(&self) -> bool {
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
            IsWindowVisible(self.hwnd.as_raw()).as_bool()
        }
    }

    /// Returns the raw window handle.
    pub fn hwnd(&self) -> HWND {
        self.hwnd.as_raw()
    }
}
