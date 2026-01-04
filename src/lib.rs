//! Brightness Control Tool library.
//!
//! A hotkey-driven brightness adjustment tool for Windows that provides:
//!
//! - **DDC/CI hardware brightness control** via monitor communication
//! - **Software dimming overlay** for sub-minimum brightness levels
//! - **Global hotkey support** for keyboard-driven adjustments
//! - **Multi-monitor support** with mouse-position-based targeting
//!
//! # Architecture
//!
//! The crate is organized into three main modules:
//!
//! - [`core`] - Platform-agnostic business logic (brightness calculations, config, state)
//! - [`error`] - Centralized error types
//! - [`platform`] - Platform-specific implementations (Windows, future Linux)
//!
//! # Example
//!
//! ```no_run
//! use darkbright_helper::Result;
//!
//! fn main() -> Result<()> {
//!     // Application initialization will go here
//!     Ok(())
//! }
//! ```

pub mod core;
pub mod error;
pub mod platform;

// ─────────────────────────────────────────────────────────────────────────────
// Public API Re-exports
// ─────────────────────────────────────────────────────────────────────────────

// Error handling
pub use error::{BrightnessError, Result};

// Core types
pub use core::brightness::{BrightnessAdjustment, calculate_adjustment};
pub use core::config::Config;
pub use core::state::{
    BrightnessMessage, MonitorId, MonitorState, TrayMenuData, TrayMonitorInfo,
};

// Platform traits
pub use platform::DimmingOverlay;

// Platform-specific types (Windows)
#[cfg(windows)]
pub use platform::windows::TrayIcon;
