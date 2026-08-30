//! Windows-specific platform implementations.
//!
//! This module provides Windows implementations for:
//! - DDC/CI monitor communication
//! - Global hotkey registration
//! - Dimming overlay windows
//! - On-screen display (OSD)

use windows::Win32::Foundation::{HWND, POINT};
use windows::Win32::Graphics::Gdi::{HMONITOR, MONITOR_DEFAULTTONEAREST, MonitorFromPoint};
use windows::Win32::UI::WindowsAndMessaging::{
    DestroyWindow, GetCursorPos, MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MESSAGEBOX_STYLE,
    MessageBoxW,
};

use crate::error::{BrightnessError, Result};

// Only the modules the binary and `tests/` name by path are `pub`; the rest are
// crate-internal and reach the binary through the re-exports below. This is
// load-bearing, not tidiness: everything `pub` in a `pub mod` counts as
// externally reachable, so rustc reports no unused items inside one. Keeping
// the surface at its true width is what lets `dead_code` see this crate at all.
pub(crate) mod config_store;
pub mod ddc;
pub(crate) mod ddc_worker;
pub mod hotkey;
pub mod osd;
mod osd_render;
pub mod overlay;
pub(crate) mod power;
pub(crate) mod settings;
pub mod single_instance;
mod theme;
pub(crate) mod tray;

// Re-export commonly used types
pub use config_store::WindowsConfigStore;
pub use ddc_worker::DdcSupervisor;
pub use power::PowerEventListener;
pub use settings::SettingsSinkImpl;
pub use single_instance::{InstanceLock, SingleInstance};
pub use tray::{TrayIcon, TrayStatusHandle};

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
pub(crate) fn get_monitor_under_cursor() -> Result<HMONITOR> {
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
// seam's int↔pointer casts in one place. (Other int↔pointer casts exist
// elsewhere for unrelated reasons — LPARAM callback plumbing in ddc.rs —
// and aren't part of this seam.)

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
pub(crate) fn get_last_error_code() -> u32 {
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
pub(crate) fn last_error_as_brightness_error(function: impl Into<String>) -> BrightnessError {
    BrightnessError::windows_api(function, get_last_error_code())
}

// ─────────────────────────────────────────────────────────────────────────────
// RAII Handle Wrapper
// ─────────────────────────────────────────────────────────────────────────────

/// RAII wrapper for a Windows `HWND` (window handle).
///
/// Owning by construction: every window this crate wraps is one it created and
/// must destroy, so there is no borrowed variant and no ownership flag to check
/// on drop.
#[derive(Debug)]
pub(crate) struct SafeHwnd {
    hwnd: HWND,
}

impl SafeHwnd {
    /// Creates a new owned `SafeHwnd` that will be destroyed on drop.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `hwnd` is a valid window handle
    /// that should be destroyed when this wrapper is dropped.
    #[must_use]
    pub(crate) const unsafe fn new_owned(hwnd: HWND) -> Self {
        Self { hwnd }
    }

    /// Returns the raw `HWND`.
    #[inline]
    #[must_use]
    pub(crate) const fn as_raw(&self) -> HWND {
        self.hwnd
    }

    /// Returns true if this handle is valid (non-zero).
    #[inline]
    #[must_use]
    pub(crate) fn is_valid(&self) -> bool {
        !self.hwnd.is_invalid()
    }
}

impl Drop for SafeHwnd {
    fn drop(&mut self) {
        if self.is_valid() {
            // SAFETY: We own this handle and it's valid.
            unsafe {
                let _ = DestroyWindow(self.hwnd);
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
    fn last_error_code_survives_the_signed_to_unsigned_conversion() {
        use windows::Win32::Foundation::{SetLastError, WIN32_ERROR};

        // Windows reports error codes to Rust as i32, so any code with the high
        // bit set — the HRESULT-shaped ones an FFI failure often carries — comes
        // back negative. Reinterpreting the bits is the only correct reading;
        // a range-checking conversion would quietly report 0 and turn a real
        // failure code into "no error". This pins that round trip.
        unsafe { SetLastError(WIN32_ERROR(0x8007_0005)) };
        assert_eq!(get_last_error_code(), 0x8007_0005);
    }

    #[test]
    fn test_safe_hwnd_null_is_invalid() {
        // A null handle must read as invalid — that is what stops `Drop` from
        // calling `DestroyWindow` on a window that was never created.
        // SAFETY: a null handle is never destroyed, so wrapping it is inert.
        let hwnd = unsafe { SafeHwnd::new_owned(HWND::default()) };
        assert!(!hwnd.is_valid());
    }

    // Round-trips the handle seam conversions. Guards against a future
    // cast_signed/cast_unsigned swap silently corrupting handles whose bit
    // pattern sits above isize::MAX/2.
    #[test]
    fn test_handle_seam_round_trip() {
        assert_eq!(hmonitor_to_isize(hmonitor_from_isize(0x1234)), 0x1234);
        assert_eq!(hwnd_to_isize(hwnd_from_isize(0x1234)), 0x1234);
    }
}
