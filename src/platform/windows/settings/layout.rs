//! Declarative layout data and geometry for the settings window: control
//! ids and styles, the [`CONTROLS`] table, DPI scaling, and window
//! placement. Pure data plus arithmetic — no window handles are created or
//! mutated here beyond positioning/range-configuring controls that
//! `window` already created; see `window` for control creation, wiring and
//! the message loop.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, HMONITOR, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
};
use windows::Win32::UI::Controls::{UDACCEL, UDM_SETACCEL, UDM_SETRANGE32};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    BeginDeferWindowPos, CB_GETITEMHEIGHT, CB_SETITEMHEIGHT, DeferWindowPos, EndDeferWindowPos,
    GetCursorPos, GetDlgItem, GetWindowRect, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOZORDER,
    SendMessageW, SetWindowPos, WINDOW_EX_STYLE, WINDOW_STYLE, WS_BORDER, WS_CAPTION,
    WS_EX_TOPMOST, WS_GROUP, WS_SYSMENU, WS_TABSTOP,
};

use crate::core::i18n::{Lang, TextKey};
use crate::core::version::version_string;
use crate::error::{BrightnessError, Result};

use super::measure::GdiMeasure;
use super::plan::{Anchor, Col, Plan, plan_layout};

/// The style and extended style the settings window is created with. Named
/// here because the frame arithmetic (`AdjustWindowRectExForDpi`, in both
/// the initial placement and every later resize) has to be told exactly the
/// pair `CreateWindowExW` was given: a mismatch sizes the client area
/// against the wrong non-client border, silently.
pub(super) const SETTINGS_STYLE: WINDOW_STYLE = WINDOW_STYLE(WS_CAPTION.0 | WS_SYSMENU.0);
pub(super) const SETTINGS_EX_STYLE: WINDOW_EX_STYLE = WS_EX_TOPMOST;

// ─────────────────────────────────────────────────────────────────────────────
// Control IDs
// ─────────────────────────────────────────────────────────────────────────────
// 100s = General, 110s = Hotkeys, 120s = On-screen display, 130s = Advanced,
// 140s = footer. 200+ are decorative (section headers, separators, plain
// text labels) — never addressed individually outside this module, but each
// still needs a distinct id: [`apply`] positions every control by
// `GetDlgItem(hwnd, id)`, which only ever finds the first match, so a shared
// id would silently move just one control instead of all of them.

pub(super) const ID_AUTOSTART: u16 = 100;
pub(super) const ID_STEP_EDIT: u16 = 101;
pub(super) const ID_STEP_UPDOWN: u16 = 102;
pub(super) const ID_LANGUAGE: u16 = 103;

pub(super) const ID_HK_UP: u16 = 110;
pub(super) const ID_HK_DOWN: u16 = 111;
pub(super) const ID_INTERCEPT: u16 = 112;
pub(super) const ID_HK_HINT: u16 = 113;
pub(super) const ID_HK_ERROR: u16 = 114;

pub(super) const ID_OSD_TIMEOUT_EDIT: u16 = 120;
pub(super) const ID_OSD_TIMEOUT_UPDOWN: u16 = 121;
pub(super) const ID_OSD_OPACITY_EDIT: u16 = 122;
pub(super) const ID_OSD_OPACITY_UPDOWN: u16 = 123;

pub(super) const ID_RESYNC_CHECK: u16 = 130;
pub(super) const ID_RESYNC_EDIT: u16 = 131;
pub(super) const ID_RESYNC_UPDOWN: u16 = 132;
pub(super) const ID_INACT_CHECK: u16 = 133;
pub(super) const ID_INACT_EDIT: u16 = 134;
pub(super) const ID_INACT_UPDOWN: u16 = 135;
pub(super) const ID_LOG_CHECK: u16 = 136;
pub(super) const ID_LOG_LEVEL: u16 = 137;
pub(super) const ID_LOG_HINT: u16 = 138;

pub(super) const ID_LINK_CONFIG: u16 = 140;
pub(super) const ID_RESTORE: u16 = 142;
pub(super) const ID_CLOSE: u16 = 143;
pub(super) const ID_VERSION: u16 = 144;

const ID_HEADER_GENERAL: u16 = 200;
pub(super) const ID_SEP_GENERAL: u16 = 201;
const ID_LABEL_STEP: u16 = 202;
pub(super) const ID_LABEL_STEP_UNIT: u16 = 203;
const ID_HEADER_HOTKEYS: u16 = 204;
const ID_SEP_HOTKEYS: u16 = 205;
pub(super) const ID_LABEL_HK_UP: u16 = 206;
const ID_LABEL_HK_DOWN: u16 = 207;
const ID_HEADER_OSD: u16 = 208;
const ID_SEP_OSD: u16 = 209;
const ID_LABEL_TIMEOUT: u16 = 210;
const ID_LABEL_TIMEOUT_UNIT: u16 = 211;
const ID_LABEL_OPACITY: u16 = 212;
const ID_LABEL_OPACITY_UNIT: u16 = 213;
const ID_HEADER_ADVANCED: u16 = 214;
const ID_SEP_ADVANCED: u16 = 215;
const ID_LABEL_RESYNC_UNIT: u16 = 216;
const ID_LABEL_INACT_UNIT: u16 = 217;
pub(super) const ID_LABEL_LOG_LEVEL: u16 = 218;
const ID_LABEL_LANGUAGE: u16 = 219;

/// Whether `id` is one of the four bold section-header labels, which need
/// `build_font`'s bold variant rather than the regular one every other
/// control gets.
pub(super) fn is_section_header(id: u16) -> bool {
    matches!(
        id,
        ID_HEADER_GENERAL | ID_HEADER_HOTKEYS | ID_HEADER_OSD | ID_HEADER_ADVANCED
    )
}

/// Whether `id` is one of the two explanatory statics that wrap onto more
/// than one line, and so can grow taller when a translation is longer.
/// Every other caption in the window is single-line at a fixed height.
#[must_use]
pub(super) fn wraps(id: u16) -> bool {
    matches!(id, ID_HK_HINT | ID_LOG_HINT)
}

