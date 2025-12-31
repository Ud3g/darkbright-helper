//! Platform abstraction layer for brightness control.
//!
//! This module defines traits that abstract platform-specific functionality,
//! allowing the core logic to remain platform-agnostic.

use crate::error::Result;

#[cfg(target_os = "windows")]
pub mod windows;

// ─────────────────────────────────────────────────────────────────────────────
// Platform Traits
// ─────────────────────────────────────────────────────────────────────────────

/// Trait for controlling a dimming overlay window.
///
/// The dimming overlay is a fullscreen, click-through, topmost window
/// filled with black color and variable opacity. It allows "dimming"
/// below the monitor's hardware minimum brightness.
///
/// # Platform Implementations
///
/// - **Windows**: GDI layered window with `SetLayeredWindowAttributes`
/// - **Linux** (future): X11 XComposite or Wayland layer-shell
pub trait DimmingOverlay {
    /// Sets the overlay opacity.
    ///
    /// # Arguments
    ///
    /// * `opacity` - Opacity value from 0.0 (invisible) to 1.0 (fully opaque/black)
    ///
    /// # Errors
    ///
    /// Returns an error if the window attribute cannot be set.
    fn set_opacity(&mut self, opacity: f32) -> Result<()>;

    /// Shows the overlay window.
    ///
    /// The window should be positioned to cover the entire monitor
    /// and set as topmost in the Z-order.
    ///
    /// # Errors
    ///
    /// Returns an error if the window cannot be shown.
    fn show(&mut self) -> Result<()>;

    /// Hides the overlay window.
    ///
    /// # Errors
    ///
    /// Returns an error if the window cannot be hidden.
    fn hide(&mut self) -> Result<()>;

    /// Returns true if the overlay is currently visible.
    fn is_visible(&self) -> bool;

    /// Returns the current opacity (0.0-1.0).
    fn opacity(&self) -> f32;
}
