# Architecture & Technical Decisions

## Tech Stack

### Language & Toolchain
- Rust 2024 edition
- MSRV: 1.88+ (let chains)

### Dependencies

| Purpose | Crate | Rationale |
|---------|-------|-----------|
| Windows API | `windows` | Official Microsoft crate, modern API, better ergonomics than `winapi` |
| Error handling | `thiserror` | Derive macros for custom error types, zero runtime cost |
| Serialization | `serde` + `serde_json` | De facto standard, config file handling |
| Logging | `log` + `env_logger` | Standard facade pattern, runtime-configurable |

Two mechanisms keep this set current, because they fail differently. CI's weekly
`cargo audit` job answers "is anything here *known bad*"; Dependabot
(`.github/dependabot.yml`, weekly) answers "is anything here *quietly stale*",
which is this repo's actual failure mode — every drift so far was found by human
review, never by tooling. Minor and patch updates arrive as one grouped PR;
majors arrive individually so a breaking one cannot hold up the safe batch.

`windows` is deliberately excluded from that group. A minor bump rewrites FFI
signatures across `platform/windows/` and is only mergeable after the manual
hardware pass noted in `Cargo.toml`, so it stands alone rather than blocking
everything grouped with it. A bump that raises the minimum toolchain is caught
by CI's separate MSRV job, not by the update PR itself.

### Build-Time Resources

`build.rs` embeds two resources through `winres`, both Windows-only:

- **The application icon** (`res/icon.ico`), used by the shell and by the tray.
- **An application manifest** declaring a dependency on version 6 of the common
  controls (`Microsoft.Windows.Common-Controls`, resource type `RT_MANIFEST`,
  ID 1). This is what enables visual styles for system-drawn controls.

Without that declaration Windows draws system controls with the pre-XP
renderer, and a themed control cannot follow the light/dark setting at all — a
control asked to theme itself dark would simply stay grey. The app's two
message boxes are the only surface this reaches today: their buttons are drawn
in the current Windows style rather than the grey 3D one. Message boxes have no
dark rendering to follow, so they stay light either way; the manifest changes
how they are painted, not which colours they use.

The manifest declares nothing else on purpose. DPI awareness, long path support
and the requested execution level are all things a manifest can set, and each
changes how the app *behaves* rather than how a control is *painted* — they are
separate changes with separate testing, not riders on this one.

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
│   ├── panic_hook.rs     # Logs panic payload/location/thread, flushes sinks before exit
│   ├── reconcile.rs      # Refresh generations, respawn backoff, watchdog policies
│   └── state.rs          # Application state, messages, DDC commands
└── platform/
    ├── mod.rs            # Gates the platform submodule (Windows-only today)
    └── windows/          # #[cfg(windows)]
        ├── mod.rs        # RAII handle wrappers, cursor locator, error helpers, message boxes
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
│  │        Controller (owner, in core/controller.rs)      │  │
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
│           │ │           │ │           │ │  │  - owns monitor HashMap│   │
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

#### Main-Loop Cadence

The main loop blocks in `rx.recv_timeout(16ms)` and pumps the Win32 message queue on
every iteration, so it wakes ~62×/s for the life of the process. That number is chosen,
not accreted, and this is the reasoning.

**Why poll at all.** Two windows live on the main thread — the OSD and the dimming
overlay — and neither has a message loop of its own; they are serviced only by
`pump_windows_messages()`. A thread cannot block on an MPSC channel and the Win32 message
queue at the same time, so one of the two has to be polled. The channel carries a rich
enum and the queue does not, so the queue is the one that gets polled.

**The timeout is not input latency.** Any `send()` wakes `recv_timeout` immediately, so
hotkey presses, DDC results and the tray's menu-data request are served without waiting
out the interval. The first OSD paint is not delayed either: after a message is handled
the loop continues to the top and pumps before blocking again. The interval governs one
thing only — how quickly *unsolicited* window messages are noticed.

