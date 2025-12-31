//! DDC/CI communication implementation for Windows.
//!
//! This module handles the low-level communication with monitors using the
//! Windows Monitor Configuration API.

use windows::core::PCWSTR;
use windows::Win32::Devices::DeviceAndDriverInstallation::{
    SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo, SetupDiGetClassDevsW,
    SetupDiGetDeviceInstanceIdW, SetupDiOpenDevRegKey, DICS_FLAG_GLOBAL, DIGCF_PRESENT,
    DIGCF_PROFILE, DIREG_DEV, GUID_DEVCLASS_MONITOR, HDEVINFO, SP_DEVINFO_DATA,
};
use windows::Win32::Devices::Display::{
    CapabilitiesRequestAndCapabilitiesReply, DestroyPhysicalMonitors, EnumDisplayDevicesW,
    GetCapabilitiesStringLength, GetNumberOfPhysicalMonitorsFromHMONITOR,
    GetPhysicalMonitorsFromHMONITOR, DISPLAY_DEVICEW, PHYSICAL_MONITOR,
};
use windows::Win32::Foundation::{
    BOOL, ERROR_SUCCESS, HANDLE, HWND, LPARAM, RECT, WIN32_ERROR,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
};
use windows::Win32::System::Registry::{RegCloseKey, RegQueryValueExW, HKEY, KEY_READ};

use crate::core::state::MonitorId;
use crate::error::{BrightnessError, Result};
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
pub struct PhysicalMonitor {
    // The raw Windows structure containing the handle and description.
    inner: PHYSICAL_MONITOR,
}

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
        GetNumberOfPhysicalMonitorsFromHMONITOR(hmonitor, &mut count).map_err(|e| {
            BrightnessError::windows_api(
                "GetNumberOfPhysicalMonitorsFromHMONITOR",
                e.code().0 as u32,
            )
        })?;
    }

    if count == 0 {
        return Ok(Vec::new());
    }

    let mut physical_monitors = vec![PHYSICAL_MONITOR::default(); count as usize];

    unsafe {
        GetPhysicalMonitorsFromHMONITOR(hmonitor, &mut physical_monitors).map_err(|e| {
            BrightnessError::windows_api("GetPhysicalMonitorsFromHMONITOR", e.code().0 as u32)
        })?;
    }

    // Wrap them in RAII structs
    Ok(physical_monitors
        .into_iter()
        .map(|inner| PhysicalMonitor { inner })
        .collect())
}

