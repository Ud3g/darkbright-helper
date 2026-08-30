//! Dark-mode painting for the settings window.
//!
//! `theme.rs` only grants the process-wide uxtheme opt-in and answers "does
//! the system want dark UI"; everything about *how* that paints onto this
//! window's controls lives here. A dark-mode spike (recorded in this
//! feature's history) found that `SetWindowTheme` plus `WM_CTLCOLOR*` alone
//! is not enough: themed controls draw their own text via
//! `DrawThemeText(Ex)` using the theme's colour, not whatever the parent's
//! device context has selected. Concretely, per control class:
//!
//! - Plain `STATIC` labels and push buttons take dark mode correctly from
//!   `SetWindowTheme("DarkMode_Explorer")` and (for statics) `WM_CTLCOLORSTATIC`
//!   alone — no special handling needed here beyond the colour table below.
//! - Checkbox labels are invisible under that combination (themed `BUTTON`
//!   ignores `WM_CTLCOLORSTATIC`'s text colour entirely), so they are
//!   custom-drawn via `NM_CUSTOMDRAW` — see [`checkbox_custom_draw`].
//! - A themed `EDIT` renders white-on-white no matter what
//!   `WM_CTLCOLOREDIT` returns (visual styles win over `WM_CTLCOLOR*` for a
//!   themed edit), so the five numeric edits are never themed at all; an
//!   *unthemed* edit takes `WM_CTLCOLOREDIT` correctly, and the sunken
//!   frame visual styles would otherwise have drawn is hand-painted back in
//!   on `WM_NCPAINT` — see [`install_edit_border_subclass`].
//! - The combo's closed face and the `msctls_updown32` spinner buttons are
//!   hand-painted outright — see [`install_combo_subclass`] and
//!   [`install_updown_subclass`].
//!
//! Every technique below is Microsoft-documented (custom draw, window
//! subclassing, the visual-styles `DrawTheme*` family) or an established
//! convention of other open-source dark-mode implementations (the
//! `"CFD::COMBOBOX"` compound theme-class string for the combo's dark arrow
//! glyph) — nothing here resolves a new undocumented ordinal.

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, SIZE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_BTNFACE, COLOR_GRAYTEXT, CreatePen, CreateSolidBrush, DT_LEFT, DT_NOPREFIX,
    DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect,
    GetStockObject, GetSysColor, GetSysColorBrush, GetWindowDC, HBRUSH, HDC, HFONT, NULL_PEN,
    PAINTSTRUCT, PS_SOLID, Polygon, RDW_ALLCHILDREN, RDW_ERASE, RDW_FRAME, RDW_INVALIDATE,
    RDW_UPDATENOW, RedrawWindow, ReleaseDC, RoundRect, SelectObject, SetBkColor, SetBkMode,
    SetTextColor, TRANSPARENT,
};
use windows::Win32::UI::Controls::{
    BP_CHECKBOX, BST_CHECKED, CBRO_DISABLED, CBRO_NORMAL, CBS_CHECKEDDISABLED, CBS_CHECKEDHOT,
    CBS_CHECKEDNORMAL, CBS_CHECKEDPRESSED, CBS_UNCHECKEDDISABLED, CBS_UNCHECKEDHOT,
    CBS_UNCHECKEDNORMAL, CBS_UNCHECKEDPRESSED, CBXSR_DISABLED, CBXSR_NORMAL, CDDS_PREPAINT,
    CDIS_DISABLED, CDIS_HOT, CDIS_SELECTED, CDRF_DODEFAULT, CDRF_SKIPDEFAULT, CHECKBOXSTATES,
    COMBOBOXINFO, CP_DROPDOWNBUTTONRIGHT, CP_DROPDOWNITEM, CloseThemeData, DTT_TEXTCOLOR, DTTOPTS,
    DrawThemeBackground, DrawThemeTextEx, GetComboBoxInfo, GetThemePartSize, HTHEME, NMCUSTOMDRAW,
    NMCUSTOMDRAW_DRAW_STATE_FLAGS, OpenThemeData, SetWindowTheme, TS_DRAW,
};
use windows::Win32::UI::Input::KeyboardAndMouse::IsWindowEnabled;
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, GCLP_HBRBACKGROUND, GetClientRect, GetDlgCtrlID, GetDlgItem, GetWindowRect,
    SendMessageW, SetClassLongPtrW, WM_ERASEBKGND, WM_NCDESTROY, WM_NCPAINT, WM_PAINT,
};
use windows::core::{PCWSTR, w};

use super::super::theme;
use super::layout::{
    CONTROLS, ID_AUTOSTART, ID_HK_ERROR, ID_HK_HINT, ID_INACT_CHECK, ID_INACT_EDIT, ID_INTERCEPT,
    ID_LOG_CHECK, ID_LOG_HINT, ID_OSD_OPACITY_EDIT, ID_OSD_TIMEOUT_EDIT, ID_RESYNC_CHECK,
    ID_RESYNC_EDIT, ID_STEP_EDIT,
};
use super::window::{WindowState, window_text, with_window_state};

// ─────────────────────────────────────────────────────────────────────────────
// Palette
// ─────────────────────────────────────────────────────────────────────────────
// COLORREF is 0x00bbggrr; every constant below is commented with the RGB
// triple it represents, matching the convention osd_render.rs already uses
// for its own colour constants.

/// Window background. BGR: 32, 32, 32.
pub(super) const DARK_WINDOW_BG: u32 = 0x0020_2020;
/// Background for anything that reads as "a control sitting on the window"
/// (edits, the combo's dropdown list, a disabled edit routed through
/// `WM_CTLCOLORSTATIC`). BGR: 43, 43, 43.
pub(super) const DARK_CONTROL_BG: u32 = 0x002B_2B2B;
/// Primary text. BGR: 240, 240, 240.
pub(super) const DARK_TEXT: u32 = 0x00F0_F0F0;
/// Muted/hint text and a disabled control's text. BGR: 160, 160, 160.
pub(super) const DARK_GRAY_TEXT: u32 = 0x00A0_A0A0;
/// The hotkey status line's error colour (tomato, RGB 255, 99, 71).
pub(super) const DARK_ERROR_TEXT: u32 = 0x0047_63FF;
/// The hotkey status line's error colour in light mode — not part of the
/// dark palette above, but decided alongside it: the error line must read
/// red in both themes (see the module doc comment on `WM_CTLCOLORSTATIC`
/// below), and light mode has no other source for that colour. RGB(200, 0,
/// 0), chosen as a readable "error red" against the light window
/// background rather than the literal dark-mode tomato.
const LIGHT_ERROR_TEXT: u32 = 0x0000_00C8;
/// A visible frame colour for the numeric edits' hand-painted border and the
/// updown spinner's button outline — not one of the five colours the dark
/// palette above defines (those cover text and fill, not a border), chosen
/// as a mid-grey that reads clearly against both `DARK_WINDOW_BG` and
/// `DARK_CONTROL_BG` without the harsh contrast a full-white border would
/// have. BGR: 90, 90, 90.
const DARK_BORDER: u32 = 0x005A_5A5A;

