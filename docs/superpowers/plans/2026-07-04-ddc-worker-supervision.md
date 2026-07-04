# DDC Worker Supervision & State Watchdog — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the DDC worker thread supervised and self-healing so its death or a lost result can no longer silently and permanently degrade the app.

**Architecture:** Two decoupled mechanisms. (1) *Death detection* — poll `JoinHandle::is_finished()` + treat `send()` errors as hard failures — respawns a dead worker in ~250 ms. (2) *State watchdog* — generous wall-clock deadlines reconcile stuck optimistic state and a latched refresh flag, and act as a backstop for a *hung* (alive-but-blocked) worker. DDC set results are correlated to their request by a per-set sequence id; refreshes by a generation counter. Pure decision logic lives in `core/` and is unit-tested; thread/FFI glue lives in `platform/windows/` and `main.rs`.

**Tech Stack:** Rust 2024, `windows` 0.52 (FFI), `std::sync::mpsc`, `std::thread`, `std::time::{Instant, Duration}`, `log` (kv). No new dependencies.

## Global Constraints

- MSRV 1.87+ (host toolchain here is 1.92). Rust 2024 edition.
- `cargo fmt -- --check` must pass. `cargo clippy -- -D warnings` must pass (`all` + `pedantic` are warn-by-default in `Cargo.toml`).
- All public items need `///` docs with `# Errors`/`# Panics` where applicable. Backtick code identifiers in docs (`clippy::doc_markdown`).
- Avoid `as` casts; use `u32::from` / `try_from` / `.min(100)`. Annotate pure fns with `#[must_use]` (except those returning `Result`).
- Structured kv logging: `log::info!(key:% = v; "message")`. Log at the point of *handling*, not occurrence.
- Do **not** cite ephemeral planning labels (task/step numbers, finding IDs like `#1`/`#2`) in code comments or commit-message-referenced code; state rationale in self-contained domain terms. (This plan's own `#N` references stay out of the code.)
- Commit messages ≤ ~50 words, terse; **no** `Co-Authored-By`/AI trailer.
- This is a Windows binary; `cargo build`/`test` compile the full app on this host. Actual DDC I/O and thread-death timing are verified **manually** (per repo convention) — noted per task.

---

## File Structure

- **Create** `src/core/reconcile.rs` — `RefreshTracker`, `respawn_allowed`, and all timeout/backoff constants. Pure, platform-agnostic, fully unit-tested.
- **Modify** `src/core/mod.rs` — declare + re-export `reconcile`.
- **Modify** `src/core/state.rs` — `PendingSet`, `SetOutcome`, seq-correlated `MonitorState` reconciliation; add `seq` to the DDC *set* messages and `generation` to the DDC *refresh* messages.
- **Modify** `src/platform/windows/ddc_worker.rs` — echo `seq`/`generation` from the worker; add `DdcSupervisor` + `RespawnOutcome`.
- **Modify** `src/platform/windows/mod.rs` — export `DdcSupervisor`.
- **Modify** `src/main.rs` — controller holds the supervisor + `RefreshTracker` + watchdog fields; wire correlation, refresh generation, supervision/watchdog into the main loop; recovery triggers.

Task order keeps the crate compiling at every boundary: pure core first (Task 1), then the set-correlation change with its consumers (Task 2), then refresh generation (Task 3), then the supervisor + startup wiring (Task 4), then the loop integration that ties it together (Task 5).

---

### Task 1: Core reconciliation primitives (`RefreshTracker`, `respawn_allowed`, constants)

Purely additive new module; nothing consumes it yet, so the crate keeps compiling and every step is real-TDD.

**Files:**
- Create: `src/core/reconcile.rs`
- Modify: `src/core/mod.rs`
- Test: in-module `#[cfg(test)]` in `src/core/reconcile.rs`

**Interfaces:**
- Consumes: `std::time::{Instant, Duration}`.
- Produces:
  - `pub const SET_TIMEOUT: Duration` (8000 ms), `pub const REFRESH_TIMEOUT: Duration` (5000 ms), `pub const RESPAWN_MAX: usize` (3), `pub const RESPAWN_WINDOW: Duration` (60 s), `pub const HUNG_TIMEOUT_LIMIT: u32` (3).
  - `pub fn respawn_allowed(recent: &[Instant], now: Instant, window: Duration, max: usize) -> bool`
  - `pub struct RefreshTracker` with: `new(now: Instant) -> Self`, `begin(&mut self, now: Instant) -> u64`, `complete(&mut self, generation: u64, now: Instant, found: bool)`, `abort(&mut self)`, `timed_out(&self, now: Instant, timeout: Duration) -> bool`, `in_progress(&self) -> bool`, `last_successful(&self) -> bool`, `elapsed_since_refresh(&self, now: Instant) -> Duration`.

- [ ] **Step 1: Declare the module**

In `src/core/mod.rs`, add under the existing `pub mod` lines (after `pub mod brightness;`):

```rust
pub mod reconcile;
```

And add to the re-export block (after `pub use config::Config;`):

```rust
pub use reconcile::{RefreshTracker, respawn_allowed};
```

- [ ] **Step 2: Write the failing tests**

Create `src/core/reconcile.rs` with only the test module first (module body empty), so the tests fail to compile/resolve:

```rust
//! Platform-agnostic reconciliation primitives for DDC supervision.
//!
//! Holds the wall-clock deadlines, the worker-respawn backoff decision, and the
//! refresh-tracking state machine. All decisions are pure functions of an
//! explicitly passed `now`, so they are unit-testable on any platform.

use std::time::{Duration, Instant};

// (implementation added in later steps)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respawn_allowed_denies_when_window_full() {
        let base = Instant::now();
        let recent = [base, base + Duration::from_secs(1), base + Duration::from_secs(2)];
        let now = base + Duration::from_secs(3);
        assert!(!respawn_allowed(&recent, now, Duration::from_secs(60), 3));
    }

    #[test]
    fn respawn_allowed_permits_when_spaced_beyond_window() {
        let base = Instant::now();
        let recent = [base, base + Duration::from_secs(1), base + Duration::from_secs(2)];
        // 61 s after the last entry: all three fall outside the 60 s window.
        let now = base + Duration::from_secs(63);
        assert!(respawn_allowed(&recent, now, Duration::from_secs(60), 3));
    }

    #[test]
    fn respawn_allowed_permits_empty_history() {
        let now = Instant::now();
        assert!(respawn_allowed(&[], now, Duration::from_secs(60), 3));
    }

    #[test]
    fn refresh_tracker_begin_marks_in_progress_and_bumps_generation() {
        let base = Instant::now();
        let mut t = RefreshTracker::new(base);
        assert!(!t.in_progress());
        let g1 = t.begin(base);
        let g2 = {
            let mut t2 = RefreshTracker::new(base);
            t2.begin(base)
        };
        assert!(t.in_progress());
        assert_eq!(g1, g2, "first generation is deterministic");
    }

    #[test]
    fn refresh_tracker_ignores_stale_generation() {
        let base = Instant::now();
        let mut t = RefreshTracker::new(base);
        let gen = t.begin(base);
        // A completion for an older generation must be ignored.
        t.complete(gen - 1, base + Duration::from_secs(1), true);
        assert!(t.in_progress(), "stale completion left the refresh in progress");
        // The matching generation completes it.
        t.complete(gen, base + Duration::from_secs(1), true);
        assert!(!t.in_progress());
        assert!(t.last_successful());
    }

    #[test]
    fn refresh_tracker_complete_records_failure() {
        let base = Instant::now();
        let mut t = RefreshTracker::new(base);
        let gen = t.begin(base);
        t.complete(gen, base + Duration::from_secs(1), false);
        assert!(!t.in_progress());
        assert!(!t.last_successful());
    }

    #[test]
    fn refresh_tracker_timed_out_only_while_in_progress_and_past_deadline() {
        let base = Instant::now();
        let mut t = RefreshTracker::new(base);
        assert!(!t.timed_out(base + Duration::from_secs(10), Duration::from_secs(5)));
        let _ = t.begin(base);
        assert!(!t.timed_out(base + Duration::from_secs(4), Duration::from_secs(5)));
        assert!(t.timed_out(base + Duration::from_secs(5), Duration::from_secs(5)));
    }

    #[test]
    fn refresh_tracker_abort_invalidates_outstanding_result() {
        let base = Instant::now();
        let mut t = RefreshTracker::new(base);
        let gen = t.begin(base);
        t.abort();
        assert!(!t.in_progress());
        assert!(!t.last_successful());
        // A late result for the aborted generation must not resurrect state.
        t.complete(gen, base + Duration::from_secs(1), true);
        assert!(!t.in_progress());
        assert!(!t.last_successful());
    }
}
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test --lib reconcile`
Expected: FAIL — `respawn_allowed`/`RefreshTracker` not found (unresolved names).

- [ ] **Step 4: Implement the constants, `respawn_allowed`, and `RefreshTracker`**

Replace the `// (implementation added in later steps)` line in `src/core/reconcile.rs` with:

```rust
/// Backstop deadline for a pending brightness set.
///
/// Deliberately larger than [`REFRESH_TIMEOUT`] plus a set's own execution
/// budget: a set can wait in the worker's serial command queue behind an
/// in-flight refresh, and its deadline is measured from enqueue time. A dead
/// worker is healed far faster by liveness detection, so this only ever fires
/// for a worker that is alive but hung inside a blocking DDC call.
pub const SET_TIMEOUT: Duration = Duration::from_millis(8000);

/// Deadline for a full monitor refresh (enumeration plus per-monitor EDID read
/// and DDC brightness read, each up to ~120 ms across up to three attempts).
pub const REFRESH_TIMEOUT: Duration = Duration::from_millis(5000);

/// Maximum worker respawns permitted within [`RESPAWN_WINDOW`] before giving up.
pub const RESPAWN_MAX: usize = 3;

/// Sliding window over which [`RESPAWN_MAX`] respawns are counted.
pub const RESPAWN_WINDOW: Duration = Duration::from_secs(60);

/// Consecutive set timeouts (worker still alive) before it is diagnosed as hung.
pub const HUNG_TIMEOUT_LIMIT: u32 = 3;

/// Returns whether another worker respawn is permitted right now.
///
/// `true` when fewer than `max` of the `recent` respawn timestamps fall within
/// `window` before `now`.
#[must_use]
pub fn respawn_allowed(recent: &[Instant], now: Instant, window: Duration, max: usize) -> bool {
    let count = recent
        .iter()
        .filter(|&&t| now.saturating_duration_since(t) < window)
        .count();
    count < max
}

/// Tracks the single in-flight monitor refresh and its outcome.
///
/// Enforces the "one refresh in flight" invariant: each [`begin`](Self::begin)
/// hands out a fresh generation, and [`complete`](Self::complete) ignores any
/// result whose generation does not match the current one.
#[derive(Debug)]
pub struct RefreshTracker {
    in_progress: bool,
    generation: u64,
    started_at: Option<Instant>,
    last_refresh: Instant,
    last_successful: bool,
}

impl RefreshTracker {
    /// Creates a tracker with no refresh in progress, stamped at `now`.
    #[must_use]
    pub fn new(now: Instant) -> Self {
        Self {
            in_progress: false,
            generation: 0,
            started_at: None,
            last_refresh: now,
            last_successful: true,
        }
    }

    /// Begins a refresh and returns the generation the result must echo.
    pub fn begin(&mut self, now: Instant) -> u64 {
        self.generation += 1;
        self.in_progress = true;
        self.started_at = Some(now);
        self.generation
    }

    /// Records a completed refresh; results from a stale generation are ignored.
    pub fn complete(&mut self, generation: u64, now: Instant, found: bool) {
        if generation != self.generation {
            return;
        }
        self.in_progress = false;
        self.started_at = None;
        self.last_refresh = now;
        self.last_successful = found;
    }

    /// Aborts an in-flight refresh and invalidates any outstanding result.
    pub fn abort(&mut self) {
        self.generation += 1;
        self.in_progress = false;
        self.started_at = None;
        self.last_successful = false;
    }

    /// Returns whether an in-flight refresh has exceeded `timeout` since it began.
    #[must_use]
    pub fn timed_out(&self, now: Instant, timeout: Duration) -> bool {
        match self.started_at {
            Some(started) => self.in_progress && now.saturating_duration_since(started) >= timeout,
            None => false,
        }
    }

    /// Whether a refresh is currently in flight.
    #[must_use]
    pub fn in_progress(&self) -> bool {
        self.in_progress
    }

    /// Whether the last completed refresh found any monitors.
    #[must_use]
    pub fn last_successful(&self) -> bool {
        self.last_successful
    }

    /// Time elapsed since the last completed refresh.
    #[must_use]
    pub fn elapsed_since_refresh(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_refresh)
    }
}
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --lib reconcile`
Expected: PASS (8 tests).

- [ ] **Step 6: Gate and commit**

Run: `cargo fmt -- --check && cargo clippy -- -D warnings && cargo test`
Expected: all pass.

```bash
git add src/core/reconcile.rs src/core/mod.rs
git commit -m "feat(core): add RefreshTracker, respawn backoff, supervision constants"
```

---

### Task 2: Sequence-correlated set reconciliation in `MonitorState`

Replaces the blind `confirm_brightness`/`revert_pending` with seq-guarded logic, and lands the `seq` field on the DDC *set* messages together with its only consumers (`main.rs`, `ddc_worker.rs`) so the crate compiles at task end.

**Files:**
- Modify: `src/core/state.rs` (`MonitorState` ~172–228; `DdcCommand::SetBrightness` ~353; `BrightnessMessage::DdcSetResult` ~272)
- Modify: `src/main.rs` (`BrightnessController` struct ~105–128 + `new` ~141–158; `handle_ddc_set_result` ~267–300; `handle_adjust` ~404–435; `handle_message` DdcSetResult arm ~209–216)
- Modify: `src/platform/windows/ddc_worker.rs` (`run` match ~63; `handle_set_brightness` ~80–112)
- Test: in-module `#[cfg(test)]` in `src/core/state.rs`

**Interfaces:**
- Consumes: `RefreshTracker` etc. not needed here; `std::time::{Instant, Duration}`.
- Produces:
  - `pub struct PendingSet { pub value: u8, pub seq: u64, pub sent_at: Instant }`
  - `pub enum SetOutcome { Confirmed, Reverted, GroundTruth, Ignored }`
  - `MonitorState` field `pub pending: Option<PendingSet>` (replaces `pending_brightness`).
  - `MonitorState::set_pending(&mut self, value: u8, seq: u64, now: Instant)`
  - `MonitorState::apply_set_result(&mut self, seq: u64, value: u8, success: bool) -> SetOutcome`
  - `MonitorState::force_revert(&mut self)`
  - `MonitorState::pending_timed_out(&self, now: Instant, timeout: Duration) -> bool`
  - `DdcCommand::SetBrightness { monitor_id, value, seq: u64 }`
  - `BrightnessMessage::DdcSetResult { monitor_id, value, seq: u64, success, error }`

- [ ] **Step 1: Write the failing tests**

Add this test module at the end of `src/core/state.rs` (the file already has a `monitor_id_tests` module; add a new sibling module):

```rust
#[cfg(test)]
mod pending_reconcile_tests {
    use super::*;
    use std::time::Duration;

    fn state() -> MonitorState {
        MonitorState::new(50)
    }

    #[test]
    fn matched_success_confirms() {
        let mut s = state();
        s.set_pending(70, 1, Instant::now());
        assert_eq!(s.apply_set_result(1, 70, true), SetOutcome::Confirmed);
        assert_eq!(s.cached_brightness, 70);
        assert!(s.pending.is_none());
        assert_eq!(s.effective_brightness(), 70);
    }

    #[test]
    fn matched_failure_reverts() {
        let mut s = state();
        s.set_pending(70, 1, Instant::now());
        assert_eq!(s.apply_set_result(1, 70, false), SetOutcome::Reverted);
        assert_eq!(s.cached_brightness, 50);
        assert!(s.pending.is_none());
    }

    #[test]
    fn stale_failure_does_not_clear_newer_pending() {
        // Repro for the un-correlated-result drift: an earlier command's failure
        // must not clear the pending that belongs to a later in-flight command.
        let mut s = state();
        s.set_pending(60, 1, Instant::now());
        s.set_pending(80, 2, Instant::now());
        assert_eq!(s.apply_set_result(1, 60, false), SetOutcome::Ignored);
        let pending = s.pending.expect("newer pending survives stale result");
        assert_eq!(pending.value, 80);
        assert_eq!(pending.seq, 2);
    }

    #[test]
    fn late_success_after_force_revert_is_ground_truth() {
        let mut s = state();
        s.set_pending(90, 1, Instant::now());
        s.force_revert(); // watchdog cleared the pending
        assert!(s.pending.is_none());
        assert_eq!(s.apply_set_result(1, 90, true), SetOutcome::GroundTruth);
        assert_eq!(s.cached_brightness, 90);
    }

    #[test]
    fn late_failure_with_no_pending_is_ignored() {
        let mut s = state();
        s.force_revert();
        assert_eq!(s.apply_set_result(1, 90, false), SetOutcome::Ignored);
        assert_eq!(s.cached_brightness, 50);
    }

    #[test]
    fn refresh_datum_preserves_live_pending() {
        // A refresh read is older intent than a live optimistic set.
        let mut s = state();
        s.set_pending(75, 1, Instant::now());
        s.update_from_ddc(40);
        assert_eq!(s.cached_brightness, 40);
        let pending = s.pending.expect("live pending survives a refresh datum");
        assert_eq!(pending.value, 75);
        assert_eq!(s.effective_brightness(), 75);
    }

    #[test]
    fn pending_timed_out_respects_deadline() {
        let base = Instant::now();
        let mut s = state();
        s.set_pending(70, 1, base);
        assert!(!s.pending_timed_out(base + Duration::from_secs(7), SET_TIMEOUT));
        assert!(s.pending_timed_out(base + Duration::from_secs(8), SET_TIMEOUT));
    }

    #[test]
    fn pending_timed_out_false_when_idle() {
        let s = state();
        assert!(!s.pending_timed_out(Instant::now(), SET_TIMEOUT));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test --lib pending_reconcile`
Expected: FAIL — `SetOutcome`, `set_pending` arity, `apply_set_result`, `force_revert`, `pending`, `pending_timed_out`, `SET_TIMEOUT` unresolved.

- [ ] **Step 3: Update `MonitorState` and add the new types**

In `src/core/state.rs`, change the imports at the top of the file from:

```rust
use std::time::Instant;
```

to:

```rust
use std::time::{Duration, Instant};
```

Add the `SET_TIMEOUT` import to the same file's `use` section (near the top, after the `std` uses):

```rust
use crate::core::reconcile::SET_TIMEOUT;
```

Add the `PendingSet` and `SetOutcome` types immediately above `pub struct MonitorState`:

```rust
/// An optimistic brightness set awaiting its DDC result.
#[derive(Debug, Clone, Copy)]
pub struct PendingSet {
    /// Target brightness value (0-100).
    pub value: u8,
    /// Sequence id correlating this set to its DDC result.
    pub seq: u64,
    /// When the command was enqueued to the worker.
    pub sent_at: Instant,
}

/// Outcome of reconciling a DDC set result against the pending set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetOutcome {
    /// Matched the pending set and committed the new value.
    Confirmed,
    /// Matched the pending set and reverted after a hardware failure.
    Reverted,
    /// Late success with no matching pending: applied as authoritative truth.
    GroundTruth,
    /// Stale or irrelevant result: no state changed.
    Ignored,
}
```

Replace the `pending_brightness` field in `MonitorState` (line ~177):

```rust
    /// Optimistic brightness set awaiting DDC confirmation.
    pub pending: Option<PendingSet>,
```

In `MonitorState::new` (line ~190), replace `pending_brightness: None,` with:

```rust
            pending: None,
```

Replace `effective_brightness` (line ~200):

```rust
    #[must_use]
    pub fn effective_brightness(&self) -> u8 {
        self.pending.map_or(self.cached_brightness, |p| p.value)
    }
```

Replace `confirm_brightness`, `revert_pending`, and `set_pending` (lines ~205–220) with the new API:

```rust
    /// Records a new optimistic brightness set (awaiting DDC confirmation).
    pub fn set_pending(&mut self, value: u8, seq: u64, now: Instant) {
        self.pending = Some(PendingSet {
            value: value.min(100),
            seq,
            sent_at: now,
        });
    }

    /// Reconciles a DDC set result against the current pending set.
    ///
    /// A result matching the pending `seq` confirms (success) or reverts
    /// (failure). A result for an older `seq` than the current pending is
    /// ignored (a newer set is in flight). A success arriving when nothing is
    /// pending — e.g. after the watchdog already reverted — is authoritative:
    /// the hardware did change, so the cached value is updated as ground truth.
    pub fn apply_set_result(&mut self, seq: u64, value: u8, success: bool) -> SetOutcome {
        match self.pending {
            Some(pending) if pending.seq == seq => {
                if success {
                    self.cached_brightness = pending.value;
                    self.last_refresh = Instant::now();
                    self.pending = None;
                    SetOutcome::Confirmed
                } else {
                    self.pending = None;
                    SetOutcome::Reverted
                }
            }
            Some(_) => SetOutcome::Ignored,
            None => {
                if success {
                    self.cached_brightness = value.min(100);
                    self.last_refresh = Instant::now();
                    SetOutcome::GroundTruth
                } else {
                    SetOutcome::Ignored
                }
            }
        }
    }

    /// Unconditionally clears any pending set (used by the state watchdog).
    pub fn force_revert(&mut self) {
        self.pending = None;
    }

    /// Whether a pending set has been outstanding for at least `timeout`.
    #[must_use]
    pub fn pending_timed_out(&self, now: Instant, timeout: Duration) -> bool {
        self.pending
            .is_some_and(|p| now.saturating_duration_since(p.sent_at) >= timeout)
    }
```

Replace `update_from_ddc` (line ~223) so it preserves a live pending:

```rust
    /// Updates the cached brightness from a DDC read.
    ///
    /// Leaves any live pending set intact: a refresh read is older intent than
    /// an optimistic set that is still awaiting its own result.
    pub fn update_from_ddc(&mut self, value: u8) {
        self.cached_brightness = value.min(100);
        self.last_refresh = Instant::now();
    }
```

(Note: `SET_TIMEOUT` is imported so the in-module tests can reference it; the production `pending_timed_out` takes `timeout` as a parameter.)

- [ ] **Step 4: Add `seq` to the DDC set messages**

In `src/core/state.rs`, in `BrightnessMessage::DdcSetResult` (line ~272), add a `seq` field after `value`:

```rust
    DdcSetResult {
        /// Target monitor.
        monitor_id: MonitorId,
        /// The brightness value that was attempted.
        value: u8,
        /// Sequence id echoed from the originating command.
        seq: u64,
        /// Success or error message.
        success: bool,
        /// Error message if failed.
        error: Option<String>,
    },
```

In `DdcCommand::SetBrightness` (line ~353), add `seq`:

```rust
    SetBrightness {
        /// Target monitor.
        monitor_id: MonitorId,
        /// Brightness value to set (0-100).
        value: u8,
        /// Sequence id correlating this command to its result.
        seq: u64,
    },
```

- [ ] **Step 5: Echo `seq` from the worker**

In `src/platform/windows/ddc_worker.rs`, change the `SetBrightness` match arm in `run` (line ~63):

```rust
                DdcCommand::SetBrightness {
                    monitor_id,
                    value,
                    seq,
                } => {
                    self.handle_set_brightness(&monitor_id, value, seq);
                }
```

Change `handle_set_brightness`'s signature and the message it builds (lines ~80, ~102):

```rust
    fn handle_set_brightness(&mut self, monitor_id: &MonitorId, value: u8, seq: u64) {
```

and

```rust
        let msg = BrightnessMessage::DdcSetResult {
            monitor_id: monitor_id.clone(),
            value,
            seq,
            success,
            error,
        };
```

- [ ] **Step 6: Add the sequence counter and rewrite the set-result handler in the controller**

In `src/main.rs`, add a field to `BrightnessController` (after `usage_window: Option<UsageWindow>,` ~127):

```rust
    /// Monotonic sequence id stamped on each DDC set command.
    next_seq: u64,
```

In `BrightnessController::new` (~145), add to the struct literal (after `usage_window: None,`):

```rust
            next_seq: 0,
```

Add the `SetOutcome` import to the `state` use group at the top of `main.rs` (~20):

```rust
use darkbright_helper::core::state::{
    BrightnessMessage, DdcCommand, MonitorId, MonitorState, SetOutcome, TrayMenuData,
    TrayMonitorInfo, generate_display_names,
};
```

Replace `handle_ddc_set_result` (lines ~267–300) entirely:

```rust
    /// Handles the result of a DDC brightness set operation.
    ///
    /// Reconciles the result against the monitor's pending set by sequence id.
    /// A confirmed or authoritative-late result refreshes the OSD; a revert
    /// shows the error state; a stale result is dropped.
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
```

Update the `handle_message` `DdcSetResult` arm (lines ~209–216) to pass `seq`:

```rust
            BrightnessMessage::DdcSetResult {
                monitor_id,
                value,
                seq,
                success,
                error,
            } => {
                self.handle_ddc_set_result(&monitor_id, value, seq, success, error.as_deref())?;
            }
```

- [ ] **Step 7: Stamp the set command in `handle_adjust`**

In `src/main.rs` `handle_adjust`, replace the optimistic-update block (lines ~404–407):

```rust
        // 4. Optimistic update (only set pending if hardware is changing)
        let seq = self.next_seq;
        if new_hardware != old_hardware {
            self.next_seq += 1;
            state.set_pending(new_hardware, seq, Instant::now());
        }
        state.overlay_opacity = new_overlay;
```

Replace the DDC-send block (lines ~426–433) — still logs on error for now; the hard-fail revert lands in Task 5:

```rust
            log::debug!(monitor_id:% = target_id, old_hw = old_hardware, new_hw = new_hardware; "Sending DDC command");
            if let Err(e) = self.ddc_cmd_tx.send(DdcCommand::SetBrightness {
                monitor_id: target_id,
                value: new_hardware,
                seq,
            }) {
                log::error!(error:% = e; "Failed to send DDC command");
            }
```

- [ ] **Step 8: Run the tests to verify they pass**

Run: `cargo test --lib pending_reconcile`
Expected: PASS (8 tests).

- [ ] **Step 9: Gate and commit**

Run: `cargo fmt -- --check && cargo clippy -- -D warnings && cargo test`
Expected: all pass (the whole crate compiles; existing tests unaffected).

```bash
git add src/core/state.rs src/main.rs src/platform/windows/ddc_worker.rs
git commit -m "feat: correlate DDC set results by sequence id

Replace blind confirm/revert with seq-guarded apply_set_result: a
stale result no longer clears a newer pending, a late success becomes
ground truth, and a refresh read preserves a live pending."
```

---

### Task 3: Refresh generation via `RefreshTracker`

Swaps the three loose refresh fields for the `RefreshTracker` and stamps refreshes with a generation so a stale result cannot corrupt the tracker.

**Files:**
- Modify: `src/core/state.rs` (`DdcCommand::RefreshAll` ~360; `BrightnessMessage::DdcRefreshResult` ~284)
- Modify: `src/platform/windows/ddc_worker.rs` (`run` RefreshAll arm ~66; `handle_refresh_all` ~118; `send_refresh_result` ~184)
- Modify: `src/main.rs` (`BrightnessController` fields ~121–125 + `new` ~152–155; `handle_refresh` ~456–470; `handle_ddc_refresh_result` ~306–328; `check_periodic_refresh` ~476–497; `check_inactivity_refresh` ~540–555; `handle_adjust` failed-refresh retry ~341; `handle_message` DdcRefreshResult arm ~217–219)
- Test: covered by Task 1's `RefreshTracker` unit tests; this task is wiring, verified by compile + clippy + existing tests.

**Interfaces:**
- Consumes: `RefreshTracker` (Task 1); `DdcCommand`/`BrightnessMessage` (Task 2).
- Produces:
  - `DdcCommand::RefreshAll { generation: u64 }`
  - `BrightnessMessage::DdcRefreshResult { generation: u64, monitors: Vec<(MonitorId, u8)> }`
  - `BrightnessController` field `refresh: RefreshTracker` (replaces `last_refresh`, `refresh_in_progress`, `last_refresh_successful`).

- [ ] **Step 1: Stamp the refresh messages**

In `src/core/state.rs`, change `BrightnessMessage::DdcRefreshResult` (line ~284):

```rust
    DdcRefreshResult {
        /// Generation echoed from the originating refresh command.
        generation: u64,
        /// List of (`monitor_id`, brightness) pairs for all detected monitors.
        monitors: Vec<(MonitorId, u8)>,
    },
```

Change `DdcCommand::RefreshAll` (line ~360):

```rust
    /// Refresh all monitors: enumerate and read current brightness values.
    RefreshAll {
        /// Generation correlating this refresh to its result.
        generation: u64,
    },
```

- [ ] **Step 2: Echo the generation from the worker**

In `src/platform/windows/ddc_worker.rs`, change the `RefreshAll` arm in `run` (line ~66):

```rust
                DdcCommand::RefreshAll { generation } => {
                    self.handle_refresh_all(generation);
                }
```

Change `handle_refresh_all` to thread the generation through (lines ~118, ~133, ~144):

```rust
    fn handle_refresh_all(&mut self, generation: u64) {
```

Update its two `self.send_refresh_result(results)` calls to `self.send_refresh_result(generation, results)`, and change `send_refresh_result` (line ~184):

```rust
    fn send_refresh_result(&self, generation: u64, monitors: Vec<(MonitorId, u8)>) {
        let msg = BrightnessMessage::DdcRefreshResult {
            generation,
            monitors,
        };

        if let Err(e) = self.resp_tx.send(msg) {
            log::error!(error:% = e; "Failed to send refresh result");
        }
    }
```

- [ ] **Step 3: Replace the controller's refresh fields with `RefreshTracker`**

In `src/main.rs`, add the import to the core use group (top of file, ~18):

```rust
use darkbright_helper::core::reconcile::RefreshTracker;
```

Replace the three fields in `BrightnessController` (lines ~120–125):

```rust
    /// Timestamp of last user-initiated brightness adjustment.
    last_activity: Instant,
    /// Refresh lifecycle: in-flight state, generation, and last outcome.
    refresh: RefreshTracker,
```

In `BrightnessController::new` (lines ~152–155), replace the three initializers with:

```rust
            last_activity: now,
            refresh: RefreshTracker::new(now),
```

- [ ] **Step 4: Rewrite `handle_refresh` to use the tracker**

Replace `handle_refresh` (lines ~456–470):

```rust
    fn handle_refresh(&mut self) {
        log::info!("Requesting monitor refresh from DDC worker");

        // Clear ID cache since handles may change after refresh.
        self.id_cache.clear();

        let generation = self.refresh.begin(Instant::now());

        // Send refresh command to worker (non-blocking).
        if let Err(e) = self.ddc_cmd_tx.send(DdcCommand::RefreshAll { generation }) {
            log::error!(error:% = e; "Failed to send refresh command to DDC worker");
            self.refresh.abort();
        }
    }
```

- [ ] **Step 5: Rewrite `handle_ddc_refresh_result` to complete the tracker**

Replace `handle_ddc_refresh_result` (lines ~306–328). Its signature gains `generation`, and the tail updates the tracker instead of the three fields:

```rust
    fn handle_ddc_refresh_result(&mut self, generation: u64, monitors: Vec<(MonitorId, u8)>) {
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

        self.refresh.complete(generation, Instant::now(), found_monitors);
    }
```

Update the `handle_message` `DdcRefreshResult` arm (lines ~217–219):

```rust
            BrightnessMessage::DdcRefreshResult {
                generation,
                monitors,
            } => {
                self.handle_ddc_refresh_result(generation, monitors);
            }
```

- [ ] **Step 6: Update the refresh-flag readers**

In `check_periodic_refresh` (lines ~476–497), replace the field reads:

```rust
        // Skip if periodic refresh is disabled (0) or refresh already in progress.
        if periodic_seconds == 0 || self.refresh.in_progress() {
            return;
        }

        // Skip if last refresh found no monitors (will retry on user activity or resume).
        if !self.refresh.last_successful() {
            log::debug!("Skipping periodic refresh (last refresh found no monitors)");
            return;
        }

        let elapsed = self.refresh.elapsed_since_refresh(Instant::now());
```

In `check_inactivity_refresh` (line ~544), replace the guard:

```rust
        if inactivity_seconds == 0 || self.refresh.in_progress() {
            return;
        }
```

In `handle_adjust` (line ~341), replace the failed-refresh retry condition:

```rust
        if !self.refresh.last_successful() && !self.refresh.in_progress() {
```

- [ ] **Step 7: Gate and commit**

Run: `cargo fmt -- --check && cargo clippy -- -D warnings && cargo test`
Expected: all pass.

```bash
git add src/core/state.rs src/platform/windows/ddc_worker.rs src/main.rs
git commit -m "feat: track refreshes with a generation counter

Replace the three loose refresh fields with RefreshTracker and stamp
each refresh so a stale result cannot clear the in-progress flag of a
newer refresh."
```

---

### Task 4: `DdcSupervisor` and startup wiring

Wraps the worker in a supervisor that owns its `JoinHandle` and can respawn it, and routes all worker communication (and shutdown) through it. No behavior change yet — the supervisor is created and used as a transparent send channel; Task 5 activates supervision.

**Files:**
- Modify: `src/platform/windows/ddc_worker.rs` (add `DdcSupervisor` + `RespawnOutcome`)
- Modify: `src/platform/windows/mod.rs` (export ~38)
- Modify: `src/main.rs` (`use` ~33; `BrightnessController` field ~117 + `new` signature ~141; `handle_adjust` send ~428; `handle_refresh` send ~465; `main` startup ~778–796 + cleanup ~875–880)
- Test: none automated (thread/FFI glue); backoff decision is Task 1's `respawn_allowed`. Verified by compile + clippy + a manual smoke run.

**Interfaces:**
- Consumes: `respawn_allowed`, `RESPAWN_MAX`, `RESPAWN_WINDOW` (Task 1); `DdcWorker` (existing); `DdcCommand`, `BrightnessMessage` (Task 2/3).
- Produces:
  - `pub enum RespawnOutcome { Respawned, BackoffExceeded }`
  - `pub struct DdcSupervisor` with: `spawn(resp_tx: Sender<BrightnessMessage>) -> Self`, `send(&self, cmd: DdcCommand) -> Result<(), SendError<DdcCommand>>`, `is_alive(&self) -> bool`, `respawn(&mut self, now: Instant) -> RespawnOutcome`, `clear_backoff(&mut self)`, `shutdown(&self)`.

- [ ] **Step 1: Implement `DdcSupervisor`**

In `src/platform/windows/ddc_worker.rs`, extend the imports at the top:

```rust
use std::sync::mpsc::{Receiver, SendError, Sender};
use std::thread::JoinHandle;
use std::time::Instant;
```

(Leave the existing `use std::collections::HashMap;` and `windows`/`crate` imports; add the `reconcile` import.)

```rust
use crate::core::reconcile::{RESPAWN_MAX, RESPAWN_WINDOW, respawn_allowed};
```

Add at the end of the file, before the `#[cfg(test)]` module:

```rust
/// Result of a supervisor respawn attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RespawnOutcome {
    /// A fresh worker thread was spawned.
    Respawned,
    /// Too many respawns within the backoff window; the worker is left dead.
    BackoffExceeded,
}

/// Owns the DDC worker thread and can respawn it after a confirmed death.
///
/// The supervisor holds the command-channel sender, the worker's join handle,
/// and a persistent response-channel sender used to wire each new worker.
/// Respawns are rate-limited by a sliding window (see [`respawn_allowed`]).
pub struct DdcSupervisor {
    cmd_tx: Sender<DdcCommand>,
    handle: JoinHandle<()>,
    resp_tx: Sender<BrightnessMessage>,
    recent_respawns: Vec<Instant>,
}

impl DdcSupervisor {
    /// Spawns the initial worker and returns its supervisor.
    ///
    /// `resp_tx` is the channel the worker sends results on; the supervisor
    /// keeps a clone so it can wire replacement workers.
    #[must_use]
    pub fn spawn(resp_tx: Sender<BrightnessMessage>) -> Self {
        let (cmd_tx, handle) = Self::spawn_worker(&resp_tx);
        Self {
            cmd_tx,
            handle,
            resp_tx,
            recent_respawns: Vec::new(),
        }
    }

    /// Creates a fresh command channel and spawns a worker draining it.
    fn spawn_worker(resp_tx: &Sender<BrightnessMessage>) -> (Sender<DdcCommand>, JoinHandle<()>) {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<DdcCommand>();
        let worker = DdcWorker::new(cmd_rx, resp_tx.clone());
        let handle = std::thread::spawn(move || worker.run());
        (cmd_tx, handle)
    }

    /// Sends a command to the worker.
    ///
    /// # Errors
    ///
    /// Returns `SendError` if the worker's receiver has been dropped (the
    /// worker has died) — callers treat this as a hard failure.
    pub fn send(&self, cmd: DdcCommand) -> Result<(), SendError<DdcCommand>> {
        self.cmd_tx.send(cmd)
    }

    /// Whether the worker thread is still running.
    #[must_use]
    pub fn is_alive(&self) -> bool {
        !self.handle.is_finished()
    }

    /// Attempts to respawn a dead worker, honouring the backoff window.
    pub fn respawn(&mut self, now: Instant) -> RespawnOutcome {
        self.recent_respawns
            .retain(|&t| now.saturating_duration_since(t) < RESPAWN_WINDOW);

        if !respawn_allowed(&self.recent_respawns, now, RESPAWN_WINDOW, RESPAWN_MAX) {
            return RespawnOutcome::BackoffExceeded;
        }

        let (cmd_tx, handle) = Self::spawn_worker(&self.resp_tx);
        self.cmd_tx = cmd_tx;
        self.handle = handle;
        self.recent_respawns.push(now);
        RespawnOutcome::Respawned
    }

    /// Clears the respawn history so recovery can retry immediately.
    pub fn clear_backoff(&mut self) {
        self.recent_respawns.clear();
    }

    /// Asks the worker to shut down (best-effort; does not join).
    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(DdcCommand::Shutdown);
    }
}
```

- [ ] **Step 2: Export the supervisor**

In `src/platform/windows/mod.rs` (line ~38), change the `ddc_worker` re-export:

```rust
pub use ddc_worker::{DdcSupervisor, DdcWorker};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo build`
Expected: builds (nothing consumes `DdcSupervisor` yet; `DdcWorker` still used by the current `main`).

- [ ] **Step 4: Route the controller through the supervisor**

In `src/main.rs`, change the platform import (line ~33):

```rust
use darkbright_helper::platform::windows::{DdcSupervisor, PowerEventListener, TrayIcon, UsageWindow};
```

Replace the controller's channel field (line ~117):

```rust
    /// Supervised DDC worker (send commands, detect death, respawn).
    ddc: DdcSupervisor,
```

Change `BrightnessController::new`'s signature and body (lines ~141, ~151). Signature:

```rust
    fn new(config: Config, ddc: DdcSupervisor) -> Result<Self> {
```

In the struct literal, replace `ddc_cmd_tx,` with:

```rust
            ddc,
```

Replace the send in `handle_adjust` (from Task 2's Step 7 block, the `self.ddc_cmd_tx.send(...)`):

```rust
            if let Err(e) = self.ddc.send(DdcCommand::SetBrightness {
                monitor_id: target_id,
                value: new_hardware,
                seq,
            }) {
                log::error!(error:% = e; "Failed to send DDC command");
            }
```

Replace the send in `handle_refresh`:

```rust
        if let Err(e) = self.ddc.send(DdcCommand::RefreshAll { generation }) {
            log::error!(error:% = e; "Failed to send refresh command to DDC worker");
            self.refresh.abort();
        }
```

- [ ] **Step 5: Rewire `main` startup and shutdown**

In `src/main.rs` `main`, replace the channel/worker setup (lines ~778–796). Remove the standalone `ddc_cmd_tx`/`ddc_cmd_rx`/`ddc_shutdown_tx`/detached-spawn block and the old `BrightnessController::new(...)` call, replacing with:

```rust
    // Spawn the supervised DDC worker.
    let supervisor = DdcSupervisor::spawn(tx.clone());
    log::info!("DDC worker thread spawned");

    // Create controller.
    let mut controller = match BrightnessController::new(config.clone(), supervisor) {
        Ok(c) => c,
        Err(e) => {
            log::error!(error:% = e; "Failed to initialize BrightnessController");
            return;
        }
    };
```

Replace the cleanup block that sent the standalone shutdown (lines ~875–880):

```rust
    // Ask the DDC worker to shut down, then destroy windows.
    log::debug!("Sending shutdown command to DDC worker");
    controller.shutdown_worker();

    // Explicitly drop controller to ensure windows are destroyed before exit.
    drop(controller);
```

Add a `shutdown_worker` method to `BrightnessController` (next to `handle_shutdown`, ~230):

```rust
    /// Asks the supervised DDC worker to shut down.
    fn shutdown_worker(&self) {
        self.ddc.shutdown();
    }
```

- [ ] **Step 6: Gate and commit**

Run: `cargo fmt -- --check && cargo clippy -- -D warnings && cargo test`
Expected: all pass.

- [ ] **Step 7: Manual smoke test**

Run: `cargo run` (debug build shows the console). Confirm: monitors enumerate on startup (log `DDC refresh complete`), a hotkey brightness change works, and quitting from the tray logs `Sending shutdown command to DDC worker` then `DDC worker thread stopped`. No behavior regression vs. before.

```bash
git add src/platform/windows/ddc_worker.rs src/platform/windows/mod.rs src/main.rs
git commit -m "feat: run the DDC worker under a supervisor

Wrap the worker in DdcSupervisor (owns the JoinHandle, can respawn,
rate-limited by a backoff window) and route all commands and shutdown
through it. Supervision is activated in the next change."
```

---

### Task 5: Watchdog + supervision loop integration

Activates the two mechanisms: the main loop polls worker liveness and the state deadlines; a dead worker is respawned; stuck pendings and a latched refresh self-heal; a send failure hard-fails immediately; a hung worker escalates to a degraded state that recovers on user activity / resume.

**Files:**
- Modify: `src/main.rs` (`BrightnessController` fields + `new`; new methods `supervise_and_watchdog`, `supervise_worker`, `check_watchdogs`, `reconcile_all_pending`, `show_error_on_visible_osd`, `clear_degraded`; `handle_adjust` recovery + osd_monitor + send hard-fail; `handle_message` `SystemResumed` arm; main loop call)
- Test: decision logic is already unit-tested (Tasks 1–2); this task is glue, verified by compile + clippy + a manual fault-injection checklist.

**Interfaces:**
- Consumes: `SET_TIMEOUT`, `REFRESH_TIMEOUT`, `HUNG_TIMEOUT_LIMIT` (Task 1); `RespawnOutcome` (Task 4); `SetOutcome`, `force_revert`, `pending_timed_out` (Task 2); `RefreshTracker::{timed_out, abort}` (Task 1/3).
- Produces: no new public API; internal controller behavior.

- [ ] **Step 1: Add watchdog imports and fields**

In `src/main.rs`, add the constants + outcome imports. Extend the reconcile import (Task 3 added `RefreshTracker`):

```rust
use darkbright_helper::core::reconcile::{
    HUNG_TIMEOUT_LIMIT, REFRESH_TIMEOUT, RefreshTracker, SET_TIMEOUT,
};
```

Add the platform import for `RespawnOutcome` (extend the Task 4 line):

```rust
use darkbright_helper::platform::windows::{
    DdcSupervisor, PowerEventListener, RespawnOutcome, TrayIcon, UsageWindow,
};
```

Export `RespawnOutcome` from the platform module — in `src/platform/windows/mod.rs`:

```rust
pub use ddc_worker::{DdcSupervisor, DdcWorker, RespawnOutcome};
```

Add fields to `BrightnessController` (after `next_seq: u64,`):

```rust
    /// Throttle for the per-tick supervision/watchdog pass.
    last_health_check: Instant,
    /// Consecutive set timeouts while the worker is still alive (hang signal).
    consecutive_set_timeouts: u32,
    /// True when DDC is disabled after respawn backoff or a diagnosed hang.
    ddc_disabled: bool,
    /// Monitor whose state the OSD is currently showing (for error restyling).
    osd_monitor: Option<MonitorId>,
```

In `BrightnessController::new` (struct literal), add:

```rust
            next_seq: 0,
            last_health_check: now,
            consecutive_set_timeouts: 0,
            ddc_disabled: false,
            osd_monitor: None,
```

- [ ] **Step 2: Add the supervision + watchdog methods**

In `src/main.rs`, add these methods to `impl BrightnessController` (place them after `handle_ddc_refresh_result`):

```rust
    /// Runs one throttled supervision + watchdog pass (called each loop tick).
    fn supervise_and_watchdog(&mut self) {
        let now = Instant::now();
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
                self.handle_refresh();
            }
            RespawnOutcome::BackoffExceeded => {
                log::error!("DDC worker respawn backoff exceeded; disabling DDC until recovery");
                self.ddc_disabled = true;
                self.show_error_on_visible_osd();
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

    /// Clears the degraded DDC state so a fresh attempt can be made.
    fn clear_degraded(&mut self) {
        if self.ddc_disabled {
            log::info!("Recovering from degraded DDC state");
        }
        self.ddc_disabled = false;
        self.ddc.clear_backoff();
        self.consecutive_set_timeouts = 0;
    }
```

- [ ] **Step 3: Track the OSD's monitor and add the send hard-fail**

In `handle_adjust`, record the monitor whenever the OSD is shown/updated. Replace the "6. Show or update OSD" block (lines ~416–421):

```rust
        // 6. Show or update OSD with optimistic values.
        self.osd_monitor = Some(target_id.clone());
        if self.osd.is_visible() {
            self.osd.update(state)?;
        } else {
            self.osd.show(hmonitor, state)?;
        }
```

Replace the DDC-send block (from Task 4 Step 4) so a send failure hard-fails immediately:

```rust
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
```

Add the recovery trigger at the top of `handle_adjust`, right after `self.check_inactivity_refresh();` (line ~338):

```rust
        // User activity is a recovery signal for a degraded DDC state.
        if self.ddc_disabled {
            self.clear_degraded();
        }
```

- [ ] **Step 4: Recover on system resume**

In `handle_message`, replace the `SystemResumed` arm (lines ~179–182):

```rust
            BrightnessMessage::SystemResumed => {
                self.clear_degraded();
                log::info!(reason = "system_resume"; "Triggering refresh");
                self.handle_refresh();
            }
```

- [ ] **Step 5: Call the pass from the main loop**

In `main`, in the loop body, add the supervision pass right after `controller.check_periodic_refresh();` (line ~843):

```rust
        // Supervise the DDC worker and reconcile state deadlines.
        controller.supervise_and_watchdog();
```

- [ ] **Step 6: Gate**

Run: `cargo fmt -- --check && cargo clippy -- -D warnings && cargo test`
Expected: all pass.

- [ ] **Step 7: Manual fault-injection verification**

The decision logic is unit-tested; this confirms the glue fires at the right time. Use a **temporary** fault injection (revert before committing):

1. In `src/platform/windows/ddc_worker.rs` `handle_set_brightness`, temporarily add at the top:
   `if value == 42 { panic!("fault injection"); }`
2. `cargo run`. Set a monitor to a brightness of 42% via hotkeys.
3. Confirm the log shows, within ~250 ms: `DDC worker died; respawned` followed by `Requesting monitor refresh` and `DDC refresh complete`. Confirm the OSD does **not** stay stuck on a wrong value, and a subsequent brightness change works (proving the respawned worker is live).
4. Trigger it 4× quickly (within 60 s) and confirm `respawn backoff exceeded; disabling DDC` appears on the 4th, then a further hotkey press logs `Recovering from degraded DDC state` and works again.
5. **Remove the fault-injection line.** Re-run `cargo build` to confirm it's gone.

Also confirm no regression on the normal path: after leaving the app idle past the inactivity threshold, a single hotkey press adjusts brightness with **no** spurious red-OSD/error (verifies the 8 s `SET_TIMEOUT` backstop does not fire on a set queued behind the resulting refresh).

- [ ] **Step 8: Commit**

```bash
git add src/main.rs src/platform/windows/mod.rs
git commit -m "feat: supervise the DDC worker and self-heal stuck state

Poll worker liveness and pending/refresh deadlines each tick: respawn a
dead worker and reconcile its stuck pendings, hard-fail on send error,
back off and degrade on repeated failure, and recover on user activity
or resume. A hung worker escalates after repeated set timeouts."
```

---

## Self-Review

**Spec coverage:**
- Two-mechanism split (death detection vs. state watchdog) → Task 5 (`supervise_worker` vs. `check_watchdogs`). ✓
- Seq correlation, `apply_set_result` cases incl. late-success ground truth (#2) and stale-ignore (#1) → Task 2. ✓
- `update_from_ddc` preserves a live pending (#9) → Task 2, Step 3 + test. ✓
- `RefreshTracker` + generation stamping, stale-complete ignored (#3) → Tasks 1 + 3. ✓
- `DdcSupervisor` spawn/send/is_alive/respawn/clear_backoff/shutdown, backoff + prune (#5) → Task 4 (`respawn` retains-then-checks; `clear_backoff`). ✓
- Watchdog deadlines; `SET_TIMEOUT` as hung-worker backstop (#1) → Task 1 constant + Task 5 `check_watchdogs`. ✓
- Hung-worker escalation via `HUNG_TIMEOUT_LIMIT` (#6) → Task 5 `check_watchdogs`. ✓
- Visibility-gated red OSD, single-monitor rule (#8) → Task 5 `show_error_on_visible_osd` + `osd_monitor`. ✓
- Recovery via user activity + `SystemResumed`, clears backoff (#4, #5) → Task 5 `clear_degraded`. ✓
- Send-failure hard fail → Task 5, Step 3. ✓
- Startup/shutdown rewiring → Task 4, Step 5. ✓
- Tests for all pure logic → Tasks 1 (8 tests) + 2 (8 tests). ✓

**Placeholder scan:** No `TBD`/`TODO`/"add error handling"/"similar to Task N"; the only `panic!` is the explicitly-temporary, explicitly-removed fault injection in Task 5 Step 7. ✓

**Type consistency:** `apply_set_result(seq, value, success) -> SetOutcome` and `SetOutcome::{Confirmed, Reverted, GroundTruth, Ignored}` consistent across Task 2 and its use in Task 5. `RefreshTracker` method names (`begin`/`complete`/`abort`/`timed_out`/`in_progress`/`last_successful`/`elapsed_since_refresh`) consistent across Tasks 1, 3, 5. `DdcSupervisor` methods (`spawn`/`send`/`is_alive`/`respawn`/`clear_backoff`/`shutdown`) consistent across Tasks 4, 5. Message fields (`seq` on set, `generation` on refresh) consistent across state.rs, ddc_worker.rs, main.rs. ✓

**Note for the implementer:** several `main.rs` line numbers shift as earlier tasks edit the file; locate the named function/field rather than trusting the absolute line. Method signatures and field names are the source of truth.
