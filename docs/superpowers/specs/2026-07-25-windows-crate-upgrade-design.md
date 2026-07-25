# Design: `windows` crate upgrade 0.52 → 0.62

**Date:** 2026-07-25
**Status:** design approved in dialogue; cold adversarial review incorporated; awaiting user review
**Branch:** `windows-crate-upgrade` (off `main`)

## Goal

Lift the deliberately pinned `windows` crate from 0.52 to the current 0.62.x in one
jump, remove the workarounds the newer crate makes unnecessary, and update the
documentation that encodes the old pin decision. No user-visible behavior change
intended.

## Scope

- **In:** mechanical port to 0.62; removal of workarounds obsoleted by 0.62;
  factual refresh of docs that reference 0.52-era behavior; a written manual test
  checklist.
- **Out:** any broader idiom sweep (rewriting call sites that still work as-is),
  unrelated refactoring, other dependency bumps, a release (decided separately
  after merge).

## Facts (measured, not estimated)

A trial compile of this repo against `windows` 0.62.2 and a signature diff of all
86 Win32 functions the repo calls, across every published crate version, produced:

- **89 compile errors in the lib, +2 under `cfg(test)`, +2 in `main.rs`** (the
  `main.rs` count is a lower bound — the lib fails first, so cargo never fully
  checked the bin; the same caveat applies to `tests/`, where `hotkey_test.rs`
  imports the `windows` crate directly — its imports were verified unchanged in
  0.62, expected breakage zero).
- **The churn is concentrated in 0.58–0.60.** 0.58 contributes the structural
  break (handles become `*mut c_void`, losing `Send`/`Sync`; `CreateWindowExW`
  returns `Result<HWND>`); 0.59 + 0.60 contribute the bulk of the signature
  changes (`Option<…>` params, flag newtypes, `bool` params). One late break:
  **`BOOL` leaves `Win32::Foundation` for `windows::core` at 0.62** (verified
  still in `Foundation` through 0.61). 0.61 changes nothing on this repo's
  API surface.
- **MSRV:** windows 0.62.x requires Rust 1.82 (transitive max across
  windows-core/-result/-strings/-link likewise 1.82). Repo pins
  `rust-version = "1.88"` — **no MSRV bump needed**; the CI 1.88 job stays valid.
- **Feature gates:** all 16 features in `Cargo.toml` survive verbatim — zero
  edits to the feature block. One namespace move: `BOOL` left
  `Win32::Foundation` for `windows::core` (at 0.62); `TRUE`/`FALSE` stay in
  `Foundation`, now typed `windows_core::BOOL`.
- **Dependency graph:** `windows-targets` and the eight arch crates disappear
  (linking now via `windows-link` raw-dylib); new small crates
  (`windows-collections`, `-future`, `-numerics`, `-threading`, `-implement`,
  `-interface`) appear. Fine on the MSVC target.

Error distribution by file: `tray.rs` 16, `osd.rs` 13, `usage.rs` 11,
`osd_render.rs` 10, `hotkey.rs` 10, `power.rs` 9, `mod.rs` 8, `overlay.rs` 5,
`ddc.rs` 5, `ddc_worker.rs` 2, `main.rs` 2. Dominant patterns: handle parameters
become `Option<…>`, `CreateWindowExW` → `Result<HWND>`, GDI calls want explicit
`HGDIOBJ`, flag parameters gain newtypes (`FONT_CHARSET` etc.),
`RegQueryValueExW` returns raw `WIN32_ERROR` (since 0.54).

## Decisions

### Strategy: single jump to 0.62, pinned as `windows = "0.62"`

Stepping stones buy nothing: the work is concentrated in three adjacent versions
(0.58–0.60), and stopping short would leave us doing nearly all of the work while
still being behind — even 0.61 would only dodge the `BOOL` namespace move.
Patch releases within 0.62 had zero API delta, so the pin allows patch drift;
minor/breaking bumps remain deliberate undertakings.

### Structural decision 1: `unsafe impl Send for PhysicalMonitor`

Since 0.58, `PHYSICAL_MONITOR.hPhysicalMonitor` is `HANDLE(*mut c_void)`, which
is `!Send`, so `DdcWorker` no longer satisfies `thread::spawn`. The design is
unaffected — `DdcWorker` is constructed empty on the main thread and moved into
the thread; every `PhysicalMonitor` is created, used, and dropped on the worker
thread; no message enum or `core/` type carries a handle. Only the type check
fails. Fix: `unsafe impl Send for PhysicalMonitor`. The safety comment commits
to the **strong** invariant — DDC physical-monitor handles are process-scoped,
not thread-affine (the same claim 0.52 made implicitly via `HANDLE(isize)`
being `Send`) — not the weaker "only ever touched on the worker thread", so a
future cross-thread move stays covered by the stated argument. `Sync` is
deliberately not implemented. This is the escape hatch the windows-rs
maintainer points to. The alternative (store `isize`, rebuild `HANDLE` per
call site) makes the same safety claim implicitly while scattering casts.

