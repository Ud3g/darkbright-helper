//! Windows settings dialog window.
//!
//! The window runs on its own thread with its own `GetMessageW` loop —
//! load-bearing, not stylistic. Dragging the title bar or opening an edit's
//! context menu enters an OS-internal modal loop that does not return until
//! the interaction ends; on the main thread that would stall controller
//! ticks and watchdogs long enough to trip the refresh watchdog. The thread
//! is spawned per open and exits when the window closes — there is no
//! long-lived idle thread and no supervision, matching how rarely this
//! window is used.
//!
//! Every group box the design mockup shows was replaced with a bold
//! `STATIC` label plus an `SS_ETCHEDHORZ` separator: a dark-mode spike
//! (see the module history) measured `BS_GROUPBOX`'s frame and caption as
//! unreadable in dark mode, while a plain `STATIC` takes the light text
//! colour correctly. That also removes any z-order constraint between
//! frames and their contents — creation order here is simply visual and
//! tab order.
//!
//! Control wiring (instant-apply, hotkey capture, dark-mode theming) is
//! later work; this module only creates the window, lays out its controls,
//! makes Tab/Enter/Esc behave, places it on the cursor's monitor, and
//! answers the [`SettingsSink`] seam.

use std::cell::RefCell;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLIP_DEFAULT_PRECIS, COLOR_BTNFACE, CreateFontIndirectW, DEFAULT_CHARSET, DEFAULT_QUALITY,
    DeleteObject, FF_SWISS, FONT_WEIGHT, FW_BOLD, FW_NORMAL, GetMonitorInfoW, GetSysColorBrush,
    HFONT, HMONITOR, LOGFONTW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    OUT_DEFAULT_PRECIS, VARIABLE_PITCH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    BST_CHECKED, BST_UNCHECKED, ICC_LINK_CLASS, ICC_STANDARD_CLASSES, ICC_UPDOWN_CLASS,
    INITCOMMONCONTROLSEX, InitCommonControlsEx, UDACCEL, UDM_SETACCEL, UDM_SETRANGE32,
    UPDOWN_CLASS, WC_BUTTON, WC_COMBOBOX, WC_EDIT, WC_LINK, WC_STATIC,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::SetFocus;
