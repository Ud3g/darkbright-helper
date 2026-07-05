# Single-Instance Guard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent a second copy of darkbright-helper from running in the same logon session; the second launch shows an "already running" info box and exits before spawning any worker, window, or hotkey.

**Architecture:** A session-local named mutex (`Local\darkbright-helper-single-instance`) is created at startup via `CreateMutexW`; its *existence* (reported through `GetLastError`) is the signal. A new `platform/windows/single_instance.rs` module exposes a safe `acquire()` returning an RAII guard (built on the existing `SafeHandle`). `main()` runs the check right after `init_logging()` and holds the guard for the process lifetime.

**Tech Stack:** Rust 2024, `windows` crate v0.52 (Win32 FFI), `log` (structured kv).

## Global Constraints

Copied verbatim from the spec and project conventions — every task implicitly includes these:

- **Platform:** Windows-only feature; all code is `windows`-crate FFI. Build/test target is Windows (this host is Windows; `cargo build`/`cargo test` run natively).
- **Toolchain:** Rust 2024 edition, MSRV 1.87. Local toolchain is stable 1.96.1; CI floats on `@stable`.
- **Lints:** `cargo clippy --all-targets -- -D warnings` must pass (clippy `all` + `pedantic` are warn-by-default). Before pushing, verify with `cargo +stable clippy --all-targets --locked -- -D warnings` (CI uses a newer clippy than an older local toolchain may have).
- **Formatting:** `cargo fmt -- --check` must pass.
- **No `as` casts:** use `u32::from` / `try_from` / `.cast_unsigned()`; the codebase's `get_last_error_code()` already returns `u32`.
- **FFI safety:** keep `unsafe` isolated behind safe wrappers, closest to point of use; wrap handles in RAII (reuse `SafeHandle`). Rust 2024 requires explicit `unsafe { }` blocks inside `unsafe fn`.
- **Docs:** all public items need `///` with `# Errors` where they return `Result` (clippy enforces). Backtick code identifiers.
- **Logging:** structured kv form — `log::info!(key:% = val; "message")`. Log at point of handling.
- **Comments:** no ephemeral planning labels (phase/step/finding IDs) — state rationale in self-contained domain terms.
- **Commits:** terse subject, ≤ ~50 words, **no** `Co-Authored-By`/AI trailer.
- **Mutex name is a stable contract** a future autostart feature relies on — do not change the string.

---

## File Structure

- **Create** `src/platform/windows/single_instance.rs` — the whole guard: `MUTEX_NAME` const, `SingleInstance` RAII guard, `InstanceLock` enum, `acquire()`. Sole responsibility: session-uniqueness via named mutex.
- **Create** `tests/single_instance_test.rs` — Windows-only integration test exercising real `acquire()` behaviour.
- **Modify** `Cargo.toml` — add two `windows` features.
- **Modify** `src/platform/windows/mod.rs` — declare + re-export the new module; add `show_info_message_box` (DRY refactor of the existing message-box helper).
- **Modify** `src/main.rs` — imports + the single-instance gate in `main()`.

---

## Task 1: `single_instance` module + automated guard test

**Files:**
- Create: `tests/single_instance_test.rs`
- Modify: `Cargo.toml` (features block, lines 9-23)
- Create: `src/platform/windows/single_instance.rs`
- Modify: `src/platform/windows/mod.rs:28-40` (module decl + re-export)

**Interfaces:**
- Produces:
  - `pub fn acquire() -> darkbright_helper::Result<InstanceLock>`
  - `pub enum InstanceLock { Acquired(SingleInstance), AlreadyRunning }`
  - `pub struct SingleInstance` (opaque RAII guard; hold it to stay the sole instance)
- Consumes (existing, from `super`): `SafeHandle` (`mod.rs:183`, `unsafe const fn new(HANDLE)`), `get_last_error_code() -> u32` (`mod.rs:81`), `BrightnessError::windows_api(impl Into<String>, u32)` (`error.rs:121`).

