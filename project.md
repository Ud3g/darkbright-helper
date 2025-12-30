# Brightness Control Tool

## Overview
A hotkey-driven brightness adjustment tool for Windows (with future Linux support).

## Language
Rust — chosen for cross-platform portability, low resource usage, and DDC/CI library support (`ddc-hi` crate).

## Features

### Brightness Below Hardware Minimum
- Use a black fullscreen overlay with variable opacity
- Allows "dimming" below what the monitor natively supports

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
TBD — configurable brightness up/down keys

## Future
- Linux support (X11/Wayland)