/// The two background brushes every `WM_CTLCOLOR*` handler below can return
/// while dark mode is active, plus the hand-painted subclasses' own fills.
/// Created once at window creation and freed on `WM_DESTROY` — text colours
/// need no such handle, since `SetTextColor` takes a plain `COLORREF`.
pub(super) struct Palette {
    pub(super) window_bg: HBRUSH,
    pub(super) control_bg: HBRUSH,
}

impl Palette {
    /// Creates both brushes. Best-effort, matching the rest of this crate's
    /// GDI resource creation: a failed `CreateSolidBrush` yields an invalid
    /// handle, logged and otherwise left alone (a `WM_CTLCOLOR*` handler
    /// returning an invalid brush just falls back to `DefWindowProcW`'s
    /// default painting for that one control, not a crash).
    pub(super) fn new() -> Self {
        let window_bg = unsafe { CreateSolidBrush(COLORREF(DARK_WINDOW_BG)) };
        let control_bg = unsafe { CreateSolidBrush(COLORREF(DARK_CONTROL_BG)) };
        if window_bg.is_invalid() || control_bg.is_invalid() {
            log::warn!("Failed to create one or more dark-mode background brushes");
        }
        Self {
            window_bg,
            control_bg,
        }
    }

    /// Frees both brushes. Must be called exactly once, from
    /// `handle_destroy` — the same single-owner convention as the window's
    /// two fonts.
    pub(super) fn destroy(&self) {
        unsafe {
            let _ = DeleteObject(self.window_bg.into());
            let _ = DeleteObject(self.control_bg.into());
        }
    }
}

/// Whether this system should open the settings window in dark mode: the
/// undocumented uxtheme opt-in has to have resolved *and* the user has to
/// actually prefer dark — either alone is not enough (see
/// `theme::dark_ui_available`'s doc comment for why painting a dark palette
/// under controls that were never granted the visual style would be
/// dark-on-dark). Where the opt-in is unavailable, the window paints light,
/// full stop.
#[must_use]
pub(super) fn initial_dark_flag() -> bool {
    theme::dark_ui_available() && theme::system_prefers_dark()
}

// ─────────────────────────────────────────────────────────────────────────────
// Applying The Theme
// ─────────────────────────────────────────────────────────────────────────────

/// Applies `state.dark`'s current value to the whole window: the title bar,
/// every themeable child control, the window's own class background, and a
/// full repaint. Called once at creation and again whenever
/// `WM_SETTINGCHANGE` reports the system theme changed.
pub(super) fn apply_theme(state: &WindowState) {
    let hwnd = state.hwnd;
    let dark = state.dark.get();

    theme::allow_dark_mode_for_window(hwnd);
    theme::enable_dark_title_bar(hwnd, dark);

    for spec in CONTROLS {
        let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(spec.id)) }) else {
            continue;
        };
        match spec.class {
            // Checkboxes and push buttons; SysLink scrollbars/chrome.
            "BUTTON" | "SysLink" => apply_window_theme(child, dark),
            "COMBOBOX" => apply_combo_theme(child, dark),
            // EDIT is deliberately never themed — see the module doc
            // comment. msctls_updown32 is fully hand-painted by its own
            // subclass, so no visual-style association is needed either.
            // HOTKEY_CAPTURE and STATIC have no theme name to apply.
            _ => {}
        }
    }

    set_class_background(hwnd, dark, &state.palette);
    redraw_all(hwnd);
    redraw_hand_painted_borders(hwnd);

    log::debug!(dark; "Settings window theme applied");
}

/// Applies (or clears) `DarkMode_Explorer` on one button-class or `SysLink`
/// control. `PCWSTR::null()` for `pszSubAppName` is the documented way to
/// restore a window's default theme, so light mode needs no separate
/// "Explorer" name.
fn apply_window_theme(hwnd: HWND, dark: bool) {
    let name = if dark {
        w!("DarkMode_Explorer")
    } else {
        PCWSTR::null()
    };
    unsafe {
        let _ = SetWindowTheme(hwnd, name, PCWSTR::null());
    }
}

/// Applies (or clears) the combo's own theme association — plain `"CFD"`,
/// used only so `GetComboBoxInfo`'s button/item metrics match what
/// [`paint_combo`] draws (see the module doc comment); the actual paint is
/// always hand-drawn, never delegated to this theme name. Also themes the
/// internal dropdown listbox directly, which is the one part of a combo box
/// that *does* render correctly from `SetWindowTheme` +
/// `WM_CTLCOLORLISTBOX` alone (confirmed in the dark-mode spike).
fn apply_combo_theme(hwnd: HWND, dark: bool) {
    let name = if dark { w!("CFD") } else { PCWSTR::null() };
    unsafe {
        let _ = SetWindowTheme(hwnd, name, PCWSTR::null());
    }

    let mut cbi = COMBOBOXINFO {
        cbSize: u32::try_from(std::mem::size_of::<COMBOBOXINFO>()).unwrap_or(0),
        ..Default::default()
    };
    if unsafe { GetComboBoxInfo(hwnd, &raw mut cbi) }.is_ok() && !cbi.hwndList.is_invalid() {
        let list_name = if dark {
            w!("DarkMode_Explorer")
        } else {
            PCWSTR::null()
        };
        unsafe {
            let _ = SetWindowTheme(cbi.hwndList, list_name, PCWSTR::null());
        }
    }
}

/// Swaps the settings window class's own background brush — what
/// `WM_ERASEBKGND` paints before any child control does, and the only
/// source of colour for the small margins between controls.
///
/// Also called directly from `handle_destroy`, with `dark: false`, to put
/// the class back on `GetSysColorBrush(COLOR_BTNFACE)` before the palette's
/// brushes are freed — `GCLP_HBRBACKGROUND` is process-global class state
/// that outlives this one window, so leaving it pointed at a brush this
/// window is about to delete would hand the next-created window of this
/// class a dangling handle (GDI recycles freed handle values) until that
/// window's own `apply_theme` ran and overwrote it.
pub(super) fn set_class_background(hwnd: HWND, dark: bool, palette: &Palette) {
    let brush = if dark {
        palette.window_bg
    } else {
        unsafe { GetSysColorBrush(COLOR_BTNFACE) }
    };
    unsafe {
        SetClassLongPtrW(
            hwnd,
            GCLP_HBRBACKGROUND,
            brush.0.expose_provenance().cast_signed(),
        );
    }
}