- [ ] **Step 1: Write the failing integration test**

Create `tests/single_instance_test.rs`:

```rust
//! Integration test for the single-instance guard.
//!
//! Windows-only: exercises the real session-local named-mutex behaviour.
//! Excluded on other hosts because the guard is Win32 FFI.

#[cfg(windows)]
#[test]
fn second_acquire_reports_already_running() {
    use darkbright_helper::platform::windows::single_instance::{acquire, InstanceLock};

    // First acquisition: `Acquired` when no other instance holds the name in
    // this session, or `AlreadyRunning` if the real app happens to be running
    // while the test executes. When `Acquired`, `first` holds a live handle to
    // the named object; when the app is running, the app holds the name. Either
    // way the named object exists for the duration of `first`.
    let first = acquire().expect("acquire must not error on the happy path");

    // A second acquisition while an instance holds the name must always observe
    // it, regardless of whether `first` was `Acquired` or `AlreadyRunning`.
    let second = acquire().expect("acquire must not error on the happy path");
    assert!(
        matches!(second, InstanceLock::AlreadyRunning),
        "a second acquire while an instance holds the name must report AlreadyRunning"
    );

    // Keep `first` alive until after the second check so its handle (when it
    // holds one) is not dropped early.
    drop(first);
}
```

- [ ] **Step 2: Run the test to verify it fails to compile**

