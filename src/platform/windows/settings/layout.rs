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
    GetCursorPos, GetDlgItem, SWP_NOACTIVATE, SWP_NOZORDER, SendMessageW, SetWindowPos, WS_BORDER,
    WS_CAPTION, WS_EX_TOPMOST, WS_GROUP, WS_SYSMENU, WS_TABSTOP,
};

use crate::error::{BrightnessError, Result};

// ─────────────────────────────────────────────────────────────────────────────
// Control IDs
// ─────────────────────────────────────────────────────────────────────────────
// 100s = General, 110s = Hotkeys, 120s = On-screen display, 130s = Advanced,
// 140s = footer. 200+ are decorative (section headers, separators, plain
// text labels) — never addressed individually outside this module, but each
// still needs a distinct id: `layout()` positions every control by
// `GetDlgItem(hwnd, id)`, which only ever finds the first match, so a shared
// id would silently move just one control instead of all of them.

pub(super) const ID_AUTOSTART: u16 = 100;
pub(super) const ID_STEP_EDIT: u16 = 101;
pub(super) const ID_STEP_UPDOWN: u16 = 102;

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

const ID_HEADER_GENERAL: u16 = 200;
const ID_SEP_GENERAL: u16 = 201;
const ID_LABEL_STEP: u16 = 202;
const ID_LABEL_STEP_UNIT: u16 = 203;
const ID_HEADER_HOTKEYS: u16 = 204;
const ID_SEP_HOTKEYS: u16 = 205;
const ID_LABEL_HK_UP: u16 = 206;
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
const ID_LABEL_LOG_LEVEL: u16 = 218;

