//! Hotkey capture control: a self-contained custom-draw window class
//! (`"DarkBrightHotkeyCapture"`, registered by `window`) that lets a user
//! click a hotkey field and press a key combination to rebind it. Owns its
//! own per-instance state (in `GWLP_USERDATA`, not a thread-local — see
//! [`CaptureState`]) and wndproc; talks to the settings window only through
//! `window`'s [`ID_HK_UP`]/[`ID_HK_DOWN`]/`ID_HK_ERROR` controls and the
//! shared `WindowState`.

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, COLOR_GRAYTEXT, COLOR_WINDOWTEXT, CreateSolidBrush, DT_END_ELLIPSIS, DT_LEFT,
    DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawFocusRect, DrawTextW, EndPaint,
    FillRect, GetSysColor, HDC, InvalidateRect, PAINTSTRUCT, SelectObject, SetBkMode, SetTextColor,
    TRANSPARENT,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetFocus, GetKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, SetFocus,
    VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_LCONTROL, VK_LMENU, VK_LSHIFT, VK_LWIN, VK_MENU,
    VK_RCONTROL, VK_RETURN, VK_RMENU, VK_RSHIFT, VK_RWIN, VK_SHIFT, VK_SPACE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    DLGC_BUTTON, DLGC_WANTALLKEYS, DLGC_WANTMESSAGE, DefWindowProcW, GWLP_USERDATA, GetClientRect,
    GetDlgCtrlID, GetWindowLongPtrW, MSG, SetWindowLongPtrW, SetWindowTextW, WM_CHAR, WM_CREATE,
    WM_ERASEBKGND, WM_GETDLGCODE, WM_KEYDOWN, WM_KEYUP, WM_KILLFOCUS, WM_LBUTTONDOWN, WM_NCDESTROY,
    WM_NCPAINT, WM_PAINT, WM_SETFOCUS, WM_SETTEXT, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSKEYUP,
};
use windows::core::PCWSTR;

use crate::core::state::{BrightnessMessage, SettingChange};

use super::super::hotkey::{bindings_conflict, hotkey_string};
use super::dark;
use super::layout::{ID_HK_DOWN, ID_HK_ERROR, ID_HK_UP};
use super::window::{
    get_text, post_change, send_message, set_text, wide, window_text, with_window_state,
};

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
//   capturing --modifier keydown, or matching keyup--> capturing (live
//       preview only, no post; the keyup half keeps the preview from going
//       stale after a modifier is released mid-capture)
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
const REJECT_NO_MODIFIER: &str = "Add Ctrl, Alt, or Win (Shift alone isn't enough)";
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
///
/// Soundness precondition (this function has a safe signature but is not
/// sound for an arbitrary `hwnd`): `hwnd` must be a window of the
/// `"DarkBrightHotkeyCapture"` class. `GWLP_USERDATA` is untyped storage —
/// this reinterprets whatever is there as `*mut CaptureState` with no tag
/// to check, so an `hwnd` of any other class would read garbage through
/// it. The precondition holds by construction: every caller in this module
/// is reached from inside [`capture_wnd_proc`], which Windows only ever
/// invokes for windows of this class; it would not hold for a call from
/// anywhere else.
fn capture_state_ptr(hwnd: HWND) -> *mut CaptureState {
    let raw = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) };
    std::ptr::with_exposed_provenance_mut(raw.cast_unsigned())
}

/// A copy of `hwnd`'s current [`CaptureState`], or the default (idle, no
/// modifiers) if the pointer isn't set yet. Same `hwnd`-must-be-this-class
/// precondition as [`capture_state_ptr`].
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
/// before `WM_CREATE` / after `WM_NCDESTROY`. Same `hwnd`-must-be-this-class
/// precondition as [`capture_state_ptr`].
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
#[must_use]
fn vk_from_wparam(wparam: WPARAM) -> VIRTUAL_KEY {
    VIRTUAL_KEY(u16::try_from(wparam.0).unwrap_or(0))
}

