# Changelog

Notable changes to darkbright-helper. Maintained from 0.8.0 onward; earlier
history lives in the git log.

## [Unreleased]

### Added

- Panics are now logged (payload, source location, thread) and the log sinks
  flushed before the process dies, so a crash in a release build — where the
  console is hidden — leaves a trace in the opt-in file log.
- Releases now ship a prebuilt `darkbright-helper.exe` on GitHub Releases,
  built by a tag-triggered workflow; `RELEASING.md` documents the procedure.
- Unknown config keys (typos, misplaced settings) are now warned about at
  load with their full path instead of being silently ignored.

### Changed

- Upgraded the `windows` crate from 0.52 to 0.62. No intended behavior change;
  debug-log output now shows window/monitor handles as pointers instead of
  integers.
- MSRV corrected to 1.88: the code already used let chains (stable since
  1.88), so 1.87 never actually built — caught by the new MSRV CI job on its
  first run.
- Quieter, more useful logs: the periodic-refresh heartbeat (two lines per
  minute) moved from info to debug so the size-capped file log covers a longer
  window; newly detected monitors and the registered hotkey combos are now
  logged at info instead. DDC set failures are logged once (at the handling
  site) rather than twice, and DDC retry warnings now name the affected
  monitor.

### Fixed

- With `intercept_brightness_keys` enabled, a failed low-level hook
  installation left the dedicated brightness keys silently dead; they now fall
  back to plain hotkey registration.
- System-resume detection never fired: `WM_POWERBROADCAST` is a sent message
  (invisible to the `GetMessageW` queue check) and message-only windows are
  excluded from broadcasts, so post-sleep resync silently fell back to the
  periodic refresh. The power window now subscribes via
  `RegisterSuspendResumeNotification` and handles the message in its window
  procedure.
- A blocked log rotation (e.g. `darkbright.log.old` transiently locked by a
  scanner) silently dropped every file-log record until rotation succeeded
  again. The file log now degrades to a temporarily oversized file as
  documented — records keep flowing, and the next over-cap write retries the
  rotation.
- An OSD failure during a brightness adjustment aborted the adjustment after
  the optimistic value was recorded but before the DDC command went out; the
  watchdog then miscounted the orphaned pending as a DDC set timeout —
  repeated OSD failures could falsely latch the degraded-DDC state on healthy
  hardware. OSD failures are now logged and skipped: the adjustment always
  reaches the hardware.

## [0.8.0] — 2026-07-24

Rollup of everything landed since 0.7.1.

### Added

- Usage window with hotkey instructions (tray → Usage).
- Single-instance guard: a second launch shows an info box and exits.
- DDC worker supervision: sequence-correlated set results, refresh
  generations, respawn with crash-loop backoff, a hang watchdog, and a
  degraded-DDC state that recovers on user activity or system resume.
- Hotkey thread supervision with restart and crash-loop backoff.
- Monitor hot-unplug pruning (absence evidence over ~90 s) — no more ghost
  rows in the tray menu after undocking.
- Config resilience: atomic writes, a `.bak` mirror with automatic recovery
  from a corrupt `config.json`, and invalid hotkey strings repairing to
  defaults instead of aborting startup.
- Tray visibility: degraded-state warnings (menu lines, tooltip, amber-badged
  icon) for lost DDC or hotkeys, plus a version line in the menu; the version
  is also logged at startup.
- Opt-in rolling file log (`logging.file_enabled`, level via
  `logging.file_level`) under `%APPDATA%\BrightnessControl\`, size-capped at
  2 × 1 MB, plus a tray "Open Log Folder" entry — release builds hide the
  console, so this is the retrievable diagnostic artifact. Structured
  key-value log fields are now rendered on console and file (`unstable-kv`).
- Load-time warnings for a config `version` mismatch (no migration yet;
  fields read as current schema) and for a non-empty `monitors` section
  (reserved, not yet implemented — entries are preserved).
- CI workflow (fmt, clippy, tests, cargo audit); MSRV pinned to 1.87.

### Changed

- Brightness orchestration extracted into a platform-agnostic, unit-tested
  core `Controller` behind OSD/overlay/DDC/locator seams.
- OSD GDI drawing moved into a render layer with RAII resource wrappers.
- Monitor serial numbers appear only in `debug`-level logs; absolute config
  paths are logged at debug only.
- A monitor whose brightness read fails during a refresh stays set-capable
  (its DDC handle is kept), instead of failing with "monitor not found"
  until the next refresh.
- An externally raised hardware brightness (physical buttons, monitor
  self-reset) now clears an active sub-zero dimming overlay on the next
  refresh, instead of leaving a black veil over a bright backlight.

### Removed

- `dirs-next`/`dirs-sys-next`/`winapi` dependency chain and unused
  `parking_lot`.
