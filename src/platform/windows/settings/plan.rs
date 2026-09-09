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

use super::dark::CHECKBOX_TEXT_GAP;
use super::layout::{
    BASE_WINDOW_HEIGHT, BASE_WINDOW_WIDTH, CONTROLS, ControlSpec, ID_VERSION, is_section_header,
    scale_dimension,
};
use super::measure::TextMeasure;

/// Which of the window's two label columns a row belongs to. The table has
/// always had two — the hotkey rows put their control at x=170, the spinner
/// and log rows at x=250 — and they are measured independently so a
/// translation moves only the one it actually outgrows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Col {
    A,
    B,
}

/// How a control responds when a translation makes surrounding text wider.
///
/// Every entry starts `Fixed`, which reproduces the authored table exactly;
/// a row earns a different anchor only when something about it must move.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Anchor {
    /// Position and width exactly as authored, scaled to the DPI.
    Fixed,
    /// A row caption: width is the column's, which is the widest measured
    /// caption in it.
    Label(Col),
    /// A checkbox that occupies the label column: as `Label`, plus the
    /// indicator and the gap its caption does not get to use.
    Checkbox(Col),
    /// The log row's checkbox, which shares its column with the level label
    /// beside it: it takes only the width its own caption needs, not the
    /// column's. The whole run it starts accumulates into column B, which is
    /// why the variant carries no [`Col`].
    CheckboxRun,
    /// The log row's level label, right-aligned against the control column.
    /// Its authored width is a floor, which is what keeps the English row
    /// where it has always been.
    InlineLabel,
    /// A control in the given column: `x` is the column's computed edge, `w`
    /// as authored.
    Control(Col),
    /// As `Control`, but running to the window's right margin.
    ControlStretch(Col),
    /// A control that follows another in the same row — a spinner's buttons,
    /// a unit suffix — keeping its authored offset from the column edge.
    AfterControl(Col),
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
    #[must_use]
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
fn caption<'a>(spec: &ControlSpec, lang: Lang, version_text: &'a str) -> &'a str {
    if spec.id == ID_VERSION {
        return version_text;
    }
    spec.text.map_or("", |key| strings(lang).get(key))
}

/// Gap between a label column and the control beside it. 6 px in the
/// authored table (170 − 24 − 140, and the same arithmetic in column B),
/// read off it rather than chosen.
const COL_GAP: i32 = 6;
/// Gap between the two halves of the log row, read off the table the same
/// way.
const INLINE_GAP: i32 = 6;
/// Left and right margin for section headers, separators and the window's
/// own edge.
const MARGIN: i32 = 12;
/// The deeper indent every row inside a section uses.
const INDENT: i32 = 24;
/// Column widths the authored table encodes. They are floors, not values:
/// English and German both fit inside them, so neither language moves a
/// single control, and only a wider translation pushes a column out.
const COL_A_FLOOR: i32 = 140;
const COL_B_FLOOR: i32 = 220;
/// What a system-drawn checkbox consumes beyond its caption at 96 DPI,
/// measured from a live control with `BCM_GETIDEALSIZE`; see
/// [`checkbox_overhead`] for how it is used.
const SYSTEM_CHECKBOX_OVERHEAD: i32 = 18;

/// What a checkbox consumes beyond its caption at `dpi`, in physical pixels.
///
/// Either theme can render this window, so the larger of their two budgets
/// is what a caption has to clear. The dark-mode painter's is the themed
/// indicator plus [`CHECKBOX_TEXT_GAP`], and it is not a smooth function of
/// DPI — the drawn glyph jumps at 192 DPI, which the measured indicator term
/// catches and no scaled constant would. The system control's cannot be
/// measured without a live control, so it enters as its 96-DPI value scaled,
/// which over-states the nearly flat measured series above 96 on purpose: a
/// column a few pixels too wide renders correctly, one a few pixels too
/// narrow clips the caption.
#[must_use]
pub(super) fn checkbox_overhead(dpi: u32, m: &mut impl TextMeasure) -> i32 {
    (m.checkbox_indicator() + scale_dimension(CHECKBOX_TEXT_GAP, dpi))
        .max(scale_dimension(SYSTEM_CHECKBOX_OVERHEAD, dpi))
}

/// The width a row needs in its label column, including whatever chrome the
/// control puts in front of its caption.
#[must_use]
fn label_requirement(
    spec: &ControlSpec,
    lang: Lang,
    version_text: &str,
    checkbox_overhead: i32,
    m: &mut impl TextMeasure,
) -> i32 {
    let width = m.text_width(
        caption(spec, lang, version_text),
        is_section_header(spec.id),
    );
    match spec.anchor {
        Anchor::Checkbox(_) | Anchor::CheckboxRun => width + checkbox_overhead,
        _ => width,
    }
}