/// Whether `style` is a checkbox's. Checkboxes and pushbuttons share the
/// `BUTTON` class, and only a checkbox spends width on an indicator its
/// caption cannot use, so the style bits are the only way to tell them
/// apart. The button type lives in the low four bits — `BS_PUSHBUTTON` is
/// zero, so the test has to be for the checkbox value being present, not for
/// the pushbutton value being absent.
#[must_use]
#[cfg_attr(
    not(test),
    expect(
        dead_code,
        reason = "the layout gate is the only caller; the painters learn a control's kind from its window handle instead"
    )
)]
pub(super) fn is_checkbox(style: u32) -> bool {
    const BS_TYPEMASK: u32 = 0xF;
    style & BS_TYPEMASK == BS_AUTOCHECKBOX
}

// ─────────────────────────────────────────────────────────────────────────────
// Control-Specific Style Bits
// ─────────────────────────────────────────────────────────────────────────────
// `BUTTON`/`EDIT`/`COMBOBOX`/`STATIC` style bits below are stable, long
// unchanged Win32 constants (`winuser.h`). Kept as local `u32` literals
// rather than importing them (most are already reachable, typed `i32`, from
// the enabled `Win32_UI_WindowsAndMessaging` feature; `SS_LEFT`/
// `SS_ETCHEDHORZ` live only in `Win32_System_SystemServices`, which nothing
// else in this crate enables) — this keeps every style bit combined into a
// `ControlSpec.style: u32` field the same type, with no feature added for
// three numbers.

const BS_AUTOCHECKBOX: u32 = 3;
const BS_PUSHBUTTON: u32 = 0;
const BS_DEFPUSHBUTTON: u32 = 1;
const ES_AUTOHSCROLL: u32 = 128;
const ES_NUMBER: u32 = 8192;
const CBS_DROPDOWNLIST: u32 = 3;
const CBS_HASSTRINGS: u32 = 512;
const SS_LEFT: u32 = 0;
const SS_ETCHEDHORZ: u32 = 16;

const STYLE_LABEL: u32 = SS_LEFT;
const STYLE_SEP: u32 = SS_ETCHEDHORZ;
const STYLE_UPDOWN: u32 = 0;
const STYLE_CHECKBOX: u32 = BS_AUTOCHECKBOX | WS_TABSTOP.0;
const STYLE_CHECKBOX_GROUP: u32 = STYLE_CHECKBOX | WS_GROUP.0;
const STYLE_EDIT: u32 = ES_AUTOHSCROLL | WS_BORDER.0 | WS_TABSTOP.0;
const STYLE_EDIT_NUM: u32 = STYLE_EDIT | ES_NUMBER;
const STYLE_EDIT_NUM_GROUP: u32 = STYLE_EDIT_NUM | WS_GROUP.0;
// The hotkey capture fields (`ID_HK_UP`/`ID_HK_DOWN`) are a custom-drawn
// class, not `EDIT` — `ES_AUTOHSCROLL` is EDIT-specific and inert here, so
// this gets its own style constant rather than borrowing `STYLE_EDIT`'s
// name for a control that isn't one (the border/tabstop/group bits it
// actually needs are identical).
const STYLE_CAPTURE: u32 = WS_BORDER.0 | WS_TABSTOP.0;
const STYLE_CAPTURE_GROUP: u32 = STYLE_CAPTURE | WS_GROUP.0;
const STYLE_COMBO: u32 = CBS_DROPDOWNLIST | CBS_HASSTRINGS | WS_TABSTOP.0;
const STYLE_COMBO_GROUP: u32 = STYLE_COMBO | WS_GROUP.0;
const STYLE_LINK: u32 = WS_TABSTOP.0;
const STYLE_LINK_GROUP: u32 = STYLE_LINK | WS_GROUP.0;
const STYLE_PUSHBUTTON: u32 = BS_PUSHBUTTON | WS_TABSTOP.0;
const STYLE_PUSHBUTTON_GROUP: u32 = STYLE_PUSHBUTTON | WS_GROUP.0;
const STYLE_DEFPUSHBUTTON: u32 = BS_DEFPUSHBUTTON | WS_TABSTOP.0;

// ─────────────────────────────────────────────────────────────────────────────
// Layout Table
// ─────────────────────────────────────────────────────────────────────────────

/// One control's window class, style, 96-DPI-baseline geometry and initial
/// text. `create_controls()` and the layout planner (`plan::plan_layout`,
/// whose output `apply()` then positions) are the only readers; both walk
/// [`CONTROLS`] in order, which is also creation order and therefore tab
/// order (every focusable entry carries `WS_TABSTOP`, and `WS_GROUP` marks
/// the first tab stop of each visual section).
#[derive(Debug, Clone, Copy)]
pub(super) struct ControlSpec {
    pub(super) id: u16,
    pub(super) class: &'static str,
    pub(super) style: u32,
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) w: i32,
    pub(super) h: i32,
    /// The label to show, or `None` for controls whose text is set at runtime
    /// (edit fields, spinners, the version line) or that have none (separators).
    pub(super) text: Option<TextKey>,
    /// How this control's position and width respond when a translation makes
    /// the text around it wider; see `plan`.
    pub(super) anchor: Anchor,
}

/// Base window client size at 96 DPI (100% scaling); see [`scale_dimension`].
pub(super) const BASE_WINDOW_WIDTH: i32 = 400;
pub(super) const BASE_WINDOW_HEIGHT: i32 = 654;