/// Forces every pixel of the window and its children to repaint immediately
/// with whatever theme [`apply_theme`] just applied.
///
/// `RDW_FRAME` here only asks for a fresh `WM_NCPAINT` on the *window handle
/// passed in* — `RDW_ALLCHILDREN` extends this call's client-area
/// invalidation to every child, but a child's own non-client area still
/// needs its own frame request. [`redraw_hand_painted_borders`] covers the
/// controls that actually hand-paint one; every other child's non-client
/// area (native `WS_BORDER`/theme chrome Windows itself draws) has nothing
/// depending on `WM_NCPAINT` firing here.
fn redraw_all(hwnd: HWND) {
    unsafe {
        let _ = RedrawWindow(
            Some(hwnd),
            None,
            None,
            RDW_INVALIDATE | RDW_FRAME | RDW_ERASE | RDW_ALLCHILDREN | RDW_UPDATENOW,
        );
    }
}

/// Forces a fresh `WM_NCPAINT` on every control whose border is hand-painted
/// rather than left to `DefWindowProc`/`DefSubclassProc`: the five numeric
/// edits (subclassed) and the two hotkey capture fields (their own window
/// class's wndproc). `RedrawWindow`'s `RDW_FRAME` flag is per-window-handle,
/// so [`redraw_all`]'s single call against the top level never reaches these
/// — each needs its own call, addressed directly by its own `HWND`.
///
/// The capture fields do not strictly need this (their first real
/// `WM_NCPAINT` already runs after `WINDOW_STATE` holds live state, since
/// nothing forces an early one the way the edits' subclass installation
/// does), but including them is harmless and keeps this list "every
/// hand-painted border" rather than "every hand-painted border that
/// happened to need it".
fn redraw_hand_painted_borders(hwnd: HWND) {
    for spec in CONTROLS {
        if !matches!(spec.class, "EDIT" | "HOTKEY_CAPTURE") {
            continue;
        }
        let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(spec.id)) }) else {
            continue;
        };
        unsafe {
            let _ = RedrawWindow(
                Some(child),
                None,
                None,
                RDW_FRAME | RDW_INVALIDATE | RDW_UPDATENOW,
            );
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// WM_SETTINGCHANGE
// ─────────────────────────────────────────────────────────────────────────────

/// Whether a `WM_SETTINGCHANGE` broadcast is the one Windows sends when the
/// system light/dark setting changes: `lParam` then points at a
/// NUL-terminated `"ImmersiveColorSet"`. A `WM_SETTINGCHANGE` for any other
/// setting either carries a different string or none at all (`lParam == 0`),
/// both of which read as "not this one" rather than being treated as a
/// theme change.
#[must_use]
pub(super) fn is_immersive_color_set_change(lparam: LPARAM) -> bool {
    if lparam.0 == 0 {
        return false;
    }
    // SAFETY: per WM_SETTINGCHANGE's documented contract, a non-zero lParam
    // points at a NUL-terminated UTF-16 string that stays valid for the
    // duration of this synchronous message.
    let text =
        unsafe { PCWSTR(std::ptr::with_exposed_provenance(lparam.0.cast_unsigned())).to_string() };
    matches!(text.as_deref(), Ok("ImmersiveColorSet"))
}

// ─────────────────────────────────────────────────────────────────────────────
// WM_CTLCOLOR* — Plain STATIC / EDIT / BUTTON / LISTBOX
// ─────────────────────────────────────────────────────────────────────────────
// Every handler below follows the same shape: decide a colour purely from
// (control id, dark) — the part worth unit-testing — then, only if that
// decision says to override anything, perform the actual SetTextColor/
// SetBkColor/brush-return dance. Light mode leaves every *plain* static,
// edit, button and listbox exactly as DefWindowProcW would have painted it;
// the two hint statics and the error line are the one exception, and read
// their own fixed colour in both themes (an earlier task deferred deciding
// their colour to this one).

/// One `WM_CTLCOLORSTATIC` control's colour treatment, decided purely from
/// its id and whether dark mode is active. `None` means "leave
/// `DefWindowProcW`'s answer alone".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StaticColor {
    /// A literal `SetTextColor` value, paired with the window background.
    Fixed(u32),
    /// A literal `SetTextColor` value, paired with the *control* background
    /// — used only for a numeric edit that Windows re-routes through
    /// `WM_CTLCOLORSTATIC` while it is disabled, so its background still
    /// matches the sunken frame the edit's own `WM_NCPAINT` subclass draws
    /// around it, rather than the plain window background every other
    /// static sits on.
    FixedOnControl(u32),
    /// `GetSysColor(COLOR_GRAYTEXT)`, resolved at paint time — light mode's
    /// hint colour, a system colour rather than a literal.
    SysGray,
}

/// The five numeric-edit ids that can arrive as `WM_CTLCOLORSTATIC` instead
/// of `WM_CTLCOLOREDIT` while disabled — Windows re-routes a disabled
/// `EDIT` through the static path.
fn is_numeric_edit_id(id: u16) -> bool {
    matches!(
        id,
        ID_STEP_EDIT | ID_OSD_TIMEOUT_EDIT | ID_OSD_OPACITY_EDIT | ID_RESYNC_EDIT | ID_INACT_EDIT
    )
}

fn static_color(id: u16, dark: bool) -> Option<StaticColor> {
    if id == ID_HK_ERROR {
        return Some(StaticColor::Fixed(if dark {
            DARK_ERROR_TEXT
        } else {
            LIGHT_ERROR_TEXT
        }));
    }
    if id == ID_HK_HINT || id == ID_LOG_HINT {
        return Some(if dark {
            StaticColor::Fixed(DARK_GRAY_TEXT)
        } else {
            StaticColor::SysGray
        });
    }
    if !dark {
        return None;
    }
    if is_numeric_edit_id(id) {
        return Some(StaticColor::FixedOnControl(DARK_GRAY_TEXT));
    }
    Some(StaticColor::Fixed(DARK_TEXT))
}

