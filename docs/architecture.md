# Architecture & Technical Decisions

## Tech Stack

### Language & Toolchain
- Rust 2024 edition
- MSRV: 1.87+

### Dependencies

| Purpose | Crate | Rationale |
|---------|-------|-----------|
| Windows API | `windows` | Official Microsoft crate, modern API, better ergonomics than `winapi` |
| Error handling | `thiserror` | Derive macros for custom error types, zero runtime cost |
| Serialization | `serde` + `serde_json` | De facto standard, config file handling |
| Logging | `log` + `env_logger` | Standard facade pattern, runtime-configurable |

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
│   ├── controller.rs     # Controller<Osd,Ovl,Ddc,Loc>: message-driven orchestration behind OSD/overlay/DDC/locator seams, unit-tested with fakes; binary injects Windows impls + explicit now: Instant
│   ├── edid.rs           # EDID → MonitorId parsing
│   ├── logfile.rs        # Size-capped rolling file log sink
│   ├── reconcile.rs      # Refresh generations, respawn backoff, watchdog policies
│   └── state.rs          # Application state, messages, DDC commands
└── platform/
    ├── mod.rs            # Gates the platform submodule (Windows-only today)
    └── windows/          # #[cfg(windows)]
        ├── mod.rs        # RAII handle wrappers, cursor locator, usage window, error helpers
        ├── ddc.rs        # DDC/CI communication (monitor handles)
        ├── ddc_worker.rs # DDC worker thread (non-blocking I/O)
        ├── hotkey.rs     # RegisterHotKey API + optional low-level keyboard hook
        ├── osd.rs        # On-screen display window
        ├── osd_render.rs # OSD GDI rendering (RAII resource wrappers)
        ├── overlay.rs    # Dimming overlay windows (implements the OverlaySink seam)
        ├── power.rs      # Power event listener (sleep/resume)
        ├── single_instance.rs # Per-session named-mutex single-instance guard
        └── tray.rs       # System tray icon and menu
```

The portability boundary is the set of controller seams in `core/controller.rs`
(`OsdSink`, `OverlaySink`, `DdcPort`, `MonitorLocator`): core logic is generic
over them and unit-tested against fakes; `platform/windows/` provides the real
implementations. A port to another OS implements those seams plus its own
hotkey/power/tray equivalents and binary wiring.

### Threading Model

Dedicated threads for I/O, main thread owns state and UI:

```
┌─────────────────────────────────────────────────────────────┐
│                      Main Thread                            │
│  ┌───────────────────────────────────────────────────────┐  │
│  │           BrightnessController (owner)                │  │
│  │    - processes messages from all threads              │  │
│  │    - owns MonitorState map                            │  │
│  │    - updates overlay/OSD (UI always responsive)       │  │
│  │    - sends DdcCommand to worker                       │  │
│  │    - checks periodic/inactivity refresh triggers      │  │
│  └───────────────────────────────────────────────────────┘  │
│              ▲                              │                │
│       recv() │                              │ send()         │
│              │                              ▼                │
└──────────────┼──────────────────────────────┼────────────────┘
               │                              │
      BrightnessMessage                  DdcCommand
               │                              │
    ┌──────────┼──────────┬──────────┐        │
    │          │          │          │        │
┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───┴───────────────────────────┐
│  Hotkey   │ │  Power    │ │   Tray    │ │       DDC Worker Thread       │
│  Thread   │ │  Thread   │ │  Thread   │ │  ┌───────────────────────┐    │
│           │ │           │ │           │ │  │  - owns Vec<DdcMonitor>│   │
│ send()    │ │ send()    │ │ send()    │ │  │  - executes DDC I/O   │    │
│  │        │ │  │        │ │  │        │ │  │  - sends results back │    │
│  │        │ │  │        │ │  │        │ │  └───────────────────────┘    │
└──┼────────┘ └──┼────────┘ └──┼────────┘ └───────────────────────────────┘
   │             │             │                      │
   └─────────────┴─────────────┴──────────────────────┘
              (all send BrightnessMessage)
```

**Rationale:**
- No async runtime needed (simpler, smaller binary)
- DDC/CI operations are inherently blocking (~40-120ms per operation)
- DDC worker thread keeps main thread responsive for OSD/overlay updates
- Power thread listens for system resume events (sleep/hibernate wake)
- Tray thread handles system tray icon and context menu
- Single owner of state eliminates data races at compile time
- MPSC channels provide natural backpressure

### State Management

Message-passing with single ownership:

```rust
// Messages TO main thread (from hotkey thread, DDC worker, power thread, or tray thread)
enum BrightnessMessage {
    Adjust { monitor_id: Option<MonitorId>, delta: i8 },      // None = monitor under cursor
    DdcSetResult { monitor_id, value, seq, success, error }, // DDC worker → main
    DdcRefreshResult { generation, monitors, enumerated },   // DDC worker → main
    Refresh,
    SystemResumed,                                            // Power thread → main
    TrayOpenUsage,                                            // Tray thread → main
    TrayOpenSettings,                                         // Tray thread → main
    TrayOpenLogFolder,                                        // Tray thread → main
    TrayRequestQuit,                                          // Tray thread → main
    TrayMenuOpening { reply_tx: Sender<TrayMenuData> },       // Tray thread ↔ main (request/response)
    Shutdown,
}

