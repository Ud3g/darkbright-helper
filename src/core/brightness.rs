//! Brightness calculation and value mapping logic.
//!
//! This module provides functions for calculating brightness adjustments
//! and managing the relationship between hardware brightness and overlay opacity.
//!
//! It also owns the conversion between the percentages used everywhere above
//! the platform seam and the raw values DDC/CI actually carries. Brightness is
//! a *percentage* throughout this crate — the one place raw hardware values
//! exist is inside the Windows DDC module, which converts at its boundary.

/// Maximum hardware brightness value.
pub(crate) const BRIGHTNESS_MAX: u8 = 100;

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
    #[must_use]
    pub(crate) const fn new(hardware_brightness: u8, overlay_opacity: u8) -> Self {
        Self {
            hardware_brightness,
            overlay_opacity,
        }
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
#[must_use]
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
            // new_overlay is clamped to 100, so try_from is safe
            BrightnessAdjustment::new(0, u8::try_from(new_overlay).unwrap_or(100))
        } else {
            // new_hardware is based on saturating_sub of 0-100 values
            BrightnessAdjustment::new(u8::try_from(new_hardware).unwrap_or(0), current_overlay)
        }
    } else {
        // Hardware already at 0, increase overlay opacity
        let new_overlay = (overlay + decrease_amount).min(100);
        BrightnessAdjustment::new(0, u8::try_from(new_overlay).unwrap_or(100))
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
            BrightnessAdjustment::new(u8::try_from(new_hardware).unwrap_or(100), 0)
        } else {
            // Safe to use current_hardware as it didn't change, and new_overlay is clamped
            BrightnessAdjustment::new(current_hardware, u8::try_from(new_overlay).unwrap_or(0))
        }
    } else {
        // No overlay, increase hardware brightness
        let new_hardware = (hardware + increase_amount).min(100);
        BrightnessAdjustment::new(u8::try_from(new_hardware).unwrap_or(100), 0)
    }
}

/// Scale assumed for a monitor that has not reported a usable VCP maximum.
///
/// Also the most common real value, which makes the assumption a pass-through
/// rather than a guess with consequences.
pub(crate) const VCP_ASSUMED_MAX: u32 = 100;

/// Resolves the scale to divide by: the monitor's own maximum when it reported
/// a usable one, otherwise the assumed scale.
///
/// A reported zero is rejected rather than trusted — a zero-width range is not
/// a scale, and dividing by it would panic.
fn vcp_scale(reported_max: Option<u32>) -> u64 {
    match reported_max {
        Some(max) if max > 0 => u64::from(max),
        _ => u64::from(VCP_ASSUMED_MAX),
    }
}

/// Converts a raw VCP luminance value into a percentage of the monitor's range.
///
/// `reported_max` is the maximum the monitor itself declared for the feature
/// (see [`crate::platform::windows::ddc`]); pass `None` when no successful read
/// has established it yet.
///
/// A value above the reported maximum reads as full rather than overflowing:
/// the monitor contradicting its own declared range is not a reason to report a
/// nonsensical percentage.
#[must_use]
pub fn percent_from_vcp(raw: u32, reported_max: Option<u32>) -> u8 {
    let scale = vcp_scale(reported_max);
    let raw = u64::from(raw).min(scale);

    // Rounded, not truncated, so that a written percentage reads back as
    // itself. Widened to u64 first: `raw * 100` leaves u32 range at a raw
    // value of ~43 million, well inside what a 32-bit reply can carry.
    let percent = (raw * u64::from(BRIGHTNESS_MAX) + scale / 2) / scale;

    u8::try_from(percent).unwrap_or(BRIGHTNESS_MAX)
}

