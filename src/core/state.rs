//! Application state types for the brightness control tool.
//!
//! This module defines the core state structures used throughout the application,
//! including monitor identification, per-monitor state, inter-thread messages,
//! and DDC worker communication types.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

// ─────────────────────────────────────────────────────────────────────────────
// Monitor Identification
// ─────────────────────────────────────────────────────────────────────────────

/// Unique identifier for a monitor based on EDID data.
///
/// This struct provides cross-platform monitor identification using
/// manufacturer ID, model name, and optional serial number from EDID.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MonitorId {
    /// 3-character `PnP` manufacturer ID (e.g., "DEL" for Dell).
    pub manufacturer: String,
    /// Model name from EDID descriptor.
    pub model_name: String,
    /// Serial number from EDID descriptor (not all monitors provide this).
    pub serial_number: Option<String>,
}

impl MonitorId {
    /// Creates a new monitor identifier.
    #[must_use]
    pub fn new(
        manufacturer: impl Into<String>,
        model_name: impl Into<String>,
        serial_number: Option<String>,
    ) -> Self {
        Self {
            manufacturer: manufacturer.into(),
            model_name: model_name.into(),
            serial_number,
        }
    }

    /// Returns the full identity including the serial number when present.
    ///
    /// The serial number is treated as PII: log this form at `debug!` level
    /// only — never at info/warn/error — and never show it in user-facing UI.
    /// For all other purposes use `Display`/[`Self::base_display_name`],
    /// which are serial-free.
    #[must_use]
    pub fn full_identity(&self) -> String {
        match &self.serial_number {
            Some(sn) => format!("{} (SN:{})", self.base_display_name(), sn),
            None => self.base_display_name(),
        }
    }

    /// Returns the base display name without serial number.
    ///
    /// Format: `"{manufacturer} {model_name}"` (e.g., "DEL U2722D")
    ///
    /// If the model name already starts with the manufacturer prefix
    /// (case-insensitive), only the model name is returned to avoid
    /// duplication (e.g., "PHL 346B1C" instead of "PHL PHL 346B1C").
    ///
    /// Use [`generate_display_names`] to get unique names with index suffixes
    /// when multiple monitors share the same base name.
    #[must_use]
    pub fn base_display_name(&self) -> String {
        // Check if model_name already starts with manufacturer prefix (case-insensitive)
        // to avoid duplication like "PHL PHL 346B1C" when EDID contains redundant info
        let prefix = format!("{} ", self.manufacturer);
        if self
            .model_name
            .to_ascii_uppercase()
            .starts_with(&prefix.to_ascii_uppercase())
        {
            self.model_name.clone()
        } else {
            format!("{} {}", self.manufacturer, self.model_name)
        }
    }
}

// Serial-free by design: `Display` is what log statements emit at every
// level via `:%`, so the PII-bearing serial must never appear here. The
// serial-bearing form is only available through the explicitly named
// `full_identity()`, reserved for `debug!`-level logging.
impl std::fmt::Display for MonitorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.base_display_name())
    }
}

#[cfg(test)]
mod monitor_id_tests {
    use super::*;

    #[test]
    fn base_display_name_normal() {
        let id = MonitorId::new("DEL", "U2722D", None);
        assert_eq!(id.base_display_name(), "DEL U2722D");
    }

    #[test]
    fn base_display_name_redundant_prefix() {
        // Simulates Philips monitor with redundant manufacturer in model name
        let id = MonitorId::new("PHL", "PHL 346B1C", None);
        assert_eq!(id.base_display_name(), "PHL 346B1C");
    }

    #[test]
    fn base_display_name_redundant_prefix_case_insensitive() {
        // Case mismatch should still be detected
        let id = MonitorId::new("PHL", "phl 346B1C", None);
        assert_eq!(id.base_display_name(), "phl 346B1C");
    }

    #[test]
    fn base_display_name_partial_match_no_skip() {
        // "PHLEGM" starts with "PHL" but not "PHL " - should NOT skip
        let id = MonitorId::new("PHL", "PHLEGM 123", None);
        assert_eq!(id.base_display_name(), "PHL PHLEGM 123");
    }

    #[test]
    fn display_omits_serial_number() {
        // Display is used by log statements at every level, so it must never
        // carry the serial (PII rule: serials at debug! only, via full_identity).
        let id = MonitorId::new("DEL", "U2722D", Some("ABC123".to_string()));
        assert_eq!(id.to_string(), "DEL U2722D");
    }

    #[test]
    fn full_identity_includes_serial_number() {
        let id = MonitorId::new("DEL", "U2722D", Some("ABC123".to_string()));
        assert_eq!(id.full_identity(), "DEL U2722D (SN:ABC123)");
    }

