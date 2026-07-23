# Testable Controller Orchestration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move the `BrightnessController` orchestration from `src/main.rs` into platform-agnostic `src/core/controller.rs` behind four trait seams, unit-test the previously untestable sequences with fakes, and fix ghost state on monitor unplug (90 s absence-window pruning).

**Architecture:** The core `Controller<Osd, Ovl, Ddc, Loc>` is built in parallel to the existing `BrightnessController` (which keeps `main.rs` compiling until the final switchover task), then `main.rs` is cut over to thin wiring. Time is injected as explicit `now: Instant` parameters; no clock trait. Spec (authoritative for every behavior decision): `docs/superpowers/specs/2026-07-23-controller-testability-design.md`.

**Tech Stack:** Rust 2024 (MSRV 1.87), `windows` crate 0.52, std mpsc, `log` kv syntax, thiserror.

## Global Constraints

- Windows-only binary; build/test on this Windows host with plain `cargo test`, `cargo build`.
- CI gates (must pass before every commit): `cargo fmt -- --check`, `cargo clippy --all-targets -- -D warnings` (clippy `all` + `pedantic` are warn-by-default in Cargo.toml), `cargo test`.
- `src/core/` must have **zero** `use crate::platform` — no Win32 types (`HMONITOR`, `HWND`) in core.
- No `as` casts: use `u32::from`, `try_from`, `.cast_unsigned()`/`.cast_signed()`.
- All public items need `///` docs with `# Errors` on `Result`-returning fns (clippy enforces). `#[must_use]` on pure fns not returning `Result`.
- Logging: kv style `log::info!(monitor_id:% = id, brightness = v; "...")`. Log at point of handling. Serial numbers only at `debug` level — the ghost-prune `info` line must use serial-free `MonitorId::base_display_name()`.
- Elapsed time: always `now.saturating_duration_since(earlier)`, never bare subtraction.
- Code comments: no planning labels (task numbers, spec section refs) — self-contained domain terms only.
- Commit messages: ≤ ~50 words, terse subject, **no** Co-Authored-By/AI trailer.

## File Structure

| File | Change | Responsibility |
|---|---|---|
| `src/core/state.rs` | Modify | `DdcRefreshResult` gains `enumerated`; `MonitorState` gains `missing_since` |
| `src/core/reconcile.rs` | Modify | `complete(..., enumerated_any) -> bool`, `last_enumerated()`, `PRUNE_ABSENCE_WINDOW`, relocated `RespawnOutcome` |
| `src/core/controller.rs` | **Create** | `MonitorHandle`, 4 trait seams, generic `Controller`, fakes + sequence tests |
| `src/core/mod.rs` | Modify | `pub mod controller;` |
| `src/platform/windows/ddc_worker.rs` | Modify | collect `enumerated`, re-export `RespawnOutcome` from core, `impl DdcPort for DdcSupervisor` |
| `src/platform/windows/osd.rs` | Modify | `impl OsdSink for OsdWindow` |
| `src/platform/windows/overlay.rs` | Modify | `OverlayManager::remove`, `impl OverlaySink for OverlayManager` |
| `src/platform/windows/mod.rs` | Modify | `CursorLocator` unit struct |
| `src/main.rs` | Modify (shrink) | wiring + shell handlers only; `BrightnessController` deleted |
| `docs/architecture.md` | Modify | module map, refresh/pruning protocol, `MonitorState` listing, testing section |

---

### Task 1: Refresh protocol — `enumerated` set + `missing_since` field

**Files:**
- Modify: `src/core/state.rs:348-353` (`DdcRefreshResult`), `src/core/state.rs:196-218` (`MonitorState`)
- Modify: `src/platform/windows/ddc_worker.rs:126-201` (worker collection)
- Modify: `src/main.rs:234-239` (destructure new field, ignore for now)

**Interfaces:**
- Produces: `BrightnessMessage::DdcRefreshResult { generation: u64, monitors: Vec<(MonitorId, u8)>, enumerated: Vec<MonitorId> }`; `MonitorState.missing_since: Option<Instant>` (pub, default `None`).
- Later tasks rely on: `enumerated ⊇` readable ids; enumeration failure sends `enumerated: vec![]`.

- [ ] **Step 1: Write the failing test** — append to `pending_reconcile_tests` in `src/core/state.rs`:

```rust
    #[test]
    fn new_state_has_no_absence_evidence() {
        let s = MonitorState::new(50);
        assert!(s.missing_since.is_none());
    }
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test new_state_has_no_absence_evidence`
Expected: COMPILE ERROR — `no field 'missing_since' on type 'MonitorState'`

- [ ] **Step 3: Implement.** In `src/core/state.rs`:

(a) Add to the `MonitorState` struct (after `last_refresh`):

```rust
    /// First observation of this monitor's current run of enumeration absence.
    ///
    /// `None` while the monitor is present (or absence was never observed).
    /// Stamped by the controller on the first current-generation refresh that
    /// does not enumerate the monitor; a later miss ≥ the prune window prunes.
    pub missing_since: Option<Instant>,
```

(b) In `MonitorState::new`, add `missing_since: None,` to the struct literal.

(c) In the `DdcRefreshResult` variant, add after `monitors`:

```rust
        /// Every monitor whose identification succeeded this pass, readable or
        /// not. Superset of `monitors`' ids; empty when enumeration itself
        /// failed. Presence proof for absence-based pruning.
        enumerated: Vec<MonitorId>,
```

- [ ] **Step 4: Update the worker.** In `src/platform/windows/ddc_worker.rs`:

(a) `handle_refresh_all` (lines 126-153) — thread the new vec through both exit paths:

```rust
    fn handle_refresh_all(&mut self, generation: u64) {
        log::debug!("Refreshing all monitors");

        // Clear existing state
        self.monitors.clear();
        self.handle_cache.clear();

        let mut results: Vec<(MonitorId, u8)> = Vec::new();
        let mut enumerated: Vec<MonitorId> = Vec::new();

        // Enumerate monitors
        let hmonitors = match enumerate_monitors() {
            Ok(h) => h,
            Err(e) => {
                log::error!(error:% = e; "Failed to enumerate monitors");
                // Send empty result
                self.send_refresh_result(generation, results, enumerated);
                return;
            }
        };

        for hmonitor in hmonitors {
            if let Err(e) = self.process_monitor(hmonitor, &mut results, &mut enumerated) {
                log::warn!(error:% = e; "Failed to process monitor");
            }
        }

        self.send_refresh_result(generation, results, enumerated);
    }
```

(b) `process_monitor` (lines 156-189) — new parameter; the push sits **immediately after `get_monitor_id` succeeds and before `get_physical_monitors`**. Placement is load-bearing: a physical-handle-open or read failure must count as present-but-unreadable, never as topology absence.

```rust
    fn process_monitor(
        &mut self,
        hmonitor: HMONITOR,
        results: &mut Vec<(MonitorId, u8)>,
        enumerated: &mut Vec<MonitorId>,
    ) -> crate::Result<()> {
        // Get monitor ID from EDID
        let monitor_id = get_monitor_id(hmonitor)?;
        // Identified ⇒ physically present. Push before opening the physical
        // handle: a handle-open or brightness-read failure below must count
        // as unreadable, not as absent from the topology.
        enumerated.push(monitor_id.clone());
        self.handle_cache.insert(hmonitor.0, monitor_id.clone());
```

(rest of the method body unchanged)

(c) `send_refresh_result` (lines 192-201):

```rust
    /// Sends refresh results back to the main thread.
    fn send_refresh_result(
        &self,
        generation: u64,
        monitors: Vec<(MonitorId, u8)>,
        enumerated: Vec<MonitorId>,
    ) {
        let msg = BrightnessMessage::DdcRefreshResult {
            generation,
            monitors,
            enumerated,
        };

        if let Err(e) = self.resp_tx.send(msg) {
            log::error!(error:% = e; "Failed to send refresh result");
        }
    }
```

- [ ] **Step 5: Fix the interim consumer.** In `src/main.rs:234-239`, the match arm becomes (old controller ignores the new field until switchover):

```rust
            BrightnessMessage::DdcRefreshResult {
                generation,
                monitors,
                enumerated: _,
            } => {
                self.handle_ddc_refresh_result(generation, monitors);
            }
```

- [ ] **Step 6: Run gates**

