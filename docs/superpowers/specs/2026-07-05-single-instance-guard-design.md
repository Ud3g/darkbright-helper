# Design: Single-instance guard

_Date: 2026-07-05 · Addresses architecture-review finding #6_

_Revised 2026-07-05 after a cold adversarial review — see "Review amendments" at the end._

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
"already running" **information** box for the rejected second instance; replacing today's accidental
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

At startup the process calls `CreateMutexW(None, false, w!("Local\\darkbright-helper-single-instance"))`
and inspects the outcome together with the thread-local last-error value. There are four cases:

| `CreateMutexW` returns | last error | Meaning | Result |
|---|---|---|---|
| `Ok(handle)` | `ERROR_ALREADY_EXISTS` (183) | named object already existed | `AlreadyRunning` (our handle is dropped/closed) |
| `Ok(handle)` | anything else | we created the object | `Acquired(guard)` |
| `Err(_)` | `ERROR_ACCESS_DENIED` (5) | name exists, owned by a higher-integrity instance we can't open | `AlreadyRunning` |
| `Err(_)` | anything else | genuinely unexpected OS failure | `Err(BrightnessError)` → fail-open in `main` |

The `Ok(handle)` case succeeds (returns a valid, non-null handle) **even when the object already
existed** — `ERROR_ALREADY_EXISTS` is reported through the last-error value, not through an `Err`.
So "already running" is detected from last-error on the *success* path, not from an `Err`.

**Reading last-error correctly is load-bearing and easy to get silently wrong** (see review
amendment R2/R3):

- Use a **compile-time wide-string constant** for the name — `w!("Local\\...")` (the codebase
  already uses `w!`). A runtime `HSTRING`/`Vec<u16>` name argument would allocate and then *drop* at
  the call statement's semicolon, and that heap free can overwrite last-error **before** we read it.
- Read last-error via the codebase's **`get_last_error_code()`** (`platform/windows/mod.rs:81`,
  which wraps `std::io::Error::last_os_error()` → raw `u32`) as the **literal next statement** after
  `CreateMutexW`. Do **not** use the `windows` crate's `GetLastError()` binding: in 0.52 it returns
  `Result<()>` and HRESULT-encodes the code (`183` → `0x800700B7`), so it can never compare equal to
  `ERROR_ALREADY_EXISTS`.
- Compare against the raw codes: `ERROR_ALREADY_EXISTS.0` (183) and `ERROR_ACCESS_DENIED.0` (5),
  both `WIN32_ERROR` constants in `Win32_Foundation`.

Rationale for a named mutex over the alternatives:

- **Race-free.** The kernel creates the named object atomically, so two simultaneous launches
  cannot both observe "first". A `FindWindow`-then-decide approach has a check-to-act race; a
  lock-file needs its own locking to be atomic.
- **No stale state.** The kernel releases the named object when the last handle closes, which
  happens when the process exits *for any reason*, including a crash or a Task-Manager kill. A
  lock-file left on disk after a crash would wrongly report "already running" on the next launch.
- **Existence, not ownership.** We never `WaitForSingleObject` on the mutex and never take
  ownership — the mere *existence* of the named object is the entire signal. Therefore
  `bInitialOwner = false`, which also sidesteps abandoned-mutex semantics entirely. The object is
  used purely as a named, kernel-lifetime-managed flag.

### Scope of the name — session-local

The name is prefixed `Local\`, placing it in the per-logon-session namespace. Each Windows user —
and each RDP session — gets its own allowed instance; only a *second launch within the same
session* is blocked. This matches the tool's per-user nature: config lives per-user under
`%APPDATA%\BrightnessControl\`, autostart is a per-user (HKCU) mechanism, and the OSD/overlay are
inherently per-session.

Honest caveat (see review amendment R10): the DDC/CI brightness write itself targets the physical
monitor's register and is **machine-global**, not per-session — so under fast user switching a
disconnected console session's instance could, in principle, still issue refresh I/O to the same
panel as the active session's instance. `Local\` is still the right choice (RDP sessions have no
DDC-capable displays, and per-user autostart is inherently per-session), but the justification is
"per-user tool, per-session UI + autostart," **not** "brightness is per-session." A machine-wide
`Global\` scope would wrongly stop a second logged-in user from running their own copy, which is the
worse failure.

### Module & API — `src/platform/windows/single_instance.rs`

A new platform module holds all of it. `unsafe`/FFI stays isolated behind a safe RAII wrapper, as
close to the point of use as possible, per project convention. The guard **reuses the existing
`SafeHandle` RAII wrapper** (`platform/windows/mod.rs:183`, `CloseHandle`-on-drop) rather than
hand-rolling a `Drop` impl.

```rust
/// RAII guard that holds the single-instance mutex *handle* for the process
/// lifetime. Dropping it closes the handle; the kernel then releases the named
/// object once no handle remains, freeing the name for the next launch.
///
/// (This holds a handle, not mutex *ownership* — the object is never acquired
/// via a wait; its existence is the signal.)
pub struct SingleInstance {
    _handle: SafeHandle,
}

