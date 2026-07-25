//! DDC/CI communication implementation for Windows.
//!
//! This module handles the low-level communication with monitors using the
//! Windows Monitor Configuration API.

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DICS_FLAG_GLOBAL, DIGCF_PRESENT, DIGCF_PROFILE, DIREG_DEV, GUID_DEVCLASS_MONITOR, HDEVINFO,
    SP_DEVINFO_DATA, SPDRP_DRIVER, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
    SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW, SetupDiOpenDevRegKey,
};
use windows::Win32::Devices::Display::{
    DestroyPhysicalMonitors, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, GetVCPFeatureAndVCPFeatureReply, PHYSICAL_MONITOR,
    SetVCPFeature,
};
use windows::Win32::Foundation::{HANDLE, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    DISPLAY_DEVICEW, EnumDisplayDevicesW, EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR,
    MONITORINFOEXW,
};
use windows::Win32::System::Registry::{
    HKEY, KEY_READ, REG_VALUE_TYPE, RegCloseKey, RegQueryValueExW,
};
use windows::core::BOOL;
use windows::core::PCWSTR;

use std::thread;
use std::time::Duration;

use crate::core::edid::parse_edid;
use crate::core::state::MonitorId;
use crate::error::{BrightnessError, Result};
use crate::platform::windows::last_error_as_brightness_error;

/// Maximum number of attempts for a DDC operation (the initial try plus 2 retries).
const DDC_MAX_ATTEMPTS: u32 = 3;
/// Delay between retries in milliseconds.
/// 40ms is chosen as a safe default for slower monitor controllers.
const DDC_RETRY_DELAY_MS: u64 = 40;

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
            None,
            None,
            Some(monitor_enum_proc),
            LPARAM(&raw mut monitors as isize),
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
pub struct PhysicalMonitor {
    // The raw Windows structure containing the handle and description.
    inner: PHYSICAL_MONITOR,
}

// SAFETY: A DDC physical-monitor handle is a process-scoped object with no
// thread affinity — any thread in this process may use or destroy it, so
// transferring ownership across threads cannot invalidate it. `Sync` is
// deliberately NOT implemented: access stays single-threaded (the DDC worker
// owns all instances). Since windows-rs 0.58, handle types implement neither
// `Send` nor `Sync` by design and callers who know their threading model opt
// in by wrapping — see microsoft/windows-rs#3093 and microsoft/windows-rs#3169.
unsafe impl Send for PhysicalMonitor {}

impl std::fmt::Debug for PhysicalMonitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let handle = self.inner.hPhysicalMonitor;
        f.debug_struct("PhysicalMonitor")
            .field("handle", &handle)
            .finish()
    }
}

impl PhysicalMonitor {
    /// Returns the raw handle to the physical monitor.
    #[must_use]
    pub fn handle(&self) -> HANDLE {
        self.inner.hPhysicalMonitor
    }
}

impl Drop for PhysicalMonitor {
    fn drop(&mut self) {
        unsafe {
            // DestroyPhysicalMonitors takes a slice in windows-rs 0.52+
            let _ = DestroyPhysicalMonitors(std::slice::from_ref(&self.inner));
        }
    }
}

/// Gets the physical monitors associated with a monitor handle (`HMONITOR`).
///
/// A single `HMONITOR` (logical monitor) can map to multiple physical monitors
/// (e.g., in daisy-chain configurations), though usually it's 1:1.
///
/// # Errors
///
/// Returns a `WindowsApi` error if the physical monitors cannot be retrieved.
pub fn get_physical_monitors(hmonitor: HMONITOR) -> Result<Vec<PhysicalMonitor>> {
    let mut count = 0;

    unsafe {
        GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &raw mut count).map_err(|e| {
            BrightnessError::windows_api(
                "GetNumberOfPhysicalMonitorsFromHMONITOR",
                e.code().0.cast_unsigned(),
            )
        })?;
    }

    if count == 0 {
        return Ok(Vec::new());
    }

    let mut physical_monitors = vec![PHYSICAL_MONITOR::default(); count as usize];

    unsafe {
        GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut physical_monitors).map_err(|e| {
            BrightnessError::windows_api(
                "GetPhysicalMonitorsFromHMONITOR",
                e.code().0.cast_unsigned(),
            )
        })?;
    }

    // Wrap them in RAII structs
    Ok(physical_monitors
        .into_iter()
        .map(|inner| PhysicalMonitor { inner })
        .collect())
}

