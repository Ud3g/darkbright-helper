//! Platform-specific implementations.
//!
//! The portability boundary lives in `core`, not here: `core::controller`
//! defines the `OsdSink`, `OverlaySink`, `DdcPort`, `MonitorLocator`,
//! `SettingsSink`, `HotkeyPort`, and `ConfigStore` seams the `Controller` is
//! generic over, and the submodules below provide the Windows
//! implementations. A port to another OS implements those seams (plus its
//! own hotkey/power/tray equivalents and binary wiring).

#[cfg(target_os = "windows")]
pub mod windows;
