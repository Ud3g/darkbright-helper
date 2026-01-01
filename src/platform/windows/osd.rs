//! On-Screen Display (OSD) implementation for Windows.

use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, GRAY_BRUSH, HBRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassExW, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT,
    WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
};

use crate::error::{BrightnessError, Result};
use super::{last_error_as_brightness_error, SafeHwnd};

/// The class name for the OSD window.
const OSD_CLASS_NAME: PCWSTR = w!("DarkBrightOSDClass");

/// Ensures the window class is registered exactly once.
static REGISTER_CLASS_ONCE: OnceLock<Result<()>> = OnceLock::new();

/// Window procedure for the OSD window.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // In Phase 4 werden wir hier WM_PAINT für das Rendering implementieren.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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
                let background_brush = HBRUSH(GetStockObject(GRAY_BRUSH).0);

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
