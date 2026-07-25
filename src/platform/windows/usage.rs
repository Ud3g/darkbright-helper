//! Modeless usage-instructions window (opened from the tray menu).

use std::sync::OnceLock;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, HBRUSH, WHITE_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    AdjustWindowRect, BS_DEFPUSHBUTTON, CreateWindowExW, DefWindowProcW, DestroyWindow,
    GetSystemMetrics, HMENU, IsWindow, RegisterClassExW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW,
    SetForegroundWindow, ShowWindow, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_CTLCOLORSTATIC,
    WM_DESTROY, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_POPUP, WS_SYSMENU, WS_VISIBLE,
};

use super::{SafeHwnd, last_error_as_brightness_error};
use crate::error::{BrightnessError, Result};

/// Window class name for the usage window.
const USAGE_WINDOW_CLASS: &str = "BrightnessControlUsageWindow";

/// Usage window client area dimensions (in pixels).
/// These are the desired client area sizes; the actual window will be larger
/// to accommodate the title bar and borders.
const USAGE_CLIENT_WIDTH: i32 = 340;
const USAGE_CLIENT_HEIGHT: i32 = 150;
const USAGE_TEXT_MARGIN: i32 = 15;
const ID_OK_BTN: isize = 1001;
const BTN_WIDTH: i32 = 80;
const BTN_HEIGHT: i32 = 25;

/// Ensures the usage window class is registered exactly once.
static USAGE_CLASS_REGISTERED: OnceLock<Result<()>> = OnceLock::new();