use windows::Win32::UI::WindowsAndMessaging::{
    BM_SETCHECK, CB_ADDSTRING, CB_SETCURSEL, CreateWindowExW, DC_HASDEFID, DM_GETDEFID,
    DefWindowProcW, DestroyWindow, DispatchMessageW, GetCursorPos, GetDlgItem, GetMessageW, HMENU,
    HWND_TOPMOST, IDC_ARROW, IsDialogMessageW, LoadCursorW, MSG, PostMessageW, PostQuitMessage,
    RegisterClassExW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW,
    SetForegroundWindow, SetWindowPos, SetWindowTextW, TranslateMessage, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_DESTROY, WM_SETFONT, WNDCLASSEXW, WS_BORDER,
    WS_CAPTION, WS_CHILD, WS_EX_TOPMOST, WS_GROUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::core::controller::SettingsSink;
use crate::core::state::{BrightnessMessage, SettingsSnapshot};
use crate::error::{BrightnessError, Result};

use super::{autostart, hwnd_from_isize, hwnd_to_isize, last_error_as_brightness_error};

// ─────────────────────────────────────────────────────────────────────────────
// Posted Messages
// ─────────────────────────────────────────────────────────────────────────────

/// Re-populate every control from a fresh `Box<SettingsSnapshot>` carried in
/// `lparam`. See [`SettingsSinkImpl::post_boxed`] for the ownership contract.
pub const WM_APP_SETTINGS_REFRESH: u32 = WM_APP + 1;
/// Bring the window to the foreground; no payload.
pub const WM_APP_SETTINGS_FOCUS: u32 = WM_APP + 2;
/// Show a hotkey registration error, a `Box<String>` carried in `lparam`.
/// See [`SettingsSinkImpl::post_boxed`] for the ownership contract.
pub const WM_APP_SETTINGS_HK_ERROR: u32 = WM_APP + 3;
/// Show a non-error hotkey notice, a `Box<String>` carried in `lparam`, same
/// ownership contract as [`WM_APP_SETTINGS_HK_ERROR`].
pub const WM_APP_SETTINGS_HK_NOTICE: u32 = WM_APP + 4;
/// Re-assert `HWND_TOPMOST`; no payload.
pub const WM_APP_SETTINGS_TOPMOST: u32 = WM_APP + 5;

// ─────────────────────────────────────────────────────────────────────────────
// Control IDs
// ─────────────────────────────────────────────────────────────────────────────
// 100s = General, 110s = Hotkeys, 120s = On-screen display, 130s = Advanced,
// 140s = footer. 200+ are decorative (section headers, separators, plain
// text labels) — never addressed individually outside this module, but each
// still needs a distinct id: `layout()` positions every control by
// `GetDlgItem(hwnd, id)`, which only ever finds the first match, so a shared
// id would silently move just one control instead of all of them.

const ID_AUTOSTART: u16 = 100;
const ID_STEP_EDIT: u16 = 101;
const ID_STEP_UPDOWN: u16 = 102;

const ID_HK_UP: u16 = 110;
const ID_HK_DOWN: u16 = 111;
const ID_INTERCEPT: u16 = 112;
const ID_HK_HINT: u16 = 113;
const ID_HK_ERROR: u16 = 114;

const ID_OSD_TIMEOUT_EDIT: u16 = 120;
const ID_OSD_TIMEOUT_UPDOWN: u16 = 121;
const ID_OSD_OPACITY_EDIT: u16 = 122;
const ID_OSD_OPACITY_UPDOWN: u16 = 123;

const ID_RESYNC_CHECK: u16 = 130;
const ID_RESYNC_EDIT: u16 = 131;
const ID_RESYNC_UPDOWN: u16 = 132;
const ID_INACT_CHECK: u16 = 133;
const ID_INACT_EDIT: u16 = 134;
const ID_INACT_UPDOWN: u16 = 135;
const ID_LOG_CHECK: u16 = 136;
const ID_LOG_LEVEL: u16 = 137;
const ID_LOG_HINT: u16 = 138;

const ID_LINK_CONFIG: u16 = 140;
const ID_LINK_LOGS: u16 = 141;
const ID_RESTORE: u16 = 142;
const ID_CLOSE: u16 = 143;

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
const ID_SEP_FOOTER_BULLET: u16 = 219;

/// `IDCANCEL`, the id `IsDialogMessageW` reports via `WM_COMMAND` on Esc.
/// Defined locally because the crate types the constant of that value as
/// `MESSAGEBOX_RESULT` (a message-box return code), not a dialog control id;
/// the numeric value is the same either way.
const IDCANCEL_ID: u16 = 2;

/// Whether `id` is one of the four bold section-header labels, which need
/// [`build_font`]'s bold variant rather than the regular one every other
/// control gets.
fn is_section_header(id: u16) -> bool {
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
const STYLE_EDIT_GROUP: u32 = STYLE_EDIT | WS_GROUP.0;
const STYLE_EDIT_NUM: u32 = STYLE_EDIT | ES_NUMBER;
const STYLE_EDIT_NUM_GROUP: u32 = STYLE_EDIT_NUM | WS_GROUP.0;
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
struct ControlSpec {
    id: u16,
    class: &'static str,
    style: u32,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    text: &'static str,
}

/// Log level dropdown entries, in the order `CB_ADDSTRING` inserts them —
/// index into this array is the combo selection index.
const LOG_LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

/// Base window client size at 96 DPI (100% scaling); see [`scale_dimension`].
const BASE_WINDOW_WIDTH: i32 = 400;
const BASE_WINDOW_HEIGHT: i32 = 624;

/// Every control in the settings window, at 96-DPI-baseline coordinates, in
/// visual and creation order. See the module doc comment for why group
/// boxes were replaced with a label + separator pair per section.
const CONTROLS: &[ControlSpec] = &[
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
        w: 50,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_STEP_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 300,
        y: 70,
        w: 16,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_STEP_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 320,
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
        class: "EDIT",
        style: STYLE_EDIT_GROUP,
        x: 170,
        y: 138,
        w: 180,
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
        class: "EDIT",
        style: STYLE_EDIT,
        x: 170,
        y: 168,
        w: 180,
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
        x: 170,
        y: 324,
        w: 60,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_OSD_TIMEOUT_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 230,
        y: 324,
        w: 16,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_TIMEOUT_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 250,
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
        x: 170,
        y: 354,
        w: 50,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_OSD_OPACITY_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 220,
        y: 354,
        w: 16,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_OPACITY_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 240,
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
        w: 50,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_RESYNC_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 300,
        y: 420,
        w: 16,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_RESYNC_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 320,
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
        w: 140,
        h: 20,
        text: "Resync after",
    },
    ControlSpec {
        id: ID_INACT_EDIT,
        class: "EDIT",
        style: STYLE_EDIT_NUM,
        x: 170,
        y: 450,
        w: 50,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_INACT_UPDOWN,
        class: "msctls_updown32",
        style: STYLE_UPDOWN,
        x: 220,
        y: 450,
        w: 16,
        h: 22,
        text: "",
    },
    ControlSpec {
        id: ID_LABEL_INACT_UNIT,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 240,
        y: 452,
        w: 120,
        h: 20,
        text: "s of inactivity",
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
        w: 40,
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
        x: 214,
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
        text: "(logging changes take effect after restart)",
    },
    // ── Footer ──────────────────────────────────────────────────────────
    // SysLink markup: the visible text is exactly the spec's wording; the
    // `<a>` tags are SysLink's own syntax for "this span is the hyperlink",
    // not additional user-facing text.
    ControlSpec {
        id: ID_LINK_CONFIG,
        class: "SysLink",
        style: STYLE_LINK_GROUP,
        x: 12,
        y: 554,
        w: 140,
        h: 20,
        text: "<a>Open config file</a>",
    },
    ControlSpec {
        id: ID_SEP_FOOTER_BULLET,
        class: "STATIC",
        style: STYLE_LABEL,
        x: 155,
        y: 554,
        w: 12,
        h: 20,
        text: "\u{b7}",
    },
    ControlSpec {
        id: ID_LINK_LOGS,
        class: "SysLink",
        style: STYLE_LINK,
        x: 170,
        y: 554,
        w: 140,
        h: 20,
        text: "<a>Open log folder</a>",
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
fn font_height_for_dpi(point_size: i32, dpi: u32) -> i32 {
    let scaled = i64::from(point_size) * i64::from(dpi) / 72;
    -i32::try_from(scaled).unwrap_or(point_size)
}

/// Positions every control in [`CONTROLS`] from its 96-DPI-baseline geometry
/// scaled to `dpi`. Creation calls this once after all controls exist; a
/// later DPI-change task reuses it for live relayout.
fn layout(hwnd: HWND, dpi: u32) {
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

// ─────────────────────────────────────────────────────────────────────────────
// Window Class Registration
// ─────────────────────────────────────────────────────────────────────────────

const SETTINGS_CLASS_NAME: PCWSTR = w!("DarkBrightSettings");

static REGISTER_CLASS_ONCE: OnceLock<Result<()>> = OnceLock::new();

/// Registers the settings window class exactly once, including the
/// `InitCommonControlsEx` call the spinner/link controls need.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if `GetModuleHandleW` or
/// `RegisterClassExW` fails.
fn ensure_settings_class_registered() -> Result<PCWSTR> {
    REGISTER_CLASS_ONCE
        .get_or_init(|| unsafe {
            let icce = INITCOMMONCONTROLSEX {
                dwSize: u32::try_from(std::mem::size_of::<INITCOMMONCONTROLSEX>()).unwrap_or(0),
                dwICC: ICC_UPDOWN_CLASS | ICC_LINK_CLASS | ICC_STANDARD_CLASSES,
            };
            let _ = InitCommonControlsEx(&raw const icce);

            let hinstance = GetModuleHandleW(None).map_err(|e| {
                BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
            })?;

            // Best-effort: an arrow cursor is a cosmetic nicety, not worth
            // failing class registration over.
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();

            let wnd_class = WNDCLASSEXW {
                cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(0),
                lpfnWndProc: Some(settings_wnd_proc),
                hInstance: hinstance.into(),
                hCursor: cursor,
                hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
                lpszClassName: SETTINGS_CLASS_NAME,
                ..Default::default()
            };

            if RegisterClassExW(&raw const wnd_class) == 0 {
                return Err(last_error_as_brightness_error("RegisterClassExW"));
            }
            Ok(())
        })
        .as_ref()
        .map_err(|e| BrightnessError::windows_api(format!("class registration failed: {e}"), 0))?;

    Ok(SETTINGS_CLASS_NAME)
}

// ─────────────────────────────────────────────────────────────────────────────
// Fonts
// ─────────────────────────────────────────────────────────────────────────────

/// Base UI font point size at 96 DPI; see [`font_height_for_dpi`].
const BASE_FONT_POINT_SIZE: i32 = 9;

/// Builds a Segoe UI font at `dpi` with the given `weight` (`FW_NORMAL` for
/// every control, `FW_BOLD` for the four section-header labels — a plain
/// `WM_SETFONT` with the regular font would render "bold" labels as
/// ordinary text, defeating the group-box-caption substitute the module doc
/// comment describes).
fn build_font(dpi: u32, weight: FONT_WEIGHT) -> HFONT {
    let mut face = [0u16; 32];
    let name: Vec<u16> = "Segoe UI".encode_utf16().collect();
    let len = name.len().min(face.len() - 1);
    face[..len].copy_from_slice(&name[..len]);

    let logfont = LOGFONTW {
        lfHeight: font_height_for_dpi(BASE_FONT_POINT_SIZE, dpi),
        lfWeight: i32::try_from(weight.0).unwrap_or(400),
        lfCharSet: DEFAULT_CHARSET,
        lfOutPrecision: OUT_DEFAULT_PRECIS,
        lfClipPrecision: CLIP_DEFAULT_PRECIS,
        lfQuality: DEFAULT_QUALITY,
        lfPitchAndFamily: VARIABLE_PITCH.0 | FF_SWISS.0,
        lfFaceName: face,
        ..Default::default()
    };

    let font = unsafe { CreateFontIndirectW(&raw const logfont) };
    if font.0.is_null() {
        log::warn!("CreateFontIndirectW failed; settings controls fall back to the system font");
    }
    font
}

// ─────────────────────────────────────────────────────────────────────────────
// Control Creation
// ─────────────────────────────────────────────────────────────────────────────

/// UTF-16, NUL-terminated encoding of `s` for a `PCWSTR` argument that only
/// needs to live for the duration of one FFI call.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Creates every control in [`CONTROLS`] as a child of `hwnd`, at a
/// placeholder position — `layout()` positions them all afterward — and
/// applies the matching font. Best-effort per control: a single failed
/// `CreateWindowExW` is logged and skipped rather than aborting the whole
/// window, matching how the rest of this crate degrades a UI by one element
/// rather than failing outright.
fn create_controls(hwnd: HWND, hinstance: HINSTANCE, font_regular: HFONT, font_bold: HFONT) {
    for spec in CONTROLS {
        let class = match spec.class {
            "STATIC" => WC_STATIC,
            "BUTTON" => WC_BUTTON,
            "EDIT" => WC_EDIT,
            "COMBOBOX" => WC_COMBOBOX,
            "msctls_updown32" => UPDOWN_CLASS,
            "SysLink" => WC_LINK,
            other => {
                log::error!(class = other; "Unknown settings control class in layout table");
                continue;
            }
        };

        let text = wide(spec.text);
        let id = HMENU(std::ptr::without_provenance_mut(usize::from(spec.id)));
        let style = WINDOW_STYLE(spec.style) | WS_CHILD | WS_VISIBLE;

        let child = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class,
                PCWSTR(text.as_ptr()),
                style,
                0,
                0,
                0,
                0,
                Some(hwnd),
                Some(id),
                Some(hinstance),
                None,
            )
        };

        let Ok(child) = child else {
            log::warn!(id = spec.id, class = spec.class; "Failed to create settings control");
            continue;
        };

        let font = if is_section_header(spec.id) {
            font_bold
        } else {
            font_regular
        };
        unsafe {
            SendMessageW(
                child,
                WM_SETFONT,
                Some(WPARAM(font.0.expose_provenance())),
                Some(LPARAM(1)),
            );
        }

        if spec.id == ID_LOG_LEVEL {
            for level in LOG_LEVELS {
                let level_wide = wide(level);
                unsafe {
                    SendMessageW(
                        child,
                        CB_ADDSTRING,
                        None,
                        Some(LPARAM(
                            level_wide.as_ptr().expose_provenance().cast_signed(),
                        )),
                    );
                }
            }
        }
    }
}

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

/// Applies every spinner's range (and, for the OSD timeout, its
/// acceleration table) per the brief's ranges: step 1-50, timeout
/// 100-10000, opacity 10-100, resync 1-3600, inactivity 1-600.
fn configure_updowns(hwnd: HWND) {
    set_updown_range(hwnd, ID_STEP_UPDOWN, 1, 50);
    set_updown_range(hwnd, ID_OSD_TIMEOUT_UPDOWN, 100, 10_000);
    set_updown_accel(
        hwnd,
        ID_OSD_TIMEOUT_UPDOWN,
        &[UDACCEL { nSec: 0, nInc: 100 }],
    );
    set_updown_range(hwnd, ID_OSD_OPACITY_UPDOWN, 10, 100);
    set_updown_range(hwnd, ID_RESYNC_UPDOWN, 1, 3600);
    set_updown_range(hwnd, ID_INACT_UPDOWN, 1, 600);
}

// ─────────────────────────────────────────────────────────────────────────────
// Populating Controls
// ─────────────────────────────────────────────────────────────────────────────

/// Sets a child control's window text. No-op if `id` was never created.
fn set_text(hwnd: HWND, id: u16, text: &str) {
    let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(id)) }) else {
        return;
    };
    let wide_text = wide(text);
    unsafe {
        let _ = SetWindowTextW(child, PCWSTR(wide_text.as_ptr()));
    }
}

