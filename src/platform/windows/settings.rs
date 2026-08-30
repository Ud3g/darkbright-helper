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
//! Control wiring is instant-apply (see the "Control Wiring" section below)
//! including hotkey capture (see "Hotkey Capture Control"); dark-mode
//! theming is later work. This module creates the window, lays out its
//! controls, makes Tab/Enter/Esc behave, places it on the cursor's monitor,
//! and answers the [`SettingsSink`] seam.

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CLIP_DEFAULT_PRECIS, COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_WINDOW, COLOR_WINDOWTEXT,
    CreateFontIndirectW, DEFAULT_CHARSET, DEFAULT_QUALITY, DT_END_ELLIPSIS, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawFocusRect, DrawTextW, EndPaint, FF_SWISS,
    FONT_WEIGHT, FW_BOLD, FW_NORMAL, GetMonitorInfoW, GetSysColor, GetSysColorBrush, HDC, HFONT,
    HMONITOR, InvalidateRect, LOGFONTW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromPoint,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    VARIABLE_PITCH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    BST_CHECKED, BST_UNCHECKED, ICC_LINK_CLASS, ICC_STANDARD_CLASSES, ICC_UPDOWN_CLASS,
    INITCOMMONCONTROLSEX, InitCommonControlsEx, NM_CLICK, NMHDR, NMUPDOWN, UDACCEL, UDM_SETACCEL,
    UDM_SETRANGE32, UDN_DELTAPOS, UPDOWN_CLASS, WC_BUTTON, WC_COMBOBOX, WC_EDIT, WC_LINK,
    WC_STATIC,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT,
    MOD_WIN, SetFocus, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LSHIFT,
    VK_LWIN, VK_MENU, VK_RCONTROL, VK_RETURN, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BN_CLICKED, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE,
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DC_HASDEFID, DLGC_BUTTON, DLGC_WANTALLKEYS,
    DLGC_WANTMESSAGE, DM_GETDEFID, DefWindowProcW, DestroyWindow, DispatchMessageW, EN_KILLFOCUS,
    GWLP_USERDATA, GetClientRect, GetCursorPos, GetDlgCtrlID, GetDlgItem, GetMessageW,
    GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW, HMENU, HWND_TOPMOST, IDC_ARROW, IDOK,
    IsDialogMessageW, LoadCursorW, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_OKCANCEL, MSG,
    MessageBoxW, PM_REMOVE, PeekMessageW, PostMessageW, PostQuitMessage, RegisterClassExW, SW_SHOW,
    SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SendMessageW, SetForegroundWindow,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_APP, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_GETDLGCODE, WM_KEYDOWN,
    WM_KILLFOCUS, WM_LBUTTONDOWN, WM_NCDESTROY, WM_NOTIFY, WM_PAINT, WM_SETFOCUS, WM_SETFONT,
    WM_SETTEXT, WM_SYSKEYDOWN, WNDCLASSEXW, WS_BORDER, WS_CAPTION, WS_CHILD, WS_EX_TOPMOST,
    WS_GROUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::core::config::{DEFAULT_REFRESH_INACTIVITY_SECONDS, DEFAULT_REFRESH_PERIODIC_SECONDS};
use crate::core::controller::SettingsSink;
use crate::core::state::{BrightnessMessage, SettingChange, SettingsSnapshot};
use crate::error::{BrightnessError, Result};

use super::hotkey::{bindings_conflict, hotkey_string};
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
        class: "HOTKEY_CAPTURE",
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
        class: "HOTKEY_CAPTURE",
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
        text: "(logging changes take effect after restart; debug and below log monitor serials and paths)",
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

const CAPTURE_CLASS_NAME: PCWSTR = w!("DarkBrightHotkeyCapture");

static REGISTER_CAPTURE_CLASS_ONCE: OnceLock<Result<()>> = OnceLock::new();

/// Registers the hotkey capture control's window class exactly once.
/// `CS_HREDRAW | CS_VREDRAW` matches every other custom-draw window in this
/// crate ([`osd.rs`](super::osd)): the control's whole client area is text,
/// so any resize (a later DPI-change task) must repaint it fully rather than
/// keeping stale pixels in a corner GDI didn't think needed erasing. The
/// class background brush is `COLOR_WINDOW` (the same white/themed-window
/// colour a real `EDIT` control paints), so `WM_ERASEBKGND`'s default
/// handling alone gets the backdrop right; [`paint_capture`] only ever draws
/// text and the focus rectangle on top of it.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if `GetModuleHandleW` or
/// `RegisterClassExW` fails.
fn ensure_capture_class_registered() -> Result<PCWSTR> {
    REGISTER_CAPTURE_CLASS_ONCE
        .get_or_init(|| unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| {
                BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
            })?;

            // Best-effort, matching ensure_settings_class_registered: an
            // arrow cursor is cosmetic, not worth failing registration over.
            let cursor = LoadCursorW(None, IDC_ARROW).unwrap_or_default();

            let wnd_class = WNDCLASSEXW {
                cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(0),
                style: CS_HREDRAW | CS_VREDRAW,
                lpfnWndProc: Some(capture_wnd_proc),
                hInstance: hinstance.into(),
                hCursor: cursor,
                hbrBackground: GetSysColorBrush(COLOR_WINDOW),
                lpszClassName: CAPTURE_CLASS_NAME,
                ..Default::default()
            };

            if RegisterClassExW(&raw const wnd_class) == 0 {
                return Err(last_error_as_brightness_error("RegisterClassExW"));
            }
            Ok(())
        })
        .as_ref()
        .map_err(|e| {
            BrightnessError::windows_api(format!("capture class registration failed: {e}"), 0)
        })?;

    Ok(CAPTURE_CLASS_NAME)
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
    if font.is_invalid() {
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
            "HOTKEY_CAPTURE" => match ensure_capture_class_registered() {
                Ok(name) => name,
                Err(e) => {
                    log::error!(error:% = e; "Failed to register hotkey capture control class");
                    continue;
                }
            },
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

/// Applies every spinner's range straight from [`NUMERIC_FIELDS`] — the
/// single source of truth for those five ranges, which also mirror what
/// `core/config.rs`'s validator accepts — plus, for the OSD timeout, its
/// acceleration table; that one entry has no home in the table itself,
/// since none of the other four spinners need one.
fn configure_updowns(hwnd: HWND) {
    for field in NUMERIC_FIELDS {
        set_updown_range(
            hwnd,
            field.updown_id,
            i32::try_from(field.min).unwrap_or(0),
            i32::try_from(field.max).unwrap_or(i32::MAX),
        );
    }
    set_updown_accel(
        hwnd,
        ID_OSD_TIMEOUT_UPDOWN,
        &[UDACCEL { nSec: 0, nInc: 100 }],
    );
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
/// Notifications are suppressed for the whole population, not per field, via
/// [`SUPPRESS_NOTIFICATIONS`] (kept out of `WindowState`/its `RefCell` on
/// purpose — see that thread-local's doc comment): a programmatic
/// re-display — creation's first values, or a later `WM_APP_SETTINGS_REFRESH`
/// (restore-defaults, a hotkey revert) — must never be mistaken for a value
/// the user is actively committing.
fn apply_snapshot(state: &WindowState, snap: &SettingsSnapshot) {
    SUPPRESS_NOTIFICATIONS.with(|s| s.set(true));

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

    // The field itself never shows 0 — only the checkbox does — so a
    // disabled field displays the remembered/default value, grayed out,
    // rather than a literal "0" a user could mistake for a real setting.
    let periodic_remembered = remembered_seconds(
        snap.refresh_periodic_seconds,
        DEFAULT_REFRESH_PERIODIC_SECONDS,
    );
    let inactivity_remembered = remembered_seconds(
        snap.refresh_inactivity_seconds,
        DEFAULT_REFRESH_INACTIVITY_SECONDS,
    );
    let periodic_checked = snap.refresh_periodic_seconds != 0;
    let inactivity_checked = snap.refresh_inactivity_seconds != 0;

    set_checked(state.hwnd, ID_RESYNC_CHECK, periodic_checked);
    set_text(state.hwnd, ID_RESYNC_EDIT, &periodic_remembered.to_string());
    // Enabled state is display, exactly like the checkbox and the text next
    // to it — it must come from the snapshot on every population, not only
    // from a click. Left out originally, this let a disabled field survive
    // a refresh (restore-defaults, a hotkey revert) as checked-but-dead
    // (from a stale enabled state a click had set before the refresh) or
    // unchecked-but-live (never disabled in the first place).
    enable_control(state.hwnd, ID_RESYNC_EDIT, periodic_checked);
    enable_control(state.hwnd, ID_RESYNC_UPDOWN, periodic_checked);

    set_checked(state.hwnd, ID_INACT_CHECK, inactivity_checked);
    set_text(
        state.hwnd,
        ID_INACT_EDIT,
        &inactivity_remembered.to_string(),
    );
    enable_control(state.hwnd, ID_INACT_EDIT, inactivity_checked);
    enable_control(state.hwnd, ID_INACT_UPDOWN, inactivity_checked);

    set_text(state.hwnd, ID_HK_UP, &snap.hotkey_up);
    set_text(state.hwnd, ID_HK_DOWN, &snap.hotkey_down);
    set_checked(state.hwnd, ID_INTERCEPT, snap.intercept_brightness_keys);

    set_checked(state.hwnd, ID_LOG_CHECK, snap.file_log_enabled);
    set_combo_selection(state.hwnd, ID_LOG_LEVEL, &snap.file_log_level);
    enable_control(state.hwnd, ID_LOG_LEVEL, snap.file_log_enabled);

    set_checked(state.hwnd, ID_AUTOSTART, autostart::is_enabled());

    // Re-derive session memory from what was just displayed, so a later
    // uncheck-then-recheck in this same session restores *this* value, not
    // one left over from before the refresh.
    state.last_periodic.set(periodic_remembered);
    state.last_inactivity.set(inactivity_remembered);

    // Seed "what was last posted" from what was just displayed, so a user
    // who tabs through the dialog right after it opens or refreshes,
    // without editing anything, is correctly seen as having changed
    // nothing — the same invariant commit_numeric_field relies on for
    // every later commit.
    state
        .last_posted_step
        .set(Some(u32::from(snap.step_percent)));
    state.last_posted_timeout.set(Some(snap.osd_timeout_ms));
    state
        .last_posted_opacity
        .set(Some(u32::from(snap.osd_opacity_percent)));
    state.last_posted_periodic.set(Some(periodic_remembered));
    state
        .last_posted_inactivity
        .set(Some(inactivity_remembered));

    SUPPRESS_NOTIFICATIONS.with(|s| s.set(false));
}

// ─────────────────────────────────────────────────────────────────────────────
// Placement
// ─────────────────────────────────────────────────────────────────────────────

/// The settings window's outer rect (position + size, in screen
/// coordinates) plus the DPI it was computed for. A named struct rather
/// than a same-typed tuple: `x`/`y`/`w`/`h`/`dpi` all being `i32`/`u32`
/// makes a positional tuple a transposition hazard for whatever calls this
/// next (the DPI-change task reuses it).
struct Placement {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    dpi: u32,
}

/// Computes [`Placement`]: centered on the *work area* (`rcWork`, which
/// excludes the taskbar — the same area shell dialogs center on) of the
/// monitor under the cursor, clamped so the window's top-left corner is
/// always inside that work area even when the work area is smaller than the
/// window itself. Geometry is computed before `CreateWindowExW` — new
/// sequencing versus `osd.rs`, which creates once with `CW_USEDEFAULT` and
/// positions per show; this window instead needs its final DPI known before
/// creation so [`layout`] only ever runs once at the right scale.
fn compute_placement() -> Result<Placement> {
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

// ─────────────────────────────────────────────────────────────────────────────
// Window State & Message Loop
// ─────────────────────────────────────────────────────────────────────────────

/// Per-window state, set once creation finishes and cleared on `WM_DESTROY`.
/// Lives in a thread-local because the window's own dedicated thread is the
/// only thread that ever touches it, and a plain `extern "system"` wndproc
/// has no other way to reach it — the same pattern `tray.rs`/`osd.rs` use
/// for their thread-local render/sender state. Every field except the seven
/// `Cell`s is written once (at construction) and only ever read afterward,
/// which is what lets every reader below use `RefCell::borrow` (any number
/// of these can be held at once) instead of `borrow_mut` (which panics if a
/// second one is attempted while the first is still live); the `Cell`
/// fields are mutated through that same shared borrow via their own
/// interior mutability, so they don't need one either — see
/// [`with_window_state`] for the invariant that keeps this sound.
struct WindowState {
    hwnd: HWND,
    sender: Sender<BrightnessMessage>,
    hwnd_slot: Arc<AtomicIsize>,
    font_regular: HFONT,
    font_bold: HFONT,
    /// Last non-zero periodic-resync value the dialog has shown this
    /// session — what re-checking "Resync brightness every" restores.
    last_periodic: Cell<u32>,
    /// Same session memory for "Resync after ... of inactivity".
    last_inactivity: Cell<u32>,
    /// What `commit_numeric_field` last actually posted for each of the
    /// five numeric fields — `None` until the first real post this session.
    /// `apply_snapshot` seeds these from what it just displayed (so a
    /// tab-through right after opening isn't a "change" either), and
    /// `commit_numeric_field` compares its clamped value against the
    /// matching cell to skip a redundant `SettingChange`: tabbing through
    /// without editing, or the spurious `EN_KILLFOCUS` a spinner click
    /// fires when it steals focus from the edit right before its own
    /// `UDN_DELTAPOS`, would otherwise both post twice for one real change.
    last_posted_step: Cell<Option<u32>>,
    last_posted_timeout: Cell<Option<u32>>,
    last_posted_opacity: Cell<Option<u32>>,
    last_posted_periodic: Cell<Option<u32>>,
    last_posted_inactivity: Cell<Option<u32>>,
}

thread_local! {
    static WINDOW_STATE: RefCell<Option<WindowState>> = const { RefCell::new(None) };

    /// Set for the duration of `apply_snapshot`, so an edit/checkbox/combo
    /// handler can tell a value it is being asked to *display* apart from
    /// one a person just committed. Deliberately not a `WindowState` field:
    /// `SetWindowTextW` on an `EDIT` control synchronously sends
    /// `EN_CHANGE` back through `WM_COMMAND` before it returns, re-entering
    /// `settings_wnd_proc` while `apply_snapshot` is still on the stack. If
    /// that live-edit handling lived behind `WINDOW_STATE`'s `RefCell` it
    /// would need a `borrow_mut` at exactly the moment a re-entrant
    /// `borrow` from the same re-entrant call is already outstanding —
    /// panicking on every single population. A separate `Cell` sidesteps
    /// the question entirely: no borrow to conflict with.
    static SUPPRESS_NOTIFICATIONS: Cell<bool> = const { Cell::new(false) };
}

/// Runs `f` with the open window's state, if any. The single choke point
/// for reaching [`WINDOW_STATE`] from a control-wiring handler.
///
/// `RefCell::borrow` (used here) allows any number of live shared borrows
/// at once, so a Win32 call inside `f` that re-enters `settings_wnd_proc` —
/// `SetWindowTextW` on an edit is the one this module actually exercises,
/// via the `EN_CHANGE` it synchronously fires — is safe exactly because
/// that re-entry only ever takes another shared `borrow()` in turn
/// (`EN_CHANGE` is unwired, so the reentrant `WM_COMMAND` does nothing
/// further). What is *not* safe is a `borrow_mut()` taken while any borrow
/// — including one further up the very call stack a re-entrant path is
/// on — is still live: that panics immediately. [`handle_destroy`] and
/// [`create_settings_window`]'s initial population are the only two places
/// in this module that take one, and each is safe for its own reason (see
/// their doc comments) — a third `borrow_mut` anywhere in this module's
/// call graph would need the same scrutiny, or today's harmless nesting
/// turns into a live panic the moment it executes.
///
/// `MessageBoxW`/`DestroyWindow` are still always called with no borrow
/// held (see `confirm_restore_defaults`, `show_owned_error_message_box`) —
/// not because a shared borrow across them would panic today, but because
/// they pump the *entire* message queue rather than firing one synchronous
/// notification, which is a much larger reentrant surface to keep
/// reasoning about; callers should keep doing that for anything beyond a
/// single synchronous control notification, even though it isn't the
/// invariant that actually has to hold.
fn with_window_state(f: impl FnOnce(&WindowState)) {
    WINDOW_STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            f(state);
        }
    });
}

