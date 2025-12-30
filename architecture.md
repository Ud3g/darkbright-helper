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
  },
  "brightness": {
    "step_percent": 5
  }
}
```

**`monitors` Field:** Reserved for future per-monitor settings (e.g., min/max limits, custom step sizes, DDC disable). Empty `{}` for MVP. Schema will be defined based on real-world user feedback after v1.0.

**Hotkey String Format:**

Hotkeys are specified as `+`-delimited strings, parsed case-insensitively.

| Component | Examples |
|-----------|----------|
| **Modifiers** | `Ctrl`, `Alt`, `Shift`, `Win` |
| **Arrows** | `Up`, `Down`, `Left`, `Right` |
| **Function keys** | `F1` - `F12` |
| **Navigation** | `PageUp`, `PageDown`, `Home`, `End`, `Insert`, `Delete` |
| **Common** | `Space`, `Tab`, `Enter`, `Escape`, `Backspace` |
| **Symbols** | `Plus` (for `+`), `Minus` (for `-`) |

Examples:
- `Ctrl+Shift+Up` — primary brightness up
- `alt+f1` — case doesn't matter
- `Ctrl+Plus` — Ctrl and the `+` key

**Merge Strategy: Shallow Replace**
User config values override defaults at the top level. When a new field is added to defaults, existing user configs will miss that field until manually updated. This is acceptable for MVP and simplifies implementation.

**Invalid Config Handling: Error and Use Default**

When a config value is invalid (e.g., `step_percent: 999`, `timeout_ms: -5`):
- Log an error describing the invalid value and which default is being used
- Use the default value for that field
- Continue startup normally

| Field | Valid Range | Default |
|-------|-------------|---------|
| `hotkeys.*` | Valid hotkey string (see format above) | `Ctrl+Shift+Up` / `Down` |
| `osd.timeout_ms` | 100 - 10000 | 1000 |
| `osd.opacity` | 0.1 - 1.0 | 0.8 |
| `brightness.step_percent` | 1 - 50 | 5 |

Example log output for invalid config:
```
[ERROR] Invalid config: brightness.step_percent=999 exceeds maximum 50, using default 5
[ERROR] Invalid config: osd.timeout_ms=-5 below minimum 100, using default 1000
```

- Human-readable for debugging
- Portable to Linux

### 5. Brightness Step Size

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **Default** | 5% per keypress | Fine-grained control; 20 steps across full range |
| **Configurable** | `brightness.step_percent` in config | Users can increase for faster adjustment (e.g., 10%) |
| **Valid range** | 1-50% | Prevents unusable extremes |

**Behavior:**
- Each hotkey press adjusts brightness by `step_percent`
- Clamped to 0-100% bounds
- Repeated rapid presses accumulate (no debouncing — user intent is clear)

### 6. OSD Design

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **Position** | Bottom-center | Familiar (matches Windows volume OSD); users expect system feedback here |
| **Style** | Semi-transparent minimal | Clean, unobtrusive; visibility on any background |
| **Opacity** | Configurable (`osd.opacity`, default 0.8) | User preference; avoids hardcoded magic numbers |
| **Timeout** | 1000ms after last keystroke | Short enough to not obstruct; resets on repeated adjustments |
| **Animation** | None (MVP) | Simplicity; animations deferred to future release |
| **Error state** | Red-tinted bar + message | Clear feedback when DDC fails; see below |

**Two-Bar Layout:**

The OSD displays one or two bars depending on overlay state:

| State | Display |
|-------|---------|
| Hardware brightness only (overlay inactive) | Single bar with 🔆 icon |
| Hardware at 0%, overlay active | Two bars: 🔆 at 0% + 🕶 showing overlay level |

```
Hardware brightness at 50%, overlay inactive:
┌────────────────────────────────────┐
│  🔆 ████████████░░░░░░░░░░  50%   │
└────────────────────────────────────┘

Hardware at 0%, overlay at 60%:
┌────────────────────────────────────┐
│  🔆 ░░░░░░░░░░░░░░░░░░░░░░   0%   │
│  🕶 ████████████████░░░░░░  60%   │
└────────────────────────────────────┘
```

**Symbols:**
- 🔆 (U+1F506 High Brightness) — hardware/DDC brightness
- 🕶 (U+1F576 Sunglasses) — dimming overlay

**Behavior:**
1. Brightness down: hardware decreases until 0%
2. At hardware 0%, continued presses increase overlay opacity (second bar appears)
3. Brightness up: overlay decreases until 0% (second bar disappears), then hardware increases
4. Full hardware resolution (0-100%) is preserved

**Error Indicator:**

When DDC communication fails after all retries:
- Progress bar changes to red/error tint
- Message displayed: "Failed to adjust brightness"
- Percentage reverts to last confirmed value
- OSD timeout remains unchanged (1000ms)

### 7. Dimming Overlay Implementation

**Method:** GDI window with `SetLayeredWindowAttributes`

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **Rendering** | GDI (`SetLayeredWindowAttributes`) | Simplest implementation for solid black rectangle; sufficient for MVP |
| **Window flags** | `WS_EX_LAYERED \| WS_EX_TRANSPARENT \| WS_EX_TOOLWINDOW` | Click-through, no taskbar entry |
| **Z-order** | `HWND_TOPMOST` | Stays above all normal windows |
| **Multi-monitor** | One overlay window per monitor | Independent opacity control per display |
| **Opacity range** | 0-100% (gradual) | Full range adjustable via continued hotkey presses below hardware 0% |

**Platform Abstraction Trait:**

```rust
pub trait DimmingOverlay {
    fn set_opacity(&mut self, opacity: f32) -> Result<()>;  // 0.0 = invisible, 1.0 = fully black
    fn show(&mut self) -> Result<()>;
    fn hide(&mut self) -> Result<()>;
}
```

This trait enables future Linux implementations (X11/Wayland) without changing core logic.

**Brightness ↔ Overlay Mapping:**

Hardware brightness and overlay are separate, controlled sequentially:

```
User presses "brightness down" repeatedly:
  100% hw → 95% hw → ... → 5% hw → 0% hw → 0% hw + 5% overlay → ... → 0% hw + 100% overlay