/// Where a column's controls start in the authored table, which is what
/// [`Anchor::AfterControl`] offsets are measured against.
#[must_use]
fn authored_edge(col: Col) -> i32 {
    match col {
        Col::A => INDENT + COL_A_FLOOR + COL_GAP,
        Col::B => INDENT + COL_B_FLOOR + COL_GAP,
    }
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
    let indent = scale_dimension(INDENT, dpi);
    let col_gap = scale_dimension(COL_GAP, dpi);
    let inline_gap = scale_dimension(INLINE_GAP, dpi);
    // Hoisted out of the per-row loop: the query opens a theme handle.
    let checkbox_overhead = checkbox_overhead(dpi, m);

    // The log row's level label: authored width is a floor, so a short
    // caption leaves the English row untouched and a long one grows leftward.
    let inline_spec = CONTROLS
        .iter()
        .find(|spec| spec.anchor == Anchor::InlineLabel);
    let inline_w = inline_spec.map_or(0, |spec| {
        let measured = m.text_width(caption(spec, lang, version_text), false);
        measured.max(scale_dimension(spec.w, dpi))
    });

    let mut col_a = scale_dimension(COL_A_FLOOR, dpi);
    let mut col_b = scale_dimension(COL_B_FLOOR, dpi);
    for spec in CONTROLS {
        let need = match spec.anchor {
            Anchor::Label(col) | Anchor::Checkbox(col) => Some((
                col,
                label_requirement(spec, lang, version_text, checkbox_overhead, m),
            )),
            // The log row's own requirement is the whole run before the
            // control column, not just its caption.
            Anchor::CheckboxRun => Some((
                Col::B,
                label_requirement(spec, lang, version_text, checkbox_overhead, m)
                    + inline_gap
                    + inline_w,
            )),
            _ => None,
        };
        match need {
            Some((Col::A, w)) => col_a = col_a.max(w),
            Some((Col::B, w)) => col_b = col_b.max(w),
            None => {}
        }
    }

    let edge_a = indent + col_a + col_gap;
    let edge_b = indent + col_b + col_gap;
    let edge = |col: Col| match col {
        Col::A => edge_a,
        Col::B => edge_b,
    };
    let client_w = scale_dimension(BASE_WINDOW_WIDTH, dpi);
    let right_margin = scale_dimension(MARGIN, dpi);

    let controls = CONTROLS
        .iter()
        .map(|spec| {
            let (x, w) = match spec.anchor {
                Anchor::Fixed => (scale_dimension(spec.x, dpi), scale_dimension(spec.w, dpi)),
                Anchor::Label(col) | Anchor::Checkbox(col) => (
                    indent,
                    match col {
                        Col::A => col_a,
                        Col::B => col_b,
                    },
                ),
                Anchor::CheckboxRun => (
                    indent,
                    label_requirement(spec, lang, version_text, checkbox_overhead, m),
                ),
                Anchor::InlineLabel => (edge_b - col_gap - inline_w, inline_w),
                Anchor::Control(col) => (edge(col), scale_dimension(spec.w, dpi)),
                Anchor::ControlStretch(col) => {
                    let x = edge(col);
                    (x, (client_w - right_margin - x).max(0))
                }
                // The authored offset from the column edge, preserved: a
                // spinner's buttons must stay glued to their edit field.
                Anchor::AfterControl(col) => (
                    edge(col) + scale_dimension(spec.x - authored_edge(col), dpi),
                    scale_dimension(spec.w, dpi),
                ),
            };
            Placed {
                id: spec.id,
                x,
                y: scale_dimension(spec.y, dpi),
                w,
                h: scale_dimension(spec.h, dpi),
            }
        })
        .collect();

    Plan {
        controls,
        client_w,
        client_h: scale_dimension(BASE_WINDOW_HEIGHT, dpi),
        dpi,
    }
}

#[cfg(test)]
mod tests {
    use super::super::layout::{
        ID_HK_UP, ID_LABEL_LOG_LEVEL, ID_LABEL_STEP_UNIT, ID_LANGUAGE, ID_LOG_CHECK, ID_LOG_LEVEL,
        ID_STEP_EDIT, ID_STEP_UPDOWN,
    };
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

    /// Column geometry the authored table encodes, restated so a failure
    /// names the number that moved.
    const AUTHORED_EDGE_A: i32 = 170;
    const AUTHORED_EDGE_B: i32 = 250;