### Structural decision 2: the `core`↔`platform` seam stays `isize`

`MonitorHandle(pub isize)` (`core/controller.rs`) and `TrayStatusHandle(isize)`
(`tray.rs`) stay as-is: `core/` stays platform-free, and `TrayStatusHandle` must
remain `Send` (crosses the tray/main thread boundary). The isize↔pointer
conversion happens at the existing handoff points (7 sites: `mod.rs` ×2,
`osd.rs`, `overlay.rs`, `ddc_worker.rs`, `tray.rs` ×2) via a central helper pair
in `platform/windows/mod.rs` rather than seven hand-written casts. This
concentrates most — not all — of the new clippy pointer lints: the `HMENU`
control-ID cast in `usage.rs` and the `ShellExecuteW` result casts in `main.rs`
(which silently become pointer→int casts in 0.62) sit outside the helpers.

### Structural decision 3: handle logging switches to Debug format

`*mut c_void` does not implement `log::kv::ToValue`. The two
`log::debug!(hwnd = self.hwnd.0)` sites in `power.rs` become
`hwnd:? = self.hwnd`. Cosmetic consequence: handles print as pointers, not
integers, in debug logs.

## Commit sequence

The version bump changes all signatures atomically — a half-ported tree does not
compile. Hence:

1. **One port commit** (large but unavoidably atomic): pin bump + everything
   needed to compile. Internal working order: (a) isize↔handle helpers +
   `unsafe impl Send` + `BOOL` import move; (b) mechanical sweep (`Option<…>`
   wrapping, `CreateWindowExW` → `?`, explicit `HGDIOBJ`, `RegQueryValueExW` →
   `.ok()`); (c) log-kv fix. Result: compiles, `cargo test` green, fmt clean.
   Note: the 7 manual `CreateWindowExW` null-checks necessarily collapse to `?`
   here — the new signature forces it; that is port, not cleanup.
2. **Cleanup commits, each individually green:**
   - `get_last_error_code()` (`mod.rs`) **stays**: it is the engine behind
     `last_error_as_brightness_error` (~25 surviving call sites), and its
     `std::io::Error::last_os_error()` body is safe and version-independent —
     "removing the shim" would trade a safe std call for an unsafe FFI call
     with no benefit. What actually changes: fix the comment in
     `single_instance.rs` claiming the crate's `GetLastError` HRESULT-wraps
     the code (true in 0.52, wrong since 0.54), and delete the caller-less
     `last_error_is_success()` (`mod.rs`) as pre-existing dead code.
   - Replace manual `.0 != 0` / `.0 != -1` checks in `SafeHwnd`/`SafeHandle`
     with `is_invalid()`.
   - Replace `SafeHandle` with `windows::core::Owned<HANDLE>` **if** it fits
     1:1 (`Free` calls the same `CloseHandle`); if friction appears, keep
     `SafeHandle` and note why.
3. **Docs commit(s)** (next section).

**Deliberately kept:** the manual `Debug` impl for `PhysicalMonitor`
(`PHYSICAL_MONITOR` never gained a `Debug` derive, still packed) and the 21
`e.code().0.cast_unsigned()` sites (`HRESULT(pub i32)` unchanged).

**Clippy gate:** `cargo +stable clippy --all-targets -- -D warnings` with
*current* stable (CI uses latest stable; an older local toolchain misses new
lints). New pointer-typed handles will likely surface cast lints at the isize
seam — they land bundled in the helper pair.

## Documentation updates

- **`Cargo.toml`** (dependency comment): replace the "deliberately pinned at
  0.52" rationale — now: tracks current minor (0.62), patch bumps free,
  minor/breaking bumps remain deliberate standalone undertakings.
- **`docs/improvement-ideas.md`** (Maintenance entry): rewrite — upgrade done
  (dated), the 2027-07 revisit trigger lapses. New, slimmer entry: future
  windows-crate bumps are routine maintenance; breaking releases again get their
  own branch + hardware test pass.
- **`docs/code-conventions.md`** §3 "Windows Crate (v0.52+) Specifics": factual
  refresh to 0.62 — handles are pointers (`is_invalid()` instead of `.0 == 0`,
  no `Send`), handle params often `Option<…>`, `BOOL` lives in `windows::core`,
  `bool` params instead of `Into<BOOL>`. Style rules that feel unidiomatic
  during the port are collected and proposed in the docs commit, not silently
  changed.
- **`docs/architecture.md`**: check for version-bound claims while porting;
  touch only where facts no longer hold.
- **`CLAUDE.md`**: the "`windows` crate v0.52" conventions bullet hardcodes
  0.52-era guidance and is read at the start of every future session — refresh
  alongside code-conventions.md (pointer handles, `is_invalid()`, `Option<…>`
  params, `BOOL` in `windows::core`, `bool` instead of `Into<BOOL>`).
- **`CHANGELOG.md`**: one `[Unreleased]` line for the dependency major, incl.
  the cosmetic handle-format change in debug logs.
