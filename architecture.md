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
| 1-100% | DDC/CI (VCP 0x10) | Native, power-efficient |
| 0% | Black overlay with opacity | Full dimming when hardware minimum is insufficient |

**Crossover Threshold: 0%**
The overlay is only used at 0% brightness. For all values > 0%, DDC/CI is used exclusively. This maximizes hardware control and minimizes GPU usage. Monitors with a high minimum brightness (e.g., DDC only goes down to 20%) will remain at that minimum level when set to 1-20%.

**Mapping Algorithm: Linear**
Logical brightness (0-100%) maps 1:1 to hardware values. While human perception is logarithmic, a linear mapping is chosen for simplicity and predictability: 50% means exactly 50% backlight power.

### 2. Monitor Identification: EDID-Based

Monitors are identified by their EDID data (Manufacturer Name, Model Name, and Serial Number) for cross-platform compatibility:

```rust
pub struct MonitorId {
    pub manufacturer: String,  // 3-character PnP ID
    pub model_name: String,
    pub serial_number: Option<String>,
}
```

**Rationale:**
- Both Windows and Linux expose identical EDID data, allowing shared parsing logic
- Config files are portable across platforms
- For identical monitors without serials, position (topology) is used as a secondary disambiguator

### 3. Multi-Monitor: Mouse Position

Hotkeys affect the monitor containing the mouse cursor:
- `GetCursorPos()` → `MonitorFromPoint()`
- Intuitive, matches user expectations from volume controls
- No configuration required

### 3. Hotkey Strategy: Hybrid Registration

**Primary Default:** `Ctrl+Shift+Up` / `Ctrl+Shift+Down`
- Reliable across all keyboard types
- Unlikely to conflict with common applications
- Two-modifier combo signals "system-level" action

**Secondary (Opportunistic):** Dedicated brightness keys
- `VK_BRIGHTNESS_UP` (`0xE8`) / `VK_BRIGHTNESS_DOWN` (`0xE9`)
- Attempted at startup; silently ignored if registration fails
- Provides native feel on keyboards that emit these codes

**Why not brightness keys as primary?**

| Challenge | Impact |
|-----------|--------|
| Laptop firmware interception | ACPI/EC handles keys before Windows; no event reaches us |
| Desktop keyboard support | Most lack dedicated brightness keys |
| Inconsistent HID translation | Some keyboards send Consumer Control codes Windows doesn't map to VK codes |
| `RegisterHotKey` compatibility | Uncertain support for special VK codes |

**Implementation:**

```rust
// Register primary hotkeys (fail = fatal error)
hotkey_manager.register(BRIGHTNESS_UP_ID, MOD_CTRL | MOD_SHIFT, VK_UP)?;
hotkey_manager.register(BRIGHTNESS_DOWN_ID, MOD_CTRL | MOD_SHIFT, VK_DOWN)?;

// Attempt secondary hotkeys (fail = log and continue)
if let Err(e) = hotkey_manager.register(BRIGHTNESS_UP_ALT_ID, 0, VK_BRIGHTNESS_UP) {
    log::debug!("VK_BRIGHTNESS_UP not available: {}", e);
}
if let Err(e) = hotkey_manager.register(BRIGHTNESS_DOWN_ALT_ID, 0, VK_BRIGHTNESS_DOWN) {
    log::debug!("VK_BRIGHTNESS_DOWN not available: {}", e);
}
```

**Technical Details:**
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
    "timeout_ms": 1000,
    "opacity": 0.8
  }
}
```

**Merge Strategy: Shallow Replace**
User config values override defaults at the top level. When a new field is added to defaults, existing user configs will miss that field until manually updated. This is acceptable for MVP and simplifies implementation.

- Human-readable for debugging
- Portable to Linux

### 5. OSD Design

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **Position** | Bottom-center | Familiar (matches Windows volume OSD); users expect system feedback here |
| **Content** | Progress bar + percentage | Complete information at a glance without clutter |
| **Style** | Semi-transparent minimal | Clean, unobtrusive; visibility on any background |
| **Opacity** | Configurable (`osd.opacity`, default 0.8) | User preference; avoids hardcoded magic numbers |
| **Timeout** | 1000ms after last keystroke | Short enough to not obstruct; resets on repeated adjustments |
| **Animation** | None (MVP) | Simplicity; animations deferred to future release |

### 6. Error Handling: Graceful Degradation

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