/// `WM_CTLCOLORSTATIC`. `wparam`/`lparam` are the message's own HDC/child
/// HWND; `None` tells the caller to fall back to `DefWindowProcW`.
pub(super) fn ctlcolor_static(
    state: &WindowState,
    wparam: WPARAM,
    lparam: LPARAM,
) -> Option<LRESULT> {
    let hdc = HDC(std::ptr::with_exposed_provenance_mut(wparam.0));
    let child = super::super::hwnd_from_isize(lparam.0);
    let id = u16::try_from(unsafe { GetDlgCtrlID(child) }).unwrap_or(0);
    let dark = state.dark.get();
    let choice = static_color(id, dark)?;

    let (color, brush) = match choice {
        StaticColor::Fixed(c) => (
            c,
            if dark {
                state.palette.window_bg
            } else {
                unsafe { GetSysColorBrush(COLOR_BTNFACE) }
            },
        ),
        StaticColor::FixedOnControl(c) => (c, state.palette.control_bg),
        StaticColor::SysGray => (unsafe { GetSysColor(COLOR_GRAYTEXT) }, unsafe {
            GetSysColorBrush(COLOR_BTNFACE)
        }),
    };
    unsafe {
        SetTextColor(hdc, COLORREF(color));
        SetBkMode(hdc, TRANSPARENT);
    }
    Some(brush_lresult(brush))
}

/// `WM_CTLCOLOREDIT`: dark-only, matching a plain unthemed edit's default
/// light-mode rendering.
pub(super) fn ctlcolor_edit(state: &WindowState, wparam: WPARAM) -> Option<LRESULT> {
    if !state.dark.get() {
        return None;
    }
    let hdc = HDC(std::ptr::with_exposed_provenance_mut(wparam.0));
    unsafe {
        SetTextColor(hdc, COLORREF(DARK_TEXT));
        SetBkColor(hdc, COLORREF(DARK_CONTROL_BG));
    }
    Some(brush_lresult(state.palette.control_bg))
}

/// `WM_CTLCOLORBTN`: dark-only. Checkboxes fully bypass this via
/// `NM_CUSTOMDRAW`'s `CDRF_SKIPDEFAULT`, and push buttons paint themselves
/// via `DarkMode_Explorer` regardless of what this returns (confirmed in
/// the dark-mode spike); implemented anyway for parity with the brief and
/// as a defensive fallback for any button state that isn't fully custom-drawn.
pub(super) fn ctlcolor_btn(state: &WindowState, wparam: WPARAM) -> Option<LRESULT> {
    if !state.dark.get() {
        return None;
    }
    let hdc = HDC(std::ptr::with_exposed_provenance_mut(wparam.0));
    unsafe {
        SetTextColor(hdc, COLORREF(DARK_TEXT));
        SetBkMode(hdc, TRANSPARENT);
    }
    Some(brush_lresult(state.palette.window_bg))
}

/// `WM_CTLCOLORLISTBOX`: the combo's dropdown list, reflected up to this
/// window's own wndproc (a standard, non-owner-drawn combo box forwards its
/// list's `WM_CTLCOLORLISTBOX` to its own parent). Dark-only.
pub(super) fn ctlcolor_listbox(state: &WindowState, wparam: WPARAM) -> Option<LRESULT> {
    if !state.dark.get() {
        return None;
    }
    let hdc = HDC(std::ptr::with_exposed_provenance_mut(wparam.0));
    unsafe {
        SetTextColor(hdc, COLORREF(DARK_TEXT));
        SetBkColor(hdc, COLORREF(DARK_CONTROL_BG));
    }
    Some(brush_lresult(state.palette.control_bg))
}

/// Encodes a background brush handle as the `LRESULT` a `WM_CTLCOLOR*`
/// handler returns.
fn brush_lresult(brush: HBRUSH) -> LRESULT {
    LRESULT(brush.0.expose_provenance().cast_signed())
}

// ─────────────────────────────────────────────────────────────────────────────
// Checkbox Labels (NM_CUSTOMDRAW)
// ─────────────────────────────────────────────────────────────────────────────
// A themed BUTTON-class control paints its own text via DrawThemeText and
// ignores WM_CTLCOLORSTATIC's colour entirely — confirmed in the dark-mode
// spike, where checkbox labels measured 1.27:1 contrast (effectively
// invisible) against the dark background while plain STATIC labels in the
// same window measured 14.3:1. Button controls support NM_CUSTOMDRAW since
// Windows Vista (a Microsoft-documented custom-draw class, not the ordinal
// surface theme.rs isolates), so the five checkboxes' labels are painted
// entirely by hand at CDDS_PREPAINT: the glyph via DrawThemeBackground, the
// text via a plain SetTextColor + DrawTextW, then CDRF_SKIPDEFAULT tells the
// control not to paint anything itself afterward.

/// Horizontal gap, in device pixels, between the checkbox glyph and its
/// label text — matches the spacing a themed checkbox's own default paint
/// uses.
const CHECKBOX_TEXT_GAP: i32 = 4;

/// Whether `id` is one of the five checkboxes whose label needs hand-painting.
#[must_use]
pub(super) fn is_checkbox_id(id: u16) -> bool {
    matches!(
        id,
        ID_AUTOSTART | ID_INTERCEPT | ID_RESYNC_CHECK | ID_INACT_CHECK | ID_LOG_CHECK
    )
}

