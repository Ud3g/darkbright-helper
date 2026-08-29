# Changelog

Notable changes to darkbright-helper. Maintained from 0.8.0 onward; earlier
history lives in the git log.

## [Unreleased]

### Added

- The tray context menu now follows the Windows light/dark setting instead of
  always being drawn light, and picks up a changed setting the next time it is
  opened. Requires Windows 10 1903 or newer; on anything older, or if the
  theming entry points are unavailable, the menu stays light exactly as before.

### Changed

- The executable now carries an application manifest declaring version 6 of the
  common controls, so system-drawn controls use the current Windows renderer
  instead of the pre-XP one. Visible only in the app's two message boxes (the
  single-instance notice and the hotkey-registration error), whose buttons are
  drawn in the modern style; they stay light, as message boxes have no dark
  rendering to follow.

### Fixed

- The `THIRD-PARTY-NOTICES.html` shipped in release archives attributed the
  twelve `windows-*` crates to `Copyright (c) <year> <copyright holders>` — the
  unfilled SPDX placeholder — instead of Microsoft. Those crates ship their
  licence as `license-mit`, a name cargo-about's automatic discovery misses, so
  it fell back to the canonical template text. `about.toml` now points it at the
  real file. Affects 0.9.0's archive, which cannot be corrected in place.

## [0.9.0] — 2026-08-16

First public release: the repository was open-sourced under `MIT OR Apache-2.0`
on this date, and this is the first version built by the packaging workflow
described below.

### Added

- Panics are now logged (payload, source location, thread) and the log sinks
  flushed before the process dies, so a crash in a release build — where the
  console is hidden — leaves a trace in the opt-in file log.
- Releases now ship a `darkbright-helper-<version>-windows-x64.zip` on GitHub
  Releases, built by a tag-triggered workflow: the executable together with both
  licence files and a generated `THIRD-PARTY-NOTICES.html` covering every crate
  compiled into it. Build provenance is attested, so `gh attestation verify`
  ties the binary to the commit and workflow run that produced it, and the
  release notes carry the zip's SHA-256. `RELEASING.md` documents the procedure.
- Unknown config keys (typos, misplaced settings) are now warned about at
  load with their full path instead of being silently ignored.
- A display change (monitor plugged, unplugged, or resolution switched) now
  triggers an immediate refresh. Previously the new topology was picked up
  only by the periodic refresh — up to a minute later, or never if that
  refresh was disabled — during which an adjustment could land on the wrong
  monitor, because Windows may reuse a monitor handle for a different display.

### Changed

- Narrowed the library's public surface to what the binary and integration tests
  actually name (116 items, down from 259). The lib exists only so `core/` is
  host-testable and three integration tests can reach the platform layer, but
  everything `pub` in a `pub mod` counts as externally reachable — so rustc had
  been reporting no unused items anywhere in the crate. Restoring that check
  found ~15 dead items, now deleted: a retired OSD error-state method, a
  redundant tray sender and its accessors, `DdcMonitor`'s unread cache, an
  unused hotkey unregistration path, `SafeHwnd`'s borrowed variant (leaving it
  owning-by-construction), a superseded `Config::load`, and an unconstructed
  error variant. No behaviour change; net 158 lines removed.
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

- A hotkey thread that hung during startup — inside window creation or hotkey
  registration — froze the main loop indefinitely, because the wait for its
  result was untimed. The wait is now bounded to 5 s; on timeout the thread is
  abandoned and the start reported as failed, which shows the error box at
  startup and goes through the respawn gate on a supervised restart.
- Enabling `logging.file_enabled` and getting no log file gave no clue why. If
  the sink could not be built (unwritable `%APPDATA%`, a locked file, a full
  disk), the warning went to the log — which is exactly what had failed — and
  to a console that release builds hide, so the feature just appeared not to
  work. The tray menu now says so, and points at the log folder, which is where
  "Open Log Folder" already leads. It deliberately does not badge the tray
  icon: brightness control is unaffected.
- The tray told users to "press a brightness hotkey to retry" for a DDC worker
  that had stopped responding — advice that cannot work, since no keypress
  unsticks a thread blocked inside a DDC call. Worse, the press cleared the
  warning it could not fix, so the badge flickered off and returned ~24 s later,
  indefinitely. The two degraded states are now distinguished: a dead worker
  keeps the (correct) hotkey advice, an unresponsive one says so and suggests a
  restart only if it persists. It usually will not: any result the worker sends
  is proof it is no longer blocked, so the state now clears itself the moment
  the hardware answers — as does a stuck worker that later exits, which is now
  respawned like any other dead one.
- A monitor that identifies itself but refuses the brightness read got no state
  at all, so every hotkey press on it failed with "monitor not found" before
  reaching the OSD — nothing moved and nothing was shown. Such monitors are now
  seeded at 50% and stay adjustable, which is what the documented "unreadable
  monitors stay set-capable" behaviour always intended; the seed is corrected by
  the first successful read or write, and the tray marks it `~` until then.
- Brightness was silently wrong on any monitor whose DDC luminance range is not
  0-100. The maximum the monitor reports was read and discarded, so raw values
  were treated as percentages: a monitor reporting 500 of 1000 (50%) displayed
  as 100%, and setting "100%" on a 0-255 monitor asked for ≈39% backlight. Both
  directions are now scaled by the reported maximum, which is also logged with
  each refresh read at `debug`. Monitors reporting a maximum of 100 — the common
  case, including the maintainer's — are unaffected.
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
