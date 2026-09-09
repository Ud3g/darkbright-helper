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
    scale_dimension, wraps,
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
/// The variants together reproduce the authored table exactly for English at
/// 96 DPI; each one describes what the row does when text around it grows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Anchor {
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
    /// Keeps the authored right margin as the window widens: headers,
    /// separators, wrapping hints, full-width checkboxes and the footer link
    /// row.
    Stretch,
    /// One of the two footer buttons, in a chain right-aligned against the
    /// window's margin.
    FooterButton,
    /// The version line, which takes whatever the footer buttons leave.
    FooterFill,
}

/// One control's final rectangle, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
pub(super) struct Plan {
    pub(super) controls: Vec<Placed>,
    pub(super) client_w: i32,
    pub(super) client_h: i32,
    /// The DPI every number above is in, carried so a consumer resizing a
    /// window from this plan cannot pair it with a different one.
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
/// Footer spacing, both read off the authored table: 6 px between the
/// version line and the restore button (190 - 184), 8 px between the two
/// buttons (308 - 300).
const FOOTER_GAP_VERSION: i32 = 6;
const FOOTER_GAP_BUTTONS: i32 = 8;
/// A pushbutton is never narrower than this, and never tighter around its
/// caption than this. The floor is "Close"'s authored 80 px, which is what
/// keeps a short caption from producing a cramped button (35 + 26 = 61 would
/// otherwise). The padding is the authored table's own: the 110 px restore
/// button less "Restore defaults" at 84 px is 26, so English renders exactly
/// as it does today. A live control's ideal size wants only 8 px around its
/// caption, so 26 is comfortably more than the control itself needs.
const BUTTON_MIN_W: i32 = 80;
const BUTTON_TEXT_PAD: i32 = 26;
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

/// The log row's level label width: its authored width acts as a floor, so a
/// short caption leaves the English row exactly where it is and a long one
/// grows the label leftward instead of overrunning the control.
#[must_use]
fn inline_label_width(lang: Lang, dpi: u32, version_text: &str, m: &mut impl TextMeasure) -> i32 {
    CONTROLS
        .iter()
        .find(|spec| spec.anchor == Anchor::InlineLabel)
        .map_or(0, |spec| {
            let measured = m.text_width(caption(spec, lang, version_text), false);
            measured.max(scale_dimension(spec.w, dpi))
        })
}

/// The two label columns' widths: each is the widest requirement among its
/// rows, floored at the authored value so no shipped language moves a control.
#[must_use]
fn measure_columns(
    lang: Lang,
    dpi: u32,
    version_text: &str,
    checkbox_overhead: i32,
    inline_w: i32,
    m: &mut impl TextMeasure,
) -> (i32, i32) {
    let inline_gap = scale_dimension(INLINE_GAP, dpi);
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
    (col_a, col_b)
}

/// The footer row, measured: how wide each of its buttons must be, and what
/// the whole row needs between the window's two margins.
struct Footer {
    /// Each footer button's id and width, in table order — the order the
    /// right-aligned chain is packed in.
    buttons: Vec<(u16, i32)>,
    need: i32,
}

/// Measures the footer. Buttons size to their captions; the version line's
/// requirement is its own measured width, which is why a development build's
/// longer build string widens the window instead of being cut.
#[must_use]
fn plan_footer(lang: Lang, dpi: u32, version_text: &str, m: &mut impl TextMeasure) -> Footer {
    let mut buttons: Vec<(u16, i32)> = Vec::new();
    for spec in CONTROLS {
        if spec.anchor == Anchor::FooterButton {
            let text = m.text_width(caption(spec, lang, version_text), false);
            let w = (text + scale_dimension(BUTTON_TEXT_PAD, dpi))
                .max(scale_dimension(BUTTON_MIN_W, dpi));
            buttons.push((spec.id, w));
        }
    }
    let version_w = CONTROLS
        .iter()
        .find(|spec| spec.anchor == Anchor::FooterFill)
        .map_or(0, |spec| {
            m.text_width(caption(spec, lang, version_text), false)
        });

    let margin = scale_dimension(MARGIN, dpi);
    let need = margin
        + version_w
        + scale_dimension(FOOTER_GAP_VERSION, dpi)
        + buttons.iter().map(|(_, w)| *w).sum::<i32>()
        + scale_dimension(FOOTER_GAP_BUTTONS, dpi)
            * i32::try_from(buttons.len().saturating_sub(1)).unwrap_or(0)
        + margin;
    Footer { buttons, need }
}

/// A footer button's measured width, from the chain [`plan_footer`] sized.
#[must_use]
fn button_width(buttons: &[(u16, i32)], id: u16) -> i32 {
    buttons
        .iter()
        .find(|(bid, _)| *bid == id)
        .map_or(0, |(_, w)| *w)
}

/// Where a footer button's right edge falls. The chain is packed against
/// `right_edge` from its last member backwards, so a button's position depends
/// only on the buttons that follow it.
#[must_use]
fn button_right(buttons: &[(u16, i32)], id: u16, right_edge: i32, gap: i32) -> i32 {
    let mut right = right_edge;
    for (bid, bw) in buttons.iter().rev() {
        if *bid == id {
            break;
        }
        right -= bw + gap;
    }
    right
}

/// The widest extent any row in `col` needs to the right of its column edge,
/// so a language that outgrows a column widens the window rather than pushing
/// controls off its right edge. A row's extent runs to the end of whatever
/// trails its control — a spinner's buttons, a unit suffix — not just the
/// control itself.
#[must_use]
fn control_run(col: Col, dpi: u32) -> i32 {
    CONTROLS
        .iter()
        .filter_map(|spec| match spec.anchor {
            Anchor::Control(c) | Anchor::AfterControl(c) if c == col => {
                Some(scale_dimension(spec.x - authored_edge(col) + spec.w, dpi))
            }
            _ => None,
        })
        .max()
        .unwrap_or(0)
}

/// Computes the whole layout for `lang` at `dpi`.
pub(super) fn plan_layout(
    lang: Lang,
    dpi: u32,
    version_text: &str,
    m: &mut impl TextMeasure,
) -> Plan {
    let indent = scale_dimension(INDENT, dpi);
    let col_gap = scale_dimension(COL_GAP, dpi);
    let margin = scale_dimension(MARGIN, dpi);
    let gap_version = scale_dimension(FOOTER_GAP_VERSION, dpi);
    let gap_buttons = scale_dimension(FOOTER_GAP_BUTTONS, dpi);
    // Hoisted out of the per-row loop: the query opens a theme handle.
    let checkbox_overhead = checkbox_overhead(dpi, m);

    let inline_w = inline_label_width(lang, dpi, version_text, m);
    let (col_a, col_b) = measure_columns(lang, dpi, version_text, checkbox_overhead, inline_w, m);

    let edge_a = indent + col_a + col_gap;
    let edge_b = indent + col_b + col_gap;
    let edge = |col: Col| match col {
        Col::A => edge_a,
        Col::B => edge_b,
    };

    let footer = plan_footer(lang, dpi, version_text, m);
    let capture_w = CONTROLS
        .iter()
        .find(|spec| matches!(spec.anchor, Anchor::ControlStretch(_)))
        .map_or(0, |spec| scale_dimension(spec.w, dpi));

    let client_w = scale_dimension(BASE_WINDOW_WIDTH, dpi)
        .max(edge_a + capture_w + margin)
        .max(edge_b + control_run(Col::B, dpi) + margin)
        .max(footer.need);

    // A hint that needs more lines than its authored height allows pushes
    // every later row, and the window's own height, down by what it grew.
    let mut y_offset = 0;
    let controls = CONTROLS
        .iter()
        .map(|spec| {
            let (x, w) = match spec.anchor {
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
                    (x, (client_w - margin - x).max(0))
                }
                // The authored offset from the column edge, preserved: a
                // spinner's buttons must stay glued to their edit field.
                Anchor::AfterControl(col) => (
                    edge(col) + scale_dimension(spec.x - authored_edge(col), dpi),
                    scale_dimension(spec.w, dpi),
                ),
                // The right margin the table already implies for this row,
                // preserved as the window widens.
                Anchor::Stretch => {
                    let x = scale_dimension(spec.x, dpi);
                    let authored_right_margin =
                        scale_dimension(BASE_WINDOW_WIDTH - spec.x - spec.w, dpi);
                    (x, (client_w - authored_right_margin - x).max(0))
                }
                // Right-aligned chain: the last button sits against the
                // margin, each earlier one to its left.
                Anchor::FooterButton => {
                    let w = button_width(&footer.buttons, spec.id);
                    let right =
                        button_right(&footer.buttons, spec.id, client_w - margin, gap_buttons);
                    (right - w, w)
                }
                // Whatever the chain leaves between it and the left margin.
                Anchor::FooterFill => {
                    let chain_left = footer.buttons.first().map_or(client_w - margin, |(id, _)| {
                        button_right(&footer.buttons, *id, client_w - margin, gap_buttons)
                            - button_width(&footer.buttons, *id)
                    });
                    (margin, (chain_left - gap_version - margin).max(0))
                }
            };
            let top = scale_dimension(spec.y, dpi) + y_offset;
            let mut height = scale_dimension(spec.h, dpi);
            if wraps(spec.id) {
                // Measured against the width this row actually got, not the
                // authored one. Grow only: a wider window wraps a hint onto
                // fewer lines, and a window that got shorter because a
                // translation was terse would be more surprise than gain.
                let needed = m.wrapped_height(caption(spec, lang, version_text), false, w);
                let delta = (needed - height).max(0);
                height += delta;
                y_offset += delta;
            }
            Placed {
                id: spec.id,
                x,
                y: top,
                w,
                h: height,
            }
        })
        .collect();

