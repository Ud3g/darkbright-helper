//! Hotkey parsing and registration for Windows.
//!
//! This module provides functionality to parse hotkey strings (e.g., "Ctrl+Shift+Up")
//! and register them as global hotkeys using the Windows `RegisterHotKey` API.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::LazyLock;
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
    VIRTUAL_KEY, VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F2, VK_F3, VK_F4, VK_F5,
    VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11, VK_F12, VK_HOME, VK_INSERT, VK_LEFT, VK_NEXT,
    VK_OEM_MINUS, VK_OEM_PLUS, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
};

// Standard Windows Virtual Key codes for brightness (not always in windows crate)
pub const VK_BRIGHTNESS_UP: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);
pub const VK_BRIGHTNESS_DOWN: VIRTUAL_KEY = VIRTUAL_KEY(0xE9);

/// Hook code indicating the hook procedure must process the message.
/// Used in low-level keyboard hook callbacks.
const HC_ACTION: i32 = 0;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GetMessageW, HHOOK, HMENU, HWND_MESSAGE, KBDLLHOOKSTRUCT, MSG,
    RegisterClassW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WM_HOTKEY, WM_KEYDOWN, WM_SYSKEYDOWN, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

use crate::core::state::BrightnessMessage;
use crate::error::{BrightnessError, Result};
use crate::platform::windows::last_error_as_brightness_error;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Hotkey ID for the primary brightness up command.
pub const BRIGHTNESS_UP_ID: i32 = 1;

/// Hotkey ID for the primary brightness down command.
pub const BRIGHTNESS_DOWN_ID: i32 = 2;

/// Hotkey ID for the secondary (dedicated key) brightness up command.
pub const BRIGHTNESS_UP_ALT_ID: i32 = 3;

/// Hotkey ID for the secondary (dedicated key) brightness down command.
pub const BRIGHTNESS_DOWN_ALT_ID: i32 = 4;

// ─────────────────────────────────────────────────────────────────────────────
// Low-Level Keyboard Hook Support
// ─────────────────────────────────────────────────────────────────────────────

/// Context data for the low-level keyboard hook callback.
///
/// Since the hook callback is a static `extern "system" fn` that cannot capture
/// state, we use thread-local storage to pass context to the callback.
struct HookContext {
    /// Channel sender to transmit brightness adjustment events.
    sender: Sender<BrightnessMessage>,
    /// Brightness step percentage (signed for directional adjustment).
    step_percent: i8,
}

thread_local! {
    /// Thread-local storage for hook callback context.
    ///
    /// Initialized by `set_hook_context()` before installing the hook,
    /// accessed by `with_hook_context()` from the callback.
    static HOOK_CONTEXT: RefCell<Option<HookContext>> = const { RefCell::new(None) };
}

/// Initializes the thread-local hook context.
///
/// Must be called on the same thread where the hook will be installed,
/// before calling `SetWindowsHookExW`.
fn set_hook_context(sender: Sender<BrightnessMessage>, step_percent: i8) {
    HOOK_CONTEXT.with(|ctx| {
        *ctx.borrow_mut() = Some(HookContext {
            sender,
            step_percent,
        });
    });
}

/// Executes a closure with access to the hook context.
///
/// Returns `None` if the context has not been initialized.
fn with_hook_context<R>(f: impl FnOnce(&HookContext) -> R) -> Option<R> {
    HOOK_CONTEXT.with(|ctx| ctx.borrow().as_ref().map(f))
}

/// RAII wrapper for a Windows hook handle (`HHOOK`).
///
/// Automatically calls `UnhookWindowsHookEx` when dropped to ensure
/// the hook is always unregistered.
struct SafeHook(HHOOK);

impl SafeHook {
    /// Creates a new `SafeHook` from a raw `HHOOK`.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `hook` is a valid hook handle
    /// returned by `SetWindowsHookExW`.
    const unsafe fn new(hook: HHOOK) -> Self {
        Self(hook)
    }

    /// Returns the raw `HHOOK` handle.
    ///
    /// Used when calling `CallNextHookEx`.
    #[allow(dead_code)]
    const fn as_raw(&self) -> HHOOK {
        self.0
    }

    /// Returns `true` if the hook handle is valid (non-null).
    #[allow(dead_code)]
    fn is_valid(&self) -> bool {
        !self.0.is_invalid()
    }
}

