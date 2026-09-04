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
pub(crate) const CONFIG_VERSION: u32 = 1;

/// Default hotkey for brightness up.
pub const DEFAULT_HOTKEY_UP: &str = "Ctrl+Shift+Up";
/// Default hotkey for brightness down.
pub const DEFAULT_HOTKEY_DOWN: &str = "Ctrl+Shift+Down";
/// Default OSD timeout in milliseconds.
pub(crate) const DEFAULT_OSD_TIMEOUT_MS: u32 = 1000;
/// Default OSD opacity (0.0-1.0).
pub(crate) const DEFAULT_OSD_OPACITY: f32 = 1.0;
/// Default brightness step percentage.
pub(crate) const DEFAULT_STEP_PERCENT: u8 = 5;
/// Default periodic refresh interval in seconds (0 = disabled).
pub(crate) const DEFAULT_REFRESH_PERIODIC_SECONDS: u32 = 60;
/// Default inactivity threshold in seconds before refresh (0 = disabled).
pub(crate) const DEFAULT_REFRESH_INACTIVITY_SECONDS: u32 = 30;
/// Default level filter for the rolling log file.
pub(crate) const DEFAULT_FILE_LOG_LEVEL: &str = "info";

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
// Settings Dialog Support
// ─────────────────────────────────────────────────────────────────────────────

/// Tracks which of the ten user-facing settings a settings-dialog session has
/// changed.
///
/// Used to gate saves (nothing to write when [`SettingsDirty::any`] is
/// `false`) and to merge a session's edits onto a `config.json` that may have
/// been hand-edited while the dialog was open, via [`Config::overlay_dirty`]:
/// only the flagged fields overwrite the on-disk value, everything else is
/// left as found on disk.
// Each flag maps 1:1 to one of the ten independently toggleable settings
// fields below; a state machine or paired enums would not fit a set of
// independent booleans that are OR'd and copied field-by-field.
#[expect(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SettingsDirty {
    /// `brightness.step_percent` was changed.
    pub step_percent: bool,
    /// `osd.timeout_ms` was changed.
    pub osd_timeout_ms: bool,
    /// `osd.opacity` was changed.
    pub osd_opacity: bool,
    /// `refresh.periodic_seconds` was changed.
    pub refresh_periodic: bool,
    /// `refresh.inactivity_seconds` was changed.
    pub refresh_inactivity: bool,
    /// `hotkeys.brightness_up` was changed.
    pub hotkey_up: bool,
    /// `hotkeys.brightness_down` was changed.
    pub hotkey_down: bool,
    /// `hotkeys.intercept_brightness_keys` was changed.
    pub intercept: bool,
    /// `logging.file_enabled` was changed.
    pub log_enabled: bool,
    /// `logging.file_level` was changed.
    pub log_level: bool,
}