/// Every control in the settings window, at 96-DPI-baseline coordinates, in
/// visual and creation order. See the module doc comment for why group
/// boxes were replaced with a label + separator pair per section.
pub(super) const CONTROLS: &[ControlSpec] = &[
    // ── General ─────────────────────────────────────────────────────────
    ControlSpec {
        id: ID_HEADER_GENERAL,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 12,
        y: 12,
        w: 376,
        h: 16,
        text: Some(TextKey::HeaderGeneral),
        anchor: Anchor::Stretch,
    },
    ControlSpec {
        id: ID_SEP_GENERAL,
        class: "STATIC",
        style: STYLE_SEP,
        x: 12,
        y: 32,
        w: 376,
        h: 2,
        text: None,
        anchor: Anchor::Stretch,
    },
    ControlSpec {
        id: ID_LABEL_LANGUAGE,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 44,
        w: 220,
        h: 20,
        text: Some(TextKey::LabelLanguage),
        anchor: Anchor::Label(Col::B),
    },
    // 120 wide, unlike the log-level combo's 76: the English entry "System
    // default" plus the 17px dropdown arrow does not fit 76 at this font.
    // Each combo is sized to its own content by design — the log-level combo's
    // 76 makes it end at 326, flush with the spinner rows' right edge, and
    // there is no third edge the two could share without breaking either that
    // alignment or this combo's fit.
    ControlSpec {
        id: ID_LANGUAGE,
        class: "COMBOBOX",
        style: STYLE_COMBO_GROUP,
        x: 250,
        y: 42,
        w: 120,
        h: 120,
        text: None,
        anchor: Anchor::Control(Col::B),
    },
    ControlSpec {
        id: ID_AUTOSTART,
        class: "BUTTON",
        style: STYLE_CHECKBOX,
        x: 24,
        y: 72,
        w: 300,
        h: 20,
        text: Some(TextKey::Autostart),
        anchor: Anchor::Stretch,
    },
    ControlSpec {
        id: ID_LABEL_STEP,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 102,
        w: 220,
        h: 20,
        text: Some(TextKey::LabelStep),
        anchor: Anchor::Label(Col::B),
    },
    ControlSpec {
        id: ID_STEP_EDIT,
        class: "EDIT",
        style: STYLE_EDIT_NUM,
        x: 250,
        y: 100,
        w: 60,
        h: 22,
        text: None,
        anchor: Anchor::Control(Col::B),
    },
    ControlSpec {
        id: ID_STEP_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 310,
        y: 100,
        w: 16,
        h: 22,
        text: None,
        anchor: Anchor::AfterControl(Col::B),
    },
    ControlSpec {
        id: ID_LABEL_STEP_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 332,
        y: 102,
        w: 20,
        h: 20,
        text: Some(TextKey::UnitPercentStep),
        anchor: Anchor::AfterControl(Col::B),
    },
    // ── Hotkeys ─────────────────────────────────────────────────────────
    ControlSpec {
        id: ID_HEADER_HOTKEYS,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 12,
        y: 138,
        w: 376,
        h: 16,
        text: Some(TextKey::HeaderHotkeys),
        anchor: Anchor::Stretch,
    },
    ControlSpec {
        id: ID_SEP_HOTKEYS,
        class: "STATIC",
        style: STYLE_SEP,
        x: 12,
        y: 158,
        w: 376,
        h: 2,
        text: None,
        anchor: Anchor::Stretch,
    },
    ControlSpec {
        id: ID_LABEL_HK_UP,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 170,
        w: 140,
        h: 20,
        text: Some(TextKey::LabelHotkeyUp),
        anchor: Anchor::Label(Col::A),
    },
    ControlSpec {
        id: ID_HK_UP,
        class: "HOTKEY_CAPTURE",
        style: STYLE_CAPTURE_GROUP,
        x: 170,
        y: 168,
        w: 218,
        h: 22,
        text: None,
        anchor: Anchor::ControlStretch(Col::A),
    },
    ControlSpec {
        id: ID_LABEL_HK_DOWN,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 200,
        w: 140,
        h: 20,
        text: Some(TextKey::LabelHotkeyDown),
        anchor: Anchor::Label(Col::A),
    },
    ControlSpec {
        id: ID_HK_DOWN,
        class: "HOTKEY_CAPTURE",
        style: STYLE_CAPTURE,
        x: 170,
        y: 198,
        w: 218,
        h: 22,
        text: None,
        anchor: Anchor::ControlStretch(Col::A),
    },
    ControlSpec {
        id: ID_INTERCEPT,
        class: "BUTTON",
        style: STYLE_CHECKBOX,
        x: 24,
        y: 228,
        w: 340,
        h: 20,
        text: Some(TextKey::Intercept),
        anchor: Anchor::Stretch,
    },
    // Muted explainer text under the intercept checkbox.
    ControlSpec {
        id: ID_HK_HINT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 36,
        y: 252,
        w: 328,
        h: 34,
        text: Some(TextKey::HintIntercept),
        anchor: Anchor::Stretch,
    },
    // Inline hotkey status line, empty until handle_hotkey_message_text sets
    // it; whether it renders as an error (red) or a notice (muted) is a
    // colour decided by the control-colour handler, not by this table.
    ControlSpec {
        id: ID_HK_ERROR,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 292,
        w: 340,
        h: 16,
        text: None,
        anchor: Anchor::Stretch,
    },
    // ── On-screen display ───────────────────────────────────────────────
    ControlSpec {
        id: ID_HEADER_OSD,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 12,
        y: 324,
        w: 376,
        h: 16,
        text: Some(TextKey::HeaderOsd),
        anchor: Anchor::Stretch,
    },
    ControlSpec {
        id: ID_SEP_OSD,
        class: "STATIC",
        style: STYLE_SEP,
        x: 12,
        y: 344,
        w: 376,
        h: 2,
        text: None,
        anchor: Anchor::Stretch,
    },
    ControlSpec {
        id: ID_LABEL_TIMEOUT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 356,
        w: 140,
        h: 20,
        text: Some(TextKey::LabelTimeout),
        anchor: Anchor::Label(Col::B),
    },
    ControlSpec {
        id: ID_OSD_TIMEOUT_EDIT,
        class: "EDIT",
        style: STYLE_EDIT_NUM_GROUP,
        x: 250,
        y: 354,
        w: 60,
        h: 22,
        text: None,
        anchor: Anchor::Control(Col::B),
    },
    ControlSpec {
        id: ID_OSD_TIMEOUT_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 310,
        y: 354,
        w: 16,
        h: 22,
        text: None,
        anchor: Anchor::AfterControl(Col::B),
    },
    ControlSpec {
        id: ID_LABEL_TIMEOUT_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 332,
        y: 356,
        w: 30,
        h: 20,
        text: Some(TextKey::UnitMilliseconds),
        anchor: Anchor::AfterControl(Col::B),
    },
    ControlSpec {
        id: ID_LABEL_OPACITY,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 386,
        w: 140,
        h: 20,
        text: Some(TextKey::LabelOpacity),
        anchor: Anchor::Label(Col::B),
    },
    ControlSpec {
        id: ID_OSD_OPACITY_EDIT,
        class: "EDIT",
        style: STYLE_EDIT_NUM,
        x: 250,
        y: 384,
        w: 60,
        h: 22,
        text: None,
        anchor: Anchor::Control(Col::B),
    },
    ControlSpec {
        id: ID_OSD_OPACITY_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 310,
        y: 384,
        w: 16,
        h: 22,
        text: None,
        anchor: Anchor::AfterControl(Col::B),
    },
    ControlSpec {
        id: ID_LABEL_OPACITY_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 332,
        y: 386,
        w: 20,
        h: 20,
        text: Some(TextKey::UnitPercentOpacity),
        anchor: Anchor::AfterControl(Col::B),
    },
    // ── Advanced ────────────────────────────────────────────────────────
    ControlSpec {
        id: ID_HEADER_ADVANCED,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 12,
        y: 422,
        w: 376,
        h: 16,
        text: Some(TextKey::HeaderAdvanced),
        anchor: Anchor::Stretch,
    },
    ControlSpec {
        id: ID_SEP_ADVANCED,
        class: "STATIC",
        style: STYLE_SEP,
        x: 12,
        y: 442,
        w: 376,
        h: 2,
        text: None,
        anchor: Anchor::Stretch,
    },
    ControlSpec {
        id: ID_RESYNC_CHECK,
        class: "BUTTON",
        style: STYLE_CHECKBOX_GROUP,
        x: 24,
        y: 452,
        w: 220,
        h: 20,
        text: Some(TextKey::ResyncCheck),
        anchor: Anchor::Checkbox(Col::B),
    },
    ControlSpec {
        id: ID_RESYNC_EDIT,
        class: "EDIT",
        style: STYLE_EDIT_NUM,
        x: 250,
        y: 450,
        w: 60,
        h: 22,
        text: None,
        anchor: Anchor::Control(Col::B),
    },
    ControlSpec {
        id: ID_RESYNC_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 310,
        y: 450,
        w: 16,
        h: 22,
        text: None,
        anchor: Anchor::AfterControl(Col::B),
    },
    ControlSpec {
        id: ID_LABEL_RESYNC_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 332,
        y: 452,
        w: 20,
        h: 20,
        text: Some(TextKey::UnitSecondsResync),
        anchor: Anchor::AfterControl(Col::B),
    },
    ControlSpec {
        id: ID_INACT_CHECK,
        class: "BUTTON",
        style: STYLE_CHECKBOX,
        x: 24,
        y: 482,
        w: 220,
        h: 20,
        text: Some(TextKey::InactivityCheck),
        anchor: Anchor::Checkbox(Col::B),
    },
    ControlSpec {
        id: ID_INACT_EDIT,
        class: "EDIT",
        style: STYLE_EDIT_NUM,
        x: 250,
        y: 480,
        w: 60,
        h: 22,
        text: None,
        anchor: Anchor::Control(Col::B),
    },
    ControlSpec {
        id: ID_INACT_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 310,
        y: 480,
        w: 16,
        h: 22,
        text: None,
        anchor: Anchor::AfterControl(Col::B),
    },
    ControlSpec {
        id: ID_LABEL_INACT_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 332,
        y: 482,
        w: 20,
        h: 20,
        text: Some(TextKey::UnitSecondsInactivity),
        anchor: Anchor::AfterControl(Col::B),
    },
    ControlSpec {
        id: ID_LOG_CHECK,
        class: "BUTTON",
        style: STYLE_CHECKBOX,
        x: 24,
        y: 512,
        w: 140,
        h: 20,
        text: Some(TextKey::LogCheck),
        anchor: Anchor::CheckboxRun,
    },
    // Same y as ID_LOG_CHECK's, not offset for its checkbox's own
    // vertical centering: a BUTTON checkbox centering its label per the
    // textbook DT_VCENTER arithmetic would put "Level:" a few pixels below
    // "Write log file", but that model does not match what actually
    // renders. Measured on hardware at 125% DPI (comparing capital-letter
    // tops, since both strings start with one): at y:512 the two already
    // read as aligned (647 vs 648 physical px, 1px); an earlier attempt to
    // "correct" this to y:515 using the centering model instead put them
    // 5px apart (647 vs 652). Left at the measured-aligned value.
    ControlSpec {
        id: ID_LABEL_LOG_LEVEL,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 170,
        y: 512,
        w: 74,
        h: 20,
        text: Some(TextKey::LabelLogLevel),
        anchor: Anchor::InlineLabel,
    },
    // The combo's `h` is the height of the *dropped-down* list, a Win32
    // quirk: the closed control renders at the font's line height regardless
    // of this value, which is why the planner's overlap check exempts the
    // class.
    // `w: 76` matches the numeric edit+updown pair's combined width (60 +
    // 16) so the combo's right edge lands exactly where the spinner rows'
    // updown buttons end (250 + 76 = 326, same as 250 + 60 + 16 on those
    // rows); the longest entry ("debug", ~34px at this font) still leaves
    // headroom against the 17px system dropdown-arrow width at that size.
    ControlSpec {
        id: ID_LOG_LEVEL,
        class: "COMBOBOX",
        style: STYLE_COMBO,
        x: 250,
        y: 510,
        w: 76,
        h: 120,
        text: None,
        anchor: Anchor::Control(Col::B),
    },
    ControlSpec {
        id: ID_LOG_HINT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 36,
        y: 536,
        w: 340,
        h: 32,
        text: Some(TextKey::HintLogging),
        anchor: Anchor::Stretch,
    },
    // ── Footer ──────────────────────────────────────────────────────────
    // One SysLink carries both links as flowing text, `iLink` (0 = config
    // file, 1 = log folder) telling window.rs's NM_CLICK handler which one
    // was clicked; the middot between them is part of that same flow, not a
    // separate control. SysLink markup: the visible text is exactly the
    // spec's wording; the `<a>` tags are SysLink's own syntax for "this span
    // is the hyperlink", not additional user-facing text.
    // The row has always had the full width between the margins to draw in;
    // the authored 250 was simply too narrow for its own text, which needs 177
    // in English and 272 in German.
    ControlSpec {
        id: ID_LINK_CONFIG,
        class: "SysLink",
        style: STYLE_LINK_GROUP,
        x: 12,
        y: 584,
        w: 376,
        h: 20,
        text: Some(TextKey::FooterLinks),
        anchor: Anchor::Stretch,
    },
    ControlSpec {
        id: ID_RESTORE,
        class: "BUTTON",
        style: STYLE_PUSHBUTTON_GROUP,
        x: 190,
        y: 616,
        w: 110,
        h: 26,
        text: Some(TextKey::ButtonRestoreDefaults),
        anchor: Anchor::FooterButton,
    },
    ControlSpec {
        id: ID_CLOSE,
        class: "BUTTON",
        style: STYLE_DEFPUSHBUTTON,
        x: 308,
        y: 616,
        w: 80,
        h: 26,
        text: Some(TextKey::ButtonClose),
        anchor: Anchor::FooterButton,
    },
    // Which build is running, in the free space left of the buttons and
    // vertically centred against them. `text` is `None` because the string is
    // only known at build time: `window`'s control creation substitutes
    // `core::version` output for this one id.
    //
    // `w` is what is left before "Restore defaults" starts; the planner
    // recomputes it from the measured version string, and widens the window
    // when a development build's longer string does not fit.
    ControlSpec {
        id: ID_VERSION,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 12,
        y: 620,
        w: 172,
        h: 18,
        text: None,
        anchor: Anchor::FooterFill,
    },
];

