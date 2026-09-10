use super::*;
use crate::core::config::{
    DEFAULT_FILE_LOG_LEVEL, DEFAULT_HOTKEY_DOWN, DEFAULT_HOTKEY_UP, DEFAULT_OSD_OPACITY,
    DEFAULT_OSD_TIMEOUT_MS, DEFAULT_REFRESH_INACTIVITY_SECONDS, DEFAULT_REFRESH_PERIODIC_SECONDS,
    DEFAULT_STEP_PERCENT,
};
use crate::core::i18n::{Lang, LanguageSetting};
use std::sync::mpsc;

// ── Fakes ────────────────────────────────────────────────────────────

#[derive(Default)]
struct FakeOsd {
    visible: bool,
    shows: Vec<(MonitorHandle, u8)>,
    updates: Vec<u8>,
    error_updates: Vec<u8>,
    appearance_calls: Vec<(f32, u32)>,
    languages: Vec<Lang>,
    fail: bool,
}

impl OsdSink for FakeOsd {
    fn show(&mut self, handle: MonitorHandle, state: &MonitorState) -> Result<()> {
        if self.fail {
            return Err(BrightnessError::ChannelSend);
        }
        self.visible = true;
        self.shows.push((handle, state.effective_brightness()));
        Ok(())
    }
    fn update(&mut self, state: &MonitorState) -> Result<()> {
        if self.fail {
            return Err(BrightnessError::ChannelSend);
        }
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
    fn set_appearance(&mut self, opacity: f32, timeout_ms: u32) {
        self.appearance_calls.push((opacity, timeout_ms));
    }
    fn set_language(&mut self, lang: Lang) {
        self.languages.push(lang);
    }
}

#[derive(Default)]
struct FakeOverlay {
    updates: Vec<(MonitorId, u8)>,
    removed: Vec<MonitorId>,
    fail_update: bool,
}

impl OverlaySink for FakeOverlay {
    fn update(&mut self, id: &MonitorId, _handle: MonitorHandle, opacity: u8) -> Result<()> {
        if self.fail_update {
            return Err(BrightnessError::ChannelSend);
        }
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

#[derive(Default)]
struct FakeSettings {
    opened: Vec<SettingsSnapshot>,
    refreshed: Vec<SettingsSnapshot>,
    errors: Vec<String>,
    notices: Vec<String>,
    topmost_asserts: u32,
    languages: Vec<Lang>,
}

impl SettingsSink for FakeSettings {
    fn open(&mut self, snapshot: &SettingsSnapshot) {
        self.opened.push(snapshot.clone());
    }
    fn refresh(&mut self, snapshot: &SettingsSnapshot) {
        self.refreshed.push(snapshot.clone());
    }
    fn hotkey_error(&mut self, message: &str) {
        self.errors.push(message.to_string());
    }
    fn hotkey_notice(&mut self, message: &str) {
        self.notices.push(message.to_string());
    }
    fn assert_topmost(&mut self) {
        self.topmost_asserts += 1;
    }
    fn set_language(&mut self, lang: Lang) {
        self.languages.push(lang);
    }
}

#[derive(Default)]
struct FakeHotkeyPort {
    rebinds: Vec<(String, String, bool)>,
    suspends: u32,
    resumes: u32,
    /// When true, the next call returns `Err` (one-shot).
    fail_next: bool,
}

impl FakeHotkeyPort {
    /// Consumes `fail_next` and reports whether this call should fail.
    fn take_fail(&mut self) -> bool {
        std::mem::take(&mut self.fail_next)
    }
}

impl HotkeyPort for FakeHotkeyPort {
    fn rebind(&mut self, up: &str, down: &str, intercept: bool) -> Result<()> {
        if self.take_fail() {
            return Err(BrightnessError::ChannelSend);
        }
        self.rebinds
            .push((up.to_string(), down.to_string(), intercept));
        Ok(())
    }
    fn suspend(&mut self) -> Result<()> {
        if self.take_fail() {
            return Err(BrightnessError::ChannelSend);
        }
        self.suspends += 1;
        Ok(())
    }
    fn resume(&mut self) -> Result<()> {
        if self.take_fail() {
            return Err(BrightnessError::ChannelSend);
        }
        self.resumes += 1;
        Ok(())
    }
}

#[derive(Default)]
struct FakeStore {
    saves: Vec<(Config, SettingsDirty, bool)>,
    result: Option<SaveResult>,
}

impl ConfigStore for FakeStore {
    fn save(&mut self, config: &Config, dirty: &SettingsDirty, force: bool) -> SaveResult {
        self.saves.push((config.clone(), *dirty, force));
        self.result.clone().unwrap_or(SaveResult::Saved)
    }
}

// ── Helpers ──────────────────────────────────────────────────────────

type TestController =
    Controller<FakeOsd, FakeOverlay, FakeDdc, FakeLocator, FakeSettings, FakeHotkeyPort, FakeStore>;

fn test_id() -> MonitorId {
    MonitorId::new("DEL", "U2722D", Some("SN123".to_string()))
}

fn other_id() -> MonitorId {
    MonitorId::new("PHL", "346B1C", Some("SN456".to_string()))
}

fn test_controller(base: Instant) -> TestController {
    test_controller_with(Config::default(), Vec::new(), base)
}

fn test_controller_with(
    config: Config,
    os_languages: Vec<String>,
    base: Instant,
) -> TestController {
    Controller::new(
        config,
        os_languages,
        FakeOsd::default(),
        FakeOverlay::default(),
        FakeDdc::default(),
        FakeLocator {
            handle: MonitorHandle(1),
            id: test_id(),
        },
        FakeSettings::default(),
        FakeHotkeyPort::default(),
        FakeStore::default(),
        base,
    )
}

fn german_os() -> Vec<String> {
    vec!["de-DE".to_string(), "en-US".to_string()]
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
    seed(&mut c, test_id(), 50);
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

// ── Enumerable-but-unreadable monitors ───────────────────────────────

#[test]
fn an_enumerated_but_unreadable_monitor_is_seeded_so_it_stays_controllable() {
    let base = Instant::now();
    let mut c = test_controller(base);

    // The panel identifies (EDID readable) but NAKs the VCP read. The
    // worker keeps its handle so writes are still attempted.
    deliver_refresh(&mut c, vec![], vec![test_id()], base);

    let state = c
        .states
        .get(&test_id())
        .expect("unreadable monitor gets state");
    assert_eq!(
        state.cached_brightness, UNREAD_BRIGHTNESS_SEED,
        "seeded at the documented midpoint"
    );
    assert!(
        !state.brightness_known,
        "the seed is a guess, not an observation"
    );
}

#[test]
fn a_seeded_monitor_accepts_an_adjustment_instead_of_a_dead_keypress() {
    let base = Instant::now();
    let mut c = test_controller(base);
    deliver_refresh(&mut c, vec![], vec![test_id()], base);

    c.handle_adjust(None, 10, base)
        .expect("an unreadable but writable monitor must still adjust");

    assert!(
        matches!(
            c.ddc.sent.last(),
            Some(DdcCommand::SetBrightness { value: 60, .. })
        ),
        "the write reaches the hardware"
    );
    assert!(c.osd.visible, "and the user gets feedback");
}

#[test]
fn a_later_successful_read_replaces_the_seed() {
    let base = Instant::now();
    let mut c = test_controller(base);
    deliver_refresh(&mut c, vec![], vec![test_id()], base);

    deliver_refresh(&mut c, vec![(test_id(), 70)], vec![test_id()], base);

    let state = &c.states[&test_id()];
    assert_eq!(state.cached_brightness, 70);
    assert!(state.brightness_known, "an observation outranks the seed");
}

#[test]
fn a_confirmed_set_establishes_a_seeded_monitors_brightness() {
    let base = Instant::now();
    let mut c = test_controller(base);
    deliver_refresh(&mut c, vec![], vec![test_id()], base);
    c.handle_adjust(None, 10, base).unwrap();

    c.handle_message(
        BrightnessMessage::DdcSetResult {
            monitor_id: test_id(),
            value: 60,
            seq: 0,
            success: true,
            error: None,
        },
        base,
    )
    .unwrap();

    let state = &c.states[&test_id()];
    assert_eq!(state.cached_brightness, 60);
    assert!(
        state.brightness_known,
        "a write the hardware accepted establishes the value"
    );
}

#[test]
fn seeding_never_overwrites_a_monitor_that_already_has_state() {
    let base = Instant::now();
    let mut c = test_controller(base);
    seed(&mut c, test_id(), 80);

    // Read starts failing (standby, KVM) while the panel stays enumerable.
    deliver_refresh(&mut c, vec![], vec![test_id()], base);

    let state = &c.states[&test_id()];
    assert_eq!(state.cached_brightness, 80, "last known value survives");
    assert!(state.brightness_known, "and stays an observation");
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

#[test]
fn periodic_refresh_frozen_after_abort() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.config.refresh.periodic_seconds = 60;

    // A send failure aborts the refresh, same gate as an empty enumerated set.
    c.ddc.fail_send = true;
    c.handle_refresh(base);
    c.ddc.fail_send = false;

    let before = sent_refresh_count(&c);
    c.check_periodic_refresh(base + Duration::from_secs(61));
    assert_eq!(
        sent_refresh_count(&c),
        before,
        "abort freezes cadence same as an empty enumerated set"
    );
}

// ── Overlay reconcile on external change ─────────────────────────────

#[test]
fn refresh_clears_overlay_when_hardware_changed_externally() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 0);
    c.states.get_mut(&id).unwrap().overlay_opacity = 40;

    // Physical buttons (or a monitor self-reset) raised the hardware
    // brightness while the sub-zero overlay was active.
    deliver_refresh(&mut c, vec![(id.clone(), 80)], vec![id.clone()], base);

    let state = &c.states[&id];
    assert_eq!(state.cached_brightness, 80);
    assert_eq!(
        state.overlay_opacity, 0,
        "external brightness change must clear the software veil"
    );
    assert_eq!(c.overlay.removed, vec![id]);
}

#[test]
fn refresh_keeps_overlay_at_hardware_floor() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 0);
    c.states.get_mut(&id).unwrap().overlay_opacity = 40;

