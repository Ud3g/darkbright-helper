# Improvement Ideas

This document tracks potential future enhancements for the Brightness Control Tool.

---

## System Tray Icon
- Info row on current brightness / overlay level
- Ability to quit the application

## Settings GUI
- Graphical configuration interface
- Real-time hotkey capture for easy customization
- Per-monitor settings management

## Add option "Always start with the System"

## Internationalization
- Allow the app to work in different languages (technical foundation)
- Add strings for some of the major languages

## OSD Animations
- Fade in/out transitions for smoother appearance
- Animated progress bar changes

## Linux Support
- X11 support via `XGrabKey` for hotkeys and `XComposite` for overlay
- Wayland support via layer-shell protocol for overlay and portal APIs for hotkeys
- DDC/CI via `/dev/i2c-*` devices or `ddcutil` integration

## Other potential Enhancements

- Ambient light sensor integration
- Brightness curves/gamma adjustment
- Per-application brightness profiles
- Brightness schedules (time-based dimming)
