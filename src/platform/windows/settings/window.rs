//! Settings window creation, control wiring and the message loop. The
//! declarative layout data (control ids, styles, geometry) lives in
//! [`super::layout`]; the hotkey capture control is [`super::capture`].

use std::cell::{Cell, RefCell};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, OnceLock};

use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CLIP_DEFAULT_PRECIS, COLOR_BTNFACE, COLOR_WINDOW, CreateFontIndirectW, DEFAULT_CHARSET,
    DEFAULT_QUALITY, DeleteObject, FF_SWISS, FONT_WEIGHT, FW_BOLD, FW_NORMAL, GetSysColorBrush,
    HFONT, LOGFONTW, OUT_DEFAULT_PRECIS, VARIABLE_PITCH,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{
    BST_CHECKED, BST_UNCHECKED, ICC_LINK_CLASS, ICC_STANDARD_CLASSES, ICC_UPDOWN_CLASS,
    INITCOMMONCONTROLSEX, InitCommonControlsEx, LIF_ITEMINDEX, LIF_STATE, LIS_FOCUSED,
    LIST_ITEM_STATE_FLAGS, LITEM, LM_SETITEM, NM_CLICK, NM_CUSTOMDRAW, NMCUSTOMDRAW, NMHDR, NMLINK,
    NMUPDOWN, UDN_DELTAPOS, UPDOWN_CLASS, WC_BUTTON, WC_COMBOBOX, WC_EDIT, WC_LINK, WC_STATIC,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetFocus, GetKeyState, IsWindowEnabled, SetFocus, VK_RETURN, VK_SHIFT, VK_TAB,
};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    BM_GETCHECK, BM_SETCHECK, BN_CLICKED, CB_ADDSTRING, CB_GETCURSEL, CB_SETCURSEL, CBN_SELCHANGE,
    CS_HREDRAW, CS_VREDRAW, CreateWindowExW, DC_HASDEFID, DLGC_HASSETSEL, DLGC_WANTCHARS,
    DLGC_WANTMESSAGE, DLGC_WANTTAB, DM_GETDEFID, DefWindowProcW, DestroyWindow, DispatchMessageW,
    EN_KILLFOCUS, GetDlgItem, GetMessageW, GetNextDlgTabItem, GetWindowTextLengthW, GetWindowTextW,
    HMENU, HWND_TOPMOST, IDC_ARROW, IDOK, IsChild, IsDialogMessageW, LoadCursorW, MB_ICONERROR,
    MB_ICONWARNING, MB_OK, MB_OKCANCEL, MSG, MessageBoxW, PM_REMOVE, PeekMessageW, PostMessageW,
    PostQuitMessage, RegisterClassExW, SW_SHOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_NOZORDER, SendMessageW, SetForegroundWindow, SetWindowPos, SetWindowTextW, ShowWindow,
    TranslateMessage, WA_INACTIVE, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE, WM_APP, WM_CLOSE,
    WM_COMMAND, WM_CTLCOLORBTN, WM_CTLCOLOREDIT, WM_CTLCOLORLISTBOX, WM_CTLCOLORSTATIC, WM_DESTROY,
    WM_DPICHANGED, WM_GETDLGCODE, WM_KEYDOWN, WM_NCDESTROY, WM_NOTIFY, WM_SETFOCUS, WM_SETFONT,
    WM_SETTINGCHANGE, WNDCLASSEXW, WS_CAPTION, WS_CHILD, WS_EX_TOPMOST, WS_SYSMENU, WS_VISIBLE,
};
use windows::core::{PCWSTR, w};

use crate::core::config::{DEFAULT_REFRESH_INACTIVITY_SECONDS, DEFAULT_REFRESH_PERIODIC_SECONDS};
use crate::core::controller::SettingsSink;
use crate::core::state::{BrightnessMessage, SettingChange, SettingsSnapshot};
use crate::core::version::version_string;
use crate::error::{BrightnessError, Result};

use super::super::{autostart, hwnd_from_isize, hwnd_to_isize, last_error_as_brightness_error};
use super::capture::capture_wnd_proc;
use super::dark;
use super::layout::{
    CONTROLS, ID_AUTOSTART, ID_CLOSE, ID_HK_DOWN, ID_HK_ERROR, ID_HK_UP, ID_INACT_CHECK,
    ID_INACT_EDIT, ID_INACT_UPDOWN, ID_INTERCEPT, ID_LINK_CONFIG, ID_LOG_CHECK, ID_LOG_LEVEL,
    ID_OSD_OPACITY_EDIT, ID_OSD_TIMEOUT_EDIT, ID_RESTORE, ID_RESYNC_CHECK, ID_RESYNC_EDIT,
    ID_RESYNC_UPDOWN, ID_STEP_EDIT, ID_VERSION, RANGE_SPECS, RangeSpec, compute_placement,
    configure_combo_height, configure_updowns, dpi_from_wparam, font_height_for_dpi,
    is_section_header, layout,
};

// ─────────────────────────────────────────────────────────────────────────────
// Posted Messages
// ─────────────────────────────────────────────────────────────────────────────

/// Re-populate every control from a fresh `Box<SettingsSnapshot>` carried in
/// `lparam`. See [`SettingsSinkImpl::post_boxed`] for the ownership contract.
pub(super) const WM_APP_SETTINGS_REFRESH: u32 = WM_APP + 1;
/// Bring the window to the foreground; no payload.
pub(super) const WM_APP_SETTINGS_FOCUS: u32 = WM_APP + 2;
/// Show a hotkey registration error, a `Box<String>` carried in `lparam`.
/// See [`SettingsSinkImpl::post_boxed`] for the ownership contract.
pub(super) const WM_APP_SETTINGS_HK_ERROR: u32 = WM_APP + 3;
/// Show a non-error hotkey notice, a `Box<String>` carried in `lparam`, same
/// ownership contract as [`WM_APP_SETTINGS_HK_ERROR`].
pub(super) const WM_APP_SETTINGS_HK_NOTICE: u32 = WM_APP + 4;
/// Re-assert `HWND_TOPMOST`; no payload.
pub(super) const WM_APP_SETTINGS_TOPMOST: u32 = WM_APP + 5;

