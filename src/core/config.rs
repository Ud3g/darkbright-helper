//! Configuration types and loading for the brightness control tool.
//!
//! Configuration is stored as JSON in `%APPDATA%\BrightnessControl\config.json`.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::{BrightnessError, Result};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Current configuration file version.
pub const CONFIG_VERSION: u32 = 1;

/// Default hotkey for brightness up.
pub const DEFAULT_HOTKEY_UP: &str = "Ctrl+Shift+Up";
/// Default hotkey for brightness down.
pub const DEFAULT_HOTKEY_DOWN: &str = "Ctrl+Shift+Down";
/// Default OSD timeout in milliseconds.
pub const DEFAULT_OSD_TIMEOUT_MS: u32 = 1000;
/// Default OSD opacity (0.0-1.0).
pub const DEFAULT_OSD_OPACITY: f32 = 1.0;
/// Default brightness step percentage.
pub const DEFAULT_STEP_PERCENT: u8 = 5;
/// Default periodic refresh interval in seconds (0 = disabled).
pub const DEFAULT_REFRESH_PERIODIC_SECONDS: u32 = 60;
/// Default inactivity threshold in seconds before refresh (0 = disabled).
pub const DEFAULT_REFRESH_INACTIVITY_SECONDS: u32 = 30;

// Validation ranges
const OSD_TIMEOUT_MIN: u32 = 100;
const OSD_TIMEOUT_MAX: u32 = 10000;
const OSD_OPACITY_MIN: f32 = 0.1;
const OSD_OPACITY_MAX: f32 = 1.0;
const STEP_PERCENT_MIN: u8 = 1;
const STEP_PERCENT_MAX: u8 = 50;
const REFRESH_PERIODIC_MAX: u32 = 3600;
const REFRESH_INACTIVITY_MAX: u32 = 600;

// ─────────────────────────────────────────────────────────────────────────────
// Configuration Structures
// ─────────────────────────────────────────────────────────────────────────────

/// Root configuration structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Configuration file version for migration support.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Hotkey bindings.
    #[serde(default)]
    pub hotkeys: HotkeyConfig,
    /// Per-monitor settings (reserved for future use).
    #[serde(default)]
    pub monitors: HashMap<String, MonitorConfig>,
    /// OSD appearance settings.
    #[serde(default)]
    pub osd: OsdConfig,
    /// Brightness adjustment settings.
    #[serde(default)]
    pub brightness: BrightnessConfig,
    /// Refresh/resync settings.
    #[serde(default)]
    pub refresh: RefreshConfig,
}

/// Hotkey configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HotkeyConfig {
    /// Hotkey string for brightness up (e.g., "Ctrl+Shift+Up").
    #[serde(default = "default_hotkey_up")]
    pub brightness_up: String,
    /// Hotkey string for brightness down (e.g., "Ctrl+Shift+Down").
    #[serde(default = "default_hotkey_down")]
    pub brightness_down: String,
    /// Enable low-level keyboard hook to intercept dedicated brightness keys.
    ///
    /// When enabled, the application installs a `WH_KEYBOARD_LL` hook to capture
    /// `VK_BRIGHTNESS_UP` and `VK_BRIGHTNESS_DOWN` keys before the Windows Shell
    /// handles them, suppressing the native brightness OSD.
    ///
    /// **Note:** Some antivirus software may flag low-level keyboard hooks as
    /// suspicious behavior. Disabled by default to avoid false positives.
    #[serde(default)]
    pub intercept_brightness_keys: bool,
}

/// Per-monitor configuration (reserved for future use).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MonitorConfig {
    /// Minimum brightness limit for this monitor.
    pub min_brightness: Option<u8>,
    /// Maximum brightness limit for this monitor.
    pub max_brightness: Option<u8>,
    /// Disable DDC for this monitor.
    pub ddc_disabled: Option<bool>,
}

/// OSD appearance configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OsdConfig {
    /// How long the OSD stays visible after the last keypress (ms).
    #[serde(default = "default_osd_timeout")]
    pub timeout_ms: u32,
    /// OSD window opacity (0.1-1.0).
    #[serde(default = "default_osd_opacity")]
    pub opacity: f32,
}

/// Brightness adjustment configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrightnessConfig {
    /// Brightness change per keypress (1-50%).
    #[serde(default = "default_step_percent")]
    pub step_percent: u8,
}