**What that leaves.** Only the OSD's auto-hide `WM_TIMER` and interaction with the usage
window (its OK button, close box and repaints). The overlay needs no pumping at all: it is
a layered `LWA_ALPHA` window with no `WM_PAINT` handler, composited by the DWM, and it is
click-through like the OSD, so neither receives input.

**The bounds are fixed by two other decisions.** Below, `osd.timeout_ms` may be configured
as low as 100 ms, and an auto-hide must not visibly overshoot it. Above, Windows treats a
top-level window whose thread has not pumped for 5 s as unresponsive and ghosts it. Any
interval from roughly 25 ms to 1 s satisfies both; 16 ms sits at the responsive end.

**Measured cost** (2026-07-26, release build, 11.9 h uptime): 0.72 s of CPU total, i.e.
**0.0017 % of one core**, or ~60 ms per hour — and that figure includes startup, EDID
parsing, window creation and every DDC refresh in the window. Sampled over 30 s of idle,
the accumulated CPU time did not advance at all: ~0.27 µs of work per wake is below the
scheduler's accounting granularity. The process does not call `timeBeginPeriod`, so it
never raises the global timer resolution and its waits stay coalescable by the OS. Not
covered by that measurement: the energy cost of denying the core deeper idle states, which
is real but small next to any GUI process on the same machine.

**An adaptive interval was considered and declined** — 16 ms while the OSD or overlay
is up, ~250 ms otherwise. It works, it is about ten lines, and it would cut idle wakes 15×.
It was not worth it: the saving is ~1.2 s of CPU per day, paid for with a predicate that
can silently rot. A window added to the main thread later would have to be remembered in
it, and forgetting is not something a test would catch — the symptom is a sluggish UI, not
a failure. The overlay would also have to be deliberately *excluded* from such a predicate,
since sub-zero dimming can be active for hours; including it out of caution would keep the
fast path running through exactly the scenario the app exists for. A fully event-driven
loop (`MsgWaitForMultipleObjects` plus a wake-post from every sender) was rejected further:
`std::sync::mpsc` exposes no waitable handle, a forgotten post would delay a message
silently, and supervision would still need a ~250 ms timer — so it lands at the same wake
rate as the adaptive variant, for a transport rewrite.

Revisit if a main-thread window ever becomes long-lived and interactive, or if battery
profiling on a mobile machine says otherwise.

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
Logical brightness (0-100%) maps linearly onto whatever luminance range the monitor exposes. While human perception is logarithmic, a linear mapping is chosen for simplicity and predictability: 50% means 50% of that range.

**The DDC scale is per-monitor and must not be assumed to be 0-100.**
VCP 0x10 is a *continuous* control: the value is 16-bit on the wire and MCCS does not require the maximum to be 100. `GetVCPFeatureAndVCPFeatureReply` returns the monitor's declared maximum alongside the current value; `DdcMonitor` records it on each successful read and converts in both directions via `core::brightness::percent_from_vcp` / `vcp_from_percent`. Percentages are therefore the only currency above the platform seam, and no raw value leaves `platform/windows/ddc.rs`.

Consequences worth knowing:
- Until a read succeeds the common 0-100 scale is assumed, which makes the conversion a pass-through — an unreadable monitor is no worse off than before scaling existed.
- On a monitor whose range is narrower than 100 steps, distinct percentages necessarily collapse onto the same raw value. Both ends stay reachable; the resolution is the hardware's.
- The declared maximum is logged with each refresh read at `debug`, because a maximum other than 100 is the one condition that makes every brightness number in a log suspect, and it is not otherwise observable.

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

**When changes take effect:** at the next start, not while running. The file is read once
during startup and the resulting `Config` is then cloned into the places that need it — the
controller keeps one, the hotkey thread keeps another, `main` keeps the original. All three
are immutable for the process lifetime, which is why no synchronisation is needed and why
they cannot drift apart. There is no reload path: the tray's "Settings" item opens
`config.json` in the default editor, but saving it changes nothing until the app is
restarted. Live reload is the first thing that has to be built if the settings GUI on the
roadmap lands, and it is what makes the three snapshots a design decision rather than an
accident.

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

