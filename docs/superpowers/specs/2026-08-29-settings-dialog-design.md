# Settings Dialog — Design

Date: 2026-08-29
Status: design approved in brainstorming, hardened by two cold adversarial
review rounds (2026-08-29/30); implementation plan pending.

## Motivation

The tray menu's "Settings" item currently opens `config.json` in the default
editor. That serves power users but overwhelms everyone else. This feature
replaces it with a native settings window that exposes every existing option,
keeps the JSON file as an escape hatch, and follows the system light/dark
theme like the tray menu already does (hard requirement).

## Decisions at a glance

| Decision | Choice |
|---|---|
| Option scope | The 10 existing config options + a new "Start with Windows" toggle. Per-monitor settings stay out (reserved, unimplemented feature). |
| Apply semantics | Mixed, maximally live: everything applies immediately — hotkey rebinds in place on the live hotkey thread — except logging options, which get a "takes effect after restart" hint. |
| UX model | One compact dialog with group boxes (General / Hotkeys / On-screen display / Advanced); no tabs, no sidebar. |
| Commit model | Instant apply (Windows 11 Settings style): every change applies and saves immediately (debounced). Buttons: "Restore defaults" and "Close" only. |
| Technology | Raw Win32 via the `windows` crate, programmatic `CreateWindowExW` + stock controls, no `.rc` templates, zero new dependencies. |
| Dark mode | Hard requirement. Extends `theme.rs`; live re-theme on system setting change. |
| External file edits | No live reload in this iteration. The apply pipeline is deliberately shaped (diff → typed change messages) so a manual reload action or a file watcher can later be added as pure message producers. |
| Language | English UI strings, matching the tray menu and OSD. |

## Scope

**In:**
- Settings window exposing: `brightness.step_percent`, autostart (new),
  `hotkeys.brightness_up`, `hotkeys.brightness_down`,
  `hotkeys.intercept_brightness_keys`, `osd.timeout_ms`, `osd.opacity`,
  `refresh.periodic_seconds`, `refresh.inactivity_seconds`,
  `logging.file_enabled`, `logging.file_level`.
- Live application of all non-logging settings.
- Dark-mode-responsive, per-monitor-DPI-correct dialog.
- "Open config file" escape hatch in the dialog footer (replaces the old
  behavior of the tray "Settings" item).

**Out (explicitly):**
- Per-monitor settings (`monitors: {}` stays reserved; activating it means
  implementing clamping/disable logic in the DDC path — its own feature).
- Live reload of externally edited `config.json` (watcher or manual reload;
  see "Future extensions").
- Localization; Mica/Fluent visual styling beyond correct light/dark palette.
- New config fields: there are none — autostart is *not* stored in the
  config file (see below).

## Architecture

### Window ownership

The settings window lives on its **own dedicated thread** with its own
`GetMessageW` loop — the same pattern the tray and power threads use, and
for the same reason: ordinary interactions with a titled window (dragging
by the title bar, an Edit control's built-in right-click menu) enter
OS-internal **modal message loops** that do not return until the
interaction ends. On the main thread that would freeze controller ticks,
watchdogs and the channel drain for the duration — verified consequence: a
block longer than `REFRESH_TIMEOUT` (5 s) during an in-flight refresh
triggers the watchdog abort, which clears `last_enumerated` and silently
freezes periodic resync until a hotkey press or resume revives it. On its
own thread, a modal loop blocks only the dialog itself. The main loop is
untouched (no `IsDialogMessage` there; the settings thread runs
`IsDialogMessage` in its own loop).

The thread is spawned per open and exits when the window closes — no
long-lived idle thread, no supervision needed (if it dies, the user
reopens). A second "Settings" activation while the window exists focuses
it instead of duplicating (the thread publishes its HWND as an `isize`,
per the existing handle-carrying convention).

Keyboard navigation truths for a programmatic (non-template) window:
every focusable control carries `WS_TABSTOP` (+ `WS_GROUP` at group
boundaries), and **tab order follows creation order** (z-order), so
controls are created in visual order and group-box frames are created
before the controls they surround. `WS_EX_CONTROLPARENT` is *not*
required on a flat window whose controls are all direct children — it is
the style for recursing into nested containers, which this layout avoids.
The hotkey-capture control additionally answers `WM_GETDLGCODE` (see
"Control decisions").

**Topmost, or invisible — and topmost must be sticky:** the dimming
overlay and the OSD are both `WS_EX_TOPMOST`; a normal-z settings window
on a monitor dimmed to 0 % would sit *beneath* the click-through black
overlay — receiving input the user cannot see. "My screen is dark, let me
open Settings" is this app's headline scenario, so the dialog is
`WS_EX_TOPMOST` too. But the style alone is not enough: the overlay
re-asserts `HWND_TOPMOST` on **every** opacity update, and the OSD on
every show — that re-assertion, not the style, is how the OSD actually
stays visible. One dim keypress after opening Settings would bury a
one-shot-topmost dialog again. Therefore: after any overlay update while
the window is open, the controller has the `SettingsSink` re-assert the
dialog's `HWND_TOPMOST` (a cheap `SetWindowPos` with
`SWP_NOMOVE|NOSIZE|NOACTIVATE`).

