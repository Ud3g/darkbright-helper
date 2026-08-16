# Architecture Review: darkbright-helper

_Review date: 2026-08-01 · Reviewed at v0.8.0 + post-review polish (commit `0d0a86e`, branch
`arch-review-fixes-2026-07-26`) · Independent pass. The 2026-07-26 cycle (`docs/architecture-review.md`)
is treated as done: its resolved rows were **not** re-litigated, only checked to be in place. Its two
still-open cosmetics are carried forward here as rows 13 and 14; two further rows it lists as open
(18 Dependabot, 19 `RespawnGate` fold) have since landed in `cb9d4b0` and `0d0a86e`. Investigation was
inline over the source, git history, CI config and the dependency tree — **every finding below was
derived from the code**. The test suite was executed: `cargo test` → **222 passed, 1 ignored**
(the DDC hardware probe), 0 failed._

## Repo profile

- **Artefact type:** Standalone Windows desktop utility — long-running tray/hotkey background
  process, single binary. Signals: `windows_subsystem="windows"` in release (`main.rs:1`),
  `publish = false` over a deliberately internal `lib.rs`, tray/OSD/overlay UI, no server, no network
  I/O anywhere in the tree.
- **Maturity:** Active product. 430 commits since 2025-12-29, 6 semver tags, maintained CHANGELOG, CI
  on `windows-latest` gating fmt + clippy (`all`+`pedantic`, `-D warnings`, `--all-targets`,
  `--locked`) + tests + release-profile check, a separate MSRV job, a weekly `cargo audit` cron, and
  Dependabot since `cb9d4b0`.
- **Size & team:** ~11.5k LOC Rust across 24 modules, 56 tracked files, single author (`Ud3g`,
  453/453 commits). A large share of the biggest files is in-module tests. **Zero `unsafe` and zero
  `windows::` paths under `src/core/`** — re-verified by grep, so the platform boundary is real by
  import, not by assertion. Zero live `TODO`/`FIXME`/`HACK` in `src/` (the two `XXX` hits are a
  literal registry-path format comment, `ddc.rs:395`). Calibration: solo-maintainer bar —
  change-safety and simplicity over ceremony.
- **Ecosystem:** Rust 2024, MSRV 1.88. `windows` 0.62, `thiserror` 2, `serde`/`serde_json`,
  `log`+`env_logger` (structured kv), `winres` (build). 46-package lock tree. No async runtime —
  single-owner state on the main thread plus MPSC channels across five threads (main / hotkey /
  ddc_worker / power / tray).
- **Ambiguities resolved:** none — classification was unambiguous.

## Overview

The design holds and the previous cycle's fixes are all present and correct. Spot-checks that
mattered: the VCP scaling boundary is intact (`DdcMonitor` learns `reported_max`, converts both
directions, no raw value escapes `ddc.rs`); `DdcHealth`'s two degraded states and the proof-of-life
path through `note_worker_alive` are wired exactly as documented; `DdcSupervisor` now runs on the one
shared `RespawnGate`; the `power.rs` listener window is top-level and pinned by a test.

Three of this pass's four significant findings share one theme: **the tray and usage windows never
got the scrutiny the DDC/refresh path did.** The `MonitorState` / refresh / supervision core is
genuinely well built and well tested. The Win32 window plumbing around it still carries defects that
core would not tolerate — an unreachable RAII teardown, an owning handle to a window something else
destroys, and a window that ignores the DPI mode the process declares.

The Critical finding is the sharpest of them, and its interest is not the bug but its lineage: it is
the **third** instance of a bug class this project has already fixed twice, in a file the previous
fixes never visited.

## Findings

### Critical

**C1. An Explorer restart permanently removes the tray icon — and with it the app's only shutdown
path and every health warning.**
`src/platform/windows/tray.rs:911-928` (window creation), `:815-839` (window procedure), `:551-567`
(`add_tray_icon`, called only from `TrayIcon::new`); `docs/architecture.md:925-929`.

