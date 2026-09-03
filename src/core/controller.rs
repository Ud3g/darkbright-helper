//! Platform-agnostic controller orchestration.
//!
//! Composes the tested core primitives (`apply_set_result`, `RefreshTracker`,
//! respawn backoff) into the message-driven control flow, generic over four
//! narrow seams so the sequences are unit-testable with fakes on any host.
//! All methods take an explicit `now: Instant`; the binary captures it
//! immediately before each call.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::core::brightness::calculate_adjustment;
use crate::core::config::{Config, SettingsDirty};
use crate::core::reconcile::{
    HUNG_TIMEOUT_LIMIT, PRUNE_ABSENCE_WINDOW, REBIND_TIMEOUT, REFRESH_TIMEOUT, RefreshTracker,
    RespawnOutcome, SAVE_DEBOUNCE, SET_TIMEOUT,
};
use crate::core::state::{
    BrightnessMessage, DdcCommand, DdcHealth, HealthWarnings, HotkeyOp, MonitorId, MonitorState,
    SetOutcome, SettingChange, SettingsSnapshot, TrayMenuData, TrayMonitorInfo,
    UNREAD_BRIGHTNESS_SEED, generate_display_names,
};
use crate::error::{BrightnessError, Result};

/// Opaque per-monitor display handle.
///
/// Carries the platform's monitor handle value (`HMONITOR` on Windows) through
/// core without a platform type dependency; the platform seam implementations
/// convert at the boundary.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct MonitorHandle(pub isize);

/// Consecutive failed dialog-save attempts (`Deferred` or `Failed`, back to
/// back) before the automatic retry stops re-arming itself and leaves the
/// change dirty for the next dialog edit or close/quit flush to pick up.
/// Matches the DDC write path's own retry budget: initial attempt plus two
/// retries.
const SAVE_FAILURE_LIMIT: u32 = 3;

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

    /// Applies an appearance change from the settings dialog (opacity,
    /// auto-hide timeout) as an immediate live preview, independent of the
    /// per-adjustment `show`/`update`/`update_error` calls. Best-effort: a
    /// platform failure here is a preview glitch, not an adjustment failure,
    /// so it is logged rather than propagated.
    fn set_appearance(&mut self, opacity: f32, timeout_ms: u32);
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

/// Seam for the settings dialog window.
pub trait SettingsSink {
    /// Opens (or focuses) the settings window with current values.
    fn open(&mut self, snapshot: &SettingsSnapshot);

    /// Re-displays all values (restore defaults, rebind revert).
    fn refresh(&mut self, snapshot: &SettingsSnapshot);

    /// Inline red error for the hotkey row (registration failure).
    fn hotkey_error(&mut self, message: &str);

    /// Non-error notice (e.g. hook fallback active).
    fn hotkey_notice(&mut self, message: &str);

    /// Re-assert `HWND_TOPMOST` (the overlay re-asserts on every update).
    fn assert_topmost(&mut self);
}

/// Seam for the hotkey thread's in-place operations (rebind/suspend/resume).
pub trait HotkeyPort {
    /// Posts a rebind with new bindings and intercept setting.
    ///
    /// # Errors
    ///
    /// Errors mean the post itself failed (thread dead/queue gone); results
    /// otherwise arrive async as `HotkeyRebindResult`.
    fn rebind(&mut self, up: &str, down: &str, intercept: bool) -> Result<()>;

    /// Posts a suspend (stop delivering brightness hotkeys).
    ///
    /// # Errors
    ///
    /// Errors mean the post itself failed (thread dead/queue gone); results
    /// otherwise arrive async as `HotkeyRebindResult`.
    fn suspend(&mut self) -> Result<()>;

    /// Posts a resume (resume delivering brightness hotkeys).
    ///
    /// # Errors
    ///
    /// Errors mean the post itself failed (thread dead/queue gone); results
    /// otherwise arrive async as `HotkeyRebindResult`.
    fn resume(&mut self) -> Result<()>;
}

/// Outcome of a [`ConfigStore::save`] attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveResult {
    /// The config was written.
    Saved,
    /// On-disk file changed AND doesn't parse; save deferred (stay dirty).
    Deferred(String),
    /// The write failed.
    Failed(String),
}

/// Seam for persisting the runtime config to disk.
pub trait ConfigStore {
    /// Saves `config`, merging onto a concurrently-edited file per the
    /// dirty set. `force` (close/quit) never defers.
    fn save(&mut self, config: &Config, dirty: &SettingsDirty, force: bool) -> SaveResult;
}

/// What one non-`Saved` outcome means for logging, in terms of how far into
/// the current consecutive-failure streak it falls.
enum SaveFailureStage {
    /// The first failure of a new streak.
    Began,
    /// A later attempt, still under `SAVE_FAILURE_LIMIT` (carries the
    /// attempt number).
    Retrying(u32),
    /// The streak just reached `SAVE_FAILURE_LIMIT` (carries the total).
    GaveUp(u32),
}

