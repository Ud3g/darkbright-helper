# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`darkbright-helper` is a hotkey-driven monitor brightness tool for Windows (Rust 2024, MSRV 1.87+). It controls hardware brightness via DDC/CI (VCP code `0x10`) for 1–100%, and falls back to a black layered overlay window for "sub-zero" dimming at 0%. Multi-monitor aware: hotkeys target the monitor under the mouse cursor. Monitors are identified by EDID (manufacturer + model + serial), not OS handles, so config is portable.

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
cargo fmt -- --check         # must pass before commit
cargo clippy -- -D warnings  # must pass; clippy `all` + `pedantic` are warn-by-default in Cargo.toml
cargo test                   # unit + integration tests
cargo test <name>            # run a single test by name (e.g. cargo test calculate_adjustment)
RUST_LOG=debug cargo run     # run with debug logging (env_logger)
```

Tests live in-module (`#[cfg(test)]`) under `src/` plus integration tests in `tests/` (`ddc_test.rs`, `hotkey_test.rs`). Hardware-dependent DDC/refresh behavior is verified **manually** — see the "Integration Testing" section of `docs/architecture.md`.

Releases are cut by pushing a bare-semver tag (`0.8.0`, no `v` prefix), which triggers the release workflow — see `RELEASING.md`.

## Architecture

Single-owner state with message passing — no async runtime. The **main thread** owns all `MonitorState` and the UI (OSD, overlay), and is the only thread that mutates state. Worker threads communicate via MPSC channels:

- **Main thread**: processes `BrightnessMessage`s, owns the `MonitorId → MonitorState` map, updates OSD/overlay immediately (optimistic), dispatches `DdcCommand` to the DDC worker, drives a `PeekMessageW` loop.
- **Hotkey thread** (`platform/windows/hotkey.rs`): `RegisterHotKey` + `WM_HOTKEY` loop; optional `WH_KEYBOARD_LL` hook for dedicated brightness keys (opt-in, default off).
- **DDC worker thread** (`platform/windows/ddc_worker.rs`): owns `Vec<DdcMonitor>`, performs blocking DDC I/O (~40–120ms, up to 3 attempts = initial try + 2 retries @ 40ms), sends results back.
- **Power thread** (`platform/windows/power.rs`): listens for resume events, triggers refresh.
- **Tray thread** (`platform/windows/tray.rs`): tray icon + context menu; uses a request/response message (`TrayMenuOpening { reply_tx }`) to fetch live monitor status when the menu opens.

Message enums (`BrightnessMessage` to main, `DdcCommand` to worker) are defined in `src/core/state.rs`. Brightness is updated **optimistically**: the OSD/overlay update instantly, then the DDC result either confirms (`confirm_brightness`) or reverts (`revert_pending`) the pending value. Hardware failure does NOT fall back to overlay dimming for values > 0% (strict error handling) — the OSD shows a red error state instead.

`MonitorState` holds `cached_brightness` (last confirmed), `pending_brightness` (optimistic, awaiting DDC), and `overlay_opacity`. Cache is refreshed on startup, periodically, after inactivity, and on system resume; a generation-countered `RefreshTracker` (`core/reconcile.rs`) gates concurrent refreshes and discards stale results.

### Module map

- `src/core/` — platform-agnostic: `brightness.rs` (adjustment math), `config.rs` (JSON config + validation), `controller.rs` (message-driven orchestration behind the platform seams), `edid.rs` (EDID → `MonitorId` parsing), `logfile.rs` (rolling file sink), `panic_hook.rs` (panic logging), `reconcile.rs` (refresh/respawn tracking), `state.rs` (state, messages, `MonitorId`, display-name generation).
- `src/platform/mod.rs` — gates the platform submodule; the portability seams (`OsdSink`, `OverlaySink`, `DdcPort`, `MonitorLocator`) live in `core/controller.rs`.
- `src/platform/windows/` — all Win32 FFI: `ddc.rs`, `ddc_worker.rs`, `hotkey.rs`, `osd.rs`, `osd_render.rs`, `overlay.rs`, `power.rs`, `single_instance.rs`, `tray.rs`, `usage.rs`.
- `src/error.rs` — `BrightnessError` enum + `pub type Result<T>`.

Read `docs/architecture.md` for full design rationale (it is the source of truth for behavior), `docs/code-conventions.md` for FFI/style rules, and `docs/improvement-ideas.md` for the roadmap.

## Project conventions (project-specific; standard Rust is assumed)

- **Config**: JSON at `%APPDATA%\BrightnessControl\config.json`. Invalid field values are logged as errors and replaced with defaults (never fatal). Valid ranges/defaults are tabulated in `docs/architecture.md` §4. The `monitors: {}` field is reserved for future per-monitor settings.
- **FFI safety**: keep `unsafe` isolated behind safe wrappers, as close to point of use as possible. Wrap Windows handles in RAII (`Drop`) types. In Rust 2024, `unsafe fn` bodies still need explicit `unsafe { }` blocks.
- **`windows` crate v0.52**: prefer the `Result`-returning bindings with `?` (+ `.map_err` to `BrightnessError`) over `BOOL` checks; use slices not ptr+len; many FFI structs lack `Debug` (impl manually, copy packed fields to locals first); use `&raw const`/`&raw mut` not `&x as *mut _`.
- **Casts**: avoid `as`; use `u32::from`, `try_from`, `.cast_unsigned()`/`.cast_signed()`. Annotate pure fns with `#[must_use]` (except those returning `Result`).
- **Docs**: all public items need `///` with `# Errors`/`# Panics` where applicable (clippy enforces this). Backtick code identifiers (`clippy::doc_markdown`).
- **Logging**: structured key-value form preferred — `log::info!(monitor_id:% = id, brightness = v; "Brightness adjusted")`. Log at the point of *handling*, not occurrence (functions returning `Result` return `Err`, don't log it). Never log PII; serials only at `debug`.

## Code comments

Do not cite ephemeral planning labels (phase/step/wave/feature IDs like `F.6`, `WB0.4`) in code comments — state the rationale in self-contained domain terms. (The existing docs use such labels; that's fine for docs, but don't propagate them into code comments.)
