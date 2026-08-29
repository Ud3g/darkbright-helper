//! Dark-mode opt-in for the app's Win32 popup menus.
//!
//! Windows draws `TrackPopupMenu` menus light unless the process has opted
//! into dark mode. The switches that grant that opt-in live in `uxtheme.dll`
//! but are exported by ordinal only and are absent from the public headers, so
//! they are resolved at runtime rather than linked.
//!
//! Everything here is best-effort: if the build is too old, the library will
//! not load, or an ordinal has moved, the menus stay exactly as light as they
//! were. There is no error path a caller could act on.

use std::sync::OnceLock;

use windows::Win32::Foundation::{ERROR_SUCCESS, HMODULE, HWND, TRUE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Registry::{HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW};
use windows::core::{BOOL, PCSTR, PCWSTR, w};

/// First Windows build whose `uxtheme.dll` carries the dark-mode ordinals in
/// the shape used here (Windows 10 1809). Earlier builds either lack them or
/// export a differently-shaped function under the same ordinal.
const MIN_DARK_MENU_BUILD: u32 = 17763;

/// Parses the `CurrentBuildNumber` registry string into a build number.
///
/// Returns `None` for anything that is not a plain decimal number, which is
/// all the value has ever been.
fn parse_build_number(raw: &str) -> Option<u32> {
    raw.trim_matches(|c: char| c.is_whitespace() || c == '\0')
        .parse()
        .ok()
}

/// Whether a Windows build exposes the dark-mode menu ordinals.
fn supports_dark_menus(build: u32) -> bool {
    build >= MIN_DARK_MENU_BUILD
}

// ─────────────────────────────────────────────────────────────────────────────
// Undocumented `uxtheme.dll` Surface
// ─────────────────────────────────────────────────────────────────────────────

/// `AllowDarkModeForWindow` — opts one window into dark mode.
const ORD_ALLOW_DARK_MODE_FOR_WINDOW: usize = 133;

/// `SetPreferredAppMode` — sets the process-wide dark-mode policy.
const ORD_SET_PREFERRED_APP_MODE: usize = 135;

/// `FlushMenuThemes` — re-reads the menu theme after the policy or the system
/// setting changed.
const ORD_FLUSH_MENU_THEMES: usize = 136;

/// `RefreshImmersiveColorPolicyState` — re-reads the system light/dark setting
/// into the process-wide cache the other calls consult.
const ORD_REFRESH_IMMERSIVE_COLOR_POLICY_STATE: usize = 104;

/// `PreferredAppMode::AllowDark` — follow the system light/dark setting. The
/// deliberate choice over `ForceDark`: the user's setting decides, not us.
const APP_MODE_ALLOW_DARK: i32 = 1;

/// The shape `GetProcAddress` hands back before it is given its real type.
type RawProc = unsafe extern "system" fn() -> isize;

type SetPreferredAppModeFn = unsafe extern "system" fn(i32) -> i32;
type AllowDarkModeForWindowFn = unsafe extern "system" fn(HWND, BOOL) -> BOOL;
type FlushMenuThemesFn = unsafe extern "system" fn();
type RefreshImmersiveColorPolicyStateFn = unsafe extern "system" fn();

/// The entry points, resolved together or not at all.
struct DarkModeApi {
    set_preferred_app_mode: SetPreferredAppModeFn,
    allow_dark_mode_for_window: AllowDarkModeForWindowFn,
    flush_menu_themes: FlushMenuThemesFn,
    refresh_immersive_color_policy_state: RefreshImmersiveColorPolicyStateFn,
}

/// Resolved once per process; `None` means "leave the menus light".
static DARK_MODE_API: OnceLock<Option<DarkModeApi>> = OnceLock::new();

/// Returns the resolved entry points, loading them on first use.
fn api() -> Option<&'static DarkModeApi> {
    DARK_MODE_API.get_or_init(load_dark_mode_api).as_ref()
}

/// Looks up one ordinal-only export.
///
/// # Safety
///
/// `module` must be a live module handle.
unsafe fn resolve(module: HMODULE, ordinal: usize) -> Option<RawProc> {
    // An ordinal is passed where a name pointer goes, as MAKEINTRESOURCE does;
    // it is a sentinel value, never dereferenced, hence `without_provenance`.
    unsafe { GetProcAddress(module, PCSTR(std::ptr::without_provenance(ordinal))) }
}

