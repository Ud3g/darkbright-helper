//! DDC/CI communication implementation for Windows.
//!
//! This module handles the low-level communication with monitors using the
//! Windows Monitor Configuration API.

use windows::Win32::Foundation::{BOOL, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, HDC, HMONITOR};

use crate::error::Result;
use crate::platform::windows::last_error_as_brightness_error;

/// Enumerates all active display monitors.
///
/// Returns a vector of `HMONITOR` handles for all currently active monitors.
/// These handles can be used to obtain physical monitor handles for DDC/CI.
///
/// # Errors
///
/// Returns a `WindowsApi` error if `EnumDisplayMonitors` fails.
pub fn enumerate_monitors() -> Result<Vec<HMONITOR>> {
    let mut monitors = Vec::new();

    // SAFETY: We pass a valid pointer to the monitors vector as LPARAM.
    // The callback function casts it back to &mut Vec<HMONITOR>.
    unsafe {
        if EnumDisplayMonitors(
            HDC::default(),
            None,
            Some(monitor_enum_proc),
            LPARAM(&mut monitors as *mut _ as isize),
        )
        .as_bool()
        {
            Ok(monitors)
        } else {
            Err(last_error_as_brightness_error("EnumDisplayMonitors"))
        }
    }
}

/// Callback for `EnumDisplayMonitors`.
unsafe extern "system" fn monitor_enum_proc(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let monitors = &mut *(lparam.0 as *mut Vec<HMONITOR>);
    monitors.push(hmonitor);
    BOOL::from(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_monitors() {
        // This test should pass on any Windows system, even headless (returns empty vec).
        let result = enumerate_monitors();
        assert!(result.is_ok());
    }
}