/// Whether `id` is one of the four bold section-header labels, which need
/// `build_font`'s bold variant rather than the regular one every other
/// control gets.
pub(super) fn is_section_header(id: u16) -> bool {
    matches!(
        id,
        ID_HEADER_GENERAL | ID_HEADER_HOTKEYS | ID_HEADER_OSD | ID_HEADER_ADVANCED
    )
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
const STYLE_LINK: u32 = WS_TABSTOP.0;
const STYLE_LINK_GROUP: u32 = STYLE_LINK | WS_GROUP.0;
const STYLE_PUSHBUTTON: u32 = BS_PUSHBUTTON | WS_TABSTOP.0;
const STYLE_PUSHBUTTON_GROUP: u32 = STYLE_PUSHBUTTON | WS_GROUP.0;
const STYLE_DEFPUSHBUTTON: u32 = BS_DEFPUSHBUTTON | WS_TABSTOP.0;

// ─────────────────────────────────────────────────────────────────────────────
// Layout Table
// ─────────────────────────────────────────────────────────────────────────────

/// One control's window class, style, 96-DPI-baseline geometry and initial
/// text. `layout()` and `create_controls()` are the only readers; both walk
/// [`CONTROLS`] in order, which is also creation order and therefore tab
/// order (every focusable entry carries `WS_TABSTOP`, and `WS_GROUP` marks
/// the first tab stop of each visual section).
#[derive(Debug, Clone, Copy)]
pub(super) struct ControlSpec {
    pub(super) id: u16,
    pub(super) class: &'static str,
    pub(super) style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    pub(super) text: &'static str,
}

/// Base window client size at 96 DPI (100% scaling); see [`scale_dimension`].
const BASE_WINDOW_WIDTH: i32 = 400;
const BASE_WINDOW_HEIGHT: i32 = 624;

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
        text: "General",
    },
    ControlSpec {
        id: ID_SEP_GENERAL,
        class: "STATIC",
        style: STYLE_SEP,
        x: 12,
        y: 32,
        w: 376,
        h: 2,
        text: "",
    },
    ControlSpec {
        id: ID_AUTOSTART,
        class: "BUTTON",
        style: STYLE_CHECKBOX_GROUP,
        x: 24,
        y: 42,
        w: 300,
        h: 20,
        text: "Start with Windows",
    },
    ControlSpec {
        id: ID_LABEL_STEP,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 72,
        w: 220,
        h: 20,
        text: "Brightness step per keypress",
    },
    ControlSpec {
        id: ID_STEP_EDIT,
        class: "EDIT",
        style: STYLE_EDIT_NUM,
        x: 250,
        y: 70,
        w: 60,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_STEP_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 310,
        y: 70,
        w: 16,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_STEP_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 326,
        y: 72,
        w: 20,
        h: 20,
        text: "%",
    },
    // ── Hotkeys ─────────────────────────────────────────────────────────
    ControlSpec {
        id: ID_HEADER_HOTKEYS,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 12,
        y: 108,
        w: 376,
        h: 16,
        text: "Hotkeys",
    },
    ControlSpec {
        id: ID_SEP_HOTKEYS,
        class: "STATIC",
        style: STYLE_SEP,
        x: 12,
        y: 128,
        w: 376,
        h: 2,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_HK_UP,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 140,
        w: 140,
        h: 20,
        text: "Brightness up",
    },
    ControlSpec {
        id: ID_HK_UP,
        class: "HOTKEY_CAPTURE",
        style: STYLE_CAPTURE_GROUP,
        x: 170,
        y: 138,
        w: 218,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_HK_DOWN,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 170,
        w: 140,
        h: 20,
        text: "Brightness down",
    },
    ControlSpec {
        id: ID_HK_DOWN,
        class: "HOTKEY_CAPTURE",
        style: STYLE_CAPTURE,
        x: 170,
        y: 168,
        w: 218,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_INTERCEPT,
        class: "BUTTON",
        style: STYLE_CHECKBOX,
        x: 24,
        y: 198,
        w: 340,
        h: 20,
        text: "Try to intercept dedicated brightness keys",
    },
    // Muted explainer text under the intercept checkbox.
    ControlSpec {
        id: ID_HK_HINT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 36,
        y: 222,
        w: 328,
        h: 34,
        text: "(may not work with all keyboards; some antivirus software flags low-level hooks)",
    },
    // Inline hotkey status line, empty until handle_hotkey_message_text sets
    // it; whether it renders as an error (red) or a notice (muted) is a
    // colour decided by the control-colour handler, not by this table.
    ControlSpec {
        id: ID_HK_ERROR,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 262,
        w: 340,
        h: 16,
        text: "",
    },
    // ── On-screen display ───────────────────────────────────────────────
    ControlSpec {
        id: ID_HEADER_OSD,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 12,
        y: 294,
        w: 376,
        h: 16,
        text: "On-screen display",
    },
    ControlSpec {
        id: ID_SEP_OSD,
        class: "STATIC",
        style: STYLE_SEP,
        x: 12,
        y: 314,
        w: 376,
        h: 2,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_TIMEOUT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 326,
        w: 140,
        h: 20,
        text: "Display duration",
    },
    ControlSpec {
        id: ID_OSD_TIMEOUT_EDIT,
        class: "EDIT",
        style: STYLE_EDIT_NUM_GROUP,
        x: 250,
        y: 324,
        w: 60,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_OSD_TIMEOUT_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 310,
        y: 324,
        w: 16,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_TIMEOUT_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 326,
        y: 326,
        w: 30,
        h: 20,
        text: "ms",
    },
    ControlSpec {
        id: ID_LABEL_OPACITY,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 24,
        y: 356,
        w: 140,
        h: 20,
        text: "Opacity",
    },
    ControlSpec {
        id: ID_OSD_OPACITY_EDIT,
        class: "EDIT",
        style: STYLE_EDIT_NUM,
        x: 250,
        y: 354,
        w: 60,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_OSD_OPACITY_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 310,
        y: 354,
        w: 16,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_OPACITY_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 326,
        y: 356,
        w: 20,
        h: 20,
        text: "%",
    },
    // ── Advanced ────────────────────────────────────────────────────────
    ControlSpec {
        id: ID_HEADER_ADVANCED,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 12,
        y: 392,
        w: 376,
        h: 16,
        text: "Advanced",
    },
    ControlSpec {
        id: ID_SEP_ADVANCED,
        class: "STATIC",
        style: STYLE_SEP,
        x: 12,
        y: 412,
        w: 376,
        h: 2,
        text: "",
    },
    ControlSpec {
        id: ID_RESYNC_CHECK,
        class: "BUTTON",
        style: STYLE_CHECKBOX_GROUP,
        x: 24,
        y: 422,
        w: 220,
        h: 20,
        text: "Resync brightness every",
    },
    ControlSpec {
        id: ID_RESYNC_EDIT,
        class: "EDIT",
        style: STYLE_EDIT_NUM,
        x: 250,
        y: 420,
        w: 60,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_RESYNC_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 310,
        y: 420,
        w: 16,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_RESYNC_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 326,
        y: 422,
        w: 20,
        h: 20,
        text: "s",
    },
    ControlSpec {
        id: ID_INACT_CHECK,
        class: "BUTTON",
        style: STYLE_CHECKBOX,
        x: 24,
        y: 452,
        w: 220,
        h: 20,
        text: "Resync after inactivity of",
    },
    ControlSpec {
        id: ID_INACT_EDIT,
        class: "EDIT",
        style: STYLE_EDIT_NUM,
        x: 250,
        y: 450,
        w: 60,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_INACT_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 310,
        y: 450,
        w: 16,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_INACT_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 326,
        y: 452,
        w: 20,
        h: 20,
        text: "s",
    },
    ControlSpec {
        id: ID_LOG_CHECK,
        class: "BUTTON",
        style: STYLE_CHECKBOX,
        x: 24,
        y: 482,
        w: 140,
        h: 20,
        text: "Write log file",
    },
    ControlSpec {
        id: ID_LABEL_LOG_LEVEL,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 170,
        y: 482,
        w: 74,
        h: 20,
        text: "Level:",
    },
    // The combo's `h` is the height of the *dropped-down* list, a Win32
    // quirk: the closed control renders at the font's line height regardless
    // of this value. See `no_two_controls_overlap`'s combo exemption below.
    ControlSpec {
        id: ID_LOG_LEVEL,
        class: "COMBOBOX",
        style: STYLE_COMBO,
        x: 250,
        y: 480,
        w: 100,
        h: 120,
        text: "",
    },
    ControlSpec {
        id: ID_LOG_HINT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 36,
        y: 506,
        w: 340,
        h: 32,
        text: "(logging changes take effect after restart; debug and below log monitor serials and paths)",
    },
    // ── Footer ──────────────────────────────────────────────────────────
    // One SysLink carries both links as flowing text, `iLink` (0 = config
    // file, 1 = log folder) telling window.rs's NM_CLICK handler which one
    // was clicked; the middot between them is part of that same flow, not a
    // separate control. SysLink markup: the visible text is exactly the
    // spec's wording; the `<a>` tags are SysLink's own syntax for "this span
    // is the hyperlink", not additional user-facing text.
    ControlSpec {
        id: ID_LINK_CONFIG,
        class: "SysLink",
        style: STYLE_LINK_GROUP,
        x: 12,
        y: 554,
        w: 250,
        h: 20,
        text: "<a>Open config file</a> \u{b7} <a>Open log folder</a>",
    },
    ControlSpec {
        id: ID_RESTORE,
        class: "BUTTON",
        style: STYLE_PUSHBUTTON_GROUP,
        x: 190,
        y: 586,
        w: 110,
        h: 26,
        text: "Restore defaults",
    },
    ControlSpec {
        id: ID_CLOSE,
        class: "BUTTON",
        style: STYLE_DEFPUSHBUTTON,
        x: 308,
        y: 586,
        w: 80,
        h: 26,
        text: "Close",
    },
];