Run: `cargo test && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: all PASS (including the new test)

- [ ] **Step 7: Commit**

```powershell
git add src/core/state.rs src/platform/windows/ddc_worker.rs src/main.rs
git commit -m "feat: report enumerated monitor set in refresh results"
```

---

### Task 2: `RefreshTracker` — `enumerated_any`, `bool` return, prune window

**Files:**
- Modify: `src/core/reconcile.rs` (struct, `complete`, `abort`, new accessor, new constant, tests at lines 179/184/194/218)
- Modify: `src/main.rs:355-356` (call site)

**Interfaces:**
- Produces: `pub fn complete(&mut self, generation: u64, now: Instant, found: bool, enumerated_any: bool) -> bool` (true ⇔ result accepted as current); `pub fn last_enumerated(&self) -> bool` (init `true`, cleared by `abort()`); `pub const PRUNE_ABSENCE_WINDOW: Duration = Duration::from_secs(90);`

- [ ] **Step 1: Write the failing tests** — append to `tests` in `src/core/reconcile.rs`:

```rust
    #[test]
    fn complete_returns_true_only_for_current_generation() {
        let base = Instant::now();
        let mut t = RefreshTracker::new(base);
        let stale = t.begin(base);
        let current = t.begin(base);
        assert!(!t.complete(stale, base + Duration::from_secs(1), true, true));
        assert!(t.complete(current, base + Duration::from_secs(1), true, true));
    }

    #[test]
    fn last_enumerated_lifecycle() {
        let base = Instant::now();
        let mut t = RefreshTracker::new(base);
        // Starts true so the periodic gate is open before the first refresh.
        assert!(t.last_enumerated());

        let g = t.begin(base);
        assert!(t.complete(g, base, false, false));
        assert!(!t.last_enumerated(), "empty enumerated set closes the gate");

        let g = t.begin(base);
        assert!(t.complete(g, base, false, true));
        assert!(
            t.last_enumerated(),
            "enumerable-but-unreadable keeps the gate open"
        );

        let stale = t.begin(base);
        let _ = t.begin(base);
        assert!(!t.complete(stale, base, false, false));
        assert!(t.last_enumerated(), "stale completion must not touch the gate");

        t.abort();
        assert!(!t.last_enumerated(), "abort freezes the periodic path");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib reconcile`
Expected: COMPILE ERROR — `complete` takes 3 arguments / no method `last_enumerated`

- [ ] **Step 3: Implement.** In `src/core/reconcile.rs`:

(a) After the `HUNG_TIMEOUT_LIMIT` constant add:

```rust
/// Minimum continuously observed enumeration absence before a monitor's state
/// is pruned. Spans at least two refresh observations, so a resume/respawn
/// refresh burst (seconds apart, while a dock's DP link is still training)
/// can never prune on its own.
pub const PRUNE_ABSENCE_WINDOW: Duration = Duration::from_secs(90);
```

(b) Add field to `RefreshTracker` struct: `last_enumerated: bool,` — and in `new()`: `last_enumerated: true,`.

(c) Replace `complete` (lines 79-88):

```rust
    /// Records a completed refresh; results from a stale generation are ignored.
    ///
    /// Returns `true` when the result matched the current generation and was
    /// recorded — the caller's license to treat it as absence evidence.
    /// `enumerated_any` reports whether the refresh identified any monitor at
    /// all (readable or not); it drives the periodic-refresh gate.
    pub fn complete(&mut self, generation: u64, now: Instant, found: bool, enumerated_any: bool) -> bool {
        if generation != self.generation {
            return false;
        }
        self.in_progress = false;
        self.started_at = None;
        self.last_refresh = now;
        self.last_successful = found;
        self.last_enumerated = enumerated_any;
        true
    }
```

(d) In `abort()` add `self.last_enumerated = false;` after `self.last_successful = false;`.

(e) Add accessor next to `last_successful()`:

```rust
    /// Whether the last completed refresh enumerated any monitor (readable or not).
    #[must_use]
    pub fn last_enumerated(&self) -> bool {
        self.last_enumerated
    }
```

(f) Update the four existing call sites (three tests): `reconcile.rs:179` and `:184` (`refresh_tracker_begin_hands_out_distinct_generations_and_ignores_stale`), `:194` (`refresh_tracker_complete_records_failure`), `:218` (`refresh_tracker_abort_invalidates_outstanding_result`) — each `t.complete(g, when, flag)` becomes `t.complete(g, when, flag, flag)` (statement position; the new `bool` return needs no `let`). Assertions unchanged.

(g) Update `src/main.rs:355-356` — interim placeholder until switchover (the old controller never reads `last_enumerated`):

```rust
        self.refresh
            .complete(generation, Instant::now(), found_monitors, found_monitors);
```

- [ ] **Step 4: Run gates**

Run: `cargo test && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: all PASS

- [ ] **Step 5: Commit**

```powershell
git add src/core/reconcile.rs src/main.rs
git commit -m "feat: RefreshTracker tracks enumerated set, complete() returns currency"
```

---

### Task 3: Relocate `RespawnOutcome` to core

**Files:**
- Modify: `src/core/reconcile.rs` (receives the enum)
- Modify: `src/platform/windows/ddc_worker.rs:204-211` (enum removed, re-export added)

**Interfaces:**
- Produces: `crate::core::reconcile::RespawnOutcome` (unchanged shape: `Respawned | BackoffExceeded`, derives `Debug, Clone, Copy, PartialEq, Eq`). The platform path `crate::platform::windows::RespawnOutcome` keeps resolving via re-export chain (`ddc_worker.rs` → `platform/windows/mod.rs:39`).

- [ ] **Step 1: Move the enum.** Cut lines 204-211 from `src/platform/windows/ddc_worker.rs` and paste into `src/core/reconcile.rs` (after `respawn_allowed`), verbatim:

```rust
/// Result of a supervisor respawn attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RespawnOutcome {
    /// A fresh worker thread was spawned.
    Respawned,
    /// Too many respawns within the backoff window; the worker is left dead.
    BackoffExceeded,
}
```

- [ ] **Step 2: Keep the platform path alive.** In `src/platform/windows/ddc_worker.rs`, where the enum was, add (a `pub use`, not a plain `use` — `platform/windows/mod.rs:39` re-exports it from here):

```rust
pub use crate::core::reconcile::RespawnOutcome;
```

- [ ] **Step 3: Run gates**

Run: `cargo test && cargo build && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: all PASS — `main.rs:37`'s `use ...platform::windows::{..., RespawnOutcome, ...}` still resolves.

- [ ] **Step 4: Commit**

```powershell
git add src/core/reconcile.rs src/platform/windows/ddc_worker.rs
git commit -m "refactor: move RespawnOutcome to core::reconcile (supervision domain)"
```

---

### Task 4: `core/controller.rs` — seams, struct, fakes, refresh + pruning

**Files:**
- Create: `src/core/controller.rs`
- Modify: `src/core/mod.rs` (add `pub mod controller;`)
- Test: in-module `#[cfg(test)]` in `src/core/controller.rs`

**Interfaces:**
- Produces (used by every later task):
  - `pub struct MonitorHandle(pub isize)` — `Clone, Copy, PartialEq, Eq, Hash, Debug`
  - `pub trait OsdSink { fn show(&mut self, handle: MonitorHandle, state: &MonitorState) -> Result<()>; fn update(&mut self, state: &MonitorState) -> Result<()>; fn update_error(&mut self, state: &MonitorState) -> Result<()>; fn is_visible(&self) -> bool; }`
  - `pub trait OverlaySink { fn update(&mut self, id: &MonitorId, handle: MonitorHandle, opacity: u8) -> Result<()>; fn remove(&mut self, id: &MonitorId); }`
  - `pub trait DdcPort { fn send(&mut self, cmd: DdcCommand) -> Result<()>; fn is_alive(&self) -> bool; fn respawn(&mut self, now: Instant) -> RespawnOutcome; fn clear_backoff(&mut self); fn shutdown(&self); }`
  - `pub trait MonitorLocator { fn monitor_under_cursor(&self) -> Result<MonitorHandle>; fn resolve_id(&self, handle: MonitorHandle) -> Result<MonitorId>; }`
  - `pub struct Controller<Osd, Ovl, Ddc, Loc>` with `pub fn new(config: Config, osd: Osd, overlay: Ovl, ddc: Ddc, locator: Loc, now: Instant) -> Self` (infallible), `pub fn handle_refresh(&mut self, now: Instant)`, `pub fn check_periodic_refresh(&mut self, now: Instant)`, `pub fn shutdown_worker(&self)`
  - Private this task: `handle_ddc_refresh_result`, `apply_absence_evidence`, `reset_absence_evidence`, `check_inactivity_refresh`, `clear_degraded`
  - Test fakes `FakeOsd`, `FakeOverlay`, `FakeDdc`, `FakeLocator` + helpers `test_id()`, `test_controller(base)` (later tasks extend these tests; fakes record calls in `Vec`s)

- [ ] **Step 1: Create the module skeleton with fakes and the first failing tests.** Full initial content of `src/core/controller.rs`:

````rust
//! Platform-agnostic controller orchestration.
//!
//! Composes the tested core primitives (`apply_set_result`, `RefreshTracker`,
//! respawn backoff) into the message-driven control flow, generic over four
//! narrow seams so the sequences are unit-testable with fakes on any host.
//! All methods take an explicit `now: Instant`; the binary captures it
//! immediately before each call.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::core::config::Config;
use crate::core::reconcile::{PRUNE_ABSENCE_WINDOW, RefreshTracker, RespawnOutcome};
use crate::core::state::{DdcCommand, MonitorId, MonitorState};
use crate::error::Result;

// NOTE for later tasks — imports are added exactly when first used, to keep
// `-D warnings` green at every commit:
//   Task 5 adds: crate::core::brightness::calculate_adjustment;
//                SetOutcome (to the state import); BrightnessError (to the error import)
//   Task 6 adds: HUNG_TIMEOUT_LIMIT, REFRESH_TIMEOUT, SET_TIMEOUT (to the reconcile import)
//   Task 7 adds: BrightnessMessage, TrayMenuData, TrayMonitorInfo, generate_display_names
//                (to the state import)

/// Opaque per-monitor display handle.
///
/// Carries the platform's monitor handle value (`HMONITOR` on Windows) through
/// core without a platform type dependency; the platform seam implementations
/// convert at the boundary.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MonitorHandle(pub isize);

/// Seam for the on-screen display window.
pub trait OsdSink {
    /// Shows the OSD on the given monitor with the state's current values.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform window cannot be shown.
    fn show(&mut self, handle: MonitorHandle, state: &MonitorState) -> Result<()>;

    /// Redraws the visible OSD with the state's current values.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform window cannot be redrawn.
    fn update(&mut self, state: &MonitorState) -> Result<()>;

    /// Restyles the visible OSD to its error state.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform window cannot be redrawn.
    fn update_error(&mut self, state: &MonitorState) -> Result<()>;

    /// Whether the OSD is currently visible.
    fn is_visible(&self) -> bool;
}

/// Seam for the per-monitor dimming overlay manager.
pub trait OverlaySink {
    /// Creates/positions the monitor's overlay and applies `opacity` (0-100).
    ///
    /// # Errors
    ///
    /// Returns an error if the platform window cannot be created or updated.
    fn update(&mut self, id: &MonitorId, handle: MonitorHandle, opacity: u8) -> Result<()>;

    /// Removes a monitor's overlay window entirely (unplug pruning).
    fn remove(&mut self, id: &MonitorId);
}

/// Seam for the supervised DDC worker.
pub trait DdcPort {
    /// Sends a command to the worker.
    ///
    /// # Errors
    ///
    /// Returns an error if the worker's channel is closed (worker died).
    fn send(&mut self, cmd: DdcCommand) -> Result<()>;

    /// Whether the worker thread is still running.
    fn is_alive(&self) -> bool;

    /// Attempts to respawn a dead worker, honouring the backoff window.
    fn respawn(&mut self, now: Instant) -> RespawnOutcome;

    /// Clears the respawn history so recovery can retry immediately.
    fn clear_backoff(&mut self);

    /// Asks the worker to shut down (best-effort).
    fn shutdown(&self);
}

/// Seam for resolving the monitor under the mouse cursor.
pub trait MonitorLocator {
    /// Returns the handle of the monitor under the cursor.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform query fails.
    fn monitor_under_cursor(&self) -> Result<MonitorHandle>;

    /// Resolves a monitor handle to its EDID-based identity.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform identification fails.
    fn resolve_id(&self, handle: MonitorHandle) -> Result<MonitorId>;
}

/// Main controller for brightness management.
///
/// Owns all `MonitorState` and drives OSD/overlay/DDC through the seams.
/// Single-threaded: the binary's main loop is the only caller.
pub struct Controller<Osd, Ovl, Ddc, Loc> {
    /// Current state (brightness, overlay, absence evidence) per monitor.
    states: HashMap<MonitorId, MonitorState>,
    /// Dimming overlay windows.
    overlay: Ovl,
    /// On-screen display.
    osd: Osd,
    /// Loaded configuration.
    config: Config,
    /// Cache mapping platform handles to monitor ids (avoids repeated EDID reads).
    id_cache: HashMap<MonitorHandle, MonitorId>,
    /// Supervised DDC worker.
    ddc: Ddc,
    /// Cursor-to-monitor resolution.
    locator: Loc,
    /// Timestamp of last user-initiated brightness adjustment.
    last_activity: Instant,
    /// Refresh lifecycle: in-flight state, generation, and last outcome.
    refresh: RefreshTracker,
    /// Monotonic sequence id stamped on each DDC set command.
    next_seq: u64,
    /// Throttle for the per-tick supervision/watchdog pass.
    last_health_check: Instant,
    /// Consecutive set timeouts while the worker is still alive (hang signal).
    consecutive_set_timeouts: u32,
    /// True when DDC is disabled after respawn backoff or a diagnosed hang.
    ddc_disabled: bool,
    /// Monitor whose state the OSD is currently showing (for error restyling).
    osd_monitor: Option<MonitorId>,
}

impl<Osd, Ovl, Ddc, Loc> Controller<Osd, Ovl, Ddc, Loc>
where
    Osd: OsdSink,
    Ovl: OverlaySink,
    Ddc: DdcPort,
    Loc: MonitorLocator,
{
    /// Creates a controller; `now` stamps the activity/health/refresh baselines.
    #[must_use]
    pub fn new(config: Config, osd: Osd, overlay: Ovl, ddc: Ddc, locator: Loc, now: Instant) -> Self {
        Self {
            states: HashMap::new(),
            overlay,
            osd,
            config,
            id_cache: HashMap::new(),
            ddc,
            locator,
            last_activity: now,
            refresh: RefreshTracker::new(now),
            next_seq: 0,
            last_health_check: now,
            consecutive_set_timeouts: 0,
            ddc_disabled: false,
            osd_monitor: None,
        }
    }

    /// Asks the supervised DDC worker to shut down.
    pub fn shutdown_worker(&self) {
        self.ddc.shutdown();
    }

    /// Requests a refresh of monitor list and brightness values.
    ///
    /// Sends a `RefreshAll` command to the DDC worker. The actual state
    /// update happens when `DdcRefreshResult` is received.
    pub fn handle_refresh(&mut self, now: Instant) {
        log::info!("Requesting monitor refresh from DDC worker");

        // Clear ID cache since handles may change after refresh.
        self.id_cache.clear();

        let generation = self.refresh.begin(now);

        // Send refresh command to worker (non-blocking).
        if let Err(e) = self.ddc.send(DdcCommand::RefreshAll { generation }) {
            log::error!(error:% = e; "Failed to send refresh command to DDC worker");
            self.refresh.abort();
        }
    }

    /// Checks if a periodic refresh is due and triggers it if needed.
    ///
    /// Gates on the *enumerated* set of the last refresh: while monitors are
    /// identifiable (even if unreadable) the cadence keeps running, so absence
    /// pruning completes while undocked. Only a topology with nothing
    /// identifiable (or an aborted refresh) freezes the timer.
    pub fn check_periodic_refresh(&mut self, now: Instant) {
        let periodic_seconds = self.config.refresh.periodic_seconds;

        // Skip if periodic refresh is disabled (0) or refresh already in progress.
        if periodic_seconds == 0 || self.refresh.in_progress() {
            return;
        }

        if !self.refresh.last_enumerated() {
            log::debug!("Skipping periodic refresh (no monitors enumerated by last refresh)");
            return;
        }

        let elapsed = self.refresh.elapsed_since_refresh(now);
        let interval = Duration::from_secs(u64::from(periodic_seconds));

        if elapsed >= interval {
            log::debug!(elapsed_seconds = elapsed.as_secs(); "Periodic refresh triggered");
            self.handle_refresh(now);
        }
    }

    /// Handles the result of a DDC refresh operation.
    ///
    /// Read brightness values are authoritative ground truth for every monitor
    /// regardless of generation: a hardware value is true no matter which
    /// refresh produced it. Absence bookkeeping (pruning) is gated on the
    /// result being current and the enumerated set being non-empty.
    fn handle_ddc_refresh_result(
        &mut self,
        generation: u64,
        monitors: Vec<(MonitorId, u8)>,
        enumerated: Vec<MonitorId>,
        now: Instant,
    ) {
        let found_monitors = !monitors.is_empty();

        if found_monitors {
            log::info!(count = monitors.len(); "DDC refresh complete");
        } else {
            log::warn!("DDC refresh completed with no monitors found");
        }

        for (monitor_id, brightness) in monitors {
            log::debug!(monitor_id:% = monitor_id, brightness = brightness; "Monitor found during refresh");

            self.states
                .entry(monitor_id)
                .and_modify(|s| s.update_from_ddc(brightness))
                .or_insert_with(|| MonitorState::new(brightness));
        }

        let current = self
            .refresh
            .complete(generation, now, found_monitors, !enumerated.is_empty());

        // No information is no evidence: a stale/aborted generation or an
        // empty enumerated set must not stamp or prune anything.
        if current && !enumerated.is_empty() {
            self.apply_absence_evidence(&enumerated, now);
        }
    }

    /// Stamps/advances per-monitor absence evidence and prunes sustained ghosts.
    ///
    /// A monitor absent from `enumerated` gets `missing_since` stamped on the
    /// first miss and is pruned when a later miss shows the absence has been
    /// continuous for at least `PRUNE_ABSENCE_WINDOW` — so a prune always
    /// spans two observations and a sustained window, never a refresh burst.
    fn apply_absence_evidence(&mut self, enumerated: &[MonitorId], now: Instant) {
        let mut pruned: Vec<MonitorId> = Vec::new();

        for (id, state) in &mut self.states {
            if enumerated.contains(id) {
                state.missing_since = None;
            } else {
                match state.missing_since {
                    None => state.missing_since = Some(now),
                    Some(since)
                        if now.saturating_duration_since(since) >= PRUNE_ABSENCE_WINDOW =>
                    {
                        pruned.push(id.clone());
                    }
                    Some(_) => {}
                }
            }
        }

        for id in pruned {
            self.states.remove(&id);
            self.overlay.remove(&id);
            if self.osd_monitor.as_ref() == Some(&id) {
                self.osd_monitor = None;
            }
            // The full cache is cleared at refresh begin, but adjusts during
            // an in-flight refresh repopulate it; a recycled platform handle
            // must not resurrect the ghost.
            self.id_cache.retain(|_, cached| cached != &id);
            log::info!(monitor:% = id.base_display_name(); "Pruned monitor absent from topology");
            log::debug!(monitor_id:% = id; "Pruned monitor identity");
        }
    }

    /// Discards all absence evidence (used when the refresh pipeline was
    /// disrupted: system resume, worker respawn). Evidence must span an
    /// undisturbed window; refresh bursts around resume/respawn can observe
    /// misses while a dock's DP link is still training.
    fn reset_absence_evidence(&mut self) {
        for state in self.states.values_mut() {
            state.missing_since = None;
        }
    }

    /// Checks if a refresh is needed due to inactivity and triggers it if so.
    ///
    /// Called at the start of a brightness adjustment to resync with external
    /// changes after the user has been away. Non-blocking: the adjustment
    /// proceeds optimistically and reconciles when the result arrives.
    fn check_inactivity_refresh(&mut self, now: Instant) {
        let inactivity_seconds = self.config.refresh.inactivity_seconds;

        // Skip if inactivity refresh is disabled (0) or refresh already in progress.
        if inactivity_seconds == 0 || self.refresh.in_progress() {
            return;
        }

        let elapsed = now.saturating_duration_since(self.last_activity);
        let threshold = Duration::from_secs(u64::from(inactivity_seconds));

        if elapsed >= threshold {
            log::debug!(elapsed_seconds = elapsed.as_secs(); "Inactivity refresh triggered");
            self.handle_refresh(now);
        }
    }

    /// Clears the degraded DDC state so a fresh attempt can be made.
    fn clear_degraded(&mut self) {
        if self.ddc_disabled {
            log::info!("Recovering from degraded DDC state");
        }
        self.ddc_disabled = false;
        self.ddc.clear_backoff();
        self.consecutive_set_timeouts = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::BrightnessError;

    // ── Fakes ────────────────────────────────────────────────────────────

    #[derive(Default)]
    struct FakeOsd {
        visible: bool,
        shows: Vec<(MonitorHandle, u8)>,
        updates: Vec<u8>,
        error_updates: Vec<u8>,
    }

    impl OsdSink for FakeOsd {
        fn show(&mut self, handle: MonitorHandle, state: &MonitorState) -> Result<()> {
            self.visible = true;
            self.shows.push((handle, state.effective_brightness()));
            Ok(())
        }
        fn update(&mut self, state: &MonitorState) -> Result<()> {
            self.updates.push(state.effective_brightness());
            Ok(())
        }
        fn update_error(&mut self, state: &MonitorState) -> Result<()> {
            self.error_updates.push(state.effective_brightness());
            Ok(())
        }
        fn is_visible(&self) -> bool {
            self.visible
        }
    }

    #[derive(Default)]
    struct FakeOverlay {
        updates: Vec<(MonitorId, u8)>,
        removed: Vec<MonitorId>,
    }

    impl OverlaySink for FakeOverlay {
        fn update(&mut self, id: &MonitorId, _handle: MonitorHandle, opacity: u8) -> Result<()> {
            self.updates.push((id.clone(), opacity));
            Ok(())
        }
        fn remove(&mut self, id: &MonitorId) {
            self.removed.push(id.clone());
        }
    }

    struct FakeDdc {
        sent: Vec<DdcCommand>,
        fail_send: bool,
        alive: bool,
        respawn_outcome: RespawnOutcome,
        respawns: u32,
        backoff_clears: u32,
    }

    impl Default for FakeDdc {
        fn default() -> Self {
            Self {
                sent: Vec::new(),
                fail_send: false,
                alive: true,
                respawn_outcome: RespawnOutcome::Respawned,
                respawns: 0,
                backoff_clears: 0,
            }
        }
    }

    impl DdcPort for FakeDdc {
        fn send(&mut self, cmd: DdcCommand) -> Result<()> {
            if self.fail_send {
                return Err(BrightnessError::ChannelSend);
            }
            self.sent.push(cmd);
            Ok(())
        }
        fn is_alive(&self) -> bool {
            self.alive
        }
        fn respawn(&mut self, _now: Instant) -> RespawnOutcome {
            self.respawns += 1;
            self.respawn_outcome
        }
        fn clear_backoff(&mut self) {
            self.backoff_clears += 1;
        }
        fn shutdown(&self) {}
    }

    struct FakeLocator {
        handle: MonitorHandle,
        id: MonitorId,
    }

    impl MonitorLocator for FakeLocator {
        fn monitor_under_cursor(&self) -> Result<MonitorHandle> {
            Ok(self.handle)
        }
        fn resolve_id(&self, _handle: MonitorHandle) -> Result<MonitorId> {
            Ok(self.id.clone())
        }
    }

    // ── Helpers ──────────────────────────────────────────────────────────

    type TestController = Controller<FakeOsd, FakeOverlay, FakeDdc, FakeLocator>;

    fn test_id() -> MonitorId {
        MonitorId::new("DEL", "U2722D", Some("SN123".to_string()))
    }

    fn other_id() -> MonitorId {
        MonitorId::new("PHL", "346B1C", Some("SN456".to_string()))
    }

    fn test_controller(base: Instant) -> TestController {
        Controller::new(
            Config::default(),
            FakeOsd::default(),
            FakeOverlay::default(),
            FakeDdc::default(),
            FakeLocator {
                handle: MonitorHandle(1),
                id: test_id(),
            },
            base,
        )
    }

    /// Seeds a monitor state and returns its id.
    fn seed(c: &mut TestController, id: MonitorId, brightness: u8) -> MonitorId {
        c.states.insert(id.clone(), MonitorState::new(brightness));
        id
    }

    fn sent_refresh_count(c: &TestController) -> usize {
        c.ddc
            .sent
            .iter()
            .filter(|cmd| matches!(cmd, DdcCommand::RefreshAll { .. }))
            .count()
    }

    /// Delivers a current-generation refresh result: begins a refresh and
    /// completes it with the given readable and enumerated sets.
    fn deliver_refresh(
        c: &mut TestController,
        readable: Vec<(MonitorId, u8)>,
        enumerated: Vec<MonitorId>,
        now: Instant,
    ) {
        let generation = c.refresh.begin(now);
        c.handle_ddc_refresh_result(generation, readable, enumerated, now);
    }

    // ── Refresh lifecycle ────────────────────────────────────────────────

    #[test]
    fn refresh_request_sends_command_and_marks_in_progress() {
        let base = Instant::now();
        let mut c = test_controller(base);
        c.handle_refresh(base);
        assert_eq!(sent_refresh_count(&c), 1);
        assert!(c.refresh.in_progress());
    }

    #[test]
    fn refresh_send_failure_aborts() {
        let base = Instant::now();
        let mut c = test_controller(base);
        c.ddc.fail_send = true;
        c.handle_refresh(base);
        assert!(!c.refresh.in_progress());
        assert!(!c.refresh.last_enumerated(), "abort freezes the periodic gate");
    }

    #[test]
    fn refresh_result_applies_ground_truth_even_when_stale() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let stale = c.refresh.begin(base);
        let _current = c.refresh.begin(base);
        c.handle_ddc_refresh_result(stale, vec![(test_id(), 42)], vec![test_id()], base);
        assert_eq!(c.states[&test_id()].cached_brightness, 42);
        assert!(c.refresh.in_progress(), "stale completion leaves newer refresh in flight");
    }

    #[test]
    fn periodic_refresh_gates_on_enumerated_not_readable() {
        let base = Instant::now();
        let mut c = test_controller(base);
        c.config.refresh.periodic_seconds = 60;

        // Undock shape: nothing readable, but the panel is identifiable.
        deliver_refresh(&mut c, vec![], vec![test_id()], base);
        let before = sent_refresh_count(&c);
        c.check_periodic_refresh(base + Duration::from_secs(61));
        assert_eq!(sent_refresh_count(&c), before + 1, "gate stays open while enumerable");
    }

    #[test]
    fn periodic_refresh_frozen_when_nothing_enumerated() {
        let base = Instant::now();
        let mut c = test_controller(base);
        c.config.refresh.periodic_seconds = 60;

        deliver_refresh(&mut c, vec![], vec![], base);
        let before = sent_refresh_count(&c);
        c.check_periodic_refresh(base + Duration::from_secs(61));
        assert_eq!(sent_refresh_count(&c), before, "empty enumerated set freezes cadence");
    }

    // ── Ghost pruning ────────────────────────────────────────────────────

    #[test]
    fn sustained_absence_prunes_state_overlay_cache_and_osd_target() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let ghost = seed(&mut c, other_id(), 70);
        seed(&mut c, test_id(), 50);
        c.id_cache.insert(MonitorHandle(9), ghost.clone());
        c.osd_monitor = Some(ghost.clone());

        // First miss stamps evidence.
        deliver_refresh(&mut c, vec![(test_id(), 50)], vec![test_id()], base);
        assert!(c.states[&ghost].missing_since.is_some());
        assert!(c.states.contains_key(&ghost), "first miss must not prune");

        // Second miss inside the window: still retained.
        deliver_refresh(
            &mut c,
            vec![(test_id(), 50)],
            vec![test_id()],
            base + Duration::from_secs(60),
        );
        assert!(c.states.contains_key(&ghost));

        // Miss with the window spanned: pruned everywhere.
        deliver_refresh(
            &mut c,
            vec![(test_id(), 50)],
            vec![test_id()],
            base + Duration::from_secs(120),
        );
        assert!(!c.states.contains_key(&ghost));
        assert_eq!(c.overlay.removed, vec![ghost.clone()]);
        assert!(c.osd_monitor.is_none());
        assert!(!c.id_cache.values().any(|v| v == &ghost));
    }

    #[test]
    fn reappearance_resets_absence_evidence() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);

        deliver_refresh(&mut c, vec![], vec![other_id()], base);
        assert!(c.states[&id].missing_since.is_some());

        deliver_refresh(&mut c, vec![(id.clone(), 50)], vec![id.clone()], base + Duration::from_secs(60));
        assert!(c.states[&id].missing_since.is_none());
    }

    #[test]
    fn burst_misses_within_seconds_do_not_prune() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);

        // Resume/respawn burst: two observations seconds apart.
        deliver_refresh(&mut c, vec![], vec![other_id()], base);
        deliver_refresh(&mut c, vec![], vec![other_id()], base + Duration::from_secs(5));
        assert!(c.states.contains_key(&id), "window not spanned — no prune");
    }

    #[test]
    fn empty_enumerated_set_is_no_evidence() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);

        deliver_refresh(&mut c, vec![], vec![], base);
        assert!(c.states[&id].missing_since.is_none(), "no information is no evidence");
    }

    #[test]
    fn stale_result_does_not_advance_absence_evidence() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);

        let stale = c.refresh.begin(base);
        let _newer = c.refresh.begin(base);
        c.handle_ddc_refresh_result(stale, vec![], vec![other_id()], base);
        assert!(c.states[&id].missing_since.is_none());
    }

    #[test]
    fn replug_after_prune_starts_fresh() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);
        c.states.get_mut(&id).unwrap().overlay_opacity = 40;

        deliver_refresh(&mut c, vec![], vec![other_id()], base);
        deliver_refresh(&mut c, vec![], vec![other_id()], base + Duration::from_secs(120));
        assert!(!c.states.contains_key(&id));

        deliver_refresh(
            &mut c,
            vec![(id.clone(), 80)],
            vec![id.clone()],
            base + Duration::from_secs(180),
        );
        let state = &c.states[&id];
        assert_eq!(state.cached_brightness, 80);
        assert_eq!(state.overlay_opacity, 0, "prune forgets the dim level by design");
    }

    #[test]
    fn reset_discards_all_absence_evidence() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);
        deliver_refresh(&mut c, vec![], vec![other_id()], base);
        assert!(c.states[&id].missing_since.is_some());

        c.reset_absence_evidence();
        assert!(c.states[&id].missing_since.is_none());
    }

    // ── Inactivity ───────────────────────────────────────────────────────

    #[test]
    fn inactivity_refresh_fires_after_threshold() {
        let base = Instant::now();
        let mut c = test_controller(base);
        c.config.refresh.inactivity_seconds = 30;

        c.check_inactivity_refresh(base + Duration::from_secs(29));
        assert_eq!(sent_refresh_count(&c), 0);
        c.check_inactivity_refresh(base + Duration::from_secs(30));
        assert_eq!(sent_refresh_count(&c), 1);
    }
}
````

