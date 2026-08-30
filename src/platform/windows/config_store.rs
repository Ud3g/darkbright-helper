//! Windows-backed persistence for the settings-dialog `ConfigStore` seam.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::core::config::{Config, SettingsDirty};
use crate::core::controller::{ConfigStore, SaveResult};

/// Saves the runtime config to `config.json` via [`Config::save_to`],
/// merging onto a file that was hand-edited (or edited by another instance)
/// since our last read or write.
///
/// Holds an `Option<PathBuf>` rather than a bare path: [`Config::default_path`]
/// itself returns `None` when `%APPDATA%` is unset, and a store built from that
/// must still exist and answer every save with [`SaveResult::Failed`] instead
/// of panicking or guessing a path.
///
/// Change detection is by file identity (length + modified time), not
/// content hashing: cheap to stat, and any edit that matters for merging
/// also changes at least one of the two.
pub struct WindowsConfigStore {
    path: Option<PathBuf>,
    /// (len, modified) of the file as of our last read or write.
    last_identity: Option<(u64, SystemTime)>,
}

impl WindowsConfigStore {
    /// Creates a store that saves to `path`; `None` makes every save fail.
    ///
    /// Captures the current identity of the file at `path` (if any) so the
    /// first save can detect edits made after startup rather than treating
    /// the already-loaded-from file as "changed".
    #[must_use]
    pub fn new(path: Option<PathBuf>) -> Self {
        let last_identity = path.as_deref().and_then(Self::identity);
        Self {
            path,
            last_identity,
        }
    }

    /// Reads `(len, modified)` for `path`, or `None` if it can't be stat'd
    /// (missing, permissions, or a filesystem that doesn't report mtimes).
    fn identity(path: &Path) -> Option<(u64, SystemTime)> {
        let meta = std::fs::metadata(path).ok()?;
        let modified = meta.modified().ok()?;
        Some((meta.len(), modified))
    }

    /// Writes `config` in full and refreshes the identity baseline.
    fn write_direct(&mut self, path: &Path, config: &Config) -> SaveResult {
        match config.save_to(path) {
            Ok(()) => {
                self.last_identity = Self::identity(path);
                SaveResult::Saved
            }
            Err(e) => {
                log::error!(error:% = e; "Failed to save config");
                SaveResult::Failed(e.to_string())
            }
        }
    }
}

