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
pub const DEFAULT_OSD_OPACITY: f32 = 0.8;
/// Default brightness step percentage.
pub const DEFAULT_STEP_PERCENT: u8 = 5;

// Validation ranges
const OSD_TIMEOUT_MIN: u32 = 100;
const OSD_TIMEOUT_MAX: u32 = 10000;
const OSD_OPACITY_MIN: f32 = 0.1;
const OSD_OPACITY_MAX: f32 = 1.0;
const STEP_PERCENT_MIN: u8 = 1;
const STEP_PERCENT_MAX: u8 = 50;

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
        }
    }
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            brightness_up: default_hotkey_up(),
            brightness_down: default_hotkey_down(),
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
                "Invalid config: osd.timeout_ms={} outside range {}-{}, using default {}",
                self.osd.timeout_ms,
                OSD_TIMEOUT_MIN,
                OSD_TIMEOUT_MAX,
                DEFAULT_OSD_TIMEOUT_MS
            );
            self.osd.timeout_ms = DEFAULT_OSD_TIMEOUT_MS;
        }

        // Validate OSD opacity
        if self.osd.opacity < OSD_OPACITY_MIN || self.osd.opacity > OSD_OPACITY_MAX {
            log::error!(
                "Invalid config: osd.opacity={} outside range {}-{}, using default {}",
                self.osd.opacity,
                OSD_OPACITY_MIN,
                OSD_OPACITY_MAX,
                DEFAULT_OSD_OPACITY
            );
            self.osd.opacity = DEFAULT_OSD_OPACITY;
        }

        // Validate step percent
        if self.brightness.step_percent < STEP_PERCENT_MIN
            || self.brightness.step_percent > STEP_PERCENT_MAX
        {
            log::error!(
                "Invalid config: brightness.step_percent={} outside range {}-{}, using default {}",
                self.brightness.step_percent,
                STEP_PERCENT_MIN,
                STEP_PERCENT_MAX,
                DEFAULT_STEP_PERCENT
            );
            self.brightness.step_percent = DEFAULT_STEP_PERCENT;
        }
    }
}