- [ ] **Step 2: Register the module.** In `src/core/mod.rs` add `pub mod controller;` after `pub mod config;` (alphabetical) and extend the doc list with `//! - [\`controller\`] - Message-driven orchestration behind platform seams`.

- [ ] **Step 3: Run the new tests**

Run: `cargo test --lib controller`
Expected: PASS (all 12 tests)

- [ ] **Step 4: Run gates**

Run: `cargo test && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: all PASS — with one systematic exception: private methods that only tests call so far (`handle_ddc_refresh_result`, `apply_absence_evidence`, `reset_absence_evidence`, `check_inactivity_refresh`, `clear_degraded`) are dead code in the lib target until `handle_message` (Task 7) wires them. Add `#[allow(dead_code)]` to exactly the items the compiler flags, each with the comment `// removed when the message dispatch lands`. Never blanket-allow at module level. Task 7 removes these markers (dispatch calls them all); any marker on platform seam impls (Task 8) is removed in Task 9.

- [ ] **Step 5: Commit**

```powershell
git add src/core/controller.rs src/core/mod.rs
git commit -m "feat: core controller skeleton with seams, refresh lifecycle, ghost pruning"
```

---

### Task 5: Adjust + set-result orchestration

**Files:**
- Modify: `src/core/controller.rs` (methods + tests; source of moved code: `src/main.rs:288-327, 437-459, 466-582`)