    // Hardware still reads 0: nothing changed externally, sub-zero
    // dimming stays.
    deliver_refresh(&mut c, vec![(id.clone(), 0)], vec![id.clone()], base);

    assert_eq!(c.states[&id].overlay_opacity, 40);
    assert!(c.overlay.removed.is_empty());
}

#[test]
fn refresh_keeps_overlay_while_set_is_pending() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 5);
    {
        let state = c.states.get_mut(&id).unwrap();
        state.overlay_opacity = 40;
        // The user is dimming into sub-zero right now; the refresh read
        // predates the in-flight set and must not undo the fresh overlay.
        state.set_pending(0, 7, base);
    }

    deliver_refresh(&mut c, vec![(id.clone(), 5)], vec![id.clone()], base);

    assert_eq!(c.states[&id].overlay_opacity, 40);
    assert!(c.overlay.removed.is_empty());
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

// ── Adjust / optimistic update ───────────────────────────────────────

#[test]
fn adjust_applies_optimistic_update_and_sends_seq_stamped_command() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 50);

    c.handle_adjust(None, 10, base).unwrap();

    let state = &c.states[&id];
    assert_eq!(
        state.effective_brightness(),
        60,
        "optimistic pending visible"
    );
    assert_eq!(state.cached_brightness, 50, "cache untouched until confirm");
    assert!(c.osd.visible, "OSD shown immediately");
    assert!(matches!(
        c.ddc.sent.last(),
        Some(DdcCommand::SetBrightness {
            value: 60,
            seq: 0,
            ..
        })
    ));
    assert_eq!(c.osd_monitor.as_ref(), Some(&id));
}

#[test]
fn adjust_step_multiplies_by_live_step_percent() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 50);

    c.config.brightness.step_percent = 7;
    c.handle_message(BrightnessMessage::AdjustStep { direction: -1 }, base)
        .unwrap();

    assert_eq!(
        c.states[&id].effective_brightness(),
        43,
        "delta must use the live step_percent (7), not a value frozen at hotkey-thread spawn"
    );
}

#[test]
fn adjust_osd_failure_still_dispatches_ddc_command() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 50);
    c.osd.fail = true;

    c.handle_adjust(None, 10, base)
        .expect("OSD failure is feedback-only and must not abort the adjustment");

    assert_eq!(
        c.states[&id].effective_brightness(),
        60,
        "optimistic pending stays in place"
    );
    assert!(
        matches!(
            c.ddc.sent.last(),
            Some(DdcCommand::SetBrightness { value: 60, .. })
        ),
        "DDC command must be dispatched despite the OSD failure"
    );
}

#[test]
fn adjust_without_change_still_shows_osd_but_sends_nothing() {
    let base = Instant::now();
    let mut c = test_controller(base);
    seed(&mut c, test_id(), 100);

    c.handle_adjust(None, 10, base).unwrap();

    assert!(c.osd.visible);
    assert!(
        c.ddc.sent.is_empty(),
        "no hardware or overlay change to apply"
    );
}

#[test]
fn adjust_below_zero_dims_via_overlay_without_pending_or_ddc() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 0);

    c.handle_adjust(None, -10, base).unwrap();

    assert!(
        c.states[&id].pending.is_none(),
        "no hardware set to confirm"
    );
    assert_eq!(c.states[&id].overlay_opacity, 10);
    assert_eq!(c.overlay.updates, vec![(id, 10)]);
    assert!(
        c.ddc.sent.is_empty(),
        "overlay-only change sends no DDC command"
    );
}

#[test]
fn adjust_overlay_failure_reverts_and_shows_error() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 0);
    let prior_overlay = c.states[&id].overlay_opacity;
    c.overlay.fail_update = true;

    // A failed overlay call is fully handled here — reverted and shown on
    // the OSD — so the press reports success and the main loop has nothing
    // left to log.
    c.handle_adjust(None, -10, base).unwrap();

    assert_eq!(
        c.states[&id].overlay_opacity, prior_overlay,
        "opacity must not be committed when the platform call fails"
    );
    assert!(
        c.states[&id].pending.is_none(),
        "no hardware pending should survive an overlay failure"
    );
    if c.osd.is_visible() {
        assert!(!c.osd.error_updates.is_empty(), "OSD restyled to error");
    }
}

#[test]
fn adjust_overlay_failure_reverts_real_pending_and_restyles_visible_osd() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 0);

    // Setup press: hardware is already 0, so `-10` only spills onto the
    // overlay (0 -> 10) and leaves no hardware pending -- see
    // `calculate_decrease`. It also shows the OSD the same way
    // `adjust_send_failure_reverts_and_marks_visible_osd` does, since
    // showing happens before the failure this test injects next.
    c.handle_adjust(None, -10, base).unwrap();
    assert!(c.osd.is_visible(), "setup press shows the OSD");
    assert!(
        c.states[&id].pending.is_none(),
        "setup press is overlay-only"
    );
    let prior_overlay = c.states[&id].overlay_opacity;
    assert_eq!(prior_overlay, 10);

    // From (hardware=0, overlay=10), +15 drains the overlay first
    // (10 -> 0, using 10 of the delta) then spills the remaining 5 onto
    // hardware (0 -> 5) -- see `calculate_increase`. This single press
    // changes both, so it sets a real hardware pending before the
    // overlay call fails; the failure must revert that pending.
    c.overlay.fail_update = true;
    c.handle_adjust(None, 15, base).unwrap();

    assert!(
        c.states[&id].pending.is_none(),
        "hardware pending set this press must be reverted"
    );
    assert_eq!(
        c.states[&id].overlay_opacity, prior_overlay,
        "opacity must not be committed when the platform call fails"
    );
    assert!(
        !c.osd.error_updates.is_empty(),
        "visible OSD restyled to error"
    );
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
fn adjust_clears_an_exhausted_respawn_backoff() {
    let base = Instant::now();
    let mut c = test_controller(base);
    seed(&mut c, test_id(), 50);
    c.ddc_health = DdcHealth::WorkerDead;

    c.handle_adjust(None, 10, base).unwrap();

    assert_eq!(
        c.ddc_health,
        DdcHealth::Ok,
        "a keypress clears the backoff so the next pass can respawn"
    );
    assert_eq!(c.ddc.backoff_clears, 1);
}

#[test]
fn adjust_does_not_retract_a_hung_worker_diagnosis() {
    let base = Instant::now();
    let mut c = test_controller(base);
    seed(&mut c, test_id(), 50);
    c.ddc_health = DdcHealth::WorkerHung;

    c.handle_adjust(None, 10, base).unwrap();

    assert_eq!(
        c.ddc_health,
        DdcHealth::WorkerHung,
        "a keypress cannot unstick a blocked DDC call, so the warning stands"
    );
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

    c.handle_ddc_set_result(&id, 60, 0, false, Some("nak"))
        .unwrap();

    assert_eq!(
        c.states[&id].effective_brightness(),
        50,
        "reverted to cache"
    );
    assert!(!c.osd.error_updates.is_empty());
}

#[test]
fn stale_set_result_is_ignored() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 50);
    c.handle_adjust(None, 10, base).unwrap(); // seq 0
    c.handle_adjust(None, 10, base).unwrap(); // seq 1, pending 70

    c.handle_ddc_set_result(&id, 60, 0, false, Some("late"))
        .unwrap();

    let pending = c.states[&id].pending.expect("newer pending survives");
    assert_eq!(pending.seq, 1);
}

#[test]
fn set_result_for_unknown_monitor_is_dropped() {
    let base = Instant::now();
    let mut c = test_controller(base);
    // Routine after pruning: a late result for a removed monitor.
    c.handle_ddc_set_result(&other_id(), 60, 0, true, None)
        .unwrap();
    assert!(c.states.is_empty(), "no ghost resurrection");
}

// ── Supervision / watchdogs ──────────────────────────────────────────

