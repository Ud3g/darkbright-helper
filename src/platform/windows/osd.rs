//! On-Screen Display (OSD) implementation for Windows.

use std::sync::OnceLock;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, GetStockObject, GRAY_BRUSH, HBRUSH, HMONITOR, MONITORINFO,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, RegisterClassExW, SetLayeredWindowAttributes, SetWindowPos,
    ShowWindow, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, HWND_TOPMOST, LWA_ALPHA, SWP_NOACTIVATE,
    SW_HIDE, SW_SHOW, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
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
        let _monitor_height = rect.bottom - rect.top;

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
}

impl OsdWindow {
    /// Creates a new OsdWindow.
    pub fn new(opacity: f32) -> Result<Self> {
        let hwnd = create_osd_window()?;
        set_osd_opacity(hwnd.as_raw(), opacity)?;

        Ok(Self { hwnd })
    }

    /// Shows the OSD for a specific monitor with the given state.
    pub fn show(&mut self, hmonitor: HMONITOR, _state: &MonitorState) -> Result<()> {
        position_osd_window(self.hwnd.as_raw(), hmonitor)?;

        unsafe {
            let _ = ShowWindow(self.hwnd.as_raw(), SW_SHOW);
            // TODO: Start auto-hide timer (Step #32)
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

    /// Triggers a redraw of the OSD.
    pub fn update(&mut self, _state: &MonitorState) -> Result<()> {
        // In späteren Schritten wird hier InvalidateRect aufgerufen
        Ok(())
    }
}