/// Scales a 96-DPI-baseline pixel value to `dpi`, `MulDiv`-style
/// (`px * dpi / 96`, truncating). Plain integer arithmetic in `i64` to avoid
/// overflow at any DPI Windows actually reports, kept pure so it — and the
/// whole [`CONTROLS`] table through it — is unit-testable without a live
/// display.
#[must_use]
pub(super) fn scale_dimension(px: i32, dpi: u32) -> i32 {
    let scaled = i64::from(px) * i64::from(dpi) / 96;
    i32::try_from(scaled).unwrap_or(px)
}

/// Scales a point size to a negative `LOGFONT.lfHeight` for `dpi`
/// (`-(points * dpi / 72)` — 72, not 96, since points are physically 1/72
/// inch; the DPI-baseline geometry above and font sizing use different
/// reference units for that reason).
#[must_use]
pub(super) fn font_height_for_dpi(point_size: i32, dpi: u32) -> i32 {
    let scaled = i64::from(point_size) * i64::from(dpi) / 72;
    -i32::try_from(scaled).unwrap_or(point_size)
}

/// Extracts the new DPI from a `WM_DPICHANGED` `wParam`: the low word is the
/// X-axis DPI, the high word the Y-axis one — Windows always reports the
/// same value in both for a system DPI change, so only the low word is
/// read. Takes the raw `wParam.0` rather than a `WPARAM` so it stays plain
/// arithmetic, testable without a live window.
#[must_use]
pub(super) fn dpi_from_wparam(wparam: usize) -> u32 {
    // Masked to 0xFFFF, so this always fits a u32 regardless of usize's
    // width — the conversion cannot fail.
    u32::try_from(wparam & 0xFFFF).expect("value masked to 0xFFFF always fits in u32")
}