// Commands TO DDC worker (from main thread)
enum DdcCommand {
    SetBrightness { monitor_id: MonitorId, value: u8, seq: u64 },
    RefreshAll { generation: u64 },
    Shutdown,
}
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

**Important Limitation:** On most laptops, brightness keys are handled by the Embedded Controller (EC) or ACPI firmware *before* they reach the Windows input system. The low-level keyboard hook (`WH_KEYBOARD_LL`) is the lowest interception point available in user-mode Windows, but it cannot capture keys that never enter the keyboard input pipeline. For these keyboards, the native Windows brightness OSD will appear regardless of hook installation, and the `intercept_brightness_keys` option will have no effect.

**Implementation:**

Primary hotkey *registration* failures (e.g. the combination is already taken by another application) are fatal. The application displays an error message box explaining the failure and suggesting solutions, then exits.

An *invalid hotkey string* in the config is **not** fatal: it is repaired to the default with an error log at load time (`Config::repair_hotkeys`, fed by the platform parser), per the "Invalid Config Handling" contract in section 4. The parse step before registration remains only as a defensive guard.

```rust
// Register primary hotkeys (fail = fatal error with message box)
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
- Secondary brightness keys (`VK_BRIGHTNESS_UP`/`DOWN`) use `SetWindowsHookExW(WH_KEYBOARD_LL)`
  to intercept before Shell handling (user-mode hook, not kernel-mode)
- Hook is **opt-in** via `hotkeys.intercept_brightness_keys` config option (default: `false`)
- Rationale for opt-in: Some antivirus software may flag low-level keyboard hooks;
  disabled by default to avoid false positives for users who don't need the feature

### 4. Configuration: JSON in AppData

Location: `%APPDATA%\BrightnessControl\config.json`

```json
{
  "version": 1,
  "hotkeys": {
    "brightness_up": "Ctrl+Shift+Up",
    "brightness_down": "Ctrl+Shift+Down",
    "intercept_brightness_keys": false
  },
  "monitors": {},
  "osd": {
    "timeout_ms": 1000,
    "opacity": 1.0
  },
  "brightness": {
    "step_percent": 5
  },
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

**`version` Field:** No migration logic exists yet. A value other than the current schema version logs a warning at load; the fields are interpreted as the current schema (unknown fields are dropped by the parser) and the value is reset to the current version, so later writes describe what the file actually contains.

**`monitors` Field:** Reserved for future per-monitor settings (e.g., min/max limits, custom step sizes, DDC disable). Empty `{}` for MVP. Schema will be defined based on real-world user feedback after v1.0. A non-empty map logs a warning at load ("not yet implemented"); entries are preserved and round-trip through saves so hand-written settings survive until the feature exists. Note that neither the key format (how a monitor is addressed in this map) nor the value shape is a contract yet — surviving hand-written entries may not match the eventual schema and may need manual migration when the feature lands.

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
| `hotkeys.brightness_up` | Valid hotkey string (see format above) | `Ctrl+Shift+Up` |
| `hotkeys.brightness_down` | Valid hotkey string (see format above) | `Ctrl+Shift+Down` |
| `hotkeys.intercept_brightness_keys` | `true` / `false` | `false` |
| `osd.timeout_ms` | 100 - 10000 | 1000 |
| `osd.opacity` | 0.1 - 1.0 | 1.0 |
| `brightness.step_percent` | 1 - 50 | 5 |
| `refresh.periodic_seconds` | 0 - 3600 | 60 |
| `refresh.inactivity_seconds` | 0 - 600 | 30 |
| `logging.file_enabled` | `true` / `false` | `false` |
| `logging.file_level` | `error` / `warn` / `info` / `debug` / `trace` (case-insensitive) | `info` |

Example log output for invalid config:
```
[ERROR] Invalid config: brightness.step_percent=999 exceeds maximum 50, using default 5
[ERROR] Invalid config: osd.timeout_ms=-5 below minimum 100, using default 1000
```

- Human-readable for debugging
- Portable to Linux

**Atomic Writes & Backup Recovery**

Config writes are atomic: the file is written to `config.json.tmp` and then renamed over `config.json` (atomic on a single volume), so a crash, power loss, or full disk mid-write can never leave a truncated file — the previous content survives.

After every *successful* parse at startup, the validated settings are mirrored to `config.json.bak` (also written atomically, best-effort — a backup failure never blocks startup). The backup therefore always holds the last-known-good configuration, including hand-edits that parsed successfully.

When `config.json` is unreadable or corrupt (typically a broken hand-edit via the tray "Settings" entry):
1. Settings are recovered from `config.json.bak` and a warning is logged — user settings are **not** silently replaced by defaults.
2. Only when the backup is also missing or corrupt do defaults substitute (logged as error).
3. The corrupt `config.json` is left untouched in both cases, so the user can inspect and fix their edit; it is not overwritten until the next successful save.

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
| **Opacity** | Configurable (`osd.opacity`, default 1.0) | User preference; avoids hardcoded magic numbers |
| **Timeout** | 1000ms after last keystroke | Short enough to not obstruct; resets on repeated adjustments |
| **Animation** | None (MVP) | Simplicity; animations deferred to future release |
| **Error state** | Red-tinted bar + message | Clear feedback when DDC fails; see below |

**Bidirectional Bar Layout (F.6):**

The OSD displays a single compact bar with two halves separated by a small gap:

| Section | Position | Fills | Shows |
|---------|----------|-------|-------|
| Overlay (🕶) | Left half | Right-to-left | Dimming level (0-100%) |
| Hardware (🔆) | Right half | Left-to-right | DDC brightness (0-100%) |

```
Layout: |pad| pct 🕶 ░░░░██████ | gap | ██████░░░░ 🔆 pct |pad|

Hardware at 100%, overlay inactive:
┌──────────────────────────────────────────────────────────┐
│  0% 🕶 ░░░░░░░░░░░░░░░░░░░░ ████████████████████ 🔆 100% │
└──────────────────────────────────────────────────────────┘

Hardware at 50%, overlay inactive:
┌──────────────────────────────────────────────────────────┐
│  0% 🕶 ░░░░░░░░░░░░░░░░░░░░ ██████████░░░░░░░░░░ 🔆  50% │
└──────────────────────────────────────────────────────────┘

Hardware at 0%, overlay inactive:
┌──────────────────────────────────────────────────────────┐
│  0% 🕶 ░░░░░░░░░░░░░░░░░░░░ ░░░░░░░░░░░░░░░░░░░░ 🔆   0% │
└──────────────────────────────────────────────────────────┘

Hardware at 0%, overlay at 30%:
┌──────────────────────────────────────────────────────────┐
│ 30% 🕶 ░░░░░░░░░░░░░░██████ ░░░░░░░░░░░░░░░░░░░░ 🔆   0% │
└──────────────────────────────────────────────────────────┘

Hardware at 0%, overlay at 100%:
┌──────────────────────────────────────────────────────────┐
│100% 🕶 ████████████████████ ░░░░░░░░░░░░░░░░░░░░ 🔆   0% │
└──────────────────────────────────────────────────────────┘
```

**Symbols:**
- 🔆 (U+1F506 High Brightness) — hardware/DDC brightness (right side, gold fill)
- 🕶 (U+1F576 Sunglasses) — dimming overlay (left side, purple fill)

**Behavior:**
1. Brightness down: hardware decreases until 0% (right bar shrinks)
2. At hardware 0%, continued presses increase overlay opacity (left bar fills toward center)
3. Brightness up: overlay decreases until 0% (left bar empties), then hardware increases
4. Both values always visible—no mode switching or bar appearance/disappearance
5. Pressing brightness-down always moves the "active" region leftward; brightness-up moves it rightward

**Dynamic Height:**
- Normal state: Compact 50px height (single bar row)
- Error state: Expands to 75px to show error message row below the bar

**Error Indicator:**

When DDC communication fails after all retries:
- Hardware (right) progress bar changes to red/error tint
- OSD expands to show error message row: "DDC Error - Adjustment failed"
- Percentage reverts to last confirmed value
- OSD timeout remains unchanged (1000ms)
- On next successful adjustment, OSD shrinks back to compact height

### 7. Dimming Overlay Implementation

**Method:** GDI window with `SetLayeredWindowAttributes`

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **Rendering** | GDI (`SetLayeredWindowAttributes`) | Simplest implementation for solid black rectangle; sufficient for MVP |
| **Window flags** | `WS_EX_LAYERED \| WS_EX_TRANSPARENT \| WS_EX_TOOLWINDOW` | Click-through, no taskbar entry |
| **Z-order** | `HWND_TOPMOST` | Stays above all normal windows |
| **Multi-monitor** | One overlay window per monitor | Independent opacity control per display |
| **Opacity range** | 0-100% (gradual) | Full range adjustable via continued hotkey presses below hardware 0% |

**Portability seam:**

The overlay is driven by the core `Controller` exclusively through the
`OverlaySink` seam defined in `core/controller.rs` (opacity 0–100 per monitor);
`OverlayManager` in `platform/windows/overlay.rs` is the Windows implementation.
The overlay opacity's single source of truth is `MonitorState.overlay_opacity`
in core — the platform side only drives windows. A port to another OS implements
the same seam against its compositor (e.g. Wayland layer-shell) without changing
core logic.

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
| Desktop / taskbar | ⚠️ Partial | `HWND_TOPMOST` covers normal windows; Shell UI (Taskbar, Start Menu) may render on top |
| Borderless windowed games | ✅ Yes | Rendered through compositor |
| Fullscreen exclusive games | ❌ No | Game bypasses DWM; writes directly to framebuffer |

**Known Limitations:**
1. **Fullscreen exclusive mode games** bypass the Windows compositor entirely. The overlay cannot dim these applications.
2. **Windows Shell UI** (Taskbar, Start Menu, Action Center) resides in higher Z-order bands. The overlay will not cover these elements when they are active or focused.

Users must either:
- Use the monitor's DDC minimum brightness (1%)
- Switch the game to borderless windowed mode

This is acceptable because:
- DDC/CI hardware brightness (1-100%) works in all scenarios including fullscreen exclusive
- Overlay is only used for the "bonus" sub-0% dimming feature
- Most modern games default to borderless windowed mode

### 8. Logging Strategy

**Console Window Behavior:**

The application uses conditional compilation to control console visibility:

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
```

| Build | Console | Rationale |
|-------|---------|-----------|
| Debug (`cargo build`) | Visible | Developers can see log output directly |
| Release (`cargo build --release`) | Hidden | Clean GUI experience for end users |

This means log output in release builds won't be visible in a console; for release diagnostics there is an opt-in rolling log file (below). Panics are recorded through the same logger: a process-wide hook installed at startup (`core/panic_hook.rs`) logs payload, source location, and thread at error level and flushes the sinks before the default handler continues — so even a crash that aborts the process (e.g. a panic unwinding into an `extern "system"` callback) leaves a trace in the file log.

**Level:** Configurable via `RUST_LOG` environment variable (standard `env_logger` behavior; console only)

> **Note:** For guidelines on *how* to write log statements (level selection heuristics, structured fields, avoiding PII), see `code-conventions.md` section 7.

| Level | Events Logged |
|-------|---------------|
| **Error** | All failures including recoverable (DDC errors, config parse failures, overlay creation failed) |
| **Warn** | Fallback triggers, performance issues, deprecated config fields, monitor disconnected |
| **Info** | Startup/shutdown, config loaded, monitors detected, hotkeys registered |
| **Debug** | Each brightness change, hotkey received, OSD show/hide, DDC command sent |
| **Trace** | DDC raw I²C bytes, window messages, config file contents |

**Default Level:** `Info` for release builds, `Debug` for debug builds.

**Rolling File Log (opt-in):**

Because the release console is hidden, `logging.file_enabled = true` additionally
tees every log record into `%APPDATA%\BrightnessControl\darkbright.log` — the
retrievable artifact for field reports. Mechanics:

- **Rotation:** size-capped two-file scheme. When the active file would exceed
  1 MB it is renamed over `darkbright.log.old` (replacing it) and a fresh file
  starts — worst-case ~2 MB disk, always ≥1 MB of recent history. A failed
  rotation (e.g. transient file lock) degrades to a temporarily oversized file
  and is retried on the next over-cap write; logging never stops.
- **Level:** `logging.file_level` filters the file independently of the
  console (`RUST_LOG` does not affect the file). At `debug` and below the file
  will contain monitor serial numbers and absolute paths (debug-only under the
  PII rule) — acceptable for a deliberately created diagnostic artifact.
- **Startup ordering:** the logger is installed console-only, and the file
  sink attaches immediately after the config is loaded (the setting lives in
  the config). The config-loading log lines themselves therefore reach only
  the console; the file starts with a version-stamped "File logging enabled"
  line.
- **Access:** the tray menu's "Open Log Folder" entry opens the directory in
  Explorer.

Formatting (timestamps, level, target, `key=value` pairs) comes from a second
`env_logger` instance writing into the rotating file via `Target::Pipe`, so
file and console lines look alike. The `unstable-kv` feature is enabled so
structured fields are actually rendered on both sinks.

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
[ERROR] DDC failed after 3 attempts: monitor LG 27UK850 not responding
```

### 9. Refresh Strategy (Cache Synchronization)

The application maintains cached brightness values for instant OSD response. These caches can become stale if brightness is changed externally (physical monitor buttons, other apps, or system events).

**Multi-Trigger Refresh Approach:**

| Trigger | Default | Config Key | Rationale |
|---------|---------|------------|-----------|
| **Periodic** | 60s | `refresh.periodic_seconds` | Catches gradual drift from external changes |
| **Inactivity** | 30s | `refresh.inactivity_seconds` | Resyncs before first adjustment after idle period |
| **System Resume** | Always | (not configurable) | Monitors may reset brightness after sleep/hibernate |

**Behavior:**

1. **Periodic Refresh**: Background poll every N seconds (0 = disabled). Conservative default balances freshness with DDC overhead. Gated on whether the last refresh *enumerated* any monitor (identification succeeded, whether or not the brightness read that followed did), not merely whether one was *readable* — so the cadence keeps running while monitors are enumerable but unreadable (e.g. undocked), which is what lets ghost pruning below complete without any user activity. An aborted refresh (a failed send to the worker, or a watchdog timeout) freezes the cadence the same way an empty enumerated set does — `RefreshTracker::abort()` clears the same flag — until a refresh completes with something enumerated again.

2. **Inactivity Refresh**: When user adjusts brightness after being inactive for N seconds, a refresh is triggered first. Uses non-blocking approach: refresh is initiated but adjustment proceeds optimistically. Values reconcile when DDC results arrive.

3. **System Resume**: Power event listener detects `PBT_APMRESUMEAUTOMATIC` and `PBT_APMRESUMESUSPEND` (`WM_POWERBROADCAST`, handled in the window procedure — it is a sent message, never queued). The listener's message-only window must be explicitly subscribed via `RegisterSuspendResumeNotification`, since message-only windows are excluded from message broadcasts. Triggers immediate refresh since monitors often reset to default brightness after sleep.

**Overlap Protection:**

A `RefreshTracker` prevents overlapping refresh requests and correlates each
refresh to its result by a generation counter, so a late result from a
superseded refresh cannot clear the in-progress state of a newer one. This
avoids DDC bus congestion when multiple triggers fire simultaneously (e.g.,
resume + periodic + inactivity at once). If a refresh result never returns
(hung or dead worker), a watchdog aborts it after `REFRESH_TIMEOUT` so
refreshes are never permanently suppressed.

**Enumerated vs. Readable Monitors:**

Each `DdcRefreshResult` reports two sets: `monitors`, the brightness values
that were successfully read, and `enumerated`, every monitor whose EDID
identification succeeded this pass, regardless of whether the brightness
read that followed it did. The ids in `enumerated` are always a superset of
the ids in `monitors`, and the set is empty when nothing could be identified: either
the top-level enumeration call failed outright, or it succeeded but every
discovered monitor's individual identity read failed. The distinction
matters because "unreadable" is common and often transient — standby, an
EDID-emulating KVM, a DDC hiccup surviving all 3 retries — while
"unenumerated" is a much closer proxy for "not physically present."

Unreadable-but-enumerated monitors stay *set-capable*: the worker keeps
their freshly opened DDC handle even when the initial brightness read fails,
so a later `SetBrightness` is attempted against hardware (confirming or
reverting through the normal optimistic protocol) instead of failing with
"monitor not found" until the next refresh. Only the reported brightness
value is missing — the main thread retains its cached value meanwhile.

**Overlay Reconcile on External Change:**

A refresh read above the hardware floor (`> 0`) for a monitor whose sub-zero
overlay is active clears that overlay: the brightness was raised externally
(physical monitor buttons, another tool, or a monitor resetting itself after
sleep), and a software veil silently fighting that change would leave the
monitor murky — bright backlight behind a half-opaque black window. The
trade-off is deliberate: a monitor that self-resets after sleep loses its
sub-zero dim and comes back bright. An in-flight optimistic set suppresses
the reconcile — the set is newer intent than the refresh read (same
precedence rule as `update_from_ddc` leaving pending sets intact). A read
of exactly 0 never clears the overlay: hardware at the floor is consistent
with sub-zero dimming.

**Ghost Pruning:**

A monitor's absence from a current-generation, non-empty `enumerated` set is
tracked in its `missing_since` field: the first miss stamps the timestamp,
and a later miss showing the absence has been continuous for at least
`PRUNE_ABSENCE_WINDOW` (90s) prunes the monitor — its state, its overlay
window, and its `id_cache` entries are all removed. A stale or aborted
refresh generation, or an empty `enumerated` set, carries no evidence and
leaves `missing_since` untouched: the absence of information is not
evidence of absence.

All accumulated absence evidence is discarded on `SystemResumed` and on a
DDC worker respawn, because a refresh burst around either event can observe
a monitor missing for a few seconds while, for example, a dock's DP link is
still training — absence evidence must span an undisturbed window, not
survive a burst.

Pruning forgets deliberately: cached brightness and overlay dim level do not
survive a > 90s absence. A monitor that reappears after being pruned starts
from a fresh state read from hardware, with its overlay back at 0%. Because
a hotkey press on a pruned (now-unknown) monitor would otherwise stay dead
until the next periodic or inactivity refresh, the unknown-monitor path in
the adjust handler also triggers a refresh (at most one in flight), so
recovery works regardless of which other monitors are still readable.

**Configuration:**

```json
{
  "refresh": {
    "periodic_seconds": 60,
    "inactivity_seconds": 30
  }
}
```

Set either to `0` to disable that trigger. System resume refresh cannot be disabled.

### 10. DDC/CI Retry Strategy

**Approach:** Optimistic update with cached values and fixed retry

**Flow:**
```
[0ms]   Keypress received (hotkey thread)
        → Send BrightnessMessage::Adjust to main thread

[<1ms]  Main thread processes adjustment
        → Calculate new value: cached_brightness ± step_percent
        → Set pending brightness (optimistic update)
        → Update overlay immediately (if changed)
        → Show OSD immediately with new value
        → Send DdcCommand::SetBrightness to DDC worker
        → Return immediately (non-blocking)

[~40ms] DDC worker: attempt 1
        → Success: send DdcSetResult(success) to main
        → Failure: wait 40ms, retry

[~80ms] DDC worker: attempt 2 (if needed)
        → Success: send DdcSetResult(success) to main
        → Failure: wait 40ms, retry

[~120ms] DDC worker: attempt 3 (if needed)
        → Success: send DdcSetResult(success) to main
        → Failure: send DdcSetResult(failure) to main

[async] Main thread receives DdcSetResult (correlated by seq)
        → Matching seq, success: promote pending → cached, update OSD
        → Matching seq, failure: revert pending, show OSD error state
        → Stale seq (a newer set is in flight): ignore
        → No pending (already reverted by the watchdog), success: apply as ground truth
```

| Aspect | Decision | Rationale |
|--------|----------|-----------|
| **Update style** | Optimistic | Instant perceived responsiveness; smooth rapid adjustments |
| **DDC execution** | Dedicated worker thread | Main thread stays responsive; OSD updates without blocking |
| **Retry count** | Up to 3 attempts | Covers most transient I²C bus hiccups |
| **Retry delay** | 40ms between retries | Safe default for slower monitor controllers |
| **Cache** | Last confirmed DDC value per monitor | Enables instant OSD; refreshed on startup |
| **Failure feedback** | Brief error indicator in OSD | User knows something went wrong without modal popups |
| **Failure recovery** | Revert displayed value to last confirmed | OSD shows accurate state after error |

**Why optimistic over conservative:**
- 30ms delay per keypress feels sluggish when adjusting
- Holding key down for rapid adjustment would be painfully slow waiting for DDC round-trips
- "Jump back" on failure is rare and accompanied by clear error indicator

**Cache Management:**
```rust
struct MonitorState {
    cached_brightness: u8,          // Last confirmed DDC value
    pending: Option<PendingSet>,    // Optimistic value + seq + sent-at, awaiting confirmation
    overlay_opacity: u8,            // Current overlay dimming level
    last_refresh: Instant,
    missing_since: Option<Instant>, // First observed miss from the enumerated set; None while present
}
```

- Cache populated on startup via `DdcCommand::RefreshAll`
- DDC worker owns all `DdcMonitor` instances
- On a seq-matching `DdcSetResult(success)`: the pending value is promoted to cached
- On a seq-matching `DdcSetResult(failure)`: the optimistic value is reverted
- A success arriving with no matching pending (already reverted by the watchdog) is applied as ground truth; other non-matching results are ignored as stale

### 11. Error Handling: Strict

If DDC communication fails after retries, the application does **not** fall back to software overlay dimming for values > 0%.

**Rationale:**
- Prevents loss of contrast ratio (software dimming washes out blacks)
- Avoids state desynchronization (user thinks brightness is 50%, backlight is actually 100%)
- Ensures the user is aware of hardware communication failures via the OSD error indicator

### 12. Worker Supervision & State Watchdog

The DDC worker is supervised so its death or a lost result cannot silently and
permanently degrade the app. Two decoupled mechanisms:

- **Death detection (fast path).** The main loop polls `JoinHandle::is_finished()`
  (~every 250 ms) and treats a command-channel `send` error as a hard failure. A
  dead worker is respawned within ~250 ms, its stuck optimistic values reverted,
  and a fresh refresh issued. Respawns are rate-limited: more than `RESPAWN_MAX`
  within `RESPAWN_WINDOW` disables DDC until recovery.
- **State watchdog (backstop).** Wall-clock deadlines reconcile state a dead-worker
  check cannot see. A pending set outstanding past `SET_TIMEOUT` is reverted with a
  red OSD; an in-flight refresh past `REFRESH_TIMEOUT` is aborted. `SET_TIMEOUT` is
  deliberately generous (larger than `REFRESH_TIMEOUT` plus a set's budget) because a
  set can queue behind an in-flight refresh; it exists to catch a worker that is
  alive but hung inside a blocking DDC call, not a merely-queued set.

A worker that is hung (not dead) is never respawned — that would run two threads
against the same physical-monitor handles. After `HUNG_TIMEOUT_LIMIT` consecutive
set timeouts it is diagnosed as hung and DDC is disabled. A disabled DDC state
recovers on user activity (a hotkey adjustment) or on system resume.

| Constant | Value | Purpose |
|---|---|---|
| `SET_TIMEOUT` | 8000 ms | Backstop revert for a pending set (hung worker) |
| `REFRESH_TIMEOUT` | 5000 ms | Abort an in-flight refresh |
| `RESPAWN_MAX` / `RESPAWN_WINDOW` | 3 / 60 s | Respawn backoff before disabling DDC |
| `HUNG_TIMEOUT_LIMIT` | 3 | Consecutive set timeouts before diagnosing a hang |

**Hotkey thread liveness.** The hotkey thread — the app's primary input — gets
the same treatment: its message loop logs both exit paths (`WM_QUIT` and a
`GetMessageW` error), and the main loop polls its `JoinHandle::is_finished()`.
A dead hotkey thread is restarted (fresh message window, hotkeys re-registered)
under a `RespawnGate` with the same `RESPAWN_MAX`/`RESPAWN_WINDOW` backoff:
deaths spaced apart restart indefinitely, a rapid crash loop (or a failed
restart attempt) latches into a logged give-up state until the app is
restarted. The tray and power threads remain unsupervised by design — both are
non-fatal conveniences. One release-build consequence is worth knowing: with
the console hidden there is no Ctrl+C, so the tray menu's Quit is the only
*graceful* shutdown path — if the tray thread dies, ending the process takes
Task Manager. Accepted: a hard kill loses nothing (settings live in the config
file, brightness in the hardware, overlay windows die with the process).

Both degraded states — DDC disabled and hotkeys given up — are surfaced to the
user through the tray icon, tooltip, and menu (see §13, "Degraded-State
Indicator"), not just the log.

**Panic policy (deliberate).** Supervision covers *environmental* failures —
hung hardware, dead threads. Panics are bugs and are handled fail-fast: a panic
in plain Rust code unwinds its thread (worker threads then hit the respawn
machinery above), while a panic inside one of the `extern "system"` callbacks
(the wnd_procs, the low-level keyboard hook, the monitor-enum callback) cannot
unwind across the ABI and aborts the whole process. That abort is accepted
deliberately rather than guarded with `catch_unwind`: running on past a broken
invariant risks worse outcomes than a restart — most concretely a sub-zero
overlay stuck black over a monitor while the app keeps "working" — whereas
process death destroys all overlay windows (hardware brightness simply stays at
its last value), and the panic hook (§8) has already logged payload + location
and flushed the file log by the time the abort happens. The callback bodies are
kept deliberately thin (mostly channel forwarding) to keep this panic surface
minimal.

### 13. System Tray

The application runs as a background process with a system tray icon for user interaction.

**Tray Icon Behavior:**

| Action | Result |
|--------|--------|
| Right-click | Opens context menu |
| Left-click | No action (intentional) |

**Context Menu Structure:**

```
┌─────────────────────────────────────────────────┐
│ ⚠ DDC unavailable — press a brightness hotkey…  │  ← Warnings (only while degraded)
│─────────────────────────────────────────────────│
│ DEL U2722D: 🕶 0% 🔆 50%                        │  ← Monitor status (disabled/info only)
│ LG 27UK850: 🕶 0% 🔆 75%                        │
│─────────────────────────────────────────────────│
│ Usage                                           │  → Opens usage instructions window
│ Settings                                        │  → Opens config.json in default editor
│ Open Log Folder                                 │  → Opens %APPDATA%\BrightnessControl in Explorer
│ Quit Brightness Control                         │  → Graceful shutdown
│─────────────────────────────────────────────────│
│ Brightness Control v0.8.0                       │  ← Version (disabled/info only)
└─────────────────────────────────────────────────┘
```

**Monitor Status Rows:**
- Displayed at the top of the menu as disabled (non-clickable) items
- Show current overlay opacity (🕶) and hardware brightness (🔆) for each monitor
- Updated each time the menu is opened via `TrayMenuOpening` request/response

**Degraded-State Indicator:**

The two supervision give-up states (§12) — DDC disabled and hotkeys lost — are
visible in the tray through two complementary paths:

- **Pull (menu):** `TrayMenuData` carries a `HealthWarnings` snapshot; while a
  warning is active the menu opens with grayed warning lines at the very top
  ("⚠ DDC unavailable — press a brightness hotkey to retry", "⚠ Hotkeys
  stopped working — restart the app"). Always current because the menu is
  populated on open.
- **Push (icon + tooltip):** the main loop compares the controller's
  `HealthWarnings` each tick and, on a transition, posts a custom window
  message to the tray thread via `TrayStatusHandle` (`PostMessageW`, safe
  cross-thread, fire-and-forget). The tray thread then swaps icon and tooltip
  via `NIM_MODIFY`: while degraded the icon carries an amber corner badge
  (generated at startup by drawing the base icon into a DIB and painting the
  badge — no second icon asset), and the tooltip appends the active warnings
  (e.g. "Brightness Control – DDC unavailable").

Recovery follows §12: DDC warnings clear on user activity or resume (the icon
reverts automatically); the hotkey warning is latched until the app restarts.

**Usage Window:**

Clicking "Usage" opens a modeless window displaying usage instructions:

```
1. Move mouse to desired monitor
2. Press Ctrl+Shift+Up (brighter) or Ctrl+Shift+Down (dimmer)
```

The window displays the user's configured hotkeys (not hardcoded defaults). This helps new users discover how to use the application without consulting documentation.

**Implementation Notes:**
- The usage window is modeless (does not block the application)
- Only one instance can exist at a time; clicking "Usage" again brings the existing window to front
- The window is centered on the primary monitor
- The window can be closed via the close button (X) or Alt+F4

### 14. Single-Instance Guard

At most one instance runs per logon session. Startup creates a named mutex **before** spawning any worker thread, window, or hotkey registration; a second launch detects the existing name, shows an informational message box, and exits without side effects (no duplicate tray icon, overlay, or failed hotkey registration).

**Reserved mutex name (stable contract):**

```
Local\darkbright-helper-single-instance
```

This name is load-bearing for external integrations — e.g. a future autostart/installer feature that needs to ask "is it already running?". Do not rename it.

**Mechanics:**
- The `Local\` prefix scopes the object to the current logon session, so each user (and each RDP session) can run its own instance.
- Detection is existence-based: the mutex is never acquired via a wait. `CreateMutexW` succeeding with last-error `ERROR_ALREADY_EXISTS` signals a second instance.
- The guard holds only a handle; the kernel deletes the named object when the last handle closes, so the name is freed on any exit path — including a crash. There is no stale-lock state to recover from.
- `ERROR_ACCESS_DENIED` (the name exists but is owned by a higher-integrity instance, e.g. one running elevated) is treated as already-running, not as an error.

**Fail-open policy:** any *other* `CreateMutexW` failure logs an error and startup continues **without** the guard. An unexpected guard failure must never block the user's only instance; the worst case is a duplicate instance, not a lockout.

Implementation: `src/platform/windows/single_instance.rs` (RAII `SingleInstance` guard held for the process lifetime), checked at the top of `main()`.

---

## Testing

### Unit Tests

Run all unit tests with:
```bash
cargo test
```

Key test areas:
- **Config validation**: Ensures invalid values are clamped to defaults
- **Brightness calculations**: Tests adjustment logic in `core/brightness.rs`
- **State management**: Tests `MonitorState` transitions
- **Controller orchestration**: `core/controller.rs` drives the optimistic-update, supervision, watchdog, refresh, and ghost-pruning sequences against fakes for the OSD/overlay/DDC/locator seams — the message-driven control flow is unit-tested on any host, no Windows target or physical monitor required

### Integration Testing (Manual)

Controller orchestration is unit-tested (see above); what remains hardware-dependent and must be tested manually is DDC/CI I/O against real monitors, the DDC worker's EDID enumeration (including the `enumerated` set it reports), and topology changes:

#### Periodic Refresh Test
1. Set `refresh.periodic_seconds` to a low value (e.g., 10) in config
2. Start the application with `RUST_LOG=debug`
3. Change monitor brightness using physical buttons or another app
4. Wait for the periodic interval to elapse
5. **Expected**: Log shows "Periodic refresh triggered" and OSD reflects actual brightness on next adjustment

#### Inactivity Refresh Test
1. Set `refresh.inactivity_seconds` to a low value (e.g., 5) in config
2. Start the application with `RUST_LOG=debug`
3. Adjust brightness, then wait longer than the inactivity threshold
4. Change monitor brightness externally
5. Press hotkey to adjust brightness
6. **Expected**: Log shows "Inactivity refresh triggered" before the adjustment

#### System Resume Test
1. Start the application with `RUST_LOG=debug`
2. Put system to sleep (`Win+X` → Sleep)
3. Wake system
4. **Expected**: Log shows "System resumed from sleep" followed by refresh

#### Overlap Protection Test
1. Set both `periodic_seconds` and `inactivity_seconds` to low values
2. Trigger conditions where multiple refresh triggers fire simultaneously
3. **Expected**: Only one refresh executes (log shows single "Requesting monitor refresh")

#### Unplug/Replug (Ghost Pruning) Test
1. Set `refresh.periodic_seconds` to a low value (e.g., 10) in config so the 90s absence window is reached quickly
2. Start the application with `RUST_LOG=debug` and **at least two monitors connected** — unplugging the only monitor would empty `enumerated` entirely, which freezes the periodic cadence (see above) before 90 seconds of absence evidence can accumulate; a second monitor must stay enumerable for the whole wait
3. Unplug one monitor (or switch it away on a non-EDID-emulating KVM), leaving the other connected
4. Wait for periodic refreshes to observe the absence continuously for at least 90 seconds
5. **Expected**: Log shows "Pruned monitor absent from topology"; the tray menu no longer lists the monitor; a dimming overlay left active on it, if any, is removed
6. Replug the monitor
7. **Expected**: The monitor reappears with brightness read fresh from hardware and overlay back at 0% — the prior dim level is not restored

#### Monitor Standby Cycle Test
1. Start the application with `RUST_LOG=debug`
2. Put one monitor into standby via its own power button (not system sleep) while others stay awake
3. **Expected**: The monitor's tray row persists — it is still enumerable, just momentarily unreadable — and no ghost pruning occurs
4. Wake the monitor
5. **Expected**: DDC reads resume on the next refresh with no special recovery needed

#### File Logging Test
1. Set `logging.file_enabled` to `true` in config
2. Start the application (release build — no console needed)
3. **Expected**: `%APPDATA%\BrightnessControl\darkbright.log` starts with a version-stamped "File logging enabled" line; adjustments append info-level lines including `key=value` fields; `RUST_LOG` has no effect on the file
4. Tray → "Open Log Folder"
5. **Expected**: Explorer opens the folder containing `config.json` and `darkbright.log`
6. Set `logging.file_level` to `"verbose"` (invalid) and restart
7. **Expected**: An error line reports the invalid value; the file logs at the default `info` level

#### Degraded-State Tray Indicator Test
1. Start the application with `RUST_LOG=debug`
2. Force a degraded DDC state (e.g. temporarily lower `HUNG_TIMEOUT_LIMIT`/`SET_TIMEOUT` in a test build and use a monitor/cable that drops DDC, or unplug all DDC-capable monitors and adjust repeatedly until "disabling DDC" is logged)
3. **Expected**: The tray icon gains an amber corner badge; hovering shows "Brightness Control – DDC unavailable"; the menu opens with the grayed "⚠ DDC unavailable" line at the top; the menu's bottom line shows the running version
4. Press a brightness hotkey (user activity is the recovery signal)
5. **Expected**: Log shows "Recovering from degraded DDC state"; icon, tooltip, and menu revert to normal

---