**Interfaces:**
- Consumes: seams, `test_controller`, `seed`, `deliver_refresh` from Task 4.
- Produces (private, dispatched in Task 7): `handle_adjust(&mut self, monitor_id: Option<MonitorId>, delta: i8, now: Instant) -> Result<()>`, `handle_ddc_set_result(&mut self, monitor_id: &MonitorId, value: u8, seq: u64, success: bool, error: Option<&str>) -> Result<()>`, `show_error_on_visible_osd(&mut self)`.

- [ ] **Step 1: Write the failing tests** — append inside `mod tests`:

```rust
    // ── Adjust / optimistic update ───────────────────────────────────────

    #[test]
    fn adjust_applies_optimistic_update_and_sends_seq_stamped_command() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);

        c.handle_adjust(None, 10, base).unwrap();

        let state = &c.states[&id];
        assert_eq!(state.effective_brightness(), 60, "optimistic pending visible");
        assert_eq!(state.cached_brightness, 50, "cache untouched until confirm");
        assert!(c.osd.visible, "OSD shown immediately");
        assert!(matches!(
            c.ddc.sent.last(),
            Some(DdcCommand::SetBrightness { value: 60, seq: 0, .. })
        ));
        assert_eq!(c.osd_monitor.as_ref(), Some(&id));
    }

    #[test]
    fn adjust_without_change_still_shows_osd_but_sends_nothing() {
        let base = Instant::now();
        let mut c = test_controller(base);
        seed(&mut c, test_id(), 100);

        c.handle_adjust(None, 10, base).unwrap();

        assert!(c.osd.visible);
        assert!(c.ddc.sent.is_empty(), "no hardware or overlay change to apply");
    }

    #[test]
    fn adjust_unknown_monitor_errors_and_triggers_one_refresh() {
        let base = Instant::now();
        let mut c = test_controller(base);
        // Pretend a previous refresh succeeded so only the new trigger can fire.
        deliver_refresh(&mut c, vec![(other_id(), 50)], vec![other_id()], base);

        let err = c.handle_adjust(None, 10, base).unwrap_err();
        assert!(matches!(err, BrightnessError::MonitorNotFound(_)));
        assert_eq!(sent_refresh_count(&c), 1, "recovery refresh dispatched");

        // While that refresh is in flight, a second press must not stack another.
        let _ = c.handle_adjust(None, 10, base).unwrap_err();
        assert_eq!(sent_refresh_count(&c), 1, "gated on in-flight refresh");
    }

    #[test]
    fn adjust_send_failure_reverts_and_marks_visible_osd() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);
        c.ddc.fail_send = true;

        c.handle_adjust(None, 10, base).unwrap();

        assert!(c.states[&id].pending.is_none(), "optimistic value reverted");
        assert_eq!(c.states[&id].effective_brightness(), 50);
        assert!(!c.osd.error_updates.is_empty(), "OSD restyled to error");
    }

    #[test]
    fn adjust_while_degraded_clears_degraded_state() {
        let base = Instant::now();
        let mut c = test_controller(base);
        seed(&mut c, test_id(), 50);
        c.ddc_disabled = true;

        c.handle_adjust(None, 10, base).unwrap();

        assert!(!c.ddc_disabled, "user activity is the recovery signal");
        assert_eq!(c.ddc.backoff_clears, 1);
    }

    // ── Set results ──────────────────────────────────────────────────────

    #[test]
    fn set_result_confirms_updates_visible_osd_and_resets_hang_counter() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);
        c.consecutive_set_timeouts = 2;
        c.handle_adjust(None, 10, base).unwrap();

        c.handle_ddc_set_result(&id, 60, 0, true, None).unwrap();

        let state = &c.states[&id];
        assert_eq!(state.cached_brightness, 60);
        assert!(state.pending.is_none());
        assert_eq!(c.consecutive_set_timeouts, 0);
        assert!(!c.osd.updates.is_empty());
    }

    #[test]
    fn set_result_failure_reverts_and_shows_error() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);
        c.handle_adjust(None, 10, base).unwrap();

        c.handle_ddc_set_result(&id, 60, 0, false, Some("nak")).unwrap();

        assert_eq!(c.states[&id].effective_brightness(), 50, "reverted to cache");
        assert!(!c.osd.error_updates.is_empty());
    }

    #[test]
    fn stale_set_result_is_ignored() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);
        c.handle_adjust(None, 10, base).unwrap(); // seq 0
        c.handle_adjust(None, 10, base).unwrap(); // seq 1, pending 70

        c.handle_ddc_set_result(&id, 60, 0, false, Some("late")).unwrap();

        let pending = c.states[&id].pending.expect("newer pending survives");
        assert_eq!(pending.seq, 1);
    }

    #[test]
    fn set_result_for_unknown_monitor_is_dropped() {
        let base = Instant::now();
        let mut c = test_controller(base);
        // Routine after pruning: a late result for a removed monitor.
        c.handle_ddc_set_result(&other_id(), 60, 0, true, None).unwrap();
        assert!(c.states.is_empty(), "no ghost resurrection");
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib controller`
Expected: COMPILE ERROR — `handle_adjust` / `handle_ddc_set_result` not found

