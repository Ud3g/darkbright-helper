//! Platform-agnostic core logic for brightness control.
//!
//! This module contains the business logic that is shared across all platforms:
//!
//! - [`brightness`] - Brightness calculation and value mapping
//! - [`config`] - Configuration types and file handling
//! - [`state`] - Application state and inter-thread messages

pub mod brightness;
pub mod config;
pub mod state;

// Re-export commonly used types
pub use brightness::{BrightnessAdjustment, calculate_adjustment};
pub use config::Config;
pub use state::{BrightnessMessage, MonitorId, MonitorState};
