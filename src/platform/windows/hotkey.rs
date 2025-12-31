//! Hotkey parsing and registration for Windows.
//!
//! This module provides functionality to parse hotkey strings (e.g., "Ctrl+Shift+Up")
//! and register them as global hotkeys using the Windows `RegisterHotKey` API.

use std::collections::HashMap;
use std::sync::LazyLock;

use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY, VK_BACK, VK_DELETE,
    VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F10, VK_F11, VK_F12, VK_F2, VK_F3, VK_F4, VK_F5, VK_F6,
    VK_F7, VK_F8, VK_F9, VK_HOME, VK_INSERT, VK_LEFT, VK_NEXT, VK_OEM_MINUS, VK_OEM_PLUS, VK_PRIOR,
    VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
};

use crate::error::{BrightnessError, Result};

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

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
            .map(|(name, _)| *name)
            .unwrap_or("Unknown");

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
/// Returns `ConfigInvalid` if:
/// - The string is empty or contains only modifiers
/// - An unknown key name is encountered
/// - No valid key (only modifiers) is specified
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
