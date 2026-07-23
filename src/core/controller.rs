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
    // removed when the message dispatch lands
    #[allow(dead_code)]
    osd: Osd,
    /// Loaded configuration.
    config: Config,
    /// Cache mapping platform handles to monitor ids (avoids repeated EDID reads).
    id_cache: HashMap<MonitorHandle, MonitorId>,
    /// Supervised DDC worker.
    ddc: Ddc,
    /// Cursor-to-monitor resolution.
    // removed when the message dispatch lands
    #[allow(dead_code)]
    locator: Loc,
    /// Timestamp of last user-initiated brightness adjustment.
    last_activity: Instant,
    /// Refresh lifecycle: in-flight state, generation, and last outcome.
    refresh: RefreshTracker,
    /// Monotonic sequence id stamped on each DDC set command.
    // removed when the message dispatch lands
    #[allow(dead_code)]
    next_seq: u64,
    /// Throttle for the per-tick supervision/watchdog pass.
    // removed when the message dispatch lands
    #[allow(dead_code)]
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
    pub fn new(
        config: Config,
        osd: Osd,
        overlay: Ovl,
        ddc: Ddc,
        locator: Loc,
        now: Instant,
    ) -> Self {
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
    // removed when the message dispatch lands
    #[allow(dead_code)]
    // The caller destructures an owned `enumerated: Vec<MonitorId>` straight out
    // of the refresh-result message; taking it by value here avoids an extra
    // borrow indirection even though this function only ever reads it.
    #[allow(clippy::needless_pass_by_value)]
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

        let current =
            self.refresh
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
    // removed when the message dispatch lands
    #[allow(dead_code)]
    fn apply_absence_evidence(&mut self, enumerated: &[MonitorId], now: Instant) {
        let mut pruned: Vec<MonitorId> = Vec::new();

        for (id, state) in &mut self.states {
            if enumerated.contains(id) {
                state.missing_since = None;
            } else {
                match state.missing_since {
                    None => state.missing_since = Some(now),
                    Some(since) if now.saturating_duration_since(since) >= PRUNE_ABSENCE_WINDOW => {
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
    // removed when the message dispatch lands
    #[allow(dead_code)]
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
    // removed when the message dispatch lands
    #[allow(dead_code)]
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
    // removed when the message dispatch lands
    #[allow(dead_code)]
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
        assert!(
            !c.refresh.last_enumerated(),
            "abort freezes the periodic gate"
        );
    }

    #[test]
    fn refresh_result_applies_ground_truth_even_when_stale() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let stale = c.refresh.begin(base);
        let _current = c.refresh.begin(base);
        c.handle_ddc_refresh_result(stale, vec![(test_id(), 42)], vec![test_id()], base);
        assert_eq!(c.states[&test_id()].cached_brightness, 42);
        assert!(
            c.refresh.in_progress(),
            "stale completion leaves newer refresh in flight"
        );
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
        assert_eq!(
            sent_refresh_count(&c),
            before + 1,
            "gate stays open while enumerable"
        );
    }

    #[test]
    fn periodic_refresh_frozen_when_nothing_enumerated() {
        let base = Instant::now();
        let mut c = test_controller(base);
        c.config.refresh.periodic_seconds = 60;

        deliver_refresh(&mut c, vec![], vec![], base);
        let before = sent_refresh_count(&c);
        c.check_periodic_refresh(base + Duration::from_secs(61));
        assert_eq!(
            sent_refresh_count(&c),
            before,
            "empty enumerated set freezes cadence"
        );
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

        deliver_refresh(
            &mut c,
            vec![(id.clone(), 50)],
            vec![id.clone()],
            base + Duration::from_secs(60),
        );
        assert!(c.states[&id].missing_since.is_none());
    }

    #[test]
    fn burst_misses_within_seconds_do_not_prune() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);

        // Resume/respawn burst: two observations seconds apart.
        deliver_refresh(&mut c, vec![], vec![other_id()], base);
        deliver_refresh(
            &mut c,
            vec![],
            vec![other_id()],
            base + Duration::from_secs(5),
        );
        assert!(c.states.contains_key(&id), "window not spanned — no prune");
    }

    #[test]
    fn empty_enumerated_set_is_no_evidence() {
        let base = Instant::now();
        let mut c = test_controller(base);
        let id = seed(&mut c, test_id(), 50);

        deliver_refresh(&mut c, vec![], vec![], base);
        assert!(
            c.states[&id].missing_since.is_none(),
            "no information is no evidence"
        );
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
        deliver_refresh(
            &mut c,
            vec![],
            vec![other_id()],
            base + Duration::from_secs(120),
        );
        assert!(!c.states.contains_key(&id));

        deliver_refresh(
            &mut c,
            vec![(id.clone(), 80)],
            vec![id.clone()],
            base + Duration::from_secs(180),
        );
        let state = &c.states[&id];
        assert_eq!(state.cached_brightness, 80);
        assert_eq!(
            state.overlay_opacity, 0,
            "prune forgets the dim level by design"
        );
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
