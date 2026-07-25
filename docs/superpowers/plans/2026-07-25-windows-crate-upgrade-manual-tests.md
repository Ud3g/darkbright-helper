# Manual hardware test pass — windows crate 0.52 → 0.62

Run on real hardware after the port branch is complete. Debug build
(`cargo run`) unless stated; one pass of everything before merge.

## Core (per docs/architecture.md "Integration Testing")
- [ ] DDC set/get: brightness up/down via hotkeys on EVERY connected monitor
      (hotkeys target the monitor under the cursor)
- [ ] OSD appears on the correct monitor, updates during adjustment,
      fades out
- [ ] Overlay: dim to 0% → black overlay engages; raise → overlay releases
- [ ] Tray: icon present; menu opens; live monitor status shown in menu
- [ ] Hotkeys: primary + secondary pairs; opt-in low-level hook for
      dedicated brightness keys (enable in config, test, disable again)
- [ ] Resume from sleep: sleep → wake → brightness cache refreshes
      (watch log for refresh)
- [ ] Monitor hot-plug: unplug/replug a monitor → topology updates
- [ ] Single instance: second launch shows the already-running notice and
      exits; first instance unaffected

## Paths the port demonstrably touches (beyond the standard list)
- [ ] Usage/instructions window: open it (tray menu), text renders, OK
      button has focus (Enter dismisses), window closes
- [ ] Ctrl+C in the debug console: clean shutdown (handler deregistered,
      no hang, exit code 0)
- [ ] Tray degraded-status PUSH path: unplug DDC (or otherwise force a
      DDC-degraded state) WITHOUT opening the menu → tray icon switches to
      the warning state on its own
- [ ] Supervised restarts: kill the DDC worker (e.g. induce a panic or use
      a debugger) → worker respawns, sets work again; same for the hotkey
      thread if force-killable
- [ ] Tray → "Open config file" and "Open data directory": both open; then
      one forced failure (temporarily rename config.json) → error handled,
      logged, app stays alive (ShellExecuteW result casts changed
      representation in 0.62)

## Targeted behavior checks (no compiler coverage)
- [ ] Single-instance detection still keys off ERROR_ALREADY_EXISTS
      (second launch, watch debug log for the last-error read)
- [ ] OSD error state (make a DDC set fail, e.g. unplugged monitor):
      error code shown/logged is plausible (codes may differ in detail
      from 0.52 — crate-captured vs deferred read — but must be sane)
- [ ] Debug logs: handle fields now print as pointers (cosmetic; confirm
      no log line panics or prints garbage)
