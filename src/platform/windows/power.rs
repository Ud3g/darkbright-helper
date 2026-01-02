//! Power event listener for Windows.
//!
//! This module provides functionality to detect system power events (sleep/resume)
//! and notify the main thread when the system wakes up, allowing brightness
//! values to be resynced with monitors.

use std::sync::mpsc::Sender;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    HMENU, HWND_MESSAGE, MSG, RegisterClassW, TranslateMessage, WM_POWERBROADCAST, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

use crate::core::state::BrightnessMessage;
use crate::error::{BrightnessError, Result};
use crate::platform::windows::last_error_as_brightness_error;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Power broadcast event: System has resumed from suspend (automatic).
const PBT_APMRESUMEAUTOMATIC: u32 = 0x12;

/// Power broadcast event: System has resumed from suspend (user action).
const PBT_APMRESUMESUSPEND: u32 = 0x07;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Window procedure for the power event message window.
///
/// # Safety
///
/// This is a Windows callback function. It must be called only by the Windows
/// message dispatch system with valid parameters.
unsafe extern "system" fn power_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // In Rust 2024, unsafe fn body still requires explicit unsafe block
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Listens for system power events (sleep/resume) and notifies the main thread.
///
/// This listener creates a hidden message-only window that receives
/// `WM_POWERBROADCAST` messages from Windows. When a resume event is detected,
/// it sends a `BrightnessMessage::SystemResumed` to the main thread.
pub struct PowerEventListener {
    /// Handle to the invisible message window that receives power events.
    hwnd: HWND,
    /// Channel sender to transmit power events to the main thread.
    sender: Sender<BrightnessMessage>,
}

impl PowerEventListener {
    /// Creates a new `PowerEventListener`.
    ///
    /// # Arguments
    ///
    /// * `sender` - Channel sender to notify the main thread of power events.
    ///
    /// # Errors
    ///
    /// Returns a `BrightnessError::WindowsApi` if the message window cannot be created.
    ///
    /// # Panics
    ///
    /// Panics if the current process module handle cannot be retrieved.
    pub fn new(sender: Sender<BrightnessMessage>) -> Result<Self> {
        let hinstance = unsafe {
            GetModuleHandleW(None).map_err(|e| {
                BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
            })?
        };
        let class_name = w!("DarkBrightPowerWindow");

        let wnd_class = WNDCLASSW {
            lpfnWndProc: Some(power_wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };

        unsafe {
            // RegisterClassW returns 0 on failure, but may also return 0
            // if the class already exists. We ignore the result and proceed.
            RegisterClassW(&raw const wnd_class);
        }

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name,
                w!("DarkBrightPower"),
                WS_POPUP,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                HWND_MESSAGE, // Message-only window
                HMENU::default(),
                hinstance,
                None,
            )
        };

        if hwnd.0 == 0 {
            return Err(last_error_as_brightness_error("CreateWindowExW"));
        }

        log::debug!("Power event listener window created");

        Ok(Self { hwnd, sender })
    }

    /// Runs the message loop to process power events.
    ///
    /// This method blocks until the message loop is terminated (e.g., by `WM_QUIT`).
    /// It listens for `WM_POWERBROADCAST` messages and sends `SystemResumed`
    /// when the system wakes from sleep/hibernate.
    pub fn run_message_loop(&self) {
        let mut msg = MSG::default();
        unsafe {
            // GetMessageW returns:
            // > 0: Message retrieved
            // 0: WM_QUIT received
            // -1: Error
            while GetMessageW(&raw mut msg, HWND::default(), 0, 0).0 > 0 {
                if msg.message == WM_POWERBROADCAST {
                    self.handle_power_broadcast(msg.wParam);
                }

                let _ = TranslateMessage(&raw const msg);
                let _ = DispatchMessageW(&raw const msg);
            }
        }
        log::debug!("Power event listener message loop exited");
    }

    /// Handles a `WM_POWERBROADCAST` message.
    ///
    /// Detects resume events and notifies the main thread.
    fn handle_power_broadcast(&self, wparam: WPARAM) {
        // wparam contains the power event type
        // PBT_APMRESUMEAUTOMATIC: Resume from suspend (automatic)
        // PBT_APMRESUMESUSPEND: Resume from suspend (user action)
        let event_type = wparam.0 as u32;

        match event_type {
            PBT_APMRESUMEAUTOMATIC => {
                log::info!("System resumed from sleep (automatic)");
                self.send_resume_notification();
            }
            PBT_APMRESUMESUSPEND => {
                log::info!("System resumed from sleep (user action)");
                self.send_resume_notification();
            }
            _ => {
                // Other power events we don't care about
                log::trace!("Power broadcast event: {event_type:#x}");
            }
        }
    }

    /// Sends a `SystemResumed` notification to the main thread.
    fn send_resume_notification(&self) {
        if let Err(e) = self.sender.send(BrightnessMessage::SystemResumed) {
            log::error!("Failed to send SystemResumed message: {e}");
        }
    }
}

impl Drop for PowerEventListener {
    fn drop(&mut self) {
        if self.hwnd.0 != 0 {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
            log::debug!("Power event listener window destroyed");
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn test_power_listener_creation() {
        let (tx, _rx) = mpsc::channel();
        let listener = PowerEventListener::new(tx);
        // Should succeed in creating the listener
        assert!(listener.is_ok());
    }
}
