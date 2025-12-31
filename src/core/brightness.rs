//! Brightness calculation and value mapping logic.
//!
//! This module provides functions for calculating brightness adjustments
//! and managing the relationship between hardware brightness and overlay opacity.

/// Minimum hardware brightness value.
pub const BRIGHTNESS_MIN: u8 = 0;
/// Maximum hardware brightness value.
pub const BRIGHTNESS_MAX: u8 = 100;

/// Result of a brightness adjustment calculation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BrightnessAdjustment {
    /// New hardware brightness value (0-100).
    pub hardware_brightness: u8,
    /// New overlay opacity (0-100, where 0 = invisible, 100 = fully opaque/black).
    pub overlay_opacity: u8,
}

impl BrightnessAdjustment {
    /// Creates a new brightness adjustment result.
    pub const fn new(hardware_brightness: u8, overlay_opacity: u8) -> Self {
        Self {
            hardware_brightness,
            overlay_opacity,
        }
    }

    /// Returns true if the overlay should be visible.
    pub const fn overlay_active(&self) -> bool {
        self.overlay_opacity > 0
    }
}

/// Calculates the new brightness state after applying a delta.
///
/// # Behavior
///
/// When decreasing brightness:
/// - Hardware brightness decreases until it reaches 0%
/// - At hardware 0%, overlay opacity increases (additional dimming)
///
/// When increasing brightness:
/// - If overlay is active, overlay opacity decreases first
/// - Once overlay is at 0%, hardware brightness increases
///
/// # Arguments
///
/// * `current_hardware` - Current hardware brightness (0-100)
/// * `current_overlay` - Current overlay opacity (0-100)
/// * `delta` - Change to apply (negative = dimmer, positive = brighter)
///
/// # Returns
///
/// The new `BrightnessAdjustment` with updated hardware and overlay values.
pub fn calculate_adjustment(
    current_hardware: u8,
    current_overlay: u8,
    delta: i8,
) -> BrightnessAdjustment {
    let delta_i16 = i16::from(delta);

    if delta < 0 {
        // Decreasing brightness
        calculate_decrease(current_hardware, current_overlay, delta_i16.unsigned_abs())
    } else {
        // Increasing brightness
        calculate_increase(current_hardware, current_overlay, delta_i16.unsigned_abs())
    }
}

/// Calculates brightness decrease (dimming).
fn calculate_decrease(
    current_hardware: u8,
    current_overlay: u8,
    decrease_amount: u16,
) -> BrightnessAdjustment {
    let hardware = u16::from(current_hardware);
    let overlay = u16::from(current_overlay);

    if hardware > 0 {
        // First, decrease hardware brightness
        let new_hardware = hardware.saturating_sub(decrease_amount);
        let remaining = decrease_amount.saturating_sub(hardware);

        if remaining > 0 {
            // Hardware hit 0, apply remaining to overlay
            let new_overlay = (overlay + remaining).min(100);
            BrightnessAdjustment::new(0, new_overlay as u8)
        } else {
            BrightnessAdjustment::new(new_hardware as u8, overlay as u8)
        }
    } else {
        // Hardware already at 0, increase overlay opacity
        let new_overlay = (overlay + decrease_amount).min(100);
        BrightnessAdjustment::new(0, new_overlay as u8)
    }
}

/// Calculates brightness increase (brightening).
fn calculate_increase(
    current_hardware: u8,
    current_overlay: u8,
    increase_amount: u16,
) -> BrightnessAdjustment {
    let hardware = u16::from(current_hardware);
    let overlay = u16::from(current_overlay);

    if overlay > 0 {
        // First, decrease overlay opacity
        let new_overlay = overlay.saturating_sub(increase_amount);
        let remaining = increase_amount.saturating_sub(overlay);

        if remaining > 0 {
            // Overlay hit 0, apply remaining to hardware
            let new_hardware = (hardware + remaining).min(100);
            BrightnessAdjustment::new(new_hardware as u8, 0)
        } else {
            BrightnessAdjustment::new(hardware as u8, new_overlay as u8)
        }
    } else {
        // No overlay, increase hardware brightness
        let new_hardware = (hardware + increase_amount).min(100);
        BrightnessAdjustment::new(new_hardware as u8, 0)
    }
}

/// Clamps a brightness value to the valid range (0-100).
#[inline]
pub const fn clamp_brightness(value: i16) -> u8 {
    if value < 0 {
        0
    } else if value > 100 {
        100
    } else {
        value as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decrease_from_50() {
        let result = calculate_adjustment(50, 0, -10);
        assert_eq!(result.hardware_brightness, 40);
        assert_eq!(result.overlay_opacity, 0);
    }

    #[test]
    fn test_decrease_to_zero() {
        let result = calculate_adjustment(5, 0, -10);
        assert_eq!(result.hardware_brightness, 0);
        assert_eq!(result.overlay_opacity, 5);
    }

    #[test]
    fn test_decrease_with_overlay() {
        let result = calculate_adjustment(0, 50, -10);
        assert_eq!(result.hardware_brightness, 0);
        assert_eq!(result.overlay_opacity, 60);
    }

    #[test]
    fn test_increase_from_50() {
        let result = calculate_adjustment(50, 0, 10);
        assert_eq!(result.hardware_brightness, 60);
        assert_eq!(result.overlay_opacity, 0);
    }

    #[test]
    fn test_increase_from_overlay() {
        let result = calculate_adjustment(0, 50, 10);
        assert_eq!(result.hardware_brightness, 0);
        assert_eq!(result.overlay_opacity, 40);
    }

    #[test]
    fn test_increase_removes_overlay() {
        let result = calculate_adjustment(0, 5, 10);
        assert_eq!(result.hardware_brightness, 5);
        assert_eq!(result.overlay_opacity, 0);
    }

    #[test]
    fn test_clamp_max() {
        let result = calculate_adjustment(95, 0, 10);
        assert_eq!(result.hardware_brightness, 100);
    }

    #[test]
    fn test_overlay_clamp_max() {
        let result = calculate_adjustment(0, 95, -10);
        assert_eq!(result.overlay_opacity, 100);
    }
}
