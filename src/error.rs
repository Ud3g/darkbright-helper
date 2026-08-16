//! Centralized error types for the brightness control application.

use thiserror::Error;

/// Type alias for Results using the application's error type.
pub type Result<T> = std::result::Result<T, BrightnessError>;

/// Unified error type for all application errors.
#[derive(Debug, Error)]
pub enum BrightnessError {
    // ── DDC/CI Errors ────────────────────────────────────────────────────
    /// DDC/CI communication failed after all retries.
    #[error("DDC communication failed for monitor '{monitor}': {message}")]
    DdcCommunication { monitor: String, message: String },

    /// Monitor not found or disconnected.
    #[error("Monitor not found: {0}")]
    MonitorNotFound(String),

    /// DDC/CI is not supported by the monitor.
    #[error("Monitor '{0}' does not support DDC/CI")]
    DdcNotSupported(String),

    // ── Hotkey Errors ────────────────────────────────────────────────────
    /// Failed to register a global hotkey.
    #[error("Failed to register hotkey '{hotkey}': {message}")]
    HotkeyRegistration { hotkey: String, message: String },

    /// Hotkey is already registered by another application.
    #[error("Hotkey '{0}' is already in use by another application")]
    HotkeyAlreadyRegistered(String),

    // ── Overlay/OSD Errors ───────────────────────────────────────────────
    /// Failed to create the dimming overlay window.
    #[error("Failed to create overlay window: {0}")]
    OverlayCreation(String),

    /// Failed to create or update the OSD window.
    #[error("Failed to create OSD window: {0}")]
    OsdCreation(String),

    // ── Tray Icon Errors ─────────────────────────────────────────────────
    /// Failed to create or register the system tray icon.
    #[error("Failed to create tray icon: {0}")]
    TrayIconCreation(String),

    // ── Configuration Errors ─────────────────────────────────────────────
    /// Failed to read the configuration file.
    #[error("Failed to read config file '{path}': {source}")]
    ConfigRead {
        path: String,
        source: std::io::Error,
    },

    /// Failed to open the configuration file with the system default application.
    #[error("Failed to open config file '{path}': {source}")]
    ConfigFileOpen {
        path: String,
        source: std::io::Error,
    },

    /// Failed to write the configuration file.
    #[error("Failed to write config file '{path}': {source}")]
    ConfigWrite {
        path: String,
        source: std::io::Error,
    },

    /// Failed to parse the configuration file.
    #[error("Failed to parse config file '{path}': {source}")]
    ConfigParse {
        path: String,
        source: serde_json::Error,
    },

    /// Invalid configuration value.
    #[error("Invalid config value for '{field}': {message}")]
    ConfigInvalid { field: String, message: String },

    // ── Platform/Windows Errors ──────────────────────────────────────────
    /// A Windows API call failed.
    #[error("Windows API error in {function}: error code {error_code}")]
    WindowsApi { function: String, error_code: u32 },

    // ── Channel/Threading Errors ─────────────────────────────────────────
    /// Failed to send a message through a channel.
    #[error("Failed to send message: channel closed")]
    ChannelSend,

    /// Failed to receive a message from a channel.
    #[error("Failed to receive message: channel closed")]
    ChannelRecv,

    /// The hotkey thread did not report its registration result in time.
    #[error("Hotkey thread did not report registration in time")]
    HotkeyThreadUnresponsive,
}

// ─────────────────────────────────────────────────────────────────────────────
// Helper Methods for Error Construction
// ─────────────────────────────────────────────────────────────────────────────

impl BrightnessError {
    /// Creates a new DDC communication error.
    pub(crate) fn ddc_communication(
        monitor: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::DdcCommunication {
            monitor: monitor.into(),
            message: message.into(),
        }
    }

    /// Creates a new hotkey registration error.
    pub fn hotkey_registration(hotkey: impl Into<String>, message: impl Into<String>) -> Self {
        Self::HotkeyRegistration {
            hotkey: hotkey.into(),
            message: message.into(),
        }
    }

    /// Creates a new Windows API error with the given function name and error code.
    pub(crate) fn windows_api(function: impl Into<String>, error_code: u32) -> Self {
        Self::WindowsApi {
            function: function.into(),
            error_code,
        }
    }

    /// Creates a new tray icon creation error.
    pub(crate) fn tray_icon_creation(message: impl Into<String>) -> Self {
        Self::TrayIconCreation(message.into())
    }

    /// Creates a new config read error.
    pub(crate) fn config_read(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::ConfigRead {
            path: path.into(),
            source,
        }
    }

    /// Creates a new config file open error.
    pub fn config_file_open(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::ConfigFileOpen {
            path: path.into(),
            source,
        }
    }

    /// Creates a new config write error.
    pub(crate) fn config_write(path: impl Into<String>, source: std::io::Error) -> Self {
        Self::ConfigWrite {
            path: path.into(),
            source,
        }
    }

    /// Creates a new config parse error.
    pub(crate) fn config_parse(path: impl Into<String>, source: serde_json::Error) -> Self {
        Self::ConfigParse {
            path: path.into(),
            source,
        }
    }

    /// Creates a new invalid config error.
    pub fn config_invalid(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ConfigInvalid {
            field: field.into(),
            message: message.into(),
        }
    }
}