/// Reclaims ownership of `lparam`'s `Box<SettingsSnapshot>` and applies it.
/// Other half of the contract documented on [`SettingsSinkImpl::post_boxed`].
fn handle_refresh_message(lparam: LPARAM) {
    let ptr: *mut SettingsSnapshot =
        std::ptr::with_exposed_provenance_mut(lparam.0.cast_unsigned());
    let snapshot = unsafe { Box::from_raw(ptr) };
    with_window_state(|state| apply_snapshot(state, &snapshot));
}

/// Reclaims ownership of `lparam`'s `Box<String>` and shows it on the
/// hotkey status line. A registration error and a non-error notice share
/// the single `ID_HK_ERROR` control — it is one inline status line, not
/// two — so both go through this same function; which colour it renders in
/// (red for an error, muted for a notice) is decided by the control-colour
/// handler, not here. Same reclaim contract as [`handle_refresh_message`].
fn handle_hotkey_message_text(lparam: LPARAM) {
    let ptr: *mut String = std::ptr::with_exposed_provenance_mut(lparam.0.cast_unsigned());
    let message = unsafe { Box::from_raw(ptr) };
    with_window_state(|state| set_text(state.hwnd, ID_HK_ERROR, &message));
}

// ─────────────────────────────────────────────────────────────────────────────
// Control Wiring (Instant Apply)
// ─────────────────────────────────────────────────────────────────────────────
// Every control applies its change immediately and the controller persists
// it on a debounce timer — there is no OK/Apply button, only "Restore
// defaults" and "Close". The hotkey capture fields (`ID_HK_UP`/`ID_HK_DOWN`)
// are wired separately, in the "Hotkey Capture Control" section below (their
// own custom-drawn window class, not one of the controls this section
// applies to). All `WM_CTLCOLOR*` handling (graying the explainer statics,
// dark mode) is later work too — the two explainer statics below are
// created with their text and left at the system's default colour.

/// Valid range for the brightness-step edit, mirroring the range
/// [`configure_updowns`] gave `ID_STEP_UPDOWN` and the validator in
/// `core/config.rs`.
const STEP_RANGE: (u32, u32) = (1, 50);
/// Valid range for the OSD auto-hide timeout, in milliseconds.
const OSD_TIMEOUT_RANGE: (u32, u32) = (100, 10_000);
/// Valid range for the OSD opacity percentage.
const OSD_OPACITY_RANGE: (u32, u32) = (10, 100);
/// Valid range for the periodic-resync edit while its checkbox is checked
/// (0, meaning disabled, only ever arrives through the checkbox itself —
/// see [`NumericField::checkbox_id`]).
const RESYNC_RANGE: (u32, u32) = (1, 3600);
/// Valid range for the inactivity-resync edit while its checkbox is
/// checked; same 0-is-checkbox-only rule as [`RESYNC_RANGE`].
const INACTIVITY_RANGE: (u32, u32) = (1, 600);

/// One spinner+edit control pair wired to instant apply: its ids, valid
/// range, the checkbox that must be checked before it applies (`None` for
/// the three unconditional fields), how a clamped value becomes the change
/// to post, and — for the two checkbox-gated fields — where to remember it
/// for the session. Both the `EN_KILLFOCUS` and `UDN_DELTAPOS` commit paths
/// dispatch through this one table instead of five hand-written near-copies
/// of the same five steps.
struct NumericField {
    edit_id: u16,
    updown_id: u16,
    min: u32,
    max: u32,
    checkbox_id: Option<u16>,
    to_change: fn(u32) -> SettingChange,
    remember: Option<fn(&WindowState, u32)>,
    /// Selects this field's slot in `WindowState` for the "what did we last
    /// actually post" comparison `commit_numeric_field` makes before
    /// posting again — see that `Cell` group's doc comment on `WindowState`.
    last_posted: fn(&WindowState) -> &Cell<Option<u32>>,
}

