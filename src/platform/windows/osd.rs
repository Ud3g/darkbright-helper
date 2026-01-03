//! On-Screen Display (OSD) implementation for Windows.
//!
//! ## Layout (F.6 Bidirectional Bar)
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
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap, CreateCompatibleDC,
    CreateFontW, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH, DeleteDC, DeleteObject,
    EndPaint, FF_DONTCARE, FW_NORMAL, FillRect, GetMonitorInfoW, HDC, HFONT, HMONITOR,
    InvalidateRect, MONITORINFO, MonitorFromWindow, MONITOR_DEFAULTTONEAREST, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, SRCCOPY, SelectObject, SetBkMode, SetTextColor, TRANSPARENT, TextOutW,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, GetClientRect,
    HWND_TOPMOST, KillTimer, LWA_ALPHA, RegisterClassExW, SW_HIDE, SW_SHOW, SWP_NOACTIVATE,
    SetLayeredWindowAttributes, SetTimer, SetWindowPos, ShowWindow, WM_PAINT, WM_TIMER,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::{SafeHwnd, last_error_as_brightness_error};
use crate::core::state::MonitorState;
use crate::error::{BrightnessError, Result};

/// The class name for the OSD window.
const OSD_CLASS_NAME: PCWSTR = w!("DarkBrightOSDClass");

/// OSD window width in pixels (wider for bidirectional bar with two labels).
const OSD_WIDTH: i32 = 360;
/// OSD window height in pixels (compact single-row layout).
const OSD_HEIGHT: i32 = 50;
/// OSD window height when displaying an error message (expanded).
const OSD_HEIGHT_WITH_ERROR: i32 = 75;
/// Margin from the bottom of the monitor in pixels.
const OSD_BOTTOM_MARGIN: i32 = 100;

/// Height reserved for the error message row (when expanded).
const ERROR_ROW_HEIGHT: i32 = 25;
/// Error message displayed when DDC communication fails.
const ERROR_MESSAGE: &str = "DDC Error - Adjustment failed";

/// OSD background color (dark gray, semi-transparent look).
const OSD_BACKGROUND_COLOR: u32 = 0x0030_3030; // RGB: 48, 48, 48

/// Padding inside the OSD window.
const OSD_PADDING: i32 = 10;
/// Height of the progress bar.
const BAR_HEIGHT: i32 = 20;
/// Gap between the left (overlay) and right (hardware) bar sections.
const BAR_GAP: i32 = 4;

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

/// Font size for percentage text.
const FONT_SIZE: i32 = 18;