/// Sets a checkbox's checked state via `BM_SETCHECK`.
fn set_checked(hwnd: HWND, id: u16, checked: bool) {
    let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(id)) }) else {
        return;
    };
    let state = if checked {
        BST_CHECKED.0
    } else {
        BST_UNCHECKED.0
    };
    unsafe {
        SendMessageW(
            child,
            BM_SETCHECK,
            Some(WPARAM(usize::try_from(state).unwrap_or(0))),
            None,
        );
    }
}

/// Index of `level` (case-insensitive) into [`LOG_LEVELS`], matching how
/// `core/config.rs` parses `file_level` via `log::LevelFilter`.
fn log_level_index(level: &str) -> Option<usize> {
    LOG_LEVELS
        .iter()
        .position(|candidate| candidate.eq_ignore_ascii_case(level))
}

/// Selects the combo entry matching `level`; leaves the current selection
/// untouched if `level` does not match one of [`LOG_LEVELS`].
fn set_combo_selection(hwnd: HWND, id: u16, level: &str) {
    let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(id)) }) else {
        return;
    };
    let Some(index) = log_level_index(level) else {
        log::debug!(level; "Unknown file log level; leaving combo selection unchanged");
        return;
    };
    unsafe {
        SendMessageW(child, CB_SETCURSEL, Some(WPARAM(index)), None);
    }
}

