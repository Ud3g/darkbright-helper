//! Windows-specific platform implementations.
//!
//! This module provides Windows implementations for:
//! - DDC/CI monitor communication
//! - Global hotkey registration
//! - Dimming overlay windows
//! - On-screen display (OSD)

use windows::Win32::Foundation::{HANDLE, HWND, POINT};
use windows::Win32::Graphics::Gdi::{HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::UI::Controls::SS_LEFT;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, GetCursorPos, GetSystemMetrics, IsWindow,
    MB_ICONERROR, MB_OK, MessageBoxW, RegisterClassExW, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW,
    SetForegroundWindow, ShowWindow, WM_CLOSE, WM_CREATE, WM_CTLCOLORSTATIC, WM_DESTROY,
    WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_POPUP, WS_SYSMENU, WS_VISIBLE,
};

use std::sync::OnceLock;

use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{GetStockObject, HBRUSH, WHITE_BRUSH};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;

use crate::error::{BrightnessError, Result};

pub mod ddc;
pub mod ddc_worker;
pub mod hotkey;
pub mod osd;
pub mod overlay;
pub mod power;
pub mod tray;

// Re-export commonly used types
pub use ddc_worker::DdcWorker;
pub use power::PowerEventListener;
pub use tray::TrayIcon;

// ─────────────────────────────────────────────────────────────────────────────
// Monitor Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the handle of the monitor under the current mouse cursor position.
///
/// Uses `GetCursorPos` and `MonitorFromPoint`. If the cursor position cannot
/// be determined, returns the primary monitor (or the nearest one).
///
/// # Errors
///
/// Returns an error if the Windows API call fails.
pub fn get_monitor_under_cursor() -> Result<HMONITOR> {
    let mut cursor_pos = POINT::default();
    unsafe {
        // SAFETY: We pass a valid raw pointer to a POINT struct.
        GetCursorPos(&raw mut cursor_pos).to_brightness_result("GetCursorPos")?;

        // MONITOR_DEFAULTTONEAREST ensures we always get a handle,
        // even if the point is outside all monitors.
        let hmonitor = MonitorFromPoint(cursor_pos, MONITOR_DEFAULTTONEAREST);
        if hmonitor.0 == 0 {
            return Err(BrightnessError::windows_api("MonitorFromPoint", 0));
        }

        Ok(hmonitor)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Error Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the last Windows error code as a `u32`.
///
/// This uses `std::io::Error::last_os_error()` which internally calls
/// the Windows `GetLastError()` API.
#[inline]
#[must_use]
pub fn get_last_error_code() -> u32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .unwrap_or(0)
        .cast_unsigned()
}

/// Converts the last Windows error into a `BrightnessError::WindowsApi`.
///
/// # Arguments
///
/// * `function` - Name of the Windows API function that failed
///
/// # Returns
///
/// A `BrightnessError` initialized with the function name and last error code.
#[must_use]
pub fn last_error_as_brightness_error(function: impl Into<String>) -> BrightnessError {
    BrightnessError::windows_api(function, get_last_error_code())
}

/// Checks if the last Windows error indicates success (`ERROR_SUCCESS` = 0).
#[inline]
#[must_use]
pub fn last_error_is_success() -> bool {
    get_last_error_code() == 0
}

// ─────────────────────────────────────────────────────────────────────────────
// RAII Handle Wrappers
// ─────────────────────────────────────────────────────────────────────────────

/// RAII wrapper for a Windows `HWND` (window handle).
///
/// Automatically calls `DestroyWindow` when dropped.
#[derive(Debug)]
pub struct SafeHwnd {
    hwnd: HWND,
    /// If false, the handle is borrowed and won't be destroyed on drop.
    owned: bool,
}

impl SafeHwnd {
    /// Creates a new owned `SafeHwnd` that will be destroyed on drop.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `hwnd` is a valid window handle
    /// that should be destroyed when this wrapper is dropped.
    #[must_use]
    pub const unsafe fn new_owned(hwnd: HWND) -> Self {
        Self { hwnd, owned: true }
    }

    /// Creates a borrowed `SafeHwnd` that will NOT be destroyed on drop.
    ///
    /// Use this for window handles that are managed elsewhere.
    #[must_use]
    pub const fn new_borrowed(hwnd: HWND) -> Self {
        Self { hwnd, owned: false }
    }

    /// Returns the raw `HWND`.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> HWND {
        self.hwnd
    }

    /// Returns true if this handle is valid (non-zero).
    #[inline]
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.hwnd.0 != 0
    }

    /// Consumes the wrapper and returns the raw handle without destroying it.
    ///
    /// The caller becomes responsible for destroying the window.
    #[must_use]
    pub fn into_raw(self) -> HWND {
        let hwnd = self.hwnd;
        std::mem::forget(self);
        hwnd
    }
}