/// Main controller for brightness management.
///
/// Owns all `MonitorState` and drives OSD/overlay/DDC through the seams.
/// Single-threaded: the binary's main loop is the only caller.
// The bools below are independent latches/flags (degraded-subsystem state,
// dialog session state), not a state machine with mutually exclusive modes —
// an enum would not fit them any better than it does `SettingsDirty`.
#[allow(clippy::struct_excessive_bools)]
pub struct Controller<Osd, Ovl, Ddc, Loc, Set, Hk, Store> {
    /// Current state (brightness, overlay, absence evidence) per monitor.
    states: HashMap<MonitorId, MonitorState>,
    /// Dimming overlay windows.
    overlay: Ovl,
    /// On-screen display.
    osd: Osd,
    /// Loaded configuration.
    config: Config,
    /// Cache mapping platform handles to monitor ids (avoids repeated EDID reads).
    ///
    /// The only handle→identity mapping in the app: resolving one costs a
    /// display-device enumeration plus a registry EDID read, far too slow for
    /// the hotkey path. Invalidated wholesale when a refresh begins (handles
    /// may be recycled across topology changes) and per-entry when a monitor
    /// is pruned.
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
    /// Condition of the DDC subsystem: healthy, or degraded with the cause
    /// that says what can end it.
    ddc_health: DdcHealth,
    /// True once hotkey supervision gave up; latched until app restart.
    hotkeys_lost: bool,
    /// True once the opt-in file log failed to attach; latched until restart.
    file_log_failed: bool,
    /// Monitor whose state the OSD is currently showing (for error restyling).
    osd_monitor: Option<MonitorId>,
    /// Settings dialog window.
    settings: Set,
    /// Hotkey thread's in-place rebind/suspend/resume port.
    hotkey_port: Hk,
    /// Config persistence.
    store: Store,
    /// Whether the settings window is currently open.
    settings_open: bool,
    /// Whether the hotkey capture field is currently capturing. While `true`,
    /// hotkey interception is suspended so the combination being captured
    /// (which may match a currently registered brightness hotkey) reaches
    /// the capture field as keystrokes instead of being intercepted.
    capture_active: bool,
    /// Settings fields changed since the last save.
    dirty: SettingsDirty,
    /// When the current debounce window for a pending save started.
    pending_save_since: Option<Instant>,
    /// Consecutive `Deferred`/`Failed` save outcomes, back to back; reset by
    /// the next `Saved` one. Caps the automatic retry loop at
    /// `SAVE_FAILURE_LIMIT`.
    consecutive_save_failures: u32,
    /// The in-place hotkey operation currently awaiting its ack, and when it
    /// was posted (for the ack deadline).
    pending_hotkey_op: Option<(HotkeyOp, Instant)>,
    /// True while a hotkey rebind/suspend/resume has failed or timed out;
    /// cleared by the next successful ack.
    hotkeys_degraded: bool,
    /// Hotkey bindings/intercept setting to revert to if a rebind fails.
    prev_hotkeys: Option<(String, String, bool)>,
}