/// Clears the shared `ID_HK_ERROR` status line. Any message standing there
/// is stale the instant a fresh capture starts or a capture completes: a
/// rejection message describes the capture attempt that just failed, so it
/// is transient by nature — a permanent message with no clear on any path
/// would leave a rejection sitting under a since-accepted binding, or a
/// controller-originated registration error sitting under a later
/// successful rebind, with no way for the user to tell whether the just-
/// shown value actually took.
fn clear_capture_error() {
    with_window_state(|state| set_text(state.hwnd, ID_HK_ERROR, ""));
}

/// Enters capture: focuses the control (harmless if it already has focus),
/// marks it capturing with no modifiers held yet, clears any stale status
/// message (including one left over from a previous controller-originated
/// registration failure — starting a new capture is exactly when the user
/// is trying to fix it), repaints to the base prompt, and tells the
/// controller to suspend hotkey interception. `SetFocus` runs first, before
/// this window's own state changes, so if it moves focus away from a
/// sibling capture field that was itself mid capture, that field's
/// `WM_KILLFOCUS` cancels *its* capture (and resumes interception) before
/// this one suspends it again — no window is ever left capturing without
/// interception suspended.
fn start_capture(hwnd: HWND) {
    unsafe {
        let _ = SetFocus(Some(hwnd));
    }
    with_capture_state_mut(hwnd, |cs| {
        cs.capturing = true;
        cs.live_modifiers = HOT_KEY_MODIFIERS(0);
    });
    clear_capture_error();
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
/// which repaints itself), clears any stale rejection/error text — the
/// binding it described no longer applies, it has just been replaced — and
/// posts the `SettingChange` the caller built. Deliberately *not*
/// `HotkeyCaptureEnded`, since the controller treats this rebind as its own
/// resume (see the state-machine comment at the top of this section).
fn accept_capture(hwnd: HWND, candidate: String, change: fn(String) -> SettingChange) {
    with_capture_state_mut(hwnd, |cs| {
        cs.capturing = false;
        cs.live_modifiers = HOT_KEY_MODIFIERS(0);
    });
    set_self_text(hwnd, &candidate);
    clear_capture_error();
    with_window_state(|state| post_change(state, change(candidate)));
}

/// Rejects a candidate without leaving capture: shows `message` on the
/// shared `ID_HK_ERROR` status line so the user can just try again. Takes
/// no `hwnd` — the capture field itself doesn't change, only the shared
/// status line reached via `WindowState::hwnd`.
fn reject_capture(message: &str) {
    with_window_state(|state| set_text(state.hwnd, ID_HK_ERROR, message));
}

/// Re-reads the live modifier mask and repaints — the shared half of
/// updating the capture preview, used on both a modifier keydown (a
/// modifier is now held that wasn't) and a modifier keyup (one that was
/// held no longer is). Without the keyup half the displayed preview goes
/// stale the moment a user releases a modifier: hold Ctrl, release it,
/// press `B` — the field would still show `"Ctrl+"` while the freshly
/// re-read mask (used for the actual accept/reject decision, so
/// correctness was never at risk) rejects the candidate as having no
/// modifier, and the displayed reason and the shown preview would disagree.
fn refresh_modifier_preview(hwnd: HWND) {
    let modifiers = live_modifier_flags();
    with_capture_state_mut(hwnd, |cs| cs.live_modifiers = modifiers);
    invalidate(hwnd);
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
        refresh_modifier_preview(hwnd);
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

/// `WM_KEYUP`/`WM_SYSKEYUP` while `hwnd` is capturing: only a modifier keyup
/// does anything (refreshes the stale-preview problem [`refresh_modifier_preview`]
/// documents). The message is swallowed either way — see
/// `capture_wnd_proc`'s `WM_KEYUP | WM_SYSKEYUP` arm for why letting it fall
/// through to `DefWindowProcW` mid-capture is unsafe, not just untidy.
fn handle_capture_keyup(hwnd: HWND, vk: VIRTUAL_KEY) {
    if is_modifier_vk(vk) {
        refresh_modifier_preview(hwnd);
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
#[must_use]
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
/// signals which field is active. Reads `hwnd`'s `CaptureState` via
/// [`capture_state`], so it carries that function's same `hwnd`-must-be-the-
/// capture-class precondition — satisfied here because this is only ever
/// called from `capture_wnd_proc`'s own `WM_PAINT` arm.
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
        let old_font = SelectObject(hdc, state.font_regular.get().into());
        SetBkMode(hdc, TRANSPARENT);
        let color = if state.dark.get() {
            if is_placeholder {
                dark::DARK_GRAY_TEXT
            } else {
                dark::DARK_TEXT
            }
        } else {
            let color_index = if is_placeholder {
                COLOR_GRAYTEXT
            } else {
                COLOR_WINDOWTEXT
            };
            GetSysColor(color_index)
        };
        SetTextColor(hdc, COLORREF(color));

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

/// `WM_ERASEBKGND`: whether this control just painted its own dark
/// background (`true`), or left it to the caller's `DefWindowProcW`
/// fallback (`false`). The class background brush (`COLOR_WINDOW`, set at
/// registration) is only ever right in light mode; dark mode paints its
/// own fill here instead, matching every other custom-drawn control in
/// this window.
fn handle_capture_erasebkgnd(hwnd: HWND, wparam: WPARAM) -> bool {
    let mut dark = false;
    with_window_state(|state| dark = state.dark.get());
    if !dark {
        return false;
    }
    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &raw mut rect) }.is_ok() {
        let hdc = HDC(std::ptr::with_exposed_provenance_mut(wparam.0));
        let brush = unsafe { CreateSolidBrush(COLORREF(dark::DARK_CONTROL_BG)) };
        if !brush.is_invalid() {
            unsafe {
                FillRect(hdc, &raw const rect, brush);
                let _ = DeleteObject(brush.into());
            }
        }
    }
    true
}

/// `WM_NCPAINT`: whether this control just hand-painted its own border
/// (`true`), or left it to the caller's `DefWindowProcW` fallback
/// (`false`). This class carries plain `WS_BORDER`, same as the five
/// numeric edits; `DefWindowProcW`'s own frame paints in
/// `COLOR_WINDOWFRAME` (black), invisible against this window's dark
/// background, so dark mode hand-paints it the same way the edits' own
/// `WM_NCPAINT` subclass does.
fn handle_capture_ncpaint(hwnd: HWND) -> bool {
    let mut dark = false;
    with_window_state(|state| dark = state.dark.get());
    if dark {
        dark::paint_edit_border(hwnd);
    }
    dark
}

/// Window procedure for the hotkey capture control class.
///
/// # Safety
///
/// This is a Windows callback. The caller (Windows) ensures `hwnd` is valid.
pub(super) unsafe extern "system" fn capture_wnd_proc(
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
            WM_ERASEBKGND => {
                if handle_capture_erasebkgnd(hwnd, wparam) {
                    LRESULT(1)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_NCPAINT => {
                if handle_capture_ncpaint(hwnd) {
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
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
            // Swallowed only while capturing. `IsDialogMessageW` still
            // translates a keydown it passed through into these follow-up
            // messages, so leaving them to `DefWindowProcW` mid-capture is
            // not just untidy but actively wrong: a `WM_SYSCHAR` beeps on a
            // non-mnemonic, and `WM_SYSKEYUP` on `VK_MENU` is what raises
            // `SC_KEYMENU` — releasing Alt mid-capture would otherwise pop
            // the window's system menu, steal focus, and thereby fire
            // `WM_KILLFOCUS`, silently canceling the capture the user was
            // still in the middle of. While idle these fall through to
            // `DefWindowProcW` exactly as before.
            WM_KEYUP | WM_SYSKEYUP => {
                if capture_state(hwnd).capturing {
                    handle_capture_keyup(hwnd, vk_from_wparam(wparam));
                    LRESULT(0)
                } else {
                    DefWindowProcW(hwnd, msg, wparam, lparam)
                }
            }
            WM_CHAR | WM_SYSCHAR => {
                if capture_state(hwnd).capturing {
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

#[cfg(test)]
mod tests {
    use super::*;
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_UP;

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