impl Drop for SafeHook {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: We own the hook handle and it was valid when created.
            // UnhookWindowsHookEx is safe to call on a valid hook handle.
            unsafe {
                if UnhookWindowsHookEx(self.0).is_err() {
                    log::warn!("Failed to unhook keyboard hook");
                }
            }
        }
    }
}

/// Low-level keyboard hook callback procedure.
///
/// This function is called by Windows for every keyboard event when the hook is installed.
/// It intercepts `VK_BRIGHTNESS_UP` and `VK_BRIGHTNESS_DOWN` keys, sends brightness
/// adjustment messages, and suppresses the native Windows brightness OSD.
///
/// # Safety
///
/// This is a Windows callback function. It must:
/// - Be called only by Windows as part of the hook chain
/// - Have a valid `KBDLLHOOKSTRUCT` pointer in `lparam` when `code == HC_ACTION`
unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // If code < 0, we must pass to next hook without processing
    if code < 0 {
        return unsafe { CallNextHookEx(HHOOK::default(), code, wparam, lparam) };
    }

    // Only process if code indicates we should (HC_ACTION = 0)
    if code == HC_ACTION {
        // Check for key-down events only (ignore key-up to avoid double-firing)
        // Message types (WM_KEYDOWN, WM_SYSKEYDOWN) are well-defined u32 constants.
        #[allow(clippy::cast_possible_truncation)]
        let msg_type = wparam.0 as u32;
        if msg_type == WM_KEYDOWN || msg_type == WM_SYSKEYDOWN {
            // SAFETY: When code == HC_ACTION, lparam points to a valid KBDLLHOOKSTRUCT
            let kb_struct = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            // Virtual key codes are 16-bit values (0x00-0xFF typical, max 0xFFFF).
            #[allow(clippy::cast_possible_truncation)]
            let vk_code = VIRTUAL_KEY(kb_struct.vkCode as u16);

            // Check if this is a brightness key we want to intercept
            let delta = if vk_code == VK_BRIGHTNESS_UP {
                Some(true) // Increase brightness
            } else if vk_code == VK_BRIGHTNESS_DOWN {
                Some(false) // Decrease brightness
            } else {
                None
            };

            if let Some(is_increase) = delta {
                // Try to send brightness adjustment via thread-local context
                let sent = with_hook_context(|ctx| {
                    let adjustment = if is_increase {
                        ctx.step_percent
                    } else {
                        -ctx.step_percent
                    };

                    // No routine logging here: with file logging at debug, a
                    // log call does mutex-guarded disk I/O, and a slow write
                    // inside an LL hook risks Windows silently removing the
                    // hook on timeout. The keypress is still logged at debug
                    // when the main loop receives the message.
                    if let Err(e) = ctx.sender.send(BrightnessMessage::Adjust {
                        monitor_id: None,
                        delta: adjustment,
                    }) {
                        log::error!(error:% = e; "Failed to send brightness adjustment from hook");
                    }
                });

                // If we successfully processed the key, suppress it (don't pass to Shell)
                if sent.is_some() {
                    return LRESULT(1);
                }
            }
        }
    }

    // Pass unhandled keys to the next hook in the chain
    unsafe { CallNextHookEx(HHOOK::default(), code, wparam, lparam) }
}

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Window procedure for the hotkey message window.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // In Rust 2024, unsafe fn body still requires explicit unsafe block
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Manages global hotkey registration and handling.
pub struct HotkeyManager {
    /// Handle to the invisible message window that receives `WM_HOTKEY` messages.
    hwnd: HWND,
    /// List of currently registered hotkey IDs.
    registered_ids: Vec<i32>,
    /// Channel sender to transmit brightness adjustment events to the main thread.
    sender: Sender<BrightnessMessage>,
    /// Brightness step percentage (1-50).
    step_percent: i8,
    /// Low-level keyboard hook for intercepting brightness keys (optional).
    keyboard_hook: Option<SafeHook>,
}

impl HotkeyManager {
    /// Creates a new `HotkeyManager`.
    ///
    /// # Errors
    ///
    /// Returns a `BrightnessError::WindowsApi` if the message window cannot be created.
    ///
    /// # Panics
    ///
    /// Panics if the current process module handle cannot be retrieved.
    pub fn new(sender: Sender<BrightnessMessage>, step_percent: u8) -> Result<Self> {
        let hinstance = unsafe {
            GetModuleHandleW(None).map_err(|e| {
                BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
            })?
        };
        let class_name = w!("DarkBrightHotkeyWindow");

        let wnd_class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };

        unsafe {
            RegisterClassW(&raw const wnd_class);
        }

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name,
                w!("DarkBrightHotkey"),
                WS_POPUP,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                HWND_MESSAGE,
                HMENU::default(),
                hinstance,
                None,
            )
        };

        if hwnd.0 == 0 {
            return Err(last_error_as_brightness_error("CreateWindowExW"));
        }

        Ok(Self {
            hwnd,
            registered_ids: Vec::new(),
            sender,
            step_percent: step_percent.cast_signed(),
            keyboard_hook: None,
        })
    }

    /// Registers a global hotkey.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for the hotkey.
    /// * `modifiers` - Modifier keys (Ctrl, Alt, Shift, Win).
    /// * `vk` - Virtual key code.
    ///
    /// # Errors
    ///
    /// Returns `BrightnessError::WindowsApi` if registration fails.
    pub fn register_hotkey(
        &mut self,
        id: i32,
        modifiers: HOT_KEY_MODIFIERS,
        vk: VIRTUAL_KEY,
    ) -> Result<()> {
        unsafe {
            RegisterHotKey(self.hwnd, id, modifiers, u32::from(vk.0)).map_err(|e| {
                BrightnessError::windows_api("RegisterHotKey", e.code().0.cast_unsigned())
            })?;
        }
        log::debug!(hotkey_id = id; "Registered hotkey");
        self.registered_ids.push(id);
        Ok(())
    }

    /// Unregisters a hotkey.
    ///
    /// # Errors
    ///
    /// Returns `BrightnessError::WindowsApi` if unregistration fails.
    pub fn unregister_hotkey(&mut self, id: i32) -> Result<()> {
        unsafe {
            UnregisterHotKey(self.hwnd, id).map_err(|e| {
                BrightnessError::windows_api("UnregisterHotKey", e.code().0.cast_unsigned())
            })?;
        }
        self.registered_ids.retain(|&x| x != id);
        Ok(())
    }

    /// Installs a low-level keyboard hook to intercept dedicated brightness keys.
    ///
    /// This hook captures `VK_BRIGHTNESS_UP` and `VK_BRIGHTNESS_DOWN` before the
    /// Windows Shell processes them, suppressing the native brightness OSD.
    ///
    /// The hook must be installed on the same thread that runs the message loop,
    /// as Windows delivers hook callbacks to the thread that installed the hook.
    ///
    /// # Errors
    ///
    /// Returns `BrightnessError::WindowsApi` if `SetWindowsHookExW` fails.
    pub fn install_brightness_hook(&mut self) -> Result<()> {
        // Initialize thread-local context for the hook callback
        set_hook_context(self.sender.clone(), self.step_percent);

        // Install the low-level keyboard hook
        // SAFETY: We pass a valid callback function. The hook handle will be
        // stored in SafeHook which ensures cleanup on drop.
        let hook = unsafe {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), None, 0).map_err(
                |e| BrightnessError::windows_api("SetWindowsHookExW", e.code().0.cast_unsigned()),
            )?
        };

        // SAFETY: The hook handle is valid as SetWindowsHookExW succeeded.
        self.keyboard_hook = Some(unsafe { SafeHook::new(hook) });

        log::debug!("Low-level keyboard hook installed for brightness keys");
        Ok(())
    }

    /// Runs the message loop to process hotkey events.
    ///
    /// This method blocks until the message loop is terminated (e.g. by `WM_QUIT`).
    pub fn run_message_loop(&self) {
        let mut msg = MSG::default();
        unsafe {
            // GetMessageW returns:
            // > 0: Message retrieved
            // 0: WM_QUIT received
            // -1: Error
            //
            // Both exit paths are logged: when this loop ends, the hotkey
            // thread dies and the app loses its primary input, so the exit
            // must never be silent. The main loop's liveness check detects
            // the dead thread and attempts a restart.
            loop {
                let ret = GetMessageW(&raw mut msg, HWND::default(), 0, 0).0;
                if ret == 0 {
                    log::info!("Hotkey message loop received WM_QUIT, exiting");
                    break;
                }
                if ret == -1 {
                    log::error!(
                        error:% = last_error_as_brightness_error("GetMessageW");
                        "Hotkey message loop failed, exiting"
                    );
                    break;
                }
                if msg.message == WM_HOTKEY {
                    // Safety: WPARAM for WM_HOTKEY is the identifier of the hotkey.
                    // Cast is safe as we only register small positive IDs (1-4).
                    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    let id = msg.wParam.0 as i32;
                    log::debug!(hotkey_id = id; "Received WM_HOTKEY");

                    let delta = match id {
                        BRIGHTNESS_UP_ID | BRIGHTNESS_UP_ALT_ID => self.step_percent,
                        BRIGHTNESS_DOWN_ID | BRIGHTNESS_DOWN_ALT_ID => -self.step_percent,
                        _ => 0,
                    };

                    if delta != 0 {
                        log::debug!(delta = delta; "Sending brightness adjustment");
                        let _ = self.sender.send(BrightnessMessage::Adjust {
                            monitor_id: None, // None = monitor under cursor
                            delta,
                        });
                    }
                }

                let _ = TranslateMessage(&raw const msg);
                let _ = DispatchMessageW(&raw const msg);
            }
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        for &id in &self.registered_ids {
            unsafe {
                let _ = UnregisterHotKey(self.hwnd, id);
            }
        }
        if self.hwnd.0 != 0 {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// A parsed hotkey consisting of modifiers and a virtual key code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedHotkey {
    /// Modifier keys (Ctrl, Alt, Shift, Win).
    pub modifiers: HOT_KEY_MODIFIERS,
    /// Virtual key code for the main key.
    pub vk_code: VIRTUAL_KEY,
}

impl ParsedHotkey {
    /// Creates a new parsed hotkey.
    #[must_use]
    pub const fn new(modifiers: HOT_KEY_MODIFIERS, vk_code: VIRTUAL_KEY) -> Self {
        Self { modifiers, vk_code }
    }
}

impl std::fmt::Display for ParsedHotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();

        if self.modifiers.contains(MOD_CONTROL) {
            parts.push("Ctrl");
        }
        if self.modifiers.contains(MOD_ALT) {
            parts.push("Alt");
        }
        if self.modifiers.contains(MOD_SHIFT) {
            parts.push("Shift");
        }
        if self.modifiers.contains(MOD_WIN) {
            parts.push("Win");
        }

        // Find key name from VK code
        let key_name = VK_TO_NAME
            .iter()
            .find(|(_, vk)| *vk == self.vk_code)
            .map_or("Unknown", |(name, _)| *name);

        parts.push(key_name);
        write!(f, "{}", parts.join("+"))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Key Mappings
// ─────────────────────────────────────────────────────────────────────────────

/// Mapping from modifier names (lowercase) to their `HOT_KEY_MODIFIERS` values.
static MODIFIER_MAP: LazyLock<HashMap<&'static str, HOT_KEY_MODIFIERS>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("ctrl", MOD_CONTROL);
    m.insert("control", MOD_CONTROL);
    m.insert("alt", MOD_ALT);
    m.insert("shift", MOD_SHIFT);
    m.insert("win", MOD_WIN);
    m.insert("windows", MOD_WIN);
    m.insert("super", MOD_WIN);
    m
});