    Plan {
        controls,
        client_w,
        client_h: scale_dimension(BASE_WINDOW_HEIGHT, dpi) + y_offset,
        dpi,
    }
}

#[cfg(test)]
mod tests {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY,
    };

    use super::super::super::hotkey::{ParsedHotkey, key_name};
    use super::super::layout::{
        ID_CLOSE, ID_HK_DOWN, ID_HK_UP, ID_LABEL_HK_UP, ID_LABEL_LOG_LEVEL, ID_LABEL_STEP_UNIT,
        ID_LANGUAGE, ID_LINK_CONFIG, ID_LOG_CHECK, ID_LOG_LEVEL, ID_RESTORE, ID_SEP_GENERAL,
        ID_STEP_EDIT, ID_STEP_UPDOWN, is_checkbox,
    };
    use super::super::measure::GdiMeasure;
    use super::super::window::{language_combo_entries, log_level_combo_entries};
    use super::*;
    use crate::core::i18n::{Lang, strings};

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
    fn rows_keep_their_authored_vertical_geometry_and_english_its_window_size() {
        // Only a hint that outgrows its authored height moves anything
        // vertically, and English fits both of them, so every row's
        // vertical geometry must still be the authored value, scaled.
        // English at the authored widths must also leave the window at its
        // authored size.
        for dpi in [96u32, 120, 144, 192] {
            let mut m = FakeMeasure::default();
            let plan = plan_layout(Lang::English, dpi, "0.10.0", &mut m);
            for spec in CONTROLS {
                let placed = plan.get(spec.id).expect("every control is placed");
                assert_eq!(placed.y, scale_dimension(spec.y, dpi), "y of {}", spec.id);
                assert_eq!(placed.h, scale_dimension(spec.h, dpi), "h of {}", spec.id);
            }
            assert_eq!(plan.client_w, scale_dimension(BASE_WINDOW_WIDTH, dpi));
            assert_eq!(plan.client_h, scale_dimension(BASE_WINDOW_HEIGHT, dpi));
        }
    }

    #[test]
    fn stretch_keeps_a_controls_authored_right_margin() {
        let mut m = FakeMeasure::default();
        let plan = plan_layout(Lang::English, 96, "v0.10.0", &mut m);
        // Headers and separators sit 12 from each edge and must still do so.
        let sep = plan.get(ID_SEP_GENERAL).unwrap();
        assert_eq!(sep.x, 12);
        assert_eq!(sep.x + sep.w, plan.client_w - 12);
        // The footer link row now spans the same full width; its authored 250
        // was simply too small for its own text.
        let link = plan.get(ID_LINK_CONFIG).unwrap();
        assert_eq!(link.x + link.w, plan.client_w - 12);
    }

    #[test]
    fn footer_buttons_are_right_aligned_and_sized_to_their_captions() {
        let mut m = FakeMeasure::default();
        let plan = plan_layout(Lang::English, 96, "v0.10.0", &mut m);
        let close = plan.get(ID_CLOSE).unwrap();
        let restore = plan.get(ID_RESTORE).unwrap();
        let version = plan.get(ID_VERSION).unwrap();
        assert_eq!(close.x + close.w, plan.client_w - 12);
        assert_eq!(restore.x + restore.w, close.x - 8);
        assert_eq!(version.x, 12);
        assert_eq!(version.x + version.w, restore.x - 6);
        assert!(version.w > 0, "the version line must not be squeezed away");
    }

    #[test]
    fn a_short_version_string_leaves_the_window_at_its_floor() {
        let mut m = FakeMeasure::default();
        let plan = plan_layout(Lang::English, 96, "v0.10.0", &mut m);
        assert_eq!(plan.client_w, 400);
    }

    #[test]
    fn a_long_version_string_widens_the_window_instead_of_being_cut() {
        let mut m = FakeMeasure::default();
        let long = "v0.10.0+64.gc4687e5.dirty (dev)";
        let plan = plan_layout(Lang::English, 96, long, &mut m);
        let version = plan.get(ID_VERSION).unwrap();
        assert!(plan.client_w > 400, "client_w={}", plan.client_w);
        assert!(
            version.w >= m.text_width(long, false),
            "version slot {} is narrower than its text",
            version.w
        );
    }

    #[test]
    fn a_wide_label_column_widens_the_window_too() {
        let mut wide = FakeMeasure {
            per_char: 20,
            ..FakeMeasure::default()
        };
        let plan = plan_layout(Lang::English, 96, "v0.10.0", &mut wide);
        assert!(plan.client_w > 400, "client_w={}", plan.client_w);
        // Nothing may hang past the right margin.
        for placed in &plan.controls {
            assert!(
                placed.x + placed.w <= plan.client_w - 12 || placed.w == 0,
                "control {} ends at {}, past {}",
                placed.id,
                placed.x + placed.w,
                plan.client_w - 12
            );
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

    #[test]
    fn a_hint_that_fits_its_authored_height_moves_nothing() {
        let mut m = FakeMeasure::default();
        let plan = plan_layout(Lang::English, 96, "v0.10.0", &mut m);
        assert_eq!(plan.client_h, 654);
        assert_eq!(plan.get(ID_CLOSE).unwrap().y, 616);
    }

    #[test]
    fn a_hint_that_needs_another_line_pushes_the_rows_below_it_down() {
        // Triple the per-character width so both hints wrap onto more lines
        // than their authored height allows.
        let mut m = FakeMeasure {
            per_char: 21,
            ..FakeMeasure::default()
        };
        let plan = plan_layout(Lang::English, 96, "v0.10.0", &mut m);
        assert!(plan.client_h > 654, "client_h={}", plan.client_h);
        assert!(
            plan.get(ID_CLOSE).unwrap().y > 616,
            "the footer must move down with the hint above it"
        );
        // A row above the first hint must not move.
        assert_eq!(plan.get(ID_LABEL_HK_UP).unwrap().y, 170);
    }

    #[test]
    fn hint_growth_is_recomputed_per_plan_and_never_latched() {
        let mut wide = FakeMeasure {
            per_char: 21,
            ..FakeMeasure::default()
        };
        let grown = plan_layout(Lang::English, 96, "v0.10.0", &mut wide).client_h;
        let mut narrow = FakeMeasure::default();
        let back = plan_layout(Lang::English, 96, "v0.10.0", &mut narrow).client_h;
        assert!(grown > back, "grown={grown} back={back}");
        assert_eq!(back, 654, "a later plan must not inherit an earlier growth");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Layout gate: the real fonts, every language, four scalings
    // ─────────────────────────────────────────────────────────────────────

    /// Logical-pixel bounds the derived layout must stay inside.
    ///
    /// Not a proof the window fits a screen: the outer rect for today's 654
    /// logical client is 1037 px at 144 DPI, against roughly 1008 usable on a
    /// 1080p work area at 150% scaling, so it already does not — the
    /// work-area clamp is what mitigates that, and it predates this layout.
    /// These are do-not-get-worse bounds.
    const MAX_CLIENT_W: i32 = 560;
    const MAX_CLIENT_H: i32 = 700;

    /// The widest realistic version string, which is what the footer must
    /// hold — not whatever `git describe` happens to produce on the machine
    /// running the test.
    const WORST_CASE_VERSION: &str = "v0.10.0+999.gc4687e5.dirty (dev)";

    /// The scalings the gate plans at: 100%, 125%, 150% and 200%.
    const GATE_DPIS: [u32; 4] = [96, 120, 144, 192];

    /// The table row `id` came from.
    fn spec_of(id: u16) -> &'static ControlSpec {
        CONTROLS
            .iter()
            .find(|spec| spec.id == id)
            .expect("every planned id comes from the table")
    }

    /// Whether `class` is exempt from the overlap check: a combo box's `h`
    /// is the height of its *dropped-down* list (a documented Win32 quirk —
    /// see the `ID_LOG_LEVEL` comment in `CONTROLS`), not the closed
    /// control's footprint, so its declared rect legitimately extends over
    /// controls below it without a real visual collision.
    fn overlap_exempt(class: &str) -> bool {
        class == "COMBOBOX"
    }

    /// The text a control actually draws in `lang`, with the worst-case
    /// version string standing in for the build's own and `SysLink`'s
    /// `<a>`/`</a>` anchor markup removed — that markup is the control's
    /// hyperlink syntax, never glyphs on screen.
    fn caption_for_gate(spec: &ControlSpec, lang: Lang) -> String {
        let text = caption(spec, lang, WORST_CASE_VERSION);
        if spec.class == "SysLink" {
            text.replace("<a>", "").replace("</a>", "")
        } else {
            text.to_string()
        }
    }

    /// Both pickers' entry lists in `lang`, from the same functions that fill
    /// the live combos, so an entry the window shows cannot escape the gate.
    fn combo_entries_for_gate(lang: Lang) -> [(u16, Vec<&'static str>); 2] {
        let s = strings(lang);
        [
            (ID_LOG_LEVEL, log_level_combo_entries(s)),
            (ID_LANGUAGE, language_combo_entries(s)),
        ]
    }

    /// The widest string a capture field can be asked to show in `lang`:
    /// every modifier plus whichever named key renders widest. Which key
    /// that is depends on the font and on the translation, so every named
    /// key is rendered and measured rather than guessed at.
    fn longest_hotkey_display_text(lang: Lang, m: &mut impl TextMeasure) -> String {
        let s = strings(lang);
        let modifiers = MOD_CONTROL | MOD_ALT | MOD_SHIFT | MOD_WIN;
        (0..=u16::from(u8::MAX))
            .map(VIRTUAL_KEY)
            .filter(|vk| key_name(*vk).is_some())
            .map(|vk| ParsedHotkey::new(modifiers, vk).display_text(s))
            .max_by_key(|text| m.text_width(text, false))
            .expect("the key-name table is never empty")
    }

    /// A measurer for `dpi`, or a panic: a gate that silently measured
    /// nothing would pass however badly the layout overflowed.
    fn gate_measure(dpi: u32) -> GdiMeasure {
        GdiMeasure::new(dpi).unwrap_or_else(|| {
            panic!("no measurement context at {dpi} dpi — the gate measured nothing")
        })
    }

    /// Drawable width of a control's slot: what the painter actually has for
    /// text, after the chrome that class puts around it.
    fn drawable(spec: &ControlSpec, placed: &Placed, dpi: u32, m: &mut impl TextMeasure) -> i32 {
        use super::super::capture::CAPTURE_TEXT_INSET;
        use super::super::dark::COMBO_TEXT_INSET;
        match spec.class {
            "COMBOBOX" => placed.w - m.combo_arrow() - 2 * scale_dimension(COMBO_TEXT_INSET, dpi),
            "HOTKEY_CAPTURE" => placed.w - 2 * scale_dimension(CAPTURE_TEXT_INSET, dpi),
            // Pushbuttons and checkboxes share the BUTTON class, so the style
            // bit is what tells them apart; only a checkbox spends width on an
            // indicator its caption cannot use.
            "BUTTON" if is_checkbox(spec.style) => placed.w - checkbox_overhead(dpi, m),
            _ => placed.w,
        }
    }

    #[test]
    fn no_caption_overflows_its_slot_in_any_language_at_any_dpi() {
        let mut failures: Vec<String> = Vec::new();
        for &lang in Lang::ALL {
            for dpi in GATE_DPIS {
                let mut m = gate_measure(dpi);
                let plan = plan_layout(lang, dpi, WORST_CASE_VERSION, &mut m);
                for spec in CONTROLS {
                    let placed = *plan.get(spec.id).expect("planned");
                    let text = caption_for_gate(spec, lang);
                    if text.is_empty() {
                        continue;
                    }
                    if wraps(spec.id) {
                        let needed = m.wrapped_height(&text, false, placed.w);
                        if needed > placed.h {
                            failures.push(format!(
                                "| {} | {dpi} | {} | {text} | h {} | needs {needed} |",
                                lang.tag(),
                                spec.id,
                                placed.h
                            ));
                        }
                        continue;
                    }
                    let available = drawable(spec, &placed, dpi, &mut m);
                    let needed = m.text_width(&text, is_section_header(spec.id));
                    if needed > available {
                        failures.push(format!(
                            "| {} | {dpi} | {} | {text} | {available} | {needed} | +{} |",
                            lang.tag(),
                            spec.id,
                            needed - available
                        ));
                    }
                }
            }
        }
        assert!(failures.is_empty(), "\n{}", failures.join("\n"));
    }

    #[test]
    fn no_combo_entry_overflows_its_combo_in_any_language_at_any_dpi() {
        // Combo entries are not CONTROLS rows, which is exactly why the
        // previous diagnostic never saw the log-level picker clip.
        for &lang in Lang::ALL {
            for dpi in GATE_DPIS {
                let mut m = gate_measure(dpi);
                let plan = plan_layout(lang, dpi, WORST_CASE_VERSION, &mut m);
                for (id, entries) in combo_entries_for_gate(lang) {
                    let placed = *plan.get(id).expect("planned");
                    let available = drawable(spec_of(id), &placed, dpi, &mut m);
                    for entry in entries {
                        let needed = m.text_width(entry, false);
                        assert!(
                            needed <= available,
                            "{} combo {id} entry {entry:?} needs {needed} of {available} at {dpi} dpi",
                            lang.tag()
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn no_two_planned_controls_overlap_in_any_language_at_any_dpi() {
        for &lang in Lang::ALL {
            for dpi in GATE_DPIS {
                let mut m = gate_measure(dpi);
                let plan = plan_layout(lang, dpi, WORST_CASE_VERSION, &mut m);
                for (i, a) in plan.controls.iter().enumerate() {
                    for b in &plan.controls[i + 1..] {
                        if overlap_exempt(spec_of(a.id).class)
                            || overlap_exempt(spec_of(b.id).class)
                        {
                            continue;
                        }
                        let overlaps = a.x < b.x + b.w
                            && b.x < a.x + a.w
                            && a.y < b.y + b.h
                            && b.y < a.y + a.h;
                        assert!(
                            !overlaps,
                            "{} at {dpi} dpi: {} ({},{},{},{}) overlaps {} ({},{},{},{})",
                            lang.tag(),
                            a.id,
                            a.x,
                            a.y,
                            a.w,
                            a.h,
                            b.id,
                            b.x,
                            b.y,
                            b.w,
                            b.h
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn the_planned_window_stays_within_its_bounds_in_any_language() {
        for &lang in Lang::ALL {
            for dpi in GATE_DPIS {
                let mut m = gate_measure(dpi);
                let plan = plan_layout(lang, dpi, WORST_CASE_VERSION, &mut m);
                assert!(
                    plan.client_w <= scale_dimension(MAX_CLIENT_W, dpi),
                    "{} at {dpi} dpi: client_w {} exceeds the bound",
                    lang.tag(),
                    plan.client_w
                );
                assert!(
                    plan.client_h <= scale_dimension(MAX_CLIENT_H, dpi),
                    "{} at {dpi} dpi: client_h {} exceeds the bound",
                    lang.tag(),
                    plan.client_h
                );
            }
        }
    }

    #[test]
    fn a_capture_field_holds_the_longest_hotkey_text_its_language_can_produce() {
        // The same seam the tray's usage rows render through, so a modifier
        // name that outgrows this field outgrows the menu too.
        for &lang in Lang::ALL {
            for dpi in GATE_DPIS {
                let mut m = gate_measure(dpi);
                let plan = plan_layout(lang, dpi, WORST_CASE_VERSION, &mut m);
                let longest = longest_hotkey_display_text(lang, &mut m);
                for id in [ID_HK_UP, ID_HK_DOWN] {
                    let placed = *plan.get(id).expect("planned");
                    let available = drawable(spec_of(id), &placed, dpi, &mut m);
                    let needed = m.text_width(&longest, false);
                    assert!(
                        needed <= available,
                        "{} at {dpi} dpi: {longest:?} needs {needed} of {available}",
                        lang.tag()
                    );
                }
            }
        }
    }
}