/// Re-populates every value control from `snap`, plus the autostart
/// checkbox from the live registry state — not part of `SettingsSnapshot`,
/// since autostart's source of truth is the registry, not `config.json`.
///
/// Notifications are suppressed for the whole population, not per field:
/// creation calls this with the window's first values, and
/// `WM_APP_SETTINGS_REFRESH` calls it again (restore-defaults, a hotkey
/// revert). A later task's `EN_CHANGE`/`BN_CLICKED` handlers are expected to
/// check `suppress_notifications` before posting a live `SettingChanged` so
/// programmatic population never looks like a user edit.
fn apply_snapshot(state: &mut WindowState, snap: &SettingsSnapshot) {
    state.suppress_notifications = true;

    set_text(state.hwnd, ID_STEP_EDIT, &snap.step_percent.to_string());
    set_text(
        state.hwnd,
        ID_OSD_TIMEOUT_EDIT,
        &snap.osd_timeout_ms.to_string(),
    );
    set_text(
        state.hwnd,
        ID_OSD_OPACITY_EDIT,
        &snap.osd_opacity_percent.to_string(),
    );

    set_checked(
        state.hwnd,
        ID_RESYNC_CHECK,
        snap.refresh_periodic_seconds != 0,
    );
    set_text(
        state.hwnd,
        ID_RESYNC_EDIT,
        &snap.refresh_periodic_seconds.to_string(),
    );
    set_checked(
        state.hwnd,
        ID_INACT_CHECK,
        snap.refresh_inactivity_seconds != 0,
    );
    set_text(
        state.hwnd,
        ID_INACT_EDIT,
        &snap.refresh_inactivity_seconds.to_string(),
    );

    set_text(state.hwnd, ID_HK_UP, &snap.hotkey_up);
    set_text(state.hwnd, ID_HK_DOWN, &snap.hotkey_down);
    set_checked(state.hwnd, ID_INTERCEPT, snap.intercept_brightness_keys);

    set_checked(state.hwnd, ID_LOG_CHECK, snap.file_log_enabled);
    set_combo_selection(state.hwnd, ID_LOG_LEVEL, &snap.file_log_level);

    set_checked(state.hwnd, ID_AUTOSTART, autostart::is_enabled());

    state.suppress_notifications = false;
}