/// The version line's caption, in the form the window actually shows it.
///
/// The label's text and the width the planner reserves for it both come
/// from this one string, so the two cannot drift into a build whose version
/// renders wider than its slot.
#[must_use]
pub(super) fn version_label() -> String {
    format!("v{}", version_string())
}

/// Positions every control from `plan`, in one batch.
///
/// `SWP_NOZORDER` on every call is load-bearing rather than a default:
/// z-order is tab order in this window (creation order, see [`CONTROLS`]),
/// so a reordering here would silently change which control Tab reaches
/// next, with nothing to catch it.
///
/// A lost `HDWP` from either `BeginDeferWindowPos` or `DeferWindowPos`
/// discards the whole batch, and at creation — where every control is still
/// at (0,0,0,0) — that would be a blank window. The per-control fallback
/// degrades one control at a time instead, matching how the rest of this
/// module fails.
pub(super) fn apply(hwnd: HWND, plan: &Plan) {
    if !apply_batched(hwnd, plan) {
        apply_individually(hwnd, plan);
    }
}

/// Queues every move into one `HDWP`. Returns `false` if the batch was lost
/// at any point, in which case nothing has moved and the caller falls back.
fn apply_batched(hwnd: HWND, plan: &Plan) -> bool {
    let count = i32::try_from(plan.controls.len()).unwrap_or(0);
    let Ok(mut hdwp) = (unsafe { BeginDeferWindowPos(count) }) else {
        log::warn!("BeginDeferWindowPos failed; positioning settings controls individually");
        return false;
    };
    for placed in &plan.controls {
        let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(placed.id)) }) else {
            continue;
        };
        match unsafe {
            DeferWindowPos(
                hdwp,
                child,
                None,
                placed.x,
                placed.y,
                placed.w,
                placed.h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            )
        } {
            Ok(next) => hdwp = next,
            Err(e) => {
                log::warn!(error:% = e; "DeferWindowPos failed; positioning settings controls individually");
                return false;
            }
        }
    }
    if let Err(e) = unsafe { EndDeferWindowPos(hdwp) } {
        log::warn!(error:% = e; "EndDeferWindowPos failed; positioning settings controls individually");
        return false;
    }
    true
}

/// One `SetWindowPos` per control, for when the batched pass lost its
/// `HDWP`.
fn apply_individually(hwnd: HWND, plan: &Plan) {
    for placed in &plan.controls {
        let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(placed.id)) }) else {
            continue;
        };
        unsafe {
            let _ = SetWindowPos(
                child,
                None,
                placed.x,
                placed.y,
                placed.w,
                placed.h,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }
}

/// Measures `lang`'s captions, plans the layout for `dpi`, applies it, and
/// resizes the window's client area to match.
///
/// Creation does not call this — it plans before `CreateWindowExW` so the
/// window is never the wrong size, not even for a frame. `WM_DPICHANGED`
/// and a language change do, because both invalidate every measurement.
pub(super) fn relayout(hwnd: HWND, dpi: u32, lang: Lang) {
    let Some(mut measure) = GdiMeasure::new(dpi) else {
        log::warn!(dpi; "No measurement context; leaving the settings layout as it is");
        return;
    };
    let plan = plan_layout(lang, dpi, &version_label(), &mut measure);
    apply(hwnd, &plan);
    resize_client(hwnd, &plan);
}

/// Grows or shrinks the window's frame to hold `plan`'s client area, without
/// moving it.
///
/// Never moving is deliberate and covers both callers: a DPI change arrives
/// with a position Windows suggested and the handler has already applied,
/// and a language change must leave a window the user dragged exactly where
/// they put it. The one place that chooses a position is the initial
/// placement, which runs before the window exists.
fn resize_client(hwnd: HWND, plan: &Plan) {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: plan.client_w,
        bottom: plan.client_h,
    };
    if let Err(e) = unsafe {
        AdjustWindowRectExForDpi(
            &raw mut rect,
            SETTINGS_STYLE,
            false,
            SETTINGS_EX_STYLE,
            plan.dpi,
        )
    } {
        log::warn!(error:% = e; "AdjustWindowRectExForDpi failed; leaving the settings frame size alone");
        return;
    }
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            None,
            0,
            0,
            rect.right - rect.left,
            rect.bottom - rect.top,
            SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
        );
    }
}