The shell broadcasts a registered `"TaskbarCreated"` message when Explorer restarts; every tray
application must re-issue `Shell_NotifyIconW(NIM_ADD)` on receipt. This app does neither half:

1. **It never registers or handles the message.** Grep for `TaskbarCreated` / `RegisterWindowMessage`
   across `src/`: zero hits. `tray_wnd_proc` handles exactly `WM_TRAY_CALLBACK`, `WM_TRAY_STATUS`
   and `WM_DESTROY`.
2. **It could not receive it if it did.** The tray window is created with `hWndParent = HWND_MESSAGE`
   (`:920`) — a message-only window, which is excluded from broadcast messages.

That second half is the part worth dwelling on. **This is the third instance of a bug class already
fixed twice here.** `power.rs` lost resume detection to it once, then nearly lost `WM_DISPLAYCHANGE`
to it in the last cycle. The fix there is now load-bearing, commented (`power.rs:191-195`), and
pinned by a test asserting the window's `GA_PARENT` is the desktop (`power.rs:351-369`). The sweep
stopped at that module. `tray.rs` has the same window shape and a different broadcast message
depending on it. A grep for `HWND_MESSAGE` at the time of the `power.rs` fix would have found it.

Consequences, all silent, none recoverable without killing the process:

- `architecture.md:925-929` states that with the console hidden "the tray menu's Quit is the only
  *graceful* shutdown path — if the tray thread dies, ending the process takes Task Manager." Here
  the thread does **not** die — it keeps pumping a live window whose icon the shell has forgotten —
  so nothing in the supervision model notices, and the documented failure mode is reached by a path
  the doc does not cover.
- Both degraded-state channels die together. Pull (menu) is gone with the icon; push still
  `PostMessageW`s successfully, the tray thread calls `NIM_MODIFY` on an unregistered icon, it fails,
  and the warning goes to `log::warn!` — i.e. to the console release builds hide, or to a file log
  the user can no longer reach, because "Open Log Folder" was in the lost menu.
- Per-monitor status and the `~` unread marker — the standing signal the previous cycle deliberately
  put in the tray rather than the OSD — become unreachable.

Rated Critical because for an app whose entire interface besides two hotkeys *is* the tray icon,
this is a permanent, silent, unrecoverable loss of all user control and all health signalling,
triggered by a routine Windows event. Brightness adjustment itself keeps working; that is the only
thing keeping it from being worse.

**Direction:** two-part, mirroring the `power.rs` fix. Make the tray window a hidden **top-level**
window (`WS_POPUP | WS_EX_TOOLWINDOW`, never shown) so broadcasts arrive; cache
`RegisterWindowMessageW(w!("TaskbarCreated"))` at class-registration time and, on receipt, re-run
`add_tray_icon` **and** re-apply the current `HealthWarnings` icon/tooltip state — a re-added icon
starts unbadged, so a standing warning would otherwise be silently dropped on the floor. Reuse
`power.rs`'s `GetAncestor(GA_PARENT) == GetDesktopWindow()` assertion as the regression pin: it is
the property, not the module, that needs protecting.

Worth a sweep for remaining `HWND_MESSAGE` while there. `hotkey.rs:275` is also message-only, which
is *correct* — thread-queue `WM_HOTKEY` messages need no broadcast reachability — and deserves a
one-line comment saying so, because the two cases now look identical in the source and are not.

Fixing this also subsumes row 10 (`SetForegroundWindow` on a message-only window).

### Important

**I1. `TrayIcon::drop` — and the `NIM_DELETE` inside it — is unreachable on every exit path.**
`src/platform/windows/tray.rs:1033-1039`, `:971-1000`; `src/main.rs:747-759`;
`src/platform/windows/power.rs:243-255`.