- Sweep the two "0.52+"-phrased comments in `ddc.rs` (near the
  `DestroyPhysicalMonitors` and `SetupDiEnumDeviceInfo` calls) — still
  factually true in 0.62 but stale-reading.

## Verification

Automated: `cargo fmt --check`, the clippy gate above, `cargo test`; MSRV is
covered by the existing CI 1.88 job.

Manual (user, on real hardware; a written checklist ships in the branch):

- **Smoke test after the port commit:** launch, brightness up/down via hotkey,
  OSD appears.
- **Full hardware pass at branch end** per `docs/architecture.md` "Integration
  Testing": DDC set/get on all monitors, OSD, overlay at 0%, tray menu incl.
  live status, hotkeys incl. opt-in low-level hook, resume from sleep, monitor
  hot-plug, single-instance behavior. Additionally — paths the port
  demonstrably touches that the architecture.md list does not reach:
  - the usage/instructions window (open it, focus behavior, OK button);
  - Ctrl+C shutdown in a debug console (`SetConsoleCtrlHandler`);
  - the tray degraded-status *push* path (induce a DDC-degraded state and
    watch the tray icon update — distinct from the menu's *pull* path);
  - the supervised-restart paths (DDC-worker respawn, hotkey-thread restart).
- **Targeted checks** (behavior changes no compiler catches): single-instance
  detection (`ERROR_ALREADY_EXISTS` — the load-bearing `GetLastError` timing
  site), and plausibility of error codes in the OSD error state (where
  crate-captured errors replace manual null-check + deferred `GetLastError`,
  codes can differ in detail — strictly more correct, but different).

## Accepted residual risks

- `Eq` was dropped from `MSG`/`POINT`/`KBDLLHOOKSTRUCT` (0.59) — not used as map
  keys today; relevant only to future code.
- Debug log output changes cosmetically (handles print as pointers).
- raw-dylib linking replaces import libs — uncritical on the MSVC target; only a
  consideration for hypothetical `-gnu` builds.

## Appendix: expected breaks by file (from the trial compile)

| File | Errors | Dominant breaks |
|---|---|---|
| `tray.rs` | 16 | `Option<HINSTANCE>`/`Option<HDC>`/`Option<HWND>` params; `SelectObject`/`DeleteObject` want `HGDIOBJ`; `CreateWindowExW` → `Result`; `TrayStatusHandle(isize)` seam (`as_raw().0` now `*mut c_void`) |
| `osd.rs` | 13 | hwnd params → `Option<HWND>`; `GetModuleHandleW` result vs `Option<HINSTANCE>` param; `CreateWindowExW` → `Result`; `HMONITOR(isize)` seam |
| `usage.rs` | 11 | `CreateWindowExW` → `Result`; `HMENU(isize)` control-ID cast; `SetFocus` → `Option<HWND>` + `Result`; `IsWindow` → `Option<HWND>` |
| `osd_render.rs` | 10 | explicit `HGDIOBJ` for `FillRect`/`SelectObject`/`DeleteObject`; `CreateFontW` flag newtypes (`FONT_CHARSET` …); `Option<HDC>`; null checks → `is_invalid()` |
| `hotkey.rs` | 10 | `CallNextHookEx` → `Option<HHOOK>`; `CreateWindowExW`; `RegisterHotKey`/`GetMessageW` hwnd → `Option<HWND>` |
| `power.rs` | 9 | `CreateWindowExW`; `GetMessageW`; log-kv of raw handle (2 sites) |
| `mod.rs` | 8 | seam conversions; `SafeHwnd`/`SafeHandle` validity checks; `MessageBoxW` hwnd → `Option` |
| `overlay.rs` | 5 | `CreateWindowExW`; `SetWindowPos` insert-after → `Option<HWND>`; seam |
| `ddc.rs` | 5 (+1 test) | `BOOL` import move; `EnumDisplayMonitors`/`SetupDiGetClassDevsW` `Option` params; `SetupDiOpenDevRegKey` scope `u32`; `RegQueryValueExW` → `WIN32_ERROR`; `assert_send::<DdcMonitor>` fails (→ Send decision) |
| `ddc_worker.rs` | 2 | `handle_cache` keyed on `hmonitor.0` (now pointer); `DdcWorker` `!Send` (→ Send decision) |
| `main.rs` | 2 (lower bound) | `BOOL` import move; `SetConsoleCtrlHandler(…, bool)` |

Verified unbroken: `SetVCPFeature`, `GetVCPFeatureAndVCPFeatureReply`,
`GetPhysicalMonitorsFromHMONITOR`, `DestroyPhysicalMonitors`,
`SetWindowsHookExW`, `CreateMutexW`, `RegisterSuspendResumeNotification`,
`DwmSetWindowAttribute`, `GetDpiForMonitor`, `SetProcessDpiAwarenessContext`,
`ShellExecuteW`, `RegisterClassW/ExW`, `w!`/`PCWSTR`, `.as_bool()`,
`PostQuitMessage`, `DefWindowProcW`, callback signatures (`WNDPROC`,
`HOOKPROC`, `TIMERPROC`, `MONITORENUMPROC`).