/// One spinner's valid range and the id of the up-down control it belongs
/// to. `window::NumericField` references one of these by pointer instead of
/// carrying its own copy, so the range lives in exactly one place — this
/// table — for both `configure_updowns` and the instant-apply clamping
/// `window.rs` does on every commit.
#[derive(Debug)]
pub(super) struct RangeSpec {
    pub(super) updown_id: u16,
    pub(super) min: u32,
    pub(super) max: u32,
}

/// Valid range for every spinner, in [`CONTROLS`] order; these also mirror
/// what `core/config.rs`'s validator accepts for the matching config field.
pub(super) const RANGE_SPECS: &[RangeSpec] = &[
    // Brightness step per keypress.
    RangeSpec {
        updown_id: ID_STEP_UPDOWN,
        min: 1,
        max: 50,
    },
    // OSD auto-hide timeout, in milliseconds.
    RangeSpec {
        updown_id: ID_OSD_TIMEOUT_UPDOWN,
        min: 100,
        max: 10_000,
    },
    // OSD opacity percentage.
    RangeSpec {
        updown_id: ID_OSD_OPACITY_UPDOWN,
        min: 10,
        max: 100,
    },
    // Periodic-resync edit while its checkbox is checked (0, meaning
    // disabled, only ever arrives through the checkbox itself — see
    // `window::NumericField::checkbox_id`).
    RangeSpec {
        updown_id: ID_RESYNC_UPDOWN,
        min: 1,
        max: 3600,
    },
    // Inactivity-resync edit while its checkbox is checked; same
    // 0-is-checkbox-only rule as the periodic-resync entry above.
    RangeSpec {
        updown_id: ID_INACT_UPDOWN,
        min: 1,
        max: 600,
    },
];

/// Sets an up-down spinner's `UDM_SETRANGE32` range. No-op if `id` was never
/// created (best-effort, matching `create_controls`).
fn set_updown_range(hwnd: HWND, id: u16, min: i32, max: i32) {
    let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(id)) }) else {
        return;
    };
    unsafe {
        SendMessageW(
            child,
            UDM_SETRANGE32,
            Some(WPARAM(usize::try_from(min).unwrap_or(0))),
            Some(LPARAM(isize::try_from(max).unwrap_or(0))),
        );
    }
}

/// Sets an up-down spinner's `UDM_SETACCEL` acceleration table.
fn set_updown_accel(hwnd: HWND, id: u16, accel: &[UDACCEL]) {
    let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(id)) }) else {
        return;
    };
    // SAFETY: `UDM_SETACCEL` reads `wparam` entries from the `UDACCEL` array
    // `lparam` points at; both come from the same slice here, so the control
    // cannot read past it. The message is sent, not posted, so `accel` is
    // still borrowed when the control copies the table.
    unsafe {
        SendMessageW(
            child,
            UDM_SETACCEL,
            Some(WPARAM(accel.len())),
            Some(LPARAM(accel.as_ptr().expose_provenance().cast_signed())),
        );
    }
}

/// Applies every spinner's range straight from [`RANGE_SPECS`] — the
/// single source of truth for those five ranges — plus, for the OSD
/// timeout, its acceleration table; that one entry has no home in the table
/// itself, since none of the other four spinners need one.
pub(super) fn configure_updowns(hwnd: HWND) {
    for spec in RANGE_SPECS {
        set_updown_range(
            hwnd,
            spec.updown_id,
            i32::try_from(spec.min).unwrap_or(0),
            i32::try_from(spec.max).unwrap_or(i32::MAX),
        );
    }
    set_updown_accel(
        hwnd,
        ID_OSD_TIMEOUT_UPDOWN,
        &[UDACCEL { nSec: 0, nInc: 100 }],
    );
}

/// Matches one dropdown's closed (undropped) face height to the numeric
/// edit rows' 22-logical-px height. Left alone, a combo's system-default
/// closed face is 1-2px taller than an edit at the same font, a visible
/// seam in that row.
///
/// `CB_SETITEMHEIGHT`'s index `-1` sets the closed-selection field only —
/// the dropdown *list* row height (index `0`) is untouched, so the entries
/// still lay out at their own default height when the list opens.
///
/// The relationship between item height and the resulting closed-face
/// height is fixed chrome (borders/margins) added on top of the item
/// height, not a ratio, so it is measured from the combo's own
/// system-default sizing for the font just applied — `default_closed - default_item`
/// — rather than assumed as a constant: that stays correct across DPI and
/// visual-style versions instead of encoding today's measured pixel count.
/// Call once after [`layout`] has positioned both controls (creation, and
/// again after `WM_DPICHANGED` rebuilds the fonts) so the reference edit's
/// rect already reflects the current DPI.
///
/// A themed combo box can refuse a request that goes below its own
/// visual-styles-driven minimum for the current font — `CB_SETITEMHEIGHT`
/// then either fails outright or silently clamps back up — so this reads
/// the result back (`CB_GETITEMHEIGHT` plus a second `GetWindowRect`) and
/// logs the outcome at debug rather than trusting the request took effect.
/// On the one DPI/font combination measured on hardware so far, the
/// requested height was one pixel below the system default and the closed
/// face did not move, consistent with that floor being hit; the mechanism
/// is kept regardless — it still narrows or removes the seam at other
/// DPI/font combinations, and does no harm where it clamps.
fn configure_one_combo_height(hwnd: HWND, combo_id: u16) {
    let (Ok(combo), Ok(edit)) = (
        unsafe { GetDlgItem(Some(hwnd), i32::from(combo_id)) },
        unsafe { GetDlgItem(Some(hwnd), i32::from(ID_STEP_EDIT)) },
    ) else {
        return;
    };

    let default_item_height =
        unsafe { SendMessageW(combo, CB_GETITEMHEIGHT, Some(WPARAM(usize::MAX)), None) }.0;
    let Ok(default_item_height) = i32::try_from(default_item_height) else {
        return;
    };

    let mut combo_rect = RECT::default();
    let mut edit_rect = RECT::default();
    let (Ok(()), Ok(())) = (
        unsafe { GetWindowRect(combo, &raw mut combo_rect) },
        unsafe { GetWindowRect(edit, &raw mut edit_rect) },
    ) else {
        return;
    };

    let default_closed_height = combo_rect.bottom - combo_rect.top;
    let target_height = edit_rect.bottom - edit_rect.top;
    let chrome = default_closed_height - default_item_height;
    // At least 1px: a zero or negative item height would be nonsensical and
    // CB_SETITEMHEIGHT rejects it outright.
    let item_height = (target_height - chrome).max(1);

    let set_result = unsafe {
        SendMessageW(
            combo,
            CB_SETITEMHEIGHT,
            Some(WPARAM(usize::MAX)),
            Some(LPARAM(isize::try_from(item_height).unwrap_or(0))),
        )
    }
    .0;

    // CB_ERR (-1): the combo rejected the request outright (bad index or
    // height) rather than clamping it — worth its own line, distinct from
    // a silent clamp, which the readback below catches instead.
    if set_result == -1 {
        log::debug!(
            combo_id,
            requested_item_height = item_height;
            "CB_SETITEMHEIGHT rejected the combo's closed-face height request"
        );
        return;
    }

    let applied_item_height =
        unsafe { SendMessageW(combo, CB_GETITEMHEIGHT, Some(WPARAM(usize::MAX)), None) }.0;
    let mut after_rect = RECT::default();
    let applied_closed_height = unsafe { GetWindowRect(combo, &raw mut after_rect) }
        .is_ok()
        .then(|| after_rect.bottom - after_rect.top);
    log::debug!(
        combo_id,
        default_item_height,
        default_closed_height,
        target_height,
        requested_item_height = item_height,
        applied_item_height,
        applied_closed_height:? = applied_closed_height;
        "Combo closed-face height applied"
    );
}