/// Advances past the 250 ms health-check throttle and runs one pass.
fn supervise_at(c: &mut TestController, now: Instant) {
    c.last_health_check = now
        .checked_sub(Duration::from_secs(1))
        .expect("now is derived from an Instant::now() baseline with headroom");
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
    assert!(
        c.states[&id].missing_since.is_none(),
        "evidence discarded on respawn"
    );
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

    assert_eq!(c.ddc_health, DdcHealth::WorkerDead);
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
        assert!(
            c.states[&id].pending.is_none(),
            "watchdog reverted the pending"
        );
    }

    assert_eq!(c.consecutive_set_timeouts, HUNG_TIMEOUT_LIMIT);
    assert_eq!(
        c.ddc_health,
        DdcHealth::WorkerHung,
        "an alive-but-unresponsive worker is a different condition from a dead one"
    );
}

/// Puts the controller in the state a hung worker leaves behind: diagnosed
/// unresponsive, with the timeout counter at the limit that got it there.
fn diagnosed_hung(c: &mut TestController) {
    c.ddc_health = DdcHealth::WorkerHung;
    c.consecutive_set_timeouts = HUNG_TIMEOUT_LIMIT;
}

#[test]
fn a_hung_worker_that_later_dies_is_respawned_without_a_keypress() {
    let base = Instant::now();
    let mut c = test_controller(base);
    seed(&mut c, test_id(), 50);
    diagnosed_hung(&mut c);

    // The blocked thread finally unwound and exited. The reason not to
    // respawn a hung worker — two threads against the same physical-monitor
    // handles — died with it, and since a keypress no longer clears this
    // state, nothing else would ever get the worker back.
    c.ddc.alive = false;
    supervise_at(&mut c, base + Duration::from_secs(1));

    assert_eq!(c.ddc.respawns, 1, "a dead worker is dead, hung or not");
    assert_eq!(c.ddc_health, DdcHealth::Ok);
}

#[test]
fn a_set_result_retracts_the_hung_diagnosis() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 50);
    diagnosed_hung(&mut c);

    // The blocked call finally returned and the worker reported it.
    c.handle_ddc_set_result(&id, 60, 0, true, None).unwrap();

    assert_eq!(
        c.ddc_health,
        DdcHealth::Ok,
        "a worker that answers is by definition not blocked"
    );
    assert_eq!(c.consecutive_set_timeouts, 0);
}

#[test]
fn a_failed_set_result_is_proof_of_life_too() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 50);
    diagnosed_hung(&mut c);

    // The worker ran the command and reported a NAK. The set failed; the
    // worker did not — a hang is about not answering, not about failing.
    c.handle_ddc_set_result(&id, 60, 0, false, Some("nak"))
        .unwrap();

    assert_eq!(c.ddc_health, DdcHealth::Ok);
    assert_eq!(c.consecutive_set_timeouts, 0);
}

#[test]
fn a_refresh_result_is_proof_of_life_too() {
    let base = Instant::now();
    let mut c = test_controller(base);
    seed(&mut c, test_id(), 50);
    diagnosed_hung(&mut c);

    deliver_refresh(&mut c, vec![(test_id(), 40)], vec![test_id()], base);

    assert_eq!(c.ddc_health, DdcHealth::Ok);
    assert_eq!(c.consecutive_set_timeouts, 0);
}

#[test]
fn a_stale_set_result_is_still_proof_of_life() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 50);
    c.handle_adjust(None, 10, base).unwrap(); // seq 0
    c.handle_adjust(None, 10, base).unwrap(); // seq 1 is now pending
    diagnosed_hung(&mut c);

    // A result the reconciler discards (superseded seq) still says the
    // worker is executing commands, which is all the diagnosis was about.
    c.handle_ddc_set_result(&id, 60, 0, true, None).unwrap();

    assert_eq!(c.ddc_health, DdcHealth::Ok);
}

#[test]
fn multi_monitor_timeout_counts_one_hang_signal_per_pass() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let a = seed(&mut c, test_id(), 50);
    let b = seed(&mut c, other_id(), 50);

    c.handle_adjust(Some(a.clone()), 10, base).unwrap();
    c.handle_adjust(Some(b.clone()), 10, base).unwrap();
    assert!(c.states[&a].pending.is_some());
    assert!(c.states[&b].pending.is_some());

    supervise_at(&mut c, base + SET_TIMEOUT);

    assert!(c.states[&a].pending.is_none(), "both pendings reverted");
    assert!(c.states[&b].pending.is_none(), "both pendings reverted");
    assert_eq!(
        c.consecutive_set_timeouts, 1,
        "one pass is one hang signal regardless of monitor count"
    );
    assert!(
        !c.ddc_health.is_degraded(),
        "a single pass must not reach the hang limit"
    );
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

// ── Dispatch ─────────────────────────────────────────────────────────

#[test]
fn system_resumed_clears_degraded_resets_evidence_and_refreshes() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let id = seed(&mut c, test_id(), 50);
    deliver_refresh(&mut c, vec![], vec![other_id()], base);
    // A hang is the harder case: unlike a keypress, a resume can genuinely
    // end one — the panel that stalled the bus was likely asleep.
    c.ddc_health = DdcHealth::WorkerHung;

    let cont = c
        .handle_message(BrightnessMessage::SystemResumed, base)
        .unwrap();

    assert!(cont);
    assert_eq!(c.ddc_health, DdcHealth::Ok);
    assert!(
        c.states[&id].missing_since.is_none(),
        "evidence discarded on resume"
    );
    assert!(c.refresh.in_progress());
}

#[test]
fn quit_and_shutdown_stop_the_loop() {
    let base = Instant::now();
    let mut c = test_controller(base);
    assert!(
        !c.handle_message(BrightnessMessage::TrayRequestQuit, base)
            .unwrap()
    );
    assert!(!c.handle_message(BrightnessMessage::Shutdown, base).unwrap());
}

#[test]
fn shell_variants_are_noops_here() {
    let base = Instant::now();
    let mut c = test_controller(base);
    assert!(
        c.handle_message(BrightnessMessage::TrayOpenLogFolder, base)
            .unwrap()
    );
    assert!(
        c.handle_message(BrightnessMessage::OpenConfigFile, base)
            .unwrap()
    );
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
}

#[test]
fn tray_menu_monitors_are_sorted_by_display_name() {
    let base = Instant::now();
    let mut c = test_controller(base);
    // test_id() -> "DEL U2722D", other_id() -> "PHL 346B1C": "DEL..." sorts first.
    seed(&mut c, other_id(), 30);
    seed(&mut c, test_id(), 55);

    let (reply_tx, reply_rx) = mpsc::channel();
    c.handle_message(BrightnessMessage::TrayMenuOpening { reply_tx }, base)
        .unwrap();

    let data = reply_rx.try_recv().expect("menu data sent");
    let names: Vec<&str> = data
        .monitors
        .iter()
        .map(|m| m.display_name.as_str())
        .collect();
    assert_eq!(
        names,
        vec!["DEL U2722D", "PHL 346B1C"],
        "monitor list must be in stable, sorted order regardless of HashMap iteration"
    );
}

#[test]
fn tray_menu_data_reports_the_ddc_condition_not_just_that_there_is_one() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.ddc_health = DdcHealth::WorkerHung;

    let (reply_tx, reply_rx) = mpsc::channel();
    c.handle_message(BrightnessMessage::TrayMenuOpening { reply_tx }, base)
        .unwrap();

    let data = reply_rx.try_recv().expect("menu data sent");
    assert_eq!(
        data.warnings.ddc,
        DdcHealth::WorkerHung,
        "the menu picks its wording from the cause, so the cause must survive"
    );
    assert!(!data.warnings.hotkeys_lost);
}

#[test]
fn a_failed_file_log_attach_is_surfaced_and_latched() {
    let base = Instant::now();
    let mut c = test_controller(base);
    seed(&mut c, test_id(), 50);

    c.set_file_log_failed();

    assert!(
        c.health_warnings().file_log_failed,
        "the console warning is invisible in release; the tray is the channel that exists"
    );

    // The attach is attempted exactly once at startup, so nothing that
    // happens afterwards can make the log appear.
    c.handle_adjust(None, 10, base).unwrap();
    c.handle_message(BrightnessMessage::SystemResumed, base)
        .unwrap();
    assert!(
        c.health_warnings().file_log_failed,
        "no recovery path exists, so the warning must not clear"
    );
}

#[test]
fn tray_menu_data_reports_hotkeys_lost_latch() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.set_hotkeys_lost();

    let (reply_tx, reply_rx) = mpsc::channel();
    c.handle_message(BrightnessMessage::TrayMenuOpening { reply_tx }, base)
        .unwrap();

    let data = reply_rx.try_recv().expect("menu data sent");
    assert!(data.warnings.hotkeys_lost);
    assert!(!data.warnings.ddc.is_degraded());
}

#[test]
fn tray_menu_data_carries_the_live_hotkey_bindings() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Ctrl+F5".to_string())),
        base,
    )
    .unwrap();
    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: true,
            fallback_active: false,
            error: None,
            restore_error: None,
        },
        base,
    )
    .unwrap();

    let (reply_tx, reply_rx) = mpsc::channel();
    c.handle_message(BrightnessMessage::TrayMenuOpening { reply_tx }, base)
        .unwrap();

    let data = reply_rx.try_recv().expect("menu data sent");
    assert_eq!(data.hotkey_up, "Ctrl+F5");
    assert_eq!(data.hotkey_down, DEFAULT_HOTKEY_DOWN);
}