/// Result of attempting to become the sole instance in this session.
pub enum InstanceLock {
    /// This process is the first/only instance; hold the guard for the
    /// process lifetime.
    Acquired(SingleInstance),
    /// Another instance in this session already holds the name.
    AlreadyRunning,
}

/// Attempts to become the single instance for the current session.
///
/// Creates a session-local named mutex. Returns [`InstanceLock::AlreadyRunning`]
/// if the named object already exists (or exists but is owned by a
/// higher-integrity instance we cannot open); otherwise returns
/// [`InstanceLock::Acquired`] wrapping the RAII guard.
///
/// # Errors
///
/// Returns `Err` only on a genuinely unexpected `CreateMutexW` failure (any
/// failure other than access-denied). "Already running" is **not** an error —
/// it is reported as [`InstanceLock::AlreadyRunning`].
pub fn acquire() -> Result<InstanceLock>;
```

- The type and `acquire`/`InstanceLock` are re-exported from `platform/windows/mod.rs` alongside
  the other public platform types (`DdcSupervisor`, `TrayIcon`, …).
- Features/paths: `HANDLE`, `CloseHandle`, `ERROR_ALREADY_EXISTS`, `ERROR_ACCESS_DENIED` are in
  `Win32_Foundation` (already enabled). `CreateMutexW` needs **both** `Win32_System_Threading`
  **and** `Win32_Security` (its `lpmutexattributes: Option<*const SECURITY_ATTRIBUTES>` parameter
  pulls in `Win32_Security`) — see Cargo.toml below. Last-error is read via the existing
  `get_last_error_code()` helper, so no `GetLastError` binding is needed.
- No `core/` split: the only "pure" fragment is the constant name string, so extracting it for a
  unit test would be ceremony with no value — consistent with the review's own "logic is thin FFI →
  verify manually" stance. (Rejected alternative: a `core/` name-builder with a unit test.)

### Placement & control flow in `main()`

The gate runs immediately after `init_logging()` (so a rejected launch is logged) and **before**
`load_config()`, `DdcSupervisor::spawn()`, the tray/power threads, window creation, and hotkey
registration. The guard is bound to an `Option<SingleInstance>` local so that all three outcomes
share one binding that lives to the end of `main`:

```rust
SetProcessDpiAwarenessContext(...);
init_logging();

// ── single-instance gate ──  (NEW)
let _instance_guard: Option<SingleInstance> = match single_instance::acquire() {
    Ok(InstanceLock::Acquired(guard)) => Some(guard),   // first instance: hold to end of main
    Ok(InstanceLock::AlreadyRunning) => {
        log::info!("Another instance is already running; exiting");
        show_info_message_box(
            "darkbright-helper",
            "darkbright-helper is already running.",
        );
        return;                                          // no workers/windows/hotkeys spawned
    }
    Err(e) => {
        // Fail open: a guard malfunction must not brick the user's only instance.
        log::error!(error:% = e; "Single-instance check failed; continuing without guard");
        None
    }
};