impl Drop for SafeHwnd {
    fn drop(&mut self) {
        if self.owned && self.is_valid() {
            // SAFETY: We own this handle and it's valid.
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// RAII wrapper for a generic Windows `HANDLE`.
///
/// Automatically calls `CloseHandle` when dropped.
#[derive(Debug)]
pub struct SafeHandle {
    handle: HANDLE,
}

impl SafeHandle {
    /// Creates a new `SafeHandle`.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `handle` is a valid handle
    /// that should be closed with `CloseHandle` when dropped.
    #[must_use]
    pub const unsafe fn new(handle: HANDLE) -> Self {
        Self { handle }
    }

    /// Returns the raw `HANDLE`.
    #[inline]
    #[must_use]
    pub const fn as_raw(&self) -> HANDLE {
        self.handle
    }

    /// Returns true if this handle is valid (non-zero and not `INVALID_HANDLE_VALUE`).
    #[inline]
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.handle.0 != 0 && self.handle.0 != -1
    }

    /// Consumes the wrapper and returns the raw handle without closing it.
    ///
    /// The caller becomes responsible for closing the handle.
    #[must_use]
    pub fn into_raw(self) -> HANDLE {
        let handle = self.handle;
        std::mem::forget(self);
        handle
    }
}

impl Drop for SafeHandle {
    fn drop(&mut self) {
        if self.is_valid() {
            // SAFETY: We own this handle and it's valid.
            unsafe {
                use windows::Win32::Foundation::CloseHandle;
                let _ = CloseHandle(self.handle);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Result Conversion Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Extension trait for converting Windows API results to `BrightnessError`.
pub trait WindowsResultExt<T> {
    /// Converts a Windows result to a `Result<T, BrightnessError>`.
    ///
    /// # Arguments
    ///
    /// * `function` - Name of the Windows API function for error messages
    ///
    /// # Errors
    ///
    /// Returns `BrightnessError::WindowsApi` if the original result is an `Err`.
    fn to_brightness_result(self, function: &str) -> Result<T>;
}

impl<T> WindowsResultExt<T> for windows::core::Result<T> {
    fn to_brightness_result(self, function: &str) -> Result<T> {
        self.map_err(|e| BrightnessError::windows_api(function, e.code().0.cast_unsigned()))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Message Box Helper
// ─────────────────────────────────────────────────────────────────────────────

/// Shows an error message box to the user.
///
/// This is a blocking call that waits for the user to dismiss the dialog.
///
/// # Arguments
///
/// * `title` - The message box title.
/// * `message` - The error message to display.
pub fn show_error_message_box(title: &str, message: &str) {
    let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let message_wide: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: We pass valid null-terminated wide strings.
    unsafe {
        MessageBoxW(
            HWND::default(),
            windows::core::PCWSTR(message_wide.as_ptr()),
            windows::core::PCWSTR(title_wide.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Usage Window
// ─────────────────────────────────────────────────────────────────────────────

/// Window class name for the usage window.
const USAGE_WINDOW_CLASS: &str = "BrightnessControlUsageWindow";

/// Usage window dimensions (in pixels, will be DPI-scaled at creation).
const USAGE_WINDOW_WIDTH: i32 = 350;
const USAGE_WINDOW_HEIGHT: i32 = 180;
const USAGE_TEXT_MARGIN: i32 = 20;

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
                    hbrBackground: HBRUSH(unsafe { GetStockObject(WHITE_BRUSH) }.0),
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
        .map_err(|e| BrightnessError::OverlayCreation(format!("Usage window class registration failed: {e}")))?;

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

        // Calculate centered position
        let (x, y) = unsafe {
            let screen_width = GetSystemMetrics(SM_CXSCREEN);
            let screen_height = GetSystemMetrics(SM_CYSCREEN);
            (
                (screen_width - USAGE_WINDOW_WIDTH) / 2,
                (screen_height - USAGE_WINDOW_HEIGHT) / 2,
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
                WS_POPUP | WS_CAPTION | WS_SYSMENU,
                x,
                y,
                USAGE_WINDOW_WIDTH,
                USAGE_WINDOW_HEIGHT,
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
            "How to adjust brightness:\r\n\r\n\
             1. Move your mouse cursor to the monitor\r\n\
                you want to adjust\r\n\r\n\
             2. Press {} to increase brightness\r\n\
                or {} to decrease brightness",
            hotkey_up, hotkey_down
        );

        let text_wide: Vec<u16> = usage_text.encode_utf16().chain(std::iter::once(0)).collect();

        unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| {
                BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
            })?;

            let static_class: Vec<u16> = "STATIC".encode_utf16().chain(std::iter::once(0)).collect();

            let _static_hwnd = CreateWindowExW(
                windows::Win32::UI::WindowsAndMessaging::WINDOW_EX_STYLE(0),
                windows::core::PCWSTR(static_class.as_ptr()),
                windows::core::PCWSTR(text_wide.as_ptr()),
                WS_CHILD | WS_VISIBLE | SS_LEFT,
                USAGE_TEXT_MARGIN,
                USAGE_TEXT_MARGIN,
                USAGE_WINDOW_WIDTH - (USAGE_TEXT_MARGIN * 2),
                USAGE_WINDOW_HEIGHT - (USAGE_TEXT_MARGIN * 2),
                hwnd,
                None,
                hinstance,
                None,
            );

            // Show the window
            ShowWindow(hwnd, SW_SHOW);
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

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_last_error_returns_value() {
        // Just verify it doesn't panic
        let _ = get_last_error_code();
    }

    #[test]
    fn test_safe_hwnd_null_is_invalid() {
        let hwnd = SafeHwnd::new_borrowed(HWND::default());
        assert!(!hwnd.is_valid());
    }

    #[test]
    fn test_safe_handle_null_is_invalid() {
        // SAFETY: We're creating from a null handle for testing
        let handle = unsafe { SafeHandle::new(HANDLE::default()) };
        assert!(!handle.is_valid());
    }

    #[test]
    fn test_into_raw_prevents_drop() {
        let hwnd = HWND(0);
        let safe = SafeHwnd::new_borrowed(hwnd);
        let raw = safe.into_raw();
        assert_eq!(raw, hwnd);
        // No drop called, no crash
    }
}