/// `NM_CUSTOMDRAW` for a checkbox: paints the glyph and label at
/// `CDDS_PREPAINT` and returns `CDRF_SKIPDEFAULT`, or `CDRF_DODEFAULT` for
/// any other draw stage or when dark mode is off (a checkbox's default
/// paint is already correct in light mode).
pub(super) fn checkbox_custom_draw(cd: &NMCUSTOMDRAW) -> u32 {
    let mut dark = false;
    let mut font_regular = None;
    with_window_state(|state| {
        dark = state.dark.get();
        font_regular = Some(state.font_regular.get());
    });
    if !dark || cd.dwDrawStage != CDDS_PREPAINT {
        return CDRF_DODEFAULT;
    }

    let hwnd = cd.hdr.hwndFrom;
    let hdc = cd.hdc;
    let rect = cd.rc;

    // CDRF_SKIPDEFAULT below tells the button to skip its own paint
    // entirely, background fill included — this control never gets the
    // parent's WM_ERASEBKGND clear a plain repaint (e.g. hot/pressed state
    // changing, or a check/uncheck) relies on, so the old glyph and text
    // have to be erased by hand before anything new is drawn over them.
    let clear_brush = unsafe { CreateSolidBrush(COLORREF(DARK_WINDOW_BG)) };
    if !clear_brush.is_invalid() {
        unsafe {
            FillRect(hdc, &raw const rect, clear_brush);
            let _ = DeleteObject(clear_brush.into());
        }
    }

    let htheme = unsafe { OpenThemeData(Some(hwnd), w!("BUTTON")) };
    if htheme.is_invalid() {
        return CDRF_DODEFAULT;
    }

    let checked =
        u32::try_from(unsafe { SendMessageW(hwnd, BM_GETCHECK, None, None) }.0).unwrap_or(0);
    let state_id = checkbox_state_id(checked == BST_CHECKED.0, cd.uItemState);

    let glyph_size =
        unsafe { GetThemePartSize(htheme, Some(hdc), BP_CHECKBOX.0, state_id.0, None, TS_DRAW) }
            .unwrap_or(SIZE { cx: 13, cy: 13 });

    let glyph_top = rect.top + (rect.bottom - rect.top - glyph_size.cy) / 2;
    let glyph_rect = RECT {
        left: rect.left,
        top: glyph_top,
        right: rect.left + glyph_size.cx,
        bottom: glyph_top + glyph_size.cy,
    };

    unsafe {
        let _ = DrawThemeBackground(
            htheme,
            hdc,
            BP_CHECKBOX.0,
            state_id.0,
            &raw const glyph_rect,
            None,
        );
        let _ = CloseThemeData(htheme);
    }

    let text = window_text(hwnd);
    if !text.is_empty() {
        let color = if cd.uItemState.contains(CDIS_DISABLED) {
            DARK_GRAY_TEXT
        } else {
            DARK_TEXT
        };
        let mut text_rect = RECT {
            left: glyph_rect.right + CHECKBOX_TEXT_GAP,
            top: rect.top,
            right: rect.right,
            bottom: rect.bottom,
        };
        let mut wide_text = wide_for_theme_draw(&text);
        unsafe {
            // The paint DC's starting font is whatever comctl32 happened to
            // leave selected for this custom-draw callback — not guaranteed
            // to be this control's own font — so it is selected explicitly,
            // matching the label font every other control in this window
            // uses, and restored once the text is drawn.
            let old_font = font_regular.map(|f| SelectObject(hdc, f.into()));
            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(color));
            DrawTextW(
                hdc,
                &mut wide_text,
                &raw mut text_rect,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
            );
            if let Some(old_font) = old_font {
                SelectObject(hdc, old_font);
            }
        }
    }

    CDRF_SKIPDEFAULT
}

/// Maps checked/disabled/pressed/hot into the theme state id for the
/// `BP_CHECKBOX` part. The dialog's five checkboxes are all
/// `BS_AUTOCHECKBOX` (never tri-state), so only the checked/unchecked
/// families are needed.
fn checkbox_state_id(checked: bool, item_state: NMCUSTOMDRAW_DRAW_STATE_FLAGS) -> CHECKBOXSTATES {
    let disabled = item_state.contains(CDIS_DISABLED);
    let pressed = item_state.contains(CDIS_SELECTED);
    let hot = item_state.contains(CDIS_HOT);
    match (checked, disabled, pressed, hot) {
        (true, true, ..) => CBS_CHECKEDDISABLED,
        (true, _, true, _) => CBS_CHECKEDPRESSED,
        (true, _, _, true) => CBS_CHECKEDHOT,
        (true, ..) => CBS_CHECKEDNORMAL,
        (false, true, ..) => CBS_UNCHECKEDDISABLED,
        (false, _, true, _) => CBS_UNCHECKEDPRESSED,
        (false, _, _, true) => CBS_UNCHECKEDHOT,
        (false, ..) => CBS_UNCHECKEDNORMAL,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Numeric Edit Border (WM_NCPAINT subclass)
// ─────────────────────────────────────────────────────────────────────────────

const EDIT_SUBCLASS_ID: usize = 1;

/// Installs the numeric edits' border subclass. Called once per edit at
/// control creation; safe to leave installed for the window's whole life —
/// the subclass proc itself checks the live dark flag on every
/// `WM_NCPAINT`, so it needs no reinstalling when the theme changes live.
pub(super) fn install_edit_border_subclass(hwnd: HWND) {
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(edit_border_subclass_proc), EDIT_SUBCLASS_ID, 0);
    }
}