/// Icon for hardware brightness (sun symbol).
const ICON_HARDWARE: &str = "🔆";
/// Icon for overlay/dimming (sunglasses symbol).
const ICON_OVERLAY: &str = "🕶";
/// Width reserved for each icon.
const ICON_WIDTH: i32 = 25;
/// Width reserved for percentage text (e.g., "100%").
const PERCENT_TEXT_WIDTH: i32 = 45;

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
    unsafe {
        match msg {
            WM_PAINT => {
                log::trace!("WM_PAINT received");
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &raw mut ps);

                if hdc.0 != 0 {
                    paint_osd(hwnd, hdc);
                    EndPaint(hwnd, &raw const ps);
                } else {
                    log::trace!("BeginPaint returned null HDC");
                }

                LRESULT(0)
            }
            WM_TIMER => {
                log::trace!(timer_id = wparam.0; "WM_TIMER received");
                if wparam.0 == HIDE_TIMER_ID {
                    log::debug!("Auto-hiding after timeout");
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
        log::trace!("Painting OSD");
        let mut rect = RECT::default();
        if GetClientRect(hwnd, &raw mut rect).is_err() {
            log::trace!("GetClientRect failed");
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
            let _ = DeleteDC(mem_dc);
            return;
        }

        let old_bitmap = SelectObject(mem_dc, mem_bitmap);

        // Fill background
        let bg_brush = CreateSolidBrush(COLORREF(OSD_BACKGROUND_COLOR));
        FillRect(mem_dc, &raw const rect, bg_brush);
        let _ = DeleteObject(bg_brush);

        // Get current state from thread-local storage
        let state = OSD_STATE.with(|s| s.borrow().clone());

        // Draw progress bar(s)
        draw_brightness_bars(mem_dc, &rect, &state);

        // Draw error message if in error state
        if state.is_error {
            draw_error_message(mem_dc, &rect, ERROR_MESSAGE);
        }

        // Copy to screen
        let _ = BitBlt(hdc, 0, 0, width, height, mem_dc, 0, 0, SRCCOPY);

        // Cleanup
        SelectObject(mem_dc, old_bitmap);
        DeleteObject(mem_bitmap);
        DeleteDC(mem_dc);
    }
}

/// Draws the bidirectional brightness bar (F.6 layout).
///
/// Layout: `|pad|pct 🕶 ░░░░██████|gap|██████░░░░ 🔆 pct|pad|`
///
/// - Left half: overlay dimming (fills right-to-left)
/// - Right half: hardware brightness (fills left-to-right)
/// - Small gap separates the two halves
///
/// # Safety
///
/// Must be called with a valid device context.
unsafe fn draw_brightness_bars(hdc: HDC, client_rect: &RECT, state: &OsdRenderState) {
    unsafe {
        log::trace!(
            hardware_brightness = state.hardware_brightness,
            overlay_opacity = state.overlay_opacity,
            is_error = state.is_error;
            "Drawing bidirectional brightness bar"
        );

        // Use fixed bar position based on compact height (OSD_HEIGHT).
        // This keeps the bar in the same position whether or not the error row is visible.
        // When expanded for errors, the extra space is at the bottom for the error message.
        let bar_top = (OSD_HEIGHT - BAR_HEIGHT) / 2;

        // Draw left side (overlay section)
        draw_overlay_section(hdc, client_rect, bar_top, state.overlay_opacity);

        // Draw right side (hardware section)
        draw_hardware_section(
            hdc,
            client_rect,
            bar_top,
            state.hardware_brightness,
            state.is_error,
        );
    }
}

/// Draws the hardware brightness section (right half of the bidirectional bar).
///
/// Layout: `|...|gap|████████░░░░░░░░░░ 🔆 100%|pad|`
///
/// The bar fills from left to right based on hardware brightness percentage.
///
/// # Safety
///
/// Must be called with a valid device context.
unsafe fn draw_hardware_section(
    hdc: HDC,
    client_rect: &RECT,
    bar_top: i32,
    hardware_brightness: u8,
    is_error: bool,
) {
    unsafe {
        let width = client_rect.right - client_rect.left;

        // Calculate layout positions
        // Full layout: |pad|pct|ico|left_bar|gap|right_bar|ico|pct|pad|
        let content_width = width - (OSD_PADDING * 2);
        let total_bar_width =
            content_width - (PERCENT_TEXT_WIDTH * 2) - (ICON_WIDTH * 2) - BAR_GAP;
        let single_bar_width = total_bar_width / 2;

        // Right bar starts after: pad + pct + ico + left_bar + gap
        let bar_left =
            OSD_PADDING + PERCENT_TEXT_WIDTH + ICON_WIDTH + single_bar_width + BAR_GAP;

        // Draw the bar (fills left-to-right)
        draw_single_bar(
            hdc,
            bar_left,
            bar_top,
            single_bar_width,
            BAR_HEIGHT,
            hardware_brightness,
            is_error,
        );

        // Icon position: right of bar with small padding
        let icon_x = bar_left + single_bar_width + 2;
        draw_icon(hdc, icon_x, bar_top, ICON_HARDWARE);

        // Percentage text position: right of icon
        let percent_x = icon_x + ICON_WIDTH;
        draw_percentage_text(hdc, percent_x, bar_top, hardware_brightness);
    }
}

/// Draws the overlay dimming section (left half of the bidirectional bar).
///
/// Layout: `|pad|30% 🕶 ░░░░░░░░░░░░░░██████|gap|...|`
///
/// The bar fills from right to left based on overlay opacity percentage.
///
/// # Safety
///
/// Must be called with a valid device context.
unsafe fn draw_overlay_section(
    hdc: HDC,
    client_rect: &RECT,
    bar_top: i32,
    overlay_opacity: u8,
) {
    unsafe {
        let width = client_rect.right - client_rect.left;

        // Calculate layout positions
        // Full layout: |pad|pct|ico|left_bar|gap|right_bar|ico|pct|pad|
        let content_width = width - (OSD_PADDING * 2);
        let total_bar_width =
            content_width - (PERCENT_TEXT_WIDTH * 2) - (ICON_WIDTH * 2) - BAR_GAP;
        let single_bar_width = total_bar_width / 2;

        // Left bar starts after: pad + pct + ico
        let bar_left = OSD_PADDING + PERCENT_TEXT_WIDTH + ICON_WIDTH;

        // Draw the bar background
        let bg_rect = RECT {
            left: bar_left,
            top: bar_top,
            right: bar_left + single_bar_width,
            bottom: bar_top + BAR_HEIGHT,
        };
        let bg_brush = CreateSolidBrush(COLORREF(BAR_BACKGROUND_COLOR));
        FillRect(hdc, &raw const bg_rect, bg_brush);
        let _ = DeleteObject(bg_brush);

        // Draw filled portion (fills from right to left)
        let fill_width =
            i32::try_from(i64::from(single_bar_width) * i64::from(overlay_opacity) / 100)
                .unwrap_or(0);
        if fill_width > 0 {
            let fill_rect = RECT {
                left: bar_left + single_bar_width - fill_width, // Start from right side
                top: bar_top,
                right: bar_left + single_bar_width,
                bottom: bar_top + BAR_HEIGHT,
            };
            let fill_brush = CreateSolidBrush(COLORREF(OVERLAY_FILL_COLOR));
            FillRect(hdc, &raw const fill_rect, fill_brush);
            let _ = DeleteObject(fill_brush);
        }

        // Percentage text position: far left after padding
        draw_percentage_text(hdc, OSD_PADDING, bar_top, overlay_opacity);

        // Icon position: left of bar with small padding
        let icon_x = OSD_PADDING + PERCENT_TEXT_WIDTH;
        draw_icon(hdc, icon_x, bar_top, ICON_OVERLAY);
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
/// The returned `HFONT` must be deleted with `DeleteObject` when no longer needed.
unsafe fn create_icon_font() -> HFONT {
    unsafe {
        CreateFontW(
            FONT_SIZE,                 // Height
            0,                         // Width (0 = auto)
            0,                         // Escapement
            0,                         // Orientation
            FW_NORMAL.0.cast_signed(), // Weight
            0,                         // Italic
            0,                         // Underline
            0,                         // StrikeOut
            u32::from(DEFAULT_CHARSET.0),
            u32::from(OUT_DEFAULT_PRECIS.0),
            u32::from(CLIP_DEFAULT_PRECIS.0),
            0, // Quality (DEFAULT_QUALITY)
            u32::from(DEFAULT_PITCH.0 | FF_DONTCARE.0),
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
        FillRect(hdc, &raw const bg_rect, bg_brush);
        let _ = DeleteObject(bg_brush);

        // Draw filled portion
        let fill_width = i32::try_from(i64::from(width) * i64::from(percent) / 100).unwrap_or(0);
        if fill_width > 0 {
            let fill_rect = RECT {
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
            let fill_brush = CreateSolidBrush(COLORREF(fill_color));
            FillRect(hdc, &raw const fill_rect, fill_brush);
            let _ = DeleteObject(fill_brush);
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
        let text = format!("{percent}%");
        let wide_text: Vec<u16> = text.encode_utf16().collect();

        // Center text vertically with the bar
        let text_y = y + (BAR_HEIGHT - FONT_SIZE).saturating_div(2);
        TextOutW(hdc, x, text_y, &wide_text);

        // Cleanup
        SelectObject(hdc, old_font);
        let _ = DeleteObject(font);
    }
}

/// Draws the error message centered in the error row at the bottom of the OSD.
///
/// The error row occupies the bottom `ERROR_ROW_HEIGHT` pixels of the expanded window.
/// This function should only be called when the window is in expanded (error) mode.
///
/// # Safety
///
/// Must be called with a valid device context.
unsafe fn draw_error_message(hdc: HDC, client_rect: &RECT, message: &str) {
    unsafe {
        // Create font
        let font = create_osd_font();
        let old_font = SelectObject(hdc, font);

        // Set text properties - use red color for error visibility
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(ERROR_TEXT_COLOR));

        // Convert message to wide string
        let wide_text: Vec<u16> = message.encode_utf16().collect();

        // Calculate position - centered horizontally, in footer area at bottom
        let width = client_rect.right - client_rect.left;
        // Approximate text width (~7 pixels per character at this font size)
        let approx_char_width = 7;
        let approx_text_width = i32::try_from(wide_text.len()).unwrap_or(0) * approx_char_width;
        let x = (width - approx_text_width) / 2;
        // Position in error row area: bottom of window minus row height, centered vertically
        let y = client_rect.bottom - ERROR_ROW_HEIGHT + (ERROR_ROW_HEIGHT - FONT_SIZE) / 2;

        TextOutW(hdc, x, y, &wide_text);

        // Cleanup
        SelectObject(hdc, old_font);
        let _ = DeleteObject(font);
    }
}

/// Creates the font used for OSD text.
///
/// # Safety
///
/// The returned `HFONT` must be deleted with `DeleteObject` when no longer needed.
unsafe fn create_osd_font() -> HFONT {
    unsafe {
        CreateFontW(
            FONT_SIZE,                 // Height
            0,                         // Width (0 = auto)
            0,                         // Escapement
            0,                         // Orientation
            FW_NORMAL.0.cast_signed(), // Weight
            0,                         // Italic
            0,                         // Underline
            0,                         // StrikeOut
            u32::from(DEFAULT_CHARSET.0),
            u32::from(OUT_DEFAULT_PRECIS.0),
            u32::from(CLIP_DEFAULT_PRECIS.0),
            0, // Quality (DEFAULT_QUALITY)
            u32::from(DEFAULT_PITCH.0 | FF_DONTCARE.0),
            w!("Segoe UI"), // Font face
        )
    }
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

/// Registers the window class for the OSD window if not already registered.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if `GetModuleHandleW` or `RegisterClassExW` fails.
pub fn ensure_osd_class_registered() -> Result<PCWSTR> {
    REGISTER_CLASS_ONCE
        .get_or_init(|| {
            unsafe {
                let hinstance = GetModuleHandleW(None).map_err(|e| {
                    BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
                })?;

                // A gray brush for the default background.
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
            0, // Size will be set by position_osd_window()
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
pub fn position_osd_window(hwnd: HWND, hmonitor: HMONITOR, with_error: bool) -> Result<()> {
    let mut mi = MONITORINFO {
        cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).unwrap_or(0),
        ..Default::default()
    };

    let height = if with_error {
        OSD_HEIGHT_WITH_ERROR
    } else {
        OSD_HEIGHT
    };

    unsafe {
        if !GetMonitorInfoW(hmonitor, &raw mut mi).as_bool() {
            return Err(last_error_as_brightness_error("GetMonitorInfoW"));
        }

        let rect = mi.rcMonitor;
        let monitor_width = rect.right - rect.left;

        // Calculate center-x and bottom-y position
        let x = rect.left + (monitor_width - OSD_WIDTH) / 2;
        let y = rect.bottom - height - OSD_BOTTOM_MARGIN;

        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            OSD_WIDTH,
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
pub fn set_osd_opacity(hwnd: HWND, opacity: f32) -> Result<()> {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let alpha = (opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
    unsafe {
        SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA).map_err(|e| {
            BrightnessError::windows_api("SetLayeredWindowAttributes", e.code().0.cast_unsigned())
        })?;
    }
    Ok(())
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
    pub fn show(&mut self, hmonitor: HMONITOR, state: &MonitorState) -> Result<()> {
        position_osd_window(self.hwnd.as_raw(), hmonitor, false)?;
        update_osd_state(state, false);

        unsafe {
            let _ = InvalidateRect(self.hwnd.as_raw(), None, true);
            let _ = ShowWindow(self.hwnd.as_raw(), SW_SHOW);
            self.reset_timer();
        }

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
    pub fn show_error(&mut self, hmonitor: HMONITOR, state: &MonitorState) -> Result<()> {
        position_osd_window(self.hwnd.as_raw(), hmonitor, true)?;
        update_osd_state(state, true);

        unsafe {
            let _ = InvalidateRect(self.hwnd.as_raw(), None, true);
            let _ = ShowWindow(self.hwnd.as_raw(), SW_SHOW);
            self.reset_timer();
        }

        Ok(())
    }

    /// Hides the OSD window immediately.
    ///
    /// # Errors
    ///
    /// This method is currently infallible but returns `Result` for consistency with Win32 APIs.
    pub fn hide(&self) -> Result<()> {
        unsafe {
            let _ = ShowWindow(self.hwnd.as_raw(), SW_HIDE);
        }
        Ok(())
    }

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
    pub fn update(&mut self, state: &MonitorState) -> Result<()> {
        update_osd_state(state, false);
        // Resize to compact height (in case we were showing an error before)
        resize_osd_window(self.hwnd.as_raw(), false)?;
        unsafe {
            // Invalidate the entire window to trigger WM_PAINT
            let _ = InvalidateRect(self.hwnd.as_raw(), None, true);
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
    pub fn update_error(&mut self, state: &MonitorState) -> Result<()> {
        update_osd_state(state, true);
        // Resize to expanded height to show error message
        resize_osd_window(self.hwnd.as_raw(), true)?;
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

    /// Returns `true` if the OSD window is currently visible.
    #[must_use]
    pub fn is_visible(&self) -> bool {
        unsafe {
            use windows::Win32::UI::WindowsAndMessaging::IsWindowVisible;
            IsWindowVisible(self.hwnd.as_raw()).as_bool()
        }
    }

    /// Returns the raw window handle for advanced operations.
    #[must_use]
    pub fn hwnd(&self) -> HWND {
        self.hwnd.as_raw()
    }
}