**Placement:** on the monitor under the cursor (the app's universal
targeting rule), geometry computed *before* `CreateWindowExW` via
`MonitorFromPoint` + `GetDpiForMonitor`. The *functions* have precedent in
`osd.rs`; computing geometry before window creation does not (the OSD is
created once with `CW_USEDEFAULT` and positioned per show) — this is new
sequencing, stated so nobody hunts for a pattern that isn't there.

### Message flow (single-owner model preserved)

- Tray "Settings" sends `BrightnessMessage::TrayOpenSettings` as today, but
  the message is **no longer intercepted in `main.rs`**: the controller
  handles it and calls the new `SettingsSink` seam to open/focus the window,
  passing a snapshot of the current config values. (Orchestration moves into
  the controller, per the project rule; opening the JSON file becomes a
  dialog-footer action instead. The controller's combined
  `TrayOpenSettings | TrayOpenLogFolder` safety-net arm splits accordingly.)
- Control changes in the dialog post typed
  `BrightnessMessage::SettingChanged(SettingChange)` messages into the
  **existing MPSC channel** (same path the tray uses). One enum variant per
  logical setting; values arrive pre-parsed (e.g. opacity as the UI's
  integer percent, converted in core). Exception: the autostart toggle
  never enters the channel — it is a pure registry operation handled in
  the platform layer (see "Autostart").
- The dialog additionally posts `BrightnessMessage::SettingsClosed` when
  the window is destroyed, so the controller can flush a pending save.
- The controller remains the sole owner of the runtime `Config`: it mutates
  its field, applies side effects through seams, and schedules a debounced
  save through the `ConfigStore` seam (see "New seams").

### Side-effect routing per setting

| Setting | Live effect | Where |
|---|---|---|
| `step_percent` | hotkey events become direction-only; the controller multiplies by its live config value (see below) | controller |
| `osd.timeout_ms`, `osd.opacity` | `OsdSink` gains an appearance-update method | controller → seam |
| `refresh.*` | controller timers already read the config field each tick | controller |
| hotkeys (both bindings + intercept flag) | in-place re-registration on the live hotkey thread via a posted thread message (see below) | controller → `HotkeyPort` seam, result reported back |
| autostart | registry write | dialog/platform layer directly (not config) |
| `logging.*` | none — save only; `SettingsSink` shows the restart hint | controller → seam |

**Brightness step becomes controller-owned.** Today `step_percent` is
frozen into the hotkey thread at spawn (`HotkeyManager` and the low-level
hook context each hold a copy) and the delta is pre-multiplied before
`Adjust` is even sent — the controller never reads the field. To make the
step live, hotkey events switch to a direction-only message variant (±1);
the controller multiplies by its live `config.brightness.step_percent`.
`Adjust { delta }` keeps its absolute-delta semantics for future producers
(tray/CLI). The hotkey thread and hook context stop knowing the step at
all — one frozen copy fewer.