Run: `cargo test --test single_instance_test`
Expected: FAIL — compile error, `unresolved import ... single_instance` / module `single_instance` not found (the module and `Cargo.toml` feature don't exist yet).

- [ ] **Step 3: Add the two required `windows` features**

In `Cargo.toml`, inside the `windows = { version = "0.52", features = [ ... ] }` list (lines 9-23), append two entries (the list has no ordering; `cargo fmt` does not format `Cargo.toml`):

```toml
    "Win32_System_Threading",
    "Win32_Security",
```

`Win32_System_Threading` provides `CreateMutexW`; `Win32_Security` is required because `CreateMutexW`'s `lpmutexattributes` parameter references `SECURITY_ATTRIBUTES` (verified: `CreateMutexW` is `#[cfg(all(feature = "Win32_Foundation", feature = "Win32_Security"))]` in windows 0.52.0).

- [ ] **Step 4: Create the module**

Create `src/platform/windows/single_instance.rs`:

```rust
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
```

- [ ] **Step 5: Wire the module into `platform/windows/mod.rs`**

Add the module declaration (keep the existing grouping; insert before `pub mod tray;` at `mod.rs:35`):

```rust
pub mod single_instance;
```

Add the re-export alongside the others (after `mod.rs:38-40`):

```rust
pub use single_instance::{InstanceLock, SingleInstance};
```

- [ ] **Step 6: Run the test to verify it passes**

Run: `cargo test --test single_instance_test`
Expected: PASS — `second_acquire_reports_already_running` succeeds.

- [ ] **Step 7: Run clippy and fmt**

Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.
Run: `cargo fmt -- --check`
Expected: clean (no diff).

- [ ] **Step 8: Commit**

```bash
git add Cargo.toml src/platform/windows/single_instance.rs src/platform/windows/mod.rs tests/single_instance_test.rs
git commit -m "feat: session-local single-instance mutex guard

Add platform/windows/single_instance with acquire() returning an RAII
guard; second acquire in a session reports AlreadyRunning. Integration
test covers the happy path. Adds Win32_System_Threading + Win32_Security."
```

---

## Task 2: `show_info_message_box` helper

**Files:**
- Modify: `src/platform/windows/mod.rs:12-18` (imports), `:272-285` (message-box helpers)

**Interfaces:**
- Produces: `pub fn show_info_message_box(title: &str, message: &str)` — an `MB_OK | MB_ICONINFORMATION` box (blocking), for normal notices.
- Consumes: existing `MessageBoxW`, `MB_OK`, `MB_ICONERROR` (already imported).

**Note:** This is pure Win32 message-box FFI with no automated test (consistent with the codebase's "thin FFI → verify manually"). Its visible behaviour is exercised by Task 3's manual verification matrix. Verification here is compile + clippy + fmt.

- [ ] **Step 1: Add the icon + style imports**

In `src/platform/windows/mod.rs`, extend the `windows::Win32::UI::WindowsAndMessaging` import group (lines 12-18) to also bring in `MB_ICONINFORMATION` and `MESSAGEBOX_STYLE`. After editing, that group's message-box entries read:

```rust
    ..., MB_ICONERROR, MB_ICONINFORMATION, MB_OK, MESSAGEBOX_STYLE, MessageBoxW, ...
```

(Keep the rest of the group and its existing alphabetical ordering intact; only add the two new identifiers in place.)

- [ ] **Step 2: Refactor the helper to share plumbing and add the info variant**

Replace the existing `show_error_message_box` function (`mod.rs:272-285`) with a private core plus two public wrappers:

```rust
/// Shows a message box with the given caption, text, and style.
///
/// Blocks until the user dismisses the dialog.
fn show_message_box(title: &str, message: &str, style: MESSAGEBOX_STYLE) {
    let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let message_wide: Vec<u16> = message.encode_utf16().chain(std::iter::once(0)).collect();

    // SAFETY: We pass valid null-terminated wide strings.
    unsafe {
        MessageBoxW(
            HWND::default(),
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
```

- [ ] **Step 3: Build, clippy, fmt**

Run: `cargo build`
Expected: compiles.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.
Run: `cargo fmt -- --check`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/platform/windows/mod.rs
git commit -m "feat: add show_info_message_box (info-icon variant)

Factor the message-box body into a shared helper; add an
MB_ICONINFORMATION variant for normal notices such as 'already running'."
```

---

## Task 3: Wire the single-instance gate into `main()`

**Files:**
- Modify: `src/main.rs:35` (imports), `:36-38` (imports), `:911-914` (the gate, after `init_logging()`)

**Interfaces:**
- Consumes: `single_instance::acquire()`, `InstanceLock`, `SingleInstance` (Task 1); `show_info_message_box` (Task 2).

**Note:** Verified manually — the second-instance behaviour is Win32-level and hardware/UI-observable. The matrix in Step 4 is the acceptance check.

- [ ] **Step 1: Update imports**

In `src/main.rs`, change the message-box import (line 35) from:

```rust
use darkbright_helper::platform::windows::show_error_message_box;
```

to:

```rust
use darkbright_helper::platform::windows::{show_error_message_box, show_info_message_box};
```

And add a new import (next to the other `platform::windows` imports, e.g. after line 38):

```rust
use darkbright_helper::platform::windows::single_instance::{self, InstanceLock, SingleInstance};
```

- [ ] **Step 2: Insert the gate after `init_logging()`**

In `fn main()`, immediately after `init_logging();` (`main.rs:911`) and before `let config = load_config();` (`main.rs:914`), insert:

```rust
    // Enforce a single instance per logon session before spawning any worker,
    // window, or hotkey. A second launch informs the user and exits, so it
    // leaves no duplicate tray icon, overlay, or failed hotkey registration.
    let _instance_guard: Option<SingleInstance> = match single_instance::acquire() {
        Ok(InstanceLock::Acquired(guard)) => Some(guard),
        Ok(InstanceLock::AlreadyRunning) => {
            log::info!("Another instance is already running; exiting");
            show_info_message_box(
                "darkbright-helper",
                "darkbright-helper is already running.",
            );
            return;
        }
        Err(e) => {
            // Fail open: an unexpected guard failure must not block the user's
            // only instance.
            log::error!(error:% = e; "Single-instance check failed; continuing without guard");
            None
        }
    };
```

The `_instance_guard` binding (named underscore, **not** bare `_`) must live to the end of `main` so the guard is held for the whole process; a bare `let _ =` would drop it immediately and defeat the guard.

- [ ] **Step 3: Build, clippy, fmt**

Run: `cargo build`
Expected: compiles.
Run: `cargo clippy --all-targets -- -D warnings`
Expected: no warnings.
Run: `cargo fmt -- --check`
Expected: clean.

- [ ] **Step 4: Manual verification matrix**

Build a runnable binary (`cargo build`) and verify each case, recording the observed result:

1. **Double-launch blocked:** run the exe (tray icon appears); run it again → an "already running" **info** box (info icon, not red) appears; that instance exits on OK; the first instance's tray icon, hotkeys, and OSD still work.
2. **No duplicate side effects:** during case 1, confirm no second tray icon and no overlay flash appear before the box.
3. **Clean release:** close the first instance normally, relaunch → starts normally (name released on drop).
4. **Crash release:** kill the first instance via Task Manager (no clean drop), relaunch → still starts normally (kernel released the object on process death; no stale-lock).
5. **Elevation mismatch (best-effort):** run the first instance elevated ("Run as administrator"), launch a second normally → the second shows "already running" and exits (exercises the `ERROR_ACCESS_DENIED` → `AlreadyRunning` path). Result can depend on the object's default security descriptor; note what you observe.
6. **Happy-path sanity:** with no prior instance, confirm via `RUST_LOG=debug` logs that the app proceeds as the first instance (guards against a last-error-clobber regression that would silently disable the guard).

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: gate startup on the single-instance guard

Run the single-instance check right after logging init, before any
worker/window/hotkey. A second instance shows an info box and exits;
an unexpected guard error fails open. Replaces the accidental
hotkey-registration guard on double-launch."
```

---

## Pre-push verification

Before pushing, run the CI-equivalent gate (CI clippy is newer than an older local toolchain):

```bash
cargo +stable fmt -- --check
cargo +stable clippy --all-targets --locked -- -D warnings
cargo +stable test --locked
```

Expected: all clean/pass. Then update `docs/architecture-review.md` finding #6 with a resolution note (matching the existing "✅ Resolved" convention used for findings #1–#4) — optional, as a follow-up doc commit.

---

## Self-Review

**Spec coverage:**
- Named mutex, existence-as-signal, `Local\` scope, `bInitialOwner=false` → Task 1 (`acquire`, `MUTEX_NAME`). ✔
- `w!()` compile-time name + immediate `get_last_error_code()` read (R2/R3) → Task 1 Step 4 (code + comments). ✔
- `ERROR_ACCESS_DENIED` → `AlreadyRunning` (R4) → Task 1 Step 4 (`Err(_) if ...`). ✔
- Reuse `SafeHandle`, no hand-rolled `Drop` (R7) → Task 1 (`handle: SafeHandle`). ✔
- Both `Win32_System_Threading` + `Win32_Security` features (R1) → Task 1 Step 3. ✔
- Info-icon box (R11) → Task 2. ✔
- Gate placement after `init_logging`, before workers; `Option<SingleInstance>` binding; fail-open (R5) → Task 3. ✔
- Removes accidental hotkey guard as consequence → Task 3 (gate returns before hotkey registration; commit note). ✔
- Verification matrix incl. elevation + happy-path sanity → Task 3 Step 4. ✔
- Autostart seam (not implemented) → no task needed; documented in spec, `MUTEX_NAME` doc comment records the stable-name contract. ✔

**Placeholder scan:** No TBD/TODO/"handle edge cases"/"similar to Task N" — every code step shows complete code. ✔

**Type consistency:** `acquire() -> Result<InstanceLock>`, `InstanceLock::{Acquired(SingleInstance), AlreadyRunning}`, `SingleInstance`, `show_info_message_box(&str, &str)` used identically in Tasks 1→3. `SafeHandle::new` (unsafe), `get_last_error_code() -> u32`, `BrightnessError::windows_api(_, u32)`, `ERROR_ALREADY_EXISTS.0`/`ERROR_ACCESS_DENIED.0` (`u32`) all match verified source. ✔