    #[test]
    fn full_identity_without_serial_is_base_name() {
        let id = MonitorId::new("DEL", "U2722D", None);
        assert_eq!(id.full_identity(), "DEL U2722D");
    }
}

/// Generates unique display names for monitors, appending indices for duplicates.
///
/// When multiple monitors have the same base name (manufacturer + model),
/// they are suffixed with `" #1"`, `" #2"`, etc. Monitors with unique base names
/// are returned without a suffix.
///
/// # Arguments
///
/// * `ids` - Slice of monitor identifiers to generate names for.
///
/// # Returns
///
/// A map from each `MonitorId` to its unique display name string.
///
/// # Example
///
/// - Single unique monitor: `"Dell U2722D"`
/// - Two identical monitors: `"Dell U2722D #1"`, `"Dell U2722D #2"`
#[must_use]
pub fn generate_display_names(ids: &[MonitorId]) -> HashMap<MonitorId, String> {
    // Count occurrences of each base name
    let mut base_name_counts: HashMap<String, usize> = HashMap::new();
    for id in ids {
        let base = id.base_display_name();
        *base_name_counts.entry(base).or_insert(0) += 1;
    }

    // Track current index for each base name (for duplicates)
    let mut base_name_indices: HashMap<String, usize> = HashMap::new();

    // Generate unique names
    let mut result = HashMap::new();
    for id in ids {
        let base = id.base_display_name();
        let display_name = if base_name_counts[&base] > 1 {
            // Multiple monitors with same base name - append index
            let index = base_name_indices.entry(base.clone()).or_insert(0);
            *index += 1;
            format!("{base} #{index}")
        } else {
            // Unique monitor - use base name as-is
            base
        };
        result.insert(id.clone(), display_name);
    }

    result
}

// ─────────────────────────────────────────────────────────────────────────────
// Monitor State
// ─────────────────────────────────────────────────────────────────────────────

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

/// Per-monitor state tracking brightness values and cache status.
#[derive(Debug)]
pub struct MonitorState {
    /// Last confirmed DDC brightness value (0-100).
    pub cached_brightness: u8,
    /// Optimistic brightness set awaiting DDC confirmation.
    pub pending: Option<PendingSet>,
    /// Current overlay opacity (0-100, where 0 = invisible).
    pub overlay_opacity: u8,
    /// Timestamp of last successful DDC read/write.
    pub last_refresh: Instant,
    /// First observation of this monitor's current run of enumeration absence.
    ///
    /// `None` while the monitor is present (or absence was never observed).
    /// Stamped by the controller on the first current-generation refresh that
    /// does not enumerate the monitor; a later miss ≥ the prune window prunes.
    pub missing_since: Option<Instant>,
}

impl MonitorState {
    /// Creates a new monitor state with the given initial brightness.
    #[must_use]
    pub fn new(initial_brightness: u8) -> Self {
        Self {
            cached_brightness: initial_brightness.min(100),
            pending: None,
            overlay_opacity: 0,
            last_refresh: Instant::now(),
            missing_since: None,
        }
    }

    /// Returns the effective brightness to display in the OSD.
    ///
    /// Uses the pending value if a set is in flight, otherwise the cached value.
    #[must_use]
    pub fn effective_brightness(&self) -> u8 {
        self.pending.map_or(self.cached_brightness, |p| p.value)
    }

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
                // No seq/recency check here — safe only because the single DDC
                // worker executes sets sequentially and delivers results in
                // issue order, so a success arriving with nothing pending is
                // always the most recent hardware write. Multiple workers or
                // out-of-order delivery would need a seq gate on this branch.
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

    /// Updates the cached brightness from a DDC read.
    ///
    /// Leaves any live pending set intact: a refresh read is older intent than
    /// an optimistic set that is still awaiting its own result.
    pub fn update_from_ddc(&mut self, value: u8) {
        self.cached_brightness = value.min(100);
        self.last_refresh = Instant::now();
    }
}

