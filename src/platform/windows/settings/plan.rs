//! Turns the authored [`CONTROLS`] table plus text measurements into
//! positioned rectangles and a client size.
//!
//! Pure arithmetic: no window handles, no FFI. Everything it needs to know
//! about text arrives through [`TextMeasure`], which is what lets the same
//! function run against a fake on any host and against GDI in the layout
//! gate.
//!
//! **Units.** The planner works in physical pixels at the DPI it is planning
//! for. Authored constants pass through [`scale_dimension`]; measurements
//! come from a measurer built for that same DPI. Mixing the two is correct
//! at 96 and silently wrong at every other scale.

use crate::core::i18n::{Lang, strings};

use super::layout::{
    BASE_WINDOW_HEIGHT, BASE_WINDOW_WIDTH, CONTROLS, ID_VERSION, is_section_header, scale_dimension,
};
use super::measure::TextMeasure;

/// How a control responds when a translation makes surrounding text wider.
///
/// Every entry starts `Fixed`, which reproduces the authored table exactly;
/// a row earns a different anchor only when something about it must move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Anchor {
    /// Position and width exactly as authored, scaled to the DPI.
    Fixed,
}

/// One control's final rectangle, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "produced by the planner, which has no non-test caller until the settings window wires it in"
    )
)]
pub(super) struct Placed {
    pub(super) id: u16,
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) w: i32,
    pub(super) h: i32,
}

/// A complete layout: where every control goes, and how big the client area
/// must be to hold them.
#[derive(Debug, Clone)]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "produced by the planner, which has no non-test caller until the settings window wires it in"
    )
)]
pub(super) struct Plan {
    pub(super) controls: Vec<Placed>,
    pub(super) client_w: i32,
    pub(super) client_h: i32,
    // Constructed by the tests below (so the outer struct-level `expect`
    // does not cover this field there), but nothing yet reads it back.
    #[cfg_attr(
        test,
        expect(
            dead_code,
            reason = "read once the settings window wires the planner in"
        )
    )]
    pub(super) dpi: u32,
}

impl Plan {
    /// The placement of `id`, or `None` if the table has no such control.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "no non-test caller until the settings window wires the planner in"
        )
    )]
    pub(super) fn get(&self, id: u16) -> Option<&Placed> {
        self.controls.iter().find(|placed| placed.id == id)
    }
}

/// The caption a control shows in `lang`: every row's text comes from the
/// string table except the version line, whose value only exists once the
/// build has run.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no non-test caller until the settings window wires the planner in"
    )
)]
fn caption<'a>(spec: &super::layout::ControlSpec, lang: Lang, version_text: &'a str) -> &'a str {
    if spec.id == ID_VERSION {
        return version_text;
    }
    spec.text.map_or("", |key| strings(lang).get(key))
}

/// Computes the whole layout for `lang` at `dpi`.
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "no non-test caller until the settings window wires the planner in"
    )
)]
pub(super) fn plan_layout(
    lang: Lang,
    dpi: u32,
    version_text: &str,
    m: &mut impl TextMeasure,
) -> Plan {
    // Every row is Fixed for now, so none of these are needed yet — the
    // anchors that read a caption or a measurement to move a control follow.
    let _ = (
        &lang,
        version_text,
        &*m,
        is_section_header as fn(u16) -> bool,
        caption,
    );
    let controls = CONTROLS
        .iter()
        .map(|spec| match spec.anchor {
            Anchor::Fixed => Placed {
                id: spec.id,
                x: scale_dimension(spec.x, dpi),
                y: scale_dimension(spec.y, dpi),
                w: scale_dimension(spec.w, dpi),
                h: scale_dimension(spec.h, dpi),
            },
        })
        .collect();

    Plan {
        controls,
        client_w: scale_dimension(BASE_WINDOW_WIDTH, dpi),
        client_h: scale_dimension(BASE_WINDOW_HEIGHT, dpi),
        dpi,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::i18n::Lang;

    /// Measures a fixed width per character, so an assertion can be read
    /// without knowing any font's metrics.
    pub(super) struct FakeMeasure {
        pub(super) per_char: i32,
        pub(super) line_height: i32,
        pub(super) indicator: i32,
        pub(super) arrow: i32,
    }

    impl Default for FakeMeasure {
        fn default() -> Self {
            Self {
                per_char: 7,
                line_height: 15,
                indicator: 13,
                arrow: 17,
            }
        }
    }

    impl TextMeasure for FakeMeasure {
        fn text_width(&mut self, text: &str, bold: bool) -> i32 {
            let base = i32::try_from(text.chars().count()).unwrap_or(0) * self.per_char;
            if bold { base + base / 10 } else { base }
        }
        fn wrapped_height(&mut self, text: &str, bold: bool, width: i32) -> i32 {
            let w = self.text_width(text, bold);
            let lines = if width <= 0 {
                1
            } else {
                (w + width - 1) / width
            };
            lines.max(1) * self.line_height
        }
        fn checkbox_indicator(&mut self) -> i32 {
            self.indicator
        }
        fn combo_arrow(&mut self) -> i32 {
            self.arrow
        }
    }

    #[test]
    fn a_plan_of_only_fixed_anchors_reproduces_the_scaled_table() {
        for dpi in [96u32, 120, 144, 192] {
            let mut m = FakeMeasure::default();
            let plan = plan_layout(Lang::English, dpi, "0.10.0", &mut m);
            for spec in CONTROLS {
                let placed = plan.get(spec.id).expect("every control is placed");
                assert_eq!(placed.x, scale_dimension(spec.x, dpi), "x of {}", spec.id);
                assert_eq!(placed.y, scale_dimension(spec.y, dpi), "y of {}", spec.id);
                assert_eq!(placed.w, scale_dimension(spec.w, dpi), "w of {}", spec.id);
                assert_eq!(placed.h, scale_dimension(spec.h, dpi), "h of {}", spec.id);
            }
            assert_eq!(plan.client_w, scale_dimension(BASE_WINDOW_WIDTH, dpi));
            assert_eq!(plan.client_h, scale_dimension(BASE_WINDOW_HEIGHT, dpi));
        }
    }

    #[test]
    fn every_control_appears_exactly_once_in_a_plan() {
        let mut m = FakeMeasure::default();
        let plan = plan_layout(Lang::English, 96, "0.10.0", &mut m);
        assert_eq!(plan.controls.len(), CONTROLS.len());
        let mut ids: Vec<u16> = plan.controls.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CONTROLS.len());
    }
}
