//! System event listener for Windows.
//!
//! Detects the two classes of system event that invalidate cached monitor
//! state — resuming from sleep, and a display topology or resolution change —
//! and notifies the main thread so it can resync with the hardware.

use std::cell::RefCell;
use std::sync::mpsc::Sender;

use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Power::{
    HPOWERNOTIFY, RegisterSuspendResumeNotification, UnregisterSuspendResumeNotification,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CreateWindowExW, DEVICE_NOTIFY_WINDOW_HANDLE, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GetMessageW, MSG, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
    RegisterClassW, TranslateMessage, WM_DISPLAYCHANGE, WM_POWERBROADCAST, WNDCLASSW,
    WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

use crate::core::state::BrightnessMessage;
use crate::error::{BrightnessError, Result};

thread_local! {
    /// Sender consulted by `power_wnd_proc` to notify the main thread of
    /// resume and display-change events. Both arrive as *sent* messages,
    /// dispatched directly to the window procedure — they never appear in the
    /// `GetMessageW` queue — so the procedure needs its own path to the
    /// channel. Installed on the listener thread before the window is created.
    static WNDPROC_SENDER: RefCell<Option<Sender<BrightnessMessage>>> =
        const { RefCell::new(None) };
}

/// Installs the sender consulted by `power_wnd_proc` on the current thread.
///
/// Must be called on the thread that owns the listener window, before any
/// system event can be delivered to it.
fn install_wndproc_sender(sender: Sender<BrightnessMessage>) {
    WNDPROC_SENDER.with(|cell| *cell.borrow_mut() = Some(sender));
}

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Window procedure for the system event window.
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
    match msg {
        WM_POWERBROADCAST => {
            handle_power_broadcast(wparam);
            // TRUE: message processed, per the WM_POWERBROADCAST contract
            return LRESULT(1);
        }
        WM_DISPLAYCHANGE => {
            // Resolution or topology change: monitors may have been added or
            // removed, and the OS may reuse a monitor handle for a different
            // display. A refresh re-enumerates and rebuilds the handle→identity
            // mapping, so an adjustment cannot land on the wrong monitor. Left
            // to the periodic refresh this could take a minute — or never,
            // since a zero interval legitimately disables it.
            log::info!(reason = "display_change"; "Triggering refresh");
            notify_main(BrightnessMessage::Refresh);
            // Zero: message processed, per the WM_DISPLAYCHANGE contract
            return LRESULT(0);
        }
        _ => {}
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
    #[expect(clippy::cast_possible_truncation)]
    let event_type = wparam.0 as u32;

    match event_type {
        PBT_APMRESUMEAUTOMATIC => {
            log::info!(resume_type = "automatic"; "System resumed from sleep");
            notify_main(BrightnessMessage::SystemResumed);
        }
        PBT_APMRESUMESUSPEND => {
            log::info!(resume_type = "user_action"; "System resumed from sleep");
            notify_main(BrightnessMessage::SystemResumed);
        }
        _ => {
            // Other power events we don't care about
            log::trace!(event_type = event_type; "Power broadcast event ignored");
        }
    }
}

/// Forwards a message to the main thread from inside the window procedure.
fn notify_main(message: BrightnessMessage) {
    WNDPROC_SENDER.with(|cell| match cell.borrow().as_ref() {
        Some(sender) => {
            if let Err(e) = sender.send(message) {
                log::error!(error:% = e; "Failed to notify main thread of system event");
            }
        }
        None => {
            log::warn!("System event received before sender was installed");
        }
    });
}

/// Listens for resume and display-change events and notifies the main thread.
///
/// The listener owns a hidden top-level window. Top-level rather than
/// message-only is load-bearing: message-only windows are excluded from
/// broadcast messages, and `WM_DISPLAYCHANGE` is broadcast. Suspend/resume is
/// additionally subscribed explicitly via `RegisterSuspendResumeNotification`,
/// which is the documented delivery guarantee for `WM_POWERBROADCAST` and does
/// not depend on the window being reachable by broadcast.
///
/// The window procedure translates both events into messages for the main
/// thread: `SystemResumed` on wake, `Refresh` on a display change.
pub struct PowerEventListener {
    /// Handle to the invisible window that receives system events.
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
                // Deliberately NOT a message-only window (`HWND_MESSAGE`):
                // those are excluded from broadcast messages, and
                // WM_DISPLAYCHANGE is broadcast. Never shown, and
                // WS_EX_TOOLWINDOW keeps it out of the taskbar and Alt-Tab.
                None,
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

    /// Runs the message loop to process system events.
    ///
    /// This method blocks until the message loop is terminated (e.g., by `WM_QUIT`).
    /// Both `WM_POWERBROADCAST` and `WM_DISPLAYCHANGE` are *sent* messages
    /// handled inside `power_wnd_proc`; this loop's job is only to keep the
    /// thread pumping so sent messages get delivered.
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

    #[test]
    fn wnd_proc_forwards_display_change_as_refresh() {
        let (tx, rx) = mpsc::channel();
        install_wndproc_sender(tx);

        let result = unsafe {
            power_wnd_proc(
                HWND::default(),
                WM_DISPLAYCHANGE,
                WPARAM(32),
                LPARAM(0x0780_0438),
            )
        };

        assert!(
            matches!(rx.try_recv(), Ok(BrightnessMessage::Refresh)),
            "a display change should trigger a monitor refresh"
        );
        assert_eq!(
            result,
            LRESULT(0),
            "WM_DISPLAYCHANGE should be reported as handled (zero)"
        );
    }

    // Broadcast messages — WM_DISPLAYCHANGE among them — are never delivered
    // to message-only windows, so the listener's window must stay top-level or
    // topology changes silently stop arriving. Pinning it here because that
    // exact mistake previously cost this module its resume detection: a
    // message-only window cannot see WM_POWERBROADCAST either.
    #[test]
    fn listener_window_is_top_level_so_broadcasts_arrive() {
        use windows::Win32::UI::WindowsAndMessaging::{GA_PARENT, GetAncestor, GetDesktopWindow};

        let (tx, _rx) = mpsc::channel();
        let listener = PowerEventListener::new(tx).expect("listener creation");

        // A real top-level window's parent is the desktop. A message-only
        // window is parented to a separate, broadcast-excluded pseudo-desktop
        // instead — the distinction `GetParent` cannot see, since it reports
        // the *owner* (null for both) rather than the parent.
        let parent = unsafe { GetAncestor(listener.hwnd, GA_PARENT) };
        let desktop = unsafe { GetDesktopWindow() };
        assert_eq!(
            parent, desktop,
            "listener window must be parented to the desktop; a message-only \
             window is excluded from broadcast messages"
        );
    }
}
