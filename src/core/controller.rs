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
use crate::core::i18n::{Lang, strings};
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

    /// Switches the language any text the OSD draws is rendered in. Takes
    /// effect on the next paint; a visible OSD is not repainted for it.
    fn set_language(&mut self, lang: Lang);
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

    /// Relabels every control, the title and both pickers in `lang`. Sent
    /// only when the resolved language actually changed.
    fn set_language(&mut self, lang: Lang);
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
#[expect(clippy::struct_excessive_bools)]
pub struct Controller<Osd, Ovl, Ddc, Loc, Set, Hk, Store> {
    /// Current state (brightness, overlay, absence evidence) per monitor.
    states: HashMap<MonitorId, MonitorState>,
    /// Dimming overlay windows.
    overlay: Ovl,
    /// On-screen display.
    osd: Osd,
    /// Loaded configuration.
    config: Config,
    /// The OS's ordered UI-language preference list, read once at startup;
    /// what a `"system"` language choice resolves through.
    os_languages: Vec<String>,
    /// The language every user-visible string is currently rendered in.
    /// Owned here, like every other piece of runtime state, and pushed to
    /// the OSD and settings seams when it changes; the tray learns it from
    /// the binary's loop, which diffs [`Controller::lang`] the way it diffs
    /// the health warnings.
    lang: Lang,
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
    #[expect(clippy::too_many_arguments)] // one independent seam per parameter
    #[must_use]
    pub fn new(
        config: Config,
        os_languages: Vec<String>,
        osd: Osd,
        overlay: Ovl,
        ddc: Ddc,
        locator: Loc,
        settings: Set,
        hotkey_port: Hk,
        store: Store,
        now: Instant,
    ) -> Self {
        let lang = config.language_setting().resolve(&os_languages);
        Self {
            states: HashMap::new(),
            overlay,
            osd,
            config,
            os_languages,
            lang,
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

    /// The language the UI is rendered in right now.
    #[must_use]
    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// Re-resolves the language from the config and pushes it to the OSD and
    /// settings window if it changed. A change of the stored choice that
    /// resolves to the same language pushes nothing: nothing on screen
    /// would change.
    fn apply_language(&mut self) {
        let lang = self.config.language_setting().resolve(&self.os_languages);
        if lang == self.lang {
            return;
        }
        log::info!(from = self.lang.tag(), to = lang.tag(); "UI language changed");
        self.lang = lang;
        self.osd.set_language(lang);
        self.settings.set_language(lang);
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

        // A configured 0 disables periodic refresh.
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
    pub(crate) fn settings_snapshot(&self) -> SettingsSnapshot {
        let opacity_percent = (self.config.osd.opacity * 100.0).round().clamp(10.0, 100.0);
        // The clamp above bounds this to 10.0..=100.0, well within u8 range.
        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
            language: self.config.language_setting(),
            lang: self.lang,
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
            SettingChange::Language(setting) => {
                self.config.language = setting.wire().to_string();
                self.dirty.language = true;
                self.apply_language();
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
            // Status text reaches the settings window, so it comes from the table.
            self.settings
                .hotkey_error(strings(self.lang).hotkey_status_unreachable);
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
                .hotkey_error(strings(self.lang).hotkey_status_unreachable);
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
                .hotkey_error(strings(self.lang).hotkey_status_unreachable);
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
        restore_error: Option<String>,
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
                self.settings
                    .hotkey_notice(strings(self.lang).hotkey_notice_interception_unavailable);
            }
            return;
        }

        let message = match (error, restore_error) {
            (Some(error), Some(restore_error)) => strings(self.lang)
                .hotkey_status_restore_also_failed_fmt
                .replace("{error}", &error)
                .replace("{restore_error}", &restore_error),
            (Some(error), None) => error,
            (None, _) => strings(self.lang).hotkey_status_unknown_error.to_string(),
        };
        self.fail_hotkey_op(op, &message, now);
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

    /// Resets the eleven settings-dialog fields to their defaults and schedules
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
            language: true,
        };
        self.pending_save_since = Some(now);

        let snapshot = self.settings_snapshot();
        self.settings.refresh(&snapshot);
        self.apply_language();

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
    #[expect(clippy::needless_pass_by_value)]
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

        // A configured 0 disables inactivity refresh.
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

        // Activity means the user is back, so a refresh that previously found
        // nothing is worth retrying.
        if !self.refresh.last_successful() && !self.refresh.in_progress() {
            log::debug!("Triggering refresh on user activity (last refresh found no monitors)");
            self.handle_refresh(now);
        }

        self.last_activity = now;

        // The handle is needed for OSD and overlay positioning.
        let handle = self.locator.monitor_under_cursor()?;

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

        // Optimistic: state advances now, the DDC result reconciles it later.
        let seq = self.next_seq;
        if new_hardware != old_hardware {
            self.next_seq += 1;
            state.set_pending(new_hardware, seq, now);
        }

        // The overlay takes effect immediately, but the opacity is committed to
        // state only once the platform call succeeds: a failed update must not
        // leave the state claiming an opacity the window never received.
        if new_overlay != old_overlay
            && let Err(e) = self.overlay_update(&target_id, handle, new_overlay)
        {
            log::error!(error:% = e; "Overlay update failed; reverting optimistic value");
            if let Some(state) = self.states.get_mut(&target_id) {
                state.force_revert();
            }
            self.show_error_on_visible_osd();
            // Handled, so reported as handled: the value is reverted and the
            // OSD says so. Propagating the error as well would have the main
            // loop log the same event a second time, with less to say about
            // it. The DDC send failure below takes the same shape.
            return Ok(());
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

        // An OSD failure is logged, not propagated: the OSD is feedback only,
        // and bailing out between set_pending and the DDC send would leave a
        // pending with no command in flight — the watchdog would then misread
        // it as a set timeout and count a healthy worker toward the hung-DDC
        // latch.
        self.osd_monitor = Some(target_id.clone());
        let osd_result = if self.osd.is_visible() {
            self.osd.update(state)
        } else {
            self.osd.show(handle, state)
        };
        if let Err(e) = osd_result {
            log::error!(error:% = e; "OSD update failed; continuing adjustment");
        }

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
            self.fail_hotkey_op(op, strings(self.lang).hotkey_status_no_response, now);
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
                restore_error,
            } => {
                self.handle_hotkey_rebind_result(
                    op,
                    success,
                    fallback_active,
                    error,
                    restore_error,
                    now,
                );
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
mod tests;
