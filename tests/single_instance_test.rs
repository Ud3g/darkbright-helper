//! Integration test for the single-instance guard.
//!
//! Windows-only: exercises the real session-local named-mutex behaviour.
//! Excluded on other hosts because the guard is Win32 FFI.

#[cfg(windows)]
#[test]
fn second_acquire_reports_already_running() {
    use darkbright_helper::platform::windows::single_instance::{InstanceLock, acquire};

    let first = acquire().expect("acquire must not error on the happy path");
    let second = acquire().expect("acquire must not error on the happy path");

    // Assert only when THIS test holds the sole instance: then `first` owns a
    // live handle to the named object, so the second acquire must observe it.
    // If another instance (e.g. the real app running on the dev machine) already
    // held the name, `first` is `AlreadyRunning` and holds no handle, so the
    // second call's outcome is environment-dependent and nothing deterministic
    // can be asserted. On CI (no app running) `first` is always `Acquired`, so
    // the assertion always runs there.
    if let InstanceLock::Acquired(_) = &first {
        assert!(
            matches!(second, InstanceLock::AlreadyRunning),
            "while this test holds the only instance, a second acquire must report AlreadyRunning"
        );
    }

    // Keep `first` alive until after the check so its handle is not dropped early.
    drop(first);
}