`Drop for TrayIcon` calls `remove_tray_icon`, which is what stops a dead icon lingering in the
notification area. It never runs. Traced end to end: the tray thread blocks in `run_message_loop`'s
`GetMessageW`; the only `PostQuitMessage` is in the `WM_DESTROY` arm (`:833`); the only thing that
would destroy that window is `SafeHwnd::drop` inside `TrayIcon::drop` — a closed loop. Nothing in
`main`'s cleanup block posts `WM_QUIT` to the tray thread, so `main` returns and process exit kills
it mid-`GetMessageW` without unwinding. `PowerEventListener::drop` (`power.rs:243`) is unreachable
for exactly the same reason, and `HotkeyManager::drop` likewise at shutdown.

Of the three only the tray one has a user-visible consequence — a ghost icon persisting until the
notification area is next hovered — but the change-safety cost is shared and is the larger half:
three `Drop` impls read as live cleanup, one of them carrying a comment about a handle "kept alive
to prevent Windows from releasing the icon while the tray is active" (`:881-883`), and nothing in
the source tells a future maintainer that none of them executes. This is the same shape as the
previous cycle's I5 — a comment asserting an invariant that does not exist — one layer up.

**Direction:** give the three threads a shutdown handshake — `PostMessageW(hwnd, WM_CLOSE, …)` for
tray and power, `PostThreadMessageW(tid, WM_QUIT, …)` for hotkey — from `main`'s cleanup, with a
short bounded join. If that is judged not worth it at this project's calibration (defensible for the
power and hotkey threads, whose handles the OS reclaims anyway), then say so at each `Drop` and
replace the tray one with an explicit `remove_tray_icon` call on the shutdown path, so the icon still
goes away. What should not survive is three RAII impls that only look like they run.

**I2. The usage window ignores DPI in a process that declares per-monitor DPI awareness v2.**
`src/platform/windows/usage.rs:25-30`, `:151-172`, `:198-234`; `src/main.rs:515-519`.

`main.rs:518` sets `DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2` with the comment "This prevents
Windows from bitmap-stretching our UI at non-100% scaling" — which is right, and is exactly why
every window must then scale itself. The OSD does: `OsdMetrics::for_dpi` (`osd.rs:113`) scales eleven
metrics off `GetDpiForMonitor` for the target monitor, re-applied on every `position_osd_window`.
The usage window does none of it:

- `USAGE_CLIENT_WIDTH`/`HEIGHT` are hard-coded at 340×150 **physical** pixels, as are the margins,
  the button box, and the derived `text_height` (`:218`). At 150% scaling the window is two-thirds
  the intended size; at 200%, half.
- No `WM_SETFONT` is ever sent to the static or the button, so both fall back to the ancient
  `SYSTEM_FONT` bitmap face rather than the shell dialog font — unscaled, and at 200% roughly a
  quarter of the intended visual size inside an already-undersized box. `text_height` is a fixed
  90 px with no measurement, so clipping is a live risk as soon as the configured hotkey strings are
  longer than the defaults (`"Ctrl+Shift+PageDown"` is already six characters longer than the string
  this was sized against).
- Centring uses `GetSystemMetrics(SM_CXSCREEN)`/`(SM_CYSCREEN)` (`:166-167`), which is the
  **primary** monitor only. On a multi-monitor desk the window opens on the primary display
  regardless of where the tray or cursor is — a mismatch with the app's own governing principle that
  everything follows the monitor under the cursor.
- No `WM_DPICHANGED` handling, so dragging it between displays of different scaling leaves it wrong.

Rated Important rather than cosmetic because high-DPI is the common case on the laptops this tool
targets, the window is the app's onboarding surface (`architecture.md:1030`: "This helps new users
discover how to use the application without consulting documentation"), and the defect is invisible
to a maintainer whose dev machine runs at 100%.