impl Default for MonitorState {
    fn default() -> Self {
        Self::new(50)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tray Menu Data
// ─────────────────────────────────────────────────────────────────────────────

/// Information about a single monitor for display in the tray menu.
#[derive(Debug, Clone)]
pub struct TrayMonitorInfo {
    /// Display name with optional index suffix (e.g., "Dell U2722D" or "Dell U2722D #1").
    pub display_name: String,
    /// Current hardware brightness value (0-100).
    pub hardware_brightness: u8,
    /// Current overlay opacity (0-100, where 0 = invisible).
    pub overlay_opacity: u8,
}

/// Degraded-subsystem warnings surfaced to the user via the tray icon/menu.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HealthWarnings {
    /// DDC is disabled after respawn backoff or a diagnosed worker hang.
    pub ddc_degraded: bool,
    /// The hotkey thread died repeatedly and supervision gave up.
    pub hotkeys_lost: bool,
}

/// Data sent from the main thread to the tray thread for menu population.
#[derive(Debug, Clone)]
pub struct TrayMenuData {
    /// List of monitors with their current brightness/overlay state.
    pub monitors: Vec<TrayMonitorInfo>,
    /// Configured hotkey string for brightness up (e.g., "Ctrl+Shift+Up").
    pub hotkey_up: String,
    /// Configured hotkey string for brightness down (e.g., "Ctrl+Shift+Down").
    pub hotkey_down: String,
    /// Active degraded-subsystem warnings to show in the menu.
    pub warnings: HealthWarnings,
}

// ─────────────────────────────────────────────────────────────────────────────
// Inter-Thread Messages
// ─────────────────────────────────────────────────────────────────────────────

/// Messages sent between threads for brightness control operations.
///
/// These messages are sent from the hotkey thread to the main thread
/// via an MPSC channel.
#[derive(Debug, Clone)]
pub enum BrightnessMessage {
    /// Result of a DDC brightness set operation (from DDC worker).
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
    /// Result of a DDC refresh operation (from DDC worker).
    DdcRefreshResult {
        /// Generation echoed from the originating refresh command.
        generation: u64,
        /// List of (`monitor_id`, brightness) pairs for all detected monitors.
        monitors: Vec<(MonitorId, u8)>,
        /// Every monitor whose identification succeeded this pass, readable or
        /// not. Superset of `monitors`' ids; empty when enumeration itself
        /// failed. Presence proof for absence-based pruning.
        enumerated: Vec<MonitorId>,
    },
    /// Adjust brightness by a relative delta.
    Adjust {
        /// Target monitor (None = monitor under cursor).
        monitor_id: Option<MonitorId>,
        /// Brightness change (-100 to +100).
        delta: i8,
    },
    /// Refresh cached brightness values from all monitors.
    Refresh,
    /// System resumed from sleep/hibernate.
    ///
    /// Sent by the power event listener when the system wakes up.
    /// Triggers a refresh to resync with monitors that may have
    /// reset their brightness during sleep.
    SystemResumed,

    // ── Tray Icon Messages ───────────────────────────────────────────────
    /// User clicked the "Usage" menu item in the tray menu.
    ///
    /// The main thread should open a modeless window displaying usage
    /// instructions (hotkeys). Only one instance of this window should
    /// exist at a time; if already open, bring it to front.
    TrayOpenUsage,

    /// User clicked the "Settings" menu item in the tray menu.
    ///
    /// The main thread should open the config file with the system default editor.
    TrayOpenSettings,

    /// User clicked the "Open Log Folder" menu item in the tray menu.
    ///
    /// The main thread should open the application data directory (config +
    /// rolling log files) in the file explorer.
    TrayOpenLogFolder,

    /// User clicked the "Quit" menu item in the tray menu.
    ///
    /// Triggers graceful application shutdown.
    TrayRequestQuit,

    /// Tray thread requests current monitor state for menu population.
    ///
    /// The main thread should respond by sending `TrayMenuData` through
    /// the provided channel.
    TrayMenuOpening {
        /// Channel to send the menu data back to the tray thread.
        reply_tx: Sender<TrayMenuData>,
    },

    /// Shutdown the application gracefully.
    Shutdown,
}

#[cfg(test)]
mod pending_reconcile_tests {
    use super::*;
    use crate::core::reconcile::SET_TIMEOUT;
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

    #[test]
    fn new_state_has_no_absence_evidence() {
        let s = MonitorState::new(50);
        assert!(s.missing_since.is_none());
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DDC Worker Commands
// ─────────────────────────────────────────────────────────────────────────────

/// Commands sent from the main thread to the DDC worker thread.
///
/// The DDC worker executes these commands and sends results back
/// via `BrightnessMessage::DdcSetResult` or `BrightnessMessage::DdcRefreshResult`.
#[derive(Debug, Clone)]
pub enum DdcCommand {
    /// Set brightness for a specific monitor.
    SetBrightness {
        /// Target monitor.
        monitor_id: MonitorId,
        /// Brightness value to set (0-100).
        value: u8,
        /// Sequence id correlating this command to its result.
        seq: u64,
    },
    /// Refresh all monitors: enumerate and read current brightness values.
    RefreshAll {
        /// Generation correlating this refresh to its result.
        generation: u64,
    },
    /// Shutdown the DDC worker thread.
    Shutdown,
}