#[test]
fn hotkeys_lost_latch_survives_ddc_recovery() {
    let base = Instant::now();
    let mut c = test_controller(base);
    seed(&mut c, test_id(), 50);
    c.ddc_health = DdcHealth::WorkerDead;
    c.set_hotkeys_lost();

    // User activity clears the degraded DDC state, but a dead hotkey
    // thread cannot come back without an app restart.
    c.handle_adjust(None, 10, base).unwrap();

    let warnings = c.health_warnings();
    assert!(!warnings.ddc.is_degraded(), "activity recovers DDC");
    assert!(warnings.hotkeys_lost, "hotkey give-up is latched");
}

#[test]
fn refresh_result_message_routes_with_enumerated_set() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let ghost = seed(&mut c, other_id(), 50);
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
    assert!(
        c.states[&ghost].missing_since.is_some(),
        "enumerated set reached absence bookkeeping"
    );
}

#[test]
fn refresh_message_requests_refresh_unconditionally() {
    let base = Instant::now();
    let mut c = test_controller(base);

    assert!(c.handle_message(BrightnessMessage::Refresh, base).unwrap());

    assert_eq!(sent_refresh_count(&c), 1);
    assert!(c.refresh.in_progress());
}

// ── Settings dialog: simple settings, debounced save, open/close ──────

#[test]
fn setting_changed_step_percent_applies_live_and_debounces_a_save() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(30)),
        base,
    )
    .unwrap();

    assert_eq!(c.config.brightness.step_percent, 30, "applies immediately");
    assert!(c.dirty.step_percent);
    assert_eq!(c.pending_save_since, Some(base));

    c.check_pending_save(base + Duration::from_millis(499));
    assert!(c.store.saves.is_empty(), "debounce window not elapsed yet");

    c.check_pending_save(base + Duration::from_millis(500));
    assert_eq!(c.store.saves.len(), 1, "exactly one save once debounced");
    let (saved_config, saved_dirty, force) = &c.store.saves[0];
    assert_eq!(saved_config.brightness.step_percent, 30);
    assert!(saved_dirty.step_percent);
    assert!(!force, "debounced save is never forced");
    assert_eq!(
        c.pending_save_since, None,
        "cleared after a successful save"
    );
    assert_eq!(
        c.dirty,
        SettingsDirty::default(),
        "dirty reset after a successful save"
    );
}

#[test]
fn setting_changed_osd_fields_apply_a_live_preview() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::OsdTimeoutMs(2500)),
        base,
    )
    .unwrap();
    assert_eq!(c.config.osd.timeout_ms, 2500);
    assert!(c.dirty.osd_timeout_ms);
    assert_eq!(c.pending_save_since, Some(base), "schedules a save");
    assert_eq!(
        c.osd.appearance_calls.len(),
        1,
        "exactly one live-preview call, not a spurious duplicate"
    );
    assert_eq!(
        c.osd.appearance_calls.last(),
        Some(&(c.config.osd.opacity, 2500))
    );

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::OsdOpacityPercent(34)),
        base,
    )
    .unwrap();
    assert!((c.config.osd.opacity - 0.34).abs() < f32::EPSILON);
    assert!(c.dirty.osd_opacity);
    assert_eq!(c.pending_save_since, Some(base), "schedules a save");
    assert_eq!(
        c.osd.appearance_calls.len(),
        2,
        "exactly one more live-preview call"
    );
    assert_eq!(
        c.osd.appearance_calls.last(),
        Some(&(c.config.osd.opacity, 2500)),
        "the live preview uses the config's current timeout too"
    );
}

#[test]
fn setting_changed_refresh_fields_have_no_side_effect() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::RefreshPeriodicSeconds(120)),
        base,
    )
    .unwrap();
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::RefreshInactivitySeconds(45)),
        base,
    )
    .unwrap();

    assert_eq!(c.config.refresh.periodic_seconds, 120);
    assert_eq!(c.config.refresh.inactivity_seconds, 45);
    assert!(c.dirty.refresh_periodic);
    assert!(c.dirty.refresh_inactivity);
    assert_eq!(c.pending_save_since, Some(base));
    assert_eq!(sent_refresh_count(&c), 0, "the timers read config live");
}

#[test]
fn setting_changed_logging_fields_are_save_only() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::FileLogEnabled(true)),
        base,
    )
    .unwrap();
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::FileLogLevel("debug".to_string())),
        base,
    )
    .unwrap();

    assert!(c.config.logging.file_enabled);
    assert_eq!(c.config.logging.file_level, "debug");
    assert!(c.dirty.log_enabled);
    assert!(c.dirty.log_level);
    assert_eq!(c.pending_save_since, Some(base), "schedules a save");
    assert!(
        c.osd.appearance_calls.is_empty(),
        "logging changes are restart-only; no live effect"
    );
    assert!(c.settings.refreshed.is_empty());
}

#[test]
fn setting_changed_coalesces_before_the_debounce_fires() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(10)),
        base,
    )
    .unwrap();
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(20)),
        base + Duration::from_millis(300),
    )
    .unwrap();

    c.check_pending_save(base + Duration::from_millis(700));
    assert!(
        c.store.saves.is_empty(),
        "second change re-stamped the debounce window"
    );

    c.check_pending_save(base + Duration::from_millis(800));
    assert_eq!(c.store.saves.len(), 1, "still exactly one save");
    assert_eq!(c.store.saves[0].0.brightness.step_percent, 20);
}

#[test]
fn restore_defaults_resets_all_fields_and_schedules_a_save() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.config.brightness.step_percent = 30;
    c.config.osd.timeout_ms = 5000;
    c.config.osd.opacity = 0.5;
    c.config.refresh.periodic_seconds = 10;
    c.config.refresh.inactivity_seconds = 10;
    c.config.logging.file_enabled = true;
    c.config.logging.file_level = "trace".to_string();

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::RestoreDefaults),
        base,
    )
    .unwrap();

    assert_eq!(c.config.brightness.step_percent, DEFAULT_STEP_PERCENT);
    assert_eq!(c.config.osd.timeout_ms, DEFAULT_OSD_TIMEOUT_MS);
    assert!((c.config.osd.opacity - DEFAULT_OSD_OPACITY).abs() < f32::EPSILON);
    assert_eq!(
        c.config.refresh.periodic_seconds,
        DEFAULT_REFRESH_PERIODIC_SECONDS
    );
    assert_eq!(
        c.config.refresh.inactivity_seconds,
        DEFAULT_REFRESH_INACTIVITY_SECONDS
    );
    assert!(!c.config.logging.file_enabled);
    assert_eq!(c.config.logging.file_level, DEFAULT_FILE_LOG_LEVEL);

    assert_eq!(
        c.dirty,
        SettingsDirty {
            step_percent: true,
            osd_timeout_ms: true,
            osd_opacity: true,
            refresh_periodic: true,
            refresh_inactivity: true,
            hotkey_up: true,
            hotkey_down: true,
            intercept: true,
            log_enabled: true,
            log_level: true,
            language: true,
        },
        "every field is marked dirty"
    );
    assert_eq!(c.pending_save_since, Some(base));
    assert_eq!(
        c.settings.refreshed.len(),
        1,
        "the dialog is told to redisplay every value"
    );
}

#[test]
fn restore_defaults_rebinds_hotkeys_when_bindings_changed() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.config.hotkeys.brightness_up = "Ctrl+Alt+Up".to_string();
    c.config.hotkeys.brightness_down = "Ctrl+Alt+Down".to_string();
    c.config.hotkeys.intercept_brightness_keys = true;

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::RestoreDefaults),
        base,
    )
    .unwrap();

    assert_eq!(
        c.hotkey_port.rebinds.last(),
        Some(&(
            DEFAULT_HOTKEY_UP.to_string(),
            DEFAULT_HOTKEY_DOWN.to_string(),
            false
        )),
        "changed hotkeys must be rebound in place, not just saved"
    );
}

#[test]
fn restore_defaults_that_changes_a_binding_clears_capture_active() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.config.hotkeys.brightness_up = "Ctrl+Alt+Up".to_string();
    c.handle_message(BrightnessMessage::HotkeyCaptureStarted, base)
        .unwrap();
    assert!(c.capture_active);

    let t2 = base + Duration::from_millis(100);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::RestoreDefaults),
        t2,
    )
    .unwrap();

    assert!(
        !c.capture_active,
        "the rebind Restore Defaults just posted doubles as the resume"
    );

    // A respawn right after must not post a spurious suspend under a
    // thread that was already un-suspended by the rebind above.
    let t3 = t2 + Duration::from_millis(50);
    c.hotkey_thread_respawned(t3);
    assert_eq!(
        c.hotkey_port.suspends, 1,
        "only the original capture-start suspend; none from the stale flag"
    );
}

#[test]
fn restore_defaults_does_not_rebind_hotkeys_when_unchanged() {
    let base = Instant::now();
    let mut c = test_controller(base);
    // Hotkeys are already at their defaults; only an unrelated field changes.
    c.config.brightness.step_percent = 30;

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::RestoreDefaults),
        base,
    )
    .unwrap();

    assert!(
        c.hotkey_port.rebinds.is_empty(),
        "unchanged hotkeys must not trigger a live rebind"
    );
    assert!(c.prev_hotkeys.is_none());
}

