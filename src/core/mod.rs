//! Platform-agnostic core logic for brightness control.
//!
//! This module contains the business logic that is shared across all platforms:
//!
//! - [`brightness`] - Brightness calculation and value mapping
//! - [`config`] - Configuration types and file handling
//! - [`controller`] - Message-driven orchestration behind platform seams
//! - `edid` - EDID parsing for monitor identification (crate-internal)
//! - [`logfile`] - Size-capped rolling file sink for diagnostic logging
//! - [`panic_hook`] - Process-wide panic logging hook
//! - [`reconcile`] - Refresh/respawn tracking and reconciliation policies
//! - [`state`] - Application state and inter-thread messages
//! - [`version`] - The user-facing version string, git-describe aware

// `edid` is crate-internal: only `platform::windows::ddc` parses EDID blobs.
// See the note in `platform::windows` — a `pub mod` suppresses `dead_code` for
// everything `pub` inside it, so the narrower the module surface, the more the
// compiler can see.
pub mod brightness;
pub mod config;
pub mod controller;
pub(crate) mod edid;
pub mod logfile;
pub mod panic_hook;
pub mod reconcile;
pub mod state;
pub mod version;
