# Architecture & Technical Decisions

## Tech Stack

### Language & Toolchain
- Rust 2021 edition
- MSRV: Latest stable (1.75+)

### Dependencies

| Purpose | Crate | Rationale |
|---------|-------|-----------|
| Windows API | `windows` | Official Microsoft crate, modern API, better ergonomics than `winapi` |
| Error handling | `thiserror` | Derive macros for custom error types, zero runtime cost |
| Serialization | `serde` + `serde_json` | De facto standard, config file handling |
| Logging | `log` + `env_logger` | Standard facade pattern, runtime-configurable |
| Synchronization | `parking_lot` | Faster than std locks, no poisoning |

---

## Core Architecture

### Platform Abstraction

Feature-gated compilation with shared core logic:

```
src/
├── main.rs
├── lib.rs
├── error.rs              # Centralized error types
├── core/                 # Platform-agnostic logic
│   ├── brightness.rs     # Brightness calculations, value mapping
│   ├── config.rs         # Configuration types and loading
│   └── state.rs          # Application state
└── platform/
    ├── mod.rs            # Platform trait definitions
    ├── windows/          # #[cfg(windows)]
    │   ├── mod.rs
    │   ├── ddc.rs        # DDC/CI communication
    │   ├── hotkey.rs     # RegisterHotKey API
    │   ├── overlay.rs    # Dimming overlay window
    │   └── osd.rs        # On-screen display
    └── linux/            # #[cfg(target_os = "linux")] - Future
        └── mod.rs
```

### Threading Model

Dedicated threads for I/O, main thread owns state:

```
┌─────────────────────────────────────────┐
│            Main Thread                  │
│  ┌───────────────────────────────────┐  │
│  │    BrightnessController (owner)   │  │
│  │    - processes adjustment msgs    │  │
│  │    - owns monitor state           │  │
│  │    - updates overlay/OSD          │  │
│  └───────────────────────────────────┘  │
│                 ▲                        │
│                 │ recv()                 │
│                 │                        │
└─────────────────┼────────────────────────┘
                  │ MPSC channel
    ┌─────────────┴─────────────┐
    │                           │
┌───────────┐                   │
│  Hotkey   │ send(Adjustment)  │
│  Thread   │───────────────────┘
└───────────┘
```

**Rationale:**
- No async runtime needed (simpler, smaller binary)
- DDC/CI operations are inherently blocking (~100-500ms, monitor limitation)
- Single owner eliminates data races at compile time
- MPSC channels provide natural backpressure

### State Management

Message-passing with single ownership:

```rust
enum BrightnessMessage {
    Adjust { monitor_id: MonitorId, delta: i8 },
    SetAbsolute { monitor_id: MonitorId, value: u8 },
    Refresh,
    Shutdown,
}

// Hotkey thread sends messages, main thread processes them
```

---

## Key Design Decisions

### 1. Brightness Control Strategy: Hybrid

| Brightness Range | Method | Rationale |
|------------------|--------|-----------|
| Above hardware minimum | DDC/CI (VCP 0x10) | Native, power-efficient |
| Below hardware minimum | Black overlay with opacity | Enables sub-minimum dimming |

The controller decides which method to use based on requested brightness level.

### 2. Multi-Monitor: Mouse Position

Hotkeys affect the monitor containing the mouse cursor:
- `GetCursorPos()` → `MonitorFromPoint()`
- Intuitive, matches user expectations from volume controls
- No configuration required

### 3. Hotkey Registration: Win32 RegisterHotKey

- Dedicated listener thread calls `RegisterHotKey()` API
- Receives `WM_HOTKEY` messages in thread message loop
- **Avoid** `SetWindowsHookEx()` (kernel-mode, antivirus concerns)

### 4. Configuration: JSON in AppData

Location: `%APPDATA%\BrightnessControl\config.json`

```json
{
  "version": 1,
  "hotkeys": {
    "brightness_up": "Ctrl+Shift+Up",
    "brightness_down": "Ctrl+Shift+Down"
  },
  "monitors": {},
  "osd": {
    "timeout_ms": 2000
  }
}
```

- Defaults provided; user config merges on top
- Human-readable for debugging
- Portable to Linux

### 5. Error Handling: Graceful Degradation

```rust
// If DDC fails, fall back to overlay
match ddc_set_brightness(monitor, value) {
    Ok(_) => { /* hardware brightness set */ }
    Err(e) => {
        log::warn!("DDC failed: {}, using overlay", e);
        overlay.set_dimming(value)?;
    }
}
```

---

## MVP Scope (v1.0)

### Included
- [x] Global hotkey brightness adjustment
- [x] DDC/CI hardware brightness control
- [x] Fullscreen overlay for sub-minimum dimming
- [x] Multi-monitor support (mouse position based)
- [x] OSD feedback indicator
- [x] JSON configuration persistence
- [x] Windows 10/11 support

### Deferred (v2.0+)
- [ ] Custom hotkey configuration UI
- [ ] Linux support (X11/Wayland)
- [ ] System tray icon
- [ ] Settings GUI
- [ ] Brightness profiles/schedules

---

## Performance Targets

| Metric | Target | Notes |
|--------|--------|-------|
| Hotkey → OSD visible | < 50ms | Perceived responsiveness |
| OSD frame rate | 60 FPS | Smooth animations |
| Memory (idle) | < 30 MB | Lightweight background app |
| CPU (idle) | < 1% | Minimal resource usage |
| DDC write latency | 100-500ms | Monitor limitation, acceptable |

---

## Future Considerations

### Linux Support
- Platform traits allow adding `linux/` module
- DDC: `/dev/i2c-*` or `ddcutil` integration
- Overlay: X11 (XComposite) or Wayland (layer-shell)
- Hotkeys: X11 (XGrabKey) or portal APIs

### Potential Enhancements
- Ambient light sensor integration
- Brightness curves/gamma adjustment
- Per-application brightness profiles
