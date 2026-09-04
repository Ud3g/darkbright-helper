# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`darkbright-helper` is a hotkey-driven monitor brightness tool for Windows (Rust 2024, MSRV 1.88+). It controls hardware brightness via DDC/CI (VCP code `0x10`) for 1–100%, and falls back to a black layered overlay window for "sub-zero" dimming at 0%. Multi-monitor aware: hotkeys target the monitor under the mouse cursor. Monitors are identified by EDID (manufacturer + model + serial), not OS handles, so config is portable.

## Platform gotcha (important)

This is a **Windows-only binary**. `src/main.rs` and everything under `src/platform/windows/` import the `windows` crate unconditionally or behind `#[cfg(windows)]`. On a Windows host, plain `cargo build`/`cargo test` covers the whole crate. **On a Linux/WSL host they will not compile the full app** — target Windows instead:

```bash
cargo build --target x86_64-pc-windows-msvc      # or build on a real Windows host
```

The platform-agnostic `src/core/` and `src/error.rs` do compile on any host, so logic in those modules can be unit-tested cross-platform. When changing core logic, prefer putting testable code in `core/` and keep `unsafe`/FFI in `platform/windows/`.

## Commands

```bash
cargo build                  # debug: console window VISIBLE, log output shown
cargo build --release        # release: console HIDDEN (windows_subsystem="windows")
cargo fmt --all -- --check   # must pass before commit
cargo clippy --all-targets --locked -- -D warnings  # matches the CI gate exactly; `all` + `pedantic` are warn-by-default in Cargo.toml
cargo test --locked          # unit + integration + doc tests
cargo test read_is_scaled_by_the_monitors_reported_maximum   # single test by name (or a module: cargo test brightness)
cargo check --release --locked     # only configuration in which the hidden-console cfg compiles
RUST_LOG=debug cargo run     # run with debug logging (env_logger)
```

Tests live in-module (`#[cfg(test)]`) under `src/` plus integration tests in `tests/` (`ddc_test.rs`, `hotkey_test.rs`, `single_instance_test.rs`); the crate-root example in `src/lib.rs` is a real doctest. Hardware-dependent DDC/refresh behavior is verified **manually** — see the "Integration Testing" section of `docs/architecture.md`.

Releases are cut by pushing a bare-semver tag (`0.8.0`, no `v` prefix), which triggers the release workflow — see `RELEASING.md`.

## Architecture

> This section is a derived summary. If it disagrees with `docs/architecture.md` or the
> code, those win — and the mismatch is a bug in this file: fix it here.

Single-owner state with message passing — no async runtime. The **main thread** owns all `MonitorState` and the UI (OSD, overlay), and is the only thread that mutates state. Worker threads communicate via MPSC channels:

- **Main thread**: processes `BrightnessMessage`s, owns the `MonitorId → MonitorState` map, updates OSD/overlay immediately (optimistic), dispatches `DdcCommand` to the DDC worker, drives a `PeekMessageW` loop. All of that orchestration lives in `Controller<Osd, Ovl, Ddc, Loc, Set, Hk, Store>` (`core/controller.rs`, host-testable — the last three type parameters are the settings-window seams, `SettingsSink`/`HotkeyPort`/`ConfigStore`); `main.rs` keeps only thread wiring and shell side effects — new orchestration logic belongs in the controller, not in `main.rs`.
- **Hotkey thread** (`platform/windows/hotkey.rs`): `RegisterHotKey` + `WM_HOTKEY` loop. Dedicated brightness keys are bound via plain `RegisterHotKey` by default; the optional `WH_KEYBOARD_LL` hook (`intercept_brightness_keys`, default off) intercepts them ahead of the Shell instead, falling back to plain registration if the hook fails.
- **DDC worker thread** (`platform/windows/ddc_worker.rs`): owns `HashMap<MonitorId, DdcMonitor>` (commands address monitors by identity, never by OS handle), performs blocking DDC I/O (~40–120ms, up to 3 attempts = initial try + 2 retries @ 40ms), sends results back.
- **Power thread** (`platform/windows/power.rs`): listens for resume events, triggers refresh.
- **Tray thread** (`platform/windows/tray.rs`): tray icon + context menu; uses a request/response message (`TrayMenuOpening { reply_tx }`) to fetch live monitor status *and* the current hotkey bindings when the menu opens, and again on a timer for as long as it stays open (monitor rows only) — the usage rows are no longer frozen at spawn, since the settings window can rebind hotkeys while the app runs.

Message enums (`BrightnessMessage` to main, `DdcCommand` to worker) are defined in `src/core/state.rs`. Brightness is updated **optimistically**: the OSD/overlay update instantly, then the DDC result is reconciled by sequence id via `MonitorState::apply_set_result`, yielding `SetOutcome::{Confirmed, Reverted, GroundTruth, Ignored}` (a late success with no pending is applied as ground truth); the controller's failure paths call `force_revert()`. Hardware failure does NOT fall back to overlay dimming for values > 0% (strict error handling) — the OSD shows a red error state instead.

`MonitorState` holds `cached_brightness` (last confirmed), `brightness_known` (false while that value is only a seed), `pending: Option<PendingSet>` (optimistic, awaiting DDC), `overlay_opacity`, and `missing_since` (absence bookkeeping for pruning). Cache is refreshed on startup, periodically, after inactivity, and on system resume; a generation-countered `RefreshTracker` (`core/reconcile.rs`) gates concurrent refreshes and discards stale results.

