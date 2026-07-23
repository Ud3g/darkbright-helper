# Design: Testable controller orchestration (`core/controller.rs`)

**Date:** 2026-07-23 (revised same day across three adversarial cold-review rounds, two
independent reviewers)
**Origin:** `docs/architecture-review-2026-07-19.md`, Finding #1 (untested, structurally
untestable controller orchestration in `main.rs`) plus Finding #3 (monitor hot-unplug
leaves permanent ghost state), which lives inside one of the methods being moved.

## Goals

1. Move the `BrightnessController` orchestration out of the binary crate into
   platform-agnostic `src/core/controller.rs`, generic over four narrow trait seams, so the
   optimistic-update, supervision, watchdog, and refresh sequences become unit-testable with
   fakes on any host (`cargo test` without a Windows target).
2. While moving `handle_ddc_refresh_result`, fix ghost state on monitor unplug: prune
   monitors whose absence from the *enumerated* (physically identified) monitor set has
   been observed spanning a sustained window (≥ 90 s) across at least two
   current-generation refreshes.
3. Shrink `main.rs` to thin wiring plus trivial shell handlers.

## Non-goals

- No behavior change other than: the pruning above **including its deliberate-forgetting
  consequences** (overlay dim level and cached brightness do not survive a > 90 s absence;
  see "Consequences" under Ghost pruning), the periodic-refresh gate change that sustains
  pruning while undocked, the refresh triggered from `handle_adjust`'s `MonitorNotFound`
  path (recovery after pruning, see "Consequences"), clearing `osd_monitor` when its
  monitor is pruned, and the sub-millisecond timestamp shift described under
  "Time injection".
