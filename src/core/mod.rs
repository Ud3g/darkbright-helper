//! Platform-agnostic core logic for brightness control.
//!
//! This module contains the business logic that is shared across all platforms:
//!
//! - [`brightness`] - Brightness calculation and value mapping
//! - [`config`] - Configuration types and file handling
//! - [`controller`] - Message-driven orchestration behind platform seams
//! - [`edid`] - EDID parsing for monitor identification
//! - [`logfile`] - Size-capped rolling file sink for diagnostic logging
//! - [`panic_hook`] - Process-wide panic logging hook
//! - [`reconcile`] - Refresh/respawn tracking and reconciliation policies
//! - [`state`] - Application state and inter-thread messages

pub mod brightness;
pub mod config;
pub mod controller;
pub mod edid;
pub mod logfile;
pub mod panic_hook;
pub mod reconcile;
pub mod state;

// Re-export commonly used types
pub use brightness::{BrightnessAdjustment, calculate_adjustment};
pub use config::Config;
pub use reconcile::{RefreshTracker, respawn_allowed};
pub use state::{BrightnessMessage, MonitorId, MonitorState};