**Unknown keys** (typos, misplaced settings) are logged as warnings at load
with their full path (e.g. `hotkeys.brightnes_up`) — never fatal, the parser
simply drops them. The check diffs the raw file against the parsed config's
own serialization, so there is no hand-maintained key list; the `monitors`
map's contents are exempt (its key format is not yet a contract).

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
- **Attach failure:** if the sink cannot be built (no `APPDATA`, an unwritable
  directory, a locked file, a full disk), the app runs on without it — but the
  failure is reported through the **tray**, not only the log. Announcing the
  loss of a diagnostic channel on that same channel would reach nobody: the
  file is by definition absent, and release builds hide the console, so a user
  who set `file_enabled = true` to chase a problem would find an empty folder
  and no explanation. The warning line points at the log folder because "Open
  Log Folder" sits in the same menu, which is where that user is heading. It is
  deliberately the one warning that does **not** raise the icon's amber badge —
  see §13. Unlike a failed rotation, the attach is attempted exactly once, so
  the condition is latched for the process.

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
| **Display Change** | Always | (not configurable) | Topology changed: monitors added/removed, and handles may be reused for a different display |

**Behavior:**

1. **Periodic Refresh**: Background poll every N seconds (0 = disabled). Conservative default balances freshness with DDC overhead. Gated on whether the last refresh *enumerated* any monitor (identification succeeded, whether or not the brightness read that followed did), not merely whether one was *readable* — so the cadence keeps running while monitors are enumerable but unreadable (e.g. undocked), which is what lets ghost pruning below complete without any user activity. An aborted refresh (a failed send to the worker, or a watchdog timeout) freezes the cadence the same way an empty enumerated set does — `RefreshTracker::abort()` clears the same flag — until a refresh completes with something enumerated again.

2. **Inactivity Refresh**: When user adjusts brightness after being inactive for N seconds, a refresh is triggered first. Uses non-blocking approach: refresh is initiated but adjustment proceeds optimistically. Values reconcile when DDC results arrive.

3. **System Resume**: The system event listener detects `PBT_APMRESUMEAUTOMATIC` and `PBT_APMRESUMESUSPEND` (`WM_POWERBROADCAST`, handled in the window procedure — it is a sent message, never queued). The window is explicitly subscribed via `RegisterSuspendResumeNotification`, the documented delivery guarantee for this message. Triggers immediate refresh since monitors often reset to default brightness after sleep.

4. **Display Change**: The same listener handles `WM_DISPLAYCHANGE` and sends a plain `Refresh`. This is the only trigger that cannot be disabled or delayed by configuration, and it is the one that keeps handle→identity mapping honest: Windows may reuse a monitor handle for a different display across a topology change, so a stale `id_cache` entry could otherwise send an adjustment to the wrong monitor. Absence evidence is deliberately *not* reset — a monitor genuinely unplugged here should still age out and be pruned.

   The listener's window is therefore a hidden **top-level** window, not a message-only one: message-only windows are excluded from broadcast messages, and `WM_DISPLAYCHANGE` is broadcast. (This is the same trap that once cost this module its resume detection; a unit test now pins the window's parent to the desktop so the property cannot silently regress.)

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

Set-capable requires *state*, so the main thread creates it for every
enumerated monitor, not only the readable ones. A monitor that has never been
read has no cached value to retain, so it is seeded at `UNREAD_BRIGHTNESS_SEED`
(50, the midpoint — it bounds how far the first adjustment can jump from a value
nobody knows) and marked `brightness_known: false`. Seeding is insert-only: a
monitor that was readable before keeps its real last-known value, which always
outranks a seed.

