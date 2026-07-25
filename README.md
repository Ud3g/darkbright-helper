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
- System tray icon with context menu: live per-monitor status, Usage, Settings, Open Log Folder, Quit — plus warning entries and an icon badge while degraded (e.g. DDC unavailable)

## Hotkeys
- **Primary**: `Ctrl+Shift+Up` / `Ctrl+Shift+Down` (reliable cross-keyboard default)
- **Secondary**: Dedicated brightness keys (`VK_BRIGHTNESS_UP/DOWN`) registered opportunistically
- Fully configurable via `config.json` (in `%APPDATA%`)

## Installation & Build

### Download

Prebuilt Windows binaries are published on
[GitHub Releases](https://github.com/Ud3g/darkbright-helper/releases)
(releases after 0.8.0). Alternatively, build from source as described below.

### Prerequisites
- Rust 1.87+ (2024 edition)
- Windows 10 or 11

### Build
```bash
git clone https://github.com/Ud3g/darkbright-helper.git
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
- To diagnose a release build, enable the opt-in file log — see [Logging](#logging)

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
  "monitors": {},
  "refresh": {
    "periodic_seconds": 60,
    "inactivity_seconds": 30
  },
  "logging": {
    "file_enabled": false,
    "file_level": "info"
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
- **logging.file_enabled**: Opt-in rolling file log for release diagnostics (default: false). See [Logging](#logging).
- **logging.file_level**: Level filter for the file log — `error`/`warn`/`info`/`debug`/`trace` (default: info).

The `monitors` field is reserved for future per-monitor settings and currently ignored.

## Logging

- **Debug builds** log to the visible console; the level is controlled by `RUST_LOG` (default: debug).
- **Release builds** hide the console. For diagnostics, set `logging.file_enabled: true`: every log record is then also written to `%APPDATA%\BrightnessControl\darkbright.log`, reachable via the tray menu's "Open Log Folder". The file is size-capped: at 1 MB it rotates to `darkbright.log.old`, bounding disk use at ~2 MB while recent history survives.
- `logging.file_level` filters the file independently of the console (`RUST_LOG` does not affect the file). At `debug` and below the file contains monitor serial numbers and absolute paths — fine for a deliberately created diagnostic artifact, but worth knowing before sharing it.
- Crashes leave a trace: panics are logged (message + source location) and flushed to the file log before the process dies.

## Usage

1. Run `darkbright-helper.exe`.
2. Use `Ctrl+Shift+Up` to increase brightness.
3. Use `Ctrl+Shift+Down` to decrease brightness.
4. If brightness reaches 0%, continuing to decrease will activate the dimming overlay.
5. Right-click the system tray icon to see per-monitor status and access Usage, Settings, the log folder, or Quit.

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