const NUMERIC_FIELDS: &[NumericField] = &[
    NumericField {
        edit_id: ID_STEP_EDIT,
        updown_id: ID_STEP_UPDOWN,
        min: STEP_RANGE.0,
        max: STEP_RANGE.1,
        checkbox_id: None,
        to_change: |v| SettingChange::StepPercent(to_u8(v)),
        remember: None,
        last_posted: |state| &state.last_posted_step,
    },
    NumericField {
        edit_id: ID_OSD_TIMEOUT_EDIT,
        updown_id: ID_OSD_TIMEOUT_UPDOWN,
        min: OSD_TIMEOUT_RANGE.0,
        max: OSD_TIMEOUT_RANGE.1,
        checkbox_id: None,
        to_change: SettingChange::OsdTimeoutMs,
        remember: None,
        last_posted: |state| &state.last_posted_timeout,
    },
    NumericField {
        edit_id: ID_OSD_OPACITY_EDIT,
        updown_id: ID_OSD_OPACITY_UPDOWN,
        min: OSD_OPACITY_RANGE.0,
        max: OSD_OPACITY_RANGE.1,
        checkbox_id: None,
        to_change: |v| SettingChange::OsdOpacityPercent(to_u8(v)),
        remember: None,
        last_posted: |state| &state.last_posted_opacity,
    },
    NumericField {
        edit_id: ID_RESYNC_EDIT,
        updown_id: ID_RESYNC_UPDOWN,
        min: RESYNC_RANGE.0,
        max: RESYNC_RANGE.1,
        checkbox_id: Some(ID_RESYNC_CHECK),
        to_change: SettingChange::RefreshPeriodicSeconds,
        remember: Some(|state, v| state.last_periodic.set(v)),
        last_posted: |state| &state.last_posted_periodic,
    },
    NumericField {
        edit_id: ID_INACT_EDIT,
        updown_id: ID_INACT_UPDOWN,
        min: INACTIVITY_RANGE.0,
        max: INACTIVITY_RANGE.1,
        checkbox_id: Some(ID_INACT_CHECK),
        to_change: SettingChange::RefreshInactivitySeconds,
        remember: Some(|state, v| state.last_inactivity.set(v)),
        last_posted: |state| &state.last_posted_inactivity,
    },
];

/// Narrows a clamped value to `u8`, matching the payload
/// `StepPercent`/`OsdOpacityPercent` carry. Every [`NumericField`] range
/// that feeds this stays within `u8`, so the fallback never actually
/// triggers; it exists so a future range change fails safe instead of
/// silently wrapping.
#[must_use]
fn to_u8(value: u32) -> u8 {
    u8::try_from(value).unwrap_or(u8::MAX)
}

/// Parses `text` as a decimal integer and clamps it into `[min, max]`.
/// Empty or non-numeric text clamps to `min`; text that parses but
/// overflows `u32` (while still fitting `u64`) clamps to `max` instead of
/// being lumped in with genuinely malformed input — the `u64` intermediate
/// is what tells those two cases apart. The edit controls are
/// `ES_NUMBER`-restricted to digits, so in practice only the empty-field
/// case is reachable; the rest is defensive.
#[must_use]
fn parse_clamped(text: &str, min: u32, max: u32) -> u32 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return min;
    }
    match trimmed.parse::<u64>() {
        Ok(value) => u32::try_from(value.clamp(u64::from(min), u64::from(max))).unwrap_or(max),
        Err(_) => min,
    }
}

/// `pos + delta`, clamped into `[min, max]`. `i64` arithmetic sidesteps
/// `u32` underflow when `delta` is negative (the down arrow) and `pos` is
/// already at `min`.
#[must_use]
fn clamp_after_delta(pos: u32, delta: i32, min: u32, max: u32) -> u32 {
    let next = i64::from(pos) + i64::from(delta);
    u32::try_from(next.clamp(i64::from(min), i64::from(max))).unwrap_or(min)
}

/// What session memory should hold for a checkbox-gated 0-or-value field:
/// `snapshot_value` unchanged if it is nonzero, otherwise `default`. This is
/// the "re-checking after the dialog opened at 0 falls back to the default"
/// rule, applied both at population time and after every later refresh.
#[must_use]
fn remembered_seconds(snapshot_value: u32, default: u32) -> u32 {
    if snapshot_value == 0 {
        default
    } else {
        snapshot_value
    }
}

/// Whether a numeric commit should actually post: yes unless `value` is
/// exactly what was last posted for this field. `None` (nothing posted this
/// session yet, right after population) always counts as a change — there
/// is no prior post to compare against — so this only ever suppresses a
/// genuine repeat: tabbing through a field without editing it, or the
/// `EN_KILLFOCUS` a spinner click fires on its way to its own
/// `UDN_DELTAPOS` (see `WindowState::last_posted_step` and friends).
#[must_use]
fn should_post_numeric(last_posted: Option<u32>, value: u32) -> bool {
    last_posted != Some(value)
}

/// Reads `id`'s current window text. Empty if the control has no text or
/// doesn't exist (best-effort, matching the rest of this module).
fn get_text(hwnd: HWND, id: u16) -> String {
    let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(id)) }) else {
        return String::new();
    };
    window_text(child)
}

/// Reads `hwnd`'s own window text directly — the other half of [`get_text`],
/// split out because the hotkey capture control (see "Hotkey Capture
/// Control" below) reads its *own* text from inside its wndproc, where there
/// is no parent/id pair to resolve through `GetDlgItem`.
fn window_text(hwnd: HWND) -> String {
    let Ok(len) = usize::try_from(unsafe { GetWindowTextLengthW(hwnd) }) else {
        return String::new();
    };
    if len == 0 {
        return String::new();
    }
    let mut buf = vec![0u16; len + 1];
    let copied = usize::try_from(unsafe { GetWindowTextW(hwnd, &mut buf) }).unwrap_or(0);
    String::from_utf16_lossy(&buf[..copied])
}

/// Reads a checkbox's checked state via `BM_GETCHECK`. `false` if `id`
/// doesn't exist.
fn is_checked(hwnd: HWND, id: u16) -> bool {
    let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(id)) }) else {
        return false;
    };
    let result = unsafe { SendMessageW(child, BM_GETCHECK, None, None) };
    u32::try_from(result.0).unwrap_or(0) == BST_CHECKED.0
}

/// Enables or disables a child control via `EnableWindow`. No-op if `id`
/// doesn't exist.
fn enable_control(hwnd: HWND, id: u16, enabled: bool) {
    let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(id)) }) else {
        return;
    };
    unsafe {
        let _ = EnableWindow(child, enabled);
    }
}

/// The combo's currently selected log level, if any (`CB_ERR`, when nothing
/// is selected, fails the `usize` conversion and reads as `None`).
fn combo_selected_level(hwnd: HWND, id: u16) -> Option<&'static str> {
    let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(id)) }) else {
        return None;
    };
    let index = unsafe { SendMessageW(child, CB_GETCURSEL, None, None) }.0;
    usize::try_from(index)
        .ok()
        .and_then(|i| LOG_LEVELS.get(i))
        .copied()
}

/// Reads `edit_id`'s current text, clamps it into `[min, max]`, writes the
/// normalized text back — so typing `999` becomes `50` on focus loss, not
/// just in the posted value — and returns the clamped value.
fn commit_edit_text(hwnd: HWND, edit_id: u16, min: u32, max: u32) -> u32 {
    let value = parse_clamped(&get_text(hwnd, edit_id), min, max);
    set_text(hwnd, edit_id, &value.to_string());
    value
}

/// Applies a spinner click's `delta` to whatever `edit_id` currently shows,
/// clamps, writes the result back, and returns it. The spinner and edit
/// have no buddy relationship (this window always writes control text
/// itself, including on restore-defaults), so the edit's own displayed
/// text — not the spinner's internal position — is the only value the two
/// controls agree on.
fn commit_spinner_delta(hwnd: HWND, edit_id: u16, min: u32, max: u32, delta: i32) -> u32 {
    let current = parse_clamped(&get_text(hwnd, edit_id), min, max);
    let value = clamp_after_delta(current, delta, min, max);
    set_text(hwnd, edit_id, &value.to_string());
    value
}

/// Whether `field` currently applies: unconditional fields always do;
/// checkbox-gated fields only while their checkbox is checked (matching
/// the disabled edit+updown a user could not have committed through).
fn field_enabled(hwnd: HWND, field: &NumericField) -> bool {
    field.checkbox_id.is_none_or(|cb| is_checked(hwnd, cb))
}

/// Records `value` in session memory (if `field` has any) and posts the
/// change it maps to — unless `value` is exactly what this field last
/// posted, in which case this is a no-op: the brief's "post if changed"
/// rule for `EN_KILLFOCUS`, and what keeps a spinner click's `EN_KILLFOCUS`
/// (fired when the click steals focus away from the edit) from applying
/// alongside its own `UDN_DELTAPOS` for the same click.
fn commit_numeric_field(field: &NumericField, value: u32) {
    with_window_state(|state| {
        let last_posted = (field.last_posted)(state);
        if !should_post_numeric(last_posted.get(), value) {
            return;
        }
        last_posted.set(Some(value));
        if let Some(remember) = field.remember {
            remember(state, value);
        }
        post_change(state, (field.to_change)(value));
    });
}

/// `EN_KILLFOCUS` on one of the five numeric edits: commit its text and
/// post the change, unless its checkbox (if any) is unchecked — the field
/// is disabled then, so nothing should have been editable to commit.
fn handle_numeric_commit(hwnd: HWND, edit_id: u16) {
    let Some(field) = NUMERIC_FIELDS.iter().find(|f| f.edit_id == edit_id) else {
        return;
    };
    if !field_enabled(hwnd, field) {
        return;
    }
    let value = commit_edit_text(hwnd, field.edit_id, field.min, field.max);
    commit_numeric_field(field, value);
}

/// `UDN_DELTAPOS` on one of the five spinners: same commit as
/// [`handle_numeric_commit`], from a click instead of a focus change.
fn handle_spinner_delta(hwnd: HWND, updown_id: u16, delta: i32) {
    let Some(field) = NUMERIC_FIELDS.iter().find(|f| f.updown_id == updown_id) else {
        return;
    };
    if !field_enabled(hwnd, field) {
        return;
    }
    let value = commit_spinner_delta(hwnd, field.edit_id, field.min, field.max, delta);
    commit_numeric_field(field, value);
}

/// Posts `change` to the controller, no-op while [`SUPPRESS_NOTIFICATIONS`]
/// is set — the one choke point every control-change handler posts
/// through, so a programmatic re-display never sends a message the user
/// never asked for.
fn post_change(state: &WindowState, change: SettingChange) {
    if SUPPRESS_NOTIFICATIONS.with(Cell::get) {
        return;
    }
    send_message(state, BrightnessMessage::SettingChanged(change));
}

/// Sends a plain, non-`SettingChange` message to the controller — the
/// footer links, which only ever fire from a real click, never from
/// programmatic repopulation, so they bypass [`post_change`]'s suppression
/// gate entirely rather than needing it.
fn send_message(state: &WindowState, message: BrightnessMessage) {
    if let Err(e) = state.sender.send(message) {
        log::warn!(error:% = e; "Failed to send settings message (controller channel closed?)");
    }
}

/// `ID_INTERCEPT` (`BN_CLICKED`): post the low-level-hook toggle.
fn handle_intercept_click(hwnd: HWND) {
    let checked = is_checked(hwnd, ID_INTERCEPT);
    with_window_state(|state| post_change(state, SettingChange::InterceptBrightnessKeys(checked)));
}