The marker matters because the seed is a guess the user should not mistake for a
measurement. It is cleared by the first evidence either way — a successful read
or a write the hardware accepted — and until then the tray menu prefixes the
value with `~`. The sharp case this covers is a panel that persistently NAKs the
VCP *read* while honouring the *write*: previously permanently and silently
uncontrollable, since every keypress returned "monitor not found" before
reaching the OSD, so nothing moved and nothing was shown.

The OSD deliberately shows the seeded value without a distinct visual state:
after the first adjustment the value is authoritative anyway (the write
established it), so a warning styling would flash once and never return. The
standing signal lives in the tray, where it can persist.

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
    brightness_known: bool,         // False while the value is a seed, not an observation
    pending: Option<PendingSet>,    // Optimistic value + seq + sent-at, awaiting confirmation
    overlay_opacity: u8,            // Current overlay dimming level
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
set timeouts it is diagnosed as unresponsive.

**The two degraded DDC states are distinct, because they differ in what ends
them** (`DdcHealth` in `core/state.rs`). Conflating them is how a tray line
comes to advertise a recovery that cannot work:

| State | Cause | What ends it |
|---|---|---|
| `WorkerDead` | Respawn backoff exhausted; the thread is gone | A hotkey press (clears the backoff, so the next supervision pass respawns), or system resume |
| `WorkerHung` | Thread alive but blocked inside a DDC call | **Any result the worker sends** — a result is proof it is not blocked. Also system resume, or the worker dying (below) |

A keypress deliberately does **not** clear `WorkerHung`: it cannot unstick a
blocked call, so clearing would only retract a warning that is still true and
let it reappear seconds later when the next set times out — a flickering badge
instead of a standing one.

Proof-of-life is taken from *every* worker result, including failures and
results the reconciler discards as stale: a worker reporting a NAK is answering,
and answering is the whole of what the diagnosis denied. This is the only exit
from `WorkerHung` that needs neither a resume nor a restart, and it costs
nothing — the results already arrive.

An unresponsive worker that subsequently *dies* is treated as a plain death and
respawned. The reason not to respawn a hung worker went with the thread, and
since no user action clears that state, leaving it standing would strand the app
with no worker at all.

Dispatch is deliberately **not** gated while degraded. A false hang diagnosis
would otherwise turn a merely slow worker into a dead one, and the queue is
harmless: sets are correlated by sequence id, so a backlog delivered late
reconciles to the correct final value.

| Constant | Value | Purpose |
|---|---|---|
| `SET_TIMEOUT` | 8000 ms | Backstop revert for a pending set (hung worker) |
| `REFRESH_TIMEOUT` | 5000 ms | Abort an in-flight refresh |
| `RESPAWN_MAX` / `RESPAWN_WINDOW` | 3 / 60 s | Respawn backoff before disabling DDC |
| `HUNG_TIMEOUT_LIMIT` | 3 | Consecutive set timeouts before diagnosing a hang |

**One restart policy, two supervised threads.** `RespawnGate`
(`core/reconcile.rs`) holds the whole rule — deaths spaced apart restart
indefinitely, a rapid crash loop within `RESPAWN_WINDOW` latches into a
give-up state — and both the DDC worker and the hotkey thread run on that one
instance of it. What differs is not the policy but the way back out:

- The **DDC worker** has external recovery triggers, so `DdcSupervisor` calls
  `RespawnGate::reset()` when one fires (a brightness keypress or a system
  resume) and the gate reopens. The supervisor itself only translates a
  decision into an action — spawn a replacement, or report that none was
  spawned — because `RespawnOutcome` is what the controller needs to know.
- The **hotkey thread** has no such trigger and never resets: once its restarts
  are exhausted the tray says "restart the app" and means it. A failed restart
  *attempt* latches immediately (`record_spawn_failure`) — a second attempt
  would fail the same way.