#[test]
fn restore_defaults_rebind_failure_reverts_only_hotkey_fields() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.config.hotkeys.brightness_up = "Ctrl+Alt+Up".to_string();
    c.config.hotkeys.brightness_down = "Ctrl+Alt+Down".to_string();
    c.config.hotkeys.intercept_brightness_keys = true;
    c.config.brightness.step_percent = 30;
    c.hotkey_port.fail_next = true;

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::RestoreDefaults),
        base,
    )
    .unwrap();

    assert_eq!(
        c.config.hotkeys.brightness_up, "Ctrl+Alt+Up",
        "hotkeys revert to what was live when the post fails"
    );
    assert_eq!(c.config.hotkeys.brightness_down, "Ctrl+Alt+Down");
    assert!(c.config.hotkeys.intercept_brightness_keys);
    assert_eq!(
        c.config.brightness.step_percent, DEFAULT_STEP_PERCENT,
        "the unrelated reset still applies"
    );
    // Restore-defaults marks every field dirty up front; a sync post
    // failure must not clear the hotkey ones back off — the reverted
    // config is exactly what belongs on disk, so leaving them dirty is
    // correct (and harmless: it just re-saves the same live values).
    assert!(c.dirty.hotkey_up);
    assert!(c.dirty.hotkey_down);
    assert!(c.dirty.intercept);
    assert!(
        c.dirty.step_percent,
        "the unrelated reset stays dirty and still saves"
    );
    assert!(c.hotkeys_degraded);
    assert_eq!(
        c.settings.errors,
        vec!["Could not reach the hotkey thread".to_string()]
    );
}

// ── Hotkey rebind flow ──────────────────────────────────────────────

#[test]
fn setting_changed_hotkey_up_stashes_prev_and_posts_a_rebind() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        base,
    )
    .unwrap();

    assert_eq!(c.config.hotkeys.brightness_up, "Alt+Up");
    assert!(c.dirty.hotkey_up);
    assert_eq!(c.pending_save_since, Some(base));
    assert_eq!(c.pending_hotkey_op, Some((HotkeyOp::Rebind, base)));
    assert_eq!(
        c.hotkey_port.rebinds,
        vec![("Alt+Up".to_string(), DEFAULT_HOTKEY_DOWN.to_string(), false)]
    );
    assert_eq!(
        c.prev_hotkeys,
        Some((
            DEFAULT_HOTKEY_UP.to_string(),
            DEFAULT_HOTKEY_DOWN.to_string(),
            false
        ))
    );
}

#[test]
fn setting_changed_hotkey_reverts_synchronously_when_the_post_itself_fails() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.hotkey_port.fail_next = true;

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        base,
    )
    .unwrap();

    assert_eq!(
        c.config.hotkeys.brightness_up, DEFAULT_HOTKEY_UP,
        "reverted because the post never reached the hotkey thread"
    );
    // The dirty flag the dialog change set stays set: the reverted value
    // is exactly what belongs on disk, so it must still reach the store.
    assert!(c.dirty.hotkey_up);
    assert!(!c.dirty.hotkey_down, "never touched by this change");
    assert!(!c.dirty.intercept, "never touched by this change");
    assert!(c.hotkeys_degraded);
    assert!(c.prev_hotkeys.is_none());
    assert!(c.pending_hotkey_op.is_none());
    assert_eq!(
        c.settings.errors,
        vec!["Could not reach the hotkey thread".to_string()]
    );
    assert_eq!(c.settings.refreshed.len(), 1);
}

#[test]
fn sync_post_failure_does_not_drop_an_earlier_still_dirty_change() {
    let base = Instant::now();
    let mut c = test_controller(base);

    // Change A posts fine and later acks success, but its own save
    // debounce has not fired yet.
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        base,
    )
    .unwrap();
    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: true,
            fallback_active: false,
            error: None,
            restore_error: None,
        },
        base + Duration::from_millis(50),
    )
    .unwrap();
    assert!(
        c.dirty.hotkey_up,
        "A is still unsaved, waiting on its debounce"
    );

    // Before A's debounce fires, change B's post fails outright (the
    // hotkey thread died in between).
    let t_b = base + Duration::from_millis(400);
    c.hotkey_port.fail_next = true;
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::InterceptBrightnessKeys(true)),
        t_b,
    )
    .unwrap();

    assert_eq!(
        c.config.hotkeys.brightness_up, "Alt+Up",
        "A's binding survives B's revert"
    );
    assert!(!c.config.hotkeys.intercept_brightness_keys, "B reverted");
    assert!(
        c.dirty.hotkey_up,
        "A must still be scheduled to save, not silently dropped"
    );

    c.check_pending_save(t_b + SAVE_DEBOUNCE);
    assert_eq!(c.store.saves.len(), 1);
    assert_eq!(c.store.saves[0].0.hotkeys.brightness_up, "Alt+Up");
}

#[test]
fn hotkey_rebind_result_with_no_pending_op_is_ignored() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: true,
            fallback_active: false,
            error: None,
            restore_error: None,
        },
        base,
    )
    .unwrap();

    assert!(!c.hotkeys_degraded);
    assert!(c.settings.refreshed.is_empty());
    assert!(c.settings.errors.is_empty());
    assert!(c.settings.notices.is_empty());
}

#[test]
fn hotkey_rebind_result_for_a_different_op_than_pending_is_ignored() {
    // A Suspend is in flight; an ack claiming to be for a Rebind must not
    // be consumed as if it resolved the Suspend — the protocol shouldn't
    // depend on the hotkey thread never mislabeling an ack.
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(BrightnessMessage::HotkeyCaptureStarted, base)
        .unwrap();
    assert_eq!(c.pending_hotkey_op, Some((HotkeyOp::Suspend, base)));

    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: true,
            fallback_active: false,
            error: None,
            restore_error: None,
        },
        base,
    )
    .unwrap();

    assert_eq!(
        c.pending_hotkey_op,
        Some((HotkeyOp::Suspend, base)),
        "the mismatched ack must not clear the actually-pending op"
    );
    assert!(!c.hotkeys_degraded);
    assert!(c.settings.refreshed.is_empty());
    assert!(c.settings.errors.is_empty());
    assert!(c.settings.notices.is_empty());
}

#[test]
fn a_late_ack_after_the_watchdog_already_reverted_is_ignored() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        base,
    )
    .unwrap();

    // The ack deadline passes with no response; the watchdog reverts and
    // reports it.
    let past_deadline = base + REBIND_TIMEOUT + Duration::from_millis(1);
    c.supervise_and_watchdog(past_deadline);
    assert!(c.hotkeys_degraded);
    let config_up_after_timeout = c.config.hotkeys.brightness_up.clone();
    let errors_before = c.settings.errors.len();
    let refreshed_before = c.settings.refreshed.len();
    let saves_before = c.store.saves.len();

    // The thread was only slow, not dead, and answers success afterward.
    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: true,
            fallback_active: false,
            error: None,
            restore_error: None,
        },
        past_deadline + Duration::from_millis(100),
    )
    .unwrap();

    assert!(
        c.hotkeys_degraded,
        "a late success must not clear a warning the watchdog already raised"
    );
    assert_eq!(
        c.config.hotkeys.brightness_up, config_up_after_timeout,
        "a late ack changes no state"
    );
    assert_eq!(c.settings.errors.len(), errors_before);
    assert_eq!(c.settings.refreshed.len(), refreshed_before);
    assert_eq!(c.store.saves.len(), saves_before);
    assert!(c.pending_hotkey_op.is_none());
}

#[test]
fn a_late_failure_ack_after_the_watchdog_already_reverted_is_ignored() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        base,
    )
    .unwrap();

    let past_deadline = base + REBIND_TIMEOUT + Duration::from_millis(1);
    c.supervise_and_watchdog(past_deadline);
    let errors_before = c.settings.errors.clone();
    let refreshed_before = c.settings.refreshed.len();
    let saves_before = c.store.saves.len();

    // A NAK for the same, already-resolved operation arrives even later.
    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: false,
            fallback_active: false,
            error: Some("device busy".to_string()),
            restore_error: None,
        },
        past_deadline + Duration::from_millis(200),
    )
    .unwrap();

    assert_eq!(
        c.settings.errors, errors_before,
        "no second, different error string for the one operation"
    );
    assert_eq!(c.settings.refreshed.len(), refreshed_before);
    assert_eq!(c.store.saves.len(), saves_before);
}

#[test]
fn a_resume_ack_does_not_clear_an_in_flight_rebinds_revert_stash() {
    let base = Instant::now();
    let mut c = test_controller(base);

    // A rebind's revert stash is parked while a later-in-flight op
    // (e.g. a capture-suspend's resume) also has an ack outstanding.
    c.prev_hotkeys = Some((
        DEFAULT_HOTKEY_UP.to_string(),
        DEFAULT_HOTKEY_DOWN.to_string(),
        false,
    ));
    c.pending_hotkey_op = Some((HotkeyOp::Resume, base));

    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Resume,
            success: true,
            fallback_active: false,
            error: None,
            restore_error: None,
        },
        base + Duration::from_millis(10),
    )
    .unwrap();

    assert!(
        c.prev_hotkeys.is_some(),
        "a non-rebind ack must not discard a pending rebind's revert stash"
    );
    assert!(c.pending_hotkey_op.is_none());
}