- No supervision of the hotkey/tray/power threads (Finding #2), no config atomicity
  (Finding #4), no other review findings. In particular, `MonitorId::Display` keeps its
  serial (Finding #8 is separate work); this spec merely avoids *adding* new serial-bearing
  log lines above `debug`.
- The existing per-window `DimmingOverlay` trait in `platform/mod.rs` is untouched; the new
  overlay seam is manager-level.

## Architecture

New module `src/core/controller.rs` containing:

- the opaque handle newtype and the four seams (below),
- `Controller<Osd, Ovl, Ddc, Loc>` — all fields of today's `BrightnessController` except
  `usage_window`, which stays in `main.rs`,
- fakes and sequence tests under `#[cfg(test)]`.

`RespawnOutcome` moves from `platform/windows/ddc_worker.rs` to `core/reconcile.rs`
(supervision domain). `ddc_worker.rs` keeps the platform-facing path alive with
`pub use crate::core::reconcile::RespawnOutcome;` so the existing re-export in
`platform/windows/mod.rs` still resolves.

The handler methods (`handle_adjust`, `handle_ddc_set_result`, `handle_ddc_refresh_result`,
`handle_refresh`, `check_periodic_refresh`, `check_inactivity_refresh`, `supervise_worker`,
`check_watchdogs`, `reconcile_all_pending`, `show_error_on_visible_osd`, `clear_degraded`,
`build_tray_menu_data`, message dispatch) move largely verbatim. The no-op
`handle_set_absolute` and `handle_shutdown` move as-is.

**Message dispatch split:** `main.rs` handles `TrayOpenUsage` / `TrayOpenSettings` locally
(one-way Windows side effects with no decision logic) and forwards every other message to
`controller.handle_message`. The core match still carries arms for the two shell variants:
they are debug-log no-ops — visible in logs if wiring ever regresses, but never a panic
(`unreachable!` is explicitly not used).

**Trait impls live on the existing concrete types** — no wrapper adapters:
`impl OsdSink for OsdWindow` (osd.rs), `impl OverlaySink for OverlayManager` (overlay.rs),
`impl DdcPort for DdcSupervisor` (ddc_worker.rs); each is pure forwarding plus
`MonitorHandle` ↔ `HMONITOR` conversion. One new unit struct `CursorLocator` in
`platform/windows/mod.rs` wraps the free functions `get_monitor_under_cursor` and
`get_monitor_id`.

**`main.rs` afterwards:** wiring (config load, thread spawns,
`Controller::new(config, osd, overlay, ddc, locator, now)`), the event loop, and the small
shell match described above. With the OSD injected, `Controller::new` has no remaining
failure path and becomes **infallible** (today's `Result` existed only because `new`
created the OSD; that creation now happens in `main` before injection).

## Seams

Modeled exactly on today's call sites, no wider. All public items carry `///` docs with
`# Errors` sections (clippy pedantic gates on this); omitted here for brevity.

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MonitorHandle(pub isize);   // opaque; Windows side converts to/from HMONITOR

pub trait OsdSink {
    fn show(&mut self, handle: MonitorHandle, state: &MonitorState) -> Result<()>;
    fn update(&mut self, state: &MonitorState) -> Result<()>;
    fn update_error(&mut self, state: &MonitorState) -> Result<()>;
    fn is_visible(&self) -> bool;
}

pub trait OverlaySink {
    fn update(&mut self, id: &MonitorId, handle: MonitorHandle, opacity: u8) -> Result<()>;
    /// Removes a monitor's overlay window entirely (unplug pruning).
    fn remove(&mut self, id: &MonitorId);
}

pub trait DdcPort {
    fn send(&mut self, cmd: DdcCommand) -> Result<()>;   // SendError mapped to BrightnessError::ChannelSend
    fn is_alive(&self) -> bool;
    fn respawn(&mut self, now: Instant) -> RespawnOutcome;
    fn clear_backoff(&mut self);
    fn shutdown(&self);
}

pub trait MonitorLocator {
    fn monitor_under_cursor(&self) -> Result<MonitorHandle>;
    fn resolve_id(&self, handle: MonitorHandle) -> Result<MonitorId>;
}
```

The `HMONITOR → MonitorId` cache stays inside the controller as
`HashMap<MonitorHandle, MonitorId>`; locator fakes need no caching.

## Time injection

No clock trait. Public controller methods take an explicit `now: Instant`
(`handle_message(msg, now)`, `handle_refresh(now)` — public because `main` calls it
directly for the startup enumeration — `check_periodic_refresh(now)`,
`supervise_and_watchdog(now)`), and `Controller::new(..., now)` stamps `last_activity`,
`RefreshTracker::new`, and `last_health_check` from its parameter. Internal
`Instant::now()` calls **in controller-owned code** are replaced by the passed `now`;
elapsed-time checks use `now.saturating_duration_since(..)` (never bare subtraction — the
codebase avoids that panic surface throughout, and `check_inactivity_refresh`'s current
`self.last_activity.elapsed()` is a hidden clock that must be converted the same way).
Explicitly out of scope: `MonitorState`'s own internal `Instant::now()` stamps touch only
its write-only `last_refresh` field — they stay as-is (no signature churn, no test impact);
removing that dead field is separate cleanup.

**Capture point:** the main loop captures `Instant::now()` immediately before each
controller call — i.e., after `recv_timeout` returns for `handle_message`, and right before
each tick call. Staleness is therefore sub-millisecond call overhead, not the up-to-16 ms
`recv_timeout` block or unbounded message-pump time that a capture-at-loop-top would add.

Tests build instants as `base + Duration`, the same pattern as the existing `reconcile.rs`
tests.

## Error handling

Unchanged. Handlers keep returning `Result<_, BrightnessError>`; the loop logs. The
send-failure revert (today `main.rs:568-578`) moves verbatim and becomes testable via a
fake DDC port with a scriptable send failure.

## Changes to existing APIs

Complete list — everything else moves or forwards without signature change:

1. `RefreshTracker::complete` becomes `complete(generation, now, found, enumerated_any) ->
   bool` — the return says whether the result was accepted as current, so pruning can never
   act on a stale or aborted refresh (`abort()` bumps the generation, which keeps this
   sound). The new `enumerated_any` flag is recorded and exposed via `last_enumerated()`;
   it initializes `true` (like `last_successful`) and `abort()` clears it alongside
   `last_successful`, so quiescence after aborts/disabled DDC is preserved.
2. `OverlayManager` gains `remove(&mut self, id: &MonitorId)` (drops the `WindowsOverlay`;
   its existing RAII `Drop` destroys the window on the owning thread).
3. `RespawnOutcome` relocates to `core/reconcile.rs` (re-export preserved, see above).
4. `BrightnessMessage::DdcRefreshResult` gains an `enumerated: Vec<MonitorId>` field, and
   the DDC worker populates it (next section).
5. `MonitorState` gains a `missing_since: Option<Instant>` field (default `None`) — the
   timestamp of the first observation in the current run of enumeration absence.
6. `check_periodic_refresh` gates on `refresh.last_enumerated()` instead of
   `last_successful()` (see "Ghost pruning" for why and what it changes).

## Ghost pruning (Finding #3)

**Why the protocol must change:** today's `DdcRefreshResult.monitors` contains only
monitors whose *brightness read succeeded* (`ddc_worker.rs` pushes results only in the `Ok`
arm). Absence therefore means "momentarily unreadable" (standby, EDID-emulating KVM, DDC hiccup
surviving all 3 retries) at least as often as "unplugged". (Ordinary KVMs without EDID
emulation remove the display from the topology entirely — for those, a switch-away is
genuine absence; see the deliberate-forgetting consequence below.) Pruning on absence from the readable set
would destroy a deliberately dimmed monitor's black overlay on a transient read failure —
and the motivating undock scenario (empty readable set) would never prune at all.

**Worker change:** during `handle_refresh_all`, the worker additionally collects
`enumerated: Vec<MonitorId>` — every monitor whose identification (`get_monitor_id`)
succeeded this pass, regardless of whether the subsequent brightness read succeeded. The
push happens **immediately after `get_monitor_id` succeeds — before the physical-monitor
handle is opened**: a `get_physical_monitors` or read failure counts as
present-but-unreadable, never as topology absence (pushing on overall per-monitor success
instead would silently reclassify handle-open failures as unplugs — the exact bug class
this design exists to avoid, and one no automated test would catch since the worker is
manual-test-only). By construction `enumerated ⊇` the readable set. No `MonitorState` is created for
enumerated-but-unreadable monitors (there is no brightness value to hold); enumeration only
proves physical presence. When `enumerate_monitors()` itself fails, the worker sends the
usual empty result with `enumerated: vec![]` — the empty-set guard below then retains
everything. Duplicates in `enumerated` are permitted and order is irrelevant: serial-less
identical monitors already collapse to a single `MonitorId` and a single state, which stays
enumerated while either physical unit remains. `found` / `last_successful` semantics stay
keyed to the readable set.

**Controller rule**, in the moved `handle_ddc_refresh_result`:

1. Ground-truth brightness values are applied to `states` for **every** reported readable
   monitor, regardless of generation (unchanged: a hardware value is true no matter which
   refresh produced it).
2. `let current = self.refresh.complete(generation, now, found, !enumerated.is_empty());`
3. Only if `current && !enumerated.is_empty()`:
   - for each state whose id is **in** `enumerated`: reset `missing_since` to `None`;
   - for each state whose id is **absent**: if `missing_since` is `None`, stamp it with
     `now`; otherwise, if `now.saturating_duration_since(missing_since)` ≥
     `PRUNE_ABSENCE_WINDOW` (new constant in `core/reconcile.rs`, 90 s), prune: remove the
     state, call `overlay.remove(id)` (otherwise Windows may migrate the orphaned
     fullscreen black window onto a remaining monitor), clear `osd_monitor` if it pointed
     at the pruned monitor, and drop any `id_cache` entries mapping to the pruned id (a
     recycled `HMONITOR` value must not resurrect the ghost). Log at `info` using the
     serial-free `MonitorId::base_display_name()`; the serial-bearing form only at `debug`
     (per the project's PII rule). Serial-less identical monitors collapse to one state;
     same-model units with *distinct* serials keep separate states sharing one base name,
     so the `info` line may be ambiguous between them — accepted, the `debug` line carries
     the serial. Do not "fix" this by re-adding the serial at `info`.
4. Empty `enumerated` set, or a stale/aborted generation (`current == false`) ⇒
   `missing_since` untouched, nothing pruned: no information is treated as no evidence.
5. **Absence evidence is discarded on pipeline disruption:** every `missing_since` resets
   to `None` on `SystemResumed` and on a worker respawn (both `RespawnOutcome` arms).
   Rationale: refresh bursts around resume/respawn (immediate resume refresh plus an
   activity-triggered follow-up) can observe two misses within seconds while a dock's DP
   link is still training (~5–30 s of genuine absence from the display topology); absence
   evidence must span an undisturbed window. The time-based window makes a burst alone
   insufficient to prune; the reset additionally discards evidence that straddles a sleep.

**Periodic-refresh gate change (required for the undock case):** `check_periodic_refresh`
now skips only when the last completed refresh's *enumerated* set was empty
(`refresh.last_enumerated()`), no longer when merely no monitor was *readable*. Undocked,
the internal panel is typically identifiable but not DDC-readable: under the old gate the
first post-undock refresh (readable = ∅ ⇒ `last_successful = false`) froze the periodic
timer, so the observation completing the prune never arrived and ghost rows persisted for
the whole undocked session. The adjust-time activity retrigger stays keyed to
`last_successful` (unchanged). Cost: while monitors are enumerable but unreadable, the
periodic refresh keeps running at its normal cadence — a few hundred milliseconds of
failing DDC I/O per minute, on the worker thread.

**Consequences, stated deliberately:**

- Ghost tray rows disappear once absence has been observed spanning ≥ 90 s — at the default
  60 s cadence that is the third missing observation, worst case ~3 min after unplug
  (misses land at +60/+120/+180 s when the unplug follows a completion) — instead of
  lingering forever. Transient glitches and resume/respawn refresh bursts are non-events by
  construction. Cadence scaling is declared, not hidden: `periodic_seconds = 3600` (valid)
  stretches prune latency to ~2 h; `periodic_seconds = 0` ("disabled", also valid) leaves
  only adjust-time/inactivity refreshes as observations, so ghosts can persist indefinitely
  under that config — a declared limitation of the fix, matching that config's general
  "no background resync" contract.
- **Pruning forgets, deliberately.** Today's ghost state incidentally preserves the
  sub-zero overlay level and cached brightness across a redock, and a first hotkey press
  re-shows the OSD from the ghost cache. After a > 90 s absence — undock/redock, a DP
  monitor powered off long enough to leave the topology, or an ordinary non-EDID-emulating
  KVM switch-away — the monitor returns as a fresh `MonitorState` from the hardware read
  with `overlay_opacity = 0`: the deliberate dim does not survive the round trip. A hotkey
  press on a monitor with no state yields `MonitorNotFound` with no OSD — and to make
  recovery topology-independent, the `MonitorNotFound` path in `handle_adjust` now also
  triggers a refresh (gated on none in flight). Without that trigger, only the
  empty-readable case would self-heal (the activity retrigger is keyed to
  `last_successful`): in a partial replug with surviving readable monitors the press would
  stay dead for up to `periodic_seconds`, with repeated presses even suppressing the
  inactivity path by resetting `last_activity`. With it, the healing refresh lands in
  ~1–2 s and the following press works, in every topology. This is the accepted price of
  removing ghost state; standby monitors and EDID-emulating KVMs stay enumerated and are
  unaffected.
- Between unplug and prune (bounded by window + cadence, ≤ ~3 min at defaults), an
  orphaned *visible* overlay can migrate onto a surviving monitor as a topmost
  click-through black sheet — exactly as today, but now bounded instead of permanent;
  `overlay.remove` ends it.
- A visible OSD showing a pruned monitor keeps its stale rendering until the auto-hide
  timer fires; no `OsdSink::hide` is added. Clearing `osd_monitor` disables
  `show_error_on_visible_osd` restyling until the next adjust — acceptable, the monitor is
  gone.
- A late `DdcSetResult` for a pruned monitor hits the existing unknown-monitor warn path
  and is dropped; with pruning this becomes a routine occurrence, covered by a test.
- Residual limitation: monitors whose identification itself fails cannot appear in
  `enumerated`; if the *entire* set is unidentifiable, ghosts persist for that period
  (conservative fail-safe, same spirit as the empty-result guard).

## Testing

Fakes in `core/controller.rs` `#[cfg(test)]` record calls (OSD show/update/error +
visibility flag, overlay updates/removes, DDC commands with scriptable send failure /
liveness / respawn outcome, locator with fixed handle→id). Sequences covered:

| Sequence | verifies |
|---|---|
| Optimistic adjust | pending set, overlay + OSD updated, `DdcCommand` sent with correct `seq` |
| Adjust on unknown monitor | `MonitorNotFound` error path; triggers a refresh when none in flight (recovery after pruning), no refresh when one is in flight |
| Confirm / revert / stale | OSD update on confirm, OSD error on revert, stale ignored; timeout-counter reset |
| Set result for pruned/unknown monitor | warn + drop, no panic, no ghost resurrection |
| Send failure | failed `send()` ⇒ `force_revert` + OSD error |
| Watchdog | `SET_TIMEOUT` exceeded ⇒ revert; ≥ `HUNG_TIMEOUT_LIMIT` ⇒ `ddc_disabled` |
| Supervision | dead worker ⇒ respawn + `reconcile_all_pending` + refresh; `BackoffExceeded` ⇒ `ddc_disabled` |
| Refresh lifecycle | generation gating, `REFRESH_TIMEOUT` abort, inactivity/periodic triggers |
| Ghost pruning | absence spanning ≥ `PRUNE_ABSENCE_WINDOW` ⇒ state removed + `overlay.remove` + `id_cache` cleaned; reappearance resets `missing_since`; two misses seconds apart (resume/respawn burst) ⇒ no prune; `SystemResumed`/respawn ⇒ evidence reset; empty `enumerated` or stale generation ⇒ timestamps untouched; monitor re-added after prune starts fresh (`overlay_opacity = 0`) |
| Undock cadence | readable = ∅ but `enumerated` non-empty ⇒ periodic refresh keeps running (gate on `last_enumerated`), pruning completes; `enumerated` = ∅ or `abort()` ⇒ periodic stays frozen as today |
| Degraded recovery | adjust while `ddc_disabled` ⇒ `clear_degraded`; `SystemResumed` ⇒ clear + refresh |
| Tray menu | display names + values from states |

Plus new `RefreshTracker` tests for the `bool` return of `complete` and the
`last_enumerated()` lifecycle (init `true`, cleared by `abort()`, untouched by stale
completions). The worker-side
`enumerated` collection is FFI code and stays under the manual integration checks
(architecture.md "Integration Testing"), with the unplug/replug and monitor-standby cycles
added to that checklist. Existing `state.rs` tests are untouched; the four `complete` call
sites across three `reconcile.rs` tests gain the new argument (call-site updates only,
assertions unchanged). Gates: `cargo fmt -- --check`,
`cargo clippy -- -D warnings`, `cargo test`.

## Documentation

`docs/architecture.md` (the source of truth) is updated in the same change: module map
gains `core/controller.rs`; the refresh section documents the enumerated-vs-readable
distinction, the 90 s absence-window pruning rule with its disruption resets, and the new
periodic-gate condition; the `MonitorState` struct listing gains the `missing_since` field;
the testing section describes the seam-based controller tests replacing the "orchestration
is manual-only" status quo.