/// Mapping from key names (lowercase) to their virtual key codes.
static KEY_MAP: LazyLock<HashMap<&'static str, VIRTUAL_KEY>> = LazyLock::new(|| {
    let mut m = HashMap::new();

    // Arrow keys
    m.insert("up", VK_UP);
    m.insert("down", VK_DOWN);
    m.insert("left", VK_LEFT);
    m.insert("right", VK_RIGHT);

    // Function keys
    m.insert("f1", VK_F1);
    m.insert("f2", VK_F2);
    m.insert("f3", VK_F3);
    m.insert("f4", VK_F4);
    m.insert("f5", VK_F5);
    m.insert("f6", VK_F6);
    m.insert("f7", VK_F7);
    m.insert("f8", VK_F8);
    m.insert("f9", VK_F9);
    m.insert("f10", VK_F10);
    m.insert("f11", VK_F11);
    m.insert("f12", VK_F12);

    // Navigation keys
    m.insert("pageup", VK_PRIOR);
    m.insert("pagedown", VK_NEXT);
    m.insert("home", VK_HOME);
    m.insert("end", VK_END);
    m.insert("insert", VK_INSERT);
    m.insert("delete", VK_DELETE);
    m.insert("del", VK_DELETE);

    // Common keys
    m.insert("space", VK_SPACE);
    m.insert("tab", VK_TAB);
    m.insert("enter", VK_RETURN);
    m.insert("return", VK_RETURN);
    m.insert("escape", VK_ESCAPE);
    m.insert("esc", VK_ESCAPE);
    m.insert("backspace", VK_BACK);

    // Symbols
    m.insert("plus", VK_OEM_PLUS);
    m.insert("minus", VK_OEM_MINUS);

    // Note: Leak is acceptable as this is done once in LazyLock for the lifetime of the process.
    // Letters A-Z (VK codes are same as ASCII uppercase)
    for c in 'a'..='z' {
        let key_name: &'static str = Box::leak(c.to_string().into_boxed_str());
        m.insert(key_name, VIRTUAL_KEY(c.to_ascii_uppercase() as u16));
    }

    // Numbers 0-9 (VK codes are same as ASCII)
    for c in '0'..='9' {
        let key_name: &'static str = Box::leak(c.to_string().into_boxed_str());
        m.insert(key_name, VIRTUAL_KEY(c as u16));
    }

    m
});

