//! Platform-agnostic reconciliation primitives for DDC supervision.
//!
//! Holds the wall-clock deadlines, the worker-respawn backoff decision, and the
//! refresh-tracking state machine. All decisions are pure functions of an
//! explicitly passed `now`, so they are unit-testable on any platform.

use std::time::{Duration, Instant};

/// Backstop deadline for a pending brightness set.
///
/// Deliberately larger than [`REFRESH_TIMEOUT`] plus a set's own execution
/// budget: a set can wait in the worker's serial command queue behind an
/// in-flight refresh, and its deadline is measured from enqueue time. A dead
/// worker is healed far faster by liveness detection, so this only ever fires
/// for a worker that is alive but hung inside a blocking DDC call.
pub const SET_TIMEOUT: Duration = Duration::from_secs(8);

/// Deadline for a full monitor refresh (enumeration plus per-monitor EDID read
/// and DDC brightness read, each up to ~120 ms across up to three attempts).
pub const REFRESH_TIMEOUT: Duration = Duration::from_secs(5);

/// Maximum worker respawns permitted within [`RESPAWN_WINDOW`] before giving up.
pub const RESPAWN_MAX: usize = 3;

/// Sliding window over which [`RESPAWN_MAX`] respawns are counted.
pub const RESPAWN_WINDOW: Duration = Duration::from_secs(60);

/// Consecutive set timeouts (worker still alive) before it is diagnosed as hung.
pub const HUNG_TIMEOUT_LIMIT: u32 = 3;

/// Minimum continuously observed enumeration absence before a monitor's state
/// is pruned. Spans at least two refresh observations, so a resume/respawn
/// refresh burst (seconds apart, while a dock's DP link is still training)
/// can never prune on its own.
pub const PRUNE_ABSENCE_WINDOW: Duration = Duration::from_secs(90);

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

/// Result of a supervisor respawn attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RespawnOutcome {
    /// A fresh worker thread was spawned.
    Respawned,
    /// Too many respawns within the backoff window; the worker is left dead.
    BackoffExceeded,
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
    last_enumerated: bool,
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
            last_enumerated: true,
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
    ///
    /// Returns `true` when the result matched the current generation and was
    /// recorded — the caller's license to treat it as absence evidence.
    /// `enumerated_any` reports whether the refresh identified any monitor at
    /// all (readable or not); it drives the periodic-refresh gate.
    pub fn complete(
        &mut self,
        generation: u64,
        now: Instant,
        found: bool,
        enumerated_any: bool,
    ) -> bool {
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

    /// Aborts an in-flight refresh and invalidates any outstanding result.
    pub fn abort(&mut self) {
        self.generation += 1;
        self.in_progress = false;
        self.started_at = None;
        self.last_successful = false;
        self.last_enumerated = false;
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

    /// Whether the last completed refresh enumerated any monitor (readable or not).
    #[must_use]
    pub fn last_enumerated(&self) -> bool {
        self.last_enumerated
    }

    /// Time elapsed since the last completed refresh.
    #[must_use]
    pub fn elapsed_since_refresh(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.last_refresh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn respawn_allowed_denies_when_window_full() {
        let base = Instant::now();
        let recent = [
            base,
            base + Duration::from_secs(1),
            base + Duration::from_secs(2),
        ];
        let now = base + Duration::from_secs(3);
        assert!(!respawn_allowed(&recent, now, Duration::from_secs(60), 3));
    }

    #[test]
    fn respawn_allowed_permits_when_spaced_beyond_window() {
        let base = Instant::now();
        let recent = [
            base,
            base + Duration::from_secs(1),
            base + Duration::from_secs(2),
        ];
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
    fn refresh_tracker_begin_marks_in_progress() {
        let base = Instant::now();
        let mut t = RefreshTracker::new(base);
        assert!(!t.in_progress());
        let _ = t.begin(base);
        assert!(t.in_progress());
    }

    #[test]
    fn refresh_tracker_begin_hands_out_distinct_generations_and_ignores_stale() {
        // Two overlapping refreshes: a completion for the first (superseded)
        // generation must be ignored while the second is still in flight.
        let base = Instant::now();
        let mut t = RefreshTracker::new(base);
        let first = t.begin(base);
        let second = t.begin(base);
        assert_ne!(first, second, "each begin hands out a fresh generation");
        t.complete(first, base + Duration::from_secs(1), true, true);
        assert!(
            t.in_progress(),
            "stale completion left the newer refresh in progress"
        );
        t.complete(second, base + Duration::from_secs(1), true, true);
        assert!(!t.in_progress());
        assert!(t.last_successful());
    }

    #[test]
    fn refresh_tracker_complete_records_failure() {
        let base = Instant::now();
        let mut t = RefreshTracker::new(base);
        let r#gen = t.begin(base);
        t.complete(r#gen, base + Duration::from_secs(1), false, false);
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
        let generation = t.begin(base);
        t.abort();
        assert!(!t.in_progress());
        assert!(!t.last_successful());
        // A late result for the aborted generation must not resurrect state.
        t.complete(generation, base + Duration::from_secs(1), true, true);
        assert!(!t.in_progress());
        assert!(!t.last_successful());
    }

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
        assert!(
            t.last_enumerated(),
            "stale completion must not touch the gate"
        );

        t.abort();
        assert!(!t.last_enumerated(), "abort freezes the periodic path");
    }
}