- [ ] **Step 3: Implement.** First extend the module imports per the note at the top of the file: add `use crate::core::brightness::calculate_adjustment;`, add `SetOutcome` to the `crate::core::state` import list, and change the error import to `use crate::error::{BrightnessError, Result};`. Then add to the `impl` block (moved from `main.rs:466-582`, `288-327`, `437-459`; every `Instant::now()` replaced by `now`, concrete types by seams, `HMONITOR` by `MonitorHandle`):

```rust
    /// Applies a relative brightness adjustment.
    ///
    /// Determines the target monitor (mouse position), calculates new values,
    /// shows the OSD immediately with optimistic update, and sends the DDC
    /// command to the worker thread (non-blocking).
    ///
    /// # Errors
    ///
    /// Returns an error if the target monitor cannot be resolved or is
    /// unknown, or if an OSD/overlay update fails.
    fn handle_adjust(&mut self, monitor_id: Option<MonitorId>, delta: i8, now: Instant) -> Result<()> {
        // Check if we need an inactivity-based refresh before processing
        // (must be checked BEFORE updating last_activity)
        self.check_inactivity_refresh(now);

        // User activity is a recovery signal for a degraded DDC state.
        if self.ddc_disabled {
            self.clear_degraded();
        }

        // If last refresh failed, trigger a new one (user activity indicates they're back).
        if !self.refresh.last_successful() && !self.refresh.in_progress() {
            log::debug!("Triggering refresh on user activity (last refresh found no monitors)");
            self.handle_refresh(now);
        }

        // Update activity timestamp for inactivity-based refresh tracking
        self.last_activity = now;

        // 1. Determine target monitor and handle
        // The handle is needed for OSD and overlay positioning.
        let handle = self.locator.monitor_under_cursor()?;

        // If no ID was provided, identify the monitor under the cursor.
        // A cache avoids repeated slow identity lookups.
        let target_id = match monitor_id {
            Some(id) => id,
            None => {
                if let Some(id) = self.id_cache.get(&handle) {
                    id.clone()
                } else {
                    let id = self.locator.resolve_id(handle)?;
                    self.id_cache.insert(handle, id.clone());
                    id
                }
            }
        };

        // 2. Find state for this monitor
        let Some(state) = self.states.get_mut(&target_id) else {
            // Recovery after pruning: a press on an unknown monitor dispatches
            // a refresh (at most one in flight) so a following press works in
            // every topology — the activity retrigger above only fires when
            // nothing was readable at all.
            if !self.refresh.in_progress() {
                self.handle_refresh(now);
            }
            return Err(BrightnessError::MonitorNotFound(target_id.to_string()));
        };

        // 3. Calculate new brightness
        let old_hardware = state.effective_brightness();
        let old_overlay = state.overlay_opacity;

        let adjustment = calculate_adjustment(old_hardware, old_overlay, delta);
        let new_hardware = adjustment.hardware_brightness;
        let new_overlay = adjustment.overlay_opacity;

        let changed = (old_hardware != new_hardware) || (old_overlay != new_overlay);

        if !changed {
            log::trace!(monitor_id:% = target_id, hardware = old_hardware, overlay = old_overlay; "No brightness change needed");
            // Still show/update OSD to reset timer and provide feedback.
            self.osd_monitor = Some(target_id.clone());
            if self.osd.is_visible() {
                self.osd.update(state)?;
            } else {
                self.osd.show(handle, state)?;
            }
            return Ok(());
        }

        log::trace!(
            monitor_id:% = target_id,
            old_hw = old_hardware,
            new_hw = new_hardware,
            old_overlay = old_overlay,
            new_overlay = new_overlay;
            "Attempting brightness adjustment"
        );

        // 4. Optimistic update (only set pending if hardware is changing)
        let seq = self.next_seq;
        if new_hardware != old_hardware {
            self.next_seq += 1;
            state.set_pending(new_hardware, seq, now);
        }
        state.overlay_opacity = new_overlay;

        // 5. Update overlay (software layer is immediately effective)
        if new_overlay != old_overlay {
            self.overlay.update(&target_id, handle, new_overlay)?;
        }

        // 6. Show or update OSD with optimistic values.
        self.osd_monitor = Some(target_id.clone());
        if self.osd.is_visible() {
            self.osd.update(state)?;
        } else {
            self.osd.show(handle, state)?;
        }

        // 7. Send DDC command to worker (non-blocking)
        if new_hardware == old_hardware {
            log::debug!(monitor_id:% = target_id, old_overlay = old_overlay, new_overlay = new_overlay; "Adjusting overlay only");
        } else {
            log::debug!(monitor_id:% = target_id, old_hw = old_hardware, new_hw = new_hardware; "Sending DDC command");
            if let Err(e) = self.ddc.send(DdcCommand::SetBrightness {
                monitor_id: target_id.clone(),
                value: new_hardware,
                seq,
            }) {
                log::error!(error:% = e; "DDC worker send failed; reverting optimistic value");
                if let Some(state) = self.states.get_mut(&target_id) {
                    state.force_revert();
                }
                self.show_error_on_visible_osd();
            }
        }

        Ok(())
    }

    /// Handles the result of a DDC brightness set operation.
    ///
    /// Reconciles the result against the monitor's pending set by sequence id.
    /// A confirmed or authoritative-late result refreshes the OSD; a revert
    /// shows the error state; a stale result is dropped.
    ///
    /// # Errors
    ///
    /// Returns an error if an OSD update fails.
    fn handle_ddc_set_result(
        &mut self,
        monitor_id: &MonitorId,
        value: u8,
        seq: u64,
        success: bool,
        error: Option<&str>,
    ) -> Result<()> {
        let Some(state) = self.states.get_mut(monitor_id) else {
            log::warn!(monitor_id:% = monitor_id; "Received DDC result for unknown monitor");
            return Ok(());
        };

        match state.apply_set_result(seq, value, success) {
            SetOutcome::Confirmed | SetOutcome::GroundTruth => {
                self.consecutive_set_timeouts = 0;
                log::debug!(monitor_id:% = monitor_id, brightness = value; "DDC confirmed brightness");
                if self.osd.is_visible() {
                    self.osd.update(state)?;
                }
            }
            SetOutcome::Reverted => {
                let error_msg = error.unwrap_or("unknown error");
                log::error!(monitor_id:% = monitor_id, target_brightness = value, error = error_msg; "DDC failed to set brightness");
                if self.osd.is_visible() {
                    self.osd.update_error(state)?;
                }
            }
            SetOutcome::Ignored => {
                log::debug!(monitor_id:% = monitor_id, seq = seq; "Ignoring stale/irrelevant DDC result");
            }
        }

        Ok(())
    }

    /// Restyles the OSD to its error state, only if it is currently visible.
    ///
    /// Never spontaneously shows a hidden OSD: the watchdog fires seconds after
    /// the keypress and the cached value is authoritative for the next display.
    fn show_error_on_visible_osd(&mut self) {
        if !self.osd.is_visible() {
            return;
        }
        let Some(id) = self.osd_monitor.clone() else {
            return;
        };
        if let Some(state) = self.states.get(&id) {
            if let Err(e) = self.osd.update_error(state) {
                log::warn!(error:% = e; "Failed to update OSD error state");
            }
        }
    }
```