// ─────────────────────────────────────────────────────────────────────────────
// Placement
// ─────────────────────────────────────────────────────────────────────────────

/// Computes the settings window's outer rect (position + size, in screen
/// coordinates) centered on the monitor under the cursor, plus that
/// monitor's DPI. Geometry is computed before `CreateWindowExW` — new
/// sequencing versus `osd.rs`, which creates once with `CW_USEDEFAULT` and
/// positions per show; this window instead needs its final DPI known before
/// creation so [`layout`] only ever runs once at the right scale.
fn compute_placement() -> Result<(i32, i32, i32, i32, u32)> {
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
        log::warn!(error_code = super::get_last_error_code(); "GetMonitorInfoW failed; placement may be off-screen");
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

    let mon = mi.rcMonitor;
    let x = mon.left + ((mon.right - mon.left) - outer_w) / 2;
    let y = mon.top + ((mon.bottom - mon.top) - outer_h) / 2;

    Ok((x, y, outer_w, outer_h, dpi))
}

// ─────────────────────────────────────────────────────────────────────────────
// Window State & Message Loop
// ─────────────────────────────────────────────────────────────────────────────

/// Per-window state, set once creation finishes and cleared on `WM_DESTROY`.
/// Lives in a thread-local because the window's own dedicated thread is the
/// only thread that ever touches it, and a plain `extern "system"` wndproc
/// has no other way to reach it — the same pattern `tray.rs`/`osd.rs` use
/// for their thread-local render/sender state.
struct WindowState {
    hwnd: HWND,
    sender: Sender<BrightnessMessage>,
    hwnd_slot: Arc<AtomicIsize>,
    font_regular: HFONT,
    font_bold: HFONT,
    /// True for the duration of `apply_snapshot`. A later task's live-edit
    /// handlers are expected to check this before posting `SettingChanged`;
    /// nothing outside `apply_snapshot` reads it yet, but the field and its
    /// toggling have to exist now so that wiring has something to check.
    #[allow(dead_code)]
    suppress_notifications: bool,
}

thread_local! {
    static WINDOW_STATE: RefCell<Option<WindowState>> = const { RefCell::new(None) };
}