**Direction:** the mechanism already exists one module over. Take `GetDpiForWindow` — or the cursor
monitor's `GetDpiForMonitor`, matching the app's targeting rule — and scale the six constants through
the same `s()` helper shape `OsdMetrics::for_dpi` uses; send `WM_SETFONT` with a `CreateFontW` sized
from that DPI to both controls; centre on `GetMonitorInfoW(rcWork)` of the cursor's monitor rather
than `SM_CXSCREEN`. `WM_DPICHANGED` is optional at this calibration — the window is short-lived —
but the initial scaling is not.

**I3. `UsageWindow` holds an owning `SafeHwnd` for a window its own window procedure destroys behind
its back.**
`src/platform/windows/usage.rs:91-103`, `:277-289`; `src/main.rs:118-138`.

`usage_wnd_proc` calls `DestroyWindow(hwnd)` directly on both the OK button (`:95`) and the close box
(`:101`). The `UsageWindow` in `main`'s `Option<UsageWindow>` still holds a `SafeHwnd` for that
now-destroyed window, and `SafeHwnd`'s contract is explicit that it owns the window and must destroy
it (`mod.rs:161-163`). So the moment the user closes the window, the wrapper's invariant is false and
the stored handle is stale.

Two consequences follow, both the classic stale-`HWND` shape:

- `is_valid()` (`:284-289`) checks `IsWindow` on that stale handle. Windows recycles `HWND` values,
  and `DestroyWindow` succeeds only for windows owned by the *calling thread* — which is the same
  thread that creates the OSD and every overlay window. A recycled value therefore names one of this
  app's own windows. `IsWindow` returning true then sends `open_usage` down the "already open" branch
  (`main.rs:119-125`) and calls `bring_to_front` on an overlay, so Usage stops opening at all.
- If the check goes the other way, `*window = Some(w)` (`main.rs:132`) drops the old `UsageWindow`,
  and `SafeHwnd::drop` calls `DestroyWindow` on the recycled handle — destroying a live overlay or
  the OSD.

Low probability: modern `HWND` values carry a reuse counter, and this app's window churn is small
(one OSD, one overlay per dimmed monitor, one usage window per open). But the ownership violation is
structural and reads as correct, which is what makes it a change-safety risk rather than just a rare
bug — the next feature that creates windows on the main thread raises the collision rate without
anyone connecting the two.

**Direction:** stop destroying the window from inside its own procedure. On `WM_CLOSE` and the OK
button, post a message to the main thread so it drops its `Option<UsageWindow>` and lets
`SafeHwnd::drop` do the destroying — one owner, one destruction, and `is_valid()` stops being needed
at all. If the direct `DestroyWindow` stays, then `WM_NCDESTROY` must clear main's `Option` before
the handle can be recycled.

**I4. Daisy-chained physical monitors under one `HMONITOR` silently collapse onto one, and all but
the last lose their handle.**
`src/platform/windows/ddc_worker.rs:178-202`; `src/platform/windows/ddc.rs:128-135`.

`get_physical_monitors`'s own doc comment states the case: "A single `HMONITOR` (logical monitor) can
map to multiple physical monitors (e.g., in daisy-chain configurations), though usually it's 1:1."
The consumer cannot represent it. `process_monitor` derives **one** `MonitorId` from the `HMONITOR`'s
EDID, then loops over every physical monitor inserting into `HashMap<MonitorId, DdcMonitor>` under
that same key. The map keeps the last; each earlier `DdcMonitor` is dropped, taking
`DestroyPhysicalMonitors` with it. `results` meanwhile collects N entries for the same id, so
`handle_ddc_refresh_result` logs "Monitor found during refresh" N times and `count = monitors.len()`
over-reports.

Net effect on such a chain: one panel is controllable, the rest silently are not, and nothing in the
log says which or why — the same silent-partial-control shape as the previous cycle's I1, one layer
down. The identity scheme is the root cause: EDID is read per `HMONITOR`, so two physical monitors
behind one logical monitor are genuinely indistinguishable to the current `MonitorId`.