/// Reverse mapping from VK codes to display names (for `Display` impl).
static VK_TO_NAME: LazyLock<Vec<(&'static str, VIRTUAL_KEY)>> = LazyLock::new(|| {
    vec![
        ("Up", VK_UP),
        ("Down", VK_DOWN),
        ("Left", VK_LEFT),
        ("Right", VK_RIGHT),
        ("F1", VK_F1),
        ("F2", VK_F2),
        ("F3", VK_F3),
        ("F4", VK_F4),
        ("F5", VK_F5),
        ("F6", VK_F6),
        ("F7", VK_F7),
        ("F8", VK_F8),
        ("F9", VK_F9),
        ("F10", VK_F10),
        ("F11", VK_F11),
        ("F12", VK_F12),
        ("PageUp", VK_PRIOR),
        ("PageDown", VK_NEXT),
        ("Home", VK_HOME),
        ("End", VK_END),
        ("Insert", VK_INSERT),
        ("Delete", VK_DELETE),
        ("Space", VK_SPACE),
        ("Tab", VK_TAB),
        ("Enter", VK_RETURN),
        ("Escape", VK_ESCAPE),
        ("Backspace", VK_BACK),
        ("Plus", VK_OEM_PLUS),
        ("Minus", VK_OEM_MINUS),
    ]
});

