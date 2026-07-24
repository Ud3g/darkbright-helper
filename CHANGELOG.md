# Changelog

Notable changes to darkbright-helper. Maintained from 0.8.0 onward; earlier
history lives in the git log.

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
- CI workflow (fmt, clippy, tests, cargo audit); MSRV pinned to 1.87.

### Changed

- Brightness orchestration extracted into a platform-agnostic, unit-tested
  core `Controller` behind OSD/overlay/DDC/locator seams.
- OSD GDI drawing moved into a render layer with RAII resource wrappers.
- Monitor serial numbers appear only in `debug`-level logs; absolute config
  paths are logged at debug only.

### Removed

- `dirs-next`/`dirs-sys-next`/`winapi` dependency chain and unused
  `parking_lot`.