impl SettingsDirty {
    /// Returns `true` if any tracked field was changed.
    #[must_use]
    pub fn any(&self) -> bool {
        self.step_percent
            || self.osd_timeout_ms
            || self.osd_opacity
            || self.refresh_periodic
            || self.refresh_inactivity
            || self.hotkey_up
            || self.hotkey_down
            || self.intercept
            || self.log_enabled
            || self.log_level
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Configuration Structures
// ─────────────────────────────────────────────────────────────────────────────

/// Root configuration structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Configuration file version. There is no migration logic yet: a
    /// mismatch is logged as a warning, the fields are interpreted as the
    /// current schema, and the value is reset to [`CONFIG_VERSION`].
    #[serde(default = "default_version")]
    pub(crate) version: u32,
    /// Hotkey bindings.
    #[serde(default)]
    pub hotkeys: HotkeyConfig,
    /// Per-monitor settings (reserved for future use).
    #[serde(default)]
    pub(crate) monitors: HashMap<String, MonitorConfig>,
    /// OSD appearance settings.
    #[serde(default)]
    pub osd: OsdConfig,
    /// Brightness adjustment settings.
    #[serde(default)]
    pub brightness: BrightnessConfig,
    /// Refresh/resync settings.
    #[serde(default)]
    pub(crate) refresh: RefreshConfig,
    /// File-logging settings.
    #[serde(default)]
    pub logging: LoggingConfig,
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
pub(crate) struct MonitorConfig {
    /// Minimum brightness limit for this monitor.
    pub(crate) min_brightness: Option<u8>,
    /// Maximum brightness limit for this monitor.
    pub(crate) max_brightness: Option<u8>,
    /// Disable DDC for this monitor.
    pub(crate) ddc_disabled: Option<bool>,
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

/// File-logging configuration.
///
/// Release builds hide the console, so `env_logger`'s stderr output is
/// unreachable there; an opt-in rolling log file in the config directory is
/// the diagnostic artifact users can actually retrieve and attach to reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Write log output to a rolling file in the config directory.
    #[serde(default)]
    pub file_enabled: bool,
    /// Level filter for the file: "error", "warn", "info", "debug" or "trace"
    /// (case-insensitive). Note: at "debug" and below, monitor serial numbers
    /// and absolute paths are included — acceptable for a deliberately created
    /// diagnostic artifact, not a default.
    #[serde(default = "default_file_log_level")]
    pub file_level: String,
}

/// Refresh/resync configuration.
///
/// Controls how the application stays in sync with external brightness changes
/// (e.g., physical monitor buttons, other apps).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct RefreshConfig {
    /// Interval in seconds for periodic background refresh (0 = disabled).
    /// Range: 0-3600 (1 hour max).
    #[serde(default = "default_refresh_periodic")]
    pub(crate) periodic_seconds: u32,
    /// Inactivity threshold in seconds before triggering a refresh on next adjustment.
    /// When the user adjusts brightness after being inactive for this duration,
    /// a refresh is triggered first. (0 = disabled). Range: 0-600 (10 min max).
    #[serde(default = "default_refresh_inactivity")]
    pub(crate) inactivity_seconds: u32,
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
fn default_file_log_level() -> String {
    DEFAULT_FILE_LOG_LEVEL.to_string()
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
            logging: LoggingConfig::default(),
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

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            file_enabled: false,
            file_level: default_file_log_level(),
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
    /// Returns the application data directory.
    ///
    /// Location: `%APPDATA%\BrightnessControl`, resolved via the `APPDATA`
    /// environment variable (always set in a normal Windows logon session).
    /// Returns `None` when the variable is absent — e.g. on non-Windows hosts
    /// running the platform-agnostic tests. Holds the config file and, when
    /// file logging is enabled, the rolling log files.
    #[must_use]
    pub fn default_dir() -> Option<PathBuf> {
        std::env::var_os("APPDATA").map(|appdata| PathBuf::from(appdata).join("BrightnessControl"))
    }

    /// Returns the default configuration file path
    /// (`config.json` inside [`Config::default_dir`]).
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        Self::default_dir().map(|dir| dir.join("config.json"))
    }

    /// Collects dotted paths of keys present in `file` but absent from
    /// `schema`, recursing into nested objects. `schema` is the parsed
    /// config's own serialization, so it contains exactly the known keys —
    /// no hand-maintained key list to drift. (Precondition: no field uses
    /// `skip_serializing_if`, which would false-positive here.)
    fn unknown_keys(file: &serde_json::Value, schema: &serde_json::Value) -> Vec<String> {
        let mut found = Vec::new();
        Self::collect_unknown_keys(file, schema, "", &mut found);
        found
    }

    fn collect_unknown_keys(
        file: &serde_json::Value,
        schema: &serde_json::Value,
        prefix: &str,
        found: &mut Vec<String>,
    ) {
        let (serde_json::Value::Object(file_map), serde_json::Value::Object(schema_map)) =
            (file, schema)
        else {
            return;
        };
        for (key, file_value) in file_map {
            let path = if prefix.is_empty() {
                key.clone()
            } else {
                format!("{prefix}.{key}")
            };
            match schema_map.get(key) {
                None => found.push(path),
                // The monitors map holds user-chosen monitor ids (format not
                // yet a contract); its contents are exempt from the diff — a
                // non-empty map already gets its own load warning.
                Some(_) if prefix.is_empty() && key == "monitors" => {}
                Some(schema_value) => {
                    Self::collect_unknown_keys(file_value, schema_value, &path, found);
                }
            }
        }
    }

    /// Loads configuration from a specific path.
    ///
    /// # Errors
    ///
    /// Returns `ConfigRead` if the file cannot be read, or `ConfigParse` if
    /// the JSON is invalid.
    pub(crate) fn load_from(path: &std::path::Path) -> Result<Self> {
        let path_str = log_safe_file_name(path);

        let contents = std::fs::read_to_string(path)
            .map_err(|e| BrightnessError::config_read(&path_str, e))?;

        let mut config: Self = serde_json::from_str(&contents)
            .map_err(|e| BrightnessError::config_parse(&path_str, e))?;

        // Serde drops unrecognized keys silently while every field has a
        // default, so a typo becomes a setting that silently does nothing.
        // Diff the raw file against the parsed config's serialization and
        // warn — never fatal.
        if let (Ok(raw), Ok(schema)) = (
            serde_json::from_str::<serde_json::Value>(&contents),
            serde_json::to_value(&config),
        ) {
            for key in Self::unknown_keys(&raw, &schema) {
                log::warn!(key:% = key; "Unknown config key ignored — check for typos");
            }
        }

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
        // Version check: no migration logic exists, so all that can be done
        // honestly is warn and interpret the fields as the current schema
        // (unknown fields were already dropped by serde). Resetting the value
        // keeps later writes (backup mirror, save) truthful about what the
        // in-memory config actually is after repair.
        if self.version != CONFIG_VERSION {
            log::warn!(
                found = self.version,
                expected = CONFIG_VERSION;
                "Config version mismatch; no migration performed, fields interpreted as current schema"
            );
            self.version = CONFIG_VERSION;
        }

        // Per-monitor settings deserialize but are not wired up anywhere yet;
        // warn so a user who sets them learns why nothing changes. The
        // entries are preserved (they round-trip through saves).
        if !self.monitors.is_empty() {
            log::warn!(
                entries = self.monitors.len();
                "Per-monitor settings ('monitors') are not yet implemented and have no effect"
            );
        }

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

        // Validate file log level (must parse as a `log` level, case-insensitive)
        if self.logging.file_level.parse::<log::LevelFilter>().is_err() {
            log::error!(
                field = "logging.file_level",
                value:% = self.logging.file_level,
                default = DEFAULT_FILE_LOG_LEVEL;
                "Invalid config value, using default"
            );
            self.logging.file_level = DEFAULT_FILE_LOG_LEVEL.to_string();
        }
    }

    /// Replaces hotkey strings that `is_valid` rejects with the defaults,
    /// logging an error for each — the "invalid config is never fatal"
    /// contract applies to hotkey strings just like to numeric fields.
    ///
    /// Hotkey validity is platform knowledge (the parser and its key-name
    /// table live in the platform layer), so it is injected as a predicate
    /// instead of being implemented here. Callers pass e.g.
    /// `|s| parse_hotkey(s).is_ok()`.
    pub fn repair_hotkeys(&mut self, is_valid: impl Fn(&str) -> bool) {
        if !is_valid(&self.hotkeys.brightness_up) {
            log::error!(
                field = "hotkeys.brightness_up",
                value:% = self.hotkeys.brightness_up,
                default = DEFAULT_HOTKEY_UP;
                "Invalid hotkey string, using default"
            );
            self.hotkeys.brightness_up = DEFAULT_HOTKEY_UP.to_string();
        }

        if !is_valid(&self.hotkeys.brightness_down) {
            log::error!(
                field = "hotkeys.brightness_down",
                value:% = self.hotkeys.brightness_down,
                default = DEFAULT_HOTKEY_DOWN;
                "Invalid hotkey string, using default"
            );
            self.hotkeys.brightness_down = DEFAULT_HOTKEY_DOWN.to_string();
        }
    }

    /// Resets the ten user-facing settings to their defaults, field by field.
    ///
    /// Never swaps in [`Config::default()`] wholesale: `monitors` entries are
    /// a user-visible, hand-editable part of `config.json` and must survive a
    /// restore, and `version` is preserved rather than reset.
    pub(crate) fn restore_defaults(&mut self) {
        self.hotkeys.brightness_up = default_hotkey_up();
        self.hotkeys.brightness_down = default_hotkey_down();
        self.hotkeys.intercept_brightness_keys = false;
        self.osd.timeout_ms = default_osd_timeout();
        self.osd.opacity = default_osd_opacity();
        self.brightness.step_percent = default_step_percent();
        self.refresh.periodic_seconds = default_refresh_periodic();
        self.refresh.inactivity_seconds = default_refresh_inactivity();
        self.logging.file_enabled = false;
        self.logging.file_level = default_file_log_level();
    }

    /// Copies exactly the dirty-flagged fields from `self` onto `disk`,
    /// leaving every other field of `disk` untouched.
    ///
    /// Lets a settings-dialog session apply only the fields it actually
    /// changed onto whatever is currently on disk (which may have been
    /// hand-edited concurrently), instead of overwriting the whole file with
    /// a possibly stale in-memory snapshot.
    pub(crate) fn overlay_dirty(&self, disk: &mut Config, dirty: &SettingsDirty) {
        if dirty.step_percent {
            disk.brightness.step_percent = self.brightness.step_percent;
        }
        if dirty.osd_timeout_ms {
            disk.osd.timeout_ms = self.osd.timeout_ms;
        }
        if dirty.osd_opacity {
            disk.osd.opacity = self.osd.opacity;
        }
        if dirty.refresh_periodic {
            disk.refresh.periodic_seconds = self.refresh.periodic_seconds;
        }
        if dirty.refresh_inactivity {
            disk.refresh.inactivity_seconds = self.refresh.inactivity_seconds;
        }
        if dirty.hotkey_up {
            disk.hotkeys
                .brightness_up
                .clone_from(&self.hotkeys.brightness_up);
        }
        if dirty.hotkey_down {
            disk.hotkeys
                .brightness_down
                .clone_from(&self.hotkeys.brightness_down);
        }
        if dirty.intercept {
            disk.hotkeys.intercept_brightness_keys = self.hotkeys.intercept_brightness_keys;
        }
        if dirty.log_enabled {
            disk.logging.file_enabled = self.logging.file_enabled;
        }
        if dirty.log_level {
            disk.logging.file_level.clone_from(&self.logging.file_level);
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
    fn test_logging_config_defaults() {
        let config = LoggingConfig::default();
        assert!(!config.file_enabled, "file logging is opt-in");
        assert_eq!(config.file_level, DEFAULT_FILE_LOG_LEVEL);
    }

    #[test]
    fn test_logging_section_absent_uses_defaults() {
        let config: Config = serde_json::from_str("{}").unwrap();
        assert!(!config.logging.file_enabled);
        assert_eq!(config.logging.file_level, DEFAULT_FILE_LOG_LEVEL);
    }

    #[test]
    fn test_invalid_file_level_repaired_to_default() {
        let json = r#"{ "logging": { "file_enabled": true, "file_level": "verbose" } }"#;
        let mut config: Config = serde_json::from_str(json).unwrap();
        config.validate_and_fix();

        assert_eq!(config.logging.file_level, DEFAULT_FILE_LOG_LEVEL);
        assert!(config.logging.file_enabled, "the enable flag is untouched");
    }

    #[test]
    fn test_valid_file_level_passes_validation() {
        // Case-insensitive, as parsed by the `log` crate.
        let json = r#"{ "logging": { "file_level": "Debug" } }"#;
        let mut config: Config = serde_json::from_str(json).unwrap();
        config.validate_and_fix();

        assert_eq!(config.logging.file_level, "Debug");
    }

    #[test]
    fn test_version_mismatch_repaired_to_current() {
        let json = r#"{ "version": 999 }"#;
        let mut config: Config = serde_json::from_str(json).unwrap();
        config.validate_and_fix();

        assert_eq!(config.version, CONFIG_VERSION);
    }

    #[test]
    fn test_version_current_passes_validation() {
        let json = format!(r#"{{ "version": {CONFIG_VERSION} }}"#);
        let mut config: Config = serde_json::from_str(&json).unwrap();
        config.validate_and_fix();

        assert_eq!(config.version, CONFIG_VERSION);
    }

    #[test]
    fn test_nonempty_monitors_map_survives_validation() {
        // The field is reserved; entries must round-trip untouched so a
        // user's hand-written settings survive until the feature exists.
        let json = r#"{ "monitors": { "DELL U2722D": { "min_brightness": 10,
            "max_brightness": null, "ddc_disabled": null } } }"#;
        let mut config: Config = serde_json::from_str(json).unwrap();
        config.validate_and_fix();

        assert_eq!(config.monitors.len(), 1);
        assert_eq!(config.monitors["DELL U2722D"].min_brightness, Some(10));
    }

    // ── Unknown-key detection ────────────────────────────────────────────

    fn schema_value() -> serde_json::Value {
        serde_json::to_value(Config::default()).unwrap()
    }

    #[test]
    fn unknown_top_level_key_is_reported() {
        let file: serde_json::Value =
            serde_json::from_str(r#"{ "version": 1, "brightnes": { "step_percent": 10 } }"#)
                .unwrap();
        assert_eq!(Config::unknown_keys(&file, &schema_value()), ["brightnes"]);
    }

    #[test]
    fn unknown_nested_key_is_reported_with_full_path() {
        let file: serde_json::Value =
            serde_json::from_str(r#"{ "hotkeys": { "brightnes_up": "Alt+F1" } }"#).unwrap();
        assert_eq!(
            Config::unknown_keys(&file, &schema_value()),
            ["hotkeys.brightnes_up"]
        );
    }

    #[test]
    fn known_keys_produce_no_reports() {
        let file = serde_json::to_value(Config::default()).unwrap();
        assert!(Config::unknown_keys(&file, &schema_value()).is_empty());
    }

    #[test]
    fn monitors_subtree_is_exempt_from_unknown_key_reports() {
        // Monitor ids are user-chosen and the map's format is not yet a
        // contract; a non-empty map already gets its own load warning.
        let file: serde_json::Value =
            serde_json::from_str(r#"{ "monitors": { "DEL U2722D": { "future_setting": 1 } } }"#)
                .unwrap();
        assert!(Config::unknown_keys(&file, &schema_value()).is_empty());
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
    fn test_repair_hotkeys_replaces_invalid_with_default() {
        let mut config = Config::default();
        config.hotkeys.brightness_up = "Ctrl+Shift+Banana".to_string();
        config.hotkeys.brightness_down = "Alt+Down".to_string();

        // Stand-in for the platform parser: rejects the unknown key name.
        config.repair_hotkeys(|s| !s.contains("Banana"));

        assert_eq!(config.hotkeys.brightness_up, DEFAULT_HOTKEY_UP);
        assert_eq!(config.hotkeys.brightness_down, "Alt+Down");
    }

    #[test]
    fn test_repair_hotkeys_keeps_valid_values() {
        let mut config = Config::default();
        config.hotkeys.brightness_up = "Alt+F1".to_string();
        config.hotkeys.brightness_down = "Alt+F2".to_string();

        config.repair_hotkeys(|_| true);

        assert_eq!(config.hotkeys.brightness_up, "Alt+F1");
        assert_eq!(config.hotkeys.brightness_down, "Alt+F2");
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

    #[test]
    fn restore_defaults_preserves_monitors_and_version() {
        let mut cfg = Config::default();
        cfg.brightness.step_percent = 20;
        cfg.hotkeys.brightness_up = "Ctrl+F1".to_string();
        cfg.monitors.insert(
            "M1".to_string(),
            MonitorConfig {
                min_brightness: Some(10),
                max_brightness: None,
                ddc_disabled: None,
            },
        );

        cfg.restore_defaults();

        assert_eq!(cfg.brightness.step_percent, DEFAULT_STEP_PERCENT);
        assert_eq!(cfg.hotkeys.brightness_up, DEFAULT_HOTKEY_UP);
        assert_eq!(cfg.monitors.len(), 1);
        assert_eq!(cfg.version, CONFIG_VERSION);
    }

    #[test]
    fn overlay_dirty_copies_only_flagged_fields() {
        let mut ours = Config::default();
        ours.brightness.step_percent = 9;
        ours.osd.timeout_ms = 3000;
        let mut disk = Config::default();
        disk.osd.timeout_ms = 7000; // external edit, NOT dirty
        disk.refresh.periodic_seconds = 300; // external edit, NOT dirty

        let dirty = SettingsDirty {
            step_percent: true,
            ..Default::default()
        };
        ours.overlay_dirty(&mut disk, &dirty);

        assert_eq!(disk.brightness.step_percent, 9); // ours won (dirty)
        assert_eq!(disk.osd.timeout_ms, 7000); // external edit survived
        assert_eq!(disk.refresh.periodic_seconds, 300);
    }
}