// ─────────────────────────────────────────────────────────────────────────────
// Parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Parses a hotkey string into modifiers and a virtual key code.
///
/// # Format
///
/// Hotkey strings are `+`-delimited, case-insensitive combinations of
/// modifiers and a key name.
///
/// ## Modifiers
/// - `Ctrl` or `Control`
/// - `Alt`
/// - `Shift`
/// - `Win`, `Windows`, or `Super`
///
/// ## Keys
/// - Arrow keys: `Up`, `Down`, `Left`, `Right`
/// - Function keys: `F1` - `F12`
/// - Navigation: `PageUp`, `PageDown`, `Home`, `End`, `Insert`, `Delete`
/// - Common: `Space`, `Tab`, `Enter`, `Escape`, `Backspace`
/// - Symbols: `Plus` (for `+`), `Minus` (for `-`)
/// - Letters: `A` - `Z`
/// - Numbers: `0` - `9`
///
/// # Examples
///
/// ```
/// use darkbright_helper::platform::windows::hotkey::parse_hotkey;
///
/// let hotkey = parse_hotkey("Ctrl+Shift+Up").unwrap();
/// let hotkey2 = parse_hotkey("alt+f1").unwrap(); // case insensitive
/// ```
///
/// # Errors
///
/// Returns `BrightnessError::ConfigInvalid` if:
/// - The string is empty or contains only modifiers.
/// - An unknown key name is encountered.
/// - No valid key (only modifiers) is specified.
pub fn parse_hotkey(s: &str) -> Result<ParsedHotkey> {
    let s = s.trim();

    if s.is_empty() {
        return Err(BrightnessError::config_invalid(
            "hotkey",
            "hotkey string is empty",
        ));
    }

    let mut modifiers = HOT_KEY_MODIFIERS::default();
    let mut vk_code: Option<VIRTUAL_KEY> = None;

    for part in s.split('+') {
        let part = part.trim().to_lowercase();

        if part.is_empty() {
            continue;
        }

        // Check if it's a modifier
        if let Some(&modifier) = MODIFIER_MAP.get(part.as_str()) {
            modifiers |= modifier;
            continue;
        }

        // Check if it's a key
        if let Some(&vk) = KEY_MAP.get(part.as_str()) {
            if vk_code.is_some() {
                return Err(BrightnessError::config_invalid(
                    "hotkey",
                    format!("multiple keys specified in '{s}', only one allowed"),
                ));
            }
            vk_code = Some(vk);
            continue;
        }

        // Unknown part
        return Err(BrightnessError::config_invalid(
            "hotkey",
            format!("unknown key or modifier: '{part}'"),
        ));
    }

    let vk_code = vk_code.ok_or_else(|| {
        BrightnessError::config_invalid("hotkey", format!("no key specified in '{s}'"))
    })?;

    Ok(ParsedHotkey::new(modifiers, vk_code))
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ctrl_shift_up() {
        let hotkey = parse_hotkey("Ctrl+Shift+Up").unwrap();
        assert!(hotkey.modifiers.contains(MOD_CONTROL));
        assert!(hotkey.modifiers.contains(MOD_SHIFT));
        assert!(!hotkey.modifiers.contains(MOD_ALT));
        assert_eq!(hotkey.vk_code, VK_UP);
    }

    #[test]
    fn test_parse_case_insensitive() {
        let hotkey = parse_hotkey("ctrl+shift+up").unwrap();
        assert!(hotkey.modifiers.contains(MOD_CONTROL));
        assert!(hotkey.modifiers.contains(MOD_SHIFT));
        assert_eq!(hotkey.vk_code, VK_UP);

        let hotkey2 = parse_hotkey("CTRL+SHIFT+UP").unwrap();
        assert_eq!(hotkey, hotkey2);
    }

    #[test]
    fn test_parse_alt_f1() {
        let hotkey = parse_hotkey("Alt+F1").unwrap();
        assert!(hotkey.modifiers.contains(MOD_ALT));
        assert!(!hotkey.modifiers.contains(MOD_CONTROL));
        assert_eq!(hotkey.vk_code, VK_F1);
    }

    #[test]
    fn test_parse_single_key() {
        let hotkey = parse_hotkey("F5").unwrap();
        assert_eq!(hotkey.modifiers, HOT_KEY_MODIFIERS::default());
        assert_eq!(hotkey.vk_code, VK_F5);
    }

    #[test]
    fn test_parse_with_spaces() {
        let hotkey = parse_hotkey("Ctrl + Shift + Down").unwrap();
        assert!(hotkey.modifiers.contains(MOD_CONTROL));
        assert!(hotkey.modifiers.contains(MOD_SHIFT));
        assert_eq!(hotkey.vk_code, VK_DOWN);
    }

    #[test]
    fn test_parse_letter() {
        let hotkey = parse_hotkey("Ctrl+A").unwrap();
        assert!(hotkey.modifiers.contains(MOD_CONTROL));
        assert_eq!(hotkey.vk_code, VIRTUAL_KEY(0x41)); // 'A'
    }

    #[test]
    fn test_parse_number() {
        let hotkey = parse_hotkey("Alt+5").unwrap();
        assert!(hotkey.modifiers.contains(MOD_ALT));
        assert_eq!(hotkey.vk_code, VIRTUAL_KEY(0x35)); // '5'
    }

    #[test]
    fn test_parse_empty_fails() {
        assert!(parse_hotkey("").is_err());
    }

    #[test]
    fn test_parse_only_modifiers_fails() {
        assert!(parse_hotkey("Ctrl+Shift").is_err());
    }

    #[test]
    fn test_parse_unknown_key_fails() {
        assert!(parse_hotkey("Ctrl+UnknownKey").is_err());
    }

    #[test]
    fn test_parse_multiple_keys_fails() {
        assert!(parse_hotkey("Ctrl+A+B").is_err());
    }

    #[test]
    fn test_display() {
        let hotkey = parse_hotkey("Ctrl+Shift+Up").unwrap();
        let display = hotkey.to_string();
        assert!(display.contains("Ctrl"));
        assert!(display.contains("Shift"));
        assert!(display.contains("Up"));
    }

    #[test]
    fn test_parse_win_modifier() {
        let hotkey = parse_hotkey("Win+E").unwrap();
        assert!(hotkey.modifiers.contains(MOD_WIN));
    }

    #[test]
    fn test_parse_plus_key() {
        let hotkey = parse_hotkey("Ctrl+Plus").unwrap();
        assert_eq!(hotkey.vk_code, VK_OEM_PLUS);
    }
}