**Hotkey rebind is in-place, not a respawn.** The hotkey thread's
`GetMessageW` loop already exits cleanly on `WM_QUIT`; the same loop can
receive a posted custom message. At spawn the thread reports its Win32
thread id through the existing startup handshake; a rebind posts an update
to the live thread, which unregisters the old combinations, registers the
new ones, and installs/uninstalls the low-level hook as requested. Because
the same thread unregisters before re-registering, an unchanged binding can
never collide with itself (`RegisterHotKey` ownership is per registering
thread, system-wide). On failure the thread re-registers the previous
combinations and posts `BrightnessMessage::HotkeyRebindResult`.

The `RespawnGate` is deliberately not involved: it is a crash-loop breaker
for unplanned deaths, its give-up latch is permanent for the hotkey thread,
and routing deliberate rebinds through it would let four quick edits within
60 s falsely latch "hotkeys lost until restart" — the opposite of a
poweruser-friendly dialog.

Three follow-on consequences the rebind path must handle:

- **The crash-respawn path must use live bindings.** Today
  `start_hotkey_thread(&config, …)` in the respawn arm re-registers the
  *startup* config; after a live rebind, one thread death would silently
  restore the old combinations. The respawn arm queries the controller
  for the current bindings and intercept flag (it already calls
  controller methods every tick) — no `main.rs`-side mirror: the
  controller stays the sole owner of the runtime config. The manual
  checklist covers "rebind, kill the hotkey thread, verify the new
  binding comes back".
- **The tray menu's hotkey rows unfreeze.** The usage rows are currently
  composed once at spawn under the code-documented invariant "the hotkeys
  cannot change while the app runs" — which this feature breaks. The rows
  move into the existing `TrayMenuOpening { reply_tx }` request/response,
  so the menu always shows the current bindings; the stale comment in
  `tray.rs` is corrected.