impl<Osd, Ovl, Ddc, Loc, Set, Hk, Store> Controller<Osd, Ovl, Ddc, Loc, Set, Hk, Store>
where
    Osd: OsdSink,
    Ovl: OverlaySink,
    Ddc: DdcPort,
    Loc: MonitorLocator,
    Set: SettingsSink,
    Hk: HotkeyPort,
    Store: ConfigStore,
{
    /// Creates a controller; `now` stamps the activity/health/refresh baselines.
    #[allow(clippy::too_many_arguments)] // one independent seam per parameter
    #[must_use]
    pub fn new(
        config: Config,
        osd: Osd,
        overlay: Ovl,
        ddc: Ddc,
        locator: Loc,
        settings: Set,
        hotkey_port: Hk,
        store: Store,
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
            ddc_health: DdcHealth::Ok,
            hotkeys_lost: false,
            file_log_failed: false,
            osd_monitor: None,
            settings,
            hotkey_port,
            store,
            settings_open: false,
            capture_active: false,
            dirty: SettingsDirty::default(),
            pending_save_since: None,
            consecutive_save_failures: 0,
            pending_hotkey_op: None,
            hotkeys_degraded: false,
            prev_hotkeys: None,
        }
    }

    /// Asks the supervised DDC worker to shut down.
    pub fn shutdown_worker(&self) {
        self.ddc.shutdown();
    }

    /// Records that hotkey supervision gave up (latched until app restart).
    pub fn set_hotkeys_lost(&mut self) {
        self.hotkeys_lost = true;
    }

    /// Records that the opt-in file log could not be attached.
    ///
    /// Latched, because the attach is attempted exactly once at startup: there
    /// is no path by which the log can start appearing later, so nothing should
    /// clear this. The report has to travel through the tray because the
    /// failure of a diagnostic channel cannot be announced on that same
    /// channel, and a release build hides the console.
    pub fn set_file_log_failed(&mut self) {
        self.file_log_failed = true;
    }

    /// Returns the currently active degraded-subsystem warnings.
    #[must_use]
    pub fn health_warnings(&self) -> HealthWarnings {
        HealthWarnings {
            ddc: self.ddc_health,
            hotkeys_lost: self.hotkeys_lost,
            hotkeys_degraded: self.hotkeys_degraded,
            file_log_failed: self.file_log_failed,
        }
    }

    /// Requests a refresh of monitor list and brightness values.
    ///
    /// Sends a `RefreshAll` command to the DDC worker. The actual state
    /// update happens when `DdcRefreshResult` is received.
    pub fn handle_refresh(&mut self, now: Instant) {
        log::debug!("Requesting monitor refresh from DDC worker");

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

    /// Builds the current config values for the settings window.
    ///
    /// Maps `osd.opacity`'s `0.1-1.0` float range to the `10-100` percent
    /// range the dialog displays, rounding to the nearest percent and
    /// clamping into range (a value written outside it by hand-editing the
    /// config file must still display sanely).
    #[must_use]
    pub fn settings_snapshot(&self) -> SettingsSnapshot {
        let opacity_percent = (self.config.osd.opacity * 100.0).round().clamp(10.0, 100.0);
        // The clamp above bounds this to 10.0..=100.0, well within u8 range.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let osd_opacity_percent = opacity_percent as u8;

        SettingsSnapshot {
            step_percent: self.config.brightness.step_percent,
            osd_timeout_ms: self.config.osd.timeout_ms,
            osd_opacity_percent,
            refresh_periodic_seconds: self.config.refresh.periodic_seconds,
            refresh_inactivity_seconds: self.config.refresh.inactivity_seconds,
            hotkey_up: self.config.hotkeys.brightness_up.clone(),
            hotkey_down: self.config.hotkeys.brightness_down.clone(),
            intercept_brightness_keys: self.config.hotkeys.intercept_brightness_keys,
            file_log_enabled: self.config.logging.file_enabled,
            file_log_level: self.config.logging.file_level.clone(),
        }
    }

    /// Flushes a debounced settings save once its window has elapsed.
    ///
    /// Called once per main-loop tick, alongside `check_periodic_refresh`.
    /// A no-op when no save is pending.
    pub fn check_pending_save(&mut self, now: Instant) {
        let Some(since) = self.pending_save_since else {
            return;
        };
        if !self.dirty.any() {
            // Defensive: `pending_save_since` must never outlive `dirty`. If
            // a future `SettingChanged` arm ever sets one without the other,
            // this stops it from silently rewriting config.json on a run
            // that never touched the dialog, instead of masking the bug.
            self.pending_save_since = None;
            return;
        }
        if now.saturating_duration_since(since) < SAVE_DEBOUNCE {
            return;
        }
        self.flush_save(now, false);
    }

    /// Applies one dialog-originated setting change to the live config and
    /// (re-)arms the debounced save.
    ///
    /// Every change lands in `config` immediately; only the OSD fields also
    /// produce an immediate platform side effect (a live preview). The rest
    /// take effect on the next timer tick (refresh interval fields, read
    /// live) or on restart (logging, by design — its dialog hint says so).
    fn handle_setting_changed(&mut self, change: SettingChange, now: Instant) {
        match change {
            SettingChange::StepPercent(pct) => {
                self.config.brightness.step_percent = pct;
                self.dirty.step_percent = true;
            }
            SettingChange::OsdTimeoutMs(ms) => {
                self.config.osd.timeout_ms = ms;
                self.dirty.osd_timeout_ms = true;
                self.osd
                    .set_appearance(self.config.osd.opacity, self.config.osd.timeout_ms);
            }
            SettingChange::OsdOpacityPercent(pct) => {
                self.config.osd.opacity = f32::from(pct) / 100.0;
                self.dirty.osd_opacity = true;
                self.osd
                    .set_appearance(self.config.osd.opacity, self.config.osd.timeout_ms);
            }
            SettingChange::RefreshPeriodicSeconds(secs) => {
                self.config.refresh.periodic_seconds = secs;
                self.dirty.refresh_periodic = true;
            }
            SettingChange::RefreshInactivitySeconds(secs) => {
                self.config.refresh.inactivity_seconds = secs;
                self.dirty.refresh_inactivity = true;
            }
            SettingChange::FileLogEnabled(enabled) => {
                self.config.logging.file_enabled = enabled;
                self.dirty.log_enabled = true;
            }
            SettingChange::FileLogLevel(level) => {
                self.config.logging.file_level = level;
                self.dirty.log_level = true;
            }
            SettingChange::RestoreDefaults => {
                self.handle_restore_defaults(now);
                return;
            }
            // Editing a hotkey binding or the intercept flag round-trips
            // through the hotkey thread (rebind, wait for the ack, revert on
            // failure) instead of being marked dirty unconditionally here.
            SettingChange::HotkeyUp(up) => {
                self.dirty.hotkey_up = true;
                // A binding produced by the capture field implicitly ends
                // capture; the rebind posted by apply_hotkey_change doubles
                // as the resume (see post_hotkey_rebind), so no separate
                // resume() call goes out.
                let down = self.config.hotkeys.brightness_down.clone();
                let intercept = self.config.hotkeys.intercept_brightness_keys;
                self.apply_hotkey_change(up, down, intercept, now);
                return;
            }
            SettingChange::HotkeyDown(down) => {
                self.dirty.hotkey_down = true;
                let up = self.config.hotkeys.brightness_up.clone();
                let intercept = self.config.hotkeys.intercept_brightness_keys;
                self.apply_hotkey_change(up, down, intercept, now);
                return;
            }
            SettingChange::InterceptBrightnessKeys(intercept) => {
                self.dirty.intercept = true;
                let up = self.config.hotkeys.brightness_up.clone();
                let down = self.config.hotkeys.brightness_down.clone();
                self.apply_hotkey_change(up, down, intercept, now);
                return;
            }
        }
        self.pending_save_since = Some(now);
    }

    /// Applies one hotkey-binding or intercept-flag change from the dialog:
    /// stashes the live triple for a possible revert, writes the new values
    /// into the config, arms the debounced save, and posts the rebind. The
    /// caller has already marked the one dirty field that changed.
    fn apply_hotkey_change(&mut self, up: String, down: String, intercept: bool, now: Instant) {
        self.prev_hotkeys = Some((
            self.config.hotkeys.brightness_up.clone(),
            self.config.hotkeys.brightness_down.clone(),
            self.config.hotkeys.intercept_brightness_keys,
        ));
        self.config.hotkeys.brightness_up = up;
        self.config.hotkeys.brightness_down = down;
        self.config.hotkeys.intercept_brightness_keys = intercept;
        self.pending_save_since = Some(now);
        self.post_hotkey_rebind(now);
    }

    /// Posts the live hotkey thread's bindings/intercept flag to match the
    /// config just applied. Errors mean the post itself never reached the
    /// thread (queue gone / thread dead), so no ack is ever coming — the
    /// revert to `prev_hotkeys` happens synchronously right here rather than
    /// waiting on one.
    ///
    /// Also clears `capture_active`: every caller of this method is a rebind
    /// that re-registers the hotkey thread, and a re-registration always
    /// doubles as the resume half of the capture-suspend cycle, whether it
    /// came from the capture field itself (a `HotkeyUp`/`HotkeyDown` change)
    /// or a Restore Defaults that happened to change a binding while capture
    /// was active. Centralizing it here means no caller can forget it.
    fn post_hotkey_rebind(&mut self, now: Instant) {
        self.capture_active = false;
        self.pending_hotkey_op = Some((HotkeyOp::Rebind, now));
        let up = self.config.hotkeys.brightness_up.clone();
        let down = self.config.hotkeys.brightness_down.clone();
        let intercept = self.config.hotkeys.intercept_brightness_keys;

        if self.hotkey_port.rebind(&up, &down, intercept).is_err() {
            log::error!("Failed to post hotkey rebind; hotkey thread unreachable");
            self.pending_hotkey_op = None;
            if let Some((prev_up, prev_down, prev_intercept)) = self.prev_hotkeys.take() {
                self.config.hotkeys.brightness_up = prev_up;
                self.config.hotkeys.brightness_down = prev_down;
                self.config.hotkeys.intercept_brightness_keys = prev_intercept;
            }
            // The dirty flag(s) the caller just set are left alone rather
            // than cleared: the reverted config is exactly what belongs on
            // disk, so leaving them dirty is a no-op if nothing else was
            // pending and correct if an earlier, still-unsaved change is
            // sitting in the same fields (clearing them here would silently
            // drop that earlier change instead of saving it).
            self.hotkeys_degraded = true;
            self.settings
                .hotkey_error("Could not reach the hotkey thread");
            let snapshot = self.settings_snapshot();
            self.settings.refresh(&snapshot);
        }
    }

    /// Posts a suspend so the hotkey thread stops delivering brightness
    /// hotkeys while the capture field has focus. A failed post means the
    /// thread is unreachable, so no ack is ever coming: there is no config
    /// change to revert (unlike a rebind), so this only raises the degraded
    /// warning.
    fn post_hotkey_suspend(&mut self, now: Instant) {
        self.pending_hotkey_op = Some((HotkeyOp::Suspend, now));
        if self.hotkey_port.suspend().is_err() {
            log::error!("Failed to post hotkey suspend; hotkey thread unreachable");
            self.pending_hotkey_op = None;
            self.hotkeys_degraded = true;
            self.settings
                .hotkey_error("Could not reach the hotkey thread");
        }
    }

    /// Posts a resume so the hotkey thread goes back to delivering brightness
    /// hotkeys after the capture field loses focus. Same failure handling as
    /// [`Self::post_hotkey_suspend`]: nothing to revert, just the degraded
    /// warning.
    fn post_hotkey_resume(&mut self, now: Instant) {
        self.pending_hotkey_op = Some((HotkeyOp::Resume, now));
        if self.hotkey_port.resume().is_err() {
            log::error!("Failed to post hotkey resume; hotkey thread unreachable");
            self.pending_hotkey_op = None;
            self.hotkeys_degraded = true;
            self.settings
                .hotkey_error("Could not reach the hotkey thread");
        }
    }

    /// Handles an ack, or a deadline expiry treated exactly like one, for the
    /// hotkey thread's most recent posted operation.
    ///
    /// An ack that arrives with nothing pending, or whose `op` does not match
    /// the operation actually pending, is stale — the watchdog's ack deadline
    /// has already passed and either reverted the config (a failed rebind) or
    /// simply moved on (suspend/resume), so the thread's actual registration
    /// state is no longer knowable from here. Adopting a late success as
    /// ground truth would leave `config` on the old binding while the thread
    /// believes it registered the new one, silently diverged with no warning
    /// left to say so; a late failure would show a second, different error
    /// for an operation already resolved. Either way the honest move is to
    /// change nothing and let the dialog's own next rebind (which re-posts
    /// both bindings) resolve any divergence deterministically.
    ///
    /// The `op` check is defense in depth, not the primary guard against a
    /// mismatched ack — the hotkey-thread side is responsible for never
    /// posting one for an operation the main thread didn't ask for — but the
    /// protocol should not depend on the producer being perfect either.
    ///
    /// A successful rebind is otherwise a recovery signal: it clears
    /// `hotkeys_degraded` even if an earlier attempt had set it, because a
    /// working binding right now is what "recoverable" means for this
    /// warning. A hook-install fallback is reported as a notice, not an
    /// error — the rebind itself still succeeded. A failure reverts the
    /// config if the op was a rebind (the only op with a config change to
    /// undo) and re-arms the save so the revert reaches disk even when the
    /// optimistic value was already written there.
    fn handle_hotkey_rebind_result(
        &mut self,
        op: HotkeyOp,
        success: bool,
        fallback_active: bool,
        error: Option<String>,
        now: Instant,
    ) {
        match self.pending_hotkey_op {
            None => {
                log::debug!(op:? = op, success; "Ignoring hotkey ack with no operation pending (stale/late)");
                return;
            }
            Some((pending_op, _)) if pending_op != op => {
                log::debug!(
                    op:? = op,
                    pending_op:? = pending_op,
                    success;
                    "Ignoring hotkey ack for a different operation than the one pending (stale/late)"
                );
                return;
            }
            Some(_) => {}
        }
        self.pending_hotkey_op = None;

        if success {
            if op == HotkeyOp::Rebind {
                self.prev_hotkeys = None;
            }
            self.hotkeys_degraded = false;
            if fallback_active {
                self.settings.hotkey_notice(
                    "Brightness-key interception unavailable; using plain key registration",
                );
            }
            return;
        }

        self.fail_hotkey_op(
            op,
            &error.unwrap_or_else(|| "unknown error".to_string()),
            now,
        );
    }

    /// Reverts a failed rebind's config change (if one is pending), marks the
    /// hotkey subsystem degraded, and tells the dialog. Shared by the ack
    /// path and the ack-timeout watchdog.
    fn fail_hotkey_op(&mut self, op: HotkeyOp, message: &str, now: Instant) {
        if op == HotkeyOp::Rebind
            && let Some((up, down, intercept)) = self.prev_hotkeys.take()
        {
            self.config.hotkeys.brightness_up = up;
            self.config.hotkeys.brightness_down = down;
            self.config.hotkeys.intercept_brightness_keys = intercept;
            self.dirty.hotkey_up = true;
            self.dirty.hotkey_down = true;
            self.dirty.intercept = true;
            self.pending_save_since = Some(now);
        }
        self.hotkeys_degraded = true;
        self.settings.hotkey_error(message);
        let snapshot = self.settings_snapshot();
        self.settings.refresh(&snapshot);
    }

    /// Reacts to a hotkey thread respawn (dead worker thread replaced).
    ///
    /// A fresh thread starts with nothing registered, so if the capture
    /// field currently has focus, interception must be suspended on it too —
    /// otherwise the new thread would deliver brightness hotkeys straight
    /// into what should be a suspended capture session.
    pub fn hotkey_thread_respawned(&mut self, now: Instant) {
        if self.capture_active {
            self.post_hotkey_suspend(now);
        }
    }

    /// Returns the currently live hotkey bindings and intercept setting.
    ///
    /// Used by the respawn path so a freshly spawned hotkey thread
    /// re-registers what is actually configured right now, not the bindings
    /// the process started with.
    #[must_use]
    pub fn hotkey_config(&self) -> (String, String, bool) {
        (
            self.config.hotkeys.brightness_up.clone(),
            self.config.hotkeys.brightness_down.clone(),
            self.config.hotkeys.intercept_brightness_keys,
        )
    }

    /// Resets the ten settings-dialog fields to their defaults and schedules
    /// a save; also refreshes the open dialog so it shows the reset values.
    ///
    /// Rebinds the live hotkey thread too, but only when the reset actually
    /// changed a binding or the intercept flag — an unconditional rebind
    /// would post a no-op round-trip (and a real chance of the ack-timeout
    /// path firing) on every restore that never touched hotkeys at all.
    fn handle_restore_defaults(&mut self, now: Instant) {
        let up_before = self.config.hotkeys.brightness_up.clone();
        let down_before = self.config.hotkeys.brightness_down.clone();
        let intercept_before = self.config.hotkeys.intercept_brightness_keys;

        self.config.restore_defaults();

        self.dirty = SettingsDirty {
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
        };
        self.pending_save_since = Some(now);

        let snapshot = self.settings_snapshot();
        self.settings.refresh(&snapshot);

        let hotkeys_changed = up_before != self.config.hotkeys.brightness_up
            || down_before != self.config.hotkeys.brightness_down
            || intercept_before != self.config.hotkeys.intercept_brightness_keys;
        if hotkeys_changed {
            self.prev_hotkeys = Some((up_before, down_before, intercept_before));
            self.post_hotkey_rebind(now);
        }
    }

    /// Forces a save right now if the dialog session left unsaved changes.
    ///
    /// Called at the two points a debounce window would otherwise be
    /// silently abandoned: the dialog closing and the app quitting. A
    /// session with no changes must not touch the file at all.
    fn flush_pending_settings(&mut self, now: Instant) {
        if self.dirty.any() {
            self.flush_save(now, true);
        }
    }

    /// Runs one save attempt and reconciles `dirty`/`pending_save_since`
    /// against its outcome.
    ///
    /// Only `SaveResult::Saved` clears the dirty set: `Deferred` (an
    /// on-disk conflict) and `Failed` both keep it. Each also counts toward
    /// `SAVE_FAILURE_LIMIT`; below the cap the debounce re-arms from `now`
    /// so the next tick retries after another full window, but once the cap
    /// is reached the retry loop stops re-arming itself (the change stays
    /// dirty, so a later dialog edit or a close/quit flush still saves it).
    /// A persistent failure — read-only file, AV lock, full disk — must not
    /// turn into unbounded synchronous file I/O on this thread.
    fn flush_save(&mut self, now: Instant, force: bool) {
        match self.store.save(&self.config, &self.dirty, force) {
            SaveResult::Saved => {
                self.dirty = SettingsDirty::default();
                self.pending_save_since = None;
                self.consecutive_save_failures = 0;
            }
            SaveResult::Deferred(reason) => {
                match self.note_save_failure() {
                    SaveFailureStage::Began => {
                        log::error!(reason:% = reason; "Settings save deferred; will retry");
                    }
                    SaveFailureStage::Retrying(attempt) => {
                        log::debug!(reason:% = reason, attempt; "Settings save deferred again; retrying");
                    }
                    SaveFailureStage::GaveUp(attempts) => {
                        log::error!(
                            reason:% = reason, attempts;
                            "Settings save deferred repeatedly; giving up until the next change"
                        );
                    }
                }
                self.rearm_or_give_up(now);
            }
            SaveResult::Failed(reason) => {
                match self.note_save_failure() {
                    SaveFailureStage::Began => {
                        log::error!(reason:% = reason; "Settings save failed; will retry");
                    }
                    SaveFailureStage::Retrying(attempt) => {
                        log::debug!(reason:% = reason, attempt; "Settings save failed again; retrying");
                    }
                    SaveFailureStage::GaveUp(attempts) => {
                        log::error!(
                            reason:% = reason, attempts;
                            "Settings save failed repeatedly; giving up until the next change"
                        );
                    }
                }
                self.rearm_or_give_up(now);
            }
        }
    }

    /// Advances the consecutive-failure counter and reports which log level
    /// this attempt calls for: `error` once when a streak begins and once
    /// when it gives up at `SAVE_FAILURE_LIMIT`, `debug` for every attempt
    /// in between — so a persistent failure logs a handful of lines, not one
    /// every debounce window forever.
    fn note_save_failure(&mut self) -> SaveFailureStage {
        self.consecutive_save_failures += 1;
        match self.consecutive_save_failures {
            1 => SaveFailureStage::Began,
            n if n < SAVE_FAILURE_LIMIT => SaveFailureStage::Retrying(n),
            n => SaveFailureStage::GaveUp(n),
        }
    }

    /// Re-arms the debounce for another retry, unless the failure streak has
    /// just reached `SAVE_FAILURE_LIMIT` — then the automatic loop stops
    /// (leaving `dirty` set) rather than re-arming itself forever.
    fn rearm_or_give_up(&mut self, now: Instant) {
        self.pending_save_since = if self.consecutive_save_failures >= SAVE_FAILURE_LIMIT {
            None
        } else {
            Some(now)
        };
    }

    /// Handles the result of a DDC refresh operation.
    ///
    /// Read brightness values are authoritative ground truth for every monitor
    /// regardless of generation: a hardware value is true no matter which
    /// refresh produced it. A value above the hardware floor also clears an
    /// active sub-zero overlay (unless a set is in flight) — an externally
    /// raised brightness wins over the software veil. Absence bookkeeping
    /// (pruning) is gated on the result being current and the enumerated set
    /// being non-empty.
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
        self.note_worker_alive();

        let found_monitors = !monitors.is_empty();

        if found_monitors {
            // Routine heartbeat (fires every periodic refresh): debug, so it
            // does not drown the rolling file log. Topology *changes* — new
            // monitor below, prune in apply_absence_evidence — stay at info.
            log::debug!(count = monitors.len(); "DDC refresh complete");
        } else {
            log::warn!("DDC refresh completed with no monitors found");
        }

        for (monitor_id, brightness) in monitors {
            log::debug!(monitor_id:% = monitor_id, brightness = brightness; "Monitor found during refresh");

            if let Some(state) = self.states.get_mut(&monitor_id) {
                state.update_from_ddc(brightness);
                // A read above the hardware floor while the sub-zero overlay
                // is active means the brightness changed externally (physical
                // buttons, another tool, a monitor self-reset): the software
                // veil would silently fight that change, so it yields. An
                // in-flight optimistic set is newer intent than the read and
                // suppresses the reconcile.
                if brightness > 0 && state.overlay_opacity > 0 && state.pending.is_none() {
                    state.overlay_opacity = 0;
                    self.overlay.remove(&monitor_id);
                    log::info!(
                        monitor:% = monitor_id.base_display_name(),
                        brightness = brightness;
                        "Cleared dimming overlay after external brightness change"
                    );
                }
            } else {
                log::info!(monitor:% = monitor_id.base_display_name(); "New monitor detected");
                log::debug!(monitor_id:% = monitor_id.full_identity(); "New monitor identity");
                self.states
                    .insert(monitor_id, MonitorState::new(brightness));
            }
        }

        // A monitor that identified itself but refused the brightness read is
        // reported as enumerated only. It still gets state: the worker kept its
        // physical handle, so writes are attempted regardless, and a panel that
        // NAKs reads while honouring writes would otherwise be permanently and
        // silently uncontrollable. Insert-only — an existing state holds a real
        // last-known value that outranks any seed.
        for monitor_id in &enumerated {
            if !self.states.contains_key(monitor_id) {
                log::warn!(
                    monitor:% = monitor_id.base_display_name(),
                    seed = UNREAD_BRIGHTNESS_SEED;
                    "Monitor enumerated but brightness unreadable; seeding so it stays adjustable"
                );
                self.states
                    .insert(monitor_id.clone(), MonitorState::unread());
            }
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
            log::debug!(monitor_id:% = id.full_identity(); "Pruned monitor identity");
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

    /// Records that the worker executed a command and reported back.
    ///
    /// Called for every result the worker sends, whatever it says. A result is
    /// proof the thread is not blocked inside a DDC call, which is the only
    /// thing an unresponsive diagnosis ever claimed — so this is where that
    /// diagnosis is retracted, and the sole path out of it that needs neither
    /// a system resume nor an app restart. Note the failure case counts too:
    /// a worker that reports a NAK is answering, not hanging.
    fn note_worker_alive(&mut self) {
        self.consecutive_set_timeouts = 0;
        if self.ddc_health == DdcHealth::WorkerHung {
            log::info!("DDC worker answered again; clearing unresponsive state");
            self.ddc_health = DdcHealth::Ok;
        }
    }

    /// Clears the degraded DDC state so a fresh attempt can be made.
    fn clear_degraded(&mut self) {
        if self.ddc_health.is_degraded() {
            log::info!("Recovering from degraded DDC state");
        }
        self.ddc_health = DdcHealth::Ok;
        self.ddc.clear_backoff();
        self.consecutive_set_timeouts = 0;
    }

    /// Updates a monitor's overlay window, then re-asserts the settings
    /// window's topmost position if it is open.
    ///
    /// The overlay re-asserts `HWND_TOPMOST` on every update it makes, so a
    /// settings dialog that only asserted its own topmost position once, at
    /// open time, would be buried under it by the very next dim keypress —
    /// defeating the "my screen went dark, let me open Settings" scenario
    /// this exists for. Every overlay update in the controller must go
    /// through this wrapper rather than the seam directly.
    ///
    /// # Errors
    ///
    /// Returns an error if the platform overlay window cannot be created or
    /// updated.
    fn overlay_update(&mut self, id: &MonitorId, handle: MonitorHandle, opacity: u8) -> Result<()> {
        self.overlay.update(id, handle, opacity)?;
        if self.settings_open {
            self.settings.assert_topmost();
        }
        Ok(())
    }

    /// Applies a relative brightness adjustment.
    ///
    /// Determines the target monitor (mouse position), calculates new values,
    /// shows the OSD immediately with optimistic update, and sends the DDC
    /// command to the worker thread (non-blocking).
    ///
    /// # Errors
    ///
    /// Returns an error if the target monitor cannot be resolved or is
    /// unknown, or if an OSD/overlay update fails. An OSD failure after the
    /// optimistic update is logged and skipped instead of propagated, so the
    /// dispatched DDC set never dangles without a command in flight.
    fn handle_adjust(
        &mut self,
        monitor_id: Option<MonitorId>,
        delta: i8,
        now: Instant,
    ) -> Result<()> {
        // Check if we need an inactivity-based refresh before processing
        // (must be checked BEFORE updating last_activity)
        self.check_inactivity_refresh(now);

        // User activity is a recovery signal only for a worker we stopped
        // restarting: clearing the backoff lets the next supervision pass spawn
        // a replacement. It cannot unstick a worker blocked inside a DDC call,
        // so that diagnosis stands until the worker itself answers — clearing
        // it here would retract a warning that is still true, only for it to
        // reappear seconds later when the next set times out.
        if self.ddc_health == DdcHealth::WorkerDead {
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
            return Err(BrightnessError::MonitorNotFound(
                target_id.base_display_name(),
            ));
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

        // 5. Update overlay (software layer is immediately effective). Commit
        // the opacity only once the platform call succeeds, so a failed
        // update cannot leave the state claiming an opacity the window never
        // received.
        if new_overlay != old_overlay
            && let Err(e) = self.overlay_update(&target_id, handle, new_overlay)
        {
            log::error!(error:% = e; "Overlay update failed; reverting optimistic value");
            if let Some(state) = self.states.get_mut(&target_id) {
                state.force_revert();
            }
            self.show_error_on_visible_osd();
            return Err(e);
        }

        // Re-borrow: `overlay_update` needs `&mut self` (it may also touch
        // `self.settings`), which the borrow checker cannot prove disjoint
        // from the `state` borrow taken above, so it is refreshed here
        // rather than held live across that call. Nothing removes monitor
        // states between the lookup above and here, so this always succeeds.
        let Some(state) = self.states.get_mut(&target_id) else {
            return Ok(());
        };
        state.overlay_opacity = new_overlay;

        // 6. Show or update OSD with optimistic values. An OSD failure is
        // logged, not propagated: the OSD is feedback only, and bailing out
        // between set_pending and the DDC send would leave a pending with no
        // command in flight — the watchdog would then misread it as a set
        // timeout and count a healthy worker toward the hung-DDC latch.
        self.osd_monitor = Some(target_id.clone());
        let osd_result = if self.osd.is_visible() {
            self.osd.update(state)
        } else {
            self.osd.show(handle, state)
        };
        if let Err(e) = osd_result {
            log::error!(error:% = e; "OSD update failed; continuing adjustment");
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
        self.note_worker_alive();

        let Some(state) = self.states.get_mut(monitor_id) else {
            log::warn!(monitor:% = monitor_id.base_display_name(); "Received DDC result for unknown monitor");
            log::debug!(monitor_id:% = monitor_id.full_identity(); "Unknown-monitor DDC result identity");
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
                log::error!(monitor:% = monitor_id.base_display_name(), target_brightness = value, error = error_msg; "DDC failed to set brightness");
                log::debug!(monitor_id:% = monitor_id.full_identity(); "DDC set failure identity");
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
        if let Some(state) = self.states.get(&id)
            && let Err(e) = self.osd.update_error(state)
        {
            log::warn!(error:% = e; "Failed to update OSD error state");
        }
    }

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
        if self.ddc.is_alive() {
            return;
        }
        // A worker diagnosed as unresponsive that has since died is simply
        // dead. The reason never to respawn a hung worker — two threads
        // against the same physical-monitor handles — went with the thread,
        // and no user action clears that diagnosis, so leaving it standing
        // here would strand the app with no worker at all.
        if self.ddc_health == DdcHealth::WorkerHung {
            log::warn!("Unresponsive DDC worker has exited; treating it as a death");
            self.ddc_health = DdcHealth::Ok;
        }
        // Backoff already exhausted: wait for a keypress or resume to retry.
        if self.ddc_health.is_degraded() {
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
                self.ddc_health = DdcHealth::WorkerDead;
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
                log::error!(monitor:% = id.base_display_name(); "DDC set timed out with no result; reverted");
                log::debug!(monitor_id:% = id.full_identity(); "Timed-out set identity");
            }
            self.consecutive_set_timeouts += 1;
            self.show_error_on_visible_osd();

            if self.ddc.is_alive()
                && !self.ddc_health.is_degraded()
                && self.consecutive_set_timeouts >= HUNG_TIMEOUT_LIMIT
            {
                log::error!(count = self.consecutive_set_timeouts; "DDC worker unresponsive; disabling DDC until it answers, resume, or restart");
                self.ddc_health = DdcHealth::WorkerHung;
            }
        }

        if self.refresh.timed_out(now, REFRESH_TIMEOUT) {
            log::error!("DDC refresh timed out with no result; aborting");
            self.refresh.abort();
        }

        if let Some((op, since)) = self.pending_hotkey_op
            && now.saturating_duration_since(since) >= REBIND_TIMEOUT
        {
            log::error!(op:? = op; "Hotkey thread did not respond to posted operation");
            self.pending_hotkey_op = None;
            self.fail_hotkey_op(op, "Hotkey thread did not respond", now);
        }
    }

    /// Force-reverts every pending set (used after a worker respawn).
    fn reconcile_all_pending(&mut self) {
        for state in self.states.values_mut() {
            state.force_revert();
        }
        self.show_error_on_visible_osd();
    }

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
            BrightnessMessage::AdjustStep { direction } => {
                let step = self.config.brightness.step_percent.cast_signed();
                let delta = direction.signum().saturating_mul(step);
                self.handle_adjust(None, delta, now)?;
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
            BrightnessMessage::TrayOpenLogFolder | BrightnessMessage::OpenConfigFile => {
                // Shell side effects (Explorer / the default JSON editor);
                // the binary's loop intercepts and handles them before this
                // point. Reaching this arm means that interception was
                // missed, silently no-op'ing the menu/dialog item.
                log::warn!(
                    "Shell message reached core controller unhandled; wiring regression \
                     (menu/dialog item will silently no-op)"
                );
            }
            BrightnessMessage::TrayOpenSettings => {
                self.settings_open = true;
                let snapshot = self.settings_snapshot();
                self.settings.open(&snapshot);
            }
            // ── Settings Dialog Messages ─────────────────────────────────
            BrightnessMessage::SettingChanged(change) => {
                self.handle_setting_changed(change, now);
            }
            BrightnessMessage::SettingsClosed => {
                self.settings_open = false;
                if self.capture_active {
                    self.capture_active = false;
                    self.post_hotkey_resume(now);
                }
                self.flush_pending_settings(now);
            }
            BrightnessMessage::HotkeyRebindResult {
                op,
                success,
                fallback_active,
                error,
            } => {
                self.handle_hotkey_rebind_result(op, success, fallback_active, error, now);
            }
            BrightnessMessage::HotkeyCaptureStarted => {
                self.capture_active = true;
                self.post_hotkey_suspend(now);
            }
            BrightnessMessage::HotkeyCaptureEnded => {
                self.capture_active = false;
                self.post_hotkey_resume(now);
            }
            BrightnessMessage::TrayRequestQuit => {
                log::info!("Quit requested from tray menu");
                self.flush_pending_settings(now);
                return Ok(false);
            }
            BrightnessMessage::TrayMenuOpening { reply_tx } => {
                log::trace!("TrayMenuOpening received");
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
                self.flush_pending_settings(now);
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Builds the data needed to populate the tray menu.
    ///
    /// Generates display names with duplicate suffixes (e.g., "Dell U2722D #1")
    /// when multiple monitors with identical manufacturer and model are connected.
    fn build_tray_menu_data(&self) -> TrayMenuData {
        // Collect monitor IDs and generate unique display names
        let monitor_ids: Vec<MonitorId> = self.states.keys().cloned().collect();
        let display_names = generate_display_names(&monitor_ids);

        let mut monitors: Vec<TrayMonitorInfo> = self
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
                    brightness_known: state.brightness_known,
                    overlay_opacity: state.overlay_opacity,
                }
            })
            .collect();

        // HashMap iteration order is nondeterministic; sort so the menu is
        // stable across openings instead of shuffling monitors each time.
        monitors.sort_by(|a, b| a.display_name.cmp(&b.display_name));

        TrayMenuData {
            monitors,
            warnings: self.health_warnings(),
            hotkey_up: self.config.hotkeys.brightness_up.clone(),
            hotkey_down: self.config.hotkeys.brightness_down.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::config::{
        DEFAULT_FILE_LOG_LEVEL, DEFAULT_HOTKEY_DOWN, DEFAULT_HOTKEY_UP, DEFAULT_OSD_OPACITY,
        DEFAULT_OSD_TIMEOUT_MS, DEFAULT_REFRESH_INACTIVITY_SECONDS,
        DEFAULT_REFRESH_PERIODIC_SECONDS, DEFAULT_STEP_PERCENT,
    };
    use std::sync::mpsc;

    // ── Fakes ────────────────────────────────────────────────────────────

    #[derive(Default)]
    struct FakeOsd {
        visible: bool,
        shows: Vec<(MonitorHandle, u8)>,
        updates: Vec<u8>,
        error_updates: Vec<u8>,
        appearance_calls: Vec<(f32, u32)>,
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

    type TestController = Controller<
        FakeOsd,
        FakeOverlay,
        FakeDdc,
        FakeLocator,
        FakeSettings,
        FakeHotkeyPort,
        FakeStore,
    >;

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
            FakeSettings::default(),
            FakeHotkeyPort::default(),
            FakeStore::default(),
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

        let err = c.handle_adjust(None, -10, base).unwrap_err();

        assert!(matches!(err, BrightnessError::ChannelSend));
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
        let err = c.handle_adjust(None, 15, base).unwrap_err();

        assert!(matches!(err, BrightnessError::ChannelSend));
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
            },
            base + Duration::from_millis(50),
        )
        .unwrap();

        assert!(c.pending_hotkey_op.is_none());
        assert!(c.prev_hotkeys.is_none());
        assert!(!c.hotkeys_degraded, "the rebind still succeeded");
        assert_eq!(
            c.settings.notices,
            vec![
                "Brightness-key interception unavailable; using plain key registration".to_string()
            ]
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
}
