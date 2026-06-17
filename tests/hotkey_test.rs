//! Integration tests for hotkey parsing and registration.
//!
//! These tests verify:
//! - Hotkey string parsing (`parse_hotkey`)
//! - `ParsedHotkey` structure and display formatting
//! - `HotkeyManager` construction (without actual registration in CI)

use darkbright_helper::platform::windows::hotkey::{
    BRIGHTNESS_DOWN_ID, BRIGHTNESS_UP_ID, ParsedHotkey, parse_hotkey,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, VIRTUAL_KEY, VK_DOWN, VK_END,
    VK_F1, VK_F12, VK_HOME, VK_LEFT, VK_NEXT, VK_OEM_MINUS, VK_OEM_PLUS, VK_PRIOR, VK_RETURN,
    VK_RIGHT, VK_SPACE, VK_UP,
};

// ─────────────────────────────────────────────────────────────────────────────
// Parsing Tests - Basic Combinations
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_parse_default_brightness_up() {
    // Test the default hotkey from config
    let hotkey = parse_hotkey("Ctrl+Shift+Up").expect("Should parse default brightness up");
    assert!(hotkey.modifiers.contains(MOD_CONTROL));
    assert!(hotkey.modifiers.contains(MOD_SHIFT));
    assert!(!hotkey.modifiers.contains(MOD_ALT));
    assert!(!hotkey.modifiers.contains(MOD_WIN));
    assert_eq!(hotkey.vk_code, VK_UP);
}

#[test]
fn test_parse_default_brightness_down() {
    // Test the default hotkey from config
    let hotkey = parse_hotkey("Ctrl+Shift+Down").expect("Should parse default brightness down");
    assert!(hotkey.modifiers.contains(MOD_CONTROL));
    assert!(hotkey.modifiers.contains(MOD_SHIFT));
    assert_eq!(hotkey.vk_code, VK_DOWN);
}

#[test]
fn test_parse_all_modifiers() {
    let hotkey = parse_hotkey("Ctrl+Alt+Shift+Win+F1").expect("Should parse all modifiers");
    assert!(hotkey.modifiers.contains(MOD_CONTROL));
    assert!(hotkey.modifiers.contains(MOD_ALT));
    assert!(hotkey.modifiers.contains(MOD_SHIFT));
    assert!(hotkey.modifiers.contains(MOD_WIN));
    assert_eq!(hotkey.vk_code, VK_F1);
}

#[test]
fn test_parse_single_modifier() {
    let ctrl = parse_hotkey("Ctrl+A").expect("Ctrl+A");
    assert!(ctrl.modifiers.contains(MOD_CONTROL));
    assert!(!ctrl.modifiers.contains(MOD_ALT));
    assert_eq!(ctrl.vk_code, VIRTUAL_KEY(0x41)); // 'A'

    let alt = parse_hotkey("Alt+B").expect("Alt+B");
    assert!(alt.modifiers.contains(MOD_ALT));
    assert!(!alt.modifiers.contains(MOD_CONTROL));
    assert_eq!(alt.vk_code, VIRTUAL_KEY(0x42)); // 'B'

    let shift = parse_hotkey("Shift+C").expect("Shift+C");
    assert!(shift.modifiers.contains(MOD_SHIFT));
    assert_eq!(shift.vk_code, VIRTUAL_KEY(0x43)); // 'C'

    let win = parse_hotkey("Win+D").expect("Win+D");
    assert!(win.modifiers.contains(MOD_WIN));
    assert_eq!(win.vk_code, VIRTUAL_KEY(0x44)); // 'D'
}