unsafe extern "system" fn edit_border_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _refdata: usize,
) -> LRESULT {
    unsafe {
        match msg {
            WM_NCPAINT => {
                let mut dark = false;
                with_window_state(|state| dark = state.dark.get());
                if dark {
                    paint_edit_border(hwnd);
                    return LRESULT(0);
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_NCDESTROY => {
                let _ =
                    RemoveWindowSubclass(hwnd, Some(edit_border_subclass_proc), EDIT_SUBCLASS_ID);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

/// Hand-paints a 1px frame around `hwnd`'s whole window rect (client +
/// border) directly onto its non-client device context. Shared by the
/// numeric edits' `WM_NCPAINT` subclass below and the hotkey capture
/// control's own `WM_NCPAINT` handling (`capture.rs`) — both classes carry
/// plain `WS_BORDER`, which `DefWindowProcW`/`DefSubclassProc` paints in
/// `COLOR_WINDOWFRAME` (black in both themes and invisible on this
/// window's dark background), and neither is themed (see the module doc
/// comment for why the edits are not; the capture control has no
/// visual-style association to begin with, being a custom window class).
pub(super) fn paint_edit_border(hwnd: HWND) {
    let mut wrect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &raw mut wrect) }.is_err() {
        return;
    }
    let frame = RECT {
        left: 0,
        top: 0,
        right: wrect.right - wrect.left,
        bottom: wrect.bottom - wrect.top,
    };
    let hdc = unsafe { GetWindowDC(Some(hwnd)) };
    if hdc.is_invalid() {
        return;
    }
    let brush = unsafe { CreateSolidBrush(COLORREF(DARK_BORDER)) };
    if !brush.is_invalid() {
        unsafe {
            let _ = FrameRect(hdc, &raw const frame, brush);
            let _ = DeleteObject(brush.into());
        }
    }
    unsafe {
        ReleaseDC(Some(hwnd), hdc);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Combo Closed Face (WM_PAINT / WM_ERASEBKGND subclass)
// ─────────────────────────────────────────────────────────────────────────────

const COMBO_SUBCLASS_ID: usize = 2;

/// Horizontal inset, in device pixels, between the combo's client edge and
/// where its text is drawn — matches a native combo's own left margin.
const COMBO_TEXT_INSET: i32 = 4;

pub(super) fn install_combo_subclass(hwnd: HWND) {
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(combo_subclass_proc), COMBO_SUBCLASS_ID, 0);
    }
}

unsafe extern "system" fn combo_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _refdata: usize,
) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => {
                let mut dark = false;
                with_window_state(|state| dark = state.dark.get());
                if dark {
                    let hdc = HDC(std::ptr::with_exposed_provenance_mut(wparam.0));
                    fill_dark_control_bg(hdc, hwnd);
                    return LRESULT(1);
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_PAINT => {
                let mut dark = false;
                with_window_state(|state| dark = state.dark.get());
                if dark {
                    paint_combo(hwnd);
                    return LRESULT(0);
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(combo_subclass_proc), COMBO_SUBCLASS_ID);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

/// Fills `hwnd`'s whole client area with the dark control background —
/// shared by the combo and updown subclasses' `WM_ERASEBKGND`/`WM_PAINT`
/// handling.
fn fill_dark_control_bg(hdc: HDC, hwnd: HWND) {
    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &raw mut rect) }.is_err() {
        return;
    }
    let brush = unsafe { CreateSolidBrush(COLORREF(DARK_CONTROL_BG)) };
    if !brush.is_invalid() {
        unsafe {
            FillRect(hdc, &raw const rect, brush);
            let _ = DeleteObject(brush.into());
        }
    }
}

/// Paints the combo's closed face: background fill, the selected item's
/// text via `DrawThemeTextEx` with an explicit colour, and the dropdown
/// arrow via `DrawThemeBackground(CP_DROPDOWNBUTTONRIGHT)`. The
/// `"CFD::COMBOBOX"` compound theme-class string is a Microsoft-documented
/// convention for reaching a control's sub-application theme variant (the
/// same one `SetWindowTheme`'s `pszSubAppName` parameter targets) — it is
/// what actually resolves to the dark button/arrow glyph, independent of
/// [`apply_combo_theme`]'s plain `"CFD"` association (which is metrics-only).
fn paint_combo(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &raw mut ps) };
    if hdc.is_invalid() {
        return;
    }

    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &raw mut rect) }.is_ok() {
        let brush = unsafe { CreateSolidBrush(COLORREF(DARK_CONTROL_BG)) };
        if !brush.is_invalid() {
            unsafe {
                FillRect(hdc, &raw const rect, brush);
                let _ = DeleteObject(brush.into());
            }
        }

        // EnableWindow only gates input, never painting, but a disabled
        // combo still has to read as disabled: grayed text and arrow, not
        // full-contrast ones a user would read as still editable.
        let enabled = unsafe { IsWindowEnabled(hwnd) }.as_bool();
        let text_color = if enabled { DARK_TEXT } else { DARK_GRAY_TEXT };

        let mut cbi = COMBOBOXINFO {
            cbSize: u32::try_from(std::mem::size_of::<COMBOBOXINFO>()).unwrap_or(0),
            ..Default::default()
        };
        let button_rect = unsafe { GetComboBoxInfo(hwnd, &raw mut cbi) }
            .is_ok()
            .then_some(cbi.rcButton);

        let htheme = open_combo_arrow_theme(hwnd);
        paint_combo_arrow(hdc, htheme, button_rect, enabled, text_color);

        let text = window_text(hwnd);
        let text_right = button_rect.map_or(rect.right, |b| b.left);
        let text_rect = RECT {
            left: rect.left + COMBO_TEXT_INSET,
            top: rect.top,
            right: text_right - COMBO_TEXT_INSET,
            bottom: rect.bottom,
        };
        let mut font_regular = None;
        with_window_state(|state| font_regular = Some(state.font_regular.get()));
        paint_combo_text(
            hdc,
            htheme,
            enabled,
            text_rect,
            &text,
            text_color,
            font_regular,
        );

        if !htheme.is_invalid() {
            unsafe {
                let _ = CloseThemeData(htheme);
            }
        }
    }

    unsafe {
        let _ = EndPaint(hwnd, &raw const ps);
    }
}

/// Draws the combo's dropdown arrow via `htheme`
/// (`DrawThemeBackground(CP_DROPDOWNBUTTONRIGHT, …)`), or hand-draws it
/// (see [`draw_combo_arrow_fallback`]) if the theme is unavailable or the
/// draw call fails. Either failure is logged at `debug` — never a
/// user-facing error, just a trail so a blank-looking combo can be
/// diagnosed later.
fn paint_combo_arrow(
    hdc: HDC,
    htheme: HTHEME,
    button_rect: Option<RECT>,
    enabled: bool,
    fallback_color: u32,
) {
    let arrow_state = if enabled {
        CBXSR_NORMAL
    } else {
        CBXSR_DISABLED
    };
    let mut drawn = false;
    if htheme.is_invalid() {
        log::debug!("OpenThemeData failed for the combo arrow theme; hand-drawing it instead");
    } else if let Some(button_rect) = button_rect {
        let result = unsafe {
            DrawThemeBackground(
                htheme,
                hdc,
                CP_DROPDOWNBUTTONRIGHT.0,
                arrow_state.0,
                &raw const button_rect,
                None,
            )
        };
        if let Err(e) = result {
            log::debug!(error:% = e; "DrawThemeBackground failed for the combo arrow; hand-drawing it instead");
        } else {
            drawn = true;
        }
    }
    if !drawn && let Some(button_rect) = button_rect {
        draw_combo_arrow_fallback(hdc, button_rect, fallback_color);
    }
}

/// Draws the combo's selected-item text via `htheme` (`DrawThemeTextEx`
/// with an explicit `crText`), or hand-draws it (see
/// [`draw_combo_text_fallback`]) if the theme is unavailable or the draw
/// call fails. Same logging rationale as [`paint_combo_arrow`].
fn paint_combo_text(
    hdc: HDC,
    htheme: HTHEME,
    enabled: bool,
    text_rect: RECT,
    text: &str,
    color: u32,
    font: Option<HFONT>,
) {
    let item_state = if enabled { CBRO_NORMAL } else { CBRO_DISABLED };
    let mut drawn = false;
    if htheme.is_invalid() {
        log::debug!("OpenThemeData failed for the combo text theme; hand-drawing it instead");
    } else {
        let wide_text = wide_for_theme_draw(text);
        let opts = DTTOPTS {
            dwSize: u32::try_from(std::mem::size_of::<DTTOPTS>()).unwrap_or(0),
            dwFlags: DTT_TEXTCOLOR,
            crText: COLORREF(color),
            ..Default::default()
        };
        let mut rect = text_rect;
        unsafe {
            // BeginPaint's DC starts with the stock system font, not this
            // control's own — select it before drawing text, same as
            // every other control's paint path in this window.
            let old_font = font.map(|f| SelectObject(hdc, f.into()));
            let result = DrawThemeTextEx(
                htheme,
                hdc,
                CP_DROPDOWNITEM.0,
                item_state.0,
                &wide_text,
                DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
                &raw mut rect,
                Some(&raw const opts),
            );
            if let Some(old_font) = old_font {
                SelectObject(hdc, old_font);
            }
            if let Err(e) = result {
                log::debug!(error:% = e; "DrawThemeTextEx failed for the combo's text; hand-drawing it instead");
            } else {
                drawn = true;
            }
        }
    }
    if !drawn {
        draw_combo_text_fallback(hdc, text_rect, text, color, font);
    }
}

