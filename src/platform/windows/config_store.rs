//! Windows-backed persistence for the settings-dialog `ConfigStore` seam.

use std::path::PathBuf;

use crate::core::config::{Config, SettingsDirty};
use crate::core::controller::{ConfigStore, SaveResult};

/// Saves the runtime config to `config.json` via [`Config::save_to`].
///
/// Holds an `Option<PathBuf>` rather than a bare path: [`Config::default_path`]
/// itself returns `None` when `%APPDATA%` is unset, and a store built from that
/// must still exist and answer every save with [`SaveResult::Failed`] instead
/// of panicking or guessing a path.
///
/// Merge/dirty-aware saving is not implemented yet — every call writes
/// `config` in full via [`Config::save_to`], ignoring the dirty set and
/// `force`. A later change replaces the internals while keeping this seam's
/// public shape.
pub struct WindowsConfigStore {
    path: Option<PathBuf>,
}

impl WindowsConfigStore {
    /// Creates a store that saves to `path`; `None` makes every save fail.
    #[must_use]
    pub fn new(path: Option<PathBuf>) -> Self {
        Self { path }
    }
}

impl ConfigStore for WindowsConfigStore {
    fn save(&mut self, config: &Config, _dirty: &SettingsDirty, _force: bool) -> SaveResult {
        let Some(path) = &self.path else {
            log::error!("No config path available (APPDATA unset?); cannot save settings");
            return SaveResult::Failed("no config path available".to_string());
        };
        match config.save_to(path) {
            Ok(()) => SaveResult::Saved,
            Err(e) => {
                log::error!(error:% = e; "Failed to save config");
                SaveResult::Failed(e.to_string())
            }
        }
    }
}