impl ConfigStore for WindowsConfigStore {
    fn save(&mut self, config: &Config, dirty: &SettingsDirty, force: bool) -> SaveResult {
        let Some(path) = self.path.clone() else {
            log::error!("No config path available (APPDATA unset?); cannot save settings");
            return SaveResult::Failed("no config path available".to_string());
        };

        let current_identity = Self::identity(&path);
        let externally_changed = matches!(
            (self.last_identity, current_identity),
            (Some(last), Some(current)) if last != current
        );

        if !externally_changed {
            return self.write_direct(&path, config);
        }

        // The file moved since we last touched it. Re-read it with a raw
        // parse (not the loader's default-substitution/repair path — a file
        // that only parses after repair counts as unparseable here, since we
        // must not silently discard whatever made it need repair) and merge
        // only our dirty fields onto it, so a concurrent hand-edit of an
        // untouched field survives.
        match std::fs::read_to_string(&path) {
            Ok(contents) => match serde_json::from_str::<Config>(&contents) {
                Ok(mut disk_config) => {
                    config.overlay_dirty(&mut disk_config, dirty);
                    match disk_config.save_to(&path) {
                        Ok(()) => {
                            self.last_identity = Self::identity(&path);
                            log::debug!("Merged settings save onto externally edited config");
                            SaveResult::Saved
                        }
                        Err(e) => {
                            log::error!(error:% = e; "Failed to save merged config");
                            SaveResult::Failed(e.to_string())
                        }
                    }
                }
                Err(_) if force => {
                    log::debug!(
                        "Externally edited config no longer parses; forcing overwrite on close"
                    );
                    self.write_direct(&path, config)
                }
                Err(_) => {
                    log::debug!("Externally edited config no longer parses; deferring save");
                    SaveResult::Deferred(
                        "config.json changed on disk and does not parse".to_string(),
                    )
                }
            },
            Err(e) => {
                log::error!(error:% = e; "Failed to read config for merge");
                SaveResult::Failed(e.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// A unique temp path per test (and per process), so parallel test runs
    /// never collide on the same file.
    fn unique_path(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!("dbh-store-{}-{label}.json", std::process::id()))
    }

    #[test]
    fn merge_preserves_external_edit_of_untouched_field() {
        let path = unique_path("merge-preserve");
        let _ = fs::remove_file(&path);

        let default_config = Config::default();
        fs::write(
            &path,
            serde_json::to_string_pretty(&default_config).expect("serialize default"),
        )
        .expect("write initial config");

        let mut store = WindowsConfigStore::new(Some(path.clone()));
        assert_eq!(
            store.save(&default_config, &SettingsDirty::default(), false),
            SaveResult::Saved,
            "baseline save should write through unchanged"
        );

        // Externally rewrite the file: default config but with a changed
        // field we never touch ourselves.
        let mut external_config = Config::default();
        external_config.refresh.periodic_seconds = 300;
        fs::write(
            &path,
            serde_json::to_string_pretty(&external_config).expect("serialize external"),
        )
        .expect("write external edit");

        let mut ours = Config::default();
        ours.brightness.step_percent = 9;
        let dirty = SettingsDirty {
            step_percent: true,
            ..Default::default()
        };

        let result = store.save(&ours, &dirty, false);
        assert_eq!(result, SaveResult::Saved);

        let contents = fs::read_to_string(&path).expect("read merged config");
        let saved: Config = serde_json::from_str(&contents).expect("parse merged config");
        assert_eq!(saved.brightness.step_percent, 9, "our dirty field applied");
        assert_eq!(
            saved.refresh.periodic_seconds, 300,
            "external edit of an untouched field preserved"
        );

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn unparseable_external_file_defers_then_forces() {
        let path = unique_path("unparseable");
        let _ = fs::remove_file(&path);

        let default_config = Config::default();
        fs::write(
            &path,
            serde_json::to_string_pretty(&default_config).expect("serialize default"),
        )
        .expect("write initial config");

        let mut store = WindowsConfigStore::new(Some(path.clone()));
        assert_eq!(
            store.save(&default_config, &SettingsDirty::default(), false),
            SaveResult::Saved,
            "baseline save should write through unchanged"
        );

        fs::write(&path, "{ not json").expect("write unparseable external edit");

        let mut ours = Config::default();
        ours.brightness.step_percent = 12;
        let dirty = SettingsDirty {
            step_percent: true,
            ..Default::default()
        };

        match store.save(&ours, &dirty, false) {
            SaveResult::Deferred(_) => {}
            other => panic!("expected Deferred, got {other:?}"),
        }
        let contents = fs::read_to_string(&path).expect("read after deferred save");
        assert_eq!(
            contents, "{ not json",
            "deferred save must leave the unparseable file untouched"
        );

        let result = store.save(&ours, &dirty, true);
        assert_eq!(result, SaveResult::Saved, "forced save must overwrite");
        let contents = fs::read_to_string(&path).expect("read after forced save");
        let saved: Config = serde_json::from_str(&contents).expect("forced save must parse");
        assert_eq!(saved.brightness.step_percent, 12);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn unchanged_file_saves_directly() {
        let path = unique_path("unchanged");
        let _ = fs::remove_file(&path);

        let default_config = Config::default();
        fs::write(
            &path,
            serde_json::to_string_pretty(&default_config).expect("serialize default"),
        )
        .expect("write initial config");

        let mut store = WindowsConfigStore::new(Some(path.clone()));

        let mut ours = Config::default();
        ours.brightness.step_percent = 7;
        let dirty = SettingsDirty {
            step_percent: true,
            ..Default::default()
        };

        let result = store.save(&ours, &dirty, false);
        assert_eq!(result, SaveResult::Saved);

        let contents = fs::read_to_string(&path).expect("read after save");
        let expected =
            serde_json::to_string_pretty(&ours).expect("serialize expected direct write");
        assert_eq!(contents, expected);

        let _ = fs::remove_file(&path);
    }
}