/// Both dropdowns: see [`configure_one_combo_height`].
pub(super) fn configure_combo_height(hwnd: HWND) {
    for id in [ID_LANGUAGE, ID_LOG_LEVEL] {
        configure_one_combo_height(hwnd, id);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Placement
// ─────────────────────────────────────────────────────────────────────────────

/// The settings window's outer rect (position + size, in screen
/// coordinates). A named struct rather than a same-typed tuple: `x`/`y`/
/// `w`/`h` all being `i32` makes a positional tuple a transposition hazard
/// worth avoiding even though [`compute_placement`] is currently its only
/// call site.
pub(super) struct Placement {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) w: i32,
    pub(super) h: i32,
}

/// The monitor the settings window will open on: the work area to centre
/// on and the DPI to plan and measure for.
///
/// Resolved once and then passed around, because the layout has to be
/// planned at the target DPI *before* a placement can be computed from its
/// size — resolving the monitor twice would risk answering for two
/// different ones if the cursor moved in between.
pub(super) struct TargetMonitor {
    pub(super) dpi: u32,
    /// `rcWork`, which excludes the taskbar — the same area shell dialogs
    /// centre on.
    work: RECT,
}

/// Resolves the monitor under the cursor.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if the cursor position cannot be
/// read. A monitor that reports neither its DPI nor its work area is not
/// fatal: 96 DPI and an empty work area still place a usable window.
pub(super) fn target_monitor() -> Result<TargetMonitor> {
    let mut cursor = POINT::default();
    unsafe { GetCursorPos(&raw mut cursor) }
        .map_err(|e| BrightnessError::windows_api("GetCursorPos", e.code().0.cast_unsigned()))?;

    let hmonitor: HMONITOR = unsafe { MonitorFromPoint(cursor, MONITOR_DEFAULTTONEAREST) };

    let mut dpi_x = 96u32;
    let mut dpi_y = 96u32;
    if let Err(e) =
        unsafe { GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &raw mut dpi_x, &raw mut dpi_y) }
    {
        log::warn!(error:% = e; "GetDpiForMonitor failed, assuming 96 DPI");
    }

    let mut mi = MONITORINFO {
        cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).unwrap_or(0),
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(hmonitor, &raw mut mi) }.as_bool() {
        log::warn!(error_code = super::super::get_last_error_code(); "GetMonitorInfoW failed; placement may be off-screen");
    }

    Ok(TargetMonitor {
        dpi: dpi_x,
        work: mi.rcWork,
    })
}

/// Top-left corner for a window of `outer_w` × `outer_h` centred on `work`,
/// clamped so the corner stays inside even when the window is larger than
/// the work area — this window is unusually tall, and at high DPI on a
/// short work area (150% on 1920x1080, say) that upper bound is what
/// actually binds.
#[must_use]
fn clamp_into(work: RECT, outer_w: i32, outer_h: i32) -> (i32, i32) {
    let x_centered = work.left + ((work.right - work.left) - outer_w) / 2;
    let y_centered = work.top + ((work.bottom - work.top) - outer_h) / 2;
    (
        x_centered.clamp(work.left, (work.right - outer_w).max(work.left)),
        y_centered.clamp(work.top, (work.bottom - outer_h).max(work.top)),
    )
}