#[test]
fn test_parse_no_modifier() {
    let hotkey = parse_hotkey("F12").expect("Should parse F12 without modifiers");
    assert_eq!(hotkey.modifiers, HOT_KEY_MODIFIERS::default());
    assert_eq!(hotkey.vk_code, VK_F12);
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing Tests - Case Insensitivity
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_parse_case_variations() {
    let lower = parse_hotkey("ctrl+shift+up").expect("lowercase");
    let upper = parse_hotkey("CTRL+SHIFT+UP").expect("uppercase");
    let mixed = parse_hotkey("Ctrl+SHIFT+Up").expect("mixed case");

    assert_eq!(lower.modifiers, upper.modifiers);
    assert_eq!(lower.modifiers, mixed.modifiers);
    assert_eq!(lower.vk_code, upper.vk_code);
    assert_eq!(lower.vk_code, mixed.vk_code);
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing Tests - Whitespace Handling
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_parse_with_spaces() {
    let hotkey = parse_hotkey("Ctrl + Shift + Down").expect("Should handle spaces");
    assert!(hotkey.modifiers.contains(MOD_CONTROL));
    assert!(hotkey.modifiers.contains(MOD_SHIFT));
    assert_eq!(hotkey.vk_code, VK_DOWN);
}

#[test]
fn test_parse_with_leading_trailing_spaces() {
    let hotkey = parse_hotkey("  Alt+F1  ").expect("Should trim spaces");
    assert!(hotkey.modifiers.contains(MOD_ALT));
    assert_eq!(hotkey.vk_code, VK_F1);
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing Tests - Modifier Aliases
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_parse_modifier_aliases() {
    // "Control" is alias for "Ctrl"
    let control = parse_hotkey("Control+A").expect("Control alias");
    assert!(control.modifiers.contains(MOD_CONTROL));

    // "Windows" and "Super" are aliases for "Win"
    let windows = parse_hotkey("Windows+E").expect("Windows alias");
    assert!(windows.modifiers.contains(MOD_WIN));

    let super_key = parse_hotkey("Super+E").expect("Super alias");
    assert!(super_key.modifiers.contains(MOD_WIN));
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing Tests - All Key Types
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_parse_arrow_keys() {
    assert_eq!(parse_hotkey("Up").unwrap().vk_code, VK_UP);
    assert_eq!(parse_hotkey("Down").unwrap().vk_code, VK_DOWN);
    assert_eq!(parse_hotkey("Left").unwrap().vk_code, VK_LEFT);
    assert_eq!(parse_hotkey("Right").unwrap().vk_code, VK_RIGHT);
}

#[test]
fn test_parse_function_keys() {
    for i in 1..=12 {
        let key_str = format!("F{i}");
        let hotkey = parse_hotkey(&key_str).unwrap_or_else(|_| panic!("Should parse {key_str}"));
        // F1 = 0x70, F2 = 0x71, etc.
        assert_eq!(hotkey.vk_code, VIRTUAL_KEY(0x6F + i));
    }
}

#[test]
fn test_parse_navigation_keys() {
    assert_eq!(parse_hotkey("PageUp").unwrap().vk_code, VK_PRIOR);
    assert_eq!(parse_hotkey("PageDown").unwrap().vk_code, VK_NEXT);
    assert_eq!(parse_hotkey("Home").unwrap().vk_code, VK_HOME);
    assert_eq!(parse_hotkey("End").unwrap().vk_code, VK_END);
}

#[test]
fn test_parse_common_keys() {
    assert_eq!(parse_hotkey("Space").unwrap().vk_code, VK_SPACE);
    assert_eq!(parse_hotkey("Enter").unwrap().vk_code, VK_RETURN);
    assert_eq!(parse_hotkey("Return").unwrap().vk_code, VK_RETURN); // alias
}

#[test]
fn test_parse_symbol_keys() {
    // "Plus" for the + key (since + is the delimiter)
    assert_eq!(parse_hotkey("Ctrl+Plus").unwrap().vk_code, VK_OEM_PLUS);
    assert_eq!(parse_hotkey("Ctrl+Minus").unwrap().vk_code, VK_OEM_MINUS);
}

#[test]
fn test_parse_letters() {
    for c in 'a'..='z' {
        let hotkey = parse_hotkey(&c.to_string()).expect("Should parse letter");
        assert_eq!(hotkey.vk_code, VIRTUAL_KEY(c.to_ascii_uppercase() as u16));
    }
}

#[test]
fn test_parse_numbers() {
    for c in '0'..='9' {
        let hotkey = parse_hotkey(&c.to_string()).expect("Should parse number");
        assert_eq!(hotkey.vk_code, VIRTUAL_KEY(c as u16));
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Parsing Tests - Error Cases
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_parse_empty_string_fails() {
    let result = parse_hotkey("");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("empty"), "Error should mention empty: {err}");
}

#[test]
fn test_parse_only_whitespace_fails() {
    let result = parse_hotkey("   ");
    assert!(result.is_err());
}

#[test]
fn test_parse_only_modifiers_fails() {
    let result = parse_hotkey("Ctrl+Shift");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("no key"), "Error should mention no key: {err}");
}

#[test]
fn test_parse_unknown_key_fails() {
    let result = parse_hotkey("Ctrl+UnknownKey");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("unknown"),
        "Error should mention unknown: {err}"
    );
}

#[test]
fn test_parse_multiple_keys_fails() {
    let result = parse_hotkey("Ctrl+A+B");
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("multiple"),
        "Error should mention multiple keys: {err}"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// ParsedHotkey Tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_parsed_hotkey_new() {
    let hotkey = ParsedHotkey::new(MOD_CONTROL | MOD_SHIFT, VK_UP);
    assert!(hotkey.modifiers.contains(MOD_CONTROL));
    assert!(hotkey.modifiers.contains(MOD_SHIFT));
    assert_eq!(hotkey.vk_code, VK_UP);
}

#[test]
fn test_parsed_hotkey_equality() {
    let a = parse_hotkey("Ctrl+Shift+Up").unwrap();
    let b = parse_hotkey("Shift+Ctrl+Up").unwrap(); // Different order, same result
    assert_eq!(a, b);
}

#[test]
fn test_parsed_hotkey_display() {
    let hotkey = parse_hotkey("Ctrl+Shift+Up").unwrap();
    let display = hotkey.to_string();

    // Display should contain all components
    assert!(display.contains("Ctrl"), "Display missing Ctrl: {display}");
    assert!(
        display.contains("Shift"),
        "Display missing Shift: {display}"
    );
    assert!(display.contains("Up"), "Display missing Up: {display}");
}

#[test]
fn test_parsed_hotkey_display_no_modifiers() {
    let hotkey = parse_hotkey("F5").unwrap();
    let display = hotkey.to_string();
    // Should just be the key name without any modifiers
    assert!(!display.contains("Ctrl"));
    assert!(!display.contains("Alt"));
    assert!(!display.contains("Shift"));
    assert!(!display.contains("Win"));
}

// ─────────────────────────────────────────────────────────────────────────────
// Constants Tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_hotkey_id_constants() {
    // Verify the hotkey IDs are distinct
    assert_ne!(BRIGHTNESS_UP_ID, BRIGHTNESS_DOWN_ID);
    const { assert!(BRIGHTNESS_UP_ID > 0) };
    const { assert!(BRIGHTNESS_DOWN_ID > 0) };
}

// ─────────────────────────────────────────────────────────────────────────────
// Config Integration Tests
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_parse_config_default_hotkeys() {
    use darkbright_helper::core::config::{DEFAULT_HOTKEY_DOWN, DEFAULT_HOTKEY_UP};

    // Verify the default config hotkeys are valid
    let up = parse_hotkey(DEFAULT_HOTKEY_UP).expect("Default hotkey_up should be valid");
    let down = parse_hotkey(DEFAULT_HOTKEY_DOWN).expect("Default hotkey_down should be valid");

    // They should be different keys
    assert_ne!(up.vk_code, down.vk_code);

    // Both should have Ctrl+Shift
    assert!(up.modifiers.contains(MOD_CONTROL));
    assert!(up.modifiers.contains(MOD_SHIFT));
    assert!(down.modifiers.contains(MOD_CONTROL));
    assert!(down.modifiers.contains(MOD_SHIFT));
}

#[test]
fn test_parse_config_loaded_hotkeys() {
    use darkbright_helper::core::config::Config;

    let config = Config::default();

    // Parse the hotkeys from config
    let up = parse_hotkey(&config.hotkeys.brightness_up).expect("Config hotkey_up should be valid");
    let down =
        parse_hotkey(&config.hotkeys.brightness_down).expect("Config hotkey_down should be valid");

    assert_eq!(up.vk_code, VK_UP);
    assert_eq!(down.vk_code, VK_DOWN);
}

// ─────────────────────────────────────────────────────────────────────────────
// Edge Cases
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn test_parse_duplicate_modifiers() {
    // Duplicate modifiers should work (idempotent OR operation)
    let hotkey = parse_hotkey("Ctrl+Ctrl+A").expect("Duplicate modifiers should work");
    assert!(hotkey.modifiers.contains(MOD_CONTROL));
    assert_eq!(hotkey.vk_code, VIRTUAL_KEY(0x41));
}

#[test]
fn test_parse_empty_parts_ignored() {
    // Multiple + signs should be handled gracefully
    let hotkey = parse_hotkey("Ctrl++A").expect("Empty parts should be ignored");
    assert!(hotkey.modifiers.contains(MOD_CONTROL));
    assert_eq!(hotkey.vk_code, VIRTUAL_KEY(0x41));
}