/// Opens the theme handle the combo's dark arrow glyph actually paints
/// from: `"DarkMode_CFD::COMBOBOX"`, the dark sub-application variant of
/// the same `"CFD"` combo-box parts [`apply_combo_theme`] associates with
/// the window for metrics only. An explicit `App::Class` list here
/// overrides whatever the window is associated with, so opening plain
/// `"CFD::COMBOBOX"` would hand back the light arrow glyph regardless of
/// dark mode — falls back to it only if the dark variant fails to open
/// (an older Windows release without it), rather than drawing nothing.
fn open_combo_arrow_theme(hwnd: HWND) -> HTHEME {
    let dark_theme = unsafe { OpenThemeData(Some(hwnd), w!("DarkMode_CFD::COMBOBOX")) };
    if !dark_theme.is_invalid() {
        return dark_theme;
    }
    unsafe { OpenThemeData(Some(hwnd), w!("CFD::COMBOBOX")) }
}

/// Hand-drawn downward arrow glyph for the combo's dropdown button, used
/// whenever the theme-drawn one (`DrawThemeBackground(CP_DROPDOWNBUTTONRIGHT,
/// …)`) is unavailable or fails — the same triangle shape
/// [`draw_spin_button`] draws for the updown spinner, so the combo never
/// renders with no visible affordance to open its list.
fn draw_combo_arrow_fallback(hdc: HDC, rect: RECT, color: u32) {
    let cx = i32::midpoint(rect.left, rect.right);
    let cy = i32::midpoint(rect.top, rect.bottom);
    let half = arrow_half_width(rect);
    let points = [
        POINT {
            x: cx - half,
            y: cy - half / 2,
        },
        POINT {
            x: cx + half,
            y: cy - half / 2,
        },
        POINT {
            x: cx,
            y: cy + half / 2,
        },
    ];
    let brush = unsafe { CreateSolidBrush(COLORREF(color)) };
    if !brush.is_invalid() {
        unsafe {
            let old_brush = SelectObject(hdc, brush.into());
            let old_pen = SelectObject(hdc, GetStockObject(NULL_PEN));
            let _ = Polygon(hdc, &points);
            SelectObject(hdc, old_brush);
            SelectObject(hdc, old_pen);
            let _ = DeleteObject(brush.into());
        }
    }
}