/// Runs a DDC operation up to `DDC_MAX_ATTEMPTS` times (the initial try plus retries),
/// sleeping `DDC_RETRY_DELAY_MS` between attempts. `monitor` names the target
/// in the retry warning (multi-monitor logs are ambiguous without it).
fn retry_ddc_op<T>(monitor: &str, mut op: impl FnMut() -> Result<T>) -> Result<T> {
    let mut attempts = 0;
    loop {
        match op() {
            Ok(val) => return Ok(val),
            Err(e) => {
                attempts += 1;
                if attempts >= DDC_MAX_ATTEMPTS {
                    return Err(e);
                }
                log::warn!(
                    monitor:% = monitor,
                    attempt = attempts,
                    max_attempts = DDC_MAX_ATTEMPTS,
                    retry_delay_ms = DDC_RETRY_DELAY_MS,
                    error:% = e;
                    "DDC operation failed, retrying"
                );
                thread::sleep(Duration::from_millis(DDC_RETRY_DELAY_MS));
            }
        }
    }
}

/// Retrieves the current value and maximum value of a VCP feature.
///
/// # Arguments
///
/// * `monitor` - The physical monitor handle.
/// * `monitor_label` - Display name used in retry log lines.
/// * `vcp_code` - The VCP code to query (e.g., 0x10 for brightness).
///
/// # Returns
///
/// Returns a tuple containing `(current_value, maximum_value)`.
///
/// # Errors
///
/// Returns a `WindowsApi` error if the VCP feature cannot be read.
pub fn get_vcp_feature(
    monitor: &PhysicalMonitor,
    monitor_label: &str,
    vcp_code: u8,
) -> Result<(u32, u32)> {
    retry_ddc_op(monitor_label, || {
        let mut current_value = 0;
        let mut max_value = 0;

        unsafe {
            if GetVCPFeatureAndVCPFeatureReply(
                monitor.handle(),
                vcp_code,
                None,
                &raw mut current_value,
                Some(&raw mut max_value),
            ) != 0
            {
                Ok((current_value, max_value))
            } else {
                Err(last_error_as_brightness_error(
                    "GetVCPFeatureAndVCPFeatureReply",
                ))
            }
        }
    })
}

