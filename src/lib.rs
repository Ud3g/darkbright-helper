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
//! ```
//! use darkbright_helper::core::brightness::calculate_adjustment;
//!
//! // +10 from 50% hardware brightness with no overlay dimming active.
//! let adj = calculate_adjustment(50, 0, 10);
//! assert_eq!(adj.hardware_brightness, 60);
//! assert_eq!(adj.overlay_opacity, 0);
//! ```

pub mod core;
pub mod error;
pub mod platform;

// Re-exported for ergonomic `darkbright_helper::Result<T>` signatures in the
// binary and integration tests; everything else is imported via full paths.
pub use error::{BrightnessError, Result};
