//! "Start with Windows" autostart, backed by the `HKCU` `Run` key.
//!
//! The registry is the single source of truth: nothing about autostart is
//! mirrored into `config.json`. That keeps the config schema unchanged and
//! avoids a second store that could drift from what Windows actually does.
//! The app ships as a portable zip and can move or be re-extracted anywhere,
//! so enabling always rewrites the `Run` value with the *current* exe path
//! rather than only when the value is missing — that unconditional rewrite
//! is what self-heals a stale entry left by an older extraction.
//!
//! Task Manager's Startup tab does not remove a disabled entry from `Run`;
//! it records the disabled state in a separate, undocumented
//! `StartupApproved\Run` key instead, and this module does not parse that
//! format. An entry Task Manager has disabled therefore still reads as
//! enabled here — a deliberate, documented trade-off, not an oversight.
//! Toggling off and back on does heal that state: `disable` removes `Run`
//! entirely, and `enable` clears the veto value when it rewrites `Run`.

use std::path::Path;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ, RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::{PCWSTR, w};

use crate::error::{BrightnessError, Result};

/// Value name under both the `Run` key and the `StartupApproved` veto key.
const VALUE_NAME: PCWSTR = w!("darkbright-helper");

/// Where Windows looks for per-user autostart entries.
const RUN_SUBKEY: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Run");

/// Where Task Manager records that a `Run` entry has been disabled. Deleting
/// the value here does not touch `Run` itself — it only clears the veto.
const STARTUP_APPROVED_SUBKEY: PCWSTR =
    w!(r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run");

/// Formats the `Run` value data for an executable path.
///
/// Quoted so a path containing spaces still names one file instead of being
/// split into separate command-line arguments — the shape every `Run`
/// consumer expects, and correct whether or not the path actually has a
/// space in it.
#[must_use]
fn format_run_value(path: &Path) -> String {
    format!("\"{}\"", path.display())
}

/// Deletes `VALUE_NAME` under `HKCU\subkey`, treating an already-absent
/// value or an already-absent key as success — there is nothing to remove
/// either way, so neither is a failure the caller needs to react to.
fn delete_value_ignoring_not_found(subkey: PCWSTR, function: &str) -> Result<()> {
    // SAFETY: `subkey` and `VALUE_NAME` are static NUL-terminated literals.
    let status = unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, subkey, VALUE_NAME) };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND || status == ERROR_PATH_NOT_FOUND {
        return Ok(());
    }
    Err(BrightnessError::windows_api(function, status.0))
}

/// True iff the `HKCU` `Run` value exists.
///
/// A Task-Manager-disabled entry still reads as enabled — see the module
/// docs for why. Toggling off then back on heals it.
#[must_use]
pub fn is_enabled() -> bool {
    // SAFETY: `RUN_SUBKEY` and `VALUE_NAME` are static NUL-terminated
    // literals. No output buffer is requested, so this only probes for the
    // value's existence and type rather than reading its data.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            VALUE_NAME,
            RRF_RT_REG_SZ,
            None,
            None,
            None,
        )
    };
    if status == ERROR_SUCCESS {
        return true;
    }
    // The value simply not being there is the normal "disabled" state and
    // not worth a log line; anything else (e.g. access denied) means the
    // query itself failed and "disabled" is a guess, not a fact.
    if status != ERROR_FILE_NOT_FOUND && status != ERROR_PATH_NOT_FOUND {
        log::warn!(status = status.0; "Failed to query autostart Run value; reporting disabled");
    }
    false
}

/// Writes the `Run` value with the quoted current exe path and deletes any
/// `StartupApproved` veto, so a Task-Manager-disabled entry re-enables.
///
/// # Errors
///
/// Returns an error if the current exe path cannot be determined, or if
/// either registry write fails.
pub fn enable() -> Result<()> {
    let exe_path = std::env::current_exe().map_err(|e| {
        BrightnessError::windows_api(
            "GetModuleFileNameW",
            e.raw_os_error().unwrap_or(0).cast_unsigned(),
        )
    })?;
    let value = format_run_value(&exe_path);
    let wide: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let cbdata = u32::try_from(std::mem::size_of_val(wide.as_slice())).unwrap_or(0);

    // SAFETY: `RUN_SUBKEY` and `VALUE_NAME` are static NUL-terminated
    // literals; `wide` is a NUL-terminated UTF-16 buffer whose byte length is
    // `cbdata`, and it is kept alive across the call.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            RUN_SUBKEY,
            VALUE_NAME,
            REG_SZ.0,
            Some(wide.as_ptr().cast()),
            cbdata,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(BrightnessError::windows_api("RegSetKeyValueW", status.0));
    }

    delete_value_ignoring_not_found(
        STARTUP_APPROVED_SUBKEY,
        "RegDeleteKeyValueW (StartupApproved)",
    )
}

/// Deletes the `Run` value. An already-absent value is success, not an
/// error — there is nothing left to disable.
///
/// # Errors
///
/// Returns an error if the registry delete fails for a reason other than the
/// value already being absent.
pub fn disable() -> Result<()> {
    delete_value_ignoring_not_found(RUN_SUBKEY, "RegDeleteKeyValueW (Run)")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_exe_path() {
        assert_eq!(
            format_run_value(Path::new(r"C:\a b\x.exe")),
            "\"C:\\a b\\x.exe\""
        );
    }

    #[test]
    fn quotes_a_path_without_spaces_the_same_way() {
        // `Run` never needs quoting when there is nothing to split on, but
        // always quoting is simpler than branching per path, and an
        // unnecessary pair of quotes around a single token is harmless —
        // Explorer strips them before launching the entry.
        assert_eq!(
            format_run_value(Path::new(r"C:\bin\x.exe")),
            "\"C:\\bin\\x.exe\""
        );
    }
}
