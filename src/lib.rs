//! Brightness Control Tool library.
//!
//! A hotkey-driven brightness adjustment tool for Windows.

pub mod core;
pub mod error;
pub mod platform;

pub use error::{BrightnessError, Result};
