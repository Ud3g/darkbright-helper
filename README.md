# Brightness Control Tool

## Overview
A hotkey-driven brightness adjustment tool for Windows (with future Linux support).

## Language
Rust (2024 edition) — chosen for cross-platform portability, low resource usage, and native Windows API integration (`windows` crate).

## Features

### Brightness Below Hardware Minimum
- Use a black fullscreen overlay with variable opacity
- Allows "dimming" below what the monitor natively supports (except in exclusive fullscreen games)

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

## Configuration

The configuration file is automatically created at:
`%APPDATA%\BrightnessControl\config.json`

### Default Configuration
```json
{
  "version": 1,
  "hotkeys": {
    "brightness_up": "Ctrl+Shift+Up",
    "brightness_down": "Ctrl+Shift+Down"
  },
  "osd": {
    "timeout_ms": 1000,
    "opacity": 0.8
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
- **hotkeys**: Combination strings (e.g., "Alt+F1", "Ctrl+Shift+Plus").
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

## Future
- Linux support (X11/Wayland)
- System tray icon & Settings GUI
- OSD animations (fade in/out transitions)
