//! DDC/CI communication implementation for Windows.
//!
//! This module handles the low-level communication with monitors using the
//! Windows Monitor Configuration API.

use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, PHYSICAL_MONITOR,
};
use windows::Win32::Foundation::{BOOL, HANDLE, LPARAM, RECT};
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
    // SAFETY: The caller ensures lparam is a valid pointer to Vec<HMONITOR>
    // and the vector outlives this callback.
    let monitors = unsafe { &mut *(lparam.0 as *mut Vec<HMONITOR>) };
    monitors.push(hmonitor);
    BOOL::from(true)
}

/// RAII wrapper for a physical monitor handle.
///
/// Ensures that `DestroyPhysicalMonitors` is called when the handle goes out of scope.
#[derive(Debug)]
pub struct PhysicalMonitor {
    // The raw Windows structure containing the handle and description.
    inner: PHYSICAL_MONITOR,
}

impl PhysicalMonitor {
    /// Returns the raw handle to the physical monitor.
    pub fn handle(&self) -> HANDLE {
        self.inner.hPhysicalMonitor
    }
}

impl Drop for PhysicalMonitor {
    fn drop(&mut self) {
        unsafe {
            // DestroyPhysicalMonitors takes an array, but passing a pointer to a single
            // struct with count 1 works for cleaning up individual handles.
            let _ = DestroyPhysicalMonitors(1, &self.inner);
        }
    }
}

/// Gets the physical monitors associated with a monitor handle (HMONITOR).
///
/// A single HMONITOR (logical monitor) can map to multiple physical monitors
/// (e.g., in daisy-chain configurations), though usually it's 1:1.
///
/// # Errors
///
/// Returns a `WindowsApi` error if the physical monitors cannot be retrieved.
pub fn get_physical_monitors(hmonitor: HMONITOR) -> Result<Vec<PhysicalMonitor>> {
    let mut count = 0;

    unsafe {
        if !GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count).as_bool() {
            return Err(last_error_as_brightness_error(
                "GetNumberOfPhysicalMonitorsFromHMONITOR",
            ));
        }
    }

    if count == 0 {
        return Ok(Vec::new());
    }

    let mut physical_monitors = vec![PHYSICAL_MONITOR::default(); count as usize];

    unsafe {
        if !GetPhysicalMonitorsFromHMONITOR(hmonitor, count, physical_monitors.as_mut_ptr())
            .as_bool()
        {
            return Err(last_error_as_brightness_error(
                "GetPhysicalMonitorsFromHMONITOR",
            ));
        }
    }

    // Wrap them in RAII structs
    Ok(physical_monitors
        .into_iter()
        .map(|inner| PhysicalMonitor { inner })
        .collect())
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