/// Plain `SetTextColor` + `DrawTextW` fallback for the combo's
/// selected-item text, used whenever `DrawThemeTextEx` is unavailable or
/// fails — same reasoning as [`draw_combo_arrow_fallback`]. A no-op for
/// empty text (nothing selected yet), matching `DrawThemeTextEx`'s own
/// behaviour on an empty string.
fn draw_combo_text_fallback(hdc: HDC, mut rect: RECT, text: &str, color: u32, font: Option<HFONT>) {
    if text.is_empty() {
        return;
    }
    let mut wide_text = wide_for_theme_draw(text);
    unsafe {
        let old_font = font.map(|f| SelectObject(hdc, f.into()));
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(color));
        DrawTextW(
            hdc,
            &mut wide_text,
            &raw mut rect,
            DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX,
        );
        if let Some(old_font) = old_font {
            SelectObject(hdc, old_font);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Updown Spinner Buttons (WM_PAINT / WM_ERASEBKGND subclass)
// ─────────────────────────────────────────────────────────────────────────────

const UPDOWN_SUBCLASS_ID: usize = 3;

pub(super) fn install_updown_subclass(hwnd: HWND) {
    unsafe {
        let _ = SetWindowSubclass(hwnd, Some(updown_subclass_proc), UPDOWN_SUBCLASS_ID, 0);
    }
}

unsafe extern "system" fn updown_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _refdata: usize,
) -> LRESULT {
    unsafe {
        match msg {
            WM_ERASEBKGND => {
                let mut dark = false;
                with_window_state(|state| dark = state.dark.get());
                if dark {
                    // WM_PAINT repaints the whole client area every time, so
                    // erasing here would only flash; just claim it handled.
                    return LRESULT(1);
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_PAINT => {
                let mut dark = false;
                with_window_state(|state| dark = state.dark.get());
                if dark {
                    paint_updown(hwnd);
                    return LRESULT(0);
                }
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(hwnd, Some(updown_subclass_proc), UPDOWN_SUBCLASS_ID);
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

fn paint_updown(hwnd: HWND) {
    let mut ps = PAINTSTRUCT::default();
    let hdc = unsafe { BeginPaint(hwnd, &raw mut ps) };
    if hdc.is_invalid() {
        return;
    }

    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &raw mut rect) }.is_ok() {
        let brush = unsafe { CreateSolidBrush(COLORREF(DARK_CONTROL_BG)) };
        if !brush.is_invalid() {
            unsafe {
                FillRect(hdc, &raw const rect, brush);
                let _ = DeleteObject(brush.into());
            }
        }

        let mid = i32::midpoint(rect.top, rect.bottom);
        let up_rect = RECT {
            left: rect.left,
            top: rect.top,
            right: rect.right,
            bottom: mid,
        };
        let down_rect = RECT {
            left: rect.left,
            top: mid,
            right: rect.right,
            bottom: rect.bottom,
        };
        draw_spin_button(hdc, up_rect, true);
        draw_spin_button(hdc, down_rect, false);
    }

    unsafe {
        let _ = EndPaint(hwnd, &raw const ps);
    }
}

/// Paints one half of the updown control: a rounded-rect face outlined in
/// [`DARK_BORDER`], then a filled triangle glyph pointing up or down.
fn draw_spin_button(hdc: HDC, rect: RECT, points_up: bool) {
    let pen = unsafe { CreatePen(PS_SOLID, 1, COLORREF(DARK_BORDER)) };
    let face_brush = unsafe { CreateSolidBrush(COLORREF(DARK_CONTROL_BG)) };
    if !pen.is_invalid() && !face_brush.is_invalid() {
        unsafe {
            let old_pen = SelectObject(hdc, pen.into());
            let old_brush = SelectObject(hdc, face_brush.into());
            let _ = RoundRect(hdc, rect.left, rect.top, rect.right, rect.bottom, 3, 3);
            SelectObject(hdc, old_pen);
            SelectObject(hdc, old_brush);
        }
    }
    unsafe {
        if !pen.is_invalid() {
            let _ = DeleteObject(pen.into());
        }
        if !face_brush.is_invalid() {
            let _ = DeleteObject(face_brush.into());
        }
    }

    let cx = i32::midpoint(rect.left, rect.right);
    let cy = i32::midpoint(rect.top, rect.bottom);
    let half = arrow_half_width(rect);
    let points = if points_up {
        [
            POINT {
                x: cx - half,
                y: cy + half / 2,
            },
            POINT {
                x: cx + half,
                y: cy + half / 2,
            },
            POINT {
                x: cx,
                y: cy - half / 2,
            },
        ]
    } else {
        [
            POINT {
                x: cx - half,
                y: cy - half / 2,
            },
            POINT {
                x: cx + half,
                y: cy - half / 2,
            },
            POINT {
                x: cx,
                y: cy + half / 2,
            },
        ]
    };

    let glyph_brush = unsafe { CreateSolidBrush(COLORREF(DARK_TEXT)) };
    if !glyph_brush.is_invalid() {
        unsafe {
            let old_brush = SelectObject(hdc, glyph_brush.into());
            let null_pen = GetStockObject(NULL_PEN);
            let old_pen = SelectObject(hdc, null_pen);
            let _ = Polygon(hdc, &points);
            SelectObject(hdc, old_brush);
            SelectObject(hdc, old_pen);
            let _ = DeleteObject(glyph_brush.into());
        }
    }
}

/// Half-width of the arrow glyph triangle, scaled to whichever half of the
/// updown control (up or down) is smaller — keeps the glyph proportionate
/// across DPI without needing its own DPI plumbing, since the button rect
/// it is drawn into is already DPI-scaled by `layout()`.
#[must_use]
fn arrow_half_width(rect: RECT) -> i32 {
    let smaller = (rect.right - rect.left).min(rect.bottom - rect.top);
    (smaller / 4).max(2)
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// UTF-16 encoding for `DrawTextW`/`DrawThemeTextEx`, deliberately without a
/// NUL terminator — both take an explicit slice length, and drawing past the
/// real text would render a trailing NUL glyph.
fn wide_for_theme_draw(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── static_color ─────────────────────────────────────────────────────

    #[test]
    fn error_line_is_fixed_in_both_themes() {
        assert_eq!(
            static_color(ID_HK_ERROR, true),
            Some(StaticColor::Fixed(DARK_ERROR_TEXT))
        );
        assert_eq!(
            static_color(ID_HK_ERROR, false),
            Some(StaticColor::Fixed(LIGHT_ERROR_TEXT))
        );
    }

    #[test]
    fn hint_lines_are_gray_in_dark_and_sys_gray_in_light() {
        for hint_id in [ID_HK_HINT, ID_LOG_HINT] {
            assert_eq!(
                static_color(hint_id, true),
                Some(StaticColor::Fixed(DARK_GRAY_TEXT))
            );
            assert_eq!(static_color(hint_id, false), Some(StaticColor::SysGray));
        }
    }

    #[test]
    fn a_plain_static_is_untouched_in_light_mode() {
        assert_eq!(static_color(999, false), None);
    }

    #[test]
    fn a_plain_static_gets_the_dark_text_colour() {
        assert_eq!(static_color(999, true), Some(StaticColor::Fixed(DARK_TEXT)));
    }

    #[test]
    fn a_disabled_numeric_edit_gets_the_control_background_when_dark() {
        for id in [
            ID_STEP_EDIT,
            ID_OSD_TIMEOUT_EDIT,
            ID_OSD_OPACITY_EDIT,
            ID_RESYNC_EDIT,
            ID_INACT_EDIT,
        ] {
            assert_eq!(
                static_color(id, true),
                Some(StaticColor::FixedOnControl(DARK_GRAY_TEXT)),
                "id {id} should read on the control background"
            );
            // In light mode a disabled edit is left to DefWindowProcW, same
            // as any other plain static.
            assert_eq!(static_color(id, false), None);
        }
    }

    // ── is_checkbox_id / is_numeric_edit_id ─────────────────────────────

    #[test]
    fn exactly_the_five_checkboxes_are_recognized() {
        for id in [
            ID_AUTOSTART,
            ID_INTERCEPT,
            ID_RESYNC_CHECK,
            ID_INACT_CHECK,
            ID_LOG_CHECK,
        ] {
            assert!(is_checkbox_id(id), "id {id} should be a checkbox");
        }
        assert!(!is_checkbox_id(ID_HK_ERROR));
        assert!(!is_checkbox_id(ID_STEP_EDIT));
    }

    // ── is_immersive_color_set_change ───────────────────────────────────

    #[test]
    fn a_null_lparam_is_not_a_theme_change() {
        assert!(!is_immersive_color_set_change(LPARAM(0)));
    }

    // ── arrow_half_width ─────────────────────────────────────────────────

    #[test]
    fn arrow_half_width_scales_with_the_smaller_dimension() {
        let small = RECT {
            left: 0,
            top: 0,
            right: 16,
            bottom: 11,
        };
        assert_eq!(arrow_half_width(small), 2); // 11/4 = 2 (floor), clamped floor is 2
        let large = RECT {
            left: 0,
            top: 0,
            right: 40,
            bottom: 40,
        };
        assert_eq!(arrow_half_width(large), 10);
    }

    #[test]
    fn arrow_half_width_never_goes_below_two() {
        let tiny = RECT {
            left: 0,
            top: 0,
            right: 4,
            bottom: 4,
        };
        assert_eq!(arrow_half_width(tiny), 2);
    }

    // ── checkbox_state_id ────────────────────────────────────────────────

    #[test]
    fn checkbox_state_id_picks_the_matching_family() {
        let none = NMCUSTOMDRAW_DRAW_STATE_FLAGS(0);
        assert_eq!(checkbox_state_id(false, none), CBS_UNCHECKEDNORMAL);
        assert_eq!(checkbox_state_id(true, none), CBS_CHECKEDNORMAL);
        assert_eq!(
            checkbox_state_id(false, CDIS_DISABLED),
            CBS_UNCHECKEDDISABLED
        );
        assert_eq!(checkbox_state_id(true, CDIS_DISABLED), CBS_CHECKEDDISABLED);
        assert_eq!(checkbox_state_id(false, CDIS_HOT), CBS_UNCHECKEDHOT);
        assert_eq!(checkbox_state_id(true, CDIS_SELECTED), CBS_CHECKEDPRESSED);
        // Disabled wins over hot/pressed if somehow both are set.
        assert_eq!(
            checkbox_state_id(true, CDIS_DISABLED | CDIS_HOT),
            CBS_CHECKEDDISABLED
        );
    }
}
