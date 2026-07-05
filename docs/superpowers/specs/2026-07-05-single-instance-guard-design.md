# Design: Single-instance guard

_Date: 2026-07-05 · Addresses architecture-review finding #6_

## Problem

`docs/architecture-review.md` finding #6: **no single-instance guard for an autostart-intended
tray app.** There is no `CreateMutex`/named-mutex check anywhere in the codebase — a second launch
double-registers hotkeys. Today the *second* `RegisterHotKey` fails, which pops a hotkey-error box
and exits (`main.rs:953`–`968`). That is a *partial, accidental* guard: it happens only because the
hotkeys are already claimed, it surfaces a confusing "failed to register hotkeys" message that
implies a config problem, and it fires only *after* the second instance has already spawned the DDC
worker, tray thread, power listener, and created windows — briefly producing a duplicate tray icon
and overlay before it dies.

Autostart ("always start with the system") is the #1 roadmap item in
`docs/improvement-ideas.md`. Once the app launches on login, a manual double-launch by the user
becomes an ordinary event, which makes the missing guard a real gap rather than a theoretical one.

## Scope

**In scope:** a named-mutex single-instance check at startup, run before any worker thread, window,
or hotkey is created; a RAII guard that holds the mutex for the process lifetime; a clear
"already running" message box for the rejected second instance; replacing today's accidental
hotkey-error guard with this intentional one.

**Out of scope (non-goals):**

- Autostart itself (the roadmap item this unblocks) — not implemented here, but the design leaves a
  clean seam for it.
- Cross-process signaling ("focus/pop the running instance when a second launch is attempted") —
  explicitly deferred; the second instance just informs and exits. The `AlreadyRunning` branch is
  the single insertion point should this be added later.
- A config knob to disable the guard — deliberately omitted (YAGNI); a guard that can be switched
  off only invites the duplicate-instance bugs it exists to prevent.
- Supervising or guarding the other threads — unrelated to instance uniqueness.

## Design

### Mechanism — named mutex, existence as the signal

At startup the process calls `CreateMutexW(NULL, FALSE, "Local\\darkbright-helper-single-instance")`
and then immediately reads `GetLastError()`:

- `ERROR_ALREADY_EXISTS` → another instance in this session already created the named object →
  this process is the second instance.
- any other (success) value → this process created the object → it is the first/only instance.

Rationale for this mechanism over the alternatives:

- **Race-free.** The kernel creates the named object atomically, so two simultaneous launches
  cannot both observe "first". A `FindWindow`-then-decide approach has a check-to-act race; a
  lock-file needs its own locking to be atomic.
- **No stale state.** The handle is released by the kernel when the process exits *for any reason*,
  including a crash or a Task-Manager kill. A lock-file left on disk after a crash would wrongly
  report "already running" on the next launch.
- **Existence, not ownership.** We never `WaitForSingleObject` on the mutex and never take
  ownership — the mere *existence* of the named object is the entire signal. Therefore
  `bInitialOwner = FALSE`, which also sidesteps abandoned-mutex semantics entirely. The object is
  used purely as a named, kernel-lifetime-managed flag.

### Scope of the name — session-local

The name is prefixed `Local\`, placing it in the per-login-session namespace. Each Windows user —
and each RDP session — gets its own allowed instance; only a *second launch within the same
session* is blocked. This matches the tool's per-user nature: config already lives per-user under
`%APPDATA%\BrightnessControl\`, and brightness/OSD/overlay are inherently per-session. A machine-wide
`Global\` scope would wrongly stop a second user from running their own copy.

### Module & API — `src/platform/windows/single_instance.rs`

A new platform module holds all of it. `unsafe`/FFI stays isolated behind a safe RAII wrapper, as
close to the point of use as possible, per project convention.

```rust
/// RAII guard holding the single-instance mutex for the process lifetime.
///
/// While this value is alive the process owns the named mutex; dropping it
/// closes the handle, releasing the name so the next launch can acquire it.
pub struct SingleInstance {
    handle: HANDLE,
}

/// Result of attempting to become the sole instance in this session.
pub enum InstanceLock {
    /// This process is the first/only instance; hold the guard for the
    /// process lifetime.
    Acquired(SingleInstance),
    /// Another instance in this session already holds the mutex.
    AlreadyRunning,
}

/// Attempts to become the single instance for the current session.
///
/// Creates a session-local named mutex. If the named object already exists,
/// returns [`InstanceLock::AlreadyRunning`]; otherwise returns
/// [`InstanceLock::Acquired`] wrapping the RAII guard.
///
/// # Errors
///
/// Returns `Err` only if the underlying `CreateMutexW` call itself fails
/// (an unexpected OS error). The "already running" condition is **not** an
/// error — it is reported as [`InstanceLock::AlreadyRunning`].
pub fn acquire() -> Result<InstanceLock>;

