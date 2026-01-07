//! Application state types for the brightness control tool.
//!
//! This module defines the core state structures used throughout the application,
//! including monitor identification, per-monitor state, inter-thread messages,
//! and DDC worker communication types.

use std::collections::HashMap;
use std::sync::mpsc::Sender;
use std::time::Instant;

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

    /// Returns a display-friendly string for this monitor.
    #[must_use]
    pub fn display_name(&self) -> String {
        match &self.serial_number {
            Some(sn) => format!("{} {} (SN:{})", self.manufacturer, self.model_name, sn),
            None => format!("{} {}", self.manufacturer, self.model_name),
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
        if self.model_name
            .to_ascii_uppercase()
            .starts_with(&prefix.to_ascii_uppercase())
        {
            self.model_name.clone()
        } else {
            format!("{} {}", self.manufacturer, self.model_name)
        }
    }
}

impl std::fmt::Display for MonitorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
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

/// Per-monitor state tracking brightness values and cache status.
#[derive(Debug)]
pub struct MonitorState {
    /// Last confirmed DDC brightness value (0-100).
    pub cached_brightness: u8,
    /// Optimistic brightness value awaiting DDC confirmation.
    pub pending_brightness: Option<u8>,
    /// Current overlay opacity (0-100, where 0 = invisible).
    pub overlay_opacity: u8,
    /// Timestamp of last successful DDC read/write.
    pub last_refresh: Instant,
}

impl MonitorState {
    /// Creates a new monitor state with the given initial brightness.
    #[must_use]
    pub fn new(initial_brightness: u8) -> Self {
        Self {
            cached_brightness: initial_brightness.min(100),
            pending_brightness: None,
            overlay_opacity: 0,
            last_refresh: Instant::now(),
        }
    }

    /// Returns the effective brightness to display in the OSD.
    ///
    /// Uses pending value if available, otherwise cached value.
    #[must_use]
    pub fn effective_brightness(&self) -> u8 {
        self.pending_brightness.unwrap_or(self.cached_brightness)
    }

    /// Confirms a pending brightness change after successful DDC write.
    pub fn confirm_brightness(&mut self) {
        if let Some(pending) = self.pending_brightness.take() {
            self.cached_brightness = pending;
            self.last_refresh = Instant::now();
        }
    }

    /// Reverts a pending brightness change after DDC failure.
    pub fn revert_pending(&mut self) {
        self.pending_brightness = None;
    }

    /// Sets a new pending brightness value (optimistic update).
    pub fn set_pending(&mut self, value: u8) {
        self.pending_brightness = Some(value.min(100));
    }

    /// Updates the cached brightness from a DDC read.
    pub fn update_from_ddc(&mut self, value: u8) {
        self.cached_brightness = value.min(100);
        self.pending_brightness = None;
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

/// Data sent from the main thread to the tray thread for menu population.
#[derive(Debug, Clone)]
pub struct TrayMenuData {
    /// List of monitors with their current brightness/overlay state.
    pub monitors: Vec<TrayMonitorInfo>,
    /// Configured hotkey string for brightness up (e.g., "Ctrl+Shift+Up").
    pub hotkey_up: String,
    /// Configured hotkey string for brightness down (e.g., "Ctrl+Shift+Down").
    pub hotkey_down: String,
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
        /// Success or error message.
        success: bool,
        /// Error message if failed.
        error: Option<String>,
    },
    /// Result of a DDC refresh operation (from DDC worker).
    DdcRefreshResult {
        /// List of (`monitor_id`, brightness) pairs for all detected monitors.
        monitors: Vec<(MonitorId, u8)>,
    },
    /// Adjust brightness by a relative delta.
    Adjust {
        /// Target monitor (None = monitor under cursor).
        monitor_id: Option<MonitorId>,
        /// Brightness change (-100 to +100).
        delta: i8,
    },
    /// Set brightness to an absolute value.
    SetAbsolute {
        /// Target monitor (None = monitor under cursor).
        monitor_id: Option<MonitorId>,
        /// Target brightness (0-100).
        value: u8,
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
    },
    /// Refresh all monitors: enumerate and read current brightness values.
    RefreshAll,
    /// Shutdown the DDC worker thread.
    Shutdown,
}
