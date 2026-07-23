# Design: Testable controller orchestration (`core/controller.rs`)

**Date:** 2026-07-23
**Origin:** `docs/architecture-review-2026-07-19.md`, Finding #1 (untested, structurally
untestable controller orchestration in `main.rs`) plus Finding #3 (monitor hot-unplug
leaves permanent ghost state), which lives inside one of the methods being moved.

## Goals

1. Move the `BrightnessController` orchestration out of the binary crate into
   platform-agnostic `src/core/controller.rs`, generic over four narrow trait seams, so the
   optimistic-update, supervision, watchdog, and refresh sequences become unit-testable with
   fakes on any host (`cargo test` without a Windows target).
2. While moving `handle_ddc_refresh_result`, fix ghost state on monitor unplug: prune
   monitors absent from a current-generation, non-empty refresh result.
3. Shrink `main.rs` to thin wiring plus trivial shell handlers.

## Non-goals

- No behavior change other than the Finding #3 pruning (and the microsecond-level timestamp
  shift described under "Time injection").
- No supervision of the hotkey/tray/power threads (Finding #2), no config atomicity
  (Finding #4), no other review findings.
- The existing per-window `DimmingOverlay` trait in `platform/mod.rs` is untouched; the new
  overlay seam is manager-level.

## Architecture

New module `src/core/controller.rs` containing:

- the opaque handle newtype and the four seams (below),
- `Controller<Osd, Ovl, Ddc, Loc>` — all fields of today's `BrightnessController` except
  `usage_window`, which stays in `main.rs`,
- fakes and sequence tests under `#[cfg(test)]`.

`RespawnOutcome` moves from `platform/windows/ddc_worker.rs` to `core/reconcile.rs`
(supervision domain); `ddc_worker.rs` imports it from there.

The handler methods (`handle_adjust`, `handle_ddc_set_result`, `handle_ddc_refresh_result`,
`handle_refresh`, `check_periodic_refresh`, `check_inactivity_refresh`, `supervise_worker`,
`check_watchdogs`, `reconcile_all_pending`, `show_error_on_visible_osd`, `clear_degraded`,
`build_tray_menu_data`, message dispatch) move largely verbatim.

**Trait impls live on the existing concrete types** — no wrapper adapters:
`impl OsdSink for OsdWindow` (osd.rs), `impl OverlaySink for OverlayManager` (overlay.rs),
`impl DdcPort for DdcSupervisor` (ddc_worker.rs); each is pure forwarding plus
`MonitorHandle` ↔ `HMONITOR` conversion. One new unit struct `CursorLocator` in
`platform/windows/mod.rs` wraps the free functions `get_monitor_under_cursor` and
`get_monitor_id`.

**`main.rs` afterwards:** wiring (config load, thread spawns,
`Controller::new(config, osd, overlay, ddc, locator)`), the event loop, and a small shell
match that handles `TrayOpenUsage` / `TrayOpenSettings` locally (these are one-way Windows
side effects with no decision logic) and forwards every other message to
`controller.handle_message`.

## Seams

Modeled exactly on today's call sites, no wider:

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
    fn send(&mut self, cmd: DdcCommand) -> Result<()>;   // SendError mapped to BrightnessError
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
(`handle_message(msg, now)`, `check_periodic_refresh(now)`, `supervise_and_watchdog(now)`),
matching the existing `RefreshTracker` / `respawn_allowed` style. Internal `Instant::now()`
calls are replaced by the passed `now`. Tests build instants as `base + Duration`, the same
pattern as the existing `reconcile.rs` tests.

Accepted nuance: timestamps become "loop-tick time" instead of "mid-handler time" — a
microsecond-level shift against 5–8 s deadlines.

## Error handling

Unchanged. Handlers keep returning `Result<_, BrightnessError>`; the loop logs. The
send-failure revert (today `main.rs:568-578`) moves verbatim and becomes testable via a
fake DDC port with a scriptable send failure.

Single API change to existing code: `RefreshTracker::complete` returns `bool` — whether the
result was accepted as current — so pruning can never act on a stale or aborted refresh.

## Ghost pruning (Finding #3)

In the moved `handle_ddc_refresh_result`:

1. Ground-truth brightness values are applied to `states` for **every** reported monitor,
   regardless of generation (unchanged: a hardware value is true no matter which refresh
   produced it).
2. `let current = self.refresh.complete(generation, now, found);`
3. Only if `current && found` (current generation **and** non-empty result): remove every
   `states` key absent from the result. Per removed monitor: call `overlay.remove(id)`
   (otherwise Windows may migrate the orphaned fullscreen black window onto a remaining
   monitor), clear `osd_monitor` if it pointed at the removed monitor, and drop any
   `id_cache` entries mapping to the removed id (a recycled `HMONITOR` value must not
   resurrect the ghost). Log at `info`.
4. Empty result ⇒ retain everything, as today (a transient enumeration failure must not
   wipe state).

`OverlayManager::remove` drops the `WindowsOverlay` from its map; the existing RAII `Drop`
destroys the window.

## Testing

Fakes in `core/controller.rs` `#[cfg(test)]` record calls (OSD show/update/error +
visibility flag, overlay updates/removes, DDC commands with scriptable send failure /
liveness / respawn outcome, locator with fixed handle→id). Sequences covered:

| Sequence | verifies |
|---|---|
| Optimistic adjust | pending set, overlay + OSD updated, `DdcCommand` sent with correct `seq` |
| Confirm / revert / stale | OSD update on confirm, OSD error on revert, stale ignored; timeout-counter reset |
| Send failure | failed `send()` ⇒ `force_revert` + OSD error |
| Watchdog | `SET_TIMEOUT` exceeded ⇒ revert; ≥ `HUNG_TIMEOUT_LIMIT` ⇒ `ddc_disabled` |
| Supervision | dead worker ⇒ respawn + `reconcile_all_pending` + refresh; `BackoffExceeded` ⇒ `ddc_disabled` |
| Refresh lifecycle | generation gating, `REFRESH_TIMEOUT` abort, inactivity/periodic triggers |
| Ghost pruning | missing monitor ⇒ state removed + `overlay.remove`; empty or stale result ⇒ no pruning |
| Degraded recovery | adjust while `ddc_disabled` ⇒ `clear_degraded`; `SystemResumed` ⇒ clear + refresh |
| Tray menu | display names + values from states |

Plus new `RefreshTracker` tests for the `bool` return of `complete`. Existing tests remain
untouched. Gates: `cargo fmt -- --check`, `cargo clippy -- -D warnings`, `cargo test`.

## Documentation

`docs/architecture.md` (the source of truth) is updated in the same change: module map
gains `core/controller.rs`, and the testing section describes the seam-based controller
tests replacing the "orchestration is manual-only" status quo.
