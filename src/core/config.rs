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

impl Config {
    /// Returns the default configuration file path.
    ///
    /// Location: `%APPDATA%\BrightnessControl\config.json`
    #[must_use]
    pub fn default_path() -> Option<PathBuf> {
        dirs_next::config_dir().map(|p| p.join("BrightnessControl").join("config.json"))
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
        let path_str = path.display().to_string();

        let contents = std::fs::read_to_string(path)
            .map_err(|e| BrightnessError::config_read(&path_str, e))?;

        let mut config: Self = serde_json::from_str(&contents)
            .map_err(|e| BrightnessError::config_parse(&path_str, e))?;

        config.validate_and_fix();
        Ok(config)
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
    /// # Errors
    ///
    /// Returns `ConfigWrite` if the file cannot be written.
    ///
    /// # Panics
    ///
    /// Panics if JSON serialization fails.
    pub fn save_to(&self, path: &std::path::Path) -> Result<()> {
        let path_str = path.display().to_string();

        // Create parent directory if needed
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| BrightnessError::config_write(&path_str, e))?;
        }

        let contents =
            serde_json::to_string_pretty(self).expect("Config serialization should never fail");

        std::fs::write(path, contents).map_err(|e| BrightnessError::config_write(&path_str, e))
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
}