/// Refresh/resync configuration.
///
/// Controls how the application stays in sync with external brightness changes
/// (e.g., physical monitor buttons, other apps).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshConfig {
    /// Interval in seconds for periodic background refresh (0 = disabled).
    /// Range: 0-3600 (1 hour max).
    #[serde(default = "default_refresh_periodic")]
    pub periodic_seconds: u32,
    /// Inactivity threshold in seconds before triggering a refresh on next adjustment.
    /// When the user adjusts brightness after being inactive for this duration,
    /// a refresh is triggered first. (0 = disabled). Range: 0-600 (10 min max).
    #[serde(default = "default_refresh_inactivity")]
    pub inactivity_seconds: u32,
}

// ─────────────────────────────────────────────────────────────────────────────
// Default Value Functions (for serde)
// ─────────────────────────────────────────────────────────────────────────────

fn default_version() -> u32 {
    CONFIG_VERSION
}
fn default_hotkey_up() -> String {
    DEFAULT_HOTKEY_UP.to_string()
}
fn default_hotkey_down() -> String {
    DEFAULT_HOTKEY_DOWN.to_string()
}
fn default_osd_timeout() -> u32 {
    DEFAULT_OSD_TIMEOUT_MS
}
fn default_osd_opacity() -> f32 {
    DEFAULT_OSD_OPACITY
}
fn default_step_percent() -> u8 {
    DEFAULT_STEP_PERCENT
}
fn default_refresh_periodic() -> u32 {
    DEFAULT_REFRESH_PERIODIC_SECONDS
}
fn default_refresh_inactivity() -> u32 {
    DEFAULT_REFRESH_INACTIVITY_SECONDS
}

// ─────────────────────────────────────────────────────────────────────────────
// Trait Implementations
// ─────────────────────────────────────────────────────────────────────────────

impl Default for Config {
    fn default() -> Self {
        Self {
            version: CONFIG_VERSION,
            hotkeys: HotkeyConfig::default(),
            monitors: HashMap::new(),
            osd: OsdConfig::default(),
            brightness: BrightnessConfig::default(),
            refresh: RefreshConfig::default(),
        }
    }
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            brightness_up: default_hotkey_up(),
            brightness_down: default_hotkey_down(),
            intercept_brightness_keys: false,
        }
    }
}

impl Default for OsdConfig {
    fn default() -> Self {
        Self {
            timeout_ms: default_osd_timeout(),
            opacity: default_osd_opacity(),
        }
    }
}

impl Default for BrightnessConfig {
    fn default() -> Self {
        Self {
            step_percent: default_step_percent(),
        }
    }
}

