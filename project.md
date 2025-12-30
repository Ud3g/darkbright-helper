# Brightness Control Tool

## Overview
A hotkey-driven brightness adjustment tool for Windows (with future Linux support).

## Language
Rust (2021 edition) — chosen for cross-platform portability, low resource usage, and native Windows API integration (`windows` crate).

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

## Future
- Linux support (X11/Wayland)
- System tray icon & Settings GUI
- OSD animations (fade in/out transitions)