**Direction:** the cheap honest fix is to detect and report. If `physical_monitors.len() > 1`, log a
warning naming the monitor and the count, keep the first deliberately, and drop the rest — so a field
log explains the missing panel instead of hiding it. Representing it properly needs a disambiguator
on `MonitorId` (`PHYSICAL_MONITOR::szPhysicalMonitorDescription`, or an index), which is a schema
change to a type that also keys the config's `monitors` map — not worth it until such hardware is
actually in use.

**Unverified:** no daisy-chained monitor available here. The defect is static; its reachability is
not.

### Cosmetic / nice-to-have

**Tray & Win32 details** _(TRAY-1 … TRAY-3)_
- `NIF_SHOWTIP` is passed on both `NIM_ADD` (`tray.rs:556`) and `NIM_MODIFY` (`:490`), but
  `NIM_SETVERSION` is never called, so the icon runs in version-0 behaviour where that flag is inert.
  Harmless today — v0 shows standard tooltips anyway — but the flag documents an intent that is not
  in force.
- `create_warning_icon` builds the icon's AND mask with `CreateBitmap(size, size, 1, 1, None)`
  (`tray.rs:454`), whose contents MSDN documents as undefined. The 32-bit colour bitmap's alpha is
  what shapes the icon on the paths that matter, and the comment says so — but an undefined mask is a
  contract violation that can surface in any legacy non-alpha render path. Passing
  `Some(vec![0u8; …].as_ptr().cast())` instead costs one line.
- `SetForegroundWindow(hwnd)` before `TrackPopupMenu` (`tray.rs:679`) is the documented workaround for
  tray menus not dismissing on an outside click — but it is documented to fail on a message-only
  window, so the workaround is not actually in effect. **Subsumed by C1**: the window becoming
  top-level fixes it. _Unverified — the symptom is a UI behaviour not reproducible from source._

**Hotkeys** _(HK-1)_
- `impl Display for ParsedHotkey` (`hotkey.rs:439-465`) resolves key names from `VK_TO_NAME`
  (`:547-579`), which omits the letters and digits `KEY_MAP` accepts — so a configured `Ctrl+A`
  renders as `"Ctrl+Unknown"`. No production caller today (grep confirms), so it is latent; but the
  type is `pub` and the parse→display round trip is silently lossy for 36 of the accepted keys.

**Dependencies** _(DEP-4)_
- `env_logger`'s default features pull `regex` + `regex-automata` + `regex-syntax` + `aho-corasick` +
  `memchr` (via `env_filter`) and `jiff` into a tray utility that needs level filtering and a
  timestamp. That is a large fraction of the 46-package tree and a meaningful slice of release binary
  size. `default-features = false` plus the colour/timestamp/`unstable-kv` features drops the regex
  chain; the only behaviour lost is `RUST_LOG`'s regex message-filter syntax (`RUST_LOG=info/pattern`),
  which degrades to substring matching. Worth a line in `architecture.md`'s dependency table either
  way, because the current set reads as deliberate and is not.

**Hygiene & docs** _(HYG-1, DOC-6)_
- The working tree fails `cargo fmt --all -- --check`: a stray blank line at `src/core/config.rs:1099`,
  the only uncommitted change on the branch. CI would go red on push.
- `docs/architecture-review.md` is two rows behind its own repo. Rows 18 (Dependabot) and 19 (fold
  `DdcSupervisor` onto `RespawnGate`) were resolved by `cb9d4b0` and `0d0a86e` — both already
  reflected in `architecture.md` and `dependabot.yml` — but its table still shows them unresolved and
  its resolution plan still says "leaving four: 15, 17, 18 and 19." Only 15 and 17 remain, and they
  are carried into this document as rows 13 and 14.

## Resolution plan

Carried forward unchanged from the 2026-07-26 cycle: **DOC-2** (row 13) and **DEP-2** (row 14). Both
were open there and remain open; neither was re-derived, only re-confirmed to still hold.

