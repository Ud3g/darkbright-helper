//! Implementation of the dimming overlay window for Windows.

use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, BLACK_BRUSH, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassExW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
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
    DefWindowProcW(hwnd, msg, wparam, lparam)
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
                    return Err(BrightnessError::windows_api(
                        "RegisterClassExW",
                        windows::Win32::Foundation::GetLastError().0,
                    ));
                }
            }
            Ok(())
        })
        .as_ref()
        .map_err(|e| {
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