- [ ] **Step 4: Run gates**

Run: `cargo test && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: all PASS (12 + 9 controller tests). Move any needed `#[allow(dead_code)]` markers per Task 4 Step 4 rule.

- [ ] **Step 5: Commit**

```powershell
git add src/core/controller.rs
git commit -m "feat: controller adjust/set-result orchestration with recovery refresh"
```

---

### Task 6: Supervision + watchdogs

**Files:**
- Modify: `src/core/controller.rs` (moved from `src/main.rs:359-431`)

**Interfaces:**
- Consumes: fakes/helpers, `reset_absence_evidence`, `show_error_on_visible_osd`.
- Produces: `pub fn supervise_and_watchdog(&mut self, now: Instant)`; private `supervise_worker(now)`, `check_watchdogs(now)`, `reconcile_all_pending()`.

- [ ] **Step 1: Write the failing tests** — append inside `mod tests`:

```rust
    // ── Supervision / watchdogs ──────────────────────────────────────────

    /// Advances past the 250 ms health-check throttle and runs one pass.
    fn supervise_at(c: &mut TestController, now: Instant) {
        c.last_health_check = now - Duration::from_secs(1);
        c.supervise_and_watchdog(now);
    }

    #[test]
    fn dead_worker_respawn_reconciles_resets_evidence_and_refreshes() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);
        c.handle_adjust(None, 10, base).unwrap();
        deliver_refresh(&mut c, vec![], vec![other_id()], base); // stamps missing_since
        c.ddc.alive = false;

        supervise_at(&mut c, base + Duration::from_secs(1));

        assert_eq!(c.ddc.respawns, 1);
        assert!(c.states[&id].pending.is_none(), "pendings force-reverted");
        assert!(c.states[&id].missing_since.is_none(), "evidence discarded on respawn");
        assert!(c.refresh.in_progress(), "fresh refresh dispatched");
    }

    #[test]
    fn backoff_exceeded_disables_ddc_and_resets_evidence() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);
        deliver_refresh(&mut c, vec![], vec![other_id()], base);
        c.ddc.alive = false;
        c.ddc.respawn_outcome = RespawnOutcome::BackoffExceeded;

        supervise_at(&mut c, base + Duration::from_secs(1));

        assert!(c.ddc_disabled);
        assert!(c.states[&id].missing_since.is_none());
    }

    #[test]
    fn set_timeout_reverts_counts_and_disables_after_limit() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);

        for round in 0..HUNG_TIMEOUT_LIMIT {
            let t = base + Duration::from_secs(u64::from(round) * 20);
            c.handle_adjust(None, 10, t).unwrap();
            assert!(c.states[&id].pending.is_some());
            supervise_at(&mut c, t + SET_TIMEOUT);
            assert!(c.states[&id].pending.is_none(), "watchdog reverted the pending");
        }

        assert_eq!(c.consecutive_set_timeouts, HUNG_TIMEOUT_LIMIT);
        assert!(c.ddc_disabled, "alive-but-hung worker diagnosed after limit");
    }

    #[test]
    fn refresh_timeout_aborts() {
        let base = Instant::now();
        let mut c = test_controller(base);
        c.handle_refresh(base);
        assert!(c.refresh.in_progress());

        supervise_at(&mut c, base + REFRESH_TIMEOUT);
        assert!(!c.refresh.in_progress());
    }

    #[test]
    fn health_pass_is_throttled() {
        let base = Instant::now();
        let mut c = test_controller(base);
        c.ddc.alive = false;

        // Constructor stamped last_health_check = base; within 250 ms nothing runs.
        c.supervise_and_watchdog(base + Duration::from_millis(100));
        assert_eq!(c.ddc.respawns, 0);
        c.supervise_and_watchdog(base + Duration::from_millis(300));
        assert_eq!(c.ddc.respawns, 1);
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib controller`
Expected: COMPILE ERROR — `supervise_and_watchdog` not found

- [ ] **Step 3: Implement.** First extend the `crate::core::reconcile` import with `HUNG_TIMEOUT_LIMIT, REFRESH_TIMEOUT, SET_TIMEOUT`. Then add (moved from `main.rs:359-431`; `now` threaded, evidence resets added per spec):

```rust
    /// Runs one throttled supervision + watchdog pass (called each loop tick).
    pub fn supervise_and_watchdog(&mut self, now: Instant) {
        if now.saturating_duration_since(self.last_health_check) < Duration::from_millis(250) {
            return;
        }
        self.last_health_check = now;
        self.supervise_worker(now);
        self.check_watchdogs(now);
    }

    /// Respawns the DDC worker if it has died (never merely because it is slow).
    fn supervise_worker(&mut self, now: Instant) {
        if self.ddc_disabled || self.ddc.is_alive() {
            return;
        }
        match self.ddc.respawn(now) {
            RespawnOutcome::Respawned => {
                log::warn!("DDC worker died; respawned");
                self.reconcile_all_pending();
                self.refresh.abort();
                self.consecutive_set_timeouts = 0;
                self.reset_absence_evidence();
                self.handle_refresh(now);
            }
            RespawnOutcome::BackoffExceeded => {
                log::error!("DDC worker respawn backoff exceeded; disabling DDC until recovery");
                self.ddc_disabled = true;
                self.reconcile_all_pending();
                self.reset_absence_evidence();
            }
        }
    }

    /// Reconciles state deadlines: stuck pendings and a latched refresh.
    fn check_watchdogs(&mut self, now: Instant) {
        let timed_out: Vec<MonitorId> = self
            .states
            .iter()
            .filter(|(_, state)| state.pending_timed_out(now, SET_TIMEOUT))
            .map(|(id, _)| id.clone())
            .collect();

        if !timed_out.is_empty() {
            for id in &timed_out {
                if let Some(state) = self.states.get_mut(id) {
                    state.force_revert();
                }
                log::error!(monitor_id:% = id; "DDC set timed out with no result; reverted");
            }
            self.consecutive_set_timeouts += 1;
            self.show_error_on_visible_osd();

            if self.ddc.is_alive()
                && !self.ddc_disabled
                && self.consecutive_set_timeouts >= HUNG_TIMEOUT_LIMIT
            {
                log::error!(count = self.consecutive_set_timeouts; "DDC worker unresponsive; disabling DDC until restart or resume");
                self.ddc_disabled = true;
            }
        }

        if self.refresh.timed_out(now, REFRESH_TIMEOUT) {
            log::error!("DDC refresh timed out with no result; aborting");
            self.refresh.abort();
        }
    }

    /// Force-reverts every pending set (used after a worker respawn).
    fn reconcile_all_pending(&mut self) {
        for state in self.states.values_mut() {
            state.force_revert();
        }
        self.show_error_on_visible_osd();
    }
```

- [ ] **Step 4: Run gates**

