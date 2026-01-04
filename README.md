# Brightness Control Tool

## Overview
A hotkey-driven brightness adjustment tool for Windows (with future Linux support).

## Language
Rust (2024 edition) — chosen for cross-platform portability, low resource usage, and native Windows API integration (`windows` crate).

## Features

### Brightness Below Hardware Minimum
- Use a black fullscreen overlay with variable opacity
- Allows "dimming" below what the monitor natively supports
- *Note: Does not cover exclusive fullscreen games or certain Windows system UI (Taskbar, Start Menu).*

### Brightness Above Hardware Minimum
- Communicate with monitor via DDC/CI protocol
- Adjust VCP code `0x10` (brightness) directly

### Multi-Monitor Support
- Per-monitor control
- Hotkeys affect the monitor where the mouse pointer currently resides

### User Interface
- OSD overlay (similar to Windows volume indicator)
- Visual feedback on brightness changes

## Hotkeys
- **Primary**: `Ctrl+Shift+Up` / `Ctrl+Shift+Down` (reliable cross-keyboard default)
- **Secondary**: Dedicated brightness keys (`VK_BRIGHTNESS_UP/DOWN`) registered opportunistically
- Fully configurable via `config.json` (in `%APPDATA%`)

## Installation & Build

### Prerequisites
- Rust 1.85+ (2024 edition)
- Windows 10 or 11

### Build
```bash
git clone https://github.com/yourusername/darkbright-helper.git
cd darkbright-helper
cargo build --release
```
The executable will be at `target/release/darkbright-helper.exe`.

### Debug vs Release Builds

| Build Type | Command | Console Window | Use Case |
|------------|---------|----------------|----------|
| **Debug** | `cargo build` | ✅ Visible | Development, viewing log output |
| **Release** | `cargo build --release` | ❌ Hidden | End-user distribution |

- **Debug builds** show a console window where log messages appear (controlled by `RUST_LOG` environment variable)
- **Release builds** use `windows_subsystem = "windows"` to hide the console, providing a clean GUI-only experience
- To debug release-specific issues, use `RUST_LOG=debug cargo run --release` (note: output goes to debug logging, not console)

## Configuration

The configuration file is automatically created at:
`%APPDATA%\BrightnessControl\config.json`

### Default Configuration
```json
{
  "version": 1,
  "hotkeys": {
    "brightness_up": "Ctrl+Shift+Up",
    "brightness_down": "Ctrl+Shift+Down",
    "intercept_brightness_keys": false
  },
  "osd": {
    "timeout_ms": 1000,
    "opacity": 1.0
  },
  "brightness": {
    "step_percent": 5
  },
  "refresh": {
    "periodic_seconds": 60,
    "inactivity_seconds": 30
  }
}
```

### Options
- **hotkeys.brightness_up/down**: Combination strings (e.g., "Alt+F1", "Ctrl+Shift+Plus").
- **hotkeys.intercept_brightness_keys**: Enable low-level keyboard hook to capture dedicated brightness keys (default: false). See [Brightness Key Limitations](#brightness-key-limitations) for compatibility information.
- **osd.timeout_ms**: How long the OSD remains visible (100-10000 ms).
- **osd.opacity**: OSD window transparency (0.1-1.0).
- **brightness.step_percent**: Amount to change per keypress (1-50%).
- **refresh.periodic_seconds**: Background refresh interval to resync with external changes (0-3600, 0 = disabled).
- **refresh.inactivity_seconds**: Refresh before adjustment if inactive for this duration (0-600, 0 = disabled).

## Usage

1. Run `darkbright-helper.exe`.
2. Use `Ctrl+Shift+Up` to increase brightness.
3. Use `Ctrl+Shift+Down` to decrease brightness.
4. If brightness reaches 0%, continuing to decrease will activate the dimming overlay.

## Brightness Key Limitations

The `intercept_brightness_keys` option attempts to capture dedicated brightness keys (`VK_BRIGHTNESS_UP`/`VK_BRIGHTNESS_DOWN`) using a low-level keyboard hook.

**This feature only works on keyboards that send brightness keys through the standard Windows keyboard input path.**

| Keyboard Type | Works? | Reason |
|---------------|--------|--------|
| Most laptop built-in keyboards | ❌ No | Keys handled by firmware/ACPI before reaching Windows |
| Some external USB keyboards | ✅ Yes | Keys sent as standard HID key codes |
| Gaming keyboards with media keys | ⚠️ Maybe | Depends on manufacturer implementation |

**If your brightness keys don't work with this option:**
- Your keyboard's brightness keys are intercepted by firmware or a dedicated driver before Windows sees them
- The native Windows brightness OSD will still appear regardless of this setting
- Use the primary hotkeys (`Ctrl+Shift+Up/Down`) instead

**Notes:**
- Some antivirus software may flag low-level keyboard hooks as suspicious behavior
- Disabled by default to avoid false positives for users who don't need the feature

## Future

See [docs/improvement-ideas.md](docs/improvement-ideas.md) for planned features and potential enhancements.