User presses "brightness up" repeatedly:
  0% hw + 100% overlay → 0% hw + 95% overlay → ... → 0% hw + 0% overlay → 5% hw → ... → 100% hw
```

This preserves full 0-100% hardware resolution while allowing additional dimming via overlay.

**Coverage by Scenario:**

| Scenario | Overlay Works? | Notes |
|----------|----------------|-------|
| Desktop / taskbar | ✅ Yes | `HWND_TOPMOST` covers all system UI |
| Borderless windowed games | ✅ Yes | Rendered through compositor |
| Fullscreen exclusive games | ❌ No | Game bypasses DWM; writes directly to framebuffer |

**Known Limitation:** Fullscreen exclusive mode games bypass the Windows compositor entirely. The overlay cannot dim these applications. Users must either:
- Use the monitor's DDC minimum brightness (1%)
- Switch the game to borderless windowed mode

This is acceptable because:
- DDC/CI hardware brightness (1-100%) works in all scenarios including fullscreen exclusive
- Overlay is only used for the "bonus" sub-0% dimming feature
- Most modern games default to borderless windowed mode

### 8. Logging Strategy

**Level:** Configurable via `RUST_LOG` environment variable (standard `env_logger` behavior)

| Level | Events Logged |
|-------|---------------|
| **Error** | All failures including recoverable (DDC errors, config parse failures, overlay creation failed) |
| **Warn** | Fallback triggers, performance issues, deprecated config fields, monitor disconnected |
| **Info** | Startup/shutdown, config loaded, monitors detected, hotkeys registered |
| **Debug** | Each brightness change, hotkey received, OSD show/hide, DDC command sent |
| **Trace** | DDC raw I²C bytes, window messages, config file contents |

**Default Level:** `Info` for release builds, `Debug` for debug builds.

**Examples:**
```
[INFO ] BrightnessControl started, version 1.0.0
[INFO ] Loaded config from C:\Users\...\AppData\Roaming\BrightnessControl\config.json
[INFO ] Detected 2 monitors: ["DELL U2722D (SN:ABC123)", "LG 27UK850 (SN:XYZ789)"]
[INFO ] Registered hotkeys: Ctrl+Shift+Up, Ctrl+Shift+Down
[DEBUG] Hotkey received: brightness_up on monitor DELL U2722D
[DEBUG] DDC write: VCP 0x10 = 55 (was 50)
[WARN ] DDC retry 1/3 failed: I2C timeout
[DEBUG] DDC retry 2/3 succeeded
[ERROR] DDC failed after 3 retries: monitor LG 27UK850 not responding
```

### 9. DDC/CI Retry Strategy

**Approach:** Optimistic update with cached values and fixed retry

**Flow:**
```
[0ms]   Keypress received
        → Calculate new value: cached_brightness ± step_percent
        → Show OSD immediately with new value (optimistic)
        → Begin DDC write attempt in background

[~10ms] DDC attempt 1
        → Success: update cache, done
        → Failure: wait 10ms, retry

[~20ms] DDC attempt 2 (if needed)
        → Success: update cache, done
        → Failure: wait 10ms, retry

[~30ms] DDC attempt 3 (if needed)
        → Success: update cache, done
        → Failure: show error indicator in OSD, revert cache to last confirmed value
```

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **Update style** | Optimistic | Instant perceived responsiveness; smooth rapid adjustments |
| **Retry count** | 3 attempts | Covers most transient I²C bus hiccups |
| **Retry delay** | 10ms between attempts | Short enough to stay imperceptible (~30ms total) |
| **Cache** | Last confirmed DDC value per monitor | Enables instant OSD; refreshed on startup and every 60s |
| **Failure feedback** | Brief error indicator in OSD | User knows something went wrong without modal popups |
| **Failure recovery** | Revert displayed value to last confirmed | OSD shows accurate state after error |

**Why optimistic over conservative:**
- 30ms delay per keypress feels sluggish when adjusting
- Holding key down for rapid adjustment would be painfully slow waiting for DDC round-trips
- "Jump back" on failure is rare and accompanied by clear error indicator

**Cache Management:**
```rust
struct MonitorState {
    cached_brightness: u8,      // Last confirmed DDC value
    pending_brightness: Option<u8>,  // Optimistic value awaiting confirmation
    last_refresh: Instant,
}
```

- Cache populated on startup via DDC read
- Background refresh every 60 seconds (handles external changes)
- On DDC success: `cached_brightness = pending_brightness`
- On DDC failure: `pending_brightness` discarded, OSD reverts to `cached_brightness`

### 10. Error Handling: Graceful Degradation

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