// These five must stay one contiguous, ordered range with REFRESH lowest and
// TOPMOST highest: `drain_pending_payload_messages` reclaims the leaked
// `Box` payloads by filtering `PeekMessageW` on exactly that span, so a
// constant added outside it would never be drained and its payload would
// leak. Adding a sixth message means extending the range here, deliberately.
const _: () = {
    assert!(WM_APP_SETTINGS_REFRESH < WM_APP_SETTINGS_FOCUS);
    assert!(WM_APP_SETTINGS_FOCUS < WM_APP_SETTINGS_HK_ERROR);
    assert!(WM_APP_SETTINGS_HK_ERROR < WM_APP_SETTINGS_HK_NOTICE);
    assert!(WM_APP_SETTINGS_HK_NOTICE < WM_APP_SETTINGS_TOPMOST);
    assert!(WM_APP_SETTINGS_TOPMOST - WM_APP_SETTINGS_REFRESH == 4);
};

// ─────────────────────────────────────────────────────────────────────────────
// Window Class Registration
// ─────────────────────────────────────────────────────────────────────────────

/// Window class of the settings window itself. Registered by this module
/// rather than taken from a dialog resource — the window is a plain
/// top-level, not a real dialog.
const SETTINGS_CLASS_NAME: PCWSTR = w!("DarkBrightSettings");

/// Ensures the settings window class is registered exactly once, and
/// remembers the outcome so a second attempt reports the original failure
/// rather than retrying.
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

/// Window class of the two hotkey capture controls. A custom class rather
/// than an `EDIT`, because the control has to swallow every key it is given
/// instead of typing it.
const CAPTURE_CLASS_NAME: PCWSTR = w!("DarkBrightHotkeyCapture");

/// Same register-once-and-remember role as [`REGISTER_CLASS_ONCE`], for the
/// capture control's class.
static REGISTER_CAPTURE_CLASS_ONCE: OnceLock<Result<()>> = OnceLock::new();

/// Registers the hotkey capture control's window class exactly once.
/// `CS_HREDRAW | CS_VREDRAW` matches every other custom-draw window in this
/// crate ([`osd.rs`](super::super::osd)): the control's whole client area is text,
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

/// Log level dropdown entries, in the order `CB_ADDSTRING` inserts them —
/// index into this array is the combo selection index.
const LOG_LEVELS: [&str; 5] = ["error", "warn", "info", "debug", "trace"];