Ordered tackle list. **Complexity**: XS ≈ minutes, one edit site · S ≈ an hour-ish, one module plus
tests/docs · M ≈ multi-site change needing real design care. **Clarity**: Clear = mechanical,
direction fully specified · Mostly clear = one small implementation choice with an obvious default ·
Needs decision = genuine open question to settle before implementation. Ordering: severity first;
within severity, clear quick wins before larger items, decision-blocked items last.

Row 1 subsumes row 10; do row 10 for free, or not at all. Rows 2 and 3 both touch `usage.rs` and
should be one pass over that module even though they are separate defects.

| #  | ID     | Issue (anchor)                                                                | Severity  | Cx   | Clarity        | Open decision |
|----|--------|-------------------------------------------------------------------------------|-----------|------|----------------|---------------|
| 1  | C1     | Top-level tray window + handle `TaskbarCreated`; re-add icon **and** warning state (`tray.rs:920`) | Critical  | M    | Mostly clear   | — |
| 2  | I3     | Let the owner destroy the usage window, not its own wnd_proc (`usage.rs:95`)   | Important | S    | Mostly clear   | — |
| 3  | I2     | Scale the usage window for DPI; set a font; centre on the cursor's monitor (`usage.rs:151`) | Important | S    | Mostly clear   | Which monitor to centre on |
| 4  | I4     | Daisy-chained physical monitors collapse onto one `MonitorId` (`ddc_worker.rs:178`) | Important | XS–M | Needs decision | Log-and-document vs. represent it in `MonitorId` |
| 5  | I1     | `TrayIcon`/`PowerEventListener` `Drop` unreachable on every exit (`tray.rs:1033`) | Important | S    | Needs decision | Shutdown handshake vs. explicit teardown + delete the `Drop`s |
| 6  | HYG-1  | `cargo fmt --check` fails on a stray blank line (`config.rs:1099`)             | Cosmetic  | XS   | Clear          | — |
| 7  | DOC-6  | Mark rows 18/19 resolved in the 2026-07-26 review                             | Cosmetic  | XS   | Clear          | — |
| 8  | TRAY-1 | `NIM_SETVERSION` missing, so `NIF_SHOWTIP` is inert (`tray.rs:556`)            | Cosmetic  | XS   | Clear          | — |
| 9  | TRAY-2 | Icon AND mask built from undefined `CreateBitmap` bits (`tray.rs:454`)         | Cosmetic  | XS   | Clear          | — |
| 10 | TRAY-3 | `SetForegroundWindow` on a message-only window (`tray.rs:679`)                 | Cosmetic  | XS   | Clear          | Subsumed by row 1 |
| 11 | HK-1   | `Display for ParsedHotkey` renders letters/digits as "Unknown" (`hotkey.rs:457`) | Cosmetic  | XS   | Clear          | — |
| 12 | DEP-4  | Trim `env_logger` default features; drops the regex chain (`Cargo.toml:46`)    | Cosmetic  | S    | Mostly clear   | Accept the `RUST_LOG` regex-filter loss |
| 13 | DOC-2  | _(carried)_ Reconcile the `monitors` round-trip claim with `MonitorConfig`     | Cosmetic  | S    | Mostly clear   | Loosen the type vs. soften the doc |
| 14 | DEP-2  | _(carried)_ Confirm `winres` upstream; swap to `winresource` or pin-note       | Cosmetic  | S    | Needs decision | Requires a crates.io check first |

**One lesson this cycle offers the next.** C1 is the strongest argument yet for treating a fixed bug
as a *class* rather than a site. The message-only-window trap has now cost this project three
separate features across two modules, and each fix was scoped to the file where the symptom appeared.
The generalisable move after any Win32 fix of this kind is a grep for the *mechanism*
(`HWND_MESSAGE`, `SetForegroundWindow`, `RegisterWindowMessage`, broadcast-dependent messages), not
just a test on the file that broke. Second, and cheaper: all four Important findings live in the two
modules the previous review explicitly recorded as "pattern-swept … not line-audited end to end."
That flag was accurate, and acting on it is what produced this pass.

