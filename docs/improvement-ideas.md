# Improvement Ideas

This document tracks potential future enhancements for the Brightness Control Tool.

---

## Planned Features

### Linux Support
- X11 support via `XGrabKey` for hotkeys and `XComposite` for overlay
- Wayland support via layer-shell protocol for overlay and portal APIs for hotkeys
- DDC/CI via `/dev/i2c-*` devices or `ddcutil` integration

### System Tray Icon
- Minimize to tray for background operation
- Quick access to brightness adjustment via tray menu
- Visual indicator of current brightness level

### Settings GUI
- Graphical configuration interface
- Real-time hotkey capture for easy customization
- Per-monitor settings management

### OSD Animations
- Fade in/out transitions for smoother appearance
- Animated progress bar changes
- Configurable animation duration

---

## Technical Improvements

### DPI Scaling
Currently, the application declares per-monitor DPI awareness to prevent bitmap stretching, but does not scale UI elements based on the actual DPI. This means at 125% or 150% scaling, the OSD will appear smaller than intended (physically correct pixels, but visually undersized). A future enhancement should query the monitor's DPI via `GetDpiForMonitor` and scale all pixel values (window size, font size, margins) proportionally.

---

## Potential Enhancements

- Ambient light sensor integration
- Brightness curves/gamma adjustment
- Per-application brightness profiles
- Brightness schedules (time-based dimming)