impl Drop for SingleInstance {
    // CloseHandle(self.handle)
}
```

- The type and `acquire`/`InstanceLock` are re-exported from `platform/windows/mod.rs` alongside
  the other public platform types (`DdcSupervisor`, `TrayIcon`, …).
- `HANDLE` comes from `Win32_Foundation` (already enabled); `CreateMutexW` needs the new
  `Win32_System_Threading` feature; `GetLastError`, `ERROR_ALREADY_EXISTS`, and `CloseHandle` are in
  `Win32_Foundation`.
- The mutex name is a module-level `const`. No `core/` split: the only "pure" fragment is that
  constant string, so extracting it for a unit test would be ceremony with no value — consistent
  with the review's own "logic is thin FFI → verify manually" stance. (Rejected alternative:
  a `core/` name-builder with a unit test.)

### Placement & control flow in `main()`

The gate runs immediately after `init_logging()` (so a rejected launch is logged) and **before**
`load_config()`, `DdcSupervisor::spawn()`, the tray/power threads, window creation, and hotkey
registration:

```
SetProcessDpiAwarenessContext(...)
init_logging()

// ── single-instance gate ──  (NEW)
let _instance_guard = match single_instance::acquire() {
    Ok(InstanceLock::Acquired(guard)) => guard,          // first instance: keep alive to end of main
    Ok(InstanceLock::AlreadyRunning) => {
        log::info!("Another instance is already running; exiting");
        show_error_message_box(
            "darkbright-helper",
            "darkbright-helper is already running.",
        );
        return;                                           // no workers/windows/hotkeys spawned
    }
    Err(e) => {
        // Fail open: a guard failure must not brick the user's only instance.
        log::error!(error:% = e; "Single-instance check failed; continuing without guard");
        // fall through and run normally (no guard held)
    }
};

load_config()
DdcSupervisor::spawn(...)
...
```

Key properties:

- **The guard is bound to a `main`-scoped local** (`_instance_guard`) that lives until the end of the
  event loop, so `Drop` → `CloseHandle` runs on normal shutdown. (The `Err` fail-open branch runs
  *without* a guard; expressing this cleanly may use a small helper or an `Option`-typed binding —
  an implementation detail for the plan.)
- **The `AlreadyRunning` path returns before any side effect.** No DDC worker, no tray icon, no
  power listener, no windows, no `RegisterHotKey`. The second instance is cheap and leaves nothing
  behind.
- **Today's accidental guard is removed as a consequence.** Because the second instance now exits
  before hotkey registration, the confusing "failed to register hotkeys" box on double-launch is
  replaced by a clear "already running" box. The existing hotkey-error handling stays for its
  *real* case (another app genuinely owns the hotkey).

### User-visible feedback

In the release build `windows_subsystem = "windows"` hides the console, so a message box is the only
way to tell a user "nothing happened because I'm already running." The rejected instance shows a
single informational box via the existing `show_error_message_box(title, message)` helper
(`MB_OK | MB_ICONERROR`), then exits `0`. No new UI code.

### Error handling / taxonomy

- `acquire()` maps a genuine `CreateMutexW` failure through the established
  `BrightnessError::windows_api("CreateMutexW", code)` convention (matching the `to_brightness_result`
  helper style), returning `Result`.
- `AlreadyRunning` is a normal control-flow variant, **not** a `BrightnessError`.
- `main` treats an `Err` from `acquire()` as **fail-open**: log and continue unguarded, rather than
  block the user's only instance on a guard malfunction.

## Cargo.toml

Add `"Win32_System_Threading"` to the `windows` crate feature list (for `CreateMutexW`). Insert it in
the existing alphabetically-adjacent position and keep the list formatted the way `cargo fmt`
expects. No other dependency changes.

## Verification

Consistent with the review's "thin FFI → verify manually" note, there is no meaningful pure logic to
unit-test here; verification is manual and documented in the plan:

1. **Double-launch blocked:** start the app (tray icon appears); launch the binary again → an
   "already running" box appears, the box's instance exits, and the *first* instance is completely
   unaffected (tray icon, hotkeys, OSD all still work).
2. **Clean release:** close the first instance normally, relaunch → succeeds (the name was released
   on `Drop`).
3. **Crash release:** kill the first instance via Task Manager (no clean `Drop`), relaunch →
   still succeeds, proving the kernel released the handle on process death (no stale-lock problem).
4. **No duplicate side effects:** during step 1, confirm no second tray icon and no second overlay
   flash before the box appears.

CI gates are unchanged and must pass before push:
`cargo +stable clippy --all-targets --locked -- -D warnings` and `cargo test` (run on
`windows-latest`; the cross-platform `core/` still builds on Linux, but this feature is
Windows-only and is checked against the `x86_64-pc-windows-msvc` target locally).

## Autostart seam (not implemented)

The stable, documented mutex name (`Local\darkbright-helper-single-instance`) is the contract a
future autostart feature relies on: autostart merely launches the exe on login, and this guard
ensures only one copy runs per session. If a later version wants a second launch to *focus or pop*
the running instance instead of just informing-and-exiting, the `InstanceLock::AlreadyRunning`
branch in `main` is the single place to add cross-process signaling (e.g. a named event or a
registered window message) — no other code needs to change. Recorded here as a future extension,
deliberately not built now.