/// Retrieves the DDC/CI capabilities string from a physical monitor.
///
/// The capabilities string is an ASCII string containing information about the
/// monitor's supported VCP codes, model name, and other features.
///
/// # Errors
///
/// Returns a `WindowsApi` error if the capabilities cannot be read.
pub fn get_capabilities_string(monitor: &PhysicalMonitor) -> Result<String> {
    let mut length = 0;

    unsafe {
        if GetCapabilitiesStringLength(monitor.handle(), &mut length) == 0 {
            return Err(last_error_as_brightness_error("GetCapabilitiesStringLength"));
        }
    }

    if length == 0 {
        return Ok(String::new());
    }

    // The length includes the null terminator.
    let mut buffer = vec![0u8; length as usize];

    unsafe {
        if CapabilitiesRequestAndCapabilitiesReply(monitor.handle(), &mut buffer) == 0 {
            return Err(last_error_as_brightness_error(
                "CapabilitiesRequestAndCapabilitiesReply",
            ));
        }
    }

    // Convert to String, stripping the null terminator and any garbage
    let s = String::from_utf8_lossy(&buffer)
        .trim_matches(char::from(0))
        .to_string();

    Ok(s)
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

// ─────────────────────────────────────────────────────────────────────────────
// EDID & Monitor ID Implementation
// ─────────────────────────────────────────────────────────────────────────────

/// Retrieves the MonitorId for a given HMONITOR by reading and parsing the EDID.
pub fn get_monitor_id(hmonitor: HMONITOR) -> Result<MonitorId> {
    let edid = get_edid_from_hmonitor(hmonitor)?;
    parse_edid(&edid).ok_or_else(|| {
        BrightnessError::ddc_communication("Unknown", "Failed to parse EDID or EDID is invalid")
    })
}

/// Reads the EDID binary data for a given HMONITOR using SetupAPI.
fn get_edid_from_hmonitor(hmonitor: HMONITOR) -> Result<Vec<u8>> {
    // 1. Get Monitor Device Name (e.g. \\.\DISPLAY1)
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;

    unsafe {
        if GetMonitorInfoW(hmonitor, &mut mi as *mut _ as *mut _).as_bool() == false {
            return Err(last_error_as_brightness_error("GetMonitorInfoW"));
        }
    }

    // 2. Get Monitor Device ID (Instance ID)
    let mut dd = DISPLAY_DEVICEW::default();
    dd.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;

    // We need to call EnumDisplayDevices with the device name from MonitorInfo
    unsafe {
        if EnumDisplayDevicesW(PCWSTR(mi.szDevice.as_ptr()), 0, &mut dd, 0).as_bool() == false {
            return Err(last_error_as_brightness_error("EnumDisplayDevicesW"));
        }
    }

    // The DeviceID in DISPLAY_DEVICEW is actually the Instance ID for monitors
    let target_instance_id = String::from_utf16_lossy(&dd.DeviceID)
        .trim_matches(char::from(0))
        .to_string();

    if target_instance_id.is_empty() {
        return Err(BrightnessError::ddc_communication(
            "Unknown",
            "Could not determine monitor instance ID",
        ));
    }

    // 3. Find the device in SetupAPI and read EDID
    find_edid_by_instance_id(&target_instance_id)
}

/// Searches for a monitor with the matching Instance ID in the SetupAPI device list
/// and reads its EDID from the registry.
fn find_edid_by_instance_id(target_instance_id: &str) -> Result<Vec<u8>> {
    let hdevinfo = unsafe {
        SetupDiGetClassDevsW(
            Some(&GUID_DEVCLASS_MONITOR),
            None,
            HWND::default(),
            DIGCF_PRESENT | DIGCF_PROFILE,
        )
    }?;

    if hdevinfo.0 == -1 {
        return Err(last_error_as_brightness_error("SetupDiGetClassDevsW"));
    }

    // RAII for HDEVINFO
    struct SafeDevInfo(HDEVINFO);
    impl Drop for SafeDevInfo {
        fn drop(&mut self) {
            unsafe {
                let _ = SetupDiDestroyDeviceInfoList(self.0);
            }
        }
    }
    let _safe_devinfo = SafeDevInfo(hdevinfo);

    let mut index = 0;
    let mut devinfo_data = SP_DEVINFO_DATA::default();
    devinfo_data.cbSize = std::mem::size_of::<SP_DEVINFO_DATA>() as u32;

    while unsafe { SetupDiEnumDeviceInfo(hdevinfo, index, &mut devinfo_data).as_bool() } {
        index += 1;

        // Get Instance ID
        let mut buffer = [0u16; 256];
        let mut required_size = 0;
        unsafe {
            if SetupDiGetDeviceInstanceIdW(
                hdevinfo,
                &devinfo_data,
                Some(&mut buffer),
                Some(&mut required_size),
            )
            .as_bool()
            {
                let instance_id = String::from_utf16_lossy(&buffer[..required_size as usize - 1]); // -1 for null

                if instance_id.eq_ignore_ascii_case(target_instance_id) {
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

/// Reads the "EDID" value from the device's registry key.
fn read_edid_from_registry(hdevinfo: HDEVINFO, devinfo_data: &SP_DEVINFO_DATA) -> Result<Vec<u8>> {
    let hkey = unsafe {
        SetupDiOpenDevRegKey(
            hdevinfo,
            devinfo_data,
            DICS_FLAG_GLOBAL,
            0,
            DIREG_DEV,
            KEY_READ,
        )
    };

    if hkey.0 == 0 || hkey.0 == -1 {
        return Err(last_error_as_brightness_error("SetupDiOpenDevRegKey"));
    }

    // RAII for HKEY
    struct SafeHKey(HKEY);
    impl Drop for SafeHKey {
        fn drop(&mut self) {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
    let _safe_hkey = SafeHKey(hkey);

    // Read "EDID" value
    let value_name = windows::core::w!("EDID");
    let mut data_type = 0;
    let mut data_len = 0;

    unsafe {
        // First call to get size
        let _ = RegQueryValueExW(
            hkey,
            value_name,
            None,
            Some(&mut data_type),
            None,
            Some(&mut data_len),
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
            Some(&mut data_type),
            Some(buffer.as_mut_ptr()),
            Some(&mut data_len),
        );

        if result == ERROR_SUCCESS {
            Ok(buffer)
        } else {
            Err(BrightnessError::windows_api(
                "RegQueryValueExW",
                result.0,
            ))
        }
    }
}

/// Parses basic information from EDID binary data.
fn parse_edid(edid: &[u8]) -> Option<MonitorId> {
    if edid.len() < 128 {
        return None;
    }

    // Manufacturer ID (bytes 8-9)
    // Encoded as 5 bits per character (A=1, Z=26)
    let mfg_id = u16::from_be_bytes([edid[8], edid[9]]);
    let char1 = ((mfg_id >> 10) & 0x1F) as u8 + b'A' - 1;
    let char2 = ((mfg_id >> 5) & 0x1F) as u8 + b'A' - 1;
    let char3 = (mfg_id & 0x1F) as u8 + b'A' - 1;
    let manufacturer = String::from_utf8_lossy(&[char1, char2, char3]).to_string();

    // Model Name and Serial Number from Descriptors (bytes 54-125)
    let mut model_name = String::new();
    let mut serial_number = None;

    for i in 0..4 {
        let offset = 54 + i * 18;
        if offset + 18 > edid.len() {
            break;
        }
        let desc = &edid[offset..offset + 18];

        // Check for string descriptors (Flag: 00 00 00 xx 00)
        if desc[0] == 0 && desc[1] == 0 && desc[2] == 0 && desc[4] == 0 {
            let tag = desc[3];
            if tag == 0xFC {
                // Model Name
                model_name = parse_descriptor_string(&desc[5..]);
            } else if tag == 0xFF {
                // Serial Number
                serial_number = Some(parse_descriptor_string(&desc[5..]));
            }
        }
    }

    if model_name.is_empty() {
        model_name = "Generic Monitor".to_string();
    }

    Some(MonitorId::new(manufacturer, model_name, serial_number))
}

/// Helper to parse a string from an EDID descriptor block.
/// Strings are terminated by 0x0A (newline) or end of block.
fn parse_descriptor_string(bytes: &[u8]) -> String {
    let len = bytes.iter().position(|&b| b == 0x0A).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..len]).trim().to_string()
}