Run: `cargo test && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: all PASS

- [ ] **Step 5: Commit**

```powershell
git add src/core/controller.rs
git commit -m "feat: controller supervision and watchdogs with absence-evidence resets"
```

---

### Task 7: Message dispatch + tray menu data

**Files:**
- Modify: `src/core/controller.rs` (dispatch from `src/main.rs:183-252, 584-594`, tray data from `:642-673`)

**Interfaces:**
- Consumes: everything above.
- Produces: `pub fn handle_message(&mut self, message: BrightnessMessage, now: Instant) -> Result<bool>` — `Ok(false)` ⇔ shutdown requested. Shell variants (`TrayOpenUsage`, `TrayOpenSettings`) are debug-log no-ops here; the binary's shell match intercepts them before this method (Task 9).

- [ ] **Step 1: Write the failing tests** — append inside `mod tests` (plus `use std::sync::mpsc;` at the top of `mod tests`):

```rust
    // ── Dispatch ─────────────────────────────────────────────────────────

    #[test]
    fn system_resumed_clears_degraded_resets_evidence_and_refreshes() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);
        deliver_refresh(&mut c, vec![], vec![other_id()], base);
        c.ddc_disabled = true;

        let cont = c.handle_message(BrightnessMessage::SystemResumed, base).unwrap();

        assert!(cont);
        assert!(!c.ddc_disabled);
        assert!(c.states[&id].missing_since.is_none(), "evidence discarded on resume");
        assert!(c.refresh.in_progress());
    }

    #[test]
    fn quit_and_shutdown_stop_the_loop() {
        let base = Instant::now();
        let mut c = test_controller(base);
        assert!(!c.handle_message(BrightnessMessage::TrayRequestQuit, base).unwrap());
        assert!(!c.handle_message(BrightnessMessage::Shutdown, base).unwrap());
    }

    #[test]
    fn shell_variants_are_noops_here() {
        let base = Instant::now();
        let mut c = test_controller(base);
        assert!(c.handle_message(BrightnessMessage::TrayOpenUsage, base).unwrap());
        assert!(c.handle_message(BrightnessMessage::TrayOpenSettings, base).unwrap());
        assert!(c.ddc.sent.is_empty());
    }

    #[test]
    fn tray_menu_opening_replies_with_names_and_values() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 55);
        c.states.get_mut(&id).unwrap().overlay_opacity = 20;

        let (reply_tx, reply_rx) = mpsc::channel();
        c.handle_message(BrightnessMessage::TrayMenuOpening { reply_tx }, base)
            .unwrap();

        let data = reply_rx.try_recv().expect("menu data sent");
        assert_eq!(data.monitors.len(), 1);
        assert_eq!(data.monitors[0].display_name, "DEL U2722D");
        assert_eq!(data.monitors[0].hardware_brightness, 55);
        assert_eq!(data.monitors[0].overlay_opacity, 20);
        assert_eq!(data.hotkey_up, c.config.hotkeys.brightness_up);
    }

    #[test]
    fn refresh_result_message_routes_with_enumerated_set() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let generation = c.refresh.begin(base);

        c.handle_message(
            BrightnessMessage::DdcRefreshResult {
                generation,
                monitors: vec![(test_id(), 33)],
                enumerated: vec![test_id()],
            },
            base,
        )
        .unwrap();

        assert_eq!(c.states[&test_id()].cached_brightness, 33);
        assert!(!c.refresh.in_progress());
    }
```

- [ ] **Step 2: Run to verify they fail**

Run: `cargo test --lib controller`
Expected: COMPILE ERROR — `handle_message` not found

- [ ] **Step 3: Implement.** First extend the `crate::core::state` import with `BrightnessMessage, TrayMenuData, TrayMonitorInfo, generate_display_names`, and remove the `#[allow(dead_code)]` markers from Tasks 4–6 (the dispatch below wires every marked method). Then add (dispatch from `main.rs:183-246`; shell arms become debug no-ops; `SetAbsolute`/`handle_shutdown` no-ops move as-is):

```rust
    /// Processes a brightness control message.
    ///
    /// Returns `Ok(true)` if the application should continue running,
    /// or `Ok(false)` if shutdown was requested.
    ///
    /// # Errors
    ///
    /// Returns an error if message processing fails.
    pub fn handle_message(&mut self, message: BrightnessMessage, now: Instant) -> Result<bool> {
        match message {
            BrightnessMessage::Adjust { monitor_id, delta } => {
                self.handle_adjust(monitor_id, delta, now)?;
            }
            BrightnessMessage::SetAbsolute { monitor_id, value } => {
                self.handle_set_absolute(monitor_id, value)?;
            }
            BrightnessMessage::Refresh => {
                self.handle_refresh(now);
            }
            BrightnessMessage::SystemResumed => {
                self.clear_degraded();
                // A refresh burst around resume can miss monitors while a
                // dock's link is still training; stale absence evidence must
                // not combine with it into a prune.
                self.reset_absence_evidence();
                log::info!(reason = "system_resume"; "Triggering refresh");
                self.handle_refresh(now);
            }
            // ── Tray Icon Messages ───────────────────────────────────────
            BrightnessMessage::TrayOpenUsage | BrightnessMessage::TrayOpenSettings => {
                // Shell side effects; the binary's loop handles them before
                // forwarding. Reaching this arm means a wiring regression.
                log::debug!("Shell message reached core controller (no-op)");
            }
            BrightnessMessage::TrayRequestQuit => {
                log::info!("Quit requested from tray menu");
                self.handle_shutdown()?;
                return Ok(false);
            }
            BrightnessMessage::TrayMenuOpening { reply_tx } => {
                log::debug!("TrayMenuOpening received");
                let menu_data = self.build_tray_menu_data();
                if let Err(e) = reply_tx.send(menu_data) {
                    log::warn!(error:? = e; "Failed to send tray menu data");
                }
            }
            BrightnessMessage::DdcSetResult {
                monitor_id,
                value,
                seq,
                success,
                error,
            } => {
                self.handle_ddc_set_result(&monitor_id, value, seq, success, error.as_deref())?;
            }
            BrightnessMessage::DdcRefreshResult {
                generation,
                monitors,
                enumerated,
            } => {
                self.handle_ddc_refresh_result(generation, monitors, enumerated, now);
            }
            BrightnessMessage::Shutdown => {
                self.handle_shutdown()?;
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Handles the shutdown process.
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    fn handle_shutdown(&mut self) -> Result<()> {
        Ok(())
    }

    /// Sets an absolute brightness value for a monitor.
    ///
    /// # Errors
    ///
    /// Currently infallible; placeholder for future extensions.
    #[allow(clippy::unused_self, clippy::unnecessary_wraps, clippy::needless_pass_by_value)]
    fn handle_set_absolute(&mut self, _monitor_id: Option<MonitorId>, _value: u8) -> Result<()> {
        // Placeholder for future extensions (e.g., fixed brightness via CLI command)
        Ok(())
    }

    /// Builds the data needed to populate the tray menu.
    ///
    /// Generates display names with duplicate suffixes (e.g., "Dell U2722D #1")
    /// when multiple monitors with identical manufacturer and model are connected.
    fn build_tray_menu_data(&self) -> TrayMenuData {
        // Collect monitor IDs and generate unique display names
        let monitor_ids: Vec<MonitorId> = self.states.keys().cloned().collect();
        let display_names = generate_display_names(&monitor_ids);

        let monitors: Vec<TrayMonitorInfo> = self
            .states
            .iter()
            .map(|(monitor_id, state)| {
                let display_name = display_names
                    .get(monitor_id)
                    .cloned()
                    .unwrap_or_else(|| monitor_id.base_display_name());

                TrayMonitorInfo {
                    display_name,
                    hardware_brightness: state.effective_brightness(),
                    overlay_opacity: state.overlay_opacity,
                }
            })
            .collect();

        TrayMenuData {
            monitors,
            hotkey_up: self.config.hotkeys.brightness_up.clone(),
            hotkey_down: self.config.hotkeys.brightness_down.clone(),
        }
    }
```

- [ ] **Step 4: Run gates**

Run: `cargo test && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: all PASS

- [ ] **Step 5: Commit**

```powershell
git add src/core/controller.rs
git commit -m "feat: controller message dispatch, resume evidence reset, tray menu data"
```

---

### Task 8: Platform seam implementations

**Files:**
- Modify: `src/platform/windows/osd.rs`, `src/platform/windows/overlay.rs`, `src/platform/windows/ddc_worker.rs`, `src/platform/windows/mod.rs`

**Interfaces:**
- Consumes: the four traits + `MonitorHandle` from `crate::core::controller`.
- Produces: `impl OsdSink for OsdWindow`, `OverlayManager::remove(&mut self, &MonitorId)` + `impl OverlaySink for OverlayManager`, `impl DdcPort for DdcSupervisor`, `pub struct CursorLocator` (unit) implementing `MonitorLocator`. FFI code — verified by compile + clippy + the manual smoke test in Task 9.

- [ ] **Step 1: OSD.** In `src/platform/windows/osd.rs`, after the `OsdWindow` impl block:

```rust
impl crate::core::controller::OsdSink for OsdWindow {
    fn show(
        &mut self,
        handle: crate::core::controller::MonitorHandle,
        state: &MonitorState,
    ) -> Result<()> {
        OsdWindow::show(self, HMONITOR(handle.0), state)
    }
    fn update(&mut self, state: &MonitorState) -> Result<()> {
        OsdWindow::update(self, state)
    }
    fn update_error(&mut self, state: &MonitorState) -> Result<()> {
        OsdWindow::update_error(self, state)
    }
    fn is_visible(&self) -> bool {
        OsdWindow::is_visible(self)
    }
}
```

- [ ] **Step 2: Overlay.** In `src/platform/windows/overlay.rs`, add to the `OverlayManager` impl block:

```rust
    /// Removes and destroys a monitor's overlay window, if one exists.
    ///
    /// Dropping the overlay destroys its window via the RAII handle; used
    /// when a monitor is pruned so the orphaned fullscreen window cannot
    /// migrate onto a surviving monitor.
    pub fn remove(&mut self, monitor_id: &MonitorId) {
        if self.overlays.remove(monitor_id).is_some() {
            log::debug!(monitor_id:% = monitor_id; "Overlay removed");
        }
    }
