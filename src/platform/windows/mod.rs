//! Windows-specific platform implementations.
//!
//! This module provides Windows implementations for:
//! - DDC/CI monitor communication
//! - Global hotkey registration
//! - Dimming overlay windows
//! - On-screen display (OSD)

use windows::Win32::Foundation::{GetLastError, HANDLE, HWND, WIN32_ERROR};
use windows::Win32::UI::WindowsAndMessaging::DestroyWindow;

use crate::error::{BrightnessError, Result};

pub mod ddc;
pub mod hotkey;
pub mod osd;
pub mod overlay;

// ─────────────────────────────────────────────────────────────────────────────
// Error Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the last Windows error code.
///
/// This is a safe wrapper around `GetLastError()`.
#[inline]
pub fn get_last_error() -> WIN32_ERROR {
    // SAFETY: GetLastError has no preconditions and is always safe to call.
    unsafe { GetLastError() }
}

/// Returns the last Windows error code as a u32.
#[inline]
pub fn get_last_error_code() -> u32 {
    get_last_error().0
}

/// Converts the last Windows error into a `BrightnessError::WindowsApi`.
///
/// # Arguments
///
/// * `function` - Name of the Windows API function that failed
pub fn last_error_as_brightness_error(function: impl Into<String>) -> BrightnessError {
    BrightnessError::windows_api(function, get_last_error_code())
}

/// Checks if the last Windows error indicates success (ERROR_SUCCESS = 0).
#[inline]
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
    pub const unsafe fn new_owned(hwnd: HWND) -> Self {
        Self { hwnd, owned: true }
    }

    /// Creates a borrowed `SafeHwnd` that will NOT be destroyed on drop.
    ///
    /// Use this for window handles that are managed elsewhere.
    pub const fn new_borrowed(hwnd: HWND) -> Self {
        Self { hwnd, owned: false }
    }

    /// Returns the raw `HWND`.
    #[inline]
    pub const fn as_raw(&self) -> HWND {
        self.hwnd
    }

    /// Returns true if this handle is valid (non-null).
    #[inline]
    pub fn is_valid(&self) -> bool {
        !self.hwnd.0.is_null()
    }

    /// Consumes the wrapper and returns the raw handle without destroying it.
    ///
    /// The caller becomes responsible for destroying the window.
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
    pub const unsafe fn new(handle: HANDLE) -> Self {
        Self { handle }
    }

    /// Returns the raw `HANDLE`.
    #[inline]
    pub const fn as_raw(&self) -> HANDLE {
        self.handle
    }

    /// Returns true if this handle is valid (non-null and not INVALID_HANDLE_VALUE).
    #[inline]
    pub fn is_valid(&self) -> bool {
        !self.handle.0.is_null() && self.handle.0 as isize != -1
    }

    /// Consumes the wrapper and returns the raw handle without closing it.
    ///
    /// The caller becomes responsible for closing the handle.
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
    fn to_brightness_result(self, function: &str) -> Result<T>;
}

impl<T> WindowsResultExt<T> for windows::core::Result<T> {
    fn to_brightness_result(self, function: &str) -> Result<T> {
        self.map_err(|e| BrightnessError::windows_api(function, e.code().0 as u32))
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
        let _ = get_last_error();
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
        let hwnd = HWND(std::ptr::null_mut());
        let safe = SafeHwnd::new_borrowed(hwnd);
        let raw = safe.into_raw();
        assert_eq!(raw, hwnd);
        // No drop called, no crash
    }
}
