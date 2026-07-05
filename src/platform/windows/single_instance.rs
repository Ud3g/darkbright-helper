//! Single-instance guard via a session-local named mutex.
//!
//! At startup the process creates a named mutex; the mere *existence* of the
//! named object (reported through the last-error value, not through an error
//! return) signals that another instance is already running in this logon
//! session. Ownership is never taken — the object is a kernel-lifetime-managed
//! flag, so it is released automatically when the owning process exits for any
//! reason, including a crash.

use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_ALREADY_EXISTS};
use windows::Win32::System::Threading::CreateMutexW;
use windows::core::{PCWSTR, w};

use crate::error::{BrightnessError, Result};

use super::{SafeHandle, get_last_error_code};

/// Session-local name of the single-instance mutex.
///
/// The `Local\` prefix scopes the object to the current logon session, so each
/// user (and each RDP session) may run its own instance. This name is a stable
/// contract that a future autostart feature relies on.
const MUTEX_NAME: PCWSTR = w!("Local\\darkbright-helper-single-instance");

/// RAII guard holding the single-instance mutex *handle* for the process
/// lifetime.
///
/// This holds a handle to the named object, not mutex ownership (the object is
/// never acquired via a wait). Dropping it closes the handle; the kernel
/// releases the named object once no handle to it remains, freeing the name for
/// the next launch.
pub struct SingleInstance {
    // Held solely to keep the mutex handle open for the process lifetime; the
    // wrapped `SafeHandle` closes it on drop.
    #[allow(dead_code)]
    handle: SafeHandle,
}

/// Outcome of [`acquire`].
pub enum InstanceLock {
    /// This process is the first/only instance in the session. Hold the guard
    /// for the process lifetime.
    Acquired(SingleInstance),
    /// Another instance already holds the name, or owns it at a higher
    /// integrity level this process cannot open.
    AlreadyRunning,
}

/// Attempts to become the single instance for the current logon session.
///
/// Creates a session-local named mutex. Returns [`InstanceLock::AlreadyRunning`]
/// when the named object already exists (or exists but is owned by a
/// higher-integrity instance that cannot be opened); otherwise returns
/// [`InstanceLock::Acquired`] wrapping the RAII guard.
///
/// # Errors
///
/// Returns `Err` only on an unexpected `CreateMutexW` failure — any failure
/// other than access-denied, which is treated as already-running. "Already
/// running" is a normal outcome, not an error.
pub fn acquire() -> Result<InstanceLock> {
    // SAFETY: `MUTEX_NAME` is a valid static, null-terminated wide string. Null
    // security attributes and no initial ownership: the existence of the named
    // object is the only signal used.
    let created = unsafe { CreateMutexW(None, false, MUTEX_NAME) };

    // Read the thread-local last error as the very next operation, before any
    // allocation, logging, or further FFI can overwrite it. `MUTEX_NAME` is a
    // compile-time constant, so constructing the name argument allocated nothing
    // whose drop could clobber the error. Do not use the `windows` crate's
    // `GetLastError` binding here: it HRESULT-wraps the code and would never
    // compare equal to `ERROR_ALREADY_EXISTS`.
    let last_error = get_last_error_code();

    match created {
        Ok(handle) => {
            // Wrap the handle immediately so it is always closed on drop, in
            // both the acquired and already-running branches.
            // SAFETY: `CreateMutexW` returned success, so `handle` is a valid
            // handle we own and must close.
            let guard = SingleInstance {
                handle: unsafe { SafeHandle::new(handle) },
            };
            if last_error == ERROR_ALREADY_EXISTS.0 {
                // The named object already existed → this is a second instance.
                // Dropping `guard` closes our handle; the first instance's
                // handle keeps the name alive.
                Ok(InstanceLock::AlreadyRunning)
            } else {
                Ok(InstanceLock::Acquired(guard))
            }
        }
        Err(_) if last_error == ERROR_ACCESS_DENIED.0 => {
            // The name exists but is owned by a higher-integrity instance we
            // cannot open. Treat as already-running rather than starting a
            // duplicate.
            Ok(InstanceLock::AlreadyRunning)
        }
        Err(_) => Err(BrightnessError::windows_api("CreateMutexW", last_error)),
    }
}
