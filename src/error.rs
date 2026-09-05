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

    // ── Hotkey Errors ────────────────────────────────────────────────────
    /// Failed to register a global hotkey.
    #[error("Failed to register hotkey '{hotkey}': {message}")]
    HotkeyRegistration { hotkey: String, message: String },

    // ── Tray Icon Errors ─────────────────────────────────────────────────
    /// Failed to create or register the system tray icon.
    #[error("Failed to create tray icon: {0}")]
    TrayIconCreation(String),

    // ── Configuration Errors ─────────────────────────────────────────────
    /// Failed to read the configuration file.
    #[error("Failed to read config file '{file}': {source}")]
    ConfigRead {
        file: String,
        source: std::io::Error,
    },

    /// Failed to open the configuration file with the system default application.
    #[error("Failed to open config file '{file}': {source}")]
    ConfigFileOpen {
        file: String,
        source: std::io::Error,
    },

    /// Failed to write the configuration file.
    #[error("Failed to write config file '{file}': {source}")]
    ConfigWrite {
        file: String,
        source: std::io::Error,
    },

    /// Failed to parse the configuration file.
    #[error("Failed to parse config file '{file}': {source}")]
    ConfigParse {
        file: String,
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

    /// The operating system refused to start a thread.
    ///
    /// `name` is a static thread name, so this variant needs no constructor
    /// helper and stays buildable directly by the binary's thread wiring.
    #[error("Failed to spawn the {name} thread: {source}")]
    ThreadSpawn {
        name: &'static str,
        source: std::io::Error,
    },

    /// The hotkey thread did not report its registration result in time.
    #[error("Hotkey thread did not report registration in time")]
    HotkeyThreadUnresponsive,
}

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
    pub(crate) fn hotkey_registration(
        hotkey: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
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
    ///
    /// `file` is a bare file name, never a full path: this message reaches `warn!`
    /// and `error!` logs, and a path under the user's profile would carry their name.
    pub(crate) fn config_read(file: impl Into<String>, source: std::io::Error) -> Self {
        Self::ConfigRead {
            file: file.into(),
            source,
        }
    }

    /// Creates a new config file open error.
    ///
    /// `file` is a bare file name, never a full path: this message reaches `warn!`
    /// and `error!` logs, and a path under the user's profile would carry their name.
    pub fn config_file_open(file: impl Into<String>, source: std::io::Error) -> Self {
        Self::ConfigFileOpen {
            file: file.into(),
            source,
        }
    }

    /// Creates a new config write error.
    ///
    /// `file` is a bare file name, never a full path: this message reaches `warn!`
    /// and `error!` logs, and a path under the user's profile would carry their name.
    pub(crate) fn config_write(file: impl Into<String>, source: std::io::Error) -> Self {
        Self::ConfigWrite {
            file: file.into(),
            source,
        }
    }

    /// Creates a new config parse error.
    ///
    /// `file` is a bare file name, never a full path: this message reaches `warn!`
    /// and `error!` logs, and a path under the user's profile would carry their name.
    pub(crate) fn config_parse(file: impl Into<String>, source: serde_json::Error) -> Self {
        Self::ConfigParse {
            file: file.into(),
            source,
        }
    }

    /// Creates a new invalid config error.
    pub(crate) fn config_invalid(field: impl Into<String>, message: impl Into<String>) -> Self {
        Self::ConfigInvalid {
            field: field.into(),
            message: message.into(),
        }
    }
}