/// `ID_LOG_CHECK` (`BN_CLICKED`): enable/disable the level combo to match,
/// and post the toggle.
fn handle_log_check_click(hwnd: HWND) {
    let checked = is_checked(hwnd, ID_LOG_CHECK);
    enable_control(hwnd, ID_LOG_LEVEL, checked);
    with_window_state(|state| post_change(state, SettingChange::FileLogEnabled(checked)));
}

/// `ID_LOG_LEVEL` (`CBN_SELCHANGE`): post the newly selected level string.
fn handle_log_level_selection(hwnd: HWND) {
    let Some(level) = combo_selected_level(hwnd, ID_LOG_LEVEL) else {
        return;
    };
    with_window_state(|state| post_change(state, SettingChange::FileLogLevel(level.to_string())));
}

/// `ID_RESYNC_CHECK` (`BN_CLICKED`): enable/disable its edit+updown; when
/// re-checked, restore the remembered value into the field before posting
/// it, so the display and the posted value never disagree.
fn handle_resync_checkbox_click(hwnd: HWND) {
    let checked = is_checked(hwnd, ID_RESYNC_CHECK);
    enable_control(hwnd, ID_RESYNC_EDIT, checked);
    enable_control(hwnd, ID_RESYNC_UPDOWN, checked);
    let value = if checked {
        let mut remembered = 0;
        with_window_state(|state| remembered = state.last_periodic.get());
        set_text(hwnd, ID_RESYNC_EDIT, &remembered.to_string());
        remembered
    } else {
        0
    };
    with_window_state(|state| post_change(state, SettingChange::RefreshPeriodicSeconds(value)));
}

/// `ID_INACT_CHECK` (`BN_CLICKED`): same behaviour as
/// [`handle_resync_checkbox_click`] for the inactivity field.
fn handle_inactivity_checkbox_click(hwnd: HWND) {
    let checked = is_checked(hwnd, ID_INACT_CHECK);
    enable_control(hwnd, ID_INACT_EDIT, checked);
    enable_control(hwnd, ID_INACT_UPDOWN, checked);
    let value = if checked {
        let mut remembered = 0;
        with_window_state(|state| remembered = state.last_inactivity.get());
        set_text(hwnd, ID_INACT_EDIT, &remembered.to_string());
        remembered
    } else {
        0
    };
    with_window_state(|state| post_change(state, SettingChange::RefreshInactivitySeconds(value)));
}

/// `ID_AUTOSTART` (`BN_CLICKED`): toggle Windows autostart directly against
/// the registry — this setting never goes through the settings channel,
/// since its source of truth is the registry, not `config.json` (see the
/// `autostart` module docs). On failure, revert the checkbox to what the
/// registry actually holds and tell the user why, so it never lies about
/// what was written.
fn handle_autostart_click(hwnd: HWND) {
    let checked = is_checked(hwnd, ID_AUTOSTART);
    let result = if checked {
        autostart::enable()
    } else {
        autostart::disable()
    };
    if let Err(e) = result {
        log::warn!(error:% = e; "Failed to update the Windows startup entry");
        // What the registry actually holds, not `!checked`: enable() can
        // fail after partially succeeding (the Run value write can succeed
        // while clearing the StartupApproved veto fails), and `!checked`
        // would then show unchecked despite Run actually being set.
        set_checked(hwnd, ID_AUTOSTART, autostart::is_enabled());
        show_owned_error_message_box(
            hwnd,
            "Brightness Control - Autostart",
            &format!("Couldn't update the Windows startup entry:\n{e}"),
        );
    }
}

/// `ID_RESTORE` (`BN_CLICKED`): confirm before applying `RestoreDefaults` —
/// under instant apply this resets ten settings, both hotkeys included, in
/// one click with no undo.
fn handle_restore_click(hwnd: HWND) {
    if !confirm_restore_defaults(hwnd) {
        return;
    }
    with_window_state(|state| post_change(state, SettingChange::RestoreDefaults));
}

/// Blocking OK/Cancel confirmation with a warning icon, owned by `hwnd` so
/// it stays in front of this topmost window; `true` iff the user chose OK.
/// A bare `MessageBoxW` call rather than `platform/windows/mod.rs`'s
/// helpers, which are OK-only and have no result to report.
fn confirm_restore_defaults(hwnd: HWND) -> bool {
    let message = wide("Reset all settings to their defaults? Hotkeys are applied immediately.");
    let title = wide("Brightness Control - Restore Defaults");
    let result = unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OKCANCEL | MB_ICONWARNING,
        )
    };
    result == IDOK
}