- **Failure has a deadline and a *clearable* floor.** If re-registering
  the *previous* combination also fails (another app grabbed it
  meanwhile), or no `HotkeyRebindResult` arrives within a wall-clock ack
  deadline (`REBIND_TIMEOUT` in `core/reconcile.rs`, alongside
  `SET_TIMEOUT`/`REFRESH_TIMEOUT`), the tray must warn — but **not** via
  `set_hotkeys_lost()`: that latch is deliberately permanent ("hotkeys
  unavailable until app restart", supervision gave up and means it),
  while a failed rebind is fixable by trying another combination in the
  still-open dialog. Per the codebase's own degraded-state doctrine
  (variants "differ in what ends them"), this is a new, *clearable*
  hotkeys-degraded warning, cleared by any subsequent successful
  rebind/resume ack. `hotkeys_lost` stays reserved for supervision
  give-up. Never a silently hotkey-less app behind a healthy-looking
  icon — and never a healthy app behind a permanently alarmed one.

**Capture must suspend interception.** While a combination is registered,
`RegisterHotKey` delivers it as `WM_HOTKEY` to the hotkey thread instead
of as keystrokes to the focused window — so with the defaults, pressing
`Ctrl+Shift+Up` inside the capture field would *adjust the brightness*
rather than be captured. Entering capture mode therefore posts a
"suspend" to the hotkey thread; leaving it (capture completed, Esc, focus
loss, window destroyed, `SettingsClosed`) posts
resume-with-current-bindings or the rebind — resume is guaranteed on
*every* exit path.

Suspension covers everything that intercepts keys: the primary
combinations, the **secondary plain registrations of
`VK_BRIGHTNESS_UP/DOWN`** (registered whenever the low-level hook is off
or failed), and the hook itself. The same completeness applies to rebind
and to toggling the intercept flag: off→on tries the hook and falls back
to the secondaries on failure (mirroring startup), on→off uninstalls the
hook and registers the secondaries; `HotkeyRebindResult` reports a
partial outcome (hook failed, fallback active) distinctly.

Lifecycle rules — `REBIND_TIMEOUT` is an **ack deadline per posted
round-trip** (suspend, resume, rebind each get acknowledged), never a
bound on capture itself, which is user-paced and unbounded: a user may
sit in "Press a key combination…" for a minute, and neither silently
re-registering hotkeys under the field nor declaring hotkeys degraded is
acceptable for thinking too long. The controller owns a "capture active"
flag and reconciles the races: hotkey thread respawned while suspended →
immediately post suspend to the new thread; settings window gone while
suspended (`SettingsClosed`, or a failed post to its HWND) → post resume;
a missed ack → the clearable hotkeys-degraded warning above.

**Rebind stays optimistic with revert**, mirroring the brightness
pipeline's `apply_set_result`/`force_revert` philosophy: the controller
applies the new binding to its config immediately; on a failed
`HotkeyRebindResult` it reverts the field and notifies the `SettingsSink`,
which shows an inline error and restores the previous binding in the
capture field.

### New seams

Three new host-fakeable traits alongside `OsdSink`/`OverlaySink` in
`core/controller.rs`; controller tests use fakes for all of them.

- **`SettingsSink`** — open/focus the window with a config snapshot,
  refresh displayed values (restore defaults, reverts), show inline errors
  (hotkey registration failure), show the restart hint for logging
  changes. The Win32 implementation (`src/platform/windows/settings.rs`)
  spawns/focuses the settings thread and forwards updates to it via
  `PostMessage` (handles crossing threads as `isize`, per convention).
- **`HotkeyPort`** — `rebind(up, down, intercept)`, `suspend()`,
  `resume()`; the Win32 implementation posts to the live hotkey thread.
  Routing this through the controller (not `main.rs` interception) is what
  makes "controller unit tests cover rebind success/failure/revert" true —
  there is a seam to fake.
- **`ConfigStore`** — `save(&Config) -> Result<()>`; the Win32
  implementation wraps `Config::save_to(default_path())`. Without this
  seam the controller would do real file I/O in tests — and CI runs the
  test suite on `windows-latest`, so every test run would write the
  runner's actual `%APPDATA%\BrightnessControl\config.json`.

`Controller` thus grows three type parameters. If the signature becomes
unwieldy, the implementation plan may group the three settings-related
seams into one trait — a mechanical choice, not a design one.

### Autostart

Single source of truth is the registry
(`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value
`darkbright-helper` = quoted exe path). The checkbox reads the actual
registry state when the dialog opens and writes it directly on toggle.
Nothing is mirrored into `config.json` — no second store, no drift.
"Restore defaults" leaves autostart untouched (system integration, not a
preference; silently removing autostart would surprise).

Edge cases (the app ships as a portable zip, so the exe moves): the
checkbox shows checked iff the `Run` value exists; toggling it on always
(re)writes the value with the *current* exe path, which self-heals a stale
entry, and also deletes any `StartupApproved\Run` veto (Task Manager's
Startup tab disables entries there while leaving the `Run` value in
place). Accepted read-side limitation, documented here: an entry disabled
via Task Manager still shows as checked — reflecting that state would
mean parsing an undocumented binary format; toggling off and on heals
it. A failed registry write reverts the checkbox and shows an inline
notice — the checkbox never lies about what was actually written.

## UI layout

```
┌─ darkbright-helper Settings ─────────────────────┐
│ ┌─ General ────────────────────────────────────┐ │
│ │ ☐ Start with Windows                         │ │
│ │ Brightness step per keypress    [ 5 ]↕ %     │ │
│ └──────────────────────────────────────────────┘ │
│ ┌─ Hotkeys ────────────────────────────────────┐ │
│ │ Brightness up      [ Ctrl+Shift+Up      ]    │ │
│ │ Brightness down    [ Ctrl+Shift+Down    ]    │ │
│ │ ☐ Try to intercept dedicated brightness keys │ │
│ │   (may not work with all keyboards; some     │ │
│ │    antivirus software flags low-level hooks) │ │
│ └──────────────────────────────────────────────┘ │
│ ┌─ On-screen display ──────────────────────────┐ │
│ │ Display duration   [ 1000 ]↕ ms              │ │
│ │ Opacity            [ 100 ]↕ %                │ │
│ └──────────────────────────────────────────────┘ │
│ ┌─ Advanced ───────────────────────────────────┐ │
│ │ ☑ Resync brightness every [ 60 ]↕ s          │ │
│ │ ☑ Resync after [ 30 ]↕ s of inactivity       │ │
│ │ ☐ Write log file   Level: [ info      ▾ ]    │ │
│ │   (logging changes take effect after restart)│ │
│ └──────────────────────────────────────────────┘ │
│ Open config file · Open log folder               │
│                       [Restore defaults] [Close] │
└──────────────────────────────────────────────────┘
```

### Control decisions

- **Numeric values use Edit + UpDown (spinner) buddies, no trackbars.**
  Trackbars are the most dark-mode-hostile common control (thumb needs
  custom draw); spinners are compact, precise, and dark-mode-manageable.
  Ranges are enforced by `UDM_SETRANGE32` plus clamp-on-focus-loss.
- **Opacity is displayed as integer percent (10–100)**, mapped to the
  config's `0.1–1.0` float in core code.
- **Hotkey capture is a custom subclassed control**, not `msctls_hotkey32`
  (which cannot represent several combinations and predates modern
  modifiers). Click → "Press a key combination… (Esc to cancel)" → next
  keydown is captured, mapped to the existing human-readable string format
  (`Ctrl+Shift+Up`) so `config.json` stays hand-editable. While capturing,
  the control answers `WM_GETDLGCODE` with `DLGC_WANTALLKEYS` — otherwise
  `IsDialogMessage` consumes Tab/arrows/Enter/Esc as dialog navigation
  before the control ever sees them, and arrows are in the default
  bindings, so this is the control's core function, not an edge case.
  Capture suspends global hotkey interception for its duration (see
  "Capture must suspend interception" above) — without that, the app's
  own registered combinations never reach the field.
  Capture rules: a binding requires at least one of Ctrl/Alt/Win (Shift
  alone is not enough — a bare captured letter would register a
  system-wide single-key hotkey and effectively steal that key from every
  app); only keys in the parser's `KEY_MAP` are accepted, and `VK_TO_NAME`
  is extended to cover everything `KEY_MAP` accepts (today it lacks A–Z
  and 0–9, so capturing `Ctrl+B` would serialize as `Ctrl+Unknown` and be
  reset to defaults at next startup) — with a round-trip property test
  over every entry. Keys outside the set are rejected with an inline
  message; the config *file* stays as permissive as today. Assigning the
  same combination to both actions is rejected before any registration
  attempt — and the check compares the *parsed canonical form*, not
  strings: `parse_hotkey` is case/order/alias-insensitive
  (`shift+ctrl+up` ≡ `Control+Shift+Up`), and a dialog opened on a
  hand-edited spelling must still detect the duplicate. The predicate is
  a helper in `hotkey.rs` with in-module tests.
  Keyboard path: capture also starts via Space/Enter on the focused
  field (not click-only); while capturing, modifier keydowns update a
  preview and only a non-modifier keydown completes the capture. Dialog
  keys: Esc closes the window (safe under instant apply), Enter
  activates the default button (Close) — the custom wndproc answers
  `DM_GETDEFID` so `IsDialogMessage` finds it.
- **"0 = disabled" becomes a checkbox.** The refresh interval fields never
  show a magic zero: checkbox unchecked = field disabled = `0` in the
  config. The config schema is unchanged. The dialog remembers the last
  non-zero value for the session, so uncheck-then-recheck restores it; if
  the dialog opened at 0, rechecking shows the default.
- **Log level** is a dropdown: error / warn / info / debug / trace — with
  a grayed explainer carrying the caveat that today lives only in the
  config source docs: at debug and below, monitor serial numbers and
  absolute paths are logged. The warning must not get lost on the way
  from source-comment readers to a novice-facing dropdown.
- The intercept checkbox carries a grayed explainer line (hardware caveat +
  antivirus note); the label says "Try to intercept" because the low-level
  hook genuinely cannot see brightness keys routed through vendor
  firmware, and the code falls back to plain registration on hook failure.
- **Opacity as integer percent** rounds an existing float on display
  (`0.335` → `34 %`); since changes only fire on commit, opening and
  closing the dialog never rewrites an untouched value.
- **"Restore defaults" confirms first** (a small message box): under
  instant apply it resets ten settings including both hotkeys in one
  click with no undo — novice-friendliness cuts both ways.
- Footer: "Open config file" and "Open log folder" as link-style controls
  on the left (the log link sits one group below the checkbox that says
  logging needs a restart — the user who enables it shouldn't have to
  hunt through the tray menu); "Restore defaults" and "Close" buttons on
  the right.

## Dark mode & DPI

Dark mode extends `theme.rs` (which already does the uxtheme opt-in for the
tray menu) in three layers — plus a prerequisite `theme.rs` cannot answer
today: **nothing in the repo can ask which mode the system is in.** The
existing code never needs the boolean (the theme engine decides for the
menu), but `DwmSetWindowAttribute` takes one and the `WM_CTLCOLOR*`
handlers must pick a brush. A new `theme::system_prefers_dark()` reads the
`AppsUseLightTheme` registry value — well-established convention, though
not formally documented by Microsoft, a status worth stating plainly in a
module that keeps careful score of documented-vs-ordinal-hack (no new
ordinal either way; the registry feature is already enabled). And the palette is gated on the *same*
condition as `theme.rs::api()`: where the uxtheme opt-in is unavailable
(pre-18362, or missing ordinals), the dialog paints **light, full stop** —
dark brushes under un-themed light controls would be dark-on-dark and
unreadable.

1. **Title bar:** `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)` —
   the one documented API. Version note: the attribute value is 20 from
   build 18985 on but 19 on 17763–18984, and `theme.rs` supports from
   18362 — so try 20, fall back to 19.
2. **Controls:** `SetWindowTheme(hwnd, "DarkMode_Explorer")` for
   buttons/checkboxes/scrollbars, `"DarkMode_CFD"` for the combo box, plus
   `WM_CTLCOLORSTATIC`/`WM_CTLCOLOREDIT`/`WM_CTLCOLORBTN` and
   `WM_CTLCOLORLISTBOX` (the combo's popped-open dropdown list) handlers
   returning dark brushes. The window's *own* background is painted via
   the class brush (swapped on re-theme) or `WM_ERASEBKGND` — not
   `WM_CTLCOLORDLG`, which only the dialog manager sends and a
   `CreateWindowExW` window with a custom wndproc never receives. The control palette avoids the worst
   offender (trackbars). **The first implementation step is a dark-mode
   spike over exactly this control set**, before any layout work — and its
   first question is the *group boxes*: in the reference implementations
   `theme.rs` itself cites, `BS_GROUPBOX` is the control that needs
   custom-drawing (frame and caption stay light), while push buttons are
   largely covered by `DarkMode_Explorer`. Pre-planned fallback if group
   boxes resist: bold label + separator line instead — which also
   simplifies tab-order/z-order (no frames to create behind controls).
   Mechanical prerequisite either way: the `Win32_UI_Controls` cargo
   feature (not currently enabled) and `InitCommonControlsEx` with
   `ICC_UPDOWN_CLASS` before creating the spinners.
3. **Live switching:** the window handles `WM_SETTINGCHANGE`
   ("ImmersiveColorSet") and re-themes immediately. This is a *different*
   mechanism from the tray menu, not the same one: the tray's window is
   message-only and therefore excluded from broadcasts (measured — see
   `architecture.md`, Menu Theming), which is why the menu refreshes on
   every open instead. The settings window is a genuine top-level window
   and does receive broadcasts — the same reason the power listener is
   deliberately top-level for `WM_DISPLAYCHANGE`. Given this codebase has
   been burned by broadcast-delivery assumptions before, the live-switch
   manual test is mandatory, not optional.

No Mica/Fluent imitation: the dialog is a clean classic window in a correct
light or dark palette.

**DPI:** the window is created at the DPI of the monitor it appears on,
reusing the 96-DPI-baseline scale-factor arithmetic from `osd.rs`
(`for_dpi`), fonts via point-size × dpi/72. Honest scoping: only the
formula is proven precedent — the OSD recomputes its metrics while hidden
and has no child controls, so live `WM_DPICHANGED` handling (resize,
re-font and reposition a dozen controls in place, no window recreation) is
new work, planned as one central relayout function that both initial
creation and DPI changes call.

## Validation, errors, persistence

- **Never fatal.** Numeric fields clamp to their valid range on focus
  loss; no error dialogs for numeric input. Deliberate divergence from the
  loader: `validate_and_fix` *substitutes the default* for out-of-range
  values (a repair policy for unattended startup), while the dialog clamps
  to the nearest bound (a guidance policy for a user mid-edit) — same
  never-fatal spirit, different mechanism, stated here so nobody "fixes"
  one to match the other. The ranges, for the record: step 1–50 %, OSD
  timeout 100–10 000 ms, opacity 10–100 % (0.1–1.0), periodic resync
  0–3600 s, inactivity resync 0–600 s.
- **When a numeric change fires:** a live `SettingChanged` is sent on
  spinner clicks and on commit (focus loss / Enter) — never on raw
  `EN_CHANGE` while typing, so retyping "5" as "30" cannot transiently
  apply "3". Clamping happens before any message is sent; the debounce is
  a write-coalescing measure, not a substitute for this.
- **"Restore defaults" resets exactly the enumerated fields, field by
  field** — never by swapping in `Config::default()`: hand-written
  `monitors` entries round-trip through saves by documented contract (the
  code comment in `core/config.rs`; `architecture.md` §4 documents the
  reservation itself) and must survive. Autostart is untouched (see
  "Autostart"). While the sink refreshes control values programmatically
  (defaults, reverts), change notifications are suppressed and any
  focused edit's uncommitted text is discarded first — refresh order must
  not commit a half-typed value.
- **Hotkey errors are inline**: registration failure → red message under the
  field, revert to the previous binding (see optimistic-with-revert flow
  above).
- **Persistence** goes through the `ConfigStore` seam wrapping
  `Config::save_to` unchanged (atomic `config.json.tmp` + rename) —
  debounced ~500 ms after the last change. Deliberately **no** `.bak`
  refresh on save: the backup is refreshed only on successful *load*
  (that is the existing contract — `.bak` = the last config that
  *parsed*), so a bad save can never be mirrored into the backup. Debounce timing is
  driven by the controller's existing injected-`now` tick, so it is
  host-testable. Known, accepted narrowing of the "a hard kill loses
  nothing" property (`architecture.md` §12): a live-applied change can be
  up to ~500 ms newer than the file; all graceful paths flush.
- **Saves are dirty-gated — the file is never rewritten for nothing.**
  Only dialog-originated changes mark the config dirty; dialog close and
  app quit flush *only if a debounced save is pending*. An app run that
  never touches the dialog never writes `config.json` — today the primary
  is written only when missing, unrecognized JSON keys are warned about
  and dropped on load, and an unconditional quit-save would therefore
  strip a hand-editor's unknown keys, ordering and formatting on every
  single run.
- **External edits: known-field values survive where the dialog didn't
  change them.** The dialog's own footer link invites hand-edits while
  the window is open, so "next save wins" is not good enough: the store
  remembers the file's identity (size + mtime — adequate, no hashing) at
  last read/write, and when a dialog-originated save finds the file
  changed on disk, it re-reads it, overlays only the fields changed
  through the dialog this session (the controller knows them from its
  `SettingChanged` history), and writes the merge. External edits to
  untouched *known* fields survive on disk and take effect at next start
  — exactly the deferred-reload semantics. Honest limits: any save still
  normalizes the file (unknown keys, ordering, formatting do not
  survive), and if the changed on-disk content does not *parse*, the
  save is deferred (config stays dirty, inline notice in the dialog,
  retried on the next change) — except at close/quit, where the merge
  base falls back to the last-good in-memory config rather than losing
  the user's dialog changes (an unparseable file would be
  default-substituted at next start anyway, and `.bak` recovery exists).
  Still no watcher, no live reload.

## Testing

Follows the project pattern — logic host-testable in `core/`, FFI verified
manually:

- Controller unit tests with fake `SettingsSink`/`HotkeyPort`/`ConfigStore`:
  every `SettingChanged` variant (config mutation + side effect + save
  scheduling), debounced save timing via injected `now`, hotkey rebind
  success / failure / revert / double-failure / missed ack (must raise
  the clearable hotkeys-degraded warning) *and the recovery transition*
  (a later successful rebind clears it), the capture-suspension
  reconciliation rules (respawn-while-suspended re-suspends;
  dialog-gone-while-suspended resumes), restore-defaults field-wise (fake
  store observes `monitors` survives), the merge-on-external-change save
  path including the unparseable-file deferral, opacity percent↔float
  mapping.
- In-module `hotkey.rs` tests (Windows-hosted like the rest of the
  suite, not `core/`): the canonical-form duplicate-binding predicate;
  the `VK_TO_NAME`↔`KEY_MAP` round-trip property over every accepted key.
- Manual checklist added to the "Integration Testing" section of
  `docs/architecture.md`: live light/dark switch with the dialog open,
  dragging across monitors with different DPI, hotkey capture incl. a
  conflicting global hotkey, rebinding repeatedly within a minute (the
  crash-loop gate must stay untouched), rebinding one action while the
  other keeps its combination, rebind → kill the hotkey thread → verify
  the *new* binding survives the respawn, open Settings on a monitor at
  0 % with the full black overlay (window must be visible above it) and
  then dim *further* (window must stay on top — the overlay re-asserts
  `HWND_TOPMOST` on every update), leave capture mode by every path
  (complete, Esc, click elsewhere, close the window) and verify hotkeys
  work again each time,
  dragging the settings title bar during a refresh (main loop must be
  unaffected — own thread), autostart registry entry appears/disappears,
  logging restart hint, both footer links.

## Module placement & docs

- New: `src/platform/windows/settings.rs` (settings thread, window,
  controls, capture subclassing, layout/DPI), wired per convention as
  `pub(crate) mod settings;` plus a `pub use` re-export in
  `platform/windows/mod.rs`; dark-mode helpers and
  `system_prefers_dark()` extend `src/platform/windows/theme.rs`.
- Extended: `core/state.rs` (`SettingChange`, `HotkeyRebindResult`,
  `SettingsClosed`, the direction-only adjust variant, the clearable
  hotkeys-degraded health warning),
  `core/controller.rs` (the three seams, handling, debounced save),
  `core/reconcile.rs` (`REBIND_TIMEOUT`), `core/config.rs`
  (restore-defaults helper), `platform/windows/hotkey.rs` (thread-id
  handshake, suspend/resume/rebind messages — `HotkeyManager` currently
  has no unregister API at all, `registered_ids` only ever grows, and
  `run_message_loop(&self)` needs `&mut self` for rebinding: real API
  surgery, not just a new message), `platform/windows/tray.rs` (usage
  rows move into the `TrayMenuOpening` response; stale
  frozen-at-startup comment corrected). Known mechanical cost: three new
  `Controller<…>` type parameters touch the `TestController` alias and
  every existing test constructor call — wide but shallow.
- Docs: new settings-window section + §4 note in `docs/architecture.md`,
  and the stale main-loop-cadence sentence there that still justifies the
  16 ms poll with the removed usage window gets corrected in the same
  pass; README feature list; CLAUDE.md module map.

## Future extensions (doors deliberately left open)

- **Config file reload**: a manual "Reload config file" action or a
  directory watcher can be added as additional producers feeding the same
  diff → `SettingChanged` pipeline. A watcher must suppress the app's own
  atomic writes and must *not* apply startup-style default-substitution on
  invalid content (log and keep current instead).
- **Per-monitor settings**: once the DDC-side clamping/disable logic
  exists, the dialog grows a monitor list + detail area; the reserved
  `monitors: {}` schema is already compatible.