#[test]
fn hotkey_rebind_result_success_clears_pending_and_prev_hotkeys() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        base,
    )
    .unwrap();
    assert!(c.pending_hotkey_op.is_some());
    assert!(c.prev_hotkeys.is_some());

    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: true,
            fallback_active: false,
            error: None,
            restore_error: None,
        },
        base + Duration::from_millis(50),
    )
    .unwrap();

    assert!(c.pending_hotkey_op.is_none());
    assert!(c.prev_hotkeys.is_none());
    assert!(!c.hotkeys_degraded);
    assert!(c.settings.notices.is_empty());
}

#[test]
fn hotkey_rebind_result_fallback_active_shows_a_notice() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::InterceptBrightnessKeys(true)),
        base,
    )
    .unwrap();

    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: true,
            fallback_active: true,
            error: None,
            restore_error: None,
        },
        base + Duration::from_millis(50),
    )
    .unwrap();

    assert!(c.pending_hotkey_op.is_none());
    assert!(c.prev_hotkeys.is_none());
    assert!(!c.hotkeys_degraded, "the rebind still succeeded");
    assert_eq!(
        c.settings.notices,
        vec!["Key interception unavailable; hotkeys still work".to_string()]
    );
}

#[test]
fn hotkey_rebind_result_failure_reverts_and_reschedules_the_save() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        base,
    )
    .unwrap();

    // The debounced save already fired with the (wrong) optimistic value
    // before the NAK arrives.
    c.check_pending_save(base + SAVE_DEBOUNCE);
    assert_eq!(c.store.saves.len(), 1);
    assert_eq!(c.dirty, SettingsDirty::default());
    assert_eq!(c.pending_save_since, None);

    let nak_time = base + SAVE_DEBOUNCE + Duration::from_millis(100);
    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: false,
            fallback_active: false,
            error: Some("device busy".to_string()),
            restore_error: None,
        },
        nak_time,
    )
    .unwrap();

    assert_eq!(c.config.hotkeys.brightness_up, DEFAULT_HOTKEY_UP);
    assert!(c.dirty.hotkey_up, "the revert must be re-saved");
    assert!(c.hotkeys_degraded);
    assert_eq!(c.settings.errors, vec!["device busy".to_string()]);
    assert_eq!(c.settings.refreshed.len(), 1);
    assert_eq!(c.pending_save_since, Some(nak_time));
    assert!(c.prev_hotkeys.is_none());
    assert!(c.pending_hotkey_op.is_none());

    // The reverted value actually reaches disk.
    c.check_pending_save(nak_time + SAVE_DEBOUNCE);
    assert_eq!(c.store.saves.len(), 2);
    assert_eq!(c.store.saves[1].0.hotkeys.brightness_up, DEFAULT_HOTKEY_UP);
}

#[test]
fn hotkey_rebind_failure_reverts_the_binding_the_tray_menu_reports() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        base,
    )
    .unwrap();

    // Optimistic: the menu shows the new binding while the ack is pending.
    let (reply_tx, reply_rx) = mpsc::channel();
    c.handle_message(BrightnessMessage::TrayMenuOpening { reply_tx }, base)
        .unwrap();
    assert_eq!(reply_rx.try_recv().unwrap().hotkey_up, "Alt+Up");

    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: false,
            fallback_active: false,
            error: Some("device busy".to_string()),
            restore_error: None,
        },
        base,
    )
    .unwrap();

    // The usage rows are fetched live on every open, so the revert must
    // be what the next open reports — not the binding that never took.
    let (reply_tx, reply_rx) = mpsc::channel();
    c.handle_message(BrightnessMessage::TrayMenuOpening { reply_tx }, base)
        .unwrap();
    let data = reply_rx.try_recv().expect("menu data sent");
    assert_eq!(data.hotkey_up, DEFAULT_HOTKEY_UP);
    assert_eq!(data.hotkey_down, DEFAULT_HOTKEY_DOWN);
}

#[test]
fn pending_hotkey_op_ack_timeout_reverts_like_a_failure() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        base,
    )
    .unwrap();

    let past_deadline = base + REBIND_TIMEOUT + Duration::from_millis(1);
    c.supervise_and_watchdog(past_deadline);

    assert_eq!(c.config.hotkeys.brightness_up, DEFAULT_HOTKEY_UP);
    assert!(c.hotkeys_degraded);
    assert!(c.pending_hotkey_op.is_none());
    assert_eq!(
        c.settings.errors,
        vec!["Hotkey thread did not respond".to_string()]
    );
}

#[test]
fn pending_hotkey_op_within_the_deadline_is_left_alone() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        base,
    )
    .unwrap();

    let just_under_deadline = REBIND_TIMEOUT
        .checked_sub(Duration::from_millis(1))
        .unwrap();
    c.supervise_and_watchdog(base + just_under_deadline);

    assert_eq!(c.config.hotkeys.brightness_up, "Alt+Up");
    assert!(!c.hotkeys_degraded);
    assert!(c.pending_hotkey_op.is_some());
    assert!(c.settings.errors.is_empty());
}

#[test]
fn hotkeys_degraded_clears_on_a_later_successful_rebind() {
    let base = Instant::now();
    let mut c = test_controller(base);

    // A first rebind whose post fails outright leaves the warning latched.
    c.hotkey_port.fail_next = true;
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        base,
    )
    .unwrap();
    assert!(c.hotkeys_degraded);

    // A later rebind posts fine and acks success.
    let t2 = base + Duration::from_secs(1);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Down".to_string())),
        t2,
    )
    .unwrap();
    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: true,
            fallback_active: false,
            error: None,
            restore_error: None,
        },
        t2 + Duration::from_millis(50),
    )
    .unwrap();

    assert!(
        !c.health_warnings().hotkeys_degraded,
        "a later successful ack clears the degraded warning"
    );
}

#[test]
fn hotkey_config_returns_current_bindings() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.config.hotkeys.brightness_up = "Alt+Up".to_string();
    c.config.hotkeys.brightness_down = "Alt+Down".to_string();
    c.config.hotkeys.intercept_brightness_keys = true;

    assert_eq!(
        c.hotkey_config(),
        ("Alt+Up".to_string(), "Alt+Down".to_string(), true)
    );
}

#[test]
fn settings_snapshot_maps_opacity_float_to_a_rounded_percent() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.config.osd.opacity = 0.335;

    assert_eq!(c.settings_snapshot().osd_opacity_percent, 34);
}

#[test]
fn tray_open_settings_opens_the_dialog_with_current_values() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.config.brightness.step_percent = 12;

    c.handle_message(BrightnessMessage::TrayOpenSettings, base)
        .unwrap();

    assert!(c.settings_open);
    assert_eq!(c.settings.opened.len(), 1);
    assert_eq!(c.settings.opened[0].step_percent, 12);
}

#[test]
fn settings_closed_clears_the_open_flag_after_an_open() {
    // `open()` cannot report failure, so a platform impl that fails to
    // show a window clears this flag by sending `SettingsClosed` itself.
    // Without that the flag latches and `assert_topmost` is called for a
    // window that does not exist. This pins the half the controller owns.
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(BrightnessMessage::TrayOpenSettings, base)
        .unwrap();
    assert!(c.settings_open);

    c.handle_message(BrightnessMessage::SettingsClosed, base)
        .unwrap();

    assert!(
        !c.settings_open,
        "a window that never appeared must not leave the flag latched"
    );
}

#[test]
fn settings_closed_forces_a_save_when_dirty() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(30)),
        base,
    )
    .unwrap();
    c.handle_message(
        BrightnessMessage::SettingsClosed,
        base + Duration::from_millis(50),
    )
    .unwrap();

    assert!(!c.settings_open);
    assert_eq!(
        c.store.saves.len(),
        1,
        "close flushes before the debounce fires"
    );
    assert!(c.store.saves[0].2, "close forces the save");
    assert_eq!(c.pending_save_since, None);
    assert_eq!(c.dirty, SettingsDirty::default());
}

#[test]
fn settings_closed_without_changes_saves_nothing() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(BrightnessMessage::TrayOpenSettings, base)
        .unwrap();
    c.handle_message(BrightnessMessage::SettingsClosed, base)
        .unwrap();

    assert!(
        c.store.saves.is_empty(),
        "nothing changed; the file must not be touched"
    );
}

#[test]
fn quit_flushes_a_pending_save_when_dirty() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(30)),
        base,
    )
    .unwrap();
    assert!(
        !c.handle_message(BrightnessMessage::TrayRequestQuit, base)
            .unwrap()
    );

    assert_eq!(c.store.saves.len(), 1);
    assert!(c.store.saves[0].2, "quit forces the save");
}