/// UTF-16, NUL-terminated encoding of `s` for a `PCWSTR` argument that only
/// needs to live for the duration of one FFI call.
pub(super) fn wide(s: &str) -> Vec<u16> {
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

        // Every control's caption is a constant in the layout table except
        // the version line, whose text only exists once the build has run.
        let text = if spec.id == ID_VERSION {
            wide(&format!("v{}", version_string()))
        } else {
            wide(spec.text)
        };
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
                // SAFETY: `CB_ADDSTRING` copies the NUL-terminated string
                // `lparam` points at into the combo's own storage. The message
                // is sent, not posted, so `level_wide` still owns that buffer
                // while the copy happens.
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

        // Dark-mode paint subclasses, installed once here regardless of the
        // window's current theme — each subclass proc reads the live dark
        // flag itself on every paint, so nothing needs reinstalling when
        // WM_SETTINGCHANGE flips it later. See dark.rs's module doc comment
        // for why these three classes are hand-painted instead of themed.
        match spec.class {
            "EDIT" => dark::install_edit_border_subclass(child),
            "COMBOBOX" => dark::install_combo_subclass(child),
            "msctls_updown32" => dark::install_updown_subclass(child),
            _ => {}
        }

        // Keyboard activation for the footer link's two embedded links —
        // see "Footer Link Keyboard Activation" below for why this needs
        // its own subclass rather than relying on the control's built-in
        // handling.
        if spec.id == ID_LINK_CONFIG {
            install_footer_link_subclass(child);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Populating Controls
// ─────────────────────────────────────────────────────────────────────────────

/// Sets a child control's window text. No-op if `id` was never created.
pub(super) fn set_text(hwnd: HWND, id: u16, text: &str) {
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
    // one left over from before the refresh. Only overwrite when the
    // snapshot actually carries a nonzero value: a disabled field's
    // snapshot value is 0, and it must not clobber the value the user had
    // remembered from before it was unchecked.
    if periodic_checked {
        state.last_periodic.set(periodic_remembered);
    }
    if inactivity_checked {
        state.last_inactivity.set(inactivity_remembered);
    }

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
// Window State & Message Loop
// ─────────────────────────────────────────────────────────────────────────────

/// Per-window state, set once creation finishes and cleared on `WM_DESTROY`.
/// Lives in a thread-local because the window's own dedicated thread is the
/// only thread that ever touches it, and a plain `extern "system"` wndproc
/// has no other way to reach it — the same pattern `tray.rs`/`osd.rs` use
/// for their thread-local render/sender state. Every field except the twelve
/// `Cell`s is written once (at construction) and only ever read afterward,
/// which is what lets every reader below use `RefCell::borrow` (any number
/// of these can be held at once) instead of `borrow_mut` (which panics if a
/// second one is attempted while the first is still live); the `Cell`
/// fields are mutated through that same shared borrow via their own
/// interior mutability, so they don't need one either — see
/// [`with_window_state`] for the invariant that keeps this sound.
pub(super) struct WindowState {
    pub(super) hwnd: HWND,
    sender: Sender<BrightnessMessage>,
    /// The slot [`SettingsSinkImpl`] posts through, shared with the
    /// controller's thread. Held here so `WM_DESTROY` can clear it back to
    /// `0` and let Settings be reopened — and clear it only if it still
    /// holds *this* window, which is why the window needs the slot rather
    /// than just its own handle.
    hwnd_slot: Arc<AtomicIsize>,
    /// The current-DPI regular-weight font, read live by every paint path
    /// that draws text with it (the hotkey capture fields, the combo's
    /// custom paint, `dark.rs`'s checkbox custom-draw) — a `Cell` so
    /// [`handle_dpichanged`] can swap it in through the same shared borrow
    /// every other reader here uses, no `borrow_mut` required.
    pub(super) font_regular: Cell<HFONT>,
    /// Same rationale as [`WindowState::font_regular`], for the four
    /// section-header labels' bold font.
    font_bold: Cell<HFONT>,
    /// The DPI every control is currently laid out and sized for — the
    /// window's creation DPI until the first `WM_DPICHANGED`, updated by
    /// [`handle_dpichanged`] on every one after that.
    dpi: Cell<u32>,
    /// Whether the window is currently painting dark — recomputed on
    /// `WM_SETTINGCHANGE`'s `"ImmersiveColorSet"` broadcast (see
    /// [`dark::apply_theme`]) and read by every custom-draw path and
    /// `WM_CTLCOLOR*` handler this window has.
    pub(super) dark: Cell<bool>,
    /// The dark-mode background brushes, created once at window creation and
    /// freed on `WM_DESTROY` — see [`dark::Palette`].
    pub(super) palette: dark::Palette,
    /// The child control that held keyboard focus the last time this window
    /// was deactivated, as an `isize` handle (same crossing-the-callback
    /// convention as `hwnd_slot`) — `0` for "none recorded yet". This window
    /// is a plain top-level, not a real dialog, so nothing else remembers
    /// which control the keyboard was in once activation moves away and
    /// back; see `restore_focus` for how it is used and
    /// `focusable_child` for why a stale or now-disabled handle here is
    /// never trusted blindly.
    last_focus: Cell<isize>,
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
    /// The open window's [`WindowState`], or `None` before creation finishes
    /// and after `WM_DESTROY`. Reached through [`with_window_state`], never
    /// borrowed directly.
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
pub(super) fn with_window_state(f: impl FnOnce(&WindowState)) {
    WINDOW_STATE.with(|s| {
        if let Some(state) = s.borrow().as_ref() {
            f(state);
        }
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Focus Bookkeeping (WM_ACTIVATE / WM_SETFOCUS)
// ─────────────────────────────────────────────────────────────────────────────
// A plain CreateWindowExW top-level gets none of a dialog manager's focus
// bookkeeping, so the functions below do it by hand: save the focused child
// on deactivate, restore it (or fall back to the first tab stop) on
// reactivate, and self-heal on WM_SETFOCUS. See docs/architecture.md §14,
// "Focus save/restore", for why focus would otherwise get stuck.

/// Whether `hwnd` is a live, focusable child of `state.hwnd` — still a real
/// window, still a descendant, and not disabled. A handle saved earlier in
/// the session can fail any of the three (the control was destroyed, or has
/// since been disabled by a checkbox toggle), in which case restoring focus
/// to it would either no-op or land on the wrong control.
fn focusable_child(state: &WindowState, hwnd: HWND) -> bool {
    !hwnd.is_invalid()
        && unsafe { IsChild(state.hwnd, hwnd) }.as_bool()
        && unsafe { IsWindowEnabled(hwnd) }.as_bool()
}

/// Restores keyboard focus to whichever child last had it, falling back to
/// the first tab stop when there is none recorded or it is no longer a live,
/// enabled child (see [`focusable_child`]). Used both when the window
/// regains activation and, as a backstop, whenever focus lands on the
/// top-level window itself.
fn restore_focus(state: &WindowState) {
    let saved = hwnd_from_isize(state.last_focus.get());
    let target = if focusable_child(state, saved) {
        Some(saved)
    } else {
        unsafe { GetNextDlgTabItem(state.hwnd, None, false) }.ok()
    };
    if let Some(target) = target {
        unsafe {
            let _ = SetFocus(Some(target));
        }
    }
}

/// `WM_ACTIVATE`: on deactivate, remember whichever child currently holds
/// focus (nothing to remember if focus was already on the top level or
/// outside this window entirely); on (re)activate, hand focus back via
/// [`restore_focus`]. This is the canonical Win32 recipe for a non-dialog
/// top-level window and covers both a modal `MessageBoxW` returning control
/// to its owner and plain Alt+Tab reactivation — both are, from this
/// window's point of view, just activation changing away and back.
fn handle_activate(state: &WindowState, wparam: WPARAM) {
    let state_word = u32::try_from(wparam.0 & 0xFFFF).unwrap_or(WA_INACTIVE);
    if state_word == WA_INACTIVE {
        let focused = unsafe { GetFocus() };
        if focusable_child(state, focused) {
            state.last_focus.set(hwnd_to_isize(focused));
        }
    } else {
        restore_focus(state);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// DPI Change (WM_DPICHANGED)
// ─────────────────────────────────────────────────────────────────────────────

/// `WM_DPICHANGED`: the window moved to a monitor with a different DPI (or,
/// on a single monitor, its scale factor changed live). `wparam`'s low word
/// is the new DPI; `lparam` points at Windows' suggested new window rect,
/// already sized and positioned for that DPI on the target monitor.
///
/// Resizes to the suggested rect, rebuilds both fonts at the new size, swaps
/// them into [`WindowState`] (so a paint reentered synchronously from the
/// `WM_SETFONT`/redraw below — the checkbox custom-draw, the combo, the
/// capture fields, all of which read the `WindowState` font directly rather
/// than a control's own stored one — already sees the new handle, never a
/// stale one), then re-sends `WM_SETFONT` to every control. The old font
/// handles are freed only after that loop finishes: GDI requires a font stay
/// alive as long as any control still has it selected, and by that point
/// nothing does — every reader has already moved on to the new handle.
/// Finally, [`layout`] repositions every control at the new DPI from the
/// same baseline table `create_settings_window` used.
fn handle_dpichanged(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    let dpi = dpi_from_wparam(wparam.0);

    let suggested_ptr: *const RECT = std::ptr::with_exposed_provenance(lparam.0.cast_unsigned());
    // SAFETY: `WM_DPICHANGED` documents `lParam` as a `RECT` with the
    // suggested new window rect; the sender owns it for the duration of the
    // message and it is only read here. `as_ref` guards against null.
    if let Some(suggested) = unsafe { suggested_ptr.as_ref() } {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
    }

    with_window_state(|state| {
        state.dpi.set(dpi);

        let new_regular = build_font(dpi, FW_NORMAL);
        let new_bold = build_font(dpi, FW_BOLD);
        let old_regular = state.font_regular.replace(new_regular);
        let old_bold = state.font_bold.replace(new_bold);

        for spec in CONTROLS {
            let Ok(child) = (unsafe { GetDlgItem(Some(state.hwnd), i32::from(spec.id)) }) else {
                continue;
            };
            let font = if is_section_header(spec.id) {
                new_bold
            } else {
                new_regular
            };
            unsafe {
                SendMessageW(
                    child,
                    WM_SETFONT,
                    Some(WPARAM(font.0.expose_provenance())),
                    Some(LPARAM(1)),
                );
            }
        }

        // SAFETY: freeing the fonts the window owned until a moment ago. The
        // position of this block is the invariant: GDI requires a font outlive
        // the last `WM_SETFONT` that points at it, and the loop above has just
        // moved every control onto the new handles.
        unsafe {
            let _ = DeleteObject(old_regular.into());
            let _ = DeleteObject(old_bold.into());
        }
    });

    layout(hwnd, dpi);
    // Re-measure and re-apply after the font rebuild above and the reflow
    // just below it: the combo's system-default item height is font-driven,
    // so a DPI change (new font, new edit rects) invalidates whatever was
    // set at creation or the last DPI change.
    configure_combo_height(hwnd);
}

/// Reclaims ownership of `lparam`'s `Box<SettingsSnapshot>` and applies it.
/// Other half of the contract documented on [`SettingsSinkImpl::post_boxed`].
fn handle_refresh_message(lparam: LPARAM) {
    let ptr: *mut SettingsSnapshot =
        std::ptr::with_exposed_provenance_mut(lparam.0.cast_unsigned());
    // SAFETY: `lparam` is the pointer `post_boxed` produced with
    // `Box::into_raw` for exactly this message id. Exactly one of three
    // mutually exclusive reclaims runs: this one on delivery, `post_boxed`'s
    // own on a failed post, or `drain_pending_payload_messages`' — and that
    // last one only for a message it removed from the queue, which is
    // therefore never delivered here.
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
    // SAFETY: one delivery, one reclaim, as in [`handle_refresh_message`].
    let message = unsafe { Box::from_raw(ptr) };
    with_window_state(|state| set_text(state.hwnd, ID_HK_ERROR, &message));
}

// ─────────────────────────────────────────────────────────────────────────────
// Control Wiring (Instant Apply)
// ─────────────────────────────────────────────────────────────────────────────
// Instant apply, debounce-persisted by the controller — see
// docs/architecture.md §14, "Instant apply, debounced saves". The hotkey
// capture fields (`ID_HK_UP`/`ID_HK_DOWN`) are wired separately, in the
// "Hotkey Capture Control" section below (their own custom-drawn window
// class, not one of the controls this section applies to). The two explainer
// statics below are created with just their text; their colour (grayed, or
// red for an error) comes from `dark.rs`'s `WM_CTLCOLORSTATIC` handling, in
// both light and dark mode.

/// One spinner+edit control pair wired to instant apply: its edit id, valid
/// range (a [`RangeSpec`] from the layout table, keyed by the matching
/// up-down control's id), the checkbox that must be checked before it
/// applies (`None` for the three unconditional fields), how a clamped value
/// becomes the change to post, and — for the two checkbox-gated fields —
/// where to remember it for the session. Both the `EN_KILLFOCUS` and
/// `UDN_DELTAPOS` commit paths dispatch through this one table instead of
/// five hand-written near-copies of the same five steps.
pub(super) struct NumericField {
    edit_id: u16,
    range: &'static RangeSpec,
    checkbox_id: Option<u16>,
    to_change: fn(u32) -> SettingChange,
    remember: Option<fn(&WindowState, u32)>,
    /// Selects this field's slot in `WindowState` for the "what did we last
    /// actually post" comparison `commit_numeric_field` makes before
    /// posting again — see that `Cell` group's doc comment on `WindowState`.
    last_posted: fn(&WindowState) -> &Cell<Option<u32>>,
}

/// The five numeric fields, in tab order. Table-driven so the edit, its
/// spinner, its optional enabling checkbox and the message it produces stay
/// declared in one place; both commit paths (`EN_KILLFOCUS` and
/// `UDN_DELTAPOS`) look their field up here instead of matching on control
/// ids, though the display and checkbox paths still address controls by id.
pub(super) const NUMERIC_FIELDS: &[NumericField] = &[
    NumericField {
        edit_id: ID_STEP_EDIT,
        range: &RANGE_SPECS[0],
        checkbox_id: None,
        to_change: |v| SettingChange::StepPercent(to_u8(v)),
        remember: None,
        last_posted: |state| &state.last_posted_step,
    },
    NumericField {
        edit_id: ID_OSD_TIMEOUT_EDIT,
        range: &RANGE_SPECS[1],
        checkbox_id: None,
        to_change: SettingChange::OsdTimeoutMs,
        remember: None,
        last_posted: |state| &state.last_posted_timeout,
    },
    NumericField {
        edit_id: ID_OSD_OPACITY_EDIT,
        range: &RANGE_SPECS[2],
        checkbox_id: None,
        to_change: |v| SettingChange::OsdOpacityPercent(to_u8(v)),
        remember: None,
        last_posted: |state| &state.last_posted_opacity,
    },
    NumericField {
        edit_id: ID_RESYNC_EDIT,
        range: &RANGE_SPECS[3],
        checkbox_id: Some(ID_RESYNC_CHECK),
        to_change: SettingChange::RefreshPeriodicSeconds,
        remember: Some(|state, v| state.last_periodic.set(v)),
        last_posted: |state| &state.last_posted_periodic,
    },
    NumericField {
        edit_id: ID_INACT_EDIT,
        range: &RANGE_SPECS[4],
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
pub(super) fn get_text(hwnd: HWND, id: u16) -> String {
    let Ok(child) = (unsafe { GetDlgItem(Some(hwnd), i32::from(id)) }) else {
        return String::new();
    };
    window_text(child)
}

/// Reads `hwnd`'s own window text directly — the other half of [`get_text`],
/// split out because the hotkey capture control (see "Hotkey Capture
/// Control" below) reads its *own* text from inside its wndproc, where there
/// is no parent/id pair to resolve through `GetDlgItem`.
pub(super) fn window_text(hwnd: HWND) -> String {
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
/// posted, in which case this is a no-op: a "post only if changed" rule for
/// `EN_KILLFOCUS`, which is what keeps a spinner click's `EN_KILLFOCUS`
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
    let value = commit_edit_text(hwnd, field.edit_id, field.range.min, field.range.max);
    commit_numeric_field(field, value);
}

/// `UDN_DELTAPOS` on one of the five spinners: same commit as
/// [`handle_numeric_commit`], from a click instead of a focus change.
fn handle_spinner_delta(hwnd: HWND, updown_id: u16, delta: i32) {
    let Some(field) = NUMERIC_FIELDS
        .iter()
        .find(|f| f.range.updown_id == updown_id)
    else {
        return;
    };
    if !field_enabled(hwnd, field) {
        return;
    }
    let value = commit_spinner_delta(hwnd, field.edit_id, field.range.min, field.range.max, delta);
    commit_numeric_field(field, value);
}

/// Posts `change` to the controller, no-op while [`SUPPRESS_NOTIFICATIONS`]
/// is set — the one choke point every control-change handler posts
/// through, so a programmatic re-display never sends a message the user
/// never asked for.
pub(super) fn post_change(state: &WindowState, change: SettingChange) {
    if SUPPRESS_NOTIFICATIONS.with(Cell::get) {
        return;
    }
    send_message(state, BrightnessMessage::SettingChanged(change));
}

/// Sends a plain, non-`SettingChange` message to the controller — the
/// footer links, which only ever fire from a real click, never from
/// programmatic repopulation, so they bypass [`post_change`]'s suppression
/// gate entirely rather than needing it.
pub(super) fn send_message(state: &WindowState, message: BrightnessMessage) {
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
///
/// No explicit focus save/restore wraps this call: a modal `MessageBoxW`
/// owned by `hwnd` is a separate top-level window taking activation away
/// from and back to its owner, so opening and closing it round-trips
/// through `WM_ACTIVATE` like any other activation change — `handle_activate`
/// already saves and restores focus for that. Even on the off chance
/// activation alone left focus stranded on `hwnd`, `WM_SETFOCUS`'s own
/// `restore_focus` call catches it immediately.
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
///
/// Same reasoning as [`confirm_restore_defaults`] for not wrapping this call
/// in an explicit focus save/restore: `WM_ACTIVATE` already covers a modal
/// child's open/close, and `WM_SETFOCUS` backstops the case where it
/// doesn't.
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

/// `IDCANCEL`, the id `IsDialogMessageW` reports via `WM_COMMAND` on Esc.
/// Defined locally because the crate types the constant of that value as
/// `MESSAGEBOX_RESULT` (a message-box return code), not a dialog control id;
/// the numeric value is the same either way.
const IDCANCEL_ID: u16 = 2;

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
        // A value typed into a numeric edit but not yet committed (no
        // EN_KILLFOCUS fired) still survives closing this way: DestroyWindow
        // moves focus off the still-focused edit before it tears the window
        // down, so that edit's EN_KILLFOCUS — and the commit it triggers —
        // reaches this thread's queue and is processed before WM_DESTROY
        // clears window state, ahead of SettingsClosed. Verified on
        // hardware. This depends on that message ordering; if a future
        // Windows build ever delivered WM_DESTROY first, an explicit commit
        // before posting WM_CLOSE below would become necessary.
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

/// Routes a `WM_NOTIFY`: a spinner's `UDN_DELTAPOS` commits its delta,
/// `NM_CLICK` (mouse) on the footer `SysLink` posts the shell side effect
/// matching whichever of its two embedded links was clicked — keyboard
/// activation of the same links is handled separately, by the subclass
/// installed on that control (see "Footer Link Keyboard Activation"
/// below), not through a notification here — and `NM_CUSTOMDRAW` on one of
/// the five checkboxes hand-paints its label (see [`dark::checkbox_custom_draw`]) —
/// the one notification code here whose return value the caller must
/// actually use, since it tells the checkbox whether to skip its own
/// default paint. Every other notification code is ignored and answers `0`
/// (`CDRF_DODEFAULT`, though it is only ever inspected for
/// `NM_CUSTOMDRAW`).
fn handle_notify(hwnd: HWND, lparam: LPARAM) -> u32 {
    let hdr_ptr: *const NMHDR = std::ptr::with_exposed_provenance(lparam.0.cast_unsigned());
    // SAFETY: `WM_NOTIFY`'s `lParam` points at an `NMHDR` the notifying
    // control owns and keeps alive for the duration of the message; nothing
    // here holds a reference past it. Each arm below widens that pointer to a
    // larger struct, and what guarantees the sender really allocated one
    // differs by arm: for `UDN_DELTAPOS`/`NM_CUSTOMDRAW` the notification code
    // alone does it, but `NM_CLICK` is generic and carries a bare `NMHDR` from
    // most controls — the `NMLINK` widening is sound only because the guard
    // also pins the sender to the `SysLink` by control id. Do not drop that id
    // test on the grounds that the notification code is enough.
    let Some(hdr) = (unsafe { hdr_ptr.as_ref() }) else {
        return 0;
    };
    let id = u16::try_from(hdr.idFrom).unwrap_or(u16::MAX);

    match hdr.code {
        UDN_DELTAPOS => {
            let nmud_ptr: *const NMUPDOWN = hdr_ptr.cast();
            if let Some(nmud) = unsafe { nmud_ptr.as_ref() } {
                handle_spinner_delta(hwnd, id, nmud.iDelta);
            }
            0
        }
        // The footer SysLink carries both links as one control; which one
        // was clicked comes from NMLINK's embedded-link index, not the
        // control id (there is only one id now).
        NM_CLICK if id == ID_LINK_CONFIG => {
            let nmlink_ptr: *const NMLINK = hdr_ptr.cast();
            let message =
                unsafe { nmlink_ptr.as_ref() }.and_then(|nmlink| match nmlink.item.iLink {
                    0 => Some(BrightnessMessage::OpenConfigFile),
                    1 => Some(BrightnessMessage::TrayOpenLogFolder),
                    _ => None,
                });
            if let Some(message) = message {
                with_window_state(|state| send_message(state, message));
            }
            0
        }
        NM_CUSTOMDRAW if dark::is_checkbox_id(id) => {
            let cd_ptr: *const NMCUSTOMDRAW = hdr_ptr.cast();
            unsafe { cd_ptr.as_ref() }.map_or(0, dark::checkbox_custom_draw)
        }
        _ => 0,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Footer Link Keyboard Activation (SysLink subclass)
// ─────────────────────────────────────────────────────────────────────────────
// SysLink's own Enter handling is unreliable past the first embedded link
// (mouse clicks are unaffected, still handled by `NM_CLICK` above) — see
// the footer SysLink note in docs/architecture.md §14. Rather than depend
// on the control's own internal link-focus state machine, this subclass
// tracks which of the two links is keyboard-focused itself, drives the
// control's `LIS_FOCUSED` bit to match (so the native paint still draws the
// focus indicator on the right link), and dispatches Enter directly instead
// of waiting for a notification that may never arrive.

/// Number of embedded links the merged footer `SysLink` carries — its
/// `<a>` markup has exactly two: "Open config file" and "Open log folder".
const FOOTER_LINK_COUNT: i32 = 2;

/// Subclass id for the footer link. Ids only have to be unique per control,
/// so this may repeat the ones `dark.rs` uses for its own subclasses.
const FOOTER_LINK_SUBCLASS_ID: usize = 1;

thread_local! {
    /// Which of the footer link's two embedded links (`0` or `1`) is
    /// keyboard-focused right now. Reset to `0` on every `WM_SETFOCUS`
    /// (SysLink's own documented default, kept here rather than trusted
    /// from the control); advanced or retreated only by this subclass's own
    /// `WM_KEYDOWN` handling for `VK_TAB`.
    static FOOTER_LINK_FOCUS: Cell<i32> = const { Cell::new(0) };
}

/// Installs the footer link's keyboard-activation subclass. Called once at
/// control creation, matching every other subclass this window installs
/// (see `dark::install_edit_border_subclass` and its siblings).
fn install_footer_link_subclass(hwnd: HWND) {
    unsafe {
        let _ = SetWindowSubclass(
            hwnd,
            Some(footer_link_subclass_proc),
            FOOTER_LINK_SUBCLASS_ID,
            0,
        );
    }
}

/// Subclass procedure for the footer `SysLink`: takes over Tab and Enter so
/// both embedded links are reachable from the keyboard, which the control's
/// own handling gets wrong past the first link (see the section comment
/// above), and removes itself on `WM_NCDESTROY`.
///
/// # Safety
///
/// This is a Windows callback, invoked by the common-controls subclass
/// dispatcher. The caller ensures `hwnd` is valid and that `wparam`/`lparam`
/// match `msg`.
unsafe extern "system" fn footer_link_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    _refdata: usize,
) -> LRESULT {
    // SAFETY: called by the common-controls subclass dispatcher, which
    // upholds the same contract as a window procedure: `hwnd` is the live
    // `SysLink` this subclass is installed on, and `wparam`/`lparam` mean
    // what `msg` documents. Forwarding to `DefSubclassProc` is only valid
    // from inside such a call, which is the invariant this block rests on.
    unsafe {
        match msg {
            // Gaining focus (Tab in from either direction) always starts on
            // the first link, matching SysLink's documented default; the
            // native call runs first so the control's own has-focus
            // bookkeeping updates, then the explicit LM_SETITEM pins the
            // link state to match rather than trusting the control got
            // there itself.
            WM_SETFOCUS => {
                let result = DefSubclassProc(hwnd, msg, wparam, lparam);
                FOOTER_LINK_FOCUS.with(|f| f.set(0));
                set_footer_link_focus(hwnd, 0);
                result
            }
            WM_KEYDOWN if wparam.0 == usize::from(VK_TAB.0) => {
                let shift = GetKeyState(i32::from(VK_SHIFT.0)) < 0;
                let focus = FOOTER_LINK_FOCUS.with(Cell::get);
                let next = if shift { focus - 1 } else { focus + 1 };
                if !(0..FOOTER_LINK_COUNT).contains(&next) {
                    // No further link in this direction. WM_GETDLGCODE
                    // below should have kept IsDialogMessageW from
                    // forwarding this key at all; this is a fail-safe for
                    // a Tab that reaches the control some other way, not a
                    // path normal keyboard navigation takes.
                    return LRESULT(0);
                }
                FOOTER_LINK_FOCUS.with(|f| f.set(next));
                set_footer_link_focus(hwnd, next);
                LRESULT(0)
            }
            // Dispatches directly from the tracked link index instead of
            // forwarding to the control's own Enter handling, which is what
            // silently drops the second link (see this section's doc
            // comment above) — so DefSubclassProc is deliberately never
            // called for this key.
            WM_KEYDOWN if wparam.0 == usize::from(VK_RETURN.0) => {
                let message = if FOOTER_LINK_FOCUS.with(Cell::get) == 0 {
                    BrightnessMessage::OpenConfigFile
                } else {
                    BrightnessMessage::TrayOpenLogFolder
                };
                with_window_state(|state| send_message(state, message));
                LRESULT(0)
            }
            // Tells IsDialogMessageW to hand Enter to this control always,
            // and Tab only while another link remains in the requested
            // direction — otherwise Tab falls through to normal dialog
            // navigation, moving focus to the next/previous real control.
            WM_GETDLGCODE => {
                let vk = get_dlg_code_query_vkey(lparam);
                let mut code = DLGC_HASSETSEL;
                if vk == usize::from(VK_RETURN.0) {
                    code |= DLGC_WANTMESSAGE;
                } else if vk == usize::from(VK_TAB.0) {
                    let shift = GetKeyState(i32::from(VK_SHIFT.0)) < 0;
                    let focus = FOOTER_LINK_FOCUS.with(Cell::get);
                    let has_more_links = if shift {
                        focus > 0
                    } else {
                        focus + 1 < FOOTER_LINK_COUNT
                    };
                    code |= if has_more_links {
                        DLGC_WANTTAB
                    } else {
                        DLGC_WANTCHARS
                    };
                }
                LRESULT(isize::try_from(code).unwrap_or(0))
            }
            WM_NCDESTROY => {
                let _ = RemoveWindowSubclass(
                    hwnd,
                    Some(footer_link_subclass_proc),
                    FOOTER_LINK_SUBCLASS_ID,
                );
                DefSubclassProc(hwnd, msg, wparam, lparam)
            }
            _ => DefSubclassProc(hwnd, msg, wparam, lparam),
        }
    }
}

/// Sets exactly one of the footer link's two embedded links as
/// keyboard-focused via `LM_SETITEM`, clearing the other. `LIS_FOCUSED`
/// only ever changes from here, in lockstep with [`FOOTER_LINK_FOCUS`], so
/// the control's own paint never disagrees with which link Enter would
/// activate.
fn set_footer_link_focus(hwnd: HWND, focused: i32) {
    for link in 0..FOOTER_LINK_COUNT {
        let mut item = LITEM {
            mask: LIF_ITEMINDEX | LIF_STATE,
            iLink: link,
            stateMask: LIS_FOCUSED,
            state: if link == focused {
                LIS_FOCUSED
            } else {
                LIST_ITEM_STATE_FLAGS(0)
            },
            ..Default::default()
        };
        unsafe {
            SendMessageW(
                hwnd,
                LM_SETITEM,
                None,
                Some(LPARAM((&raw mut item).expose_provenance().cast_signed())),
            );
        }
    }
}

/// The virtual-key code a `WM_GETDLGCODE` query is asking about, read from
/// the `MSG` its `lParam` points at — `0` (matches no key this handler
/// cares about) when the query has no message context, which happens for
/// callers that query `WM_GETDLGCODE` outside of `IsDialogMessageW`'s own
/// per-key routing.
fn get_dlg_code_query_vkey(lparam: LPARAM) -> usize {
    let msg_ptr: *const MSG = std::ptr::with_exposed_provenance(lparam.0.cast_unsigned());
    // SAFETY: on a per-key `WM_GETDLGCODE`, `lparam` points at the `MSG`
    // `IsDialogMessageW` is asking about, alive for the duration of the query
    // and only read here; `as_ref` covers the zero `lparam` of the
    // context-free form of the query documented above.
    unsafe { msg_ptr.as_ref() }.map_or(0, |msg| msg.wParam.0)
}

/// Shared shape for every `WM_CTLCOLOR*` handler: runs `f` against the open
/// window's state, and falls back to `DefWindowProcW`'s own answer whenever
/// `f` returns `None` — no open window, or (see each `dark::ctlcolor_*`
/// function) light mode leaving a particular control's colour untouched.
fn ctlcolor_reply(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    f: impl FnOnce(&WindowState) -> Option<LRESULT>,
) -> LRESULT {
    let mut result = None;
    with_window_state(|state| result = f(state));
    result.unwrap_or_else(|| unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) })
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
            // SAFETY: the window owns both fonts and frees each exactly once.
            // Reaching this only on `WM_DESTROY` is what makes it sound: no
            // control can still have one selected, since they are gone with
            // the window.
            unsafe {
                let _ = DeleteObject(state.font_regular.get().into());
                let _ = DeleteObject(state.font_bold.get().into());
            }
            // Puts the window class's background brush back on the system
            // default before freeing the brushes it might currently point
            // at — GCLP_HBRBACKGROUND is class-wide state that survives
            // this window, so a later window of the same class must never
            // find it pointed at a handle this call is about to delete.
            dark::set_class_background(state.hwnd, false, &state.palette);
            state.palette.destroy();
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
    // SAFETY: Windows is the caller, so `hwnd` is a live window of this
    // class and `wparam`/`lparam` carry whatever `msg` documents them to.
    // Matching on `msg` first is what makes that reading correct, and it is
    // the guarantee the per-message handlers below are written against —
    // they take a bare `LPARAM` and cannot re-establish it themselves.
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
            WM_ACTIVATE => {
                with_window_state(|state| handle_activate(state, wparam));
                LRESULT(0)
            }
            WM_SETFOCUS => {
                with_window_state(restore_focus);
                LRESULT(0)
            }
            WM_NOTIFY => LRESULT(isize::try_from(handle_notify(hwnd, lparam)).unwrap_or(0)),
            WM_CTLCOLORSTATIC => ctlcolor_reply(hwnd, msg, wparam, lparam, |s| {
                dark::ctlcolor_static(s, wparam, lparam)
            }),
            WM_CTLCOLOREDIT => ctlcolor_reply(hwnd, msg, wparam, lparam, |s| {
                dark::ctlcolor_edit(s, wparam)
            }),
            WM_CTLCOLORBTN => {
                ctlcolor_reply(hwnd, msg, wparam, lparam, |s| dark::ctlcolor_btn(s, wparam))
            }
            WM_CTLCOLORLISTBOX => ctlcolor_reply(hwnd, msg, wparam, lparam, |s| {
                dark::ctlcolor_listbox(s, wparam)
            }),
            WM_DPICHANGED => {
                handle_dpichanged(hwnd, wparam, lparam);
                LRESULT(0)
            }
            WM_SETTINGCHANGE => {
                if dark::is_immersive_color_set_change(lparam) {
                    with_window_state(|state| {
                        state.dark.set(dark::initial_dark_flag());
                        dark::apply_theme(state);
                    });
                }
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
    configure_combo_height(hwnd);

    let state = WindowState {
        hwnd,
        sender: tx.clone(),
        hwnd_slot: Arc::clone(hwnd_slot),
        font_regular: Cell::new(font_regular),
        font_bold: Cell::new(font_bold),
        dpi: Cell::new(placement.dpi),
        dark: Cell::new(dark::initial_dark_flag()),
        palette: dark::Palette::new(),
        // No control has ever had focus yet, so there is nothing to
        // restore to until the first WM_ACTIVATE deactivate records one.
        last_focus: Cell::new(0),
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
    // Store state before applying the theme, not after: apply_theme's
    // RedrawWindow call synchronously re-enters every child's paint path,
    // and each of those reads the live dark flag through
    // with_window_state — which sees nothing at all while WINDOW_STATE is
    // still empty, so the very first repaint would silently fall back to
    // light regardless of what dark actually holds.
    WINDOW_STATE.with(|s| *s.borrow_mut() = Some(state));
    with_window_state(dark::apply_theme);

    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        if let Ok(first) = GetDlgItem(Some(hwnd), i32::from(ID_AUTOSTART)) {
            let _ = SetFocus(Some(first));
        }
    }

    Ok(hwnd)
}

/// Defensive backstop for `WM_APP_SETTINGS_*` payload messages still
/// carrying a `Box` for `hwnd`, walked after the message loop above has
/// ended.
///
/// In practice this never finds anything to reclaim: `PostQuitMessage`
/// only yields `WM_QUIT` once the rest of the thread's queue is otherwise
/// empty, so every message posted to `hwnd` — payload messages included —
/// is always retrieved and dispatched by the loop's own `GetMessageW`/
/// `DispatchMessageW` cycling *before* `WM_QUIT` can ever surface. A post
/// attempted after the window is already gone fails at `PostMessageW`
/// itself and is reclaimed immediately in [`SettingsSinkImpl::post_boxed`],
/// not here. This walk exists as depth against that queue-ordering
/// guarantee, not because it is known to run against a live message.
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
            // SAFETY (both arms): these messages were removed from the queue
            // by `PeekMessageW` above, so their wndproc arms will never run
            // and no other reclaim of these pointers exists. Each `lparam` is
            // the `Box::into_raw` pointer `post_boxed` produced for that
            // message id, which is what makes the type of each cast correct.
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
            // Creation never reached handle_destroy, which is the usual
            // sender of this message, so send it here — otherwise the
            // controller's settings_open latches true for the rest of the
            // process's life.
            if let Err(e) = tx.send(BrightnessMessage::SettingsClosed) {
                log::warn!(error:% = e; "Failed to send SettingsClosed (controller channel closed?)");
            }
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
        // SAFETY: `hwnd` was reconstituted from the `AtomicIsize` the
        // settings thread stored, so this posts to another thread's window.
        // `PostMessageW` is one of the few window APIs documented as callable
        // from any thread, and it fails cleanly rather than misbehaving if
        // that window has since been destroyed — which is the whole reason
        // the handle may cross the boundary as a bare `isize` at all.
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
    /// `Box::from_raw`. If the post itself fails — including a post
    /// attempted after the window is already gone, which fails right here
    /// at `PostMessageW` rather than surfacing later as a dropped message —
    /// the box is reconstituted and dropped right here instead of leaking.
    /// Between the window being alive when a post is made (reclaimed by the
    /// wndproc) and a post failing outright once it is not (reclaimed
    /// above), every `Box` this function hands off is accounted for;
    /// `drain_pending_payload_messages` is defensive depth on top, not a
    /// third path this relies on.
    fn post_boxed<T>(&self, msg: u32, value: T) {
        let Some(hwnd) = self.target_hwnd() else {
            return;
        };
        let ptr = Box::into_raw(Box::new(value));
        let lparam = LPARAM(ptr.expose_provenance().cast_signed());
        // SAFETY: cross-thread `PostMessageW` as in [`Self::post_simple`].
        // Additionally, ownership of the box travels in `lparam`: on success
        // the receiving arm's single `Box::from_raw` frees it, and on failure
        // the branch below does, so the value is freed exactly once either
        // way. `T` must match what the receiver casts `lparam` back to, which
        // the per-message-id call sites are responsible for.
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
                let spawned = std::thread::Builder::new()
                    .name("settings".to_string())
                    .spawn(move || run_settings_window(&tx, &hwnd_slot, &snapshot));
                if let Err(e) = spawned {
                    // The slot is still OPENING, which every later activation
                    // reads as "a window is on its way". Release it, or Settings
                    // stays unopenable for the rest of the process.
                    self.hwnd.store(0, Ordering::SeqCst);
                    log::error!(error:% = e; "Failed to spawn settings window thread");
                    // No thread means no window and so no handle_destroy, the
                    // usual sender of this message; without it the controller's
                    // settings_open latches true for the rest of the process.
                    if let Err(e) = self.tx.send(BrightnessMessage::SettingsClosed) {
                        log::warn!(error:% = e; "Failed to send SettingsClosed (controller channel closed?)");
                    }
                }
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
    use super::super::layout::{ID_OSD_OPACITY_UPDOWN, ID_OSD_TIMEOUT_UPDOWN, ID_STEP_UPDOWN};
    use super::*;

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
        // Typing 999 into the step field (range 1-50) clamps to 50 on
        // focus loss, rather than rejecting the edit outright.
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
                ids.contains(&field.range.updown_id),
                "NumericField.range.updown_id {} has no CONTROLS entry",
                field.range.updown_id
            );
            if let Some(checkbox_id) = field.checkbox_id {
                assert!(
                    ids.contains(&checkbox_id),
                    "NumericField.checkbox_id {checkbox_id} has no CONTROLS entry"
                );
            }
            assert!(
                field.range.min <= field.range.max,
                "NumericField for edit {} has min > max",
                field.edit_id
            );
        }
    }

    #[test]
    fn every_numeric_field_range_matches_the_spinner_range_it_names() {
        // configure_updowns and every NumericField both read RANGE_SPECS
        // directly now, so there is no second copy left for this table to
        // drift from — this pins the layout table against a checked-in
        // expectation instead, so an accidental edit to one of those values
        // shows up as a failing test rather than silently reaching every
        // spinner and every EN_KILLFOCUS/UDN_DELTAPOS clamp.
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
                .find(|f| f.range.updown_id == updown_id)
                .unwrap_or_else(|| panic!("no NumericField for updown {updown_id}"));
            assert_eq!(field.range.min, min, "min mismatch for updown {updown_id}");
            assert_eq!(field.range.max, max, "max mismatch for updown {updown_id}");
        }
    }

    #[test]
    fn every_range_spec_is_referenced_by_exactly_one_numeric_field() {
        // The layout table (RANGE_SPECS) and this module's control table
        // (NUMERIC_FIELDS) must stay in a strict one-to-one correspondence:
        // pointer identity, not value equality, so a NumericField that
        // accidentally carried its own inline RangeSpec copy (reintroducing
        // the drift this refactor removes) would fail this too.
        for spec in RANGE_SPECS {
            let referencing = NUMERIC_FIELDS
                .iter()
                .filter(|field| std::ptr::eq(field.range, spec))
                .count();
            assert_eq!(
                referencing, 1,
                "RangeSpec for updown {} is referenced by {referencing} NumericFields, expected exactly 1",
                spec.updown_id
            );
        }
    }
}
