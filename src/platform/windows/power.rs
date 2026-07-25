//! Power event listener for Windows.
//!
//! This module provides functionality to detect system power events (sleep/resume)
//! and notify the main thread when the system wakes up, allowing brightness
//! values to be resynced with monitors.

use std::cell::RefCell;
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    HPOWERNOTIFY, RegisterSuspendResumeNotification, UnregisterSuspendResumeNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GetMessageW, HWND_MESSAGE, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
    RegisterClassW, TranslateMessage, WM_POWERBROADCAST, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

use crate::core::state::BrightnessMessage;
use crate::error::{BrightnessError, Result};

thread_local! {
    /// Sender consulted by `power_wnd_proc` to notify the main thread of
    /// resume events. `WM_POWERBROADCAST` arrives as a *sent* message,
    /// dispatched directly to the window procedure — it never appears in the
    /// `GetMessageW` queue — so the procedure needs its own path to the
    /// channel. Installed on the power thread before the window is created.
    static WNDPROC_SENDER: RefCell<Option<Sender<BrightnessMessage>>> =
        const { RefCell::new(None) };
}

/// Installs the sender consulted by `power_wnd_proc` on the current thread.
///
/// Must be called on the thread that owns the power event window, before
/// any `WM_POWERBROADCAST` can be delivered.
fn install_wndproc_sender(sender: Sender<BrightnessMessage>) {
    WNDPROC_SENDER.with(|cell| *cell.borrow_mut() = Some(sender));
}

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
    if msg == WM_POWERBROADCAST {
        handle_power_broadcast(wparam);
        // TRUE: message processed, per the WM_POWERBROADCAST contract
        return LRESULT(1);
    }
    // In Rust 2024, unsafe fn body still requires explicit unsafe block
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Handles a `WM_POWERBROADCAST` message received by the window procedure.
///
/// Detects resume events and notifies the main thread.
fn handle_power_broadcast(wparam: WPARAM) {
    // wparam contains the power event type
    // PBT_APMRESUMEAUTOMATIC: Resume from suspend (automatic)
    // PBT_APMRESUMESUSPEND: Resume from suspend (user action)
    // Power event types fit in u32; truncation is acceptable on 64-bit
    #[allow(clippy::cast_possible_truncation)]
    let event_type = wparam.0 as u32;

    match event_type {
        PBT_APMRESUMEAUTOMATIC => {
            log::info!(resume_type = "automatic"; "System resumed from sleep");
            send_resume_notification();
        }
        PBT_APMRESUMESUSPEND => {
            log::info!(resume_type = "user_action"; "System resumed from sleep");
            send_resume_notification();
        }
        _ => {
            // Other power events we don't care about
            log::trace!(event_type = event_type; "Power broadcast event ignored");
        }
    }
}

/// Sends a `SystemResumed` notification to the main thread.
fn send_resume_notification() {
    WNDPROC_SENDER.with(|cell| match cell.borrow().as_ref() {
        Some(sender) => {
            if let Err(e) = sender.send(BrightnessMessage::SystemResumed) {
                log::error!(error:% = e; "Failed to send SystemResumed message");
            }
        }
        None => {
            log::warn!("Power broadcast received before sender was installed");
        }
    });
}

/// Listens for system power events (sleep/resume) and notifies the main thread.
///
/// This listener creates a hidden message-only window and subscribes it to
/// suspend/resume events via `RegisterSuspendResumeNotification`. The explicit
/// subscription is required: message-only windows are excluded from message
/// broadcasts, so `WM_POWERBROADCAST` would otherwise never reach the window.
/// When a resume event is detected, the window procedure sends a
/// `BrightnessMessage::SystemResumed` to the main thread.
pub struct PowerEventListener {
    /// Handle to the invisible message window that receives power events.
    hwnd: HWND,
    /// Registration handle for the suspend/resume notification subscription.
    notification: HPOWERNOTIFY,
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
    /// Returns a `BrightnessError::WindowsApi` if the message window cannot be
    /// created or the suspend/resume notification cannot be registered.
    ///
    /// # Panics
    ///
    /// Panics if the current process module handle cannot be retrieved.
    pub fn new(sender: Sender<BrightnessMessage>) -> Result<Self> {
        install_wndproc_sender(sender);

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
                Some(HWND_MESSAGE), // Message-only window
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .map_err(|e| BrightnessError::windows_api("CreateWindowExW", e.code().0.cast_unsigned()))?;

        let notification = unsafe {
            RegisterSuspendResumeNotification(HANDLE(hwnd.0), DEVICE_NOTIFY_WINDOW_HANDLE)
        }
        .map_err(|e| {
            unsafe {
                let _ = DestroyWindow(hwnd);
            }
            BrightnessError::windows_api(
                "RegisterSuspendResumeNotification",
                e.code().0.cast_unsigned(),
            )
        })?;

        log::debug!(hwnd:? = hwnd; "Power event listener window created and registered");

        Ok(Self { hwnd, notification })
    }

    /// Runs the message loop to process power events.
    ///
    /// This method blocks until the message loop is terminated (e.g., by `WM_QUIT`).
    /// `WM_POWERBROADCAST` is a *sent* message handled inside `power_wnd_proc`;
    /// this loop's job is only to keep the thread pumping so sent messages get
    /// delivered.
    pub fn run_message_loop(&self) {
        let mut msg = MSG::default();
        unsafe {
            // GetMessageW returns:
            // > 0: Message retrieved
            // 0: WM_QUIT received
            // -1: Error
            while GetMessageW(&raw mut msg, None, 0, 0).0 > 0 {
                let _ = TranslateMessage(&raw const msg);
                let _ = DispatchMessageW(&raw const msg);
            }
        }
        log::debug!("Power event listener message loop exited");
    }
}

impl Drop for PowerEventListener {
    fn drop(&mut self) {
        unsafe {
            let _ = UnregisterSuspendResumeNotification(self.notification);
        }
        if !self.hwnd.is_invalid() {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
            log::debug!(hwnd:? = self.hwnd; "Power event listener window destroyed");
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
    fn wnd_proc_forwards_resume_events_to_main_thread() {
        let (tx, rx) = mpsc::channel();
        install_wndproc_sender(tx);

        for event in [PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND] {
            let result = unsafe {
                power_wnd_proc(
                    HWND::default(),
                    WM_POWERBROADCAST,
                    WPARAM(usize::try_from(event).unwrap()),
                    LPARAM(0),
                )
            };
            assert!(
                matches!(rx.try_recv(), Ok(BrightnessMessage::SystemResumed)),
                "resume event {event:#x} should notify the main thread"
            );
            assert_eq!(
                result,
                LRESULT(1),
                "WM_POWERBROADCAST should be reported as handled (TRUE)"
            );
        }
    }

    #[test]
    fn wnd_proc_ignores_non_resume_power_events() {
        use windows::Win32::UI::WindowsAndMessaging::PBT_APMSUSPEND;

        let (tx, rx) = mpsc::channel();
        install_wndproc_sender(tx);

        unsafe {
            power_wnd_proc(
                HWND::default(),
                WM_POWERBROADCAST,
                WPARAM(usize::try_from(PBT_APMSUSPEND).unwrap()),
                LPARAM(0),
            );
        }
        assert!(
            rx.try_recv().is_err(),
            "suspend event must not notify the main thread"
        );
    }

    #[test]
    fn test_power_listener_creation() {
        let (tx, _rx) = mpsc::channel();
        let listener = PowerEventListener::new(tx);
        // Should succeed in creating the listener
        assert!(listener.is_ok());
    }
}