load_config();
DdcSupervisor::spawn(...);
// ...
```

Key properties:

- **`let _instance_guard = ...` (named-underscore binding), not `let _ = ...`.** The value must live
  to the end of `main`; `let _ =` would drop it immediately and silently defeat the guard.
- **The `AlreadyRunning` path returns before any side effect.** No DDC worker, no tray icon, no
  power listener, no windows, no `RegisterHotKey`. The second instance is cheap and leaves nothing
  behind.
- **Today's accidental guard is removed as a consequence.** Because the second instance now exits
  before hotkey registration, the confusing "failed to register hotkeys" box on double-launch is
  replaced by a clear "already running" information box. The existing hotkey-error handling stays
  for its *real* case (another app genuinely owns the hotkey).

### User-visible feedback

In the release build `windows_subsystem = "windows"` hides the console, so a message box is the only
way to tell a user "nothing happened because I'm already running." A **new**
`show_info_message_box(title, message)` helper is added next to `show_error_message_box`
(`platform/windows/mod.rs:272`), identical but with `MB_OK | MB_ICONINFORMATION` instead of
`MB_ICONERROR` — because "already running" is a normal notice, not an error. It reuses the same
wide-string plumbing. The rejected instance shows one such box, then returns from `main` (exit `0`).

### Error handling / taxonomy

- On the `Err(_)` path, `acquire()` builds the error from the **already-captured** last-error value:
  `BrightnessError::windows_api("CreateMutexW", last_error)` — it does not re-read last-error (that
  could have been clobbered by then).
- `AlreadyRunning` (from `ERROR_ALREADY_EXISTS` **or** `ERROR_ACCESS_DENIED`) is a normal
  control-flow variant, **not** a `BrightnessError`.
- `main` treats an `Err` from `acquire()` as **fail-open**: log and continue unguarded, rather than
  block the user's only instance on an unexpected guard malfunction.

### Guard lifetime — what actually guarantees cleanup

The kernel releasing the named object on process death (any cause) is the **real** guarantee; the
`SafeHandle` `Drop` closing the handle on normal exit is conventional and harmless but *not*
load-bearing (see review amendment R6). We still hold it in `SafeHandle` for consistency with the
codebase's RAII convention. No exit path in the app calls `std::process::exit`/`ExitProcess`; normal
shutdown and the Ctrl-C handler both return through `main`, so `Drop` does run on the clean path —
it simply isn't the thing we depend on.

## Cargo.toml

Add **two** features to the `windows` crate list: `"Win32_System_Threading"` (for `CreateMutexW`)
and `"Win32_Security"` (for the `SECURITY_ATTRIBUTES` parameter type it references). The existing
list has no particular ordering and `cargo fmt` does not format `Cargo.toml`, so just append the two
entries. No other dependency changes.

## Verification

Consistent with the review's "thin FFI → verify manually" note, there is no meaningful pure logic to
unit-test here; verification is manual and documented in the plan:

1. **Double-launch blocked:** start the app (tray icon appears); launch the binary again → an
   "already running" **info** box appears, that instance exits, and the *first* instance is
   completely unaffected (tray icon, hotkeys, OSD all still work).
2. **No duplicate side effects:** during step 1, confirm no second tray icon and no second overlay
   flash before the box appears.
3. **Clean release:** close the first instance normally, relaunch → succeeds (handle closed on
   `Drop`, kernel released the name).
4. **Crash release:** kill the first instance via Task Manager (no clean `Drop`), relaunch →
   still succeeds, proving the kernel released the object on process death (no stale-lock problem).
5. **Elevation mismatch (best-effort):** run the first instance elevated ("Run as administrator"),
   launch a second normally → the second shows "already running" and exits (exercises the
   `ERROR_ACCESS_DENIED` → `AlreadyRunning` path). Outcome can depend on the object's default
   security descriptor; note the observed result.
6. **Happy-path last-error sanity:** on a *fresh* first launch (no prior instance), confirm via logs
   that the app proceeds as the first instance — guards against a last-error-clobber regression that
   would make every launch read "first" and silently disable the guard.

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

---

## Review amendments

A cold adversarial reviewer (Fable 5) checked the first draft against the vendored `windows` 0.52.0
source and the codebase. Findings judged relevant, and how this revision addresses them:

- **R1 — `Win32_Security` feature also required.** Verified: `CreateMutexW` is
  `#[cfg(all(feature = "Win32_Foundation", feature = "Win32_Security"))]`. The draft's "add one
  feature" would not compile. → Cargo.toml section now adds both features.
- **R2 — last-error read is a silent trap.** A `log` line or a dropped `HSTRING`/`Vec<u16>` name
  temporary between `CreateMutexW` and the last-error read can clobber last-error, making every
  launch read "first instance." → Mechanism section mandates a `w!()` compile-time name and reading
  `get_last_error_code()` as the literal next statement; added verification step 6.
- **R3 — the `windows` `GetLastError()` binding is unusable here.** Verified: it returns
  `Result<()>` and HRESULT-encodes the code, so it never equals `ERROR_ALREADY_EXISTS`. → Use the
  existing `get_last_error_code()` (raw `u32` via `std::io::Error::last_os_error()`).
- **R4 — fail-open bypass on elevation mismatch.** An elevated first instance can make the second's
  `CreateMutexW` fail with `ERROR_ACCESS_DENIED` rather than succeed-with-already-exists; pure
  fail-open would then run a duplicate. → `acquire()` maps `ERROR_ACCESS_DENIED` to `AlreadyRunning`;
  only *other* errors fail open. Added verification step 5.
- **R6 — RAII `Drop` is not load-bearing.** Correct; kernel cleanup on process death is the real
  guarantee. → Reframed in "Guard lifetime"; the guard reuses `SafeHandle` so `Drop` costs no extra
  code.
- **R7 — duplicate handle wrapper.** The codebase already has `SafeHandle`. → `SingleInstance`
  embeds `SafeHandle`; no hand-rolled `Drop`.
- **R10 — wrong per-session premise.** DDC brightness is machine-global, not per-session. →
  "Scope of the name" corrects the rationale and states the fast-user-switching caveat honestly
  while keeping `Local\`.
- **R11 — error icon for a non-error.** → New `show_info_message_box` (`MB_ICONINFORMATION`).
- **R5 / R8 — non-compiling `match` sketch and "owns the mutex" wording.** → `main` sketch now uses
  `Option<SingleInstance>`; the guard's doc comment says it holds a *handle*, not ownership.
- **R9 — bogus Cargo ordering/`cargo fmt` instruction.** → Removed; just append the two features.

Reviewer items intentionally **not** changed: the atomic-creation race-freedom, mutex-name validity,
`Win32_Foundation` membership of the error/handle symbols, and the exit-path analysis all held up
under the review and needed no change.