/// Converts a percentage into a raw VCP luminance value on the monitor's scale.
///
/// `reported_max` is the maximum the monitor itself declared for the feature;
/// pass `None` when no successful read has established it yet.
///
/// On a monitor whose range is narrower than 100 steps, distinct percentages
/// necessarily collapse onto the same raw value — both ends of the range stay
/// reachable, but the resolution is the hardware's, not ours.
#[must_use]
pub(crate) fn vcp_from_percent(percent: u8, reported_max: Option<u32>) -> u32 {
    let scale = vcp_scale(reported_max);
    let percent = u64::from(percent.min(BRIGHTNESS_MAX));

    let raw = (percent * scale + u64::from(BRIGHTNESS_MAX) / 2) / u64::from(BRIGHTNESS_MAX);

    u32::try_from(raw.min(scale)).unwrap_or(u32::MAX)
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

    // ── VCP luminance scaling ────────────────────────────────────────────────
    //
    // DDC/CI luminance is a continuous control whose range the monitor itself
    // declares; MCCS does not require that maximum to be 100.

    #[test]
    fn read_is_scaled_by_the_monitors_reported_maximum() {
        // Half of a 0-1000 range is 50%, not 500 — a raw value taken for a
        // percentage truncates into the plausible range and hides itself.
        assert_eq!(percent_from_vcp(500, Some(1000)), 50);
        assert_eq!(percent_from_vcp(128, Some(255)), 50);
        assert_eq!(percent_from_vcp(255, Some(255)), 100);
        assert_eq!(percent_from_vcp(0, Some(255)), 0);
    }

    #[test]
    fn write_is_scaled_to_the_monitors_reported_maximum() {
        // "100%" on a 0-255 monitor is 255; sending 100 sets ~39% backlight.
        assert_eq!(vcp_from_percent(100, Some(255)), 255);
        assert_eq!(vcp_from_percent(50, Some(255)), 128);
        assert_eq!(vcp_from_percent(0, Some(255)), 0);
        assert_eq!(vcp_from_percent(50, Some(1000)), 500);
    }

    #[test]
    fn an_unknown_or_nonsensical_maximum_falls_back_to_pass_through() {
        // No successful read yet, or a monitor claiming a zero-width range:
        // assume the common 0-100 scale, which makes both directions identity.
        assert_eq!(percent_from_vcp(42, None), 42);
        assert_eq!(vcp_from_percent(42, None), 42);
        assert_eq!(percent_from_vcp(42, Some(0)), 42);
        assert_eq!(vcp_from_percent(42, Some(0)), 42);
    }

    #[test]
    fn a_value_above_the_reported_maximum_reads_as_full() {
        assert_eq!(percent_from_vcp(300, Some(255)), 100);
        assert_eq!(percent_from_vcp(u32::MAX, Some(100)), 100);
    }

    #[test]
    fn a_maximum_below_100_saturates_instead_of_overshooting() {
        // Fewer than 100 distinct steps, so percentages collapse onto raw
        // values — but both ends of the range stay reachable.
        assert_eq!(vcp_from_percent(100, Some(10)), 10);
        assert_eq!(vcp_from_percent(95, Some(10)), 10);
        assert_eq!(percent_from_vcp(10, Some(10)), 100);
    }

    #[test]
    fn percentages_round_trip_through_any_maximum_of_at_least_100() {
        // Reading back what was just written must yield the same percentage,
        // or every refresh would nudge the displayed value by a step.
        for max in [100_u32, 101, 255, 1000, 65535] {
            for percent in 0..=100_u8 {
                let raw = vcp_from_percent(percent, Some(max));
                assert_eq!(
                    percent_from_vcp(raw, Some(max)),
                    percent,
                    "round trip failed for {percent}% at max {max} (raw {raw})"
                );
            }
        }
    }

    #[test]
    fn a_full_width_maximum_does_not_overflow_the_scaling_math() {
        // raw * 100 leaves u32 range long before this; the math must widen.
        assert_eq!(percent_from_vcp(u32::MAX / 2, Some(u32::MAX)), 50);
        assert_eq!(vcp_from_percent(100, Some(u32::MAX)), u32::MAX);
    }
}
