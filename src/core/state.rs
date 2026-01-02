//! Application state types for the brightness control tool.
//!
//! This module defines the core state structures used throughout the application,
//! including monitor identification, per-monitor state, inter-thread messages,
//! and DDC worker communication types.

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
}

impl std::fmt::Display for MonitorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.display_name())
    }
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
        /// List of (monitor_id, brightness) pairs for all detected monitors.
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
