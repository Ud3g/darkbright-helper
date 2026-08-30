//! Windows settings dialog window.
//!
//! Stubbed for now: window wiring lands with the settings-window work. Every
//! method just logs what it would have shown/asserted so the controller's
//! seam has something to call against while it is developed.

use crate::core::controller::SettingsSink;
use crate::core::state::SettingsSnapshot;

/// Placeholder [`SettingsSink`]: no window exists yet.
pub struct SettingsSinkImpl;

impl SettingsSink for SettingsSinkImpl {
    fn open(&mut self, snapshot: &SettingsSnapshot) {
        log::debug!(snapshot:? = snapshot; "SettingsSink::open (stub, window wiring lands with the settings-window work)");
    }

    fn refresh(&mut self, snapshot: &SettingsSnapshot) {
        log::debug!(snapshot:? = snapshot; "SettingsSink::refresh (stub, window wiring lands with the settings-window work)");
    }

    fn hotkey_error(&mut self, message: &str) {
        log::debug!(message = message; "SettingsSink::hotkey_error (stub, window wiring lands with the settings-window work)");
    }

    fn hotkey_notice(&mut self, message: &str) {
        log::debug!(message = message; "SettingsSink::hotkey_notice (stub, window wiring lands with the settings-window work)");
    }

    fn assert_topmost(&mut self) {
        log::debug!(
            "SettingsSink::assert_topmost (stub, window wiring lands with the settings-window work)"
        );
    }
}