/// Scales a 96-DPI-baseline pixel value to `dpi`, `MulDiv`-style
/// (`px * dpi / 96`, truncating). Plain integer arithmetic in `i64` to avoid
/// overflow at any DPI Windows actually reports, kept pure so it — and the
/// whole [`CONTROLS`] table through it — is unit-testable without a live
/// display.
#[must_use]
fn scale_dimension(px: i32, dpi: u32) -> i32 {
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
    u32::try_from(wparam & 0xFFFF).unwrap_or(96)
}

/// Positions every control in [`CONTROLS`] from its 96-DPI-baseline geometry
/// scaled to `dpi`. Creation calls this once after all controls exist; a
/// later DPI-change task reuses it for live relayout.
pub(super) fn layout(hwnd: HWND, dpi: u32) {
    for spec in CONTROLS {
        let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(spec.id)) }) else {
            continue;
        };
        let x = scale_dimension(spec.x, dpi);
        let y = scale_dimension(spec.y, dpi);
        let w = scale_dimension(spec.w, dpi);
        let h = scale_dimension(spec.h, dpi);
        unsafe {
            let _ = SetWindowPos(child, None, x, y, w, h, SWP_NOZORDER | SWP_NOACTIVATE);
        }
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

// ─────────────────────────────────────────────────────────────────────────────
// Placement
// ─────────────────────────────────────────────────────────────────────────────

/// The settings window's outer rect (position + size, in screen
/// coordinates) plus the DPI it was computed for. A named struct rather
/// than a same-typed tuple: `x`/`y`/`w`/`h`/`dpi` all being `i32`/`u32`
/// makes a positional tuple a transposition hazard for whatever calls this
/// next (the DPI-change task reuses it).
pub(super) struct Placement {
    pub(super) x: i32,
    pub(super) y: i32,
    pub(super) w: i32,
    pub(super) h: i32,
    pub(super) dpi: u32,
}