```

and after the impl block:

```rust
impl crate::core::controller::OverlaySink for OverlayManager {
    fn update(
        &mut self,
        id: &MonitorId,
        handle: crate::core::controller::MonitorHandle,
        opacity: u8,
    ) -> Result<()> {
        OverlayManager::update(self, id, HMONITOR(handle.0), opacity)
    }
    fn remove(&mut self, id: &MonitorId) {
        OverlayManager::remove(self, id);
    }
}
```

- [ ] **Step 3: DDC port.** In `src/platform/windows/ddc_worker.rs`, after the `DdcSupervisor` impl block:

```rust
impl crate::core::controller::DdcPort for DdcSupervisor {
    fn send(&mut self, cmd: DdcCommand) -> crate::Result<()> {
        DdcSupervisor::send(self, cmd).map_err(|_| crate::BrightnessError::ChannelSend)
    }
    fn is_alive(&self) -> bool {
        DdcSupervisor::is_alive(self)
    }
    fn respawn(&mut self, now: Instant) -> RespawnOutcome {
        DdcSupervisor::respawn(self, now)
    }
    fn clear_backoff(&mut self) {
        DdcSupervisor::clear_backoff(self);
    }
    fn shutdown(&self) {
        DdcSupervisor::shutdown(self);
    }
}
```

- [ ] **Step 4: Locator.** In `src/platform/windows/mod.rs`, after `get_monitor_under_cursor`:

```rust
/// Resolves the monitor under the cursor via Win32 (`GetCursorPos`,
/// `MonitorFromPoint`) and identifies monitors from EDID.
pub struct CursorLocator;

impl crate::core::controller::MonitorLocator for CursorLocator {
    fn monitor_under_cursor(&self) -> Result<crate::core::controller::MonitorHandle> {
        get_monitor_under_cursor().map(|h| crate::core::controller::MonitorHandle(h.0))
    }
    fn resolve_id(&self, handle: crate::core::controller::MonitorHandle) -> Result<crate::core::state::MonitorId> {
        ddc::get_monitor_id(HMONITOR(handle.0))
    }
}
```

- [ ] **Step 5: Run gates**

Run: `cargo build && cargo test && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: all PASS. If clippy flags the impls as dead code, apply the Task 4 Step 4 marker rule (removed next task).

- [ ] **Step 6: Commit**

```powershell
git add src/platform/windows/osd.rs src/platform/windows/overlay.rs src/platform/windows/ddc_worker.rs src/platform/windows/mod.rs
git commit -m "feat: implement controller seams on Windows types, CursorLocator"
```

---

### Task 9: `main.rs` switchover

**Files:**
- Modify: `src/main.rs` — delete `BrightnessController` (`main.rs:107-699`), rewire.

**Interfaces:**
- Consumes: `Controller`, seam impls, `CursorLocator`.
- Produces: the shipped binary. `now` capture rule: `Instant::now()` immediately before each controller call (after `recv_timeout` returns for `handle_message`; before the two tick calls).

- [ ] **Step 1: Delete** the `BrightnessController` struct + impl (`main.rs:107-699`) and `pump_windows_messages`' unchanged neighbors stay. Keep: `open_with_default_app`, `ctrl_handler`, `SHUTDOWN_SENDER`, `pump_windows_messages`, `load_config`, `init_logging`, `spawn_power_listener`, `spawn_tray_thread`, `start_hotkey_thread`.

- [ ] **Step 2: Add the shell handlers** (replacing the deleted controller methods `handle_open_usage` / `TrayOpenSettings` arm):

```rust
/// Opens or focuses the usage instructions window (shell side effect).
fn open_usage(window: &mut Option<UsageWindow>, config: &Config) {
    if let Some(w) = window {
        if w.is_valid() {
            log::debug!("Usage window already open, bringing to front");
            w.bring_to_front();
            return;
        }
    }
    match UsageWindow::new(
        &config.hotkeys.brightness_up,
        &config.hotkeys.brightness_down,
    ) {
        Ok(w) => {
            log::info!("Usage window opened");
            *window = Some(w);
        }
        Err(e) => {
            log::error!(error:% = e; "Failed to create usage window");
        }
    }
}

/// Opens the config file in the system default editor (shell side effect).
fn open_settings() {
    log::debug!("TrayOpenSettings received");
    if let Some(path) = Config::default_path() {
        if let Err(e) = open_with_default_app(&path) {
            log::error!(error:% = e; "Failed to open config file");
        }
    } else {
        log::error!("Could not determine config file path");
    }
}
```

- [ ] **Step 3: Rewire `main()`.** Replace the controller creation (`main.rs:946-956`) and the main loop (`main.rs:993-1028`) with:

```rust
    // Create controller: OSD is created here (its failure path), then injected.
    let osd = match OsdWindow::new(config.osd.opacity, config.osd.timeout_ms) {
        Ok(osd) => osd,
        Err(e) => {
            log::error!(error:% = e; "Failed to create OSD window");
            return;
        }
    };
    let mut controller = Controller::new(
        config.clone(),
        osd,
        OverlayManager::default(),
        supervisor,
        CursorLocator,
        Instant::now(),
    );

    // Request initial monitor enumeration from DDC worker
    controller.handle_refresh(Instant::now());
```

(the spawns and hotkey registration in between stay exactly as they are), then the loop:

```rust
    // Main Loop
    log::info!("Entering main event loop");
    let mut usage_window: Option<UsageWindow> = None;
    loop {
        // Pump Windows messages (for OSD WM_PAINT, WM_TIMER, etc.)
        pump_windows_messages();

        let now = Instant::now();
        controller.check_periodic_refresh(now);
        controller.supervise_and_watchdog(now);

        // Check for brightness messages with a short timeout
        match rx.recv_timeout(Duration::from_millis(16)) {
            Ok(msg) => {
                log::debug!(message:? = msg; "Main loop received message");
                match msg {
                    // Shell side effects stay out of the core controller.
                    BrightnessMessage::TrayOpenUsage => open_usage(&mut usage_window, &config),
                    BrightnessMessage::TrayOpenSettings => open_settings(),
                    other => match controller.handle_message(other, Instant::now()) {
                        Ok(should_continue) => {
                            if !should_continue {
                                break;
                            }
                        }
                        Err(e) => {
                            log::error!(error:% = e; "Error processing message");
                        }
                    },
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No message received, continue pumping Windows messages
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::info!("Channel disconnected, shutting down");
                break;
            }
        }
    }
```

The post-loop cleanup (`SetConsoleCtrlHandler` removal, `controller.shutdown_worker()`, `drop(controller)`, final log line) stays exactly as it is — both methods exist unchanged on the new `Controller`.

- [ ] **Step 4: Fix imports.** Remove now-unused imports (`calculate_adjustment`, `MonitorId`, `MonitorState`, `SetOutcome`, `TrayMenuData`, `TrayMonitorInfo`, `generate_display_names`, `get_monitor_id`, `get_monitor_under_cursor`, reconcile constants, `RespawnOutcome`, `DdcSupervisor` if only the type name is unused — keep what's still referenced); add:

```rust
use darkbright_helper::core::controller::Controller;
use darkbright_helper::platform::windows::CursorLocator;
```

Let the compiler drive: `cargo build` and remove/add until clean. Remove all `#[allow(dead_code)]` markers introduced by Tasks 4–8 (everything is wired now).

- [ ] **Step 5: Run gates**

Run: `cargo build && cargo test && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: all PASS; binary compiles with the core controller only.

- [ ] **Step 6: Manual smoke test** (hardware-dependent behavior is manual-only per `docs/architecture.md` Integration Testing):

Run: `cargo run` (debug build, console visible). Verify: hotkeys adjust brightness with OSD on the monitor under the cursor; sub-zero dimming engages the overlay; tray menu lists monitors with values; Usage and Settings tray items open their window/editor; quit works. Record the result in the task report.

- [ ] **Step 7: Commit**

```powershell
git add src/main.rs
git commit -m "refactor: switch main.rs to core Controller; shell handlers stay in binary"
```

---

### Task 10: `docs/architecture.md` update + final verification

**Files:**
- Modify: `docs/architecture.md`

**Interfaces:** none (documentation), but the spec makes this part of the same change — architecture.md is the source of truth and must not drift.

- [ ] **Step 1: Update the module map** — add to the core module list (wording to match surrounding entries):

```markdown
- `core/controller.rs` — message-driven orchestration (`Controller<Osd, Ovl, Ddc, Loc>`), generic over the OSD/overlay/DDC/locator seams so the optimistic-update, supervision, watchdog, and refresh sequences are unit-tested with fakes. The binary injects the Windows implementations and an explicit `now: Instant` per call.
```

- [ ] **Step 2: Update the refresh section** — document, in architecture.md's own voice next to the existing refresh/generation text:
  - `DdcRefreshResult` carries both the readable set (`monitors`) and the `enumerated` set (identification succeeded, read may have failed; empty when enumeration itself failed).
  - Absence pruning: first current-generation miss stamps `missing_since`; a later miss spanning ≥ 90 s (`PRUNE_ABSENCE_WINDOW`) prunes state + overlay + id-cache entries; evidence resets on resume and worker respawn; empty enumerated set or stale generation is no evidence. Pruning forgets deliberately: overlay dim level and cached brightness do not survive a > 90 s absence.
  - Periodic gate: `check_periodic_refresh` skips only when the last refresh enumerated nothing (`last_enumerated`), so the cadence keeps running while monitors are enumerable-but-unreadable (undocked); `abort()` freezes it as before.
  - Recovery: a hotkey press on an unknown monitor dispatches a refresh (at most one in flight).

- [ ] **Step 3: Update the `MonitorState` struct listing** (architecture.md lines ~578-584) — add the `missing_since: Option<Instant>` field with a one-line description matching the code doc.

- [ ] **Step 4: Update the testing section** — replace the statement that orchestration is manual-only: controller sequences are unit-tested via fakes in `core/controller.rs`; hardware-dependent behavior (DDC I/O, enumerated-set collection in the worker, unplug/replug and monitor-standby cycles) remains under the manual Integration Testing checklist — add those two cycles to that checklist.

- [ ] **Step 5: Cross-check spec vs. shipped code.** Re-read `docs/superpowers/specs/2026-07-23-controller-testability-design.md` section by section; verify each declared behavior exists in code and each test-table row has a matching test (`cargo test --lib controller -- --list`). Fix any gap found before committing.

- [ ] **Step 6: Run full gates one last time**

Run: `cargo build --release && cargo test && cargo fmt -- --check && cargo clippy --all-targets -- -D warnings`
Expected: all PASS (release build proves the `windows_subsystem` path still compiles).

- [ ] **Step 7: Commit**

```powershell
git add docs/architecture.md
git commit -m "docs: architecture.md — controller seams, enumerated-set pruning, periodic gate"
```