    #[test]
    fn english_columns_land_exactly_where_the_authored_table_puts_them() {
        let mut m = FakeMeasure::default();
        let plan = plan_layout(Lang::English, 96, "0.10.0", &mut m);
        assert_eq!(plan.get(ID_HK_UP).unwrap().x, AUTHORED_EDGE_A);
        assert_eq!(plan.get(ID_STEP_EDIT).unwrap().x, AUTHORED_EDGE_B);
        assert_eq!(plan.get(ID_LANGUAGE).unwrap().x, AUTHORED_EDGE_B);
        assert_eq!(plan.get(ID_LOG_LEVEL).unwrap().x, AUTHORED_EDGE_B);
        // The spinner buttons and unit suffixes keep their offset from the
        // control edge.
        assert_eq!(plan.get(ID_STEP_UPDOWN).unwrap().x, AUTHORED_EDGE_B + 60);
        assert_eq!(
            plan.get(ID_LABEL_STEP_UNIT).unwrap().x,
            AUTHORED_EDGE_B + 82
        );
    }

    #[test]
    fn the_inline_log_level_label_keeps_its_authored_width_as_a_floor() {
        // "Level:" is 6 chars = 42 px in the fake, well under the authored 74,
        // so the label must stay 74 wide and land at its authored x — sizing it
        // by measurement alone would shift the English row right.
        let mut m = FakeMeasure::default();
        let plan = plan_layout(Lang::English, 96, "0.10.0", &mut m);
        let inline = plan.get(ID_LABEL_LOG_LEVEL).unwrap();
        assert_eq!(inline.w, 74);
        assert_eq!(inline.x, 170);
    }

    #[test]
    fn a_wide_label_pushes_the_control_column_right() {
        // 40 chars at 7 px = 280, well past column B's 220 floor.
        let mut m = FakeMeasure {
            per_char: 7,
            ..FakeMeasure::default()
        };
        let narrow = plan_layout(Lang::English, 96, "0.10.0", &mut m)
            .get(ID_STEP_EDIT)
            .unwrap()
            .x;
        let mut wide = FakeMeasure {
            per_char: 14,
            ..FakeMeasure::default()
        };
        let pushed = plan_layout(Lang::English, 96, "0.10.0", &mut wide)
            .get(ID_STEP_EDIT)
            .unwrap()
            .x;
        assert_eq!(narrow, AUTHORED_EDGE_B);
        assert!(pushed > narrow, "narrow={narrow} pushed={pushed}");
    }

    #[test]
    fn the_log_row_run_feeds_the_same_column_maximum_as_a_plain_label() {
        // A caption long enough that indicator + caption + gap + inline label
        // exceeds the floor must move the control column, even though no
        // single label in the column is that wide.
        let mut m = FakeMeasure {
            per_char: 7,
            ..FakeMeasure::default()
        };
        let plan = plan_layout(Lang::German, 96, "0.10.0", &mut m);
        let check = plan.get(ID_LOG_CHECK).unwrap();
        let inline = plan.get(ID_LABEL_LOG_LEVEL).unwrap();
        assert!(
            check.x + check.w <= inline.x,
            "log checkbox {}..{} runs into the level label at {}",
            check.x,
            check.x + check.w,
            inline.x
        );
    }

    #[test]
    fn every_label_column_control_scales_with_dpi() {
        let mut m = FakeMeasure::default();
        let at96 = plan_layout(Lang::English, 96, "0.10.0", &mut m)
            .get(ID_STEP_EDIT)
            .unwrap()
            .x;
        let at192 = plan_layout(Lang::English, 192, "0.10.0", &mut m)
            .get(ID_STEP_EDIT)
            .unwrap()
            .x;
        assert!(at192 > at96, "96={at96} 192={at192}");
    }

    #[test]
    fn rows_keep_their_authored_vertical_geometry_and_fixed_rows_their_horizontal() {
        // Nothing in this planner touches `y` or `h` — only widths and the
        // columns that follow from them — so every row's vertical geometry
        // must still be the authored value, scaled. A row that was never
        // given an anchor keeps its `x` and `w` too.
        for dpi in [96u32, 120, 144, 192] {
            let mut m = FakeMeasure::default();
            let plan = plan_layout(Lang::English, dpi, "0.10.0", &mut m);
            for spec in CONTROLS {
                let placed = plan.get(spec.id).expect("every control is placed");
                assert_eq!(placed.y, scale_dimension(spec.y, dpi), "y of {}", spec.id);
                assert_eq!(placed.h, scale_dimension(spec.h, dpi), "h of {}", spec.id);
                if spec.anchor == Anchor::Fixed {
                    assert_eq!(placed.x, scale_dimension(spec.x, dpi), "x of {}", spec.id);
                    assert_eq!(placed.w, scale_dimension(spec.w, dpi), "w of {}", spec.id);
                }
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