**Supervision & degraded operation** (see `docs/architecture.md` §12): startup acquires the single-instance guard *before* any thread, window, or hotkey exists (second launch → message box + exit; fail-open by design). The DDC worker runs under `DdcSupervisor` (respawn with backoff), the hotkey thread is respawned via `RespawnGate`; the respawn limits and the wall-clock watchdog timeouts (`SET_TIMEOUT` 8 s, `REFRESH_TIMEOUT` 5 s) live in `core/reconcile.rs`. Monitors absent longer than `PRUNE_ABSENCE_WINDOW` (90 s) are pruned — that is what `missing_since` feeds. Degraded subsystems surface as `HealthWarnings` on the tray icon/tooltip.

### Module map

- `src/core/` — platform-agnostic: `brightness.rs` (adjustment math), `config.rs` (JSON config + validation, `restore_defaults`, `overlay_dirty` for merge-on-save), `controller.rs` (message-driven orchestration behind the platform seams), `edid.rs` (EDID → `MonitorId` parsing), `logfile.rs` (rolling file sink), `panic_hook.rs` (panic logging), `reconcile.rs` (refresh/respawn/save-debounce/rebind-ack timing constants), `state.rs` (state, messages, `MonitorId`, display-name generation).
- `src/platform/mod.rs` — gates the platform submodule; the portability seams (`OsdSink`, `OverlaySink`, `DdcPort`, `MonitorLocator`, `SettingsSink`, `HotkeyPort`, `ConfigStore`) live in `core/controller.rs`.
- `src/platform/windows/` — all Win32 FFI: `ddc.rs`, `ddc_worker.rs`, `hotkey.rs`, `osd.rs`, `osd_render.rs` (private), `overlay.rs`, `power.rs`, `single_instance.rs`, `theme.rs` (dark-mode opt-in for the tray menu and, via `system_prefers_dark()`, the settings window), `tray.rs`, `autostart.rs` (`HKCU\Run`-backed "Start with Windows"), `config_store.rs` (`WindowsConfigStore`, the `ConfigStore` seam — save-with-merge onto externally-edited config); `settings/` (directory module: `mod.rs`, `layout.rs` declarative control table, `window.rs` window/wiring/`SettingsSinkImpl`, `capture.rs` hotkey-capture control, `dark.rs` dark-mode painting) — the settings dialog, on its own thread; plus `mod.rs`, which is not a bare gate — it holds `CursorLocator` (the `MonitorLocator` impl), the message-box helpers, and the public re-exports the binary imports.
- `src/error.rs` — `BrightnessError` enum + `pub type Result<T>`.
- `src/lib.rs` — the crate is a lib + bin pair; `main.rs` consumes everything through `darkbright_helper::…`. Visibility rule: `pub` means the binary or `tests/` names it (see `docs/code-conventions.md`).

Read `docs/architecture.md` for full design rationale (it is the source of truth for behavior) and `docs/code-conventions.md` for FFI/style rules. There is no roadmap doc in the repo: the README's "Scope" section states the non-goals, and deliberate technical positions live under "Maintenance Decisions" in `docs/architecture.md`.

## Project conventions (project-specific; standard Rust is assumed)

- **Config**: JSON at `%APPDATA%\BrightnessControl\config.json`. Invalid field values are logged as errors and replaced with defaults (never fatal). Durability contract: writes are atomic (`config.json.tmp` + rename), a `config.json.bak` mirror is refreshed after every successful parse, and a corrupt primary is recovered from the backup (warning) before defaults are substituted (error) — do not break this when touching the save/load paths. Valid ranges/defaults are tabulated in `docs/architecture.md` §4. The `monitors: {}` field is reserved for future per-monitor settings. The settings window (§14) writes through the same atomic path but merges onto a concurrently hand-edited file instead of refreshing `.bak` — a different contract, documented where it lives.
- **FFI safety**: keep `unsafe` isolated behind safe wrappers, as close to point of use as possible. Wrap Windows handles in RAII (`Drop`) types. A `// SAFETY:` comment goes on every `unsafe` block whose correctness rests on something the block does not show — pointer provenance, a separately passed buffer length, a handle's validity or thread affinity, a `transmute`, a callback contract, an `unsafe impl` — and *only* there; a plain Win32 call on scalars needs none, and a cast or lint suppression gets an ordinary comment (see `docs/code-conventions.md` §3).
- **`windows` crate v0.62**: prefer the `Result`-returning bindings with `?` (+ `.map_err` to `BrightnessError`) over `BOOL` checks; handles are pointers — use `is_invalid()`, never `.0 == 0`, and note handles are not `Send`/`Sync` (core seam carries them as `isize`); optional handle params take `None`; `BOOL` lives in `windows::core`; use slices not ptr+len; many FFI structs lack `Debug` (impl manually, copy packed fields to locals first); use `&raw const`/`&raw mut` not `&x as *mut _`.
- **Casts**: avoid `as`; use `u32::from`, `try_from`, `.cast_unsigned()`/`.cast_signed()`. Annotate pure fns with `#[must_use]` (except those returning `Result`).
- **Docs**: all public items need `///` with `# Errors`/`# Panics` where applicable (clippy enforces this). Backtick code identifiers (`clippy::doc_markdown`).
- **Logging**: structured key-value form preferred — `log::info!(monitor_id:% = id, brightness = v; "Brightness adjusted")`. Log at the point of *handling*, not occurrence (functions returning `Result` return `Err`, don't log it). Never log PII; serials only at `debug`.

## Code comments

Do not cite ephemeral planning labels (phase/step/wave/feature IDs like `F.6`, `WB0.4`) in code comments — state the rationale in self-contained domain terms. (The existing docs use such labels; that's fine for docs, but don't propagate them into code comments.)