#[test]
fn quit_saves_nothing_when_not_dirty() {
    let base = Instant::now();
    let mut c = test_controller(base);

    assert!(
        !c.handle_message(BrightnessMessage::TrayRequestQuit, base)
            .unwrap()
    );

    assert!(c.store.saves.is_empty());
}

#[test]
fn shutdown_also_flushes_a_pending_save_when_dirty() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(30)),
        base,
    )
    .unwrap();
    assert!(!c.handle_message(BrightnessMessage::Shutdown, base).unwrap());

    assert_eq!(
        c.store.saves.len(),
        1,
        "Ctrl+C shutdown is as much an app quit as the tray Quit item"
    );
    assert!(c.store.saves[0].2, "shutdown forces the save");
}

#[test]
fn deferred_save_keeps_dirty_and_rearms_the_debounce() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.store.result = Some(SaveResult::Deferred("disk file changed".to_string()));

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(30)),
        base,
    )
    .unwrap();
    c.check_pending_save(base + SAVE_DEBOUNCE);

    assert_eq!(c.store.saves.len(), 1);
    assert!(
        c.dirty.step_percent,
        "stays dirty so the retry has something to save"
    );
    assert_eq!(
        c.pending_save_since,
        Some(base + SAVE_DEBOUNCE),
        "re-armed from the tick that just ran, not the original change"
    );

    // A retry after another full debounce window saves again.
    c.store.result = None;
    c.check_pending_save(base + SAVE_DEBOUNCE + SAVE_DEBOUNCE);
    assert_eq!(c.store.saves.len(), 2);
    assert_eq!(c.pending_save_since, None);
}

#[test]
fn a_deferred_retry_stays_unforced_and_can_defer_again() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.store.result = Some(SaveResult::Deferred("disk file changed".to_string()));

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(30)),
        base,
    )
    .unwrap();
    c.check_pending_save(base + SAVE_DEBOUNCE);
    let retry = base + SAVE_DEBOUNCE * 2;
    c.check_pending_save(retry);

    // A deferral means "the on-disk file is in a state we must not
    // clobber"; the debounced retry has to keep asking, not escalate to
    // a forced overwrite — that is reserved for close/quit.
    assert_eq!(c.store.saves.len(), 2);
    assert!(!c.store.saves[1].2, "retry must not force");
    assert!(
        c.dirty.step_percent,
        "still dirty after the second deferral"
    );
    assert_eq!(c.pending_save_since, Some(retry), "re-armed again");
}

#[test]
fn failed_save_keeps_dirty_and_rearms_the_debounce() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.store.result = Some(SaveResult::Failed("disk full".to_string()));

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(30)),
        base,
    )
    .unwrap();
    c.check_pending_save(base + SAVE_DEBOUNCE);

    assert_eq!(c.store.saves.len(), 1);
    assert!(c.dirty.step_percent);
    assert_eq!(c.pending_save_since, Some(base + SAVE_DEBOUNCE));
}

#[test]
fn a_persistent_save_failure_stops_retrying_at_the_cap_but_stays_dirty() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.store.result = Some(SaveResult::Failed("disk full".to_string()));

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(30)),
        base,
    )
    .unwrap();

    // Attempts 1 and 2 (below SAVE_FAILURE_LIMIT) keep re-arming.
    c.check_pending_save(base + SAVE_DEBOUNCE);
    assert_eq!(c.store.saves.len(), 1);
    assert!(c.pending_save_since.is_some(), "attempt 1 re-arms");

    c.check_pending_save(base + SAVE_DEBOUNCE * 2);
    assert_eq!(c.store.saves.len(), 2);
    assert!(c.pending_save_since.is_some(), "attempt 2 re-arms");

    // Attempt 3 reaches SAVE_FAILURE_LIMIT and stops re-arming.
    c.check_pending_save(base + SAVE_DEBOUNCE * 3);
    assert_eq!(c.store.saves.len(), 3, "the cap is 3 attempts");
    assert_eq!(
        c.pending_save_since, None,
        "the automatic retry gives up once the cap is reached"
    );
    assert!(
        c.dirty.step_percent,
        "the change is not lost, just not retried automatically"
    );

    // No further tick attempts a save on its own.
    c.check_pending_save(base + SAVE_DEBOUNCE * 10);
    assert_eq!(
        c.store.saves.len(),
        3,
        "no unbounded retry loop once given up"
    );
}

#[test]
fn a_new_change_after_giving_up_retries_and_a_success_resets_the_streak() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.store.result = Some(SaveResult::Failed("disk full".to_string()));

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(30)),
        base,
    )
    .unwrap();
    c.check_pending_save(base + SAVE_DEBOUNCE);
    c.check_pending_save(base + SAVE_DEBOUNCE * 2);
    c.check_pending_save(base + SAVE_DEBOUNCE * 3);
    assert_eq!(c.pending_save_since, None, "given up after 3 attempts");
    assert_eq!(c.consecutive_save_failures, 3);

    // A later dialog edit re-arms the debounce even though the loop had
    // stopped retrying on its own.
    let later = base + SAVE_DEBOUNCE * 5;
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::StepPercent(40)),
        later,
    )
    .unwrap();
    assert_eq!(c.pending_save_since, Some(later));

    // The underlying problem clears; the next attempt succeeds and
    // resets everything, including the failure streak.
    c.store.result = None;
    c.check_pending_save(later + SAVE_DEBOUNCE);

    assert_eq!(c.store.saves.len(), 4);
    assert_eq!(c.pending_save_since, None);
    assert_eq!(c.dirty, SettingsDirty::default());
    assert_eq!(c.consecutive_save_failures, 0);
}

// ── Capture suspension & sticky topmost ──────────────────────────────

#[test]
fn hotkey_capture_started_suspends_interception() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.handle_message(BrightnessMessage::HotkeyCaptureStarted, base)
        .unwrap();

    assert!(c.capture_active);
    assert_eq!(c.hotkey_port.suspends, 1);
    assert_eq!(c.pending_hotkey_op, Some((HotkeyOp::Suspend, base)));
}

#[test]
fn hotkey_capture_started_post_failure_marks_degraded() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.hotkey_port.fail_next = true;

    c.handle_message(BrightnessMessage::HotkeyCaptureStarted, base)
        .unwrap();

    assert!(
        c.capture_active,
        "the capture field still has focus even though the suspend post failed"
    );
    assert!(c.hotkeys_degraded);
    assert!(c.pending_hotkey_op.is_none());
    assert_eq!(
        c.settings.errors,
        vec!["Could not reach the hotkey thread".to_string()]
    );
}

#[test]
fn hotkey_capture_ended_resumes_interception() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(BrightnessMessage::HotkeyCaptureStarted, base)
        .unwrap();

    let t2 = base + Duration::from_millis(500);
    c.handle_message(BrightnessMessage::HotkeyCaptureEnded, t2)
        .unwrap();

    assert!(!c.capture_active);
    assert_eq!(c.hotkey_port.resumes, 1);
    assert_eq!(c.pending_hotkey_op, Some((HotkeyOp::Resume, t2)));
}

#[test]
fn hotkey_up_during_capture_implicitly_ends_capture_and_the_rebind_serves_as_resume() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(BrightnessMessage::HotkeyCaptureStarted, base)
        .unwrap();

    let t2 = base + Duration::from_millis(500);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+Up".to_string())),
        t2,
    )
    .unwrap();

    assert!(!c.capture_active);
    assert_eq!(c.hotkey_port.suspends, 1);
    assert_eq!(c.hotkey_port.rebinds.len(), 1);
    assert_eq!(
        c.hotkey_port.resumes, 0,
        "the rebind itself doubles as the resume; no separate resume() call"
    );
}

#[test]
fn suspend_ack_success_clears_pending_op() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(BrightnessMessage::HotkeyCaptureStarted, base)
        .unwrap();

    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Suspend,
            success: true,
            fallback_active: false,
            error: None,
            restore_error: None,
        },
        base + Duration::from_millis(20),
    )
    .unwrap();

    assert!(c.pending_hotkey_op.is_none());
    assert!(!c.hotkeys_degraded);
}

#[test]
fn suspend_ack_failure_marks_degraded_without_reverting_config() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let up_before = c.config.hotkeys.brightness_up.clone();
    c.handle_message(BrightnessMessage::HotkeyCaptureStarted, base)
        .unwrap();
    // A rebind's revert stash is parked while the suspend's own ack is
    // still outstanding (the single-slot `pending_hotkey_op` limitation:
    // a later op can be posted before an earlier one's ack lands). If
    // the `op == HotkeyOp::Rebind` guard on the revert were ever
    // dropped, this stash would get reverted by the suspend's failure
    // below, which is exactly what this test exists to catch.
    c.prev_hotkeys = Some(("Alt+F1".to_string(), "Alt+F2".to_string(), false));

    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Suspend,
            success: false,
            fallback_active: false,
            error: Some("device busy".to_string()),
            restore_error: None,
        },
        base + Duration::from_millis(20),
    )
    .unwrap();

    assert_eq!(
        c.config.hotkeys.brightness_up, up_before,
        "a suspend has no config change to revert"
    );
    assert!(
        c.prev_hotkeys.is_some(),
        "a non-rebind failure must not consume or apply a parked rebind revert stash"
    );
    assert!(c.hotkeys_degraded);
    assert!(
        !c.hotkeys_lost,
        "a recoverable degraded warning must not touch the permanent give-up latch"
    );
    assert!(c.pending_hotkey_op.is_none());
    assert_eq!(c.settings.errors, vec!["device busy".to_string()]);
}

