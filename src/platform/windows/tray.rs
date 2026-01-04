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
use std::sync::OnceLock;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, HICON, HWND_MESSAGE, MSG,
    PostQuitMessage, RegisterClassExW, TranslateMessage, WM_DESTROY, WNDCLASSEXW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

use crate::core::state::BrightnessMessage;
use crate::error::{BrightnessError, Result};

use super::{SafeHwnd, last_error_as_brightness_error};

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
// Window Class Registration
// ─────────────────────────────────────────────────────────────────────────────

/// Ensures the tray window class is registered exactly once.
static REGISTER_CLASS_ONCE: OnceLock<Result<()>> = OnceLock::new();

/// Registers the window class for the tray message-only window if not already registered.
///
/// # Errors
///
/// Returns `BrightnessError::TrayIconCreation` if `GetModuleHandleW` or `RegisterClassExW` fails.
fn ensure_tray_class_registered() -> Result<PCWSTR> {
    REGISTER_CLASS_ONCE
        .get_or_init(|| {
            unsafe {
                let hinstance = GetModuleHandleW(None).map_err(|e| {
                    BrightnessError::tray_icon_creation(format!(
                        "GetModuleHandleW failed: {}",
                        e.code().0
                    ))
                })?;

                let class_name = w!("BrightnessControlTrayWindow");

                let wnd_class = WNDCLASSEXW {
                    cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(0),
                    lpfnWndProc: Some(tray_wnd_proc),
                    hInstance: hinstance.into(),
                    lpszClassName: class_name,
                    ..Default::default()
                };

                if RegisterClassExW(&raw const wnd_class) == 0 {
                    return Err(last_error_as_brightness_error("RegisterClassExW"));
                }

                log::debug!("Tray window class registered");
            }
            Ok(())
        })
        .as_ref()
        .map_err(|e| BrightnessError::tray_icon_creation(format!("Class registration failed: {e}")))?;

    Ok(w!("BrightnessControlTrayWindow"))
}

// ─────────────────────────────────────────────────────────────────────────────
// Window Procedure
// ─────────────────────────────────────────────────────────────────────────────

/// Window procedure for the tray message-only window.
///
/// Handles tray icon callback messages and cleanup.
///
/// # Safety
///
/// This is a Windows callback. The caller (Windows) ensures `hwnd` is valid.
unsafe extern "system" fn tray_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        match msg {
            WM_TRAY_CALLBACK => {
                handle_tray_callback(hwnd, lparam);
                LRESULT(0)
            }
            WM_DESTROY => {
                log::debug!("Tray window WM_DESTROY received");
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }
}

/// Handles tray icon callback messages (mouse events on the tray icon).
///
/// # Arguments
///
/// * `_hwnd` - The window handle (unused for now, needed for menu positioning later).
/// * `lparam` - Contains the mouse message (e.g., `WM_RBUTTONUP`).
fn handle_tray_callback(_hwnd: HWND, lparam: LPARAM) {
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_RBUTTONUP};

    // The low word of lparam contains the mouse message
    #[allow(clippy::cast_possible_truncation)]
    let mouse_msg = (lparam.0 & 0xFFFF) as u32;

    match mouse_msg {
        WM_RBUTTONUP => {
            log::debug!("Tray icon right-clicked");
            // TODO: Step 9 - Show context menu
        }
        WM_LBUTTONUP => {
            log::debug!("Tray icon left-clicked (no action)");
            // Per requirements: left-click does nothing
        }
        _ => {}
    }
}

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
        let class_name = ensure_tray_class_registered()?;

        // Create message-only window (HWND_MESSAGE parent = no visible window)
        let hwnd = unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| {
                BrightnessError::tray_icon_creation(format!(
                    "GetModuleHandleW failed: {}",
                    e.code().0
                ))
            })?;

            let hwnd = CreateWindowExW(
                Default::default(), // No extended styles needed
                class_name,
                w!("BrightnessControlTray"),
                WS_OVERLAPPED, // Minimal style for message-only window
                0,
                0,
                0,
                0,
                HWND_MESSAGE, // Message-only window
                None,
                hinstance,
                None,
            );

            if hwnd.0 == 0 {
                return Err(last_error_as_brightness_error("CreateWindowExW"));
            }

            hwnd
        };

        log::debug!("Tray message window created");

        // TODO: Step 7 - Load icon and register with shell

        Ok(Self {
            hwnd: unsafe { SafeHwnd::new_owned(hwnd) },
            sender,
            icon_handle: HICON::default(),
        })
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
        log::info!("Tray message loop started");

        let mut msg = MSG::default();

        loop {
            let result = unsafe { GetMessageW(&raw mut msg, HWND::default(), 0, 0) };

            match result.0 {
                -1 => {
                    // Error occurred
                    return Err(last_error_as_brightness_error("GetMessageW"));
                }
                0 => {
                    // WM_QUIT received
                    log::info!("Tray message loop received WM_QUIT");
                    break;
                }
                _ => {
                    // Process the message
                    unsafe {
                        let _ = TranslateMessage(&raw const msg);
                        DispatchMessageW(&raw const msg);
                    }
                }
            }
        }

        Ok(())
    }

    /// Returns the window handle for the tray message window.
    #[must_use]
    pub fn hwnd(&self) -> HWND {
        self.hwnd.as_raw()
    }

    /// Returns a clone of the message sender.
    #[must_use]
    pub fn sender(&self) -> Sender<BrightnessMessage> {
        self.sender.clone()
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        // TODO: Step 7 - Remove tray icon with Shell_NotifyIconW(NIM_DELETE)
        log::debug!("TrayIcon dropped");
    }
}
