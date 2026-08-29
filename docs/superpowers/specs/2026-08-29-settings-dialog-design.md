# Settings Dialog — Design

Date: 2026-08-29
Status: design approved in brainstorming; implementation plan pending.

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
| Apply semantics | Mixed, maximally live: everything applies immediately — including hotkey rebinds via thread respawn — except logging options, which get a "takes effect after restart" hint. |
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

The settings window is a modeless top-level window owned by the **main
thread** — the thread that already owns the OSD and overlay. It is created
lazily on first open; a second "Settings" activation brings the existing
window to the foreground instead of duplicating it. Closing destroys the
window (recreation is cheap). While the window exists, the main
`PeekMessageW` loop routes messages through `IsDialogMessage`, providing
Tab/Shift+Tab keyboard navigation.

### Message flow (single-owner model preserved)

- Tray "Settings" sends `BrightnessMessage::TrayOpenSettings` as today, but
  the message is **no longer intercepted in `main.rs`**: the controller
  handles it and calls the new `SettingsSink` seam to open/focus the window,
  passing a snapshot of the current config values. (Orchestration moves into
  the controller, per the project rule; opening the JSON file becomes a
  dialog-footer action instead.)
- Control changes in the dialog post typed
  `BrightnessMessage::SettingChanged(SettingChange)` messages into the
  **existing MPSC channel** (same path the tray uses). One enum variant per
  logical setting; values arrive pre-parsed (e.g. opacity as the UI's
  integer percent, converted in core). Exception: the autostart toggle
  never enters the channel — it is a pure registry operation handled in
  the platform layer (see "Autostart").
- The controller remains the sole owner of the runtime `Config`: it mutates
  its field, applies side effects, and schedules a debounced save.

### Side-effect routing per setting

| Setting | Live effect | Where |
|---|---|---|
| `step_percent` | none needed (controller reads its config on next adjust) | controller |
| `osd.timeout_ms`, `osd.opacity` | `OsdSink` gains an appearance-update method | controller → seam |
| `refresh.*` | controller timers already read the config field each tick | controller |
| hotkeys (both bindings + intercept flag) | hotkey thread respawn with new bindings via the existing `RespawnGate` machinery | `main.rs` (platform), result reported back |
| autostart | registry write | dialog/platform layer directly (not config) |
| `logging.*` | none — save only; `SettingsSink` shows the restart hint | controller → seam |

**Hotkey rebind is optimistic with revert**, mirroring the brightness
pipeline's `apply_set_result`/`force_revert` philosophy: the controller
applies the new binding to its config immediately; `main.rs` performs the
respawn and posts a `BrightnessMessage::HotkeyRebindResult { success, .. }`;
on failure the controller reverts the config field and notifies the
`SettingsSink`, which shows an inline error and restores the previous
binding in the capture field.

### `SettingsSink` seam

New host-fakeable trait alongside `OsdSink`/`OverlaySink` in
`core/controller.rs`. Responsibilities: open/focus the window with a config
snapshot, refresh displayed values (restore defaults, reverts), show inline
errors (hotkey registration failure), show the restart hint for logging
changes. The Win32 implementation lives in
`src/platform/windows/settings.rs`; controller tests use a fake.

### Autostart