/// Loads the dark-mode entry points, or gives up quietly.
fn load_dark_mode_api() -> Option<DarkModeApi> {
    match current_build_number() {
        Some(build) if supports_dark_menus(build) => {}
        Some(build) => {
            log::debug!(build = build; "Windows build predates dark menus; tray menu stays light");
            return None;
        }
        None => {
            log::debug!("Windows build number unreadable; tray menu stays light");
            return None;
        }
    }

    // SAFETY: a literal library name and, below, ordinals looked up in the
    // module that call returned. Every result is checked before it is used.
    unsafe {
        // Deliberately never freed: the resolved pointers have to stay valid
        // for as long as menus can be opened, i.e. the life of the process.
        let module = LoadLibraryW(w!("uxtheme.dll"))
            .inspect_err(|e| {
                log::warn!(error:% = e; "uxtheme.dll did not load; tray menu stays light");
            })
            .ok()?;

        let set = resolve(module, ORD_SET_PREFERRED_APP_MODE)?;
        let allow = resolve(module, ORD_ALLOW_DARK_MODE_FOR_WINDOW)?;
        let flush = resolve(module, ORD_FLUSH_MENU_THEMES)?;
        let refresh = resolve(module, ORD_REFRESH_IMMERSIVE_COLOR_POLICY_STATE)?;

        Some(DarkModeApi {
            // SAFETY: the ordinals identify these signatures on every build at
            // or above `MIN_DARK_MENU_BUILD`, which the gate above enforced.
            set_preferred_app_mode: std::mem::transmute::<RawProc, SetPreferredAppModeFn>(set),
            allow_dark_mode_for_window: std::mem::transmute::<RawProc, AllowDarkModeForWindowFn>(
                allow,
            ),
            flush_menu_themes: std::mem::transmute::<RawProc, FlushMenuThemesFn>(flush),
            refresh_immersive_color_policy_state: std::mem::transmute::<
                RawProc,
                RefreshImmersiveColorPolicyStateFn,
            >(refresh),
        })
    }
}

/// Reads `CurrentBuildNumber` from the registry.
///
/// Returns `None` if the value is missing, longer than a build number has ever
/// been, or not a number — each of which leaves the menus light.
fn current_build_number() -> Option<u32> {
    const SUBKEY: PCWSTR = w!(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion");
    const VALUE: PCWSTR = w!("CurrentBuildNumber");

    let mut buffer = [0u16; 16];
    let mut size_bytes = u32::try_from(std::mem::size_of_val(&buffer)).ok()?;

    // SAFETY: the buffer and its byte size are handed over together, and the
    // key/value names are static NUL-terminated literals.
    let status = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            SUBKEY,
            VALUE,
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&raw mut size_bytes),
        )
    };

    if status != ERROR_SUCCESS {
        log::debug!(error_code = status.0; "Reading CurrentBuildNumber failed");
        return None;
    }

    parse_build_number(&String::from_utf16_lossy(&buffer))
}

// ─────────────────────────────────────────────────────────────────────────────
// Public Surface
// ─────────────────────────────────────────────────────────────────────────────

/// Opts the process into dark menus whenever the system asks for them.
///
/// Call once, before the first menu is shown. Does nothing on a Windows build
/// without the ordinals — the menus then look exactly as they did before.
pub(crate) fn init_dark_menus() {
    let Some(api) = api() else { return };
    // SAFETY: pointers resolved from the ordinals for exactly these signatures.
    unsafe {
        (api.set_preferred_app_mode)(APP_MODE_ALLOW_DARK);
        (api.refresh_immersive_color_policy_state)();
        (api.flush_menu_themes)();
    }
    log::debug!("Dark-mode menus enabled; the system setting decides the colour");
}

/// Opts a single window into dark mode.
pub(crate) fn allow_dark_mode_for_window(hwnd: HWND) {
    let Some(api) = api() else { return };
    // SAFETY: as above; `hwnd` is a live window handle owned by the caller.
    unsafe {
        // The return value is the window's previous state, which we never need.
        let _ = (api.allow_dark_mode_for_window)(hwnd, TRUE);
    }
}

/// Picks up a system light/dark setting that changed since the last menu.
///
/// Call immediately before building a popup menu. Windows announces such a
/// change with a `WM_SETTINGCHANGE` broadcast, but the tray's window is
/// message-only and message-only windows are excluded from broadcasts — so the
/// menu is refreshed where it is built rather than where the news would have
/// arrived. At one call per right-click the cost is irrelevant.
///
/// Both calls are needed: `FlushMenuThemes` on its own leaves the menu in the
/// colour it had when the process started, which is why the refresh comes
/// first. The same pairing appears in the reference implementations that use
/// this API (wxWidgets `darkmode.cpp`, `ysc3839/win32-darkmode`).
pub(crate) fn refresh_menu_theme() {
    let Some(api) = api() else { return };
    // SAFETY: pointers resolved from the ordinals for exactly these signatures.
    unsafe {
        (api.refresh_immersive_color_policy_state)();
        (api.flush_menu_themes)();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_plain_build_number() {
        assert_eq!(parse_build_number("22621"), Some(22621));
    }

    #[test]
    fn tolerates_surrounding_whitespace_and_nul_padding() {
        // RegGetValueW hands back a NUL-terminated string; a stray terminator
        // left in the buffer must not turn a good value into "unknown".
        assert_eq!(parse_build_number("22621\0"), Some(22621));
        assert_eq!(parse_build_number(" 19045 "), Some(19045));
    }

    #[test]
    fn rejects_a_non_numeric_build_number() {
        assert_eq!(parse_build_number(""), None);
        assert_eq!(parse_build_number("22H2"), None);
    }

    #[test]
    fn gates_on_the_first_build_with_dark_menus() {
        assert!(!supports_dark_menus(17134));
        assert!(supports_dark_menus(MIN_DARK_MENU_BUILD));
        assert!(supports_dark_menus(22621));
    }
}