/// Reclaims ownership of `lparam`'s `Box<SettingsSnapshot>` and applies it.
/// Other half of the contract documented on [`SettingsSinkImpl::post_boxed`].
fn handle_refresh_message(lparam: LPARAM) {
    let ptr: *mut SettingsSnapshot =
        std::ptr::with_exposed_provenance_mut(lparam.0.cast_unsigned());
    let snapshot = unsafe { Box::from_raw(ptr) };
    WINDOW_STATE.with(|s| {
        if let Some(state) = s.borrow_mut().as_mut() {
            apply_snapshot(state, &snapshot);
        }
    });
}

/// Reclaims ownership of `lparam`'s `Box<String>` and shows it on the
/// hotkey-error line. Same contract as [`handle_refresh_message`]. Error and
/// notice share the single `ID_HK_ERROR` line: the brief's control-id list
/// has no separate notice control, and coloring the two differently is
/// theming work for a later task.
fn handle_hotkey_message_text(lparam: LPARAM) {
    let ptr: *mut String = std::ptr::with_exposed_provenance_mut(lparam.0.cast_unsigned());
    let message = unsafe { Box::from_raw(ptr) };
    WINDOW_STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            set_text(state.hwnd, ID_HK_ERROR, &message);
        }
    });
}

/// Routes a `WM_COMMAND`: Close (button click or `IsDialogMessageW`'s
/// simulated default-button click on Enter) and Esc's `IDCANCEL` both close
/// the window; everything else is unwired in this task (instant-apply is
/// later work).
fn handle_command(hwnd: HWND, wparam: WPARAM) {
    let Ok(id) = u16::try_from(wparam.0 & 0xFFFF) else {
        return;
    };
    if id == ID_CLOSE || id == IDCANCEL_ID {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Clears the shared HWND slot, notifies the controller, and frees the
/// fonts this window owns. Runs on `WM_DESTROY`, so every exit path (Esc,
/// Enter-to-Close, the Close button, Alt+F4, the system-menu Close item)
/// converges on it via `WM_CLOSE` -> `DestroyWindow` -> `WM_DESTROY`.
fn handle_destroy() {
    WINDOW_STATE.with(|s| {
        if let Some(state) = s.borrow_mut().take() {
            state.hwnd_slot.store(0, Ordering::SeqCst);
            if let Err(e) = state.sender.send(BrightnessMessage::SettingsClosed) {
                log::warn!(error:% = e; "Failed to send SettingsClosed (controller channel closed?)");
            }
            unsafe {
                let _ = DeleteObject(state.font_regular.into());
                let _ = DeleteObject(state.font_bold.into());
            }
        }
    });
    unsafe {
        PostQuitMessage(0);
    }
}

/// Window procedure for the settings window.
///
/// # Safety
///
/// This is a Windows callback. The caller (Windows) ensures `hwnd` is valid.
unsafe extern "system" fn settings_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            DM_GETDEFID => {
                // Tells IsDialogMessageW's Enter handling that ID_CLOSE is
                // the default button: high word DC_HASDEFID, low word the id.
                let packed = (DC_HASDEFID << 16) | u32::from(ID_CLOSE);
                LRESULT(isize::try_from(packed.cast_signed()).unwrap_or(0))
            }
            WM_COMMAND => {
                handle_command(hwnd, wparam);
                LRESULT(0)
            }
            WM_APP_SETTINGS_REFRESH => {
                handle_refresh_message(lparam);
                LRESULT(0)
            }
            WM_APP_SETTINGS_FOCUS => {
                let _ = SetForegroundWindow(hwnd);
                LRESULT(0)
            }
            WM_APP_SETTINGS_HK_ERROR | WM_APP_SETTINGS_HK_NOTICE => {
                handle_hotkey_message_text(lparam);
                LRESULT(0)
            }
            WM_APP_SETTINGS_TOPMOST => {
                let _ = SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
                );
                LRESULT(0)
            }
            WM_CLOSE => {
                let _ = DestroyWindow(hwnd);
                LRESULT(0)
            }
            WM_DESTROY => {
                handle_destroy();
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Creates the window, its controls and fonts, positions everything, and
/// populates it from `snapshot`. Returns the window's `HWND` once it is
/// ready to pump messages.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if class registration, placement,
/// or `CreateWindowExW` fails. A failure here means no window and no thread
/// state was left behind — nothing to unwind.
fn create_settings_window(
    tx: &Sender<BrightnessMessage>,
    hwnd_slot: &Arc<AtomicIsize>,
    snapshot: &SettingsSnapshot,
) -> Result<HWND> {
    let class_name = ensure_settings_class_registered()?;
    let (x, y, outer_w, outer_h, dpi) = compute_placement()?;

    let hinstance = unsafe { GetModuleHandleW(None) }.map_err(|e| {
        BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
    })?;

    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST,
            class_name,
            w!("darkbright-helper Settings"),
            WS_CAPTION | WS_SYSMENU | WS_VISIBLE,
            x,
            y,
            outer_w,
            outer_h,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
    }
    .map_err(|e| BrightnessError::windows_api("CreateWindowExW", e.code().0.cast_unsigned()))?;

    let font_regular = build_font(dpi, FW_NORMAL);
    let font_bold = build_font(dpi, FW_BOLD);

    create_controls(hwnd, hinstance.into(), font_regular, font_bold);
    layout(hwnd, dpi);
    configure_updowns(hwnd);

    let mut state = WindowState {
        hwnd,
        sender: tx.clone(),
        hwnd_slot: Arc::clone(hwnd_slot),
        font_regular,
        font_bold,
        suppress_notifications: false,
    };
    apply_snapshot(&mut state, snapshot);
    WINDOW_STATE.with(|s| *s.borrow_mut() = Some(state));

    // WS_VISIBLE on the creation style already shows the window; only
    // activation and initial focus are left to do here.
    unsafe {
        let _ = SetForegroundWindow(hwnd);
        if let Ok(first) = GetDlgItem(Some(hwnd), i32::from(ID_AUTOSTART)) {
            let _ = SetFocus(Some(first));
        }
    }

    Ok(hwnd)
}

/// Runs the settings window on the calling thread: creates it, stores its
/// `HWND` into the shared slot, then pumps messages until the window closes.
/// Spawned fresh by [`SettingsSinkImpl::open`] per activation; returns (and
/// the thread ends) once the message loop exits.
fn run_settings_window(
    tx: &Sender<BrightnessMessage>,
    hwnd_slot: &Arc<AtomicIsize>,
    snapshot: &SettingsSnapshot,
) {
    let hwnd = match create_settings_window(tx, hwnd_slot, snapshot) {
        Ok(hwnd) => hwnd,
        Err(e) => {
            log::error!(error:% = e; "Failed to create settings window");
            return;
        }
    };

    hwnd_slot.store(hwnd_to_isize(hwnd), Ordering::SeqCst);
    log::debug!("Settings window created");

    let mut msg = MSG::default();
    while unsafe { GetMessageW(&raw mut msg, None, 0, 0) }.0 > 0 {
        if unsafe { IsDialogMessageW(hwnd, &raw const msg) }.as_bool() {
            continue;
        }
        unsafe {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }

    log::debug!("Settings window message loop ended");
}

// ─────────────────────────────────────────────────────────────────────────────
// Sink
// ─────────────────────────────────────────────────────────────────────────────

/// [`SettingsSink`] backed by a real Win32 window on its own thread.
///
/// `hwnd` is `0` when no window is open. It crosses the thread boundary as
/// an `AtomicIsize` rather than a raw `HWND` because `HWND` is a pointer
/// type and not `Send`/`Sync` — the same seam shape `TrayStatusHandle` uses
/// for the tray window's handle.
pub struct SettingsSinkImpl {
    tx: Sender<BrightnessMessage>,
    hwnd: Arc<AtomicIsize>,
}

impl SettingsSinkImpl {
    /// Creates a sink with no window open yet.
    #[must_use]
    pub fn new(tx: Sender<BrightnessMessage>) -> Self {
        Self {
            tx,
            hwnd: Arc::new(AtomicIsize::new(0)),
        }
    }

    /// The open window's `HWND`, if any.
    fn target_hwnd(&self) -> Option<HWND> {
        let raw = self.hwnd.load(Ordering::SeqCst);
        (raw != 0).then(|| hwnd_from_isize(raw))
    }

    /// Posts a payload-free message. Fails harmlessly (logged at debug) once
    /// the window is gone, matching `TrayStatusHandle::notify`.
    fn post_simple(&self, msg: u32) {
        let Some(hwnd) = self.target_hwnd() else {
            return;
        };
        unsafe {
            if let Err(e) = PostMessageW(Some(hwnd), msg, WPARAM(0), LPARAM(0)) {
                log::debug!(error:% = e; "Settings window post failed (window gone?)");
            }
        }
    }

    /// Posts a message whose `lparam` hands ownership of a heap value to the
    /// settings window thread.
    ///
    /// `Box::into_raw` here transfers ownership into the posted message; the
    /// wndproc's matching arm (`handle_refresh_message` /
    /// `handle_hotkey_message_text`) reclaims it with exactly one
    /// `Box::from_raw`. If the post itself fails, the box is reconstituted
    /// and dropped right here instead of leaking. The one gap neither side
    /// can close: a window destroyed after a *successful* post but before
    /// the message is pumped — Windows discards a destroyed window's queued
    /// posted messages without dispatching them, so that Box would never be
    /// reclaimed. Accepted as a narrow, rare leak (a few dozen bytes, once,
    /// only on that exact race) rather than a correctness risk; see the
    /// module report's Concerns for why redesigning around it (e.g.
    /// `SendMessageW`) was rejected.
    fn post_boxed<T>(&self, msg: u32, value: T) {
        let Some(hwnd) = self.target_hwnd() else {
            return;
        };
        let ptr = Box::into_raw(Box::new(value));
        let lparam = LPARAM(ptr.expose_provenance().cast_signed());
        unsafe {
            if let Err(e) = PostMessageW(Some(hwnd), msg, WPARAM(0), lparam) {
                log::debug!(error:% = e; "Settings window post failed (window gone?)");
                drop(Box::from_raw(ptr));
            }
        }
    }
}

impl SettingsSink for SettingsSinkImpl {
    fn open(&mut self, snapshot: &SettingsSnapshot) {
        if let Some(hwnd) = self.target_hwnd() {
            unsafe {
                if let Err(e) =
                    PostMessageW(Some(hwnd), WM_APP_SETTINGS_FOCUS, WPARAM(0), LPARAM(0))
                {
                    log::debug!(error:% = e; "Focus post failed (settings window gone?)");
                }
            }
            return;
        }

        let tx = self.tx.clone();
        let hwnd_slot = Arc::clone(&self.hwnd);
        let snapshot = snapshot.clone();
        std::thread::spawn(move || run_settings_window(&tx, &hwnd_slot, &snapshot));
    }

    fn refresh(&mut self, snapshot: &SettingsSnapshot) {
        self.post_boxed(WM_APP_SETTINGS_REFRESH, snapshot.clone());
    }

    fn hotkey_error(&mut self, message: &str) {
        self.post_boxed(WM_APP_SETTINGS_HK_ERROR, message.to_string());
    }

    fn hotkey_notice(&mut self, message: &str) {
        self.post_boxed(WM_APP_SETTINGS_HK_NOTICE, message.to_string());
    }

    fn assert_topmost(&mut self) {
        self.post_simple(WM_APP_SETTINGS_TOPMOST);
    }
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
    fn scaling_at_125_percent_is_exact_for_multiples_of_4() {
        // 120 DPI = 125% scaling; px * 120 / 96 = px * 1.25, exact whenever
        // px is a multiple of 4 (every x/y/w/h in CONTROLS is).
        assert_eq!(scale_dimension(400, 120), 500);
        assert_eq!(scale_dimension(24, 120), 30);
        assert_eq!(scale_dimension(16, 120), 20);
    }

    #[test]
    fn scaling_at_150_percent_is_exact_for_even_values() {
        // 144 DPI = 150% scaling; px * 144 / 96 = px * 1.5, exact whenever
        // px is even (every x/y/w/h in CONTROLS is).
        assert_eq!(scale_dimension(400, 144), 600);
        assert_eq!(scale_dimension(24, 144), 36);
        assert_eq!(scale_dimension(16, 144), 24);
    }

    #[test]
    fn every_control_x_y_w_h_scales_without_overflow_or_panic_at_150_percent() {
        for spec in CONTROLS {
            let _ = scale_dimension(spec.x, 144);
            let _ = scale_dimension(spec.y, 144);
            let _ = scale_dimension(spec.w, 144);
            let _ = scale_dimension(spec.h, 144);
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

    #[test]
    fn no_two_controls_overlap() {
        for (i, a) in CONTROLS.iter().enumerate() {
            for b in &CONTROLS[i + 1..] {
                if overlap_exempt(a.class) || overlap_exempt(b.class) {
                    continue;
                }
                assert!(
                    !rects_overlap(a, b),
                    "controls {} and {} overlap: {:?} vs {:?}",
                    a.id,
                    b.id,
                    a,
                    b
                );
            }
        }
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
            ID_LINK_LOGS,
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
    fn every_tabstop_control_belongs_to_the_window() {
        // Sanity check on the table itself: every WS_TABSTOP entry must sit
        // inside the window's base client rect, or Tab navigation would
        // reach an off-screen control.
        for spec in CONTROLS {
            if spec.style & WS_TABSTOP.0 != 0 {
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
    }

    #[test]
    fn log_level_index_matches_insertion_order_case_insensitively() {
        assert_eq!(log_level_index("error"), Some(0));
        assert_eq!(log_level_index("WARN"), Some(1));
        assert_eq!(log_level_index("Info"), Some(2));
        assert_eq!(log_level_index("debug"), Some(3));
        assert_eq!(log_level_index("trace"), Some(4));
        assert_eq!(log_level_index("bogus"), None);
    }

    #[test]
    fn section_headers_are_exactly_the_four_named_labels() {
        let headers: Vec<u16> = CONTROLS
            .iter()
            .filter(|c| is_section_header(c.id))
            .map(|c| c.id)
            .collect();
        assert_eq!(headers.len(), 4);
        for id in headers {
            assert!(matches!(
                id,
                ID_HEADER_GENERAL | ID_HEADER_HOTKEYS | ID_HEADER_OSD | ID_HEADER_ADVANCED
            ));
        }
    }
}