/// Computes [`Placement`] for a client area of `client_w` × `client_h` on
/// `target`: the frame that holds it, centred on the monitor's work area
/// and clamped into it by [`clamp_into`].
///
/// The client size is the planner's, not a constant, and the whole geometry
/// is computed before `CreateWindowExW` — new sequencing versus `osd.rs`,
/// which creates once with `CW_USEDEFAULT` and positions per show; this
/// window instead opens at its final measured size rather than being
/// resized into it afterwards.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if the frame size cannot be
/// derived from the client size.
pub(super) fn compute_placement(
    target: &TargetMonitor,
    client_w: i32,
    client_h: i32,
) -> Result<Placement> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    unsafe {
        AdjustWindowRectExForDpi(
            &raw mut rect,
            SETTINGS_STYLE,
            false,
            SETTINGS_EX_STYLE,
            target.dpi,
        )
    }
    .map_err(|e| {
        BrightnessError::windows_api("AdjustWindowRectExForDpi", e.code().0.cast_unsigned())
    })?;

    let outer_w = rect.right - rect.left;
    let outer_h = rect.bottom - rect.top;
    let (x, y) = clamp_into(target.work, outer_w, outer_h);

    Ok(Placement {
        x,
        y,
        w: outer_w,
        h: outer_h,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_is_identity_at_96_dpi() {
        for spec in CONTROLS {
            assert_eq!(scale_dimension(spec.x, 96), spec.x);
            assert_eq!(scale_dimension(spec.y, 96), spec.y);
            assert_eq!(scale_dimension(spec.w, 96), spec.w);
            assert_eq!(scale_dimension(spec.h, 96), spec.h);
        }
    }

    #[test]
    fn scaling_at_125_percent_matches_the_mul_div_formula_on_a_few_values() {
        // 120 DPI = 125% scaling; px * 120 / 96 = px * 1.25. These three
        // values happen to divide evenly — a property of these particular
        // numbers, not a claim about every entry in CONTROLS (several of
        // which do not, e.g. x: 250, x: 310, h: 22).
        assert_eq!(scale_dimension(400, 120), 500);
        assert_eq!(scale_dimension(24, 120), 30);
        assert_eq!(scale_dimension(16, 120), 20);
    }

    #[test]
    fn scaling_at_150_percent_matches_the_mul_div_formula_on_a_few_values() {
        // 144 DPI = 150% scaling; px * 144 / 96 = px * 1.5. Same caveat as
        // the 125% case above: these three values divide evenly; that is
        // not true of the whole table.
        assert_eq!(scale_dimension(400, 144), 600);
        assert_eq!(scale_dimension(24, 144), 36);
        assert_eq!(scale_dimension(16, 144), 24);
    }

    #[test]
    fn every_dimension_scales_to_a_sane_positive_size_at_125_and_150_percent() {
        // A real property over the whole table (not just hand-picked
        // values): positions never go negative and every width/height stays
        // positive after scaling, at both DPIs the layout is expected to
        // run at.
        for dpi in [120, 144] {
            for spec in CONTROLS {
                assert!(
                    scale_dimension(spec.x, dpi) >= 0,
                    "{spec:?} x went negative at {dpi} dpi"
                );
                assert!(
                    scale_dimension(spec.y, dpi) >= 0,
                    "{spec:?} y went negative at {dpi} dpi"
                );
                assert!(
                    scale_dimension(spec.w, dpi) > 0,
                    "{spec:?} w is not positive at {dpi} dpi"
                );
                assert!(
                    scale_dimension(spec.h, dpi) > 0,
                    "{spec:?} h is not positive at {dpi} dpi"
                );
            }
        }
    }

    #[test]
    fn font_height_scales_from_points_using_a_72_dpi_reference() {
        // 9pt at 96 DPI: 9 * 96 / 72 = 12, negated (LOGFONT character-height
        // convention).
        assert_eq!(font_height_for_dpi(9, 96), -12);
        // 9pt at 144 DPI (150%): 9 * 144 / 72 = 18.
        assert_eq!(font_height_for_dpi(9, 144), -18);
    }

    #[test]
    fn dpi_from_wparam_reads_the_low_word_only() {
        // 120 = 125% DPI in the low word; a nonzero high word (Y-axis DPI,
        // always equal in practice) must not leak into the result.
        assert_eq!(dpi_from_wparam(0x0078_0078), 120);
        // Asymmetric words: a wrong-word extraction would return 0xFF here,
        // which the symmetric case above cannot detect.
        assert_eq!(dpi_from_wparam(0x00FF_0078), 120);
        assert_eq!(dpi_from_wparam(96), 96);
        assert_eq!(dpi_from_wparam(144), 144);
    }

    #[test]
    fn the_work_area_clamp_keeps_a_window_inside_it() {
        // Work area 0..1000 x 0..800, window larger than it on both axes:
        // the top-left corner must stay inside rather than being pushed
        // above and left of it.
        let work = RECT {
            left: 0,
            top: 0,
            right: 1000,
            bottom: 800,
        };
        assert_eq!(clamp_into(work, 400, 654), (300, 73));
        assert_eq!(clamp_into(work, 1200, 900), (0, 0));
    }

    #[test]
    fn every_control_id_is_unique() {
        let mut ids: Vec<u16> = CONTROLS.iter().map(|c| c.id).collect();
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "duplicate control id in CONTROLS");
    }

    #[test]
    fn every_brief_mandated_id_is_present_in_the_table() {
        let ids: std::collections::HashSet<u16> = CONTROLS.iter().map(|c| c.id).collect();
        for id in [
            ID_LANGUAGE,
            ID_AUTOSTART,
            ID_STEP_EDIT,
            ID_STEP_UPDOWN,
            ID_HK_UP,
            ID_HK_DOWN,
            ID_INTERCEPT,
            ID_HK_HINT,
            ID_HK_ERROR,
            ID_OSD_TIMEOUT_EDIT,
            ID_OSD_TIMEOUT_UPDOWN,
            ID_OSD_OPACITY_EDIT,
            ID_OSD_OPACITY_UPDOWN,
            ID_RESYNC_CHECK,
            ID_RESYNC_EDIT,
            ID_RESYNC_UPDOWN,
            ID_INACT_CHECK,
            ID_INACT_EDIT,
            ID_INACT_UPDOWN,
            ID_LOG_CHECK,
            ID_LOG_LEVEL,
            ID_LOG_HINT,
            ID_LINK_CONFIG,
            ID_RESTORE,
            ID_CLOSE,
            ID_VERSION,
        ] {
            assert!(
                ids.contains(&id),
                "brief-mandated id {id} missing from CONTROLS"
            );
        }
    }

    #[test]
    fn the_language_row_leads_the_general_section_and_shifts_the_rest_by_one_row() {
        let spec = |id: u16| {
            CONTROLS
                .iter()
                .find(|c| c.id == id)
                .expect("control exists")
        };
        let language = spec(ID_LANGUAGE);
        assert_eq!((language.x, language.y, language.w), (250, 42, 120));
        assert_eq!(language.class, "COMBOBOX");
        assert!(
            language.style & WS_GROUP.0 != 0,
            "first tab stop of the section"
        );
        assert_eq!(spec(ID_AUTOSTART).y, 72);
        assert_eq!(spec(ID_AUTOSTART).style & WS_GROUP.0, 0);
        assert_eq!(spec(ID_STEP_EDIT).y, 100);
        assert_eq!(spec(ID_LOG_LEVEL).y, 510);
        assert_eq!(spec(ID_CLOSE).y, 616);
        assert_eq!(BASE_WINDOW_HEIGHT, 654);
    }

    #[test]
    fn every_range_spec_updown_id_is_a_real_control_with_a_sane_range() {
        let ids: std::collections::HashSet<u16> = CONTROLS.iter().map(|c| c.id).collect();
        for spec in RANGE_SPECS {
            assert!(
                ids.contains(&spec.updown_id),
                "RangeSpec.updown_id {} has no CONTROLS entry",
                spec.updown_id
            );
            assert!(
                spec.min <= spec.max,
                "RangeSpec for updown {} has min > max",
                spec.updown_id
            );
        }
    }
}