/// Registers the window class for the usage window if not already registered.
fn ensure_usage_class_registered() -> Result<()> {
    USAGE_CLASS_REGISTERED
        .get_or_init(|| {
            unsafe {
                let hinstance = GetModuleHandleW(None).map_err(|e| {
                    BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
                })?;

                let class_name: Vec<u16> = USAGE_WINDOW_CLASS
                    .encode_utf16()
                    .chain(std::iter::once(0))
                    .collect();

                let wnd_class = WNDCLASSEXW {
                    cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(0),
                    lpfnWndProc: Some(usage_wnd_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: windows::core::PCWSTR(class_name.as_ptr()),
                    hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
                    ..Default::default()
                };

                if RegisterClassExW(&raw const wnd_class) == 0 {
                    return Err(last_error_as_brightness_error("RegisterClassExW"));
                }

                log::debug!("Usage window class registered");
            }
            Ok(())
        })
        .as_ref()
        .map_err(|e| {
            BrightnessError::OverlayCreation(format!("Usage window class registration failed: {e}"))
        })?;

    Ok(())
}

/// Window procedure for the usage window.
unsafe extern "system" fn usage_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                log::debug!("Usage window WM_CREATE");
                LRESULT(0)
            }
            WM_CTLCOLORSTATIC => {
                // Return white background brush for static controls
                LRESULT(GetStockObject(WHITE_BRUSH).0 as isize)
            }
            WM_COMMAND => {
                // Check if the OK button was clicked (low word of wparam is the ID)
                let id = (wparam.0 & 0xFFFF).cast_signed();
                if id == ID_OK_BTN {
                    let _ = DestroyWindow(hwnd);
                }
                LRESULT(0)
            }
            WM_CLOSE => {
                log::debug!("Usage window WM_CLOSE");
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                log::debug!("Usage window WM_DESTROY");
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// A modeless window displaying usage instructions.
///
/// Only one instance should exist at a time. The window can be closed
/// by clicking the X button or pressing Alt+F4.
#[derive(Debug)]
pub struct UsageWindow {
    hwnd: SafeHwnd,
}

impl UsageWindow {
    /// Creates and shows a new usage window with the given hotkey information.
    ///
    /// The window is positioned at the center of the primary monitor.
    ///
    /// # Arguments
    ///
    /// * `hotkey_up` - The configured hotkey string for brightness up.
    /// * `hotkey_down` - The configured hotkey string for brightness down.
    ///
    /// # Errors
    ///
    /// Returns `BrightnessError::WindowsApi` if window creation fails.
    // Sequential Win32 window and child-control creation is inherently long.
    #[allow(clippy::too_many_lines)]
    pub fn new(hotkey_up: &str, hotkey_down: &str) -> Result<Self> {
        ensure_usage_class_registered()?;

        let class_name: Vec<u16> = USAGE_WINDOW_CLASS
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        let title: Vec<u16> = "Brightness Control - Usage"
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        // Calculate window size from desired client area
        let window_style = WS_POPUP | WS_CAPTION | WS_SYSMENU;
        let (window_width, window_height) = unsafe {
            let mut rect = RECT {
                left: 0,
                top: 0,
                right: USAGE_CLIENT_WIDTH,
                bottom: USAGE_CLIENT_HEIGHT,
            };
            // AdjustWindowRect calculates the required window size for a given client area
            let _ = AdjustWindowRect(&raw mut rect, window_style, false);
            (rect.right - rect.left, rect.bottom - rect.top)
        };

        // Calculate centered position
        let (x, y) = unsafe {
            let screen_width = GetSystemMetrics(SM_CXSCREEN);
            let screen_height = GetSystemMetrics(SM_CYSCREEN);
            (
                (screen_width - window_width) / 2,
                (screen_height - window_height) / 2,
            )
        };

        let hwnd = unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| {
                BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
            })?;

            let hwnd = CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
                windows::core::PCWSTR(class_name.as_ptr()),
                windows::core::PCWSTR(title.as_ptr()),
                window_style,
                x,
                y,
                window_width,
                window_height,
                None,
                None,
                hinstance,
                None,
            );

            if hwnd.0 == 0 {
                return Err(last_error_as_brightness_error("CreateWindowExW"));
            }

            hwnd
        };

        // Create static text control with usage instructions
        let usage_text = format!(
            "1. Move mouse to desired monitor\r\n\
             2. Press {hotkey_up} (brighter) or {hotkey_down} (dimmer)"
        );

        let text_wide: Vec<u16> = usage_text
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();

        unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| {
                BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
            })?;

            let static_class: Vec<u16> =
                "STATIC".encode_utf16().chain(std::iter::once(0)).collect();

            // Text area takes remaining space above the button (using client area dimensions)
            let text_height = USAGE_CLIENT_HEIGHT - (USAGE_TEXT_MARGIN * 2) - BTN_HEIGHT - 10;

            let _static_hwnd = CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
                windows::core::PCWSTR(static_class.as_ptr()),
                windows::core::PCWSTR(text_wide.as_ptr()),
                WS_CHILD | WS_VISIBLE,
                USAGE_TEXT_MARGIN,
                USAGE_TEXT_MARGIN,
                USAGE_CLIENT_WIDTH - (USAGE_TEXT_MARGIN * 2),
                text_height,
                hwnd,
                None,
                hinstance,
                None,
            );

            let button_class: Vec<u16> =
                "BUTTON".encode_utf16().chain(std::iter::once(0)).collect();
            let button_text: Vec<u16> = "OK".encode_utf16().chain(std::iter::once(0)).collect();

            let btn_x = (USAGE_CLIENT_WIDTH - BTN_WIDTH) / 2;
            let btn_y = USAGE_CLIENT_HEIGHT - USAGE_TEXT_MARGIN - BTN_HEIGHT;

            let btn_hwnd = CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
                windows::core::PCWSTR(button_class.as_ptr()),
                windows::core::PCWSTR(button_text.as_ptr()),
                WS_CHILD
                    | WS_VISIBLE
                    | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                        BS_DEFPUSHBUTTON as u32,
                    ),
                btn_x,
                btn_y,
                BTN_WIDTH,
                BTN_HEIGHT,
                hwnd,
                HMENU(ID_OK_BTN),
                hinstance,
                None,
            );

            // Show the window and set focus
            ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);

            // Set focus to the OK button so Enter key dismisses the window
            if btn_hwnd.0 != 0 {
                SetFocus(btn_hwnd);
            }
        }

        log::debug!("Usage window created");

        Ok(Self {
            hwnd: unsafe { SafeHwnd::new_owned(hwnd) },
        })
    }

    /// Returns true if the window is still valid (not closed).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        if !self.hwnd.is_valid() {
            return false;
        }
        unsafe { IsWindow(self.hwnd.as_raw()).as_bool() }
    }

    /// Brings the window to the foreground if it exists.
    pub fn bring_to_front(&self) {
        if self.is_valid() {
            unsafe {
                let _ = SetForegroundWindow(self.hwnd.as_raw());
            }
            log::debug!("Usage window brought to front");
        }
    }
}
