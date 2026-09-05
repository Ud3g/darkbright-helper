# Architecture & Technical Decisions

> **Keeping this file true.** This document is the source of truth for behaviour, which only
> works if a behaviour change and its description land in the same commit. Two rules follow
> from that, both learned from breakage:
>
> - **Supersede in place, never by addition.** When a later section describes new behaviour,
>   go back and fix every earlier passage it contradicts. A reader — human or LLM — takes the
>   first statement it finds as authoritative and has no way to know that something 800 lines
>   further down overrides it. The settings window (§14) invalidated three claims in §4 this
>   way, and they stood uncorrected until a documentation review went looking.
> - **Section numbers are a stable contract; do not renumber.** Cross-references use `§N`,
>   including from `CLAUDE.md`, from dated design and review documents that must not be
>   rewritten after the fact, and from other sections here. Add new sections at the end,
>   keep a retired section's number retired, and resolve a numbering mistake by merging or
>   retitling rather than by shifting everything after it.
>
> Passages that go stale quietly, worth a glance whenever the shape of the code changes: the
> module tree and seam list under [Platform Abstraction](#platform-abstraction), the config
> field table in §4, the constants in §12, and the manual checklists under
> [Integration Testing (Manual)](#integration-testing-manual).

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
│   ├── mod.rs            # Declares the core submodules; `edid` stays crate-internal
│   ├── brightness.rs     # Brightness calculations, value mapping
│   ├── config.rs         # Configuration types and loading
│   ├── controller.rs     # Controller<Osd,Ovl,Ddc,Loc,Set,Hk,Store>: message-driven orchestration behind the seams below, unit-tested with fakes; binary injects Windows impls + explicit now: Instant
│   ├── edid.rs           # EDID → MonitorId parsing
│   ├── logfile.rs        # Size-capped rolling file log sink
│   ├── panic_hook.rs     # Logs panic payload/location/thread, flushes sinks before exit
│   ├── reconcile.rs      # Refresh generations, respawn backoff, watchdog policies
│   ├── state.rs          # Application state, messages, DDC commands
│   └── version.rs        # Build/version string shown in the tray and settings footer
└── platform/
    ├── mod.rs            # Gates the platform submodule (Windows-only today)
    └── windows/          # #[cfg(windows)]
        ├── mod.rs        # RAII handle wrappers, cursor locator, error helpers, message boxes
        ├── autostart.rs  # "Start with Windows" via HKCU\Run
        ├── config_store.rs # ConfigStore seam: atomic save with merge-on-external-edit
        ├── ddc.rs        # DDC/CI communication (monitor handles)
        ├── ddc_worker.rs # DDC worker thread (non-blocking I/O)
        ├── hotkey.rs     # RegisterHotKey API + optional low-level keyboard hook
        ├── osd.rs        # On-screen display window
        ├── osd_render.rs # OSD GDI rendering (RAII resource wrappers)
        ├── overlay.rs    # Dimming overlay windows (implements the OverlaySink seam)
        ├── power.rs      # Power event listener (sleep/resume)
        ├── settings/     # Settings window, on its own thread (§14)
        │   ├── mod.rs    # Directory module gate
        │   ├── capture.rs # Hotkey-capture control
        │   ├── dark.rs   # Dark-mode painting
        │   ├── layout.rs # Declarative CONTROLS/RANGE_SPECS tables, DPI scaling
        │   └── window.rs # Window creation, wiring, SettingsSinkImpl
        ├── single_instance.rs # Per-session named-mutex single-instance guard
        ├── theme.rs      # Dark-mode opt-in (tray menu, settings window)
        └── tray.rs       # System tray icon and menu
```

The portability boundary is the set of controller seams in `core/controller.rs`
(`OsdSink`, `OverlaySink`, `DdcPort`, `MonitorLocator`, `SettingsSink`, `HotkeyPort`,
`ConfigStore`): core logic is generic over them and unit-tested against fakes;
`platform/windows/` provides the real implementations. A port to another OS implements those
seams plus its own hotkey/power/tray equivalents and binary wiring.

**Adding a seam**

The seam count went from four to seven in a single feature, so this is a path that gets
walked. A new seam is one trait and five other edits; the compiler catches every one of
them but the last, and the last is the one that rots:

In `core/controller.rs`:

1. The trait, with a doc comment saying which side of the boundary owns the work and whether
   a result comes back later as a `BrightnessMessage`.
2. A type parameter on `Controller<…>` and on its `impl` block's bounds.
3. A field on the struct and a parameter on `Controller::new`.
4. A `Fake…` in the test module, plus its line in the `TestController` alias and its argument
   in `test_controller()`. See section 8 of `docs/code-conventions.md` for what a fake in
   this codebase may and may not do — the short version is a plain struct with `Vec` fields
   and no interior mutability.

In `platform/windows/` and `main.rs`:

5. The real implementation, and the wiring that injects it.

In the docs:

6. Six places name this set, and they are one edit: the seam list in the paragraph above,
   the `controller.rs` line of the module tree, the seam enumeration under §Testing, the
   seam table in §14, the module doc comment at the top of `src/platform/mod.rs`, and the
   module map in `CLAUDE.md`. Two of them have already rotted once: the §Testing copy still
   said "four" long after the settings window shipped, and `platform/mod.rs` named only the
   original four seams. Neither is reachable from the compiler, which is why they are
   listed by name here.

Note what is deliberately absent: no rule about how many tests a seam needs, and no coverage
claim here — §Testing owns the unit-vs-manual boundary.

### Threading Model

Dedicated threads for I/O, main thread owns state and UI:

```
┌───────────────────────────────────────────────────────────────────────────┐
│                               Main Thread                                 │
│  ┌───────────────────────────────────────────────────────┐                │
│  │        Controller (owner, in core/controller.rs)      │                │
│  │    - processes messages from all threads              │                │
│  │    - owns MonitorState map                            │                │
│  │    - updates overlay/OSD (UI always responsive)       │                │
│  │    - sends DdcCommand to worker                       │                │
│  │    - checks periodic/inactivity refresh triggers      │                │
│  └───────────────────────────────────────────────────────┘                │
│             ▲                                             │               │
│      recv() │                                             │ send()        │
│             │                                             ▼               │
└─────────────┼─────────────────────────────────────────────┼───────────────┘
              │                                             │
     BrightnessMessage                                 DdcCommand
              │                                             │
   ┌──────────┴──┬─────────────┬─────────────┐              │
   │             │             │             │              │
┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───────────┐ ┌───┴───────────────────────────┐
│  Hotkey   │ │  Power    │ │   Tray    │ │ Settings  │ │       DDC Worker Thread       │
│  Thread   │ │  Thread   │ │  Thread   │ │  Thread   │ │  ┌────────────────────────┐   │
│           │ │           │ │           │ │(per open) │ │  │  - owns monitor HashMap│   │
│ send()    │ │ send()    │ │ send()    │ │ send()    │ │  │  - executes DDC I/O    │   │
│  │        │ │  │        │ │  │        │ │  │        │ │  │  - sends results back  │   │
└──┼────────┘ └──┼────────┘ └──┼────────┘ └──┼────────┘ │  └────────────────────────┘   │
   │             │             │             │          └───────────────────────────────┘
   │             │             │             │                        │
   └─────────────┴─────────────┴─────────────┴────────────────────────┘
              (all send BrightnessMessage)
```

**Rationale:**
- No async runtime needed (simpler, smaller binary)
- DDC/CI operations are inherently blocking (~40-120ms per operation)
- DDC worker thread keeps main thread responsive for OSD/overlay updates
- Power thread listens for system resume events (sleep/hibernate wake)
- Tray thread handles system tray icon and context menu
- Settings thread hosts the settings window and its own `GetMessageW` loop, so a modal
  dialog blocks only itself; it is spawned per open and exits with the window (§14)
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

**What that leaves.** Only the OSD's auto-hide `WM_TIMER`. The overlay needs no pumping at
all: it is a layered `LWA_ALPHA` window with no `WM_PAINT` handler, composited by the DWM,
and it is click-through like the OSD, so neither receives input. (A modeless "usage" window
once lived on this thread too and would have belonged in this paragraph; it was removed in
favour of tray menu rows — see §13. The settings window added later needs no pumping either,
for the opposite reason: it runs on its own dedicated thread with its own `GetMessageW` loop,
see §14, so it never reaches this one at all.)

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

#### Thread Conventions

Four rules that were each decided three different ways before being written down.

**Every thread this process starts is named.** Threads are started through
`thread::Builder::new().name("ddc" | "hotkey" | "power" | "settings" | "tray")`, never
`thread::spawn`. The panic hook (`core/panic_hook.rs`) logs the thread name, and in a release
build the log file is the only diagnostic that survives a crash — a report saying
`thread = <unnamed>` names the one thing the reader needed. The rule cannot reach threads
Windows creates to call into us, notably the console control handler: a panic there aborts
inside an `extern "system"` callback and does report `<unnamed>`.

A spawn that fails is handled where the subsystem's other failures are handled: the tray and
power threads log and carry on without that subsystem; the hotkey thread reports
`ThreadSpawn`, fatal at startup and a degraded `set_hotkeys_lost()` on a respawn; the DDC
worker reports a refused respawn as `RespawnOutcome::BackoffExceeded`, the same degraded
state a worker that died repeatedly produces, and only its *initial* spawn is fatal. Two
subsystems have extra cleanup: the settings thread must release its `OPENING` slot **and**
send `SettingsClosed`, or the window stays unopenable and the controller's `settings_open`
latches true for the rest of the run.

**A failed `send` is logged at the level its message deserves.** `error!` when the message
carries user intent or state the receiver must reconcile — a brightness adjustment, a DDC
result, a menu command the user clicked; `warn!` when nobody is harmed by the loss, typically
a reply whose requester may already be gone (the tray's menu data, sent after the menu may
have closed) or a notification that only fails because the app is shutting down.
`let _ = tx.send(...)` is reserved for sends whose failure is the normal
shutdown case, and the reason then goes in a comment or doc comment at the site. A dropped
keypress that leaves no trace anywhere is the failure this rule exists to prevent.

**A poisoned mutex is recovered, not avoided.** Every shared mutex in this process guards
plain data — a command queue, a sender — that a panicking thread cannot leave half-written in
a way a second thread would misread, so the policy is
`unwrap_or_else(PoisonError::into_inner)`. Silently skipping the critical section
(`if let Ok(guard)`) hides the failure and can turn Ctrl+C into a no-op; converting poison to
an error reports a condition the caller cannot act on. If a mutex ever guards a multi-field
invariant, that one gets its own documented policy rather than changing this one.

**Worker threads are never joined.** There is no `.join()` in `src/`, deliberately: process
exit is the join. Only the DDC worker is asked to stop (`Controller::shutdown_worker`, so a
DDC transaction in flight is not cut mid-write); the tray, hotkey, power and settings threads
are simply left to die with the process. A thread blocked in `GetMessageW` would make a join
a deadlock, and one hotkey spawn is already abandoned on purpose when it fails to report in
time — see `start_hotkey_thread` in `main.rs` for that case. That abandoned spawn is a live
writer, not a dead one: the hotkey thread id and command queue are shared cells (the id is
0 until a thread signals ready and reset to 0 when it exits, so a rebind against a missing
thread ordinarily fails cleanly), and a spawn that finishes registering after its deadline
still publishes into those same cells. The clean-failure property is therefore a strong
default, not an absolute guarantee. A thread that genuinely needs joining needs a design
discussion first.

**`WM_APP` messages are allocated per receiving window class, and the one
thread-addressed message is the exception that needs a guard.** Three modules define custom
messages, and the values live in three files today:

| Constant | Value | Owning thread | Receiving class | Delivery |
|---|---|---|---|---|
| `WM_APP_SETTINGS_REFRESH` … `_TOPMOST` | `WM_APP + 1` … `+ 5` | settings | `DarkBrightSettings` | `PostMessageW` to the window |
| `WM_APP_HOTKEY_WAKE` | `WM_APP + 10` | hotkey | none — see below | **`PostThreadMessageW` to the thread** |
| `WM_TRAY_CALLBACK` / `WM_TRAY_STATUS` | `WM_APP + 100` / `+ 101` | tray | `BrightnessControlTrayWindow` | `PostMessageW` to the window |

For a window-addressed message, `WM_APP + n` only has to be unique among the messages that
one *class* handles; two classes may reuse a value without interfering, which is why the
settings thread can host both `DarkBrightSettings` and `DarkBrightHotkeyCapture` without a
shared numbering scheme. The disjoint bases above are therefore convenience, not a
requirement.

A thread-addressed message is different, and `WM_APP_HOTKEY_WAKE` is the only one in the
process. The hotkey thread does host a class of its own (`DarkBrightHotkeyWindow`, a
message-only window used for `RegisterHotKey`), but the wake is deliberately not addressed to
it: it arrives with a null `hwnd`, so `DispatchMessageW` can deliver it nowhere and it is
handled in the loop body or dropped. That is what keeps it from being confused with a
window message. The `msg.hwnd.is_invalid()` test in `run_message_loop` guards the converse
case — a window message that happens to carry the same value being mistaken for a wake — and
is the reason the wake value must be unique across *every* class on that thread, not merely
within one. Introducing a second thread-addressed message means repeating that check and
re-checking every class hosted on the receiving thread.

One allocation carries a further constraint that is not obvious from the values: the five
settings messages are a contiguous, ordered range because `drain_pending_payload_messages`
reclaims their heap payloads with a `PeekMessageW` range filter over `[REFRESH, TOPMOST]`. A
constant added outside that span would be skipped and its `Box` leaked, so a compile-time
assertion beside the constants pins the ordering and the width.

### State Management

Message-passing with single ownership:

```rust
// Messages TO main thread (from the hotkey, DDC worker, power, tray, or settings thread)
enum BrightnessMessage {
    // Brightness and cache
    Adjust { monitor_id: Option<MonitorId>, delta: i8 },       // None = monitor under cursor
    AdjustStep { direction: i8 },                              // One configured step, cursor monitor
    DdcSetResult { monitor_id, value, seq, success, error },    // DDC worker → main
    DdcRefreshResult { generation, monitors, enumerated },      // DDC worker → main
    Refresh,
    SystemResumed,                                             // Power thread → main

    // Tray thread → main
    TrayOpenSettings,
    TrayOpenLogFolder,
    TrayRequestQuit,
    TrayMenuOpening { reply_tx: Sender<TrayMenuData> },         // request/response

    // Settings window
    SettingChanged(SettingChange),                             // Settings thread → main
    HotkeyRebindResult { op, success, fallback_active, error }, // Hotkey thread → main (ack)
    SettingsClosed,                                            // Flush pending save, end capture
    HotkeyCaptureStarted,                                      // Suspend interception while capturing
    HotkeyCaptureEnded,                                        // Capture ended with no new binding
    OpenConfigFile,                                            // Shell side effect, like TrayOpenLogFolder

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

### 3. Hotkeys: Targeting and Hybrid Registration

**Which monitor a hotkey affects: the one under the mouse cursor.**
- `GetCursorPos()` → `MonitorFromPoint()`
- Intuitive, matches user expectations from volume controls
- No configuration required

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
// In `apply_bindings`, used for the initial registration and every rebind.
// A primary that will not register is fatal: the app cannot do its job.
self.register_hotkey(BRIGHTNESS_UP_ID, up.modifiers, up.vk_code)
    .map_err(|e| BrightnessError::hotkey_registration(up.to_string(), e.to_string()))?;

// The dedicated brightness keys are best-effort: another app or the shell may
// already own them, or the low-level hook may be handling them instead.
fn register_secondary_brightness_hotkeys(&mut self) {
    if let Err(e) =
        self.register_hotkey(BRIGHTNESS_UP_ALT_ID, HOT_KEY_MODIFIERS(0), VK_BRIGHTNESS_UP)
    {
        log::debug!(error:% = e; "Secondary brightness up hotkey not registered");
    }
    // … the same for BRIGHTNESS_DOWN_ALT_ID / VK_BRIGHTNESS_DOWN
}
```

**Technical Details:**
- Dedicated listener thread calls `RegisterHotKey()` API
- Receives `WM_HOTKEY` messages in thread message loop
- Secondary brightness keys (`VK_BRIGHTNESS_UP`/`DOWN`) are, by default, registered through
  the same plain `RegisterHotKey` path as the primaries
  (`register_secondary_brightness_hotkeys`), so the Shell sees them first and the app only
  gets what the Shell leaves over
- Intercepting them *ahead of* the Shell instead requires the low-level keyboard hook
  (`SetWindowsHookExW(WH_KEYBOARD_LL)` — a user-mode hook, not kernel-mode), which is
  **opt-in** via `hotkeys.intercept_brightness_keys` (default: `false`). When the hook is
  requested but cannot be installed, the dedicated keys fall back to plain registration and
  the failure is reported as a notice rather than an error
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

JSON rather than a binary or bespoke format for two reasons that both outlive the MVP: the
file is human-readable, so a user chasing a problem can inspect and hand-edit it without a
tool, and it is portable to Linux, where a future port would read the very same file.

**`version` Field:** No migration logic exists yet. A value other than the current schema version logs a warning at load; the fields are interpreted as the current schema (unknown fields are dropped by the parser) and the value is reset to the current version, so later writes describe what the file actually contains.

**`monitors` Field:** Reserved for future per-monitor settings (e.g., min/max limits, custom step sizes, DDC disable). Empty `{}` for MVP. Schema will be defined based on real-world user feedback after v1.0. A non-empty map logs a warning at load ("not yet implemented"); entries are preserved and round-trip through saves so hand-written settings survive until the feature exists. Note that neither the key format (how a monitor is addressed in this map) nor the value shape is a contract yet — surviving hand-written entries may not match the eventual schema and may need manual migration when the feature lands.

**When changes take effect:** it depends on *how* the change is made, and the split is a
design decision rather than an accident.

Changes made in the settings window (§14) apply immediately — including hotkey rebinds,
which happen in place on the live hotkey thread. The controller is the sole owner of the
runtime `Config`; the dialog never mutates it directly but posts
`BrightnessMessage::SettingChanged(SettingChange)` down the existing channel, so the single
owner keeps the invariant that only the main thread mutates state. The startup `Config` that
`main` loads is a snapshot used to build the log sink and to seed the threads it spawns; the
hotkey thread is given the two hotkey strings and the intercept flag, not a `Config`, which
is why a rebind does not have to reach a second copy. Only the **logging** options are
exempt: the rolling file sink is attached once during startup, so `logging.file_enabled` and
`logging.file_level` take effect at the next start (a static label in the dialog says so).

Changes made by hand-editing `config.json` take effect at the next start. There is no file
watcher and no reload path — the file is read at startup, and thereafter only re-read when a
save needs to merge onto it (§14, "Merge-on-external-change"). A hand-edit therefore
survives on disk but does not apply while the app is running.

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

**Merge Strategy: Per-Field Defaults**

Every field of the config schema carries a serde default — a named function where the
default is a real value (`#[serde(default = "default_osd_timeout")]`), the bare
`#[serde(default)]` where `Default` already gives the right answer, as it does for the
nested sections and for the `false` booleans. The reserved `monitors`
map is defaulted as a whole, its inner shape being no contract yet — so the merge happens per
field rather than per section: a key the file does not contain takes its default, and the
rest of the file is still honoured. Adding a field to the schema therefore does not
invalidate existing configs and needs no manual edit by the user — the key reappears in the
file the next time a save rewrites it. This is what makes step 1 of the checklist below load-bearing: a field without
a serde default fails the whole parse for every user whose file predates it, which the
`.bak` recovery path would then mask as a corrupt config.

**Invalid Config Handling: Error and Use Default**

When a config value is invalid (e.g., `step_percent: 999`, `timeout_ms: 50`):
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

These ranges are shared with, but enforced differently by, the settings window (§14): the
loader above *substitutes the default* for an out-of-range value (a repair policy for
unattended startup), while the dialog *clamps to the nearest bound* on focus loss (a guidance
policy for a user mid-edit). Same never-fatal spirit, deliberately different mechanism — don't
"fix" one to match the other.

Two spinner ranges deliberately do *not* match the table, and the mismatch is the point: in
`settings/layout.rs` the periodic-resync spinner is bounded 1–3600 and the inactivity-resync
spinner 1–600, while the config accepts 0 for both. Zero means "disabled" there, and in the
dialog that state is expressed by clearing the field's checkbox rather than by typing a `0`
into it — so the spinner never has to produce a value whose meaning is not a duration. Every
other range appears identically in all three places.

Example log output for an invalid value, as the structured `key=value` sink renders it:
```
[ERROR] Invalid config value, using default field=brightness.step_percent value=999 min=1 max=50 default=5
[ERROR] Invalid config value exceeds maximum, using default field=refresh.periodic_seconds value=99999 max=3600 default=60
```

A *type* error never reaches this path. `osd.timeout_ms` is a `u32`, so a hand-written `-5`
fails in serde before `validate_and_fix` ever runs; the file is then treated as unparseable
and goes down the backup-recovery route described below, not the repair route described here.
Only a value that parses and then falls outside its range is repaired against the table.

**Atomic Writes & Backup Recovery**

Config writes are atomic: the file is written to `config.json.tmp` and then renamed over `config.json` (atomic on a single volume), so a crash, power loss, or full disk mid-write can never leave a truncated file — the previous content survives.

After every *successful* parse at startup, the validated settings are mirrored to `config.json.bak` (also written atomically, best-effort — a backup failure never blocks startup). The backup therefore always holds the last-known-good configuration, including hand-edits that parsed successfully.

When `config.json` is unreadable or corrupt (typically a broken hand-edit, reached through the settings window's "Open config file" footer link or the file itself):
1. Settings are recovered from `config.json.bak` and a warning is logged — user settings are **not** silently replaced by defaults.
2. Only when the backup is also missing or corrupt do defaults substitute (logged as error).
3. The corrupt `config.json` is left untouched in both cases, so the user can inspect and fix their edit; it is not overwritten until the next successful save.

**Adding or changing a field**

A field is not one edit but a dozen, spread across core, the settings window and the docs,
and nothing but this list makes them findable. Two of them are silent when forgotten: a field
with no arm in `validate_and_fix` accepts garbage instead of repairing it, breaking the
never-fatal contract above; a field missing from `Config::overlay_dirty` is dropped whenever
a save merges onto an externally edited file (§14), so it appears to save and then vanishes.

In `core/config.rs`:

1. The struct field with a serde default, so an older config file without the key still
   loads: `#[serde(default = "default_…")]` plus the function, or the bare
   `#[serde(default)]` when `Default::default()` is already the value you want.
2. An arm in `validate_and_fix` that substitutes the default for an out-of-range value and
   logs an error — never fatal.
3. A line in `Config::restore_defaults`, unless the field is deliberately preserved across a
   reset (as `monitors` and `version` are).
4. A flag on `SettingsDirty` and a guarded copy in `Config::overlay_dirty`, if the settings
   window can change the field.

In the controller and the settings window, if the field is user-editable:

5. A `SettingChange` variant and its arm in `Controller::handle_setting_changed`, applying
   the change live where that is possible and marking the dirty flag.
6. A `ControlSpec` row in `settings/layout.rs`'s `CONTROLS` table — creation order is also
   tab order.
7. For a numeric field, a `RangeSpec` in the same file's `RANGE_SPECS`. This is the third
   place the range appears, after the validator and the table above; all three must agree —
   apart from a documented "0 means disabled" spinner exception — and they enforce it
   differently on purpose (see the notes under the table).

In the docs:

8. The range/default row in the table above, and the key in this section's JSON sample.
9. The README's configuration section — the sample block and the per-field description.
10. A `[Unreleased]` entry in `CHANGELOG.md`, written from the user's point of view.

**When `version` has to be bumped:** only when an existing field is removed, renamed, or
changes meaning — a reader of an older file would otherwise misinterpret it. Adding an
optional field with a default does not qualify, because the loader already tolerates its
absence, and neither does widening a valid range. There is still no migration logic (see the
`version` note above), so a bump is a decision to write some.

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
give-up state — and both the DDC worker and the hotkey thread are gated by it.
One policy, two *separate* instances of it: `DdcSupervisor` owns its own gate as
a field, `main.rs` builds a second one (`hotkey_gate`) for the hotkey thread.
Nothing is shared between them, so one subsystem's crash loop can never spend
the other's respawn budget. What differs is not the policy but the way back out:

- The **DDC worker** has external recovery triggers, so `DdcSupervisor` calls
  `RespawnGate::reset()` when one fires (a brightness keypress or a system
  resume) and the gate reopens. The supervisor itself only translates a
  decision into an action — spawn a replacement, or report that none was
  spawned — because `RespawnOutcome` is what the controller needs to know.
  Deliberately, a failed restart *attempt* does **not** latch here, unlike the
  hotkey case below: the next keypress or resume would lift the latch moments
  later anyway, and the attempt has already been charged against the respawn
  budget, so the retries stay bounded without it.
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
│ Settings                                        │  → Opens the settings window (§14)
│ Open Log Folder                                 │  → Opens %APPDATA%\BrightnessControl in Explorer
│ Quit Brightness Control                         │  → Graceful shutdown
│─────────────────────────────────────────────────│
│ Brightness Control v0.9.0+55.gc4687e5 (dev)     │  ← Version (disabled/info only)
└─────────────────────────────────────────────────┘
```

**Version Row:**

The row names the build, not just the release. `core::version` combines the
package version with `git describe` output that `build.rs` passes in at
compile time, and appends a `+<metadata> (dev)` suffix **only** when the build
is not a clean checkout of the tag matching the package version — so a
released binary shows a bare `Brightness Control v0.9.0` and anything built
further along the cycle identifies its commit. A build with no git available
at all (an unpacked source archive) also shows the bare version, which is the
right answer for it. The same string opens the log and appears in the settings
window footer (§14), so a bug report carries it whichever way it is written.
The release workflow checks out with full history for this reason: a shallow
checkout without the tag would stamp a release binary as a dev build.

**Monitor Status Rows:**
- Displayed at the top of the menu as disabled (non-clickable) items; Windows'
  keyboard menu navigation skips grayed items too, so a monitor row is reachable
  only by mouse
- Show current overlay opacity (🕶) and hardware brightness (🔆) for each monitor
- Built when the menu opens via `TrayMenuOpening` request/response, then refreshed
  every 250 ms for as long as it stays open — a `SetTimer` started immediately
  before `TrackPopupMenu` and killed immediately after, so an idle process does no
  periodic work. Each tick re-runs the same pull and rewrites only the rows whose
  text changed, addressed by command ID via `SetMenuItemInfoW`.
- Three measured Win32 behaviours carry that design, none of them documented by
  Microsoft: `WM_TIMER` is delivered inside the modal `TrackPopupMenu` loop (and is
  not starved by mouse motion over the popup); a visible popup repaints itself after
  `SetMenuItemInfoW`, with no `InvalidateRect` on the `#32768` window; and a row that
  outgrows the popup widens it rather than being clipped — at the right work-area
  edge it grows leftward, so the popup only ever gains area. Verified light and dark
  on Windows 11 build 22621. If a future build stops repainting, the row simply keeps
  its old text, which is the behaviour this replaced — no error path is needed.
- A refresh may only rewrite row `i` while row `i` still means the same monitor, so
  each tick compares the display-name list against the one drawn. Any difference —
  hot-plug, pruning, reordering — stops refreshing for that menu and leaves the rows
  standing. Warning lines and the usage rows are likewise fixed at open: they change
  the menu's height, which an in-place text update cannot do.
- Each tick's round trip to the main thread waits at most `MENU_POLL_TIMEOUT` (50 ms):
  the tray thread pumps nothing else inside the modal loop, so the wait itself is a
  frozen menu, and it must stay short. Two consecutive unanswered polls
  (`MENU_POLL_MISS_LIMIT`) end refreshing for that menu session, logged as "Main
  thread did not answer; tray menu rows stay as they are". Every giving-up condition —
  a missed poll limit, a display-name mismatch, a failed `SetMenuItemInfoW` — leaves
  the rows exactly as last written rather than retrying or clearing them.

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

Three disabled rows, sourced live from the same `TrayMenuOpening` reply as the
monitor status rows above — a user who rebound the hotkeys in the settings
window (§14) is taught the keys that actually work, not the ones the process
started with. Everything after a `	` is drawn right-aligned in the menu's
shortcut column, which is where a reader already scans for a key combination
and what keeps the rows no wider than the monitor status lines above them.

**Implementation Notes:**
- The rows carry no dark-mode handling of their own: they are part of the menu, which already follows the system setting (see Menu Theming above)
- The bindings arrive with each `TrayMenuOpening` reply, so a rebind shows up the next time the menu opens; if the request times out, the tray thread keeps a thread-local copy of the last pair it did receive and shows that rather than nothing
- Disabled (`MF_GRAYED`) throughout — informational, never clickable
- The live refresh of the monitor rows above decides whether a row still means the same
  monitor by comparing the display-name vector between polls, and gives up (silently, at
  `debug`) if the vector differs. That rests on a contract in `core/state.rs`: display-name
  generation sorts by (manufacturer, model, serial) before assigning `#N` suffixes, so the
  numbering is independent of the caller's `HashMap` iteration order and two identical
  panels cannot swap numbers between one reading and the next. Both that sort and the one
  ordering the menu rows have to stay stable together.

A modeless window held these two lines before, opened from a "Usage" menu item.
It was removed with the move: a window class, centring, focus handling and a
theming path of its own — dark mode included — is a large apparatus for one
sentence the menu can simply hold. The rows cost none of that and cannot fall
out of step with the menu around them. What went with the window is the option
to leave the instructions on screen while trying the keys.

### 14. Settings Window

The tray menu's "Settings" item opens a native settings window instead of
`config.json` in a text editor — the file stays reachable through the
dialog's own "Open config file" footer link (see "Message flow" below). The
window exposes every existing config option plus a "Start with Windows"
toggle, applies changes live, and follows the system light/dark theme like
the tray menu already does.

**Own thread — load-bearing, not stylistic.** The window is spawned on a
dedicated thread with its own `GetMessageW` loop, the same pattern the tray
and power threads use. Ordinary interactions with a titled window — dragging
it by the title bar, an Edit control's built-in right-click menu — enter
OS-internal *modal* message loops that block the calling thread until the
interaction ends. Run on the main thread, that block would stall controller
ticks and the channel drain long enough to trip the refresh watchdog
(`REFRESH_TIMEOUT`, §12) if it outlasted an in-flight refresh. On its own
thread, a modal loop blocks only the dialog. The thread is spawned per open
and exits when the window closes — no supervision needed; if it dies, the
user reopens Settings. A second "Settings" click while the window already
exists focuses it instead of spawning a duplicate.

**Message flow.** The controller stays the sole owner of the runtime
`Config`. Three new host-fakeable seams join `OsdSink`/`OverlaySink`/`DdcPort`/
`MonitorLocator` in `core/controller.rs`, bringing the total to seven:

| Seam | Responsibility | Windows implementation |
|---|---|---|
| `SettingsSink` | open/focus the window with a config snapshot, refresh displayed values (restore defaults, reverts), show inline hotkey errors/notices, re-assert `HWND_TOPMOST` | `settings::SettingsSinkImpl` |
| `HotkeyPort` | `rebind`/`suspend`/`resume` — posts an in-place operation to the live hotkey thread; results arrive async as `BrightnessMessage::HotkeyRebindResult` | posts to the hotkey thread |
| `ConfigStore` | `save(&Config, &SettingsDirty, force) -> SaveResult` | `config_store::WindowsConfigStore` |

`TrayOpenSettings` is handled in the controller, not intercepted in
`main.rs`, matching the project's orchestration-lives-in-the-controller rule.
Control edits in the dialog post `BrightnessMessage::SettingChanged(SettingChange)`
into the existing MPSC channel — the same path the tray uses — one enum
variant per setting, values pre-parsed (opacity as the UI's integer percent,
converted to the config's `0.1–1.0` float in core). The window posts
`SettingsClosed` on destruction so the controller can flush a pending save,
and `OpenConfigFile` for its "Open config file" footer link (handled like
`TrayOpenLogFolder`, a shell side effect outside the controller).

**Instant apply, debounced saves.** Every change applies immediately —
including hotkey rebinds, live on the hotkey thread — except the logging
options, which only take effect after a restart (a static label in the
dialog says so). Brightness step became controller-owned as part of this:
hotkey events now carry a direction only (`AdjustStep`), and the controller
multiplies by its live `config.brightness.step_percent`, so a changed step
applies without the hotkey thread knowing about it. Saves are debounced
(`SAVE_DEBOUNCE`, 500 ms, `core/reconcile.rs`) and dirty-gated: only fields a
dialog session actually touched are marked dirty, and a run that never opens
the dialog never rewrites `config.json`. Close and quit flush a pending save
immediately regardless of the debounce window — the flush itself is still
dirty-gated, so a session with nothing to save still writes nothing. When a
hotkey rebind fails and the controller reverts the hotkey fields, the dirty
flags the dialog just set are deliberately left set: the reverted config is
what belongs on disk, so saving it is a no-op if nothing else was pending and
correct if an earlier, still-unsaved change sits in the same fields — clearing
the flags would silently drop that earlier change.

**Merge-on-external-change.** `WindowsConfigStore` tracks the config file's
identity (length + modified time — cheap to stat, and any edit that matters
also changes one of the two) as of its last read or write. A save that finds
the file unchanged since then writes straight through. A save that finds it
changed re-reads the file with a *raw* `serde_json` parse — deliberately
**not** the loader's `validate_and_fix`/default-substitution repair path,
because a file that only parses after repair counts as unparseable here and
must not be silently discarded — then calls `Config::overlay_dirty` to copy
only the fields this dialog session actually changed onto the disk config,
and writes the merge. An external hand-edit of an untouched field therefore
survives on disk. If the changed file no longer parses at all, the save
result is `SaveResult::Deferred` (stays dirty, retried on the next change) —
except with `force: true` (close/quit), which overwrites rather than losing
the session's changes; an unparseable file would be default-substituted at
next start anyway, and the `.bak` recovery path (§4) still applies. A failed stat on a
file the store has a baseline for counts as "possibly changed", not "unchanged": a
transient permissions error or momentary lock must not fall through to a full overwrite of
a file that may have just been hand-edited. A genuine not-found (the file was deleted)
is not a stat failure in that sense and keeps the direct write.

**Capture suspension protocol.** While `RegisterHotKey` owns a combination,
pressing it delivers `WM_HOTKEY` to the hotkey thread instead of a keystroke
to the focused control — so typing the default `Ctrl+Shift+Up` into the
capture field would adjust brightness instead of being captured. Entering
capture mode posts `HotkeyOp::Suspend` to the hotkey thread; every exit path
(capture completes, Esc, focus loss, window destroyed) posts `Resume` or a
`Rebind`. Window destruction is the safety net under all of them: the
controller ends capture on `SettingsClosed` unconditionally, so even a missed
explicit post cannot leave interception suspended once the window is gone.
Suspension covers everything that intercepts keys: the primary
combinations, the secondary plain `VK_BRIGHTNESS_UP/DOWN` registrations, and
the low-level hook. `REBIND_TIMEOUT` (3 s, `core/reconcile.rs`) is an ack
deadline **per posted round-trip** — each `Suspend`/`Resume`/`Rebind` gets its
own ack — never a bound on capture itself, which is user-paced and
unbounded: a user may sit in "Press a key combination…" for a minute without
the controller silently re-registering hotkeys underneath the field or
declaring hotkeys degraded. While capturing, the control also swallows
`WM_KEYUP`/`WM_SYSKEYUP` itself instead of passing them to `DefWindowProcW`:
releasing Alt mid-capture would otherwise raise `SC_KEYMENU`, pop the
window's system menu, steal focus and thereby fire `WM_KILLFOCUS` — silently
cancelling a capture the user was still in the middle of. The controller
reconciles the races: hotkey thread respawned while suspended → immediately
re-post suspend to the new thread; settings window gone while suspended →
post resume. Known
unverified: killing the hotkey thread while a capture is suspended, to
confirm the respawn-then-re-suspend race above end to end, is not something
that can be staged safely against a live thread — `RespawnGate` itself is
unit-tested instead.

**Two degraded hotkey states, now genuinely distinct.** `HealthWarnings`
carries both `hotkeys_lost` (§12's permanent supervision give-up, ends only
at restart) and the new `hotkeys_degraded` — set by a failed rebind, a failed
suspend/resume, or a missed ack, and cleared by the next successful hotkey
operation. A failed rebind is fixable by trying another combination in the
still-open dialog, so it must never route through `set_hotkeys_lost()`: doing
so would mean a recoverable dialog-level hiccup permanently disables hotkeys
in the tray's eyes. Rebind itself stays optimistic with revert, mirroring the
brightness pipeline's `apply_set_result`/`force_revert`: the controller
applies a new binding to its config immediately and reverts it (notifying
`SettingsSink`, which restores the previous binding in the capture field) if
the ack reports failure or never arrives.

**Sticky topmost, above the overlay.** The dimming overlay and the OSD are
both `WS_EX_TOPMOST`, and re-assert `HWND_TOPMOST` on every update — that
re-assertion, not the style alone, is what keeps them visible over each
other. "My screen is dark, let me open Settings" is this app's headline
scenario, so the settings window is `WS_EX_TOPMOST` too, and the controller
calls `SettingsSink::assert_topmost()` after every overlay update while the
window is open — otherwise one dim keypress after opening Settings would
bury a one-shot-topmost dialog beneath the click-through black overlay,
receiving input the user cannot see.

**Dark mode.** Extends `theme.rs`'s existing uxtheme opt-in (§13) with a new
`theme::system_prefers_dark()` (reads the `AppsUseLightTheme` registry value)
and paints dark only when `theme::dark_ui_available() && system_prefers_dark()`
both hold — where the uxtheme opt-in itself is unavailable (pre-18362, or a
missing ordinal), the window paints light, full stop, because dark brushes
under un-themed light controls would be dark-on-dark and unreadable. Within
that gate: `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)` for the
title bar; `SetWindowTheme` plus `WM_CTLCOLOR*` handlers for buttons, edits
and the combo's popup list; checkbox labels are custom-drawn via
`NM_CUSTOMDRAW` — a themed `BUTTON`-class control paints its own text via
`DrawThemeText` and ignores `WM_CTLCOLORSTATIC`'s colour entirely; in the
dark-mode spike its labels measured 1.27:1 contrast against the dark
background while plain `STATIC` labels in the same window measured 14.3:1.
`NM_CUSTOMDRAW` on buttons is a Microsoft-documented surface since Windows
Vista, not part of the undocumented-ordinal surface `theme.rs` isolates; the
combo face (arrow + selected text) is drawn through
`DrawThemeBackground`/`DrawThemeTextEx` against the `DarkMode_CFD::COMBOBOX`
theme class, with a hand-drawn GDI fallback whenever `OpenThemeData` or the
theme draw call fails — that fallback path has been exercised on real
hardware during development, not just as a theoretical branch. The
five numeric edits are stripped of their visual-styles theme association
entirely (empty sub-app/sub-id name) — not to disable painting, but because
a themed edit ignores `WM_CTLCOLOREDIT`'s colours outright, so detachment is
what lets that handler's interior colours apply at all; their border is then
hand-painted on a direct, per-child `WM_NCPAINT` message the window sends
itself (neither `RDW_FRAME` nor `RDW_ALLCHILDREN` reaches a child's
non-client area, so nothing shorter actually triggers one), correct
immediately after every theme application but overwritten by comctl32 v6's
own direct-painted `COLOR_WINDOWFRAME` border on every focus transition and
at first show regardless of the detachment — measured on hardware as a 1px
colour delta and accepted rather than chased further. The updown spinner is
a different case: hand-painted from its own `WM_PAINT`/`WM_ERASEBKGND`
subclass as ordinary client-area content, covered by the general
`RDW_ALLCHILDREN` client-area invalidation rather than needing the edits'
direct-message treatment. Group boxes were dropped from the design entirely:
a spike found `BS_GROUPBOX`'s frame and caption unreadable in dark mode, so
each group is a bold `STATIC` label plus an `SS_ETCHEDHORZ` separator
instead — which also removes any z-order dependency between a frame and the
controls inside it. The window handles live `WM_SETTINGCHANGE`
(`"ImmersiveColorSet"`) and re-themes immediately; unlike the tray's popup
menu (§13), this is a genuine top-level window and does receive the
broadcast, so no per-open refresh trick is needed here. One ordering
constraint at creation: the window state is published to its thread-local
*before* the first theme application, because `apply_theme`'s `RedrawWindow`
synchronously re-enters every child's paint path, and each of those reads the
live dark flag through that thread-local — while it is still empty the very
first repaint would fall back to light regardless of the actual setting.

The footer's merged SysLink also needed its own keyboard-activation
subclass, unrelated to theming: past the first embedded link, the control's
native `NM_RETURN` delivery on Enter proved unreliable (mouse clicks on
either link were never affected), so the subclass tracks link focus itself
and dispatches Enter directly instead of waiting on a notification that
might never arrive.

**DPI.** The window is created at the DPI of the monitor under the cursor
(`MonitorFromPoint` + `GetDpiForMonitor`, computed before `CreateWindowExW`),
reusing the scale-factor arithmetic `osd.rs` already has. `WM_DPICHANGED`
resizes to Windows' suggested rect, rebuilds both fonts at the new size,
swaps them into the window's state before re-sending `WM_SETFONT` to every
control (so a paint reentered synchronously from that already sees the new
font handle), and relayouts every control from the same baseline table
`create_settings_window` used initially. Known unverified: dragging the
window across two monitors at different DPIs has not been exercised on real
hardware; `WM_DPICHANGED` has instead been confirmed through a live
per-monitor scale-factor change on a single monitor, which drives the same
handler. Placement centres the window on the *work area* (`rcWork`, taskbar
excluded — the area shell dialogs centre on) of the monitor under the cursor,
clamped so the top-left corner always stays inside it; the clamp's upper
bound is itself floored at the work area's origin, because this window is
tall enough that on a short work area at high DPI (150% on 1920×1080) it can
exceed the work area, and it must then start at the top-left corner rather
than be pushed above or left of the screen.

**Focus save/restore.** A programmatic (non-template) `CreateWindowExW`
window gets none of a real dialog's automatic keyboard-focus bookkeeping: a
modal `MessageBoxW` closing, or Alt+Tab deactivation/reactivation, has
nowhere to hand focus back to except the top-level window itself — which
`IsDialogMessageW`'s Tab handling can then never move focus out of, because
there is no dialog-manager state recording which control had it. This was
found the hard way (focus stranding on the window after a message box
closed) and is now handled by hand: `WM_ACTIVATE` saves the focused child on
deactivate and restores it on reactivate, and `WM_SETFOCUS` self-heals
whenever focus lands on the top level through some other path.

**Uncommitted edits on Close.** A value typed into a numeric edit but not yet
committed (no `EN_KILLFOCUS` yet) still survives Close or Esc without an
explicit commit step: `DestroyWindow` moves focus off the focused edit before
tearing the window down, so that edit's `EN_KILLFOCUS` — and the commit it
triggers — is processed before `WM_DESTROY` clears the window state and posts
`SettingsClosed`. Verified on hardware, but it is a dependency on Windows'
message ordering; should a future build ever deliver `WM_DESTROY` first, the
Close handler would need an explicit commit before posting `WM_CLOSE`.

**Autostart** (`platform/windows/autostart.rs`) is registry-backed, not a
config field: `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value
name `darkbright-helper`, is the single source of truth. The checkbox reads
the actual registry state when the dialog opens and writes it directly on
toggle; nothing is mirrored into `config.json`, so there is no second store
to drift. Because the app ships as a portable zip that can be moved or
re-extracted, enabling always rewrites the value with the *current* exe path
(self-healing a stale entry) and clears any `StartupApproved\Run` veto that
Task Manager's Startup tab may have set — Task Manager disables an entry by
writing that separate, undocumented key while leaving `Run` itself in place,
so a Task-Manager-disabled entry reads as enabled here; toggling off and back
on heals it. "Restore defaults" leaves autostart untouched — it is system
integration, not a preference, and silently removing it would surprise. A
failed registry write reverts the checkbox and shows an inline notice.

**Version line.** The footer carries the same build string as the tray's
version row (§13), greyed, in the space left of the buttons. It is the one
control whose caption is not a constant in the layout table — the string only
exists once the build has run — so control creation substitutes it for that
single id. Its width is what remains before "Restore defaults", which bounds
what it can show: the longest realistic string, a three-digit commit count on
a dirty tree, measures 165px in the dialog font at 96 DPI against 172px of
room. A released build needs 30px of it.

**Module placement.** `src/platform/windows/settings/` is a directory
module: `mod.rs` (module wiring, `pub use` re-exports), `layout.rs`
(declarative control table + DPI geometry, no window logic), `window.rs`
(window creation and the per-open named `settings` thread — a
`thread::Builder`, per the naming rule under Thread Conventions — control
wiring, the message loop, `SettingsSinkImpl`), `capture.rs` (the hotkey-capture control,
its own subclassed window class), `dark.rs` (all dark-mode painting: custom
draw, subclassing, the `WM_CTLCOLOR*` colour table). Two supporting pieces
live outside the directory, alongside the platform module's other
single-file seams: `platform/windows/autostart.rs` and
`platform/windows/config_store.rs` (`WindowsConfigStore`, the `ConfigStore`
implementation above).

### 15. Single-Instance Guard

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

### Tracking tooltips via `tooltips_class32` do not work

Kept as a negative result: the app has no menu tooltips today, and this is the record of why
a future attempt should not start with the obvious API.

The Win32 tooltip control is unreliable for *tracking tooltips* — manually positioned
tooltips near a popup menu. `TTM_ADDTOOLW` can fail silently, returning 0 with
`GetLastError()` also 0, even with correct parameters. Configurations tried, all failing the
same way: `hwnd` as `HWND_MESSAGE`, as `GetDesktopWindow()`, as the tooltip's own handle, and
as a dedicated invisible `STATIC` window; various combinations of `TTF_TRACK`, `TTF_ABSOLUTE`
and `TTF_IDISHWND`; and a verified-correct `cbSize` (72 bytes on 64-bit).

What works instead is a plain custom popup: a `STATIC`-class window with
`WS_POPUP | WS_BORDER | WS_EX_TOPMOST | WS_EX_TOOLWINDOW`, shown with `ShowWindow` and placed
with `SetWindowPos`. More reliable, and it gives full control over appearance — which a
themed tooltip would not.

---

## Testing

### Unit Tests

`cargo test --locked` runs them; `.github/workflows/ci.yml` is the authority on the full
check set, and `docs/code-conventions.md` §8 covers how the tests themselves are built.

Key test areas:
- **Config validation**: Ensures invalid values are clamped to defaults
- **Brightness calculations**: Tests adjustment logic in `core/brightness.rs`
- **State management**: Tests `MonitorState` transitions
- **Controller orchestration**: `core/controller.rs` drives the optimistic-update, supervision, watchdog, refresh, and ghost-pruning sequences against fakes for all seven seams (`OsdSink`, `OverlaySink`, `DdcPort`, `MonitorLocator`, `SettingsSink`, `HotkeyPort`, `ConfigStore`) — the message-driven control flow is unit-tested on any host, no Windows target or physical monitor required

### Integration Testing (Manual)

Controller orchestration is unit-tested (see above); what remains hardware-dependent and must be tested manually is DDC/CI I/O against real monitors, the DDC worker's EDID enumeration (including the `enumerated` set it reports), and topology changes:

**Keeping these procedures true.** For DDC, the OSD, the overlay, the tray and power events
these procedures are the whole verification story, because CI cannot run any of them. So
a change to one of those paths updates its procedure in the same PR. In practice that means
the **Expected** line rather than the steps: the steps age slowly — a monitor is still
unplugged by unplugging it — while the expected observation goes stale the moment a log
message or a visible behaviour changes. A procedure whose Expected line no longer matches is
worse than no procedure at all, because it fails on a correct build and teaches whoever runs
it to wave the failure through.

There is deliberately **no record of which procedure last ran against which version**. Such a
note is only ever as good as its upkeep, and a stale one claiming a verification that never
happened would be worse than the silence it replaced — the same failure this document's
freshness rule exists to prevent. The verification history therefore lives in the commit and
release history, not in a table here.

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

#### Live Tray Menu Rows Test
1. Start the application with `RUST_LOG=debug`
2. Open the tray menu and, with it still open, press a brightness hotkey
3. **Expected**: the row follows within ~250 ms and matches the OSD
4. Hold the hotkey through key-repeat
5. **Expected**: the row counts along without flicker; the menu may widen once and
   must not shrink back or jitter
6. Hover a monitor row itself, then press the hotkey while the cursor stays there
7. **Expected**: that row updates and keeps the mouse highlight
8. Hover "Settings" instead, then press the hotkey
9. **Expected**: the highlight stays on "Settings", undisturbed
10. Arrow-key down from the top of the menu — the grayed monitor rows are skipped,
    so the keyboard selection lands on "Settings" — then press the hotkey
11. **Expected**: the keyboard highlight is undisturbed
12. Repeat on a dark system theme
13. **Expected**: the updated row keeps the system menu colours
14. With two identical monitors connected, keep the menu open for a minute
15. **Expected**: the `#1` and `#2` rows keep their monitors for the whole session
16. Unplug a monitor while the menu is open
17. **Expected**: rows freeze, "Monitor set changed while the tray menu was open"
    appears in the log, nothing crashes, and the next open is correct
18. With the menu still open, right-click the tray icon a second time
19. **Expected**: nothing breaks; the menu either stays up or reopens. Whichever
    happens is timing-dependent — if the re-entrancy guard fired, the log carries
    "Tray menu already open; ignoring the second open"
20. Close the menu
21. Restart the application with `RUST_LOG=trace`
22. Open the tray menu again and leave it open for a few seconds
23. **Expected**: per-tick `TrayMenuOpening` lines appear in the log, about four
    times a second
24. Close the menu
25. **Expected**: no further `TrayMenuOpening` traffic, and the idle main-loop
    cadence is unchanged

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

#### Settings Window Test

The controller's own logic (every `SettingChanged` variant, debounced save timing, rebind success/failure/revert, the capture-suspension reconciliation rules, restore-defaults, the merge-on-external-change save path) is covered by controller unit tests against fake seams. What remains — real window behaviour, real theming, real hardware timing — is manual, carried over from the settings-window design spec's Testing section:

- Live light/dark switch with the dialog open
- Dragging across monitors with different DPI
- Hotkey capture, including a conflicting global hotkey
- Rebinding repeatedly within a minute (the crash-loop gate must stay untouched)
- Rebinding one action while the other keeps its combination
- Rebind → kill the hotkey thread → verify the *new* binding survives the respawn
- Open Settings on a monitor at 0% with the full black overlay (window must be visible above it) and then dim further (window must stay on top — the overlay re-asserts `HWND_TOPMOST` on every update)
- Leave capture mode by every path (complete, Esc, click elsewhere, close the window) and verify hotkeys work again each time
- Dragging the settings title bar during a refresh (main loop must be unaffected — own thread)
- Autostart registry entry appears/disappears
- Logging restart hint
- Both footer links

---