/// Sets the value of a VCP feature.
///
/// # Arguments
///
/// * `monitor` - The physical monitor handle.
/// * `monitor_label` - Display name used in retry log lines.
/// * `vcp_code` - The VCP code to set (e.g., 0x10 for brightness).
/// * `value` - The new value to set.
///
/// # Errors
///
/// Returns a `WindowsApi` error if the VCP feature cannot be set.
pub fn set_vcp_feature(
    monitor: &PhysicalMonitor,
    monitor_label: &str,
    vcp_code: u8,
    value: u32,
) -> Result<()> {
    retry_ddc_op(monitor_label, || unsafe {
        if SetVCPFeature(monitor.handle(), vcp_code, value) != 0 {
            Ok(())
        } else {
            Err(last_error_as_brightness_error("SetVCPFeature"))
        }
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// DdcMonitor Struct
// ─────────────────────────────────────────────────────────────────────────────

/// High-level wrapper for a physical monitor with DDC/CI capabilities.
///
/// Combines the physical handle, monitor ID, and brightness caching.
#[derive(Debug)]
pub struct DdcMonitor {
    handle: PhysicalMonitor,
    id: MonitorId,
    cached_brightness: Option<u32>,
}

impl DdcMonitor {
    /// Creates a new `DdcMonitor` instance.
    #[must_use]
    pub fn new(handle: PhysicalMonitor, id: MonitorId) -> Self {
        Self {
            handle,
            id,
            cached_brightness: None,
        }
    }

    /// Returns the monitor's unique identifier.
    #[must_use]
    pub fn id(&self) -> &MonitorId {
        &self.id
    }

    /// Returns the cached brightness value, if available.
    #[must_use]
    pub fn cached_brightness(&self) -> Option<u32> {
        self.cached_brightness
    }

    /// Reads the current brightness from the monitor via DDC/CI.
    ///
    /// Updates the cached brightness value on success.
    ///
    /// # Errors
    ///
    /// Returns a `WindowsApi` error if the brightness cannot be read.
    pub fn get_brightness(&mut self) -> Result<u32> {
        let (current, _) = get_vcp_feature(&self.handle, &self.id.base_display_name(), 0x10)?;
        self.cached_brightness = Some(current);
        Ok(current)
    }

    /// Sets the brightness of the monitor via DDC/CI.
    ///
    /// Updates the cached brightness value on success.
    ///
    /// # Errors
    ///
    /// Returns a `WindowsApi` error if the brightness cannot be set.
    pub fn set_brightness(&mut self, value: u32) -> Result<()> {
        set_vcp_feature(&self.handle, &self.id.base_display_name(), 0x10, value)?;
        self.cached_brightness = Some(value);
        Ok(())
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// EDID & Monitor ID Implementation
// ─────────────────────────────────────────────────────────────────────────────

/// Retrieves the `MonitorId` for a given `HMONITOR` by reading and parsing the EDID.
///
/// # Errors
///
/// Returns an error if the EDID cannot be read or parsed.
pub fn get_monitor_id(hmonitor: HMONITOR) -> Result<MonitorId> {
    let edid = get_edid_from_hmonitor(hmonitor)?;
    parse_edid(&edid).ok_or_else(|| {
        BrightnessError::ddc_communication("Unknown", "Failed to parse EDID or EDID is invalid")
    })
}

/// Reads the EDID binary data for a given `HMONITOR` using `SetupAPI`.
///
/// # Errors
///
/// Returns a `WindowsApi` error if the EDID cannot be retrieved.
fn get_edid_from_hmonitor(hmonitor: HMONITOR) -> Result<Vec<u8>> {
    // 1. Get Monitor Device Name (e.g. \\.\DISPLAY1)
    let mut mi = MONITORINFOEXW {
        monitorInfo: windows::Win32::Graphics::Gdi::MONITORINFO {
            cbSize: u32::try_from(std::mem::size_of::<MONITORINFOEXW>()).unwrap_or(0),
            ..Default::default()
        },
        ..Default::default()
    };

    unsafe {
        if !GetMonitorInfoW(hmonitor, (&raw mut mi).cast()).as_bool() {
            return Err(last_error_as_brightness_error("GetMonitorInfoW"));
        }
    }

    // 2. Get Monitor Device ID (Instance ID)
    let mut dd = DISPLAY_DEVICEW {
        cb: u32::try_from(std::mem::size_of::<DISPLAY_DEVICEW>()).unwrap_or(0),
        ..Default::default()
    };

    // We need to call EnumDisplayDevices with the device name from MonitorInfo
    unsafe {
        if !EnumDisplayDevicesW(PCWSTR(mi.szDevice.as_ptr()), 0, &raw mut dd, 0).as_bool() {
            return Err(last_error_as_brightness_error("EnumDisplayDevicesW"));
        }
    }

    // The DeviceKey in DISPLAY_DEVICEW contains the registry path to the driver key
    // Format: \Registry\Machine\System\CurrentControlSet\Control\Class\{GUID}\XXXX
    let device_key = String::from_utf16_lossy(&dd.DeviceKey)
        .trim_matches(char::from(0))
        .to_string();

    // Extract the driver key part: {GUID}\XXXX
    let target_driver_key = if let Some(start) = device_key.find('{') {
        device_key[start..].to_string()
    } else {
        log::warn!(device_key:% = device_key; "DeviceKey does not contain GUID, using fallback");
        // Fallback: try using DeviceID as before, though it likely fails
        String::from_utf16_lossy(&dd.DeviceID)
            .trim_matches(char::from(0))
            .to_string()
    };

    log::debug!(driver_key:% = target_driver_key; "Looking for EDID");

    if target_driver_key.is_empty() {
        return Err(BrightnessError::ddc_communication(
            "Unknown",
            "Could not determine monitor driver key",
        ));
    }

    // 3. Find the device in SetupAPI and read EDID
    find_edid_by_driver_key(&target_driver_key)
}

/// RAII for `HDEVINFO`
struct SafeDevInfo(HDEVINFO);
impl Drop for SafeDevInfo {
    fn drop(&mut self) {
        unsafe {
            let _ = SetupDiDestroyDeviceInfoList(self.0);
        }
    }
}

/// Searches for a monitor with the matching Driver Key in the `SetupAPI` device list
/// and reads its EDID from the registry.
///
/// # Errors
///
/// Returns an error if the monitor cannot be found in `SetupAPI`.
fn find_edid_by_driver_key(target_driver_key: &str) -> Result<Vec<u8>> {
    let hdevinfo = unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVCLASS_MONITOR),
            None,
            None,
            DIGCF_PRESENT | DIGCF_PROFILE,
        )
    }
    .map_err(|e| {
        BrightnessError::windows_api("SetupDiGetClassDevsW", e.code().0.cast_unsigned())
    })?;

    let _safe_devinfo = SafeDevInfo(hdevinfo);

    let mut index = 0;
    let mut devinfo_data = SP_DEVINFO_DATA {
        cbSize: u32::try_from(std::mem::size_of::<SP_DEVINFO_DATA>()).unwrap_or(0),
        ..Default::default()
    };

    // SetupDiEnumDeviceInfo returns Result<()> in windows 0.52+
    while unsafe { SetupDiEnumDeviceInfo(hdevinfo, index, &raw mut devinfo_data).is_ok() } {
        index += 1;

        // Get Driver Key (SPDRP_DRIVER)
        // Use a u16 buffer for WCHAR alignment
        let mut buffer = [0u16; 256];
        let mut required_size = 0;
        let mut property_type = 0;

        let success = unsafe {
            let buffer_u8 = std::slice::from_raw_parts_mut(
                buffer.as_mut_ptr().cast::<u8>(),
                std::mem::size_of_val(&buffer),
            );

            SetupDiGetDeviceRegistryPropertyW(
                hdevinfo,
                &raw const devinfo_data,
                SPDRP_DRIVER,
                Some(&raw mut property_type),
                Some(buffer_u8),
                Some(&raw mut required_size),
            )
            .is_ok()
        };

        if success {
            // Buffer contains WCHAR string
            let len = required_size as usize / 2;
            if len > 0 {
                // Trim null terminator
                let driver_key = String::from_utf16_lossy(&buffer[..len.saturating_sub(1)]);

                log::trace!(driver_key:% = driver_key; "Checking device driver key");

                if driver_key.eq_ignore_ascii_case(target_driver_key) {
                    // Found it! Read EDID from registry.
                    return read_edid_from_registry(hdevinfo, &devinfo_data);
                }
            }
        }
    }

    Err(BrightnessError::ddc_communication(
        "Unknown",
        "Monitor EDID not found in registry",
    ))
}

/// RAII for `HKEY`
struct SafeHKey(HKEY);
impl Drop for SafeHKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

/// Reads the "EDID" value from the device's registry key.
///
/// # Errors
///
/// Returns an error if the registry key cannot be opened or read.
fn read_edid_from_registry(hdevinfo: HDEVINFO, devinfo_data: &SP_DEVINFO_DATA) -> Result<Vec<u8>> {
    let hkey = unsafe {
        SetupDiOpenDevRegKey(
            hdevinfo,
            devinfo_data,
            DICS_FLAG_GLOBAL.0,
            0,
            DIREG_DEV,
            KEY_READ.0,
        )
    }
    .map_err(|e| {
        BrightnessError::windows_api("SetupDiOpenDevRegKey", e.code().0.cast_unsigned())
    })?;

    let _safe_hkey = SafeHKey(hkey);

    // Read "EDID" value
    let value_name = windows::core::w!("EDID");
    let mut data_type = REG_VALUE_TYPE::default();
    let mut data_len = 0;

    unsafe {
        // First call to get size
        let _ = RegQueryValueExW(
            hkey,
            value_name,
            None,
            Some(&raw mut data_type),
            None,
            Some(&raw mut data_len),
        );

        if data_len == 0 {
            return Err(BrightnessError::ddc_communication(
                "Unknown",
                "EDID registry value empty",
            ));
        }

        let mut buffer = vec![0u8; data_len as usize];
        let result = RegQueryValueExW(
            hkey,
            value_name,
            None,
            Some(&raw mut data_type),
            Some(buffer.as_mut_ptr()),
            Some(&raw mut data_len),
        );

        result.ok().map_err(|e| {
            BrightnessError::windows_api("RegQueryValueExW", e.code().0.cast_unsigned())
        })?;
        Ok(buffer)
    }
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

    /// Static assertion that `DdcMonitor` is `Send`.
    /// This is required for the DDC worker thread to own monitors.
    const _: () = {
        const fn assert_send<T: Send>() {}
        assert_send::<DdcMonitor>();
    };
}