Single source of truth is the registry
(`HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, value
`darkbright-helper` = quoted exe path). The checkbox reads the actual
registry state when the dialog opens and writes it directly on toggle.
Nothing is mirrored into `config.json` — no second store, no drift.
"Restore defaults" leaves autostart untouched (system integration, not a
preference; silently removing autostart would surprise).

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
│ Open config file      [Restore defaults] [Close] │
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
  (`Ctrl+Shift+Up`) so `config.json` stays hand-editable. Assigning the
  same combination to both actions is rejected in the dialog before any
  registration attempt.
- **"0 = disabled" becomes a checkbox.** The refresh interval fields never
  show a magic zero: checkbox unchecked = field disabled = `0` in the
  config. The config schema is unchanged.
- **Log level** is a dropdown: error / warn / info / debug / trace.
- The intercept checkbox carries a grayed explainer line (hardware caveat +
  antivirus note); the label says "Try to intercept" because the low-level
  hook genuinely cannot see brightness keys routed through vendor
  firmware, and the code falls back to plain registration on hook failure.
- Footer: "Open config file" as a link-style control on the left;
  "Restore defaults" and "Close" buttons on the right.

## Dark mode & DPI

Dark mode extends `theme.rs` (which already does the uxtheme opt-in for the
tray menu) in three layers:

1. **Title bar:** `DwmSetWindowAttribute(DWMWA_USE_IMMERSIVE_DARK_MODE)` —
   the one documented API.
2. **Controls:** `SetWindowTheme(hwnd, "DarkMode_Explorer")` for
   buttons/checkboxes/scrollbars, `"DarkMode_CFD"` for the combo box, plus
   `WM_CTLCOLORDLG`/`WM_CTLCOLORSTATIC`/`WM_CTLCOLOREDIT` handlers returning
   dark brushes. The control palette chosen above (checkbox, edit, updown,
   combobox, button) deliberately avoids the controls that resist this
   technique.
3. **Live switching:** the window handles `WM_SETTINGCHANGE`
   ("ImmersiveColorSet") and re-themes immediately — same responsiveness as
   the tray menu.

No Mica/Fluent imitation: the dialog is a clean classic window in a correct
light or dark palette.

**DPI:** the window is created at the DPI of the monitor it appears on,
using the 96-DPI-baseline scale-factor pattern from `osd.rs` (`for_dpi`),
fonts via point-size × dpi/72. `WM_DPICHANGED` triggers a relayout with the
new factor (no window recreation).

## Validation, errors, persistence

- **Never fatal, mirroring the config contract.** Numeric fields clamp to
  their valid range on focus loss; no error dialogs for numeric input.
- **Hotkey errors are inline**: registration failure → red message under the
  field, revert to the previous binding (see optimistic-with-revert flow
  above).
- **Persistence** reuses `Config::save_to` unchanged (atomic
  `config.json.tmp` + rename, `.bak` refresh) — debounced ~500 ms after the
  last change, plus an unconditional flush when the dialog closes and on
  app quit. Debounce timing is driven by the controller's existing
  injected-`now` tick, so it is host-testable.
- **External edits while running**: not reloaded; the next dialog-triggered
  save wins. Documented in `architecture.md`.

## Testing

Follows the project pattern — logic host-testable in `core/`, FFI verified
manually:

- Controller unit tests with a fake `SettingsSink`: every `SettingChanged`
  variant (config mutation + side effect + save scheduling), debounced save
  timing via injected `now`, hotkey rebind success/failure/revert,
  restore-defaults, opacity percent↔float mapping, duplicate-binding
  rejection.
- Manual checklist added to the "Integration Testing" section of
  `docs/architecture.md`: live light/dark switch with the dialog open,
  dragging across monitors with different DPI, hotkey capture incl. a
  conflicting global hotkey, autostart registry entry appears/disappears,
  logging restart hint, escape-hatch link.

## Module placement & docs

- New: `src/platform/windows/settings.rs` (window, controls, capture
  subclassing, layout/DPI); dark-mode helpers extend
  `src/platform/windows/theme.rs`.
- Extended: `core/state.rs` (`SettingChange`, `HotkeyRebindResult`),
  `core/controller.rs` (`SettingsSink`, handling, debounced save),
  `core/config.rs` (restore-defaults helper).
- Docs: new settings-window section + §4 note in `docs/architecture.md`,
  README feature list, CLAUDE.md module map.

## Future extensions (doors deliberately left open)

- **Config file reload**: a manual "Reload config file" action or a
  directory watcher can be added as additional producers feeding the same
  diff → `SettingChanged` pipeline. A watcher must suppress the app's own
  atomic writes and must *not* apply startup-style default-substitution on
  invalid content (log and keep current instead).
- **Per-monitor settings**: once the DDC-side clamping/disable logic
  exists, the dialog grows a monitor list + detail area; the reserved
  `monitors: {}` schema is already compatible.