#[test]
fn resume_ack_timeout_marks_degraded_without_reverting_config() {
    let base = Instant::now();
    let mut c = test_controller(base);
    let up_before = c.config.hotkeys.brightness_up.clone();
    c.handle_message(BrightnessMessage::HotkeyCaptureStarted, base)
        .unwrap();
    c.handle_message(BrightnessMessage::HotkeyCaptureEnded, base)
        .unwrap();
    // Same rationale as the suspend-failure test above: a parked rebind
    // revert stash must survive a resume's ack timeout untouched.
    c.prev_hotkeys = Some(("Alt+F1".to_string(), "Alt+F2".to_string(), false));

    let past_deadline = base + REBIND_TIMEOUT + Duration::from_millis(1);
    c.supervise_and_watchdog(past_deadline);

    assert_eq!(c.config.hotkeys.brightness_up, up_before);
    assert!(
        c.prev_hotkeys.is_some(),
        "a non-rebind timeout must not consume or apply a parked rebind revert stash"
    );
    assert!(c.hotkeys_degraded);
    assert!(
        !c.hotkeys_lost,
        "a recoverable degraded warning must not touch the permanent give-up latch"
    );
    assert!(c.pending_hotkey_op.is_none());
    assert_eq!(
        c.settings.errors,
        vec!["Hotkey thread did not respond".to_string()]
    );
}

#[test]
fn hotkey_thread_respawned_resuspends_when_capture_is_active() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(BrightnessMessage::HotkeyCaptureStarted, base)
        .unwrap();
    assert_eq!(c.hotkey_port.suspends, 1);

    let t2 = base + Duration::from_secs(1);
    c.hotkey_thread_respawned(t2);

    assert_eq!(
        c.hotkey_port.suspends, 2,
        "the fresh thread registered everything and must be resuspended"
    );
    assert_eq!(c.pending_hotkey_op, Some((HotkeyOp::Suspend, t2)));
}

#[test]
fn hotkey_thread_respawned_is_a_no_op_without_active_capture() {
    let base = Instant::now();
    let mut c = test_controller(base);

    c.hotkey_thread_respawned(base);

    assert_eq!(c.hotkey_port.suspends, 0);
    assert!(c.pending_hotkey_op.is_none());
}

#[test]
fn settings_closed_while_capturing_ends_capture_and_resumes() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(BrightnessMessage::TrayOpenSettings, base)
        .unwrap();
    c.handle_message(BrightnessMessage::HotkeyCaptureStarted, base)
        .unwrap();

    let t2 = base + Duration::from_millis(300);
    c.handle_message(BrightnessMessage::SettingsClosed, t2)
        .unwrap();

    assert!(!c.capture_active);
    assert_eq!(c.hotkey_port.resumes, 1);
    assert!(!c.settings_open);
}

#[test]
fn overlay_update_reasserts_topmost_while_settings_is_open() {
    let base = Instant::now();
    let mut c = test_controller(base);
    seed(&mut c, test_id(), 0);
    c.handle_message(BrightnessMessage::TrayOpenSettings, base)
        .unwrap();

    // Hardware already at 0; dimming further only touches the overlay.
    c.handle_adjust(None, -10, base).unwrap();

    assert!(c.settings.topmost_asserts >= 1);
}

#[test]
fn overlay_update_does_not_touch_settings_when_it_is_closed() {
    let base = Instant::now();
    let mut c = test_controller(base);
    seed(&mut c, test_id(), 0);

    c.handle_adjust(None, -10, base).unwrap();

    assert_eq!(c.settings.topmost_asserts, 0);
}

// ── Language ─────────────────────────────────────────────────────────

#[test]
fn language_resolves_from_config_and_os_at_construction() {
    let base = Instant::now();
    let c = test_controller_with(Config::default(), german_os(), base);
    assert_eq!(c.lang(), Lang::German);

    let fixed = Config {
        language: "en".to_string(),
        ..Config::default()
    };
    let c = test_controller_with(fixed, german_os(), base);
    assert_eq!(c.lang(), Lang::English);

    let c = test_controller_with(Config::default(), Vec::new(), base);
    assert_eq!(c.lang(), Lang::English);
}

#[test]
fn snapshot_carries_the_setting_and_the_resolved_language() {
    let base = Instant::now();
    let c = test_controller_with(Config::default(), german_os(), base);
    let snap = c.settings_snapshot();
    assert_eq!(snap.language, LanguageSetting::System);
    assert_eq!(snap.lang, Lang::German);
}

#[test]
fn a_language_change_that_alters_the_result_pushes_once_and_dirties() {
    let base = Instant::now();
    let mut c = test_controller_with(Config::default(), Vec::new(), base);
    let change = SettingChange::Language(LanguageSetting::Fixed(Lang::German));
    c.handle_message(BrightnessMessage::SettingChanged(change), base)
        .unwrap();

    assert_eq!(c.lang(), Lang::German);
    assert_eq!(c.config.language, "de");
    assert!(c.dirty.language);
    assert_eq!(c.pending_save_since, Some(base));
    assert_eq!(c.osd.languages, vec![Lang::German]);
    assert_eq!(c.settings.languages, vec![Lang::German]);
}

#[test]
fn a_language_change_with_the_same_result_dirties_but_pushes_nothing() {
    // "System default" on a German OS to "Deutsch": the stored choice changes,
    // the displayed language does not, so nothing is relabelled.
    let base = Instant::now();
    let mut c = test_controller_with(Config::default(), german_os(), base);
    let change = SettingChange::Language(LanguageSetting::Fixed(Lang::German));
    c.handle_message(BrightnessMessage::SettingChanged(change), base)
        .unwrap();

    assert_eq!(c.config.language, "de");
    assert!(c.dirty.language);
    assert!(c.osd.languages.is_empty());
    assert!(c.settings.languages.is_empty());
}

#[test]
fn a_language_only_session_saves_on_close() {
    let base = Instant::now();
    let mut c = test_controller_with(Config::default(), Vec::new(), base);
    let change = SettingChange::Language(LanguageSetting::Fixed(Lang::German));
    c.handle_message(BrightnessMessage::SettingChanged(change), base)
        .unwrap();
    c.handle_message(BrightnessMessage::SettingsClosed, base)
        .unwrap();

    assert_eq!(c.store.saves.len(), 1);
    let (saved, dirty, force) = &c.store.saves[0];
    assert_eq!(saved.language, "de");
    assert!(dirty.language);
    assert!(force);
}

#[test]
fn restore_defaults_pushes_the_os_language_after_the_refresh() {
    let base = Instant::now();
    let fixed = Config {
        language: "en".to_string(),
        ..Config::default()
    };
    let mut c = test_controller_with(fixed, german_os(), base);
    assert_eq!(c.lang(), Lang::English);

    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::RestoreDefaults),
        base,
    )
    .unwrap();

    assert_eq!(c.lang(), Lang::German);
    assert_eq!(c.config.language, "system");
    assert!(c.dirty.language);
    assert_eq!(c.settings.refreshed.len(), 1);
    assert_eq!(
        c.settings.refreshed[0].lang,
        Lang::English,
        "refresh carries the pre-push snapshot"
    );
    assert_eq!(c.settings.languages, vec![Lang::German]);
    assert_eq!(c.osd.languages, vec![Lang::German]);
}

#[test]
fn restore_defaults_without_a_language_change_pushes_nothing() {
    let base = Instant::now();
    let mut c = test_controller_with(Config::default(), german_os(), base);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::RestoreDefaults),
        base,
    )
    .unwrap();
    assert!(c.settings.languages.is_empty());
    assert!(c.osd.languages.is_empty());
}

#[test]
fn hotkey_status_text_is_composed_in_the_controllers_language() {
    let base = Instant::now();
    let mut c = test_controller_with(Config::default(), german_os(), base);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+F1".to_string())),
        base,
    )
    .unwrap();
    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: false,
            fallback_active: false,
            error: Some("boom".to_string()),
            restore_error: Some("worse".to_string()),
        },
        base,
    )
    .unwrap();

    let expected = strings(Lang::German)
        .hotkey_status_restore_also_failed_fmt
        .replace("{error}", "boom")
        .replace("{restore_error}", "worse");
    assert_eq!(c.settings.errors, vec![expected]);
}

#[test]
fn a_failed_rebind_without_a_restore_error_shows_the_error_alone() {
    let base = Instant::now();
    let mut c = test_controller(base);
    c.handle_message(
        BrightnessMessage::SettingChanged(SettingChange::HotkeyUp("Alt+F1".to_string())),
        base,
    )
    .unwrap();
    c.handle_message(
        BrightnessMessage::HotkeyRebindResult {
            op: HotkeyOp::Rebind,
            success: false,
            fallback_active: false,
            error: Some("boom".to_string()),
            restore_error: None,
        },
        base,
    )
    .unwrap();
    assert_eq!(c.settings.errors, vec!["boom".to_string()]);
}
