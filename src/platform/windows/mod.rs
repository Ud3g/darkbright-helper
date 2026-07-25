//! Windows-specific platform implementations.
//!
//! This module provides Windows implementations for:
//! - DDC/CI monitor communication
//! - Global hotkey registration
//! - Dimming overlay windows
//! - On-screen display (OSD)

use windows::Win32::Foundation::{HANDLE, HWND, POINT};
use windows::Win32::Graphics::Gdi::{HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, GetCursorPos, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MESSAGEBOX_STYLE,
    MessageBoxW,
};

use crate::error::{BrightnessError, Result};

pub mod ddc;
pub mod ddc_worker;
pub mod hotkey;
pub mod osd;
mod osd_render;
pub mod overlay;
pub mod power;
pub mod single_instance;
pub mod tray;
pub mod usage;

// Re-export commonly used types
pub use ddc_worker::{DdcSupervisor, DdcWorker, RespawnOutcome};
pub use power::PowerEventListener;
pub use single_instance::{InstanceLock, SingleInstance};
pub use tray::{TrayIcon, TrayStatusHandle};
pub use usage::UsageWindow;

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
        GetCursorPos(&raw mut cursor_pos).map_err(|e| {
            BrightnessError::windows_api("GetCursorPos", e.code().0.cast_unsigned())
        })?;

        // MONITOR_DEFAULTTONEAREST ensures we always get a handle,
        // even if the point is outside all monitors.
        let hmonitor = MonitorFromPoint(cursor_pos, MONITOR_DEFAULTTONEAREST);
        if hmonitor.is_invalid() {
            return Err(BrightnessError::windows_api("MonitorFromPoint", 0));
        }

        Ok(hmonitor)
    }
}

/// Resolves the monitor under the cursor via Win32 (`GetCursorPos`,
/// `MonitorFromPoint`) and identifies monitors from EDID.
#[derive(Debug, Clone, Copy)]
pub struct CursorLocator;

impl crate::core::controller::MonitorLocator for CursorLocator {
    fn monitor_under_cursor(&self) -> Result<crate::core::controller::MonitorHandle> {
        get_monitor_under_cursor()
            .map(|h| crate::core::controller::MonitorHandle(hmonitor_to_isize(h)))
    }
    fn resolve_id(
        &self,
        handle: crate::core::controller::MonitorHandle,
    ) -> Result<crate::core::state::MonitorId> {
        ddc::get_monitor_id(hmonitor_from_isize(handle.0))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Handle ↔ isize Seam
// ─────────────────────────────────────────────────────────────────────────────
// `core/` carries monitor/window handles as plain `isize` (`MonitorHandle`,
// `TrayStatusHandle`) to stay platform-free and `Send`. Win32 handles are
// pointers, so every crossing of that seam converts here — keeping the
// int↔pointer casts (and their lints) in one place.

/// Rebuilds an `HMONITOR` from the `isize` form that crosses the `core` seam.
#[must_use]
pub(crate) fn hmonitor_from_isize(value: isize) -> HMONITOR {
    HMONITOR(std::ptr::with_exposed_provenance_mut(value.cast_unsigned()))
}

/// Flattens an `HMONITOR` into the `isize` form that crosses the `core` seam.
#[must_use]
pub(crate) fn hmonitor_to_isize(handle: HMONITOR) -> isize {
    handle.0.expose_provenance().cast_signed()
}

/// Rebuilds an `HWND` from the `isize` form stored in `TrayStatusHandle`.
#[must_use]
pub(crate) fn hwnd_from_isize(value: isize) -> HWND {
    HWND(std::ptr::with_exposed_provenance_mut(value.cast_unsigned()))
}

/// Flattens an `HWND` into the `isize` form stored in `TrayStatusHandle`.
#[must_use]
pub(crate) fn hwnd_to_isize(handle: HWND) -> isize {
    handle.0.expose_provenance().cast_signed()
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
        !self.hwnd.is_invalid()
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
        !self.handle.is_invalid()
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
// Message Box Helper
// ─────────────────────────────────────────────────────────────────────────────

/// Shows a message box with the given caption, text, and style.
///
/// Blocks until the user dismisses the dialog.
fn show_message_box(title: &str, message: &str, style: MESSAGEBOX_STYLE) {
    let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let message_wide: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: We pass valid null-terminated wide strings.
    unsafe {
        MessageBoxW(
            None,
            windows::core::PCWSTR(message_wide.as_ptr()),
            windows::core::PCWSTR(title_wide.as_ptr()),
            style,
        );
    }
}

/// Shows an error message box (red error icon) to the user.
///
/// This is a blocking call that waits for the user to dismiss the dialog.
///
/// # Arguments
///
/// * `title` - The message box title.
/// * `message` - The error message to display.
pub fn show_error_message_box(title: &str, message: &str) {
    show_message_box(title, message, MB_OK | MB_ICONERROR);
}

/// Shows an informational message box (info icon) to the user.
///
/// Use this for normal notices — situations that are expected, not errors.
/// This is a blocking call that waits for the user to dismiss the dialog.
///
/// # Arguments
///
/// * `title` - The message box title.
/// * `message` - The message to display.
pub fn show_info_message_box(title: &str, message: &str) {
    show_message_box(title, message, MB_OK | MB_ICONINFORMATION);
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
        let hwnd = HWND::default();
        let safe = SafeHwnd::new_borrowed(hwnd);
        let raw = safe.into_raw();
        assert_eq!(raw, hwnd);
        // No drop called, no crash
    }
}