impl Default for RefreshConfig {
    fn default() -> Self {
        Self {
            periodic_seconds: default_refresh_periodic(),
            inactivity_seconds: default_refresh_inactivity(),
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Configuration Loading and Saving
// ─────────────────────────────────────────────────────────────────────────────

/// Returns just the file name of `path` for embedding in errors.
///
/// Config errors end up in warn/error logs; parent directories are omitted
/// because absolute config paths contain the Windows user name (PII). The
/// config location is fixed and documented, so the file name alone is enough
/// to identify the file.
fn log_safe_file_name(path: &std::path::Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// How [`Config::load_or_recover`] obtained its result.
///
/// Returned alongside the config so the caller can log or surface the
/// outcome; the recovery decision itself stays in testable core code.
#[derive(Debug)]
pub enum ConfigLoadOutcome {
    /// The primary config file parsed successfully.
    Loaded,
    /// The primary file was unreadable or corrupt; settings were recovered
    /// from the `.bak` sibling written after the last successful load.
    RecoveredFromBackup {
        /// Why the primary file could not be used.
        primary_error: BrightnessError,
    },
    /// Neither the primary file nor a backup was usable; defaults were
    /// substituted.
    DefaultsSubstituted {
        /// Why the primary file could not be used.
        primary_error: BrightnessError,
        /// Why the backup could not be used, or `None` if none existed.
        backup_error: Option<BrightnessError>,
    },
}

impl Config {
    /// Returns the default configuration file path.
    ///
    /// Location: `%APPDATA%\BrightnessControl\config.json`, resolved via the
    /// `APPDATA` environment variable (always set in a normal Windows logon
    /// session). Returns `None` when the variable is absent — e.g. on
    /// non-Windows hosts running the platform-agnostic tests.
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|appdata| {
            PathBuf::from(appdata)
                .join("BrightnessControl")
                .join("config.json")
        })
    }

    /// Loads configuration from the default path, or returns defaults if not found.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be read or parsed.
    pub fn load() -> Result<Self> {
        match Self::default_path() {
            Some(path) if path.exists() => Self::load_from(&path),
            _ => Ok(Self::default()),
        }
    }

    /// Loads configuration from a specific path.
    ///
    /// # Errors
    ///
    /// Returns `ConfigRead` if the file cannot be read, or `ConfigParse` if
    /// the JSON is invalid.
    pub fn load_from(path: &std::path::Path) -> Result<Self> {
        let path_str = log_safe_file_name(path);

        let contents = std::fs::read_to_string(path)
            .map_err(|e| BrightnessError::config_read(&path_str, e))?;

        let mut config: Self = serde_json::from_str(&contents)
            .map_err(|e| BrightnessError::config_parse(&path_str, e))?;

        config.validate_and_fix();
        Ok(config)
    }

    /// Loads configuration from `path`, recovering from the `.bak` sibling
    /// when the primary file is unreadable or corrupt.
    ///
    /// On a successful load the backup is refreshed (best-effort) so it
    /// always holds the last settings that parsed successfully — regardless
    /// of whether the primary file was last written by the app or edited by
    /// hand. On failure the corrupt primary file is left untouched for
    /// inspection. Never fails: when neither file is usable, defaults are
    /// returned, per the "invalid config is never fatal" contract.
    ///
    /// # Panics
    ///
    /// Panics if JSON serialization fails while refreshing the backup.
    #[must_use]
    pub fn load_or_recover(path: &std::path::Path) -> (Self, ConfigLoadOutcome) {
        match Self::load_from(path) {
            Ok(config) => {
                config.refresh_backup(path);
                (config, ConfigLoadOutcome::Loaded)
            }
            Err(primary_error) => {
                let backup = Self::backup_path(path);
                if backup.exists() {
                    match Self::load_from(&backup) {
                        Ok(config) => (
                            config,
                            ConfigLoadOutcome::RecoveredFromBackup { primary_error },
                        ),
                        Err(backup_error) => (
                            Self::default(),
                            ConfigLoadOutcome::DefaultsSubstituted {
                                primary_error,
                                backup_error: Some(backup_error),
                            },
                        ),
                    }
                } else {
                    (
                        Self::default(),
                        ConfigLoadOutcome::DefaultsSubstituted {
                            primary_error,
                            backup_error: None,
                        },
                    )
                }
            }
        }
    }

    /// Returns the backup sibling of `path` (`config.json` → `config.json.bak`).
    fn backup_path(path: &std::path::Path) -> PathBuf {
        Self::sibling_with_suffix(path, ".bak")
    }

    /// Returns `path` with `suffix` appended to its file name
    /// (`config.json` + `.tmp` → `config.json.tmp`).
    fn sibling_with_suffix(path: &std::path::Path, suffix: &str) -> PathBuf {
        let mut name = path.file_name().map_or_else(
            || std::ffi::OsString::from("config.json"),
            std::ffi::OsStr::to_os_string,
        );
        name.push(suffix);
        path.with_file_name(name)
    }

    /// Atomically replaces `path` with `contents`: writes a `.tmp` sibling,
    /// then renames it over the target. The rename is atomic on a single
    /// volume, so a crash, power loss, or full disk mid-write can never leave
    /// a truncated file at `path` — the old content survives instead.
    fn write_atomically(path: &std::path::Path, contents: &str) -> std::io::Result<()> {
        let tmp = Self::sibling_with_suffix(path, ".tmp");
        std::fs::write(&tmp, contents)?;
        std::fs::rename(&tmp, path).inspect_err(|_| {
            // Don't leave an orphaned temp file behind on a failed rename.
            let _ = std::fs::remove_file(&tmp);
        })
    }

    /// Best-effort refresh of the backup file with this config's contents.
    /// A failure is logged and swallowed — backup maintenance must never
    /// break startup.
    fn refresh_backup(&self, path: &std::path::Path) {
        let backup = Self::backup_path(path);
        let contents =
            serde_json::to_string_pretty(self).expect("Config serialization should never fail");
        if let Err(e) = Self::write_atomically(&backup, &contents) {
            log::warn!(error:% = e; "Failed to refresh config backup");
        }
    }

    /// Saves configuration to the default path.
    ///
    /// Creates the parent directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns `ConfigWrite` if the file cannot be written.
    pub fn save(&self) -> Result<()> {
        match Self::default_path() {
            Some(path) => self.save_to(&path),
            None => Err(BrightnessError::ConfigWrite {
                path: "unknown".to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    "Could not determine config directory",
                ),
            }),
        }
    }

    /// Saves configuration to a specific path.
    ///
    /// The write is atomic (temp file + rename), so an interrupted save
    /// leaves the previous file intact rather than a truncated one.
    ///
    /// # Errors
    ///
    /// Returns `ConfigWrite` if the file cannot be written.
    ///
    /// # Panics
    ///
    /// Panics if JSON serialization fails.
    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        let path_str = log_safe_file_name(path);

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| BrightnessError::config_write(&path_str, e))?;
        }

        let contents =
            serde_json::to_string_pretty(self).expect("Config serialization should never fail");

        Self::write_atomically(path, &contents)
            .map_err(|e| BrightnessError::config_write(&path_str, e))
    }

    /// Validates configuration values and replaces invalid ones with defaults.
    ///
    /// Logs errors for each invalid value found.
    fn validate_and_fix(&mut self) {
        // Validate OSD timeout
        if self.osd.timeout_ms < OSD_TIMEOUT_MIN || self.osd.timeout_ms > OSD_TIMEOUT_MAX {
            log::error!(
                field = "osd.timeout_ms",
                value = self.osd.timeout_ms,
                min = OSD_TIMEOUT_MIN,
                max = OSD_TIMEOUT_MAX,
                default = DEFAULT_OSD_TIMEOUT_MS;
                "Invalid config value, using default"
            );
            self.osd.timeout_ms = DEFAULT_OSD_TIMEOUT_MS;
        }

        // Validate OSD opacity
        if self.osd.opacity < OSD_OPACITY_MIN || self.osd.opacity > OSD_OPACITY_MAX {
            log::error!(
                field = "osd.opacity",
                value = self.osd.opacity,
                min = OSD_OPACITY_MIN,
                max = OSD_OPACITY_MAX,
                default = DEFAULT_OSD_OPACITY;
                "Invalid config value, using default"
            );
            self.osd.opacity = DEFAULT_OSD_OPACITY;
        }

        // Validate step percent
        if self.brightness.step_percent < STEP_PERCENT_MIN
            || self.brightness.step_percent > STEP_PERCENT_MAX
        {
            log::error!(
                field = "brightness.step_percent",
                value = self.brightness.step_percent,
                min = STEP_PERCENT_MIN,
                max = STEP_PERCENT_MAX,
                default = DEFAULT_STEP_PERCENT;
                "Invalid config value, using default"
            );
            self.brightness.step_percent = DEFAULT_STEP_PERCENT;
        }

        // Validate periodic refresh (0 is valid = disabled)
        if self.refresh.periodic_seconds > REFRESH_PERIODIC_MAX {
            log::error!(
                field = "refresh.periodic_seconds",
                value = self.refresh.periodic_seconds,
                max = REFRESH_PERIODIC_MAX,
                default = DEFAULT_REFRESH_PERIODIC_SECONDS;
                "Invalid config value exceeds maximum, using default"
            );
            self.refresh.periodic_seconds = DEFAULT_REFRESH_PERIODIC_SECONDS;
        }

        // Validate inactivity refresh (0 is valid = disabled)
        if self.refresh.inactivity_seconds > REFRESH_INACTIVITY_MAX {
            log::error!(
                field = "refresh.inactivity_seconds",
                value = self.refresh.inactivity_seconds,
                max = REFRESH_INACTIVITY_MAX,
                default = DEFAULT_REFRESH_INACTIVITY_SECONDS;
                "Invalid config value exceeds maximum, using default"
            );
            self.refresh.inactivity_seconds = DEFAULT_REFRESH_INACTIVITY_SECONDS;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_refresh_config_defaults() {
        let config = RefreshConfig::default();
        assert_eq!(config.periodic_seconds, DEFAULT_REFRESH_PERIODIC_SECONDS);
        assert_eq!(
            config.inactivity_seconds,
            DEFAULT_REFRESH_INACTIVITY_SECONDS
        );
    }

    #[test]
    fn test_refresh_config_zero_is_valid() {
        // 0 means disabled, should not be changed by validation
        let json = r#"{
            "refresh": {
                "periodic_seconds": 0,
                "inactivity_seconds": 0
            }
        }"#;

        let mut config: Config = serde_json::from_str(json).unwrap();
        config.validate_and_fix();

        assert_eq!(config.refresh.periodic_seconds, 0);
        assert_eq!(config.refresh.inactivity_seconds, 0);
    }

    #[test]
    fn test_refresh_config_valid_values() {
        let json = r#"{
            "refresh": {
                "periodic_seconds": 120,
                "inactivity_seconds": 60
            }
        }"#;

        let mut config: Config = serde_json::from_str(json).unwrap();
        config.validate_and_fix();

        assert_eq!(config.refresh.periodic_seconds, 120);
        assert_eq!(config.refresh.inactivity_seconds, 60);
    }

    #[test]
    fn test_refresh_config_periodic_exceeds_max() {
        let json = r#"{
            "refresh": {
                "periodic_seconds": 9999
            }
        }"#;

        let mut config: Config = serde_json::from_str(json).unwrap();
        config.validate_and_fix();

        // Should be reset to default when exceeding max (3600)
        assert_eq!(
            config.refresh.periodic_seconds,
            DEFAULT_REFRESH_PERIODIC_SECONDS
        );
    }

    #[test]
    fn test_refresh_config_inactivity_exceeds_max() {
        let json = r#"{
            "refresh": {
                "inactivity_seconds": 9999
            }
        }"#;

        let mut config: Config = serde_json::from_str(json).unwrap();
        config.validate_and_fix();

        // Should be reset to default when exceeding max (600)
        assert_eq!(
            config.refresh.inactivity_seconds,
            DEFAULT_REFRESH_INACTIVITY_SECONDS
        );
    }

    #[test]
    fn test_refresh_config_at_max_boundary() {
        let json = r#"{
            "refresh": {
                "periodic_seconds": 3600,
                "inactivity_seconds": 600
            }
        }"#;

        let mut config: Config = serde_json::from_str(json).unwrap();
        config.validate_and_fix();

        // Max values should be accepted
        assert_eq!(config.refresh.periodic_seconds, 3600);
        assert_eq!(config.refresh.inactivity_seconds, 600);
    }

    #[test]
    fn test_refresh_config_missing_uses_defaults() {
        // Config without refresh section should use defaults
        let json = r#"{
            "version": 1
        }"#;

        let config: Config = serde_json::from_str(json).unwrap();

        assert_eq!(
            config.refresh.periodic_seconds,
            DEFAULT_REFRESH_PERIODIC_SECONDS
        );
        assert_eq!(
            config.refresh.inactivity_seconds,
            DEFAULT_REFRESH_INACTIVITY_SECONDS
        );
    }

    #[test]
    fn test_save_and_load_config() {
        // Use a subdirectory to verify that save_to() creates missing directories
        let test_dir = std::env::temp_dir().join("darkbright_test_dir");
        let file_path = test_dir.join("config.json");

        // Ensure cleanup from previous runs
        if test_dir.exists() {
            let _ = fs::remove_dir_all(&test_dir);
        }

        let mut config = Config::default();
        config.osd.opacity = 0.5;
        config.hotkeys.brightness_up = "Alt+Up".to_string();
        config.brightness.step_percent = 10;

        // Test saving
        assert!(config.save_to(&file_path).is_ok());
        assert!(file_path.exists());

        // Test loading
        let loaded_config = Config::load_from(&file_path).expect("Failed to load config");

        assert!((loaded_config.osd.opacity - 0.5).abs() < f32::EPSILON);
        assert_eq!(loaded_config.hotkeys.brightness_up, "Alt+Up");
        assert_eq!(loaded_config.brightness.step_percent, 10);

        // Cleanup
        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn test_save_to_replaces_existing_file_and_leaves_no_tmp_residue() {
        let test_dir = std::env::temp_dir().join("darkbright_test_atomic_save");
        let _ = fs::remove_dir_all(&test_dir);
        let config_path = test_dir.join("config.json");

        let mut config = Config::default();
        config.brightness.step_percent = 7;
        config.save_to(&config_path).expect("first save");
        config.brightness.step_percent = 13;
        config.save_to(&config_path).expect("second save");

        let loaded = Config::load_from(&config_path).expect("load after overwrite");
        assert_eq!(loaded.brightness.step_percent, 13);
        assert!(
            !test_dir.join("config.json.tmp").exists(),
            "temp file must be consumed by the rename"
        );

        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn test_save_to_failed_rename_cleans_up_tmp() {
        let test_dir = std::env::temp_dir().join("darkbright_test_atomic_save_fail");
        let _ = fs::remove_dir_all(&test_dir);
        // A non-empty directory at the target path makes the final rename
        // fail, exercising the error path after the temp file was written.
        let config_path = test_dir.join("config.json");
        fs::create_dir_all(config_path.join("occupied")).expect("create blocking dir");

        let result = Config::default().save_to(&config_path);

        assert!(result.is_err());
        assert!(
            !test_dir.join("config.json.tmp").exists(),
            "failed save must not leave an orphaned temp file"
        );

        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn test_config_errors_omit_parent_directories() {
        let test_dir = std::env::temp_dir().join("darkbright_test_err_no_path");
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir).expect("create test dir");
        let config_path = test_dir.join("config.json");
        fs::write(&config_path, "{ not json").expect("write corrupt");

        let err = Config::load_from(&config_path).expect_err("parse must fail");
        let msg = err.to_string();

        assert!(
            msg.contains("config.json"),
            "error should name the file: {msg}"
        );
        assert!(
            !msg.contains("darkbright_test_err_no_path"),
            "error must not embed parent directories (they can contain the user name): {msg}"
        );

        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn test_load_or_recover_prefers_backup_when_primary_corrupt() {
        let test_dir = std::env::temp_dir().join("darkbright_test_recover_from_backup");
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir).expect("create test dir");
        let config_path = test_dir.join("config.json");

        let mut backup = Config::default();
        backup.brightness.step_percent = 17;
        fs::write(
            test_dir.join("config.json.bak"),
            serde_json::to_string_pretty(&backup).expect("serialize"),
        )
        .expect("write backup");
        fs::write(&config_path, "{ this is not json").expect("write corrupt primary");

        let (loaded, outcome) = Config::load_or_recover(&config_path);

        assert_eq!(loaded.brightness.step_percent, 17);
        assert!(matches!(
            outcome,
            ConfigLoadOutcome::RecoveredFromBackup { .. }
        ));

        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn test_load_or_recover_defaults_when_primary_corrupt_and_no_backup() {
        let test_dir = std::env::temp_dir().join("darkbright_test_recover_no_backup");
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir).expect("create test dir");
        let config_path = test_dir.join("config.json");

        fs::write(&config_path, "{ this is not json").expect("write corrupt primary");

        let (loaded, outcome) = Config::load_or_recover(&config_path);

        assert_eq!(
            loaded.brightness.step_percent,
            Config::default().brightness.step_percent
        );
        assert!(matches!(
            outcome,
            ConfigLoadOutcome::DefaultsSubstituted {
                backup_error: None,
                ..
            }
        ));

        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn test_load_or_recover_defaults_when_both_corrupt() {
        let test_dir = std::env::temp_dir().join("darkbright_test_recover_both_corrupt");
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir).expect("create test dir");
        let config_path = test_dir.join("config.json");

        fs::write(&config_path, "{ this is not json").expect("write corrupt primary");
        fs::write(test_dir.join("config.json.bak"), "also garbage").expect("write corrupt backup");

        let (loaded, outcome) = Config::load_or_recover(&config_path);

        assert_eq!(
            loaded.brightness.step_percent,
            Config::default().brightness.step_percent
        );
        assert!(matches!(
            outcome,
            ConfigLoadOutcome::DefaultsSubstituted {
                backup_error: Some(_),
                ..
            }
        ));

        let _ = fs::remove_dir_all(test_dir);
    }

    #[test]
    fn test_load_or_recover_refreshes_backup_after_successful_load() {
        let test_dir = std::env::temp_dir().join("darkbright_test_backup_refresh");
        let _ = fs::remove_dir_all(&test_dir);
        fs::create_dir_all(&test_dir).expect("create test dir");
        let config_path = test_dir.join("config.json");

        let mut config = Config::default();
        config.brightness.step_percent = 9;
        fs::write(
            &config_path,
            serde_json::to_string_pretty(&config).expect("serialize"),
        )
        .expect("write primary");

        let (loaded, outcome) = Config::load_or_recover(&config_path);

        assert_eq!(loaded.brightness.step_percent, 9);
        assert!(matches!(outcome, ConfigLoadOutcome::Loaded));

        // The backup must now hold the successfully parsed settings, so a
        // later corruption of the primary file can be recovered from it.
        let backup = Config::load_from(&test_dir.join("config.json.bak"))
            .expect("backup should exist and parse after a successful load");
        assert_eq!(backup.brightness.step_percent, 9);

        let _ = fs::remove_dir_all(test_dir);
    }
}
