# Improvement Ideas

This document tracks potential future enhancements for the Brightness Control Tool.

---

## Linux Support
- X11 support via `XGrabKey` for hotkeys and `XComposite` for overlay
- Wayland support via layer-shell protocol for overlay and portal APIs for hotkeys
- DDC/CI via `/dev/i2c-*` devices or `ddcutil` integration

## System Tray Icon
- Minimize to tray for background operation
- Quick access to brightness adjustment via tray menu
- Visual indicator of current brightness level

## Settings GUI
- Graphical configuration interface
- Real-time hotkey capture for easy customization
- Per-monitor settings management

## OSD Animations
- Fade in/out transitions for smoother appearance
- Animated progress bar changes

## Other potential Enhancements

- Ambient light sensor integration
- Brightness curves/gamma adjustment
- Per-application brightness profiles
- Brightness schedules (time-based dimming)