/// Blocking OK message box with an error icon, owned by `hwnd` so it stays
/// in front of this `WS_EX_TOPMOST` window and disables it for the
/// duration — a plain `MessageBoxW` call rather than
/// `platform/windows/mod.rs`'s `show_error_message_box`, which is
/// owner-less and would otherwise render in the normal window band
/// (potentially behind this window) without disabling it, leaving room to
/// click the same control again and stack a second modal box on top.
fn show_owned_error_message_box(hwnd: HWND, title: &str, message: &str) {
    let message_wide = wide(message);
    let title_wide = wide(title);
    unsafe {
        MessageBoxW(
            Some(hwnd),
            PCWSTR(message_wide.as_ptr()),
            PCWSTR(title_wide.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// Routes a `WM_COMMAND`: a genuine click (`BN_CLICKED`) on Close, or
/// `IsDialogMessageW`'s simulated default-button click on Enter, or Esc's
/// `IDCANCEL`, close the window. `BN_CLICKED` on any other control,
/// `EN_KILLFOCUS` on a numeric edit, and `CBN_SELCHANGE` on the log-level
/// combo dispatch to their handler above; anything else (`EN_CHANGE`
/// included — deliberately never wired, so retyping a value can't
/// transiently apply a half-typed one) is ignored.
fn handle_command(hwnd: HWND, wparam: WPARAM) {
    let Ok(id) = u16::try_from(wparam.0 & 0xFFFF) else {
        return;
    };
    let notify_code = u32::try_from((wparam.0 >> 16) & 0xFFFF).unwrap_or(u32::MAX);
    let is_close_click = id == ID_CLOSE && notify_code == BN_CLICKED;
    let is_cancel = id == IDCANCEL_ID;
    if is_close_click || is_cancel {
        unsafe {
            let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
        return;
    }

    match notify_code {
        BN_CLICKED => match id {
            ID_AUTOSTART => handle_autostart_click(hwnd),
            ID_INTERCEPT => handle_intercept_click(hwnd),
            ID_LOG_CHECK => handle_log_check_click(hwnd),
            ID_RESYNC_CHECK => handle_resync_checkbox_click(hwnd),
            ID_INACT_CHECK => handle_inactivity_checkbox_click(hwnd),
            ID_RESTORE => handle_restore_click(hwnd),
            _ => {}
        },
        EN_KILLFOCUS => handle_numeric_commit(hwnd, id),
        CBN_SELCHANGE if id == ID_LOG_LEVEL => handle_log_level_selection(hwnd),
        _ => {}
    }
}

/// Routes a `WM_NOTIFY`: a spinner's `UDN_DELTAPOS` commits its delta, and
/// `NM_CLICK` on a footer `SysLink` posts the matching shell side effect.
/// Every other notification code (including the `SysLink`'s own
/// `NM_RETURN`, unreachable from a mouse click) is ignored.
fn handle_notify(hwnd: HWND, lparam: LPARAM) {
    let hdr_ptr: *const NMHDR = std::ptr::with_exposed_provenance(lparam.0.cast_unsigned());
    let Some(hdr) = (unsafe { hdr_ptr.as_ref() }) else {
        return;
    };
    let id = u16::try_from(hdr.idFrom).unwrap_or(u16::MAX);

    match hdr.code {
        UDN_DELTAPOS => {
            let nmud_ptr: *const NMUPDOWN = hdr_ptr.cast();
            if let Some(nmud) = unsafe { nmud_ptr.as_ref() } {
                handle_spinner_delta(hwnd, id, nmud.iDelta);
            }
        }
        NM_CLICK => {
            let message = match id {
                ID_LINK_CONFIG => Some(BrightnessMessage::OpenConfigFile),
                ID_LINK_LOGS => Some(BrightnessMessage::TrayOpenLogFolder),
                _ => None,
            };
            if let Some(message) = message {
                with_window_state(|state| send_message(state, message));
            }
        }
        _ => {}
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Hotkey Capture Control
// ─────────────────────────────────────────────────────────────────────────────
// `ID_HK_UP`/`ID_HK_DOWN` are windows of the `"DarkBrightHotkeyCapture"`
// class registered above, each with its own copy of the state below (kept
// in the window's `GWLP_USERDATA` slot, not a thread-local, because two
// instances of this class are alive at once — one thread-local `Option<T>`,
// the pattern the rest of this module uses for the single settings window,
// cannot hold two). The window's own text (`SetWindowTextW`/`GetWindowTextW`,
// which every window answers by default, custom class or not) doubles as
// the stored "current binding" string: `apply_snapshot`'s existing
// `set_text(state.hwnd, ID_HK_UP, &snap.hotkey_up)` needs no change to
// populate this control, and [`paint_capture`] reads it back for the idle
// display. `capture_wnd_proc`'s `WM_SETTEXT` arm invalidates after
// `DefWindowProcW` stores the new text — the one place this control leans
// on a native control's behaviour it does not get for free: a real `EDIT`
// repaints itself on `WM_SETTEXT`; a bare `DefWindowProcW` does not.
//
// State machine (every transition and what it posts to the controller,
// which owns hotkey-thread suspend/resume — see `core/controller.rs`):
//   idle --click, or Space/Enter while focused-idle--> capturing
//       posts HotkeyCaptureStarted (suspends interception)
//   capturing --Esc, or WM_KILLFOCUS--> idle
//       posts HotkeyCaptureEnded (resumes interception)
//   capturing --modifier keydown--> capturing (live preview only, no post)
//   capturing --accepted non-modifier keydown--> idle
//       posts SettingChanged::HotkeyUp/Down; NOT HotkeyCaptureEnded — the
//       controller treats a rebind as its own resume (see
//       `Controller::post_hotkey_rebind`), so posting both would resume
//       twice.
//   capturing --rejected non-modifier keydown--> capturing (unchanged;
//       shows an inline message on ID_HK_ERROR, no post)
// A fifth path — the settings window itself closing while capture is still
// active — needs no explicit `HotkeyCaptureEnded` from this control at all:
// `WM_DESTROY` (`handle_destroy`) always sends `SettingsClosed`, and the
// controller already ends capture on that message unconditionally. That is
// the safety net under every path above, not just this one: even if a
// future bug skipped one of the explicit posts, closing the window still
// resumes interception.

/// Per-control capture state, stored in `GWLP_USERDATA`. `Copy` so every
/// reader/writer below takes a snapshot or writes one field at a time
/// through the raw pointer rather than holding a live `&mut` across any
/// call that could re-enter [`capture_wnd_proc`] for this same window.
#[derive(Debug, Clone, Copy)]
struct CaptureState {
    /// Whether this field is currently capturing a new binding.
    capturing: bool,
    /// Modifiers held down so far this capture, refreshed from
    /// `GetKeyState` on every modifier keydown; drives the live preview.
    live_modifiers: HOT_KEY_MODIFIERS,
}

impl Default for CaptureState {
    fn default() -> Self {
        Self {
            capturing: false,
            live_modifiers: HOT_KEY_MODIFIERS(0),
        }
    }
}

/// The prompt shown for the whole time a field is capturing and no modifier
/// is held yet. Exact wording is part of this control's contract.
const CAPTURE_PROMPT: &str = "Press a key combination… (Esc to cancel)";

/// Inline rejection: the candidate had no Ctrl/Alt/Win modifier. Shift alone
/// is deliberately insufficient — see [`has_required_modifier`].
const REJECT_NO_MODIFIER: &str = "Add Ctrl, Alt, or Win \u{2014} Shift alone can't be a hotkey";
/// Inline rejection: `hotkey_string` returned `None`, i.e. `key_name`
/// cannot represent the pressed key (so it could not round-trip through
/// `config.json`).
const REJECT_UNNAMEABLE_KEY: &str = "That key can't be used as a hotkey";
/// Inline rejection: the candidate's canonical form matches the other
/// hotkey field's current binding (see [`bindings_conflict`]).
const REJECT_DUPLICATE: &str = "Already assigned to the other brightness hotkey";

/// Retrieves `hwnd`'s [`CaptureState`] pointer from `GWLP_USERDATA`, or a
/// null pointer before `WM_CREATE` has run / after `WM_NCDESTROY` has freed
/// it.
fn capture_state_ptr(hwnd: HWND) -> *mut CaptureState {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    std::ptr::with_exposed_provenance_mut(raw.cast_unsigned())
}

/// A copy of `hwnd`'s current [`CaptureState`], or the default (idle, no
/// modifiers) if the pointer isn't set yet.
fn capture_state(hwnd: HWND) -> CaptureState {
    let ptr = capture_state_ptr(hwnd);
    if ptr.is_null() {
        CaptureState::default()
    } else {
        unsafe { *ptr }
    }
}

/// Mutates `hwnd`'s [`CaptureState`] in place through the raw pointer — no
/// Rust reference is held past this call, so nothing here can alias across
/// a re-entrant call the mutation triggers (e.g. `InvalidateRect` posts a
/// repaint asynchronously; it does not call back into this wndproc). No-op
/// before `WM_CREATE` / after `WM_NCDESTROY`.
fn with_capture_state_mut(hwnd: HWND, f: impl FnOnce(&mut CaptureState)) {
    let ptr = capture_state_ptr(hwnd);
    if !ptr.is_null() {
        unsafe { f(&mut *ptr) }
    }
}

/// Sets `hwnd`'s own window text (`SetWindowTextW` on the control itself,
/// not a child lookup — see [`get_text`]/[`window_text`] for the
/// `GetDlgItem`-based counterpart used everywhere else in this module).
/// Triggers `WM_SETTEXT`, which `capture_wnd_proc` answers by storing the
/// text via `DefWindowProcW` and then repainting.
fn set_self_text(hwnd: HWND, text: &str) {
    let wide_text = wide(text);
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide_text.as_ptr()));
    }
}

/// Schedules a repaint of `hwnd`'s whole client area.
fn invalidate(hwnd: HWND) {
    unsafe {
        let _ = InvalidateRect(Some(hwnd), None, true);
    }
}

/// Whether `modifiers` includes at least one of Ctrl/Alt/Win. Shift alone
/// fails this: a captured bare-letter-plus-Shift binding would still
/// register as a system-wide single-modifier hotkey, silently taking that
/// key away from every other application.
#[must_use]
fn has_required_modifier(modifiers: HOT_KEY_MODIFIERS) -> bool {
    modifiers.contains(MOD_CONTROL) || modifiers.contains(MOD_ALT) || modifiers.contains(MOD_WIN)
}

/// Four independent held/not-held modifier flags — a named struct rather
/// than four positional `bool` parameters (which clippy's
/// `fn_params_excessive_bools` flags past three, precisely because four
/// same-typed positional bools is a transposition hazard at every call
/// site). Each field mirrors one of `HOT_KEY_MODIFIERS`'s four independent
/// bits one-for-one, so — unlike clippy's usual concern with bool-heavy
/// structs — there is no hidden state machine or invalid combination to
/// model instead; a two-variant-enum-per-field refactor would just be this
/// same struct with extra ceremony.
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct ModifierFlags {
    ctrl: bool,
    alt: bool,
    shift: bool,
    win: bool,
}

/// Builds a `HOT_KEY_MODIFIERS` mask from [`ModifierFlags`] — the pure half
/// of reading live modifier state, split out from the `GetKeyState` calls
/// themselves ([`live_modifier_flags`]) so the mapping is unit-testable
/// without a live keyboard.
#[must_use]
fn modifiers_from_flags(flags: ModifierFlags) -> HOT_KEY_MODIFIERS {
    let mut modifiers = HOT_KEY_MODIFIERS(0);
    if flags.ctrl {
        modifiers |= MOD_CONTROL;
    }
    if flags.alt {
        modifiers |= MOD_ALT;
    }
    if flags.shift {
        modifiers |= MOD_SHIFT;
    }
    if flags.win {
        modifiers |= MOD_WIN;
    }
    modifiers
}

/// Whether `vk` is itself a modifier key (either sided variant included) —
/// such a keydown updates the live preview instead of being evaluated as a
/// candidate binding.
#[must_use]
fn is_modifier_vk(vk: VIRTUAL_KEY) -> bool {
    matches!(
        vk,
        VK_CONTROL
            | VK_LCONTROL
            | VK_RCONTROL
            | VK_MENU
            | VK_LMENU
            | VK_RMENU
            | VK_SHIFT
            | VK_LSHIFT
            | VK_RSHIFT
            | VK_LWIN
            | VK_RWIN
    )
}

/// The live modifier mask, read fresh from `GetKeyState` — always the
/// *current* held set, not an incremental track of one key, matching how a
/// user can hold e.g. Ctrl+Shift together in either press order.
fn live_modifier_flags() -> HOT_KEY_MODIFIERS {
    modifiers_from_flags(ModifierFlags {
        ctrl: key_is_down(VK_CONTROL),
        alt: key_is_down(VK_MENU),
        shift: key_is_down(VK_SHIFT),
        win: key_is_down(VK_LWIN) || key_is_down(VK_RWIN),
    })
}

/// Whether `vk` is currently held down, via `GetKeyState`'s high bit.
fn key_is_down(vk: VIRTUAL_KEY) -> bool {
    unsafe { GetKeyState(i32::from(vk.0)) < 0 }
}

/// The modifier-prefix preview shown while capturing (`"Ctrl+Shift+"`), or
/// empty while no modifier is held yet — in which case the caller shows
/// [`CAPTURE_PROMPT`] instead. Order matches [`hotkey::ParsedHotkey`]'s
/// `Display` impl (Ctrl, Alt, Shift, Win) so the preview never reorders
/// itself relative to the string a completed capture actually posts.
#[must_use]
fn preview_text(modifiers: HOT_KEY_MODIFIERS) -> String {
    let mut parts = Vec::new();
    if modifiers.contains(MOD_CONTROL) {
        parts.push("Ctrl");
    }
    if modifiers.contains(MOD_ALT) {
        parts.push("Alt");
    }
    if modifiers.contains(MOD_SHIFT) {
        parts.push("Shift");
    }
    if modifiers.contains(MOD_WIN) {
        parts.push("Win");
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{}+", parts.join("+"))
    }
}

/// What [`paint_capture`] draws: `idle_text` unchanged while idle, otherwise
/// the live preview or, before any modifier is held, [`CAPTURE_PROMPT`].
/// Pure and unit-tested without a live window — `idle_text` stands in for
/// `window_text(hwnd)`.
#[must_use]
fn capture_display_text(capturing: bool, modifiers: HOT_KEY_MODIFIERS, idle_text: &str) -> String {
    if !capturing {
        return idle_text.to_string();
    }
    let preview = preview_text(modifiers);
    if preview.is_empty() {
        CAPTURE_PROMPT.to_string()
    } else {
        preview
    }
}

/// What a completed (non-modifier) keydown while capturing resolves to.
#[derive(Debug, Clone, PartialEq)]
enum CaptureOutcome {
    /// A valid, non-conflicting binding, already formatted as the
    /// `config.json` string (e.g. `"Ctrl+Shift+Up"`).
    Accept(String),
    /// Rejected; the inline message to show on `ID_HK_ERROR`.
    Rejected(&'static str),
}

/// The accept/reject predicate for a captured `(modifiers, vk)` pair against
/// `other_binding` (the *other* hotkey field's current string). Pure and
/// composed entirely from Task 7's already-tested `hotkey_string`/
/// `bindings_conflict`: `hotkey_string` returning `None` is exactly
/// "`key_name(vk)` is `None`" (it calls `key_name` internally), so no
/// separate check is needed for that rule.
#[must_use]
fn evaluate_candidate(
    modifiers: HOT_KEY_MODIFIERS,
    vk: VIRTUAL_KEY,
    other_binding: &str,
) -> CaptureOutcome {
    if !has_required_modifier(modifiers) {
        return CaptureOutcome::Rejected(REJECT_NO_MODIFIER);
    }
    let Some(candidate) = hotkey_string(modifiers, vk) else {
        return CaptureOutcome::Rejected(REJECT_UNNAMEABLE_KEY);
    };
    if bindings_conflict(&candidate, other_binding) {
        return CaptureOutcome::Rejected(REJECT_DUPLICATE);
    }
    CaptureOutcome::Accept(candidate)
}

/// Extracts the virtual-key code carried in a keydown message's `wParam`
/// (the whole low value, unlike `WM_COMMAND`'s packed word — see
/// `handle_command`).
fn vk_from_wparam(wparam: WPARAM) -> VIRTUAL_KEY {
    VIRTUAL_KEY(u16::try_from(wparam.0).unwrap_or(0))
}

/// Enters capture: focuses the control (harmless if it already has focus),
/// marks it capturing with no modifiers held yet, repaints to the base
/// prompt, and tells the controller to suspend hotkey interception.
/// `SetFocus` runs first, before this window's own state changes, so if it
/// moves focus away from a sibling capture field that was itself mid
/// capture, that field's `WM_KILLFOCUS` cancels *its* capture (and resumes
/// interception) before this one suspends it again — no window is ever left
/// capturing without interception suspended.
fn start_capture(hwnd: HWND) {
    unsafe {
        let _ = SetFocus(Some(hwnd));
    }
    with_capture_state_mut(hwnd, |cs| {
        cs.capturing = true;
        cs.live_modifiers = HOT_KEY_MODIFIERS(0);
    });
    invalidate(hwnd);
    with_window_state(|state| send_message(state, BrightnessMessage::HotkeyCaptureStarted));
}

/// Leaves capture without a new binding (Esc or focus loss): repaints back
/// to the idle display and tells the controller to resume interception.
fn cancel_capture(hwnd: HWND) {
    with_capture_state_mut(hwnd, |cs| {
        cs.capturing = false;
        cs.live_modifiers = HOT_KEY_MODIFIERS(0);
    });
    invalidate(hwnd);
    with_window_state(|state| send_message(state, BrightnessMessage::HotkeyCaptureEnded));
}

/// Leaves capture with a new binding: displays it (via [`set_self_text`],
/// which repaints itself) and posts the `SettingChange` the caller built —
/// deliberately *not* `HotkeyCaptureEnded`, since the controller treats this
/// rebind as its own resume (see the state-machine comment at the top of
/// this section).
fn accept_capture(hwnd: HWND, candidate: String, change: fn(String) -> SettingChange) {
    with_capture_state_mut(hwnd, |cs| {
        cs.capturing = false;
        cs.live_modifiers = HOT_KEY_MODIFIERS(0);
    });
    set_self_text(hwnd, &candidate);
    with_window_state(|state| post_change(state, change(candidate)));
}

/// Rejects a candidate without leaving capture: shows `message` on the
/// shared `ID_HK_ERROR` status line so the user can just try again. Takes
/// no `hwnd` — the capture field itself doesn't change, only the shared
/// status line reached via `WindowState::hwnd`.
fn reject_capture(message: &str) {
    with_window_state(|state| set_text(state.hwnd, ID_HK_ERROR, message));
}

/// `WM_KEYDOWN`/`WM_SYSKEYDOWN` while `hwnd` is capturing: Esc cancels, a
/// modifier keydown updates the live preview, anything else is evaluated as
/// a candidate binding against whichever id is *not* `hwnd`'s own.
fn handle_capture_keydown(hwnd: HWND, vk: VIRTUAL_KEY) {
    if vk == VK_ESCAPE {
        cancel_capture(hwnd);
        return;
    }
    if is_modifier_vk(vk) {
        let modifiers = live_modifier_flags();
        with_capture_state_mut(hwnd, |cs| cs.live_modifiers = modifiers);
        invalidate(hwnd);
        return;
    }

    let own_id = u16::try_from(unsafe { GetDlgCtrlID(hwnd) }).unwrap_or(0);
    let (change, other_id): (fn(String) -> SettingChange, u16) = match own_id {
        ID_HK_UP => (SettingChange::HotkeyUp, ID_HK_DOWN),
        ID_HK_DOWN => (SettingChange::HotkeyDown, ID_HK_UP),
        // This class is only ever instantiated for the two ids above.
        _ => return,
    };
    let mut other_binding = String::new();
    with_window_state(|state| other_binding = get_text(state.hwnd, other_id));

    let modifiers = live_modifier_flags();
    match evaluate_candidate(modifiers, vk, &other_binding) {
        CaptureOutcome::Accept(candidate) => accept_capture(hwnd, candidate, change),
        CaptureOutcome::Rejected(message) => reject_capture(message),
    }
}

/// `WM_GETDLGCODE`: while capturing, claim every key so `IsDialogMessageW`
/// stops intercepting Tab/arrows/Enter/Esc as dialog navigation before this
/// control ever sees them (arrows are in the default bindings). While idle,
/// answer `DLGC_BUTTON` for everything *except* a Space/Enter keydown
/// specifically — checked via the `MSG` `lParam` carries for a
/// per-key query — which also claims the message, so pressing either key
/// while focused-idle starts capture instead of `IsDialogMessageW` routing
/// Enter to the dialog's default button (`ID_CLOSE`).
fn handle_capture_getdlgcode(hwnd: HWND, lparam: LPARAM) -> u32 {
    if capture_state(hwnd).capturing {
        return DLGC_WANTALLKEYS;
    }

    let mut code = DLGC_BUTTON;
    if lparam.0 != 0 {
        let msg_ptr: *const MSG = std::ptr::with_exposed_provenance(lparam.0.cast_unsigned());
        if let Some(msg) = unsafe { msg_ptr.as_ref() } {
            let wants_key = matches!(msg.message, WM_KEYDOWN | WM_SYSKEYDOWN)
                && matches!(vk_from_wparam(msg.wParam), VK_SPACE | VK_RETURN);
            if wants_key {
                code |= DLGC_WANTMESSAGE;
            }
        }
    }
    code
}

/// Horizontal padding, in device pixels, between the control's client edge
/// and where [`paint_capture`] draws text — matches a native `EDIT`
/// control's own left/right margin.
const CAPTURE_TEXT_INSET: i32 = 4;

/// UTF-16 encoding for `DrawTextW`, deliberately without [`wide`]'s
/// NUL terminator: `DrawTextW` takes an explicit slice length, and drawing
/// past the real text would render a trailing NUL glyph.
fn wide_for_draw(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

/// Paints one capture control: idle shows its own window text (the current
/// binding, however it last got there — population or a completed
/// capture); capturing shows the live preview or, before any modifier is
/// held, [`CAPTURE_PROMPT`] in `COLOR_GRAYTEXT` (a placeholder look,
/// `GetSysColor`-based per this task's palette rule); an idle control that
/// currently holds keyboard focus also gets a focus rectangle. Capturing
/// never draws a focus rectangle of its own — the prompt/preview already
/// signals which field is active.
fn paint_capture(hwnd: HWND, hdc: HDC) {
    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &raw mut rect) }.is_err() {
        return;
    }

    let cs = capture_state(hwnd);
    let idle_text = window_text(hwnd);
    let text = capture_display_text(cs.capturing, cs.live_modifiers, &idle_text);
    let is_placeholder = cs.capturing && preview_text(cs.live_modifiers).is_empty();
    let has_focus = unsafe { GetFocus() } == hwnd;

    with_window_state(|state| unsafe {
        let old_font = SelectObject(hdc, state.font_regular.into());
        SetBkMode(hdc, TRANSPARENT);
        let color_index = if is_placeholder {
            COLOR_GRAYTEXT
        } else {
            COLOR_WINDOWTEXT
        };
        SetTextColor(hdc, COLORREF(GetSysColor(color_index)));

        let mut text_rect = RECT {
            left: rect.left + CAPTURE_TEXT_INSET,
            top: rect.top,
            right: rect.right - CAPTURE_TEXT_INSET,
            bottom: rect.bottom,
        };
        let mut wide_text = wide_for_draw(&text);
        DrawTextW(
            hdc,
            &mut wide_text,
            &raw mut text_rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX | DT_END_ELLIPSIS,
        );

        SelectObject(hdc, old_font);
    });

    if has_focus && !cs.capturing {
        let focus_rect = RECT {
            left: rect.left + 1,
            top: rect.top + 1,
            right: rect.right - 1,
            bottom: rect.bottom - 1,
        };
        unsafe {
            let _ = DrawFocusRect(hdc, &raw const focus_rect);
        }
    }
}

/// Window procedure for the hotkey capture control class.
///
/// # Safety
///
/// This is a Windows callback. The caller (Windows) ensures `hwnd` is valid.
unsafe extern "system" fn capture_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_CREATE => {
                let ptr = Box::into_raw(Box::new(CaptureState::default()));
                let _ =
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, ptr.expose_provenance().cast_signed());
                LRESULT(0)
            }
            WM_NCDESTROY => {
                let ptr = capture_state_ptr(hwnd);
                if !ptr.is_null() {
                    drop(Box::from_raw(ptr));
                    let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                }
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_PAINT => {
                let mut ps = PAINTSTRUCT::default();
                let hdc = BeginPaint(hwnd, &raw mut ps);
                if !hdc.is_invalid() {
                    paint_capture(hwnd, hdc);
                }
                let _ = EndPaint(hwnd, &raw const ps);
                LRESULT(0)
            }
            WM_GETDLGCODE => {
                LRESULT(isize::try_from(handle_capture_getdlgcode(hwnd, lparam)).unwrap_or(0))
            }
            WM_LBUTTONDOWN => {
                if !capture_state(hwnd).capturing {
                    start_capture(hwnd);
                }
                LRESULT(0)
            }
            WM_KEYDOWN | WM_SYSKEYDOWN => {
                let vk = vk_from_wparam(wparam);
                if capture_state(hwnd).capturing {
                    handle_capture_keydown(hwnd, vk);
                    LRESULT(0)
                } else if matches!(vk, VK_SPACE | VK_RETURN) {
                    start_capture(hwnd);
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_KILLFOCUS => {
                if capture_state(hwnd).capturing {
                    cancel_capture(hwnd);
                } else {
                    invalidate(hwnd);
                }
                LRESULT(0)
            }
            WM_SETFOCUS => {
                invalidate(hwnd);
                LRESULT(0)
            }
            WM_SETTEXT => {
                let result = DefWindowProcW(hwnd, msg, wparam, lparam);
                invalidate(hwnd);
                result
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Clears this window's own slot value (never a value a newer window may
/// have written there since), notifies the controller, and frees the fonts
/// this window owns. Runs on `WM_DESTROY`, so every exit path (Esc,
/// Enter-to-Close, the Close button, Alt+F4, the system-menu Close item)
/// converges on it via `WM_CLOSE` -> `DestroyWindow` -> `WM_DESTROY`.
///
/// This and [`create_settings_window`]'s initial population are the only
/// two `WINDOW_STATE.borrow_mut()` call sites in this module — see
/// [`with_window_state`]'s doc comment for why every other reader gets away
/// with a plain `borrow()`, nested or not. `WM_DESTROY` only ever arrives
/// via the top-level `GetMessageW`/`DispatchMessageW` loop, never
/// synchronously nested under a Win32 call this module makes while a
/// `borrow()` from further up the stack is still live, so this `borrow_mut`
/// never actually contends with one (`create_settings_window`'s is safe for
/// a different reason — see that function's doc comment). If a third
/// `borrow_mut` were ever added elsewhere in this module, that safety would
/// need re-checking, and the first reentrant call to reach it would panic.
fn handle_destroy() {
    WINDOW_STATE.with(|s| {
        if let Some(state) = s.borrow_mut().take() {
            let my_value = hwnd_to_isize(state.hwnd);
            // compare_exchange, not store(0): if a second window has
            // already claimed and overwritten the slot (this window closing
            // late, after a newer one opened), clearing unconditionally
            // would strand that newer window unreachable by focus/refresh.
            let _ =
                state
                    .hwnd_slot
                    .compare_exchange(my_value, 0, Ordering::SeqCst, Ordering::SeqCst);
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
            WM_NOTIFY => {
                handle_notify(hwnd, lparam);
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
/// Its `WINDOW_STATE.with(|s| *s.borrow_mut() = Some(state))` below is one
/// of the two `borrow_mut()` call sites in this module (see
/// [`with_window_state`]'s doc comment) — safe because it runs before
/// `WINDOW_STATE` is reachable at all: every `with_window_state`/
/// `handle_destroy` reader goes through that same thread-local, and nothing
/// can call in through it before this line has run for the first time.
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
    let placement = compute_placement()?;

    let hinstance = unsafe { GetModuleHandleW(None) }.map_err(|e| {
        BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
    })?;

    // No WS_VISIBLE here: control creation, layout and snapshot population
    // all happen before the window is ever shown, so the open does not
    // visibly assemble itself on screen.
    let hwnd = unsafe {
        CreateWindowExW(
            WS_EX_TOPMOST,
            class_name,
            w!("darkbright-helper Settings"),
            WS_CAPTION | WS_SYSMENU,
            placement.x,
            placement.y,
            placement.w,
            placement.h,
            None,
            None,
            Some(hinstance.into()),
            None,
        )
    }
    .map_err(|e| BrightnessError::windows_api("CreateWindowExW", e.code().0.cast_unsigned()))?;

    let font_regular = build_font(placement.dpi, FW_NORMAL);
    let font_bold = build_font(placement.dpi, FW_BOLD);

    create_controls(hwnd, hinstance.into(), font_regular, font_bold);
    layout(hwnd, placement.dpi);
    configure_updowns(hwnd);

    let state = WindowState {
        hwnd,
        sender: tx.clone(),
        hwnd_slot: Arc::clone(hwnd_slot),
        font_regular,
        font_bold,
        // apply_snapshot (right below) overwrites every one of these before
        // the window is ever shown or focusable, so the placeholder values
        // here never reach a user-visible codepath.
        last_periodic: Cell::new(0),
        last_inactivity: Cell::new(0),
        last_posted_step: Cell::new(None),
        last_posted_timeout: Cell::new(None),
        last_posted_opacity: Cell::new(None),
        last_posted_periodic: Cell::new(None),
        last_posted_inactivity: Cell::new(None),
    };
    apply_snapshot(&state, snapshot);
    WINDOW_STATE.with(|s| *s.borrow_mut() = Some(state));

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        if let Ok(first) = GetDlgItem(Some(hwnd), i32::from(ID_AUTOSTART)) {
            let _ = SetFocus(Some(first));
        }
    }

    Ok(hwnd)
}

/// Reclaims any `WM_APP_SETTINGS_*` payload messages still queued for
/// `hwnd` after the message loop has ended. `PostQuitMessage` only yields
/// `WM_QUIT` once the rest of the thread's queue is drained, so a message
/// posted shortly before the window was destroyed is still retrieved by
/// `GetMessageW` right along with everything else in the loop above — it is
/// only `DispatchMessageW` that silently drops a message whose target
/// window is already gone, since there is no wndproc left to route it to.
/// `PeekMessageW` can still read it back by (now-stale) target `hwnd`, so
/// this reclaims each `Box` directly instead of leaking it — see
/// [`SettingsSinkImpl::post_boxed`] for the other two reclaim sites this
/// closes the loop with.
fn drain_pending_payload_messages(hwnd: HWND) {
    let mut msg = MSG::default();
    while unsafe {
        PeekMessageW(
            &raw mut msg,
            Some(hwnd),
            WM_APP_SETTINGS_REFRESH,
            WM_APP_SETTINGS_TOPMOST,
            PM_REMOVE,
        )
    }
    .as_bool()
    {
        match msg.message {
            WM_APP_SETTINGS_REFRESH => {
                let ptr: *mut SettingsSnapshot =
                    std::ptr::with_exposed_provenance_mut(msg.lParam.0.cast_unsigned());
                drop(unsafe { Box::from_raw(ptr) });
            }
            WM_APP_SETTINGS_HK_ERROR | WM_APP_SETTINGS_HK_NOTICE => {
                let ptr: *mut String =
                    std::ptr::with_exposed_provenance_mut(msg.lParam.0.cast_unsigned());
                drop(unsafe { Box::from_raw(ptr) });
            }
            // WM_APP_SETTINGS_FOCUS / WM_APP_SETTINGS_TOPMOST carry no
            // payload; PeekMessageW's range just happens to include them.
            _ => {}
        }
    }
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
            // Release the "opening" claim so a later activation can retry
            // instead of finding the slot permanently stuck.
            hwnd_slot.store(0, Ordering::SeqCst);
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

    drain_pending_payload_messages(hwnd);
    log::debug!("Settings window message loop ended");
}

// ─────────────────────────────────────────────────────────────────────────────
// Sink
// ─────────────────────────────────────────────────────────────────────────────

/// Sentinel for [`SettingsSinkImpl::hwnd`] while a window is being created:
/// claimed (via `compare_exchange` from `0`) but not yet a real handle. A
/// second `open()` landing in that window must neither spawn a competing
/// thread (there would then be two windows and only one slot) nor try to
/// focus a window that does not exist yet — it sees this value and no-ops.
const OPENING: isize = -1;

/// [`SettingsSink`] backed by a real Win32 window on its own thread.
///
/// `hwnd` is `0` when no window is open, [`OPENING`] while one is being
/// created, otherwise the real `HWND`. It crosses the thread boundary as an
/// `AtomicIsize` rather than a raw `HWND` because `HWND` is a pointer type
/// and not `Send`/`Sync` — the same seam shape `TrayStatusHandle` uses for
/// the tray window's handle. The slot is claimed with a `compare_exchange`
/// in `open()` *before* spawning, not written after the window finishes
/// creating: creation is not instant (class registration, 45 child
/// `CreateWindowExW` calls, two fonts, layout, population), and a second
/// activation landing inside that window would otherwise see the slot still
/// at `0` and spawn a duplicate window/thread.
///
/// Known limitation: if the spawned thread panics after the claim above but
/// before `run_settings_window` stores the real handle, the slot is left
/// stuck at [`OPENING`] and Settings can never be reopened without
/// restarting the process. Nothing on that path unwraps, expects, or
/// panics today, so it isn't reachable — this is recorded so a future
/// change to that path is made with the failure mode in mind, in keeping
/// with this project's fail-fast panic policy elsewhere.
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

    /// The open window's `HWND`, if one exists and isn't still [`OPENING`].
    fn target_hwnd(&self) -> Option<HWND> {
        let raw = self.hwnd.load(Ordering::SeqCst);
        (raw != 0 && raw != OPENING).then(|| hwnd_from_isize(raw))
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
    /// and dropped right here instead of leaking. A message posted just
    /// before the window is destroyed is *not* lost either: it is still
    /// retrieved by `GetMessageW` along with everything else (`PostQuitMessage`
    /// only yields `WM_QUIT` once the rest of the queue is drained) but never
    /// reaches this wndproc, since `DispatchMessageW` drops a message whose
    /// target window is already gone — `run_settings_window` closes that gap
    /// with `drain_pending_payload_messages` right after its loop ends. Every
    /// posted `Box` is reclaimed on exactly one of these three paths.
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
        match self
            .hwnd
            .compare_exchange(0, OPENING, Ordering::SeqCst, Ordering::SeqCst)
        {
            Ok(_) => {
                let tx = self.tx.clone();
                let hwnd_slot = Arc::clone(&self.hwnd);
                let snapshot = snapshot.clone();
                std::thread::spawn(move || run_settings_window(&tx, &hwnd_slot, &snapshot));
            }
            Err(OPENING) => {
                log::debug!(
                    "Settings window is already being created; ignoring a duplicate activation"
                );
            }
            Err(raw) => {
                let hwnd = hwnd_from_isize(raw);
                unsafe {
                    if let Err(e) =
                        PostMessageW(Some(hwnd), WM_APP_SETTINGS_FOCUS, WPARAM(0), LPARAM(0))
                    {
                        log::debug!(error:% = e; "Focus post failed (settings window gone?)");
                    }
                }
            }
        }
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
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_UP;

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
        // which do not, e.g. x: 155, x: 214, h: 22).
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
    fn log_level_index_matches_insertion_order_case_insensitively() {
        assert_eq!(log_level_index("error"), Some(0));
        assert_eq!(log_level_index("WARN"), Some(1));
        assert_eq!(log_level_index("Info"), Some(2));
        assert_eq!(log_level_index("debug"), Some(3));
        assert_eq!(log_level_index("trace"), Some(4));
        assert_eq!(log_level_index("bogus"), None);
    }

    #[test]
    fn parse_clamped_accepts_an_in_range_value() {
        assert_eq!(parse_clamped("30", 1, 50), 30);
        assert_eq!(parse_clamped("  30  ", 1, 50), 30);
    }

    #[test]
    fn parse_clamped_clamps_a_too_large_value_to_the_max() {
        // The scenario the brief calls out by name: typing 999 into the
        // step field (range 1-50) clamps to 50 on focus loss.
        assert_eq!(parse_clamped("999", 1, 50), 50);
    }

    #[test]
    fn parse_clamped_clamps_a_too_small_value_to_the_min() {
        assert_eq!(parse_clamped("0", 1, 50), 1);
    }

    #[test]
    fn parse_clamped_treats_empty_text_as_below_range() {
        assert_eq!(parse_clamped("", 1, 50), 1);
        assert_eq!(parse_clamped("   ", 100, 10_000), 100);
    }

    #[test]
    fn parse_clamped_treats_unparseable_text_as_below_range() {
        assert_eq!(parse_clamped("abc", 1, 50), 1);
    }

    #[test]
    fn parse_clamped_clamps_a_u64_scale_overflow_to_the_max_not_the_min() {
        // Too big to fit u32 but not u64: a real "too large" value, not
        // malformed text, so it clamps to max rather than falling back to
        // min like a parse failure would.
        assert_eq!(parse_clamped("99999999999", 1, 3600), 3600);
    }

    #[test]
    fn clamp_after_delta_adds_a_normal_step() {
        assert_eq!(clamp_after_delta(5000, 100, 100, 10_000), 5100);
        assert_eq!(clamp_after_delta(30, -1, 1, 50), 29);
    }

    #[test]
    fn clamp_after_delta_stops_at_the_top_of_the_range() {
        assert_eq!(clamp_after_delta(50, 1, 1, 50), 50);
    }

    #[test]
    fn clamp_after_delta_stops_at_the_bottom_without_underflowing() {
        assert_eq!(clamp_after_delta(1, -1, 1, 50), 1);
    }

    #[test]
    fn remembered_seconds_keeps_a_nonzero_snapshot_value() {
        assert_eq!(remembered_seconds(45, DEFAULT_REFRESH_PERIODIC_SECONDS), 45);
    }

    #[test]
    fn remembered_seconds_falls_back_to_the_default_when_the_snapshot_was_zero() {
        assert_eq!(
            remembered_seconds(0, DEFAULT_REFRESH_PERIODIC_SECONDS),
            DEFAULT_REFRESH_PERIODIC_SECONDS
        );
        assert_eq!(
            remembered_seconds(0, DEFAULT_REFRESH_INACTIVITY_SECONDS),
            DEFAULT_REFRESH_INACTIVITY_SECONDS
        );
    }

    #[test]
    fn should_post_numeric_is_true_the_first_time_a_field_ever_commits() {
        // No prior post to compare against (None) always counts as changed,
        // regardless of the value — there is nothing to be "unchanged" from.
        assert!(should_post_numeric(None, 0));
        assert!(should_post_numeric(None, 30));
    }

    #[test]
    fn should_post_numeric_is_false_when_the_value_matches_the_last_post() {
        assert!(!should_post_numeric(Some(30), 30));
    }

    #[test]
    fn should_post_numeric_is_true_when_the_value_differs_from_the_last_post() {
        assert!(should_post_numeric(Some(30), 31));
    }

    #[test]
    fn to_u8_narrows_values_within_its_callers_ranges() {
        assert_eq!(to_u8(1), 1u8);
        assert_eq!(to_u8(50), 50u8);
        assert_eq!(to_u8(100), 100u8);
    }

    #[test]
    fn every_numeric_field_id_matches_a_real_control_in_the_layout_table() {
        let ids: std::collections::HashSet<u16> = CONTROLS.iter().map(|c| c.id).collect();
        for field in NUMERIC_FIELDS {
            assert!(
                ids.contains(&field.edit_id),
                "NumericField.edit_id {} has no CONTROLS entry",
                field.edit_id
            );
            assert!(
                ids.contains(&field.updown_id),
                "NumericField.updown_id {} has no CONTROLS entry",
                field.updown_id
            );
            if let Some(checkbox_id) = field.checkbox_id {
                assert!(
                    ids.contains(&checkbox_id),
                    "NumericField.checkbox_id {checkbox_id} has no CONTROLS entry"
                );
            }
            assert!(
                field.min <= field.max,
                "NumericField for edit {} has min > max",
                field.edit_id
            );
        }
    }

    #[test]
    fn every_numeric_field_range_matches_the_spinner_range_it_names() {
        // configure_updowns now reads its ranges straight from
        // NUMERIC_FIELDS (only its accel table is still hand-written), so
        // there is no second copy left for this table to drift from — this
        // pins STEP_RANGE/OSD_TIMEOUT_RANGE/... against a checked-in
        // expectation instead, so an accidental edit to one of those
        // constants shows up as a failing test rather than silently
        // reaching every spinner and every EN_KILLFOCUS/UDN_DELTAPOS clamp.
        let expected: &[(u16, u32, u32)] = &[
            (ID_STEP_UPDOWN, 1, 50),
            (ID_OSD_TIMEOUT_UPDOWN, 100, 10_000),
            (ID_OSD_OPACITY_UPDOWN, 10, 100),
            (ID_RESYNC_UPDOWN, 1, 3600),
            (ID_INACT_UPDOWN, 1, 600),
        ];
        for &(updown_id, min, max) in expected {
            let field = NUMERIC_FIELDS
                .iter()
                .find(|f| f.updown_id == updown_id)
                .unwrap_or_else(|| panic!("no NumericField for updown {updown_id}"));
            assert_eq!(field.min, min, "min mismatch for updown {updown_id}");
            assert_eq!(field.max, max, "max mismatch for updown {updown_id}");
        }
    }

    // ── Hotkey Capture Control (pure logic) ─────────────────────────────
    // Win32-free: no window, no live keyboard. `evaluate_candidate` calls
    // straight into Task 7's already-tested `hotkey_string`/
    // `bindings_conflict`, so these tests exercise the actual accept/reject
    // rules end to end, not a re-implementation of them.

    #[test]
    fn modifiers_from_flags_sets_exactly_the_flags_that_are_true() {
        let m = modifiers_from_flags(ModifierFlags {
            ctrl: true,
            alt: false,
            shift: true,
            win: false,
        });
        assert!(m.contains(MOD_CONTROL));
        assert!(!m.contains(MOD_ALT));
        assert!(m.contains(MOD_SHIFT));
        assert!(!m.contains(MOD_WIN));
    }

    #[test]
    fn modifiers_from_flags_all_false_is_the_empty_mask() {
        assert_eq!(
            modifiers_from_flags(ModifierFlags::default()),
            HOT_KEY_MODIFIERS(0)
        );
    }

    #[test]
    fn has_required_modifier_accepts_ctrl_alt_or_win() {
        assert!(has_required_modifier(MOD_CONTROL));
        assert!(has_required_modifier(MOD_ALT));
        assert!(has_required_modifier(MOD_WIN));
        assert!(has_required_modifier(MOD_CONTROL | MOD_SHIFT));
    }

    #[test]
    fn has_required_modifier_rejects_shift_alone_and_no_modifier() {
        assert!(!has_required_modifier(MOD_SHIFT));
        assert!(!has_required_modifier(HOT_KEY_MODIFIERS(0)));
    }

    #[test]
    fn is_modifier_vk_recognizes_every_sided_variant() {
        for vk in [
            VK_CONTROL,
            VK_LCONTROL,
            VK_RCONTROL,
            VK_MENU,
            VK_LMENU,
            VK_RMENU,
            VK_SHIFT,
            VK_LSHIFT,
            VK_RSHIFT,
            VK_LWIN,
            VK_RWIN,
        ] {
            assert!(is_modifier_vk(vk), "{vk:?} should be a modifier key");
        }
    }

    #[test]
    fn is_modifier_vk_rejects_an_ordinary_key() {
        assert!(!is_modifier_vk(VK_UP));
        assert!(!is_modifier_vk(VIRTUAL_KEY(0x42))); // 'B'
    }

    #[test]
    fn preview_text_is_empty_with_no_modifiers_held() {
        assert_eq!(preview_text(HOT_KEY_MODIFIERS(0)), "");
    }

    #[test]
    fn preview_text_orders_modifiers_ctrl_alt_shift_win() {
        let mods = MOD_WIN | MOD_SHIFT | MOD_ALT | MOD_CONTROL;
        assert_eq!(preview_text(mods), "Ctrl+Alt+Shift+Win+");
    }

    #[test]
    fn preview_text_a_single_modifier_still_gets_a_trailing_plus() {
        assert_eq!(preview_text(MOD_CONTROL), "Ctrl+");
    }

    #[test]
    fn capture_display_text_shows_idle_text_while_not_capturing() {
        assert_eq!(
            capture_display_text(false, MOD_CONTROL, "Ctrl+Shift+Up"),
            "Ctrl+Shift+Up"
        );
    }

    #[test]
    fn capture_display_text_shows_the_prompt_before_any_modifier_is_held() {
        assert_eq!(
            capture_display_text(true, HOT_KEY_MODIFIERS(0), "Ctrl+Shift+Up"),
            CAPTURE_PROMPT
        );
    }

    #[test]
    fn capture_display_text_shows_the_live_preview_once_a_modifier_is_held() {
        assert_eq!(
            capture_display_text(true, MOD_CONTROL, "Ctrl+Shift+Up"),
            "Ctrl+"
        );
    }

    #[test]
    fn evaluate_candidate_accepts_a_valid_non_conflicting_binding() {
        let outcome = evaluate_candidate(MOD_CONTROL, VIRTUAL_KEY(0x42), "Ctrl+Shift+Up"); // Ctrl+B
        assert_eq!(outcome, CaptureOutcome::Accept("Ctrl+B".to_string()));
    }

    #[test]
    fn evaluate_candidate_rejects_no_modifier_at_all() {
        let outcome = evaluate_candidate(HOT_KEY_MODIFIERS(0), VIRTUAL_KEY(0x42), "Ctrl+Shift+Up");
        assert_eq!(outcome, CaptureOutcome::Rejected(REJECT_NO_MODIFIER));
    }

    #[test]
    fn evaluate_candidate_rejects_shift_alone() {
        let outcome = evaluate_candidate(MOD_SHIFT, VK_UP, "Ctrl+Shift+Down");
        assert_eq!(outcome, CaptureOutcome::Rejected(REJECT_NO_MODIFIER));
    }

    #[test]
    fn evaluate_candidate_rejects_a_key_the_parser_cannot_name() {
        // No entry in KEY_MAP/VK_TO_NAME for this virtual-key code.
        let outcome = evaluate_candidate(MOD_CONTROL, VIRTUAL_KEY(0x07), "Ctrl+Shift+Up");
        assert_eq!(outcome, CaptureOutcome::Rejected(REJECT_UNNAMEABLE_KEY));
    }

    #[test]
    fn evaluate_candidate_rejects_a_binding_the_other_field_already_has() {
        let outcome = evaluate_candidate(MOD_CONTROL | MOD_SHIFT, VK_UP, "Ctrl+Shift+Up");
        assert_eq!(outcome, CaptureOutcome::Rejected(REJECT_DUPLICATE));
    }

    #[test]
    fn evaluate_candidate_rejects_a_duplicate_in_a_different_canonical_spelling() {
        // The scenario the brief calls out by name: a hand-edited
        // config.json spelling must still be caught as the same binding.
        let outcome = evaluate_candidate(MOD_CONTROL | MOD_SHIFT, VK_UP, "shift+ctrl+up");
        assert_eq!(outcome, CaptureOutcome::Rejected(REJECT_DUPLICATE));
    }

    #[test]
    fn evaluate_candidate_does_not_conflict_with_unparseable_other_field_text() {
        // Matches bindings_conflict's own permissiveness: an unparseable
        // "other" binding never blocks a capture.
        let outcome = evaluate_candidate(MOD_CONTROL, VK_UP, "garbage");
        assert_eq!(outcome, CaptureOutcome::Accept("Ctrl+Up".to_string()));
    }
}