/// Computes [`Placement`]: centered on the *work area* (`rcWork`, which
/// excludes the taskbar — the same area shell dialogs center on) of the
/// monitor under the cursor, clamped so the window's top-left corner is
/// always inside that work area even when the work area is smaller than the
/// window itself. Geometry is computed before `CreateWindowExW` — new
/// sequencing versus `osd.rs`, which creates once with `CW_USEDEFAULT` and
/// positions per show; this window instead needs its final DPI known before
/// creation so `layout()` only ever runs once at the right scale.
pub(super) fn compute_placement() -> Result<Placement> {
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
    let dpi = dpi_x;

    let mut mi = MONITORINFO {
        cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).unwrap_or(0),
        ..Default::default()
    };
    if !unsafe { GetMonitorInfoW(hmonitor, &raw mut mi) }.as_bool() {
        log::warn!(error_code = super::super::get_last_error_code(); "GetMonitorInfoW failed; placement may be off-screen");
    }

    let client_w = scale_dimension(BASE_WINDOW_WIDTH, dpi);
    let client_h = scale_dimension(BASE_WINDOW_HEIGHT, dpi);

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client_w,
        bottom: client_h,
    };
    unsafe {
        AdjustWindowRectExForDpi(
            &raw mut rect,
            WS_CAPTION | WS_SYSMENU,
            false,
            WS_EX_TOPMOST,
            dpi,
        )
    }
    .map_err(|e| {
        BrightnessError::windows_api("AdjustWindowRectExForDpi", e.code().0.cast_unsigned())
    })?;

    let outer_w = rect.right - rect.left;
    let outer_h = rect.bottom - rect.top;

    let work = mi.rcWork;
    let x_centered = work.left + ((work.right - work.left) - outer_w) / 2;
    let y_centered = work.top + ((work.bottom - work.top) - outer_h) / 2;
    // Clamp to [work.left, work.right - outer_w] (and the same on the y
    // axis): the upper bound is floored at work.left via `.max` so a window
    // taller/wider than the work area still starts at the work area's
    // top-left corner instead of being pushed above/left of it — this
    // window is unusually tall, so at high DPI on a short work area (e.g.
    // 150% on 1920x1080) that upper bound is what actually binds.
    let x = x_centered.clamp(work.left, (work.right - outer_w).max(work.left));
    let y = y_centered.clamp(work.top, (work.bottom - outer_h).max(work.top));

    Ok(Placement {
        x,
        y,
        w: outer_w,
        h: outer_h,
        dpi,
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
        assert_eq!(dpi_from_wparam(96), 96);
        assert_eq!(dpi_from_wparam(144), 144);
    }

    /// Whether `class` is exempt from the overlap check: a combo box's `h`
    /// is the height of its *dropped-down* list (a documented Win32 quirk —
    /// see the `ID_LOG_LEVEL` comment in `CONTROLS`), not the closed
    /// control's footprint, so its declared rect legitimately extends over
    /// controls below it without a real visual collision.
    fn overlap_exempt(class: &str) -> bool {
        class == "COMBOBOX"
    }

    fn rects_overlap(a: &ControlSpec, b: &ControlSpec) -> bool {
        a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
    }

    /// `spec` scaled to `dpi` — same fields, just run through
    /// `scale_dimension` — so overlap checks can reuse [`rects_overlap`] on
    /// the geometry `layout()` would actually produce at that DPI, not only
    /// on the 96-DPI baseline table.
    fn scaled(spec: &ControlSpec, dpi: u32) -> ControlSpec {
        ControlSpec {
            x: scale_dimension(spec.x, dpi),
            y: scale_dimension(spec.y, dpi),
            w: scale_dimension(spec.w, dpi),
            h: scale_dimension(spec.h, dpi),
            ..*spec
        }
    }

    fn assert_no_overlap_at(dpi: u32) {
        let scaled: Vec<ControlSpec> = CONTROLS.iter().map(|c| scaled(c, dpi)).collect();
        for (i, a) in scaled.iter().enumerate() {
            for b in &scaled[i + 1..] {
                if overlap_exempt(a.class) || overlap_exempt(b.class) {
                    continue;
                }
                assert!(
                    !rects_overlap(a, b),
                    "controls {} and {} overlap at {dpi} dpi: {:?} vs {:?}",
                    a.id,
                    b.id,
                    a,
                    b
                );
            }
        }
    }

    #[test]
    fn no_two_controls_overlap() {
        assert_no_overlap_at(96);
    }

    #[test]
    fn no_two_controls_overlap_at_125_percent() {
        assert_no_overlap_at(120);
    }

    #[test]
    fn no_two_controls_overlap_at_150_percent() {
        assert_no_overlap_at(144);
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
        ] {
            assert!(
                ids.contains(&id),
                "brief-mandated id {id} missing from CONTROLS"
            );
        }
    }

    #[test]
    fn every_tabstop_control_fits_inside_the_client_rect() {
        // A bounds check, not a membership check — despite the name (kept
        // for continuity with the invariant it was written to guard), every
        // entry is checked, not just WS_TABSTOP ones: a decorative label
        // that overflows the window clips just as visibly as a control
        // someone could tab to.
        for spec in CONTROLS {
            assert!(
                spec.x >= 0 && spec.x + spec.w <= BASE_WINDOW_WIDTH,
                "{spec:?} exceeds window width"
            );
            assert!(
                spec.y >= 0 && spec.y + spec.h <= BASE_WINDOW_HEIGHT,
                "{spec:?} exceeds window height"
            );
        }
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
