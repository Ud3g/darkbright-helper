//! System tray icon and menu implementation for Windows.
//!
//! This module provides a system tray icon with a context menu that allows users to:
//! - View current brightness/overlay levels for all monitors
//! - Open the settings (config.json) file
//! - Quit the application
//!
//! The tray icon runs its own message loop on a dedicated thread and communicates
//! with the main thread via `BrightnessMessage` channels.

use std::sync::mpsc::Sender;

use windows::Win32::UI::WindowsAndMessaging::HICON;

use crate::core::state::BrightnessMessage;
use crate::error::Result;

use super::SafeHwnd;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Window class name for the tray message-only window.
const TRAY_WINDOW_CLASS: &str = "BrightnessControlTrayWindow";

/// Unique identifier for the tray icon (used with Shell_NotifyIconW).
const TRAY_ICON_ID: u32 = 1;

/// Custom window message for tray icon callbacks.
/// Using WM_APP + offset to avoid conflicts with system messages.
const WM_TRAY_CALLBACK: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 100;

// ─────────────────────────────────────────────────────────────────────────────
// Menu Item IDs
// ─────────────────────────────────────────────────────────────────────────────

/// Menu item ID for the "Settings" option.
const MENU_ID_SETTINGS: u32 = 1001;

/// Menu item ID for the "Quit" option.
const MENU_ID_QUIT: u32 = 1002;

/// Base ID for monitor info rows (non-clickable).
/// Each monitor uses MENU_ID_MONITOR_BASE + index.
const MENU_ID_MONITOR_BASE: u32 = 2000;

// ─────────────────────────────────────────────────────────────────────────────
// Tray Icon Manager
// ─────────────────────────────────────────────────────────────────────────────

/// Manages the system tray icon and its context menu.
///
/// The `TrayIcon` creates an invisible message-only window to receive
/// tray icon notifications and handles the popup menu when the user
/// right-clicks the icon.
pub struct TrayIcon {
    /// Message-only window that receives tray notifications.
    hwnd: SafeHwnd,
    /// Channel sender to communicate with the main thread.
    sender: Sender<BrightnessMessage>,
    /// Handle to the loaded icon resource.
    icon_handle: HICON,
}

impl TrayIcon {
    /// Creates a new `TrayIcon` and registers it with the system tray.
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel to send messages to the main thread.
    ///
    /// # Errors
    ///
    /// Returns `BrightnessError::TrayIconCreation` if:
    /// - The message window cannot be created
    /// - The icon resource cannot be loaded
    /// - The tray icon cannot be registered with the shell
    pub fn new(sender: Sender<BrightnessMessage>) -> Result<Self> {
        // TODO: Step 6 - Create message-only window
        // TODO: Step 7 - Load icon and register with shell
        todo!("TrayIcon::new() - to be implemented in Steps 6-7")
    }

    /// Runs the message loop for the tray icon.
    ///
    /// This method blocks until a `WM_QUIT` message is received.
    /// It should be called on a dedicated thread.
    ///
    /// # Errors
    ///
    /// Returns an error if the message loop encounters a fatal error.
    pub fn run_message_loop(&self) -> Result<()> {
        // TODO: Step 6 - Implement message loop
        todo!("TrayIcon::run_message_loop() - to be implemented in Step 6")
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        // TODO: Step 7 - Remove tray icon with Shell_NotifyIconW(NIM_DELETE)
        log::debug!("TrayIcon dropped");
    }
}