## Categories reviewed

| Category | Status | Reason / scope |
|---|---|---|
| Structure & Modularity | Applicable | Reviewed; core/platform boundary re-verified by grep (0 `unsafe`, 0 `windows::` under `core/`), platform→core direction holds, no cycles. Source of I1. |
| Dependencies | Applicable | Reviewed; 6 runtime + 1 build dep, all used; 46-package tree analysed. Source of DEP-4; DEP-2 re-confirmed open. |
| Data Flow & State | Conditional → applicable | The app's central concern. Optimistic-set reconciliation, `RefreshTracker` generations, ghost pruning, `DdcHealth` and the seeding path all re-read and sound. Source of I4. |
| Interfaces & Contracts | Conditional → narrow | Only external contracts: `config.json`, the hotkey/tray UX, the single-instance mutex name. No code consumers ⇒ no API/SemVer review. Source of HK-1. |
| Error Handling & Resilience | Applicable | Both halves. Supervision, watchdogs, retry and the never-fatal config policy verified end to end. **No new findings — the strongest area of the codebase.** |
| Testability & Tests | Applicable | Reviewed **and executed** (222 passed, 1 ignored by design, 0 failed). Seam-based fakes keep the control flow host-testable; remaining gaps are exactly the ones the docs already name. |
| Configuration & Secrets | Applicable | Atomic writes, `.bak` recovery chain, unknown-key diff and validation-to-defaults verified. Secret scan clean across the tracked tree; `personal/` and `venv/` are git-ignored and untracked. |
| Operations & Observability | Conditional → applicable | Long-running background process. Logging, rotation, panic hook and the release workflow reviewed. Source of C1's warning-channel consequence. |
| Security | Conditional → local-app depth | No network or service surface ⇒ no AuthN/AuthZ review. Trust boundaries checked: EDID parsing (no panic path; the 5-bit manufacturer decode is provably ASCII-bounded), registry read, config file, window messages, opt-in LL hook, single-instance object. Dependency scanning present (weekly `cargo audit` + Dependabot). |
| Documentation & Evolvability | Applicable | Reviewed; doc discipline is exceptional — intent is reconstructible almost everywhere, which is why the drift items are worth naming. Churn hotspots (`main.rs` 90, `osd.rs` 57, `mod.rs` 43) are all well-commented; no unexplained load-bearing code found. |

## What I could not verify

- **The Explorer-restart symptom itself** (C1) — reasoned from the Win32 broadcast contract plus the
  code, not reproduced. Decisive check: restart Explorer from Task Manager and see whether the icon
  returns. Worth adding to `architecture.md` §14's manual list once fixed.
- **Ghost-icon persistence after Quit** (I1) — shell-dependent timing. The structural claim (`Drop`
  unreachable on every path) is verified from code; the visible symptom is not.
- **Usage-window appearance above 100% scaling** (I2) — the missing DPI scaling is verified by code;
  how bad it looks is not measured. One screenshot at 150% settles it.
- **`HWND` recycling actually colliding** (I3) — the ownership violation is certain, the collision is
  probabilistic and was not provoked.
- **Daisy-chained N:1 physical monitors** (I4) — no such hardware available.
- **`winres`/`winresource` upstream status and dates** (row 14) — no network access this session;
  unchanged from the previous cycle's note.
- **Live RustSec advisory state** — `cargo audit` not installed locally; CI's weekly job remains the
  standing authority.
- **`cargo clippy -D warnings`** — not run this session. The tree currently fails
  `cargo fmt --check` (row 6), which should be cleared first. `cargo test` **was** run and is green.
- **The outstanding manual passes the project's own docs call for**: the sleep/wake System Resume
  Test, and the full hardware pass for the `windows` 0.62 migration (DDC, OSD, overlay, tray,
  hotkeys, power events).
- `osd_render.rs` and the ~450-line test block in `config.rs` were read at the seams, not
  line-audited end to end.