**Hotkey thread liveness.** The hotkey thread — the app's primary input — gets
the same treatment: its message loop logs both exit paths (`WM_QUIT` and a
`GetMessageW` error), and the main loop polls its `JoinHandle::is_finished()`.
A dead hotkey thread is restarted with a fresh message window and hotkeys
re-registered. The tray and power threads remain unsupervised by design — both are
non-fatal conveniences. One release-build consequence is worth knowing: with
the console hidden there is no Ctrl+C, so the tray menu's Quit is the only
*graceful* shutdown path — if the tray thread dies, ending the process takes
Task Manager. Accepted: a hard kill loses nothing (settings live in the config
file, brightness in the hardware, overlay windows die with the process).

Both degraded states — DDC disabled and hotkeys given up — are surfaced to the
user through the tray icon, tooltip, and menu (see §13, "Degraded-State
Indicator"), not just the log. A third condition rides the same channel without
being a supervision state at all: a file log that failed to attach (§8).

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
│ Point mouse at a monitor, then:                 │  ← Usage rows (disabled/info only)
│ Brighter                        Ctrl+Shift+Up   │
│ Dimmer                        Ctrl+Shift+Down   │
│─────────────────────────────────────────────────│
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

**Menu Theming:**

The context menu is a plain `TrackPopupMenu` popup, which Windows draws light
unless the process has opted into dark mode. `platform/windows/theme.rs` makes
that opt-in once, before the tray window exists: `SetPreferredAppMode(AllowDark)`,
`RefreshImmersiveColorPolicyState`, `FlushMenuThemes`, plus
`AllowDarkModeForWindow` for the message-only window that owns the menu.
`AllowDark` rather than `ForceDark` is the point — the system light/dark
setting decides the colour, the app never overrides it.

Keeping up with a *changed* setting deliberately ignores the notification
Windows sends for it. The `WM_SETTINGCHANGE` broadcast carrying
`"ImmersiveColorSet"` never reaches this app: the tray's window is
message-only, and message-only windows are excluded from broadcasts — measured,
not assumed, after a first attempt to handle that message logged nothing across
half an hour of theme switching. Rather than acquire a top-level window to hear
the news, `show_context_menu` refreshes the theme itself immediately before it
builds the popup. The menu is rebuilt on every right-click anyway, so it always
reflects the setting as it stands at that moment, and no delivery path is left
that could silently fail.

That refresh is `RefreshImmersiveColorPolicyState` **followed by**
`FlushMenuThemes`, and the order matters: `FlushMenuThemes` alone leaves the
menu in whichever colour it had at process start — the first version of this
did exactly that and did not follow a live theme change. The same pairing
appears in the reference implementations of this API (wxWidgets `darkmode.cpp`,
`ysc3839/win32-darkmode`).

All four switches are exported by `uxtheme.dll` by ordinal only and appear in
no public header, so they are resolved at runtime and gated on Windows build
18362 (see [Maintenance Decisions](#maintenance-decisions)). Every failure path
— build too old, library absent, export gone — leaves the menu precisely as it
was before: light, fully functional, and named in the log (`debug` for the
build gate, `warn` for a library or export that should have been there).

`AllowDarkModeForWindow` is called for the tray's window and for no other; the
OSD and overlay paint their own colours regardless. `SetPreferredAppMode` is
process-wide, so other system-drawn surfaces follow the setting as well, but
the only ones this app raises are its two message boxes (the single-instance
notice and the hotkey-registration error) — and a message box has no dark
rendering to follow. Both were checked and stay light on a dark system.

**Degraded-State Indicator:**

Three conditions are visible in the tray: the two supervision give-up states
(§12) — DDC disabled and hotkeys lost — plus a file log that failed to attach
(§8). They reach the user through two complementary paths:

- **Pull (menu):** `TrayMenuData` carries a `HealthWarnings` snapshot; while a
  warning is active the menu opens with grayed warning lines at the very top.
  The DDC line is chosen by cause, because each names the recovery that
  actually applies: "⚠ DDC unavailable — press a brightness hotkey to retry"
  for a dead worker, "⚠ Monitor not responding — restart the app if this
  persists" for an unresponsive one (hedged, since it often frees itself).
  Alongside them, "⚠ Hotkeys stopped working — restart the app" and "⚠ File
  logging failed to start — check the log folder is writable". Always current
  because the menu is populated on open.
- **Push (icon + tooltip):** the main loop compares the controller's
  `HealthWarnings` each tick and, on a transition, posts a custom window
  message to the tray thread via `TrayStatusHandle` (`PostMessageW`, safe
  cross-thread, fire-and-forget). The tray thread then swaps icon and tooltip
  via `NIM_MODIFY`: while degraded the icon carries an amber corner badge
  (generated at startup by drawing the base icon into a DIB and painting the
  badge — no second icon asset), and the tooltip appends the active warnings
  (e.g. "Brightness Control – DDC unavailable"). The `HealthWarnings` snapshot
  crosses to the tray thread packed into the message's `wparam`, so the cause
  has to survive that hop — a round-trip test pins the encoding. The badge
  itself is raised only by the two supervision states: it means the app cannot
  do its job, and a missing diagnostic log does not stop a single adjustment,
  so letting it light the badge would weaken the signal for the conditions that
  do. A failed file log therefore shows in the menu and the tooltip, never on
  the icon.

Recovery follows §12 and differs per cause: a dead worker's warning clears on
user activity or resume, an unresponsive worker's clears when the worker answers
again or on resume (the icon reverts automatically in both cases); the hotkey
warning is latched until the app restarts.

**Usage Rows:**

The menu states the core interaction instead of hiding it behind a click, so a
first-time user meets the instructions while looking for them rather than after
finding the right item:

```
Point mouse at a monitor, then:
Brighter                           Ctrl+Shift+Up
Dimmer                           Ctrl+Shift+Down
```

Three disabled rows, composed once at tray startup from the running
configuration — a user who rebound the hotkeys is taught the keys that actually
work, not the defaults. Everything after a `	` is drawn right-aligned in the
menu's shortcut column, which is where a reader already scans for a key
combination and what keeps the rows no wider than the monitor status lines
above them.

**Implementation Notes:**
- The rows carry no dark-mode handling of their own: they are part of the menu, which already follows the system setting (see Menu Theming above)
- They do not come from `TrayMenuOpening`, so they still appear when the main thread misses the reply timeout
- Disabled (`MF_GRAYED`) throughout — informational, never clickable

A modeless window held these two lines before, opened from a "Usage" menu item.
It was removed with the move: a window class, centring, focus handling and a
theming path of its own — dark mode included — is a large apparatus for one
sentence the menu can simply hold. The rows cost none of that and cannot fall
out of step with the menu around them. What went with the window is the option
to leave the instructions on screen while trying the keys.

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

## Maintenance Decisions

Deliberate technical positions with no user-visible effect. They are recorded here so they
do not silently drift into something nobody chose.

### `windows` crate version policy

The crate tracks the current minor; patch releases drift freely. Minor and major bumps are
routine maintenance, but a **breaking** release gets its own branch and a full manual
hardware pass — DDC, OSD, overlay, tray, hotkeys, power events — and is never folded in as a
side effect of another change. The mechanics of why this crate sits outside the grouped
Dependabot PR are covered under [Dependencies](#dependencies).

There is no standing revisit interval: re-evaluate whenever upstream ships a breaking
release that touches the API surface this project actually uses.

### Dark menus through undocumented `uxtheme` ordinals

`SetPreferredAppMode`, `AllowDarkModeForWindow`, `FlushMenuThemes` and
`RefreshImmersiveColorPolicyState` are the only way to get a system-drawn Win32
popup menu to follow the light/dark setting. Microsoft exports them from
`uxtheme.dll` by ordinal (135, 133, 136, 104), without names, and documents
none of them. The alternatives were weighed and
rejected: owner-drawing the menu means several hundred lines of GDI that still
misses the rounded corners and backdrop of a system menu, and a custom popup
window means owning keyboard navigation, dismissal and accessibility for six
menu rows.

The exposure is bounded but not eliminated. The ordinals are resolved once,
behind a Windows-build gate (≥ 18362), and every failure — gate,
`LoadLibraryW`, any one `GetProcAddress` — returns the same nothing: the menu
is drawn light, as it was before this existed. No `Result` propagates, no
`HealthWarnings` entry is raised, and no other code path depends on the call
having worked, so an ordinal that is *removed* costs a light menu and a `warn`
line, nothing more.

The residual risk is an ordinal that is *reassigned* rather than removed. The
lookup then succeeds and the process calls a function with the wrong signature,
and no gate can see that coming — the build check has a floor, not a ceiling.
This is not hypothetical: ordinal 135 was `AllowDarkModeForApp(bool)` on
builds 17763–18361 and became `SetPreferredAppMode(PreferredAppMode)` at 18362,
which is why the gate sits at 18362 rather than at the ordinals' first
appearance. That risk is accepted, not engineered away: it is bounded to a
menu that no longer themes correctly in the likely case, and it is the price of
the only mechanism Windows offers.

Revisit when a Windows release changes the ordinals or when a documented API
for menu theming appears. There is no interval to check on — a removed export
now announces itself in the log, and a wrong theme is visible on sight.

### Hand-rolled handle wrappers predating `Owned<T>`

`SafeHKey` and `SafeDevInfo` (`src/platform/windows/ddc.rs`) hand-roll their cleanup
(`RegCloseKey`, `SetupDiDestroyDeviceInfoList`). Both predate the move to `windows` 0.62,
whose `windows::core::Owned<T>` now covers exactly that pattern through its `Free` impls.

They are functionally identical to an `Owned<T>` today, so this is not a defect and carries
no deadline — fold the migration in opportunistically, the next time `ddc.rs` is touched for
another reason. `SafeHwnd` is explicitly **not** a candidate: `DestroyWindow` does not fit
the `Free` pattern. See `docs/code-conventions.md` § 3 for the rule this illustrates.

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
8. With `file_enabled` still `true`, make the sink unbuildable — e.g. deny your user write access to `%APPDATA%\BrightnessControl`, or hold `darkbright.log` open exclusively from another process — and start a **release** build (no console)
9. **Expected**: The app runs normally and the tray menu opens with a grayed "⚠ File logging failed to start — check the log folder is writable" line; the tray icon stays unbadged (this condition does not mean brightness control is broken); the tooltip reads "Brightness Control – file logging off"

#### Degraded-State Tray Indicator Test
1. Start the application with `RUST_LOG=debug`
2. Force a degraded DDC state (e.g. temporarily lower `HUNG_TIMEOUT_LIMIT`/`SET_TIMEOUT` in a test build and use a monitor/cable that drops DDC, or unplug all DDC-capable monitors and adjust repeatedly until "disabling DDC" is logged)
3. **Expected**: The tray icon gains an amber corner badge; the menu's bottom line shows the running version. The wording depends on which state was reached — check the log line to know which one you provoked:
   - respawn backoff exhausted (`respawn backoff exceeded`): tooltip "Brightness Control – DDC unavailable", menu line "⚠ DDC unavailable — press a brightness hotkey to retry"
   - unresponsive worker (`DDC worker unresponsive`): tooltip "Brightness Control – monitor not responding", menu line "⚠ Monitor not responding — restart the app if this persists"
4. Press a brightness hotkey
5. **Expected**: after the *backoff* state, the log shows "Recovering from degraded DDC state" and the icon, tooltip and menu revert. After the *unresponsive* state, the warning deliberately stays — a keypress cannot unstick a blocked call. It clears on its own once the worker answers again ("DDC worker answered again"), on system resume, or if the stuck thread exits ("Unresponsive DDC worker has exited")

---
