//! Implementation of the dimming overlay window for Windows.

use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, GetStockObject, BLACK_BRUSH, HBRUSH, HMONITOR, MONITORINFO,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassExW, SetLayeredWindowAttributes, SetWindowPos,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE, WNDCLASSEXW,
    WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::error::{BrightnessError, Result};
use super::{last_error_as_brightness_error, SafeHwnd};

/// The class name for the overlay window.
const OVERLAY_CLASS_NAME: PCWSTR = w!("DarkBrightOverlayClass");

/// Ensures the window class is registered exactly once.
static REGISTER_CLASS_ONCE: OnceLock<Result<()>> = OnceLock::new();

/// Window procedure for the overlay window.
///
/// Since the overlay is a passive visual element (click-through),
/// we delegate almost everything to the default window procedure.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Registers the window class for the overlay window if not already registered.
///
/// Returns the class name on success.
pub fn ensure_overlay_class_registered() -> Result<PCWSTR> {
    // Initialize the registration once.
    REGISTER_CLASS_ONCE
        .get_or_init(|| {
            unsafe {
                let hinstance = GetModuleHandleW(None).map_err(|e| {
                    BrightnessError::windows_api("GetModuleHandleW", e.code().0 as u32)
                })?;

                // Use the stock BLACK_BRUSH for the background.
                // This ensures the window is black by default, which is what we want for dimming.
                let black_brush = HBRUSH(GetStockObject(BLACK_BRUSH).0);

                let wnd_class = WNDCLASSEXW {
                    cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: hinstance.into(),
                    hCursor: Default::default(), // No cursor needed, or default arrow
                    hbrBackground: black_brush,
                    lpszClassName: OVERLAY_CLASS_NAME,
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
            // Clone the error since we can't move out of the OnceLock reference
            // BrightnessError doesn't implement Clone, so we reconstruct a generic one here
            // or rely on the fact that if it failed once, it fails always.
            BrightnessError::windows_api("ensure_overlay_class_registered", 0)
        })?;

    Ok(OVERLAY_CLASS_NAME)
}

/// Creates a new overlay window.
///
/// The window is created with the following attributes:
/// - Layered (for opacity control)
/// - Transparent (click-through)
/// - Tool window (hidden from taskbar)
/// - Topmost (always on top)
/// - Popup (no border/caption)
///
/// The window is initially hidden and has 0 size.
pub fn create_overlay_window() -> Result<SafeHwnd> {
    let class_name = ensure_overlay_class_registered()?;

    unsafe {
        // WS_EX_LAYERED: Allows transparency/opacity
        // WS_EX_TRANSPARENT: Click-through (events pass to window underneath)
        // WS_EX_TOOLWINDOW: Hides from taskbar and Alt-Tab
        // WS_EX_TOPMOST: Keeps window above others
        let ex_style = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;

        // WS_POPUP: No border, title bar, etc.
        let style = WS_POPUP;

        let hwnd = CreateWindowExW(
            ex_style,
            class_name,
            w!("DarkBrightOverlay"), // Window title (not visible)
            style,
            CW_USEDEFAULT, // x
            CW_USEDEFAULT, // y
            0,             // width (will be resized later)
            0,             // height
            None,          // Parent
            None,          // Menu
            GetModuleHandleW(None).unwrap_or_default(),
            None,          // lpParam
        );

        if hwnd.0 == 0 {
            return Err(last_error_as_brightness_error("CreateWindowExW"));
        }

        // Wrap in SafeHwnd for automatic cleanup
        Ok(SafeHwnd::new_owned(hwnd))
    }
}

/// Positions the overlay window to cover the specified monitor.
///
/// This function moves and resizes the window to match the monitor's bounds
/// and ensures it is topmost.
pub fn position_window_fullscreen(hwnd: HWND, hmonitor: HMONITOR) -> Result<()> {
    let mut mi = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };

    unsafe {
        if !GetMonitorInfoW(hmonitor, &mut mi).as_bool() {
            return Err(last_error_as_brightness_error("GetMonitorInfoW"));
        }

        let rect = mi.rcMonitor;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            rect.left,
            rect.top,
            width,
            height,
            SWP_NOACTIVATE,
        )
        .map_err(|e| BrightnessError::windows_api("SetWindowPos", e.code().0 as u32))?;
    }

    Ok(())
}

/// Sets the opacity of the overlay window.
///
/// # Arguments
///
/// * `hwnd` - The window handle.
/// * `opacity` - Opacity value from 0.0 (invisible) to 1.0 (fully opaque).
pub fn set_window_opacity(hwnd: HWND, opacity: f32) -> Result<()> {
    // Clamp opacity to 0.0 - 1.0
    let opacity = opacity.clamp(0.0, 1.0);

    // Convert to 0-255
    let alpha = (opacity * 255.0).round() as u8;

    unsafe {
        SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA).map_err(|e| {
            BrightnessError::windows_api("SetLayeredWindowAttributes", e.code().0 as u32)
        })?;
    }

    Ok(())
}
