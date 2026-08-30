//! Windows settings dialog window.
//!
//! The window runs on its own thread with its own `GetMessageW` loop —
//! load-bearing, not stylistic. Dragging the title bar or opening an edit's
//! context menu enters an OS-internal modal loop that does not return until
//! the interaction ends; on the main thread that would stall controller
//! ticks and watchdogs long enough to trip the refresh watchdog. The thread
//! is spawned per open and exits when the window closes — there is no
//! long-lived idle thread and no supervision, matching how rarely this
//! window is used.
//!
//! Every group box the design mockup shows was replaced with a bold
//! `STATIC` label plus an `SS_ETCHEDHORZ` separator: a dark-mode spike
//! (see the module history) measured `BS_GROUPBOX`'s frame and caption as
//! unreadable in dark mode, while a plain `STATIC` takes the light text
//! colour correctly. That also removes any z-order constraint between
//! frames and their contents — creation order here is simply visual and
//! tab order.
//!
//! Control wiring is instant-apply (see [`window`]'s "Control Wiring"
//! section) including hotkey capture (see [`capture`]). Declarative layout
//! data and geometry (control ids, styles, the [`layout::CONTROLS`] table,
//! DPI scaling, window placement) lives in [`layout`]; the hotkey capture
//! control is its own self-contained window class in [`capture`]; every
//! control's dark-mode painting (custom draw, window subclassing, the
//! `WM_CTLCOLOR*` colour table) lives in [`dark`]; everything else — window
//! creation, control wiring, the message loop and the `SettingsSink` seam —
//! lives in [`window`].

mod capture;
mod dark;
mod layout;
mod window;

pub use window::SettingsSinkImpl;
// Nothing elsewhere in the crate currently names these through this path —
// `window` itself reaches them directly, being their defining module — but
// they were `pub` before this module became a directory, so the path stays
// live for whatever future caller (or external consumer, since `settings`
// could widen past `pub(crate)` later) expects it.
#[allow(unused_imports)]
pub use window::{
    WM_APP_SETTINGS_FOCUS, WM_APP_SETTINGS_HK_ERROR, WM_APP_SETTINGS_HK_NOTICE,
    WM_APP_SETTINGS_REFRESH, WM_APP_SETTINGS_TOPMOST,
};
