//! Hotkey parsing and registration for Windows.
//!
//! This module provides functionality to parse hotkey strings (e.g., "Ctrl+Shift+Up")
//! and register them as global hotkeys using the Windows `RegisterHotKey` API.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, LazyLock, Mutex};

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_SHIFT, MOD_WIN, RegisterHotKey, UnregisterHotKey,
    VIRTUAL_KEY, VK_BACK, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F1, VK_F2, VK_F3, VK_F4, VK_F5,
    VK_F6, VK_F7, VK_F8, VK_F9, VK_F10, VK_F11, VK_F12, VK_HOME, VK_INSERT, VK_LEFT, VK_NEXT,
    VK_OEM_MINUS, VK_OEM_PLUS, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SPACE, VK_TAB, VK_UP,
};

/// Virtual-key codes for the dedicated brightness keys many laptop keyboards
/// carry. Spelled out here because the `windows` crate does not name them.
pub(crate) const VK_BRIGHTNESS_UP: VIRTUAL_KEY = VIRTUAL_KEY(0xE8);
/// Companion of [`VK_BRIGHTNESS_UP`].
pub(crate) const VK_BRIGHTNESS_DOWN: VIRTUAL_KEY = VIRTUAL_KEY(0xE9);

/// Hook code indicating the hook procedure must process the message.
/// Used in low-level keyboard hook callbacks.
const HC_ACTION: i32 = 0;
use windows::Win32::UI::WindowsAndMessaging::{
    CW_USEDEFAULT, CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow,
    DispatchMessageW, GetMessageW, HHOOK, HWND_MESSAGE, KBDLLHOOKSTRUCT, MSG, PostThreadMessageW,
    RegisterClassW, SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL,
    WM_APP, WM_HOTKEY, WM_KEYDOWN, WM_SYSKEYDOWN, WNDCLASSW, WS_EX_TOOLWINDOW, WS_POPUP,
};
use windows::core::w;

use crate::core::controller::HotkeyPort;
use crate::core::state::{BrightnessMessage, HotkeyOp};
use crate::error::{BrightnessError, Result};
use crate::platform::windows::last_error_as_brightness_error;

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Hotkey ID for the primary brightness up command.
pub const BRIGHTNESS_UP_ID: i32 = 1;

/// Hotkey ID for the primary brightness down command.
pub const BRIGHTNESS_DOWN_ID: i32 = 2;

/// Hotkey ID for the secondary (dedicated key) brightness up command.
pub(crate) const BRIGHTNESS_UP_ALT_ID: i32 = 3;

/// Hotkey ID for the secondary (dedicated key) brightness down command.
pub(crate) const BRIGHTNESS_DOWN_ALT_ID: i32 = 4;

/// Thread message posted to wake the hotkey message loop and make it drain
/// [`HotkeyCommandQueue`]. Delivered via `PostThreadMessageW`, so it always
/// arrives with `MSG::hwnd` null — that is what distinguishes it from a
/// window message in `run_message_loop`.
///
/// Because it is thread-addressed rather than class-addressed, its value has
/// to be free across every window class hosted on this thread, which is
/// `DarkBrightHotkeyWindow` alone. Adding another class here, or another
/// thread message, means re-checking that.
pub(crate) const WM_APP_HOTKEY_WAKE: u32 = WM_APP + 10;

// ─────────────────────────────────────────────────────────────────────────────
// In-Place Rebind Command Queue
// ─────────────────────────────────────────────────────────────────────────────

/// One in-place operation posted to the hotkey thread.
///
/// Carries plain strings (not `ParsedHotkey`) because the queue crosses a
/// thread boundary and the hotkey thread is the one place equipped to parse
/// and act on them; a string that fails to parse is reported back as a
/// failed [`BrightnessMessage::HotkeyRebindResult`] rather than panicking.
///
/// `pub` even though the binary never names it: it appears inside
/// [`HotkeyCommandQueue`], which the binary does name, so narrowing this to
/// `pub(crate)` makes that alias expose a private type.
#[derive(Debug, Clone)]
pub enum HotkeyThreadCommand {
    /// Re-register with new bindings and/or a new intercept setting.
    Rebind {
        /// New brightness-up hotkey string.
        up: String,
        /// New brightness-down hotkey string.
        down: String,
        /// New low-level-hook interception setting.
        intercept: bool,
    },
    /// Stop delivering brightness hotkeys (capture field has focus).
    Suspend,
    /// Resume delivering brightness hotkeys (capture field lost focus).
    Resume,
}

/// Queue of pending [`HotkeyThreadCommand`]s, shared between [`HotkeyPortImpl`]
/// (producer, on the main thread) and the hotkey thread's message loop
/// (consumer). Commands are drained out of the lock before being acted on,
/// since applying one can call into Win32 (`RegisterHotKey`,
/// `SetWindowsHookExW`) and must never do so while holding the mutex.
pub type HotkeyCommandQueue = Arc<Mutex<VecDeque<HotkeyThreadCommand>>>;

/// Drains every command currently queued, releasing the lock before the
/// caller acts on any of them.
fn drain_commands(queue: &HotkeyCommandQueue) -> Vec<HotkeyThreadCommand> {
    let mut guard = queue
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.drain(..).collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Low-Level Keyboard Hook Support
// ─────────────────────────────────────────────────────────────────────────────

/// Context data for the low-level keyboard hook callback.
///
/// Since the hook callback is a static `extern "system" fn` that cannot capture
/// state, we use thread-local storage to pass context to the callback.
struct HookContext {
    /// Channel sender to transmit brightness adjustment events.
    sender: Sender<BrightnessMessage>,
}

thread_local! {
    /// Thread-local storage for hook callback context.
    ///
    /// Initialized by `set_hook_context()` before installing the hook,
    /// accessed by `with_hook_context()` from the callback.
    static HOOK_CONTEXT: RefCell<Option<HookContext>> = const { RefCell::new(None) };
}

/// Initializes the thread-local hook context.
///
/// Must be called on the same thread where the hook will be installed,
/// before calling `SetWindowsHookExW`.
fn set_hook_context(sender: Sender<BrightnessMessage>) {
    HOOK_CONTEXT.with(|ctx| {
        *ctx.borrow_mut() = Some(HookContext { sender });
    });
}

/// Executes a closure with access to the hook context.
///
/// Returns `None` if the context has not been initialized.
fn with_hook_context<R>(f: impl FnOnce(&HookContext) -> R) -> Option<R> {
    HOOK_CONTEXT.with(|ctx| ctx.borrow().as_ref().map(f))
}

/// RAII wrapper for a Windows hook handle (`HHOOK`).
///
/// Automatically calls `UnhookWindowsHookEx` when dropped to ensure
/// the hook is always unregistered.
struct SafeHook(HHOOK);

impl SafeHook {
    /// Creates a new `SafeHook` from a raw `HHOOK`.
    ///
    /// # Safety
    ///
    /// The caller must ensure that `hook` is a valid hook handle
    /// returned by `SetWindowsHookExW`.
    const unsafe fn new(hook: HHOOK) -> Self {
        Self(hook)
    }
}

impl Drop for SafeHook {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: We own the hook handle and it was valid when created.
            // UnhookWindowsHookEx is safe to call on a valid hook handle.
            unsafe {
                if UnhookWindowsHookEx(self.0).is_err() {
                    log::warn!("Failed to unhook keyboard hook");
                }
            }
        }
    }
}

/// Low-level keyboard hook callback procedure.
///
/// This function is called by Windows for every keyboard event when the hook is installed.
/// It intercepts `VK_BRIGHTNESS_UP` and `VK_BRIGHTNESS_DOWN` keys, sends brightness
/// adjustment messages, and suppresses the native Windows brightness OSD.
///
/// # Safety
///
/// This is a Windows callback function. It must:
/// - Be called only by Windows as part of the hook chain
/// - Have a valid `KBDLLHOOKSTRUCT` pointer in `lparam` when `code == HC_ACTION`
unsafe extern "system" fn low_level_keyboard_proc(
    code: i32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // If code < 0, we must pass to next hook without processing
    if code < 0 {
        return unsafe { CallNextHookEx(None, code, wparam, lparam) };
    }

    // Only process if code indicates we should (HC_ACTION = 0)
    if code == HC_ACTION {
        // Check for key-down events only (ignore key-up to avoid double-firing)
        // Message types (WM_KEYDOWN, WM_SYSKEYDOWN) are well-defined u32 constants.
        #[expect(clippy::cast_possible_truncation)]
        let msg_type = wparam.0 as u32;
        if msg_type == WM_KEYDOWN || msg_type == WM_SYSKEYDOWN {
            // SAFETY: When code == HC_ACTION, lparam points to a valid KBDLLHOOKSTRUCT
            let kb_struct = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            // Virtual key codes are 16-bit values (0x00-0xFF typical, max 0xFFFF).
            #[expect(clippy::cast_possible_truncation)]
            let vk_code = VIRTUAL_KEY(kb_struct.vkCode as u16);

            // Check if this is a brightness key we want to intercept
            let direction = if vk_code == VK_BRIGHTNESS_UP {
                Some(1)
            } else if vk_code == VK_BRIGHTNESS_DOWN {
                Some(-1)
            } else {
                None
            };

            if let Some(direction) = direction {
                // Try to send brightness adjustment via thread-local context
                let sent = with_hook_context(|ctx| {
                    // No routine logging here: with file logging at debug, a
                    // log call does mutex-guarded disk I/O, and a slow write
                    // inside an LL hook risks Windows silently removing the
                    // hook on timeout. The keypress is still logged at debug
                    // when the main loop receives the message.
                    if let Err(e) = ctx.sender.send(BrightnessMessage::AdjustStep { direction }) {
                        log::error!(error:% = e; "Failed to send brightness adjustment from hook");
                    }
                });

                // If we successfully processed the key, suppress it (don't pass to Shell)
                if sent.is_some() {
                    return LRESULT(1);
                }
            }
        }
    }

    // Pass unhandled keys to the next hook in the chain
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Window procedure for the hotkey message window.
///
/// `WM_HOTKEY` is handled in the thread's message loop rather than here — a
/// message-only window needs no painting or input behaviour of its own — so
/// every message goes straight to the default procedure.
///
/// # Safety
///
/// This is a Windows callback. The caller (Windows) ensures `hwnd` is valid.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // In Rust 2024, unsafe fn body still requires explicit unsafe block
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Manages global hotkey registration and handling.
pub struct HotkeyManager {
    /// Handle to the invisible message window that receives `WM_HOTKEY` messages.
    hwnd: HWND,
    /// List of currently registered hotkey IDs.
    registered_ids: Vec<i32>,
    /// Channel sender to transmit brightness adjustment events to the main thread.
    sender: Sender<BrightnessMessage>,
    /// Low-level keyboard hook for intercepting brightness keys (optional).
    keyboard_hook: Option<SafeHook>,
    /// Bindings this manager is meant to have registered: the target that
    /// [`Self::restore_previous_bindings`] restores to after a failed rebind
    /// and that [`Self::resume`] reapplies. `None` before the first
    /// successful apply. Not a live guarantee of what is actually
    /// registered right now — if a restore attempt itself fails, these still
    /// describe it (nothing else to fall back to), even though the OS-level
    /// registration may be partial or absent; the next successful apply
    /// (including the next resume, since `apply_bindings` always
    /// unregisters before it registers) reconciles the two again.
    current_up: Option<ParsedHotkey>,
    /// See [`Self::current_up`].
    current_down: Option<ParsedHotkey>,
    /// Intercept setting paired with [`Self::current_up`]/[`Self::current_down`].
    current_intercept: bool,
}

impl HotkeyManager {
    /// Creates a new `HotkeyManager`.
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
        let class_name = w!("DarkBrightHotkeyWindow");

        let wnd_class = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            lpszClassName: class_name,
            ..Default::default()
        };

        unsafe {
            RegisterClassW(&raw const wnd_class);
        }

        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TOOLWINDOW,
                class_name,
                w!("DarkBrightHotkey"),
                WS_POPUP,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                Some(HWND_MESSAGE),
                None,
                Some(hinstance.into()),
                None,
            )
        }
        .map_err(|e| BrightnessError::windows_api("CreateWindowExW", e.code().0.cast_unsigned()))?;

        Ok(Self {
            hwnd,
            registered_ids: Vec::new(),
            sender,
            keyboard_hook: None,
            current_up: None,
            current_down: None,
            current_intercept: false,
        })
    }

    /// Registers a global hotkey.
    ///
    /// # Arguments
    ///
    /// * `id` - Unique identifier for the hotkey.
    /// * `modifiers` - Modifier keys (Ctrl, Alt, Shift, Win).
    /// * `vk` - Virtual key code.
    ///
    /// # Errors
    ///
    /// Returns `BrightnessError::WindowsApi` if registration fails.
    pub(crate) fn register_hotkey(
        &mut self,
        id: i32,
        modifiers: HOT_KEY_MODIFIERS,
        vk: VIRTUAL_KEY,
    ) -> Result<()> {
        unsafe {
            RegisterHotKey(Some(self.hwnd), id, modifiers, u32::from(vk.0)).map_err(|e| {
                BrightnessError::windows_api("RegisterHotKey", e.code().0.cast_unsigned())
            })?;
        }
        log::debug!(hotkey_id = id; "Registered hotkey");
        self.registered_ids.push(id);
        Ok(())
    }

    /// Installs a low-level keyboard hook to intercept dedicated brightness keys.
    ///
    /// This hook captures `VK_BRIGHTNESS_UP` and `VK_BRIGHTNESS_DOWN` before the
    /// Windows Shell processes them, suppressing the native brightness OSD.
    ///
    /// The hook must be installed on the same thread that runs the message loop,
    /// as Windows delivers hook callbacks to the thread that installed the hook.
    ///
    /// # Errors
    ///
    /// Returns `BrightnessError::WindowsApi` if `SetWindowsHookExW` fails.
    fn install_brightness_hook(&mut self) -> Result<()> {
        // Initialize thread-local context for the hook callback
        set_hook_context(self.sender.clone());

        // Install the low-level keyboard hook
        // SAFETY: We pass a valid callback function. The hook handle will be
        // stored in SafeHook which ensures cleanup on drop.
        let hook = unsafe {
            SetWindowsHookExW(WH_KEYBOARD_LL, Some(low_level_keyboard_proc), None, 0).map_err(
                |e| BrightnessError::windows_api("SetWindowsHookExW", e.code().0.cast_unsigned()),
            )?
        };

        // SAFETY: The hook handle is valid as SetWindowsHookExW succeeded.
        self.keyboard_hook = Some(unsafe { SafeHook::new(hook) });

        log::debug!("Low-level keyboard hook installed for brightness keys");
        Ok(())
    }

    /// Unregisters every currently registered hotkey id and uninstalls the
    /// keyboard hook (if installed), leaving nothing registered.
    ///
    /// `UnregisterHotKey` failures are logged, not propagated: whatever
    /// happens, `registered_ids` is cleared and the hook handle is dropped,
    /// so the manager's own bookkeeping never disagrees with reality even if
    /// the OS call itself failed (e.g. the id was already gone).
    fn unregister_all(&mut self) {
        for id in self.registered_ids.drain(..) {
            unsafe {
                if let Err(e) = UnregisterHotKey(Some(self.hwnd), id) {
                    log::debug!(hotkey_id = id, error:% = e; "UnregisterHotKey failed");
                }
            }
        }
        // Dropping the hook here (rather than only in HotkeyManager's own
        // Drop) is what lets suspend/rebind stop key interception without
        // tearing down the whole manager.
        self.keyboard_hook = None;
    }

    /// Registers the dedicated brightness keys (`VK_BRIGHTNESS_UP/DOWN`) as
    /// plain hotkeys. Non-fatal: another app or the shell may already own
    /// them, or the low-level hook may already be handling them instead.
    fn register_secondary_brightness_hotkeys(&mut self) {
        if let Err(e) =
            self.register_hotkey(BRIGHTNESS_UP_ALT_ID, HOT_KEY_MODIFIERS(0), VK_BRIGHTNESS_UP)
        {
            log::debug!(error:% = e; "Secondary brightness up hotkey not registered");
        }

        if let Err(e) = self.register_hotkey(
            BRIGHTNESS_DOWN_ALT_ID,
            HOT_KEY_MODIFIERS(0),
            VK_BRIGHTNESS_DOWN,
        ) {
            log::debug!(error:% = e; "Secondary brightness down hotkey not registered");
        }
    }

    /// Replaces whatever is currently registered with `up`/`down`/`intercept`,
    /// used for the initial startup registration, every live rebind, and to
    /// restore the previous bindings after a failed rebind.
    ///
    /// Always starts from a clean slate (`unregister_all`), so re-applying
    /// the same combination the manager already holds can never collide with
    /// itself — the same thread that owns a `RegisterHotKey` registration is
    /// the only one allowed to replace it, and it always unregisters first.
    ///
    /// Returns `Ok(true)` when `intercept` was requested but the low-level
    /// hook could not be installed and the dedicated keys fall back to plain
    /// registration instead (a notice, not a failure); `Ok(false)` when the
    /// requested mode was applied exactly as asked.
    ///
    /// # Errors
    ///
    /// Returns `BrightnessError::HotkeyRegistration` if either primary
    /// combination fails to register. Nothing further is attempted in that
    /// case: whichever primary registered stays registered, and the caller
    /// decides whether to try restoring a previous, known-good binding.
    fn apply_bindings(
        &mut self,
        up: ParsedHotkey,
        down: ParsedHotkey,
        intercept: bool,
    ) -> Result<bool> {
        self.unregister_all();

        self.register_hotkey(BRIGHTNESS_UP_ID, up.modifiers, up.vk_code)
            .map_err(|e| BrightnessError::hotkey_registration(up.to_string(), e.to_string()))?;
        self.register_hotkey(BRIGHTNESS_DOWN_ID, down.modifiers, down.vk_code)
            .map_err(|e| BrightnessError::hotkey_registration(down.to_string(), e.to_string()))?;

        // Runs on every apply — startup, a live rebind, and a resume — not
        // just once at process start, so the "installed" confirmation is
        // left to install_brightness_hook's own debug-level log rather than
        // repeated here at info on every call.
        let fallback_active = if intercept {
            match self.install_brightness_hook() {
                Ok(()) => false,
                Err(e) => {
                    log::warn!(
                        error:% = e;
                        "Failed to install brightness key hook; falling back to plain registration"
                    );
                    self.register_secondary_brightness_hotkeys();
                    true
                }
            }
        } else {
            log::debug!(
                "Brightness key interception disabled (starting up with it off, or a live setting change turned it off)"
            );
            self.register_secondary_brightness_hotkeys();
            false
        };

        self.current_up = Some(up);
        self.current_down = Some(down);
        self.current_intercept = intercept;
        Ok(fallback_active)
    }

    /// Unregisters everything (primaries, secondaries, hook) without
    /// changing `current_up`/`current_down`/`current_intercept`, so a later
    /// resume knows what to re-apply.
    fn suspend_all(&mut self) {
        self.unregister_all();
    }

    /// Re-applies `current_up`/`current_down`/`current_intercept`, i.e. what
    /// resume means: undo a suspend without changing the configured bindings.
    ///
    /// # Errors
    ///
    /// Returns `BrightnessError::HotkeyRegistration` if nothing has ever been
    /// successfully applied yet (defensive; the thread always applies the
    /// startup config before the message loop can receive a resume) — this
    /// text ends up in the settings dialog's inline error, so it must
    /// describe this condition rather than borrow a channel-closed message
    /// that would misdescribe it. Otherwise propagates whatever
    /// [`Self::apply_bindings`] returns.
    fn resume(&mut self) -> Result<bool> {
        let (Some(up), Some(down)) = (self.current_up, self.current_down) else {
            return Err(BrightnessError::hotkey_registration(
                "brightness hotkeys",
                "no previously applied bindings to resume (thread never completed its initial registration)",
            ));
        };
        self.apply_bindings(up, down, self.current_intercept)
    }

    /// After a failed rebind, tries to put back the bindings that were
    /// active before the attempt. Returns `None` on success, or `Some`
    /// describing why the restore itself also failed (there is nothing left
    /// to fall back to at that point; the caller reports both failures and
    /// leaves the thread alive with whatever partial state resulted).
    fn restore_previous_bindings(&mut self) -> Option<String> {
        let (Some(up), Some(down)) = (self.current_up, self.current_down) else {
            return Some("no previous bindings to restore".to_string());
        };
        self.apply_bindings(up, down, self.current_intercept)
            .err()
            .map(|e| e.to_string())
    }

    /// Sends the ack the controller's [`crate::core::controller::HotkeyPort`]
    /// side waits for. Errors (channel closed) are dropped: if the main
    /// thread is gone there is nothing left to notify.
    fn send_ack(&self, op: HotkeyOp, success: bool, fallback_active: bool, error: Option<String>) {
        let _ = self.sender.send(BrightnessMessage::HotkeyRebindResult {
            op,
            success,
            fallback_active,
            error,
        });
    }

    /// Applies one posted command and sends its ack.
    ///
    /// A `Rebind` with an unparseable string is reported as a failure with
    /// no restore attempt (nothing was unregistered yet, so there is nothing
    /// to put back) — this should not happen in practice since the dialog
    /// validates strings before posting, but the thread must never trust
    /// that from across a channel.
    fn handle_command(&mut self, command: HotkeyThreadCommand) {
        match command {
            HotkeyThreadCommand::Rebind {
                up,
                down,
                intercept,
            } => {
                let parsed = parse_hotkey(&up).and_then(|u| parse_hotkey(&down).map(|d| (u, d)));
                match parsed {
                    Ok((up, down)) => match self.apply_bindings(up, down, intercept) {
                        Ok(fallback_active) => {
                            self.send_ack(HotkeyOp::Rebind, true, fallback_active, None);
                        }
                        Err(e) => {
                            let message = match self.restore_previous_bindings() {
                                Some(restore_err) => {
                                    format!("{e}; restore also failed: {restore_err}")
                                }
                                None => e.to_string(),
                            };
                            self.send_ack(HotkeyOp::Rebind, false, false, Some(message));
                        }
                    },
                    Err(e) => {
                        self.send_ack(HotkeyOp::Rebind, false, false, Some(e.to_string()));
                    }
                }
            }
            HotkeyThreadCommand::Suspend => {
                self.suspend_all();
                self.send_ack(HotkeyOp::Suspend, true, false, None);
            }
            HotkeyThreadCommand::Resume => match self.resume() {
                Ok(fallback_active) => {
                    self.send_ack(HotkeyOp::Resume, true, fallback_active, None);
                }
                Err(e) => {
                    self.send_ack(HotkeyOp::Resume, false, false, Some(e.to_string()));
                }
            },
        }
    }

    /// Runs the message loop to process hotkey events.
    ///
    /// This method blocks until the message loop is terminated (e.g. by `WM_QUIT`).
    /// `queue` carries in-place rebind/suspend/resume commands, woken by
    /// [`WM_APP_HOTKEY_WAKE`] — recognizable among the loop's other messages
    /// because a thread-posted message always arrives with a null `hwnd`,
    /// unlike the window messages `GetMessageW` also returns here.
    pub fn run_message_loop(&mut self, queue: &HotkeyCommandQueue) {
        let mut msg = MSG::default();
        unsafe {
            // GetMessageW returns:
            // > 0: Message retrieved
            // 0: WM_QUIT received
            // -1: Error
            //
            // Both exit paths are logged: when this loop ends, the hotkey
            // thread dies and the app loses its primary input, so the exit
            // must never be silent. The main loop's liveness check detects
            // the dead thread and attempts a restart.
            loop {
                let ret = GetMessageW(&raw mut msg, None, 0, 0).0;
                if ret == 0 {
                    log::info!("Hotkey message loop received WM_QUIT, exiting");
                    break;
                }
                if ret == -1 {
                    log::error!(
                        error:% = last_error_as_brightness_error("GetMessageW");
                        "Hotkey message loop failed, exiting"
                    );
                    break;
                }
                if msg.message == WM_HOTKEY {
                    // WPARAM for WM_HOTKEY is the identifier of the hotkey; the
                    // cast cannot lose anything because we only ever register
                    // small positive IDs (1-4).
                    #[expect(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    let id = msg.wParam.0 as i32;
                    log::debug!(hotkey_id = id; "Received WM_HOTKEY");

                    let direction = match id {
                        BRIGHTNESS_UP_ID | BRIGHTNESS_UP_ALT_ID => 1,
                        BRIGHTNESS_DOWN_ID | BRIGHTNESS_DOWN_ALT_ID => -1,
                        _ => 0,
                    };

                    if direction != 0 {
                        log::debug!(direction = direction; "Sending brightness adjustment");
                        if let Err(e) = self
                            .sender
                            .send(BrightnessMessage::AdjustStep { direction })
                        {
                            log::error!(error:% = e; "Failed to send brightness adjustment");
                        }
                    }
                } else if msg.hwnd.is_invalid() && msg.message == WM_APP_HOTKEY_WAKE {
                    for command in drain_commands(queue) {
                        self.handle_command(command);
                    }
                }

                let _ = TranslateMessage(&raw const msg);
                let _ = DispatchMessageW(&raw const msg);
            }
        }
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        self.unregister_all();
        if !self.hwnd.is_invalid() {
            // SAFETY: this manager created the window and owns it, and the
            // hotkeys registered against it were just unregistered above.
            // `DestroyWindow` only works from the thread that created the
            // window, which holds here because the manager is created, used
            // and dropped entirely on the hotkey thread.
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}

/// A parsed hotkey consisting of modifiers and a virtual key code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParsedHotkey {
    /// Modifier keys (Ctrl, Alt, Shift, Win).
    pub modifiers: HOT_KEY_MODIFIERS,
    /// Virtual key code for the main key.
    pub vk_code: VIRTUAL_KEY,
}

impl ParsedHotkey {
    /// Creates a new parsed hotkey.
    #[must_use]
    pub const fn new(modifiers: HOT_KEY_MODIFIERS, vk_code: VIRTUAL_KEY) -> Self {
        Self { modifiers, vk_code }
    }
}

impl std::fmt::Display for ParsedHotkey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut parts = Vec::new();

        if self.modifiers.contains(MOD_CONTROL) {
            parts.push("Ctrl");
        }
        if self.modifiers.contains(MOD_ALT) {
            parts.push("Alt");
        }
        if self.modifiers.contains(MOD_SHIFT) {
            parts.push("Shift");
        }
        if self.modifiers.contains(MOD_WIN) {
            parts.push("Win");
        }

        // Find key name from VK code
        let key_name = VK_TO_NAME
            .iter()
            .find(|(_, vk)| *vk == self.vk_code)
            .map_or("Unknown", |(name, _)| name.as_str());

        parts.push(key_name);
        write!(f, "{}", parts.join("+"))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Hotkey Thread Entry Point
// ─────────────────────────────────────────────────────────────────────────────

/// Body of the dedicated hotkey thread: creates the manager, applies the
/// startup bindings, signals readiness, then runs the message loop until
/// `WM_QUIT` or a fatal `GetMessageW` error.
///
/// `up`/`down`/`intercept` are the bindings to start with — the startup
/// config on first spawn, or [`crate::core::controller::Controller::hotkey_config`]
/// on a supervised respawn, so a respawn after a live rebind re-registers
/// what is actually configured now rather than reverting to stale bindings.
///
/// `thread_id` is populated with `GetCurrentThreadId()` right before the
/// ready signal: `RegisterHotKey`/`WM_HOTKEY` delivery is tied to the
/// registering thread, so [`HotkeyPortImpl`] needs this thread's id to
/// address it, and must never see a non-zero id before this thread is
/// actually ready to receive `WM_APP_HOTKEY_WAKE`. It is reset to 0 when the
/// loop exits, so a post to a dead thread fails cleanly instead of silently
/// vanishing.
///
/// A registration failure here is fatal — reported once through `ready_tx`,
/// exactly like every other constructor/registration failure — and the
/// thread exits without ever running the loop.
// Every non-Copy parameter is owned rather than borrowed on purpose: this
// is the spawned thread's entry point (see `start_hotkey_thread` in `main.rs`),
// so the caller hands over the shared cells and the startup values outright
// instead of keeping borrows alive across the thread boundary, even though
// the body itself only ever reads through most of them afterward.
#[expect(clippy::needless_pass_by_value)]
pub fn run_hotkey_thread(
    up: String,
    down: String,
    intercept: bool,
    tx: Sender<BrightnessMessage>,
    thread_id: Arc<AtomicU32>,
    queue: HotkeyCommandQueue,
    ready_tx: Sender<Result<()>>,
) {
    let up_hotkey = match parse_hotkey(&up) {
        Ok(h) => h,
        Err(e) => {
            let _ = ready_tx.send(Err(BrightnessError::config_invalid(
                "hotkeys.brightness_up",
                e.to_string(),
            )));
            return;
        }
    };
    let down_hotkey = match parse_hotkey(&down) {
        Ok(h) => h,
        Err(e) => {
            let _ = ready_tx.send(Err(BrightnessError::config_invalid(
                "hotkeys.brightness_down",
                e.to_string(),
            )));
            return;
        }
    };

    let mut manager = match HotkeyManager::new(tx) {
        Ok(m) => m,
        Err(e) => {
            let _ = ready_tx.send(Err(e));
            return;
        }
    };

    if let Err(e) = manager.apply_bindings(up_hotkey, down_hotkey, intercept) {
        let _ = ready_tx.send(Err(e));
        return;
    }

    // SAFETY: no preconditions; this reads the calling thread's own id.
    thread_id.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);

    let _ = ready_tx.send(Ok(()));

    manager.run_message_loop(&queue);

    thread_id.store(0, Ordering::SeqCst);
}

// ─────────────────────────────────────────────────────────────────────────────
// HotkeyPort Seam
// ─────────────────────────────────────────────────────────────────────────────

/// [`HotkeyPort`] backed by the real hotkey thread: posts a command to
/// [`HotkeyCommandQueue`] and wakes the thread's message loop with
/// `PostThreadMessageW(WM_APP_HOTKEY_WAKE)`. The result of the operation
/// itself always arrives later, asynchronously, as
/// `BrightnessMessage::HotkeyRebindResult` — a successful post here only
/// means the command was queued and the thread was woken.
pub struct HotkeyPortImpl {
    /// The hotkey thread's id, or 0 before it is ready / after it exits.
    /// Shared with [`run_hotkey_thread`], which is the only writer.
    thread_id: Arc<AtomicU32>,
    /// Shared with the hotkey thread's message loop, which is the only
    /// consumer (drains it after waking on `WM_APP_HOTKEY_WAKE`).
    queue: HotkeyCommandQueue,
}

impl HotkeyPortImpl {
    /// Creates a port over the given thread-id cell and command queue. Both
    /// are constructed once at startup and shared with every hotkey thread
    /// spawn (initial and every supervised respawn), so a rebind posted
    /// mid-respawn ordinarily either reaches the new thread or fails cleanly
    /// rather than talking to a queue nobody drains anymore — except for a
    /// spawn the main thread gave up waiting on and abandoned, which can
    /// still finish registering later and publish into these same cells.
    #[must_use]
    pub const fn new(thread_id: Arc<AtomicU32>, queue: HotkeyCommandQueue) -> Self {
        Self { thread_id, queue }
    }

    /// Queues `command` and wakes the hotkey thread.
    ///
    /// If the wake itself fails, `command` is removed from the queue again
    /// before returning: nobody is going to drain it now, and leaving it
    /// behind would let a later, unrelated wake (or a supervised respawn
    /// sharing this same queue) apply a command the caller has already
    /// treated as failed — diverging silently from the config the caller
    /// reverted to.
    ///
    /// # Errors
    ///
    /// Returns `BrightnessError::ChannelSend` if no thread is currently
    /// ready (id is 0), and `BrightnessError::WindowsApi` if
    /// `PostThreadMessageW` itself fails (e.g. the thread died between the id
    /// check and the post). A poisoned queue lock is recovered rather than
    /// reported: the queue is plain data a panicking thread cannot leave torn.
    fn post(&mut self, command: HotkeyThreadCommand) -> Result<()> {
        let tid = self.thread_id.load(Ordering::SeqCst);
        if tid == 0 {
            return Err(BrightnessError::ChannelSend);
        }

        self.queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(command);

        let wake = unsafe { PostThreadMessageW(tid, WM_APP_HOTKEY_WAKE, WPARAM(0), LPARAM(0)) };
        if let Err(e) = wake {
            // The main thread is the only producer, so the tail is exactly
            // the entry just pushed above; if something already drained the
            // queue in between, this is a harmless no-op.
            self.queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .pop_back();
            return Err(BrightnessError::windows_api(
                "PostThreadMessageW",
                e.code().0.cast_unsigned(),
            ));
        }
        Ok(())
    }
}

impl HotkeyPort for HotkeyPortImpl {
    fn rebind(&mut self, up: &str, down: &str, intercept: bool) -> Result<()> {
        self.post(HotkeyThreadCommand::Rebind {
            up: up.to_string(),
            down: down.to_string(),
            intercept,
        })
    }

    fn suspend(&mut self) -> Result<()> {
        self.post(HotkeyThreadCommand::Suspend)
    }

    fn resume(&mut self) -> Result<()> {
        self.post(HotkeyThreadCommand::Resume)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Key Mappings
// ─────────────────────────────────────────────────────────────────────────────

/// Mapping from modifier names (lowercase) to their `HOT_KEY_MODIFIERS` values.
static MODIFIER_MAP: LazyLock<HashMap<&'static str, HOT_KEY_MODIFIERS>> = LazyLock::new(|| {
    let mut m = HashMap::new();
    m.insert("ctrl", MOD_CONTROL);
    m.insert("control", MOD_CONTROL);
    m.insert("alt", MOD_ALT);
    m.insert("shift", MOD_SHIFT);
    m.insert("win", MOD_WIN);
    m.insert("windows", MOD_WIN);
    m.insert("super", MOD_WIN);
    m
});

/// Mapping from key names (lowercase) to their virtual key codes.
static KEY_MAP: LazyLock<HashMap<&'static str, VIRTUAL_KEY>> = LazyLock::new(|| {
    let mut m = HashMap::new();

    // Arrow keys
    m.insert("up", VK_UP);
    m.insert("down", VK_DOWN);
    m.insert("left", VK_LEFT);
    m.insert("right", VK_RIGHT);

    // Function keys
    m.insert("f1", VK_F1);
    m.insert("f2", VK_F2);
    m.insert("f3", VK_F3);
    m.insert("f4", VK_F4);
    m.insert("f5", VK_F5);
    m.insert("f6", VK_F6);
    m.insert("f7", VK_F7);
    m.insert("f8", VK_F8);
    m.insert("f9", VK_F9);
    m.insert("f10", VK_F10);
    m.insert("f11", VK_F11);
    m.insert("f12", VK_F12);

    // Navigation keys
    m.insert("pageup", VK_PRIOR);
    m.insert("pagedown", VK_NEXT);
    m.insert("home", VK_HOME);
    m.insert("end", VK_END);
    m.insert("insert", VK_INSERT);
    m.insert("delete", VK_DELETE);
    m.insert("del", VK_DELETE);

    // Common keys
    m.insert("space", VK_SPACE);
    m.insert("tab", VK_TAB);
    m.insert("enter", VK_RETURN);
    m.insert("return", VK_RETURN);
    m.insert("escape", VK_ESCAPE);
    m.insert("esc", VK_ESCAPE);
    m.insert("backspace", VK_BACK);

    // Symbols
    m.insert("plus", VK_OEM_PLUS);
    m.insert("minus", VK_OEM_MINUS);

    // Note: Leak is acceptable as this is done once in LazyLock for the lifetime of the process.
    // Letters A-Z (VK codes are same as ASCII uppercase)
    for c in 'a'..='z' {
        let key_name: &'static str = Box::leak(c.to_string().into_boxed_str());
        m.insert(key_name, VIRTUAL_KEY(c.to_ascii_uppercase() as u16));
    }

    // Numbers 0-9 (VK codes are same as ASCII)
    for c in '0'..='9' {
        let key_name: &'static str = Box::leak(c.to_string().into_boxed_str());
        m.insert(key_name, VIRTUAL_KEY(c as u16));
    }

    m
});

/// Reverse mapping from VK codes to display names (for `Display` impl and
/// [`key_name`]).
///
/// Owned `String`s (rather than `&'static str`) because letters and digits
/// are generated in a loop instead of written as literals.
static VK_TO_NAME: LazyLock<Vec<(String, VIRTUAL_KEY)>> = LazyLock::new(|| {
    let mut v = vec![
        ("Up".to_string(), VK_UP),
        ("Down".to_string(), VK_DOWN),
        ("Left".to_string(), VK_LEFT),
        ("Right".to_string(), VK_RIGHT),
        ("F1".to_string(), VK_F1),
        ("F2".to_string(), VK_F2),
        ("F3".to_string(), VK_F3),
        ("F4".to_string(), VK_F4),
        ("F5".to_string(), VK_F5),
        ("F6".to_string(), VK_F6),
        ("F7".to_string(), VK_F7),
        ("F8".to_string(), VK_F8),
        ("F9".to_string(), VK_F9),
        ("F10".to_string(), VK_F10),
        ("F11".to_string(), VK_F11),
        ("F12".to_string(), VK_F12),
        ("PageUp".to_string(), VK_PRIOR),
        ("PageDown".to_string(), VK_NEXT),
        ("Home".to_string(), VK_HOME),
        ("End".to_string(), VK_END),
        ("Insert".to_string(), VK_INSERT),
        ("Delete".to_string(), VK_DELETE),
        ("Space".to_string(), VK_SPACE),
        ("Tab".to_string(), VK_TAB),
        ("Enter".to_string(), VK_RETURN),
        ("Escape".to_string(), VK_ESCAPE),
        ("Backspace".to_string(), VK_BACK),
        ("Plus".to_string(), VK_OEM_PLUS),
        ("Minus".to_string(), VK_OEM_MINUS),
    ];

    // Letters A-Z and digits 0-9 (VK codes match ASCII uppercase / ASCII).
    for c in 'A'..='Z' {
        let code = u16::try_from(u32::from(c)).expect("ASCII letter code always fits in a u16");
        v.push((c.to_string(), VIRTUAL_KEY(code)));
    }
    for c in '0'..='9' {
        let code = u16::try_from(u32::from(c)).expect("ASCII digit code always fits in a u16");
        v.push((c.to_string(), VIRTUAL_KEY(code)));
    }

    v
});

// ─────────────────────────────────────────────────────────────────────────────
// Parsing
// ─────────────────────────────────────────────────────────────────────────────

/// Parses a hotkey string into modifiers and a virtual key code.
///
/// # Format
///
/// Hotkey strings are `+`-delimited, case-insensitive combinations of
/// modifiers and a key name.
///
/// ## Modifiers
/// - `Ctrl` or `Control`
/// - `Alt`
/// - `Shift`
/// - `Win`, `Windows`, or `Super`
///
/// ## Keys
/// - Arrow keys: `Up`, `Down`, `Left`, `Right`
/// - Function keys: `F1` - `F12`
/// - Navigation: `PageUp`, `PageDown`, `Home`, `End`, `Insert`, `Delete`
/// - Common: `Space`, `Tab`, `Enter`, `Escape`, `Backspace`
/// - Symbols: `Plus` (for `+`), `Minus` (for `-`)
/// - Letters: `A` - `Z`
/// - Numbers: `0` - `9`
///
/// # Examples
///
/// ```
/// use darkbright_helper::platform::windows::hotkey::parse_hotkey;
///
/// let hotkey = parse_hotkey("Ctrl+Shift+Up").unwrap();
/// let hotkey2 = parse_hotkey("alt+f1").unwrap(); // case insensitive
/// ```
///
/// # Errors
///
/// Returns `BrightnessError::ConfigInvalid` if:
/// - The string is empty or contains only modifiers.
/// - An unknown key name is encountered.
/// - No valid key (only modifiers) is specified.
pub fn parse_hotkey(s: &str) -> Result<ParsedHotkey> {
    let s = s.trim();

    if s.is_empty() {
        return Err(BrightnessError::config_invalid(
            "hotkey",
            "hotkey string is empty",
        ));
    }

    let mut modifiers = HOT_KEY_MODIFIERS::default();
    let mut vk_code: Option<VIRTUAL_KEY> = None;

    for part in s.split('+') {
        let part = part.trim().to_lowercase();

        if part.is_empty() {
            continue;
        }

        // Check if it's a modifier
        if let Some(&modifier) = MODIFIER_MAP.get(part.as_str()) {
            modifiers |= modifier;
            continue;
        }

        // Check if it's a key
        if let Some(&vk) = KEY_MAP.get(part.as_str()) {
            if vk_code.is_some() {
                return Err(BrightnessError::config_invalid(
                    "hotkey",
                    format!("multiple keys specified in '{s}', only one allowed"),
                ));
            }
            vk_code = Some(vk);
            continue;
        }

        // Unknown part
        return Err(BrightnessError::config_invalid(
            "hotkey",
            format!("unknown key or modifier: '{part}'"),
        ));
    }

    let vk_code = vk_code.ok_or_else(|| {
        BrightnessError::config_invalid("hotkey", format!("no key specified in '{s}'"))
    })?;

    Ok(ParsedHotkey::new(modifiers, vk_code))
}

// ─────────────────────────────────────────────────────────────────────────────
// Capture-field support
// ─────────────────────────────────────────────────────────────────────────────

/// Returns the human-readable name for `vk`, if `parse_hotkey` would accept it
/// as a key.
///
/// Used by the capture control to reject a pressed key it cannot represent
/// (and therefore cannot round-trip through `config.json`).
pub(crate) fn key_name(vk: VIRTUAL_KEY) -> Option<String> {
    VK_TO_NAME
        .iter()
        .find(|(_, candidate)| *candidate == vk)
        .map(|(name, _)| name.clone())
}

/// Formats `modifiers` and `vk` as a `"Ctrl+Shift+Up"`-style string, in the
/// same human-readable format `config.json` already stores hotkeys in.
///
/// Returns `None` if `vk` has no name, i.e. is not a key `parse_hotkey`
/// accepts.
#[must_use]
pub(crate) fn hotkey_string(modifiers: HOT_KEY_MODIFIERS, vk: VIRTUAL_KEY) -> Option<String> {
    key_name(vk)?;
    Some(ParsedHotkey::new(modifiers, vk).to_string())
}

/// True when `a` and `b` parse to the same modifiers and key, regardless of
/// case, modifier order, or alias spelling (e.g. `shift+ctrl+up` equals
/// `Control+Shift+Up`).
///
/// Input that fails to parse never conflicts with anything, so a hand-edited
/// but unparseable `config.json` entry stays as permissive as it is today.
#[must_use]
pub(crate) fn bindings_conflict(a: &str, b: &str) -> bool {
    let (Ok(a), Ok(b)) = (parse_hotkey(a), parse_hotkey(b)) else {
        return false;
    };
    a.modifiers == b.modifiers && a.vk_code == b.vk_code
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ctrl_shift_up() {
        let hotkey = parse_hotkey("Ctrl+Shift+Up").unwrap();
        assert!(hotkey.modifiers.contains(MOD_CONTROL));
        assert!(hotkey.modifiers.contains(MOD_SHIFT));
        assert!(!hotkey.modifiers.contains(MOD_ALT));
        assert_eq!(hotkey.vk_code, VK_UP);
    }

    #[test]
    fn test_parse_case_insensitive() {
        let hotkey = parse_hotkey("ctrl+shift+up").unwrap();
        assert!(hotkey.modifiers.contains(MOD_CONTROL));
        assert!(hotkey.modifiers.contains(MOD_SHIFT));
        assert_eq!(hotkey.vk_code, VK_UP);

        let hotkey2 = parse_hotkey("CTRL+SHIFT+UP").unwrap();
        assert_eq!(hotkey, hotkey2);
    }

    #[test]
    fn test_parse_alt_f1() {
        let hotkey = parse_hotkey("Alt+F1").unwrap();
        assert!(hotkey.modifiers.contains(MOD_ALT));
        assert!(!hotkey.modifiers.contains(MOD_CONTROL));
        assert_eq!(hotkey.vk_code, VK_F1);
    }

    #[test]
    fn test_parse_single_key() {
        let hotkey = parse_hotkey("F5").unwrap();
        assert_eq!(hotkey.modifiers, HOT_KEY_MODIFIERS::default());
        assert_eq!(hotkey.vk_code, VK_F5);
    }

    #[test]
    fn test_parse_with_spaces() {
        let hotkey = parse_hotkey("Ctrl + Shift + Down").unwrap();
        assert!(hotkey.modifiers.contains(MOD_CONTROL));
        assert!(hotkey.modifiers.contains(MOD_SHIFT));
        assert_eq!(hotkey.vk_code, VK_DOWN);
    }

    #[test]
    fn test_parse_letter() {
        let hotkey = parse_hotkey("Ctrl+A").unwrap();
        assert!(hotkey.modifiers.contains(MOD_CONTROL));
        assert_eq!(hotkey.vk_code, VIRTUAL_KEY(0x41)); // 'A'
    }

    #[test]
    fn test_parse_number() {
        let hotkey = parse_hotkey("Alt+5").unwrap();
        assert!(hotkey.modifiers.contains(MOD_ALT));
        assert_eq!(hotkey.vk_code, VIRTUAL_KEY(0x35)); // '5'
    }

    #[test]
    fn test_parse_empty_fails() {
        assert!(parse_hotkey("").is_err());
    }

    #[test]
    fn test_parse_only_modifiers_fails() {
        assert!(parse_hotkey("Ctrl+Shift").is_err());
    }

    #[test]
    fn test_parse_unknown_key_fails() {
        assert!(parse_hotkey("Ctrl+UnknownKey").is_err());
    }

    #[test]
    fn test_parse_multiple_keys_fails() {
        assert!(parse_hotkey("Ctrl+A+B").is_err());
    }

    #[test]
    fn test_display() {
        let hotkey = parse_hotkey("Ctrl+Shift+Up").unwrap();
        let display = hotkey.to_string();
        assert!(display.contains("Ctrl"));
        assert!(display.contains("Shift"));
        assert!(display.contains("Up"));
    }

    #[test]
    fn test_parse_win_modifier() {
        let hotkey = parse_hotkey("Win+E").unwrap();
        assert!(hotkey.modifiers.contains(MOD_WIN));
    }

    #[test]
    fn test_parse_plus_key() {
        let hotkey = parse_hotkey("Ctrl+Plus").unwrap();
        assert_eq!(hotkey.vk_code, VK_OEM_PLUS);
    }

    #[test]
    fn every_accepted_key_round_trips_through_display() {
        for name in KEY_MAP.keys() {
            let s = format!("Ctrl+{name}");
            let parsed = parse_hotkey(&s).unwrap_or_else(|_| panic!("parse {s}"));
            let shown = parsed.to_string();
            let reparsed =
                parse_hotkey(&shown).unwrap_or_else(|_| panic!("re-parse {shown} (from {s})"));
            assert_eq!(parsed.vk_code, reparsed.vk_code, "{s} -> {shown}");
            assert_eq!(parsed.modifiers, reparsed.modifiers);
        }
    }

    #[test]
    fn conflict_is_canonical_not_textual() {
        assert!(bindings_conflict("Ctrl+Shift+Up", "shift+control+up"));
        assert!(bindings_conflict("Ctrl+B", "ctrl+b"));
        assert!(!bindings_conflict("Ctrl+Shift+Up", "Ctrl+Shift+Down"));
        assert!(!bindings_conflict("garbage", "Ctrl+Shift+Up"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // In-place rebind machinery: pure/host-testable parts only. Anything
    // that needs a live message loop (apply_bindings, suspend/resume against
    // a real HotkeyManager) is exercised manually — see the task report.
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn wake_message_id_does_not_collide_with_wm_hotkey() {
        assert_ne!(WM_APP_HOTKEY_WAKE, WM_HOTKEY);
    }

    #[test]
    fn drain_commands_removes_everything_in_fifo_order() {
        let queue: HotkeyCommandQueue = Arc::new(Mutex::new(VecDeque::new()));
        queue
            .lock()
            .unwrap()
            .push_back(HotkeyThreadCommand::Suspend);
        queue
            .lock()
            .unwrap()
            .push_back(HotkeyThreadCommand::Rebind {
                up: "Ctrl+Up".to_string(),
                down: "Ctrl+Down".to_string(),
                intercept: true,
            });
        queue.lock().unwrap().push_back(HotkeyThreadCommand::Resume);

        let drained = drain_commands(&queue);

        assert_eq!(drained.len(), 3);
        assert!(matches!(drained[0], HotkeyThreadCommand::Suspend));
        assert!(matches!(drained[1], HotkeyThreadCommand::Rebind { .. }));
        assert!(matches!(drained[2], HotkeyThreadCommand::Resume));
        assert!(
            queue.lock().unwrap().is_empty(),
            "drain must remove everything it returns"
        );
    }

    #[test]
    fn drain_commands_on_empty_queue_returns_empty_vec() {
        let queue: HotkeyCommandQueue = Arc::new(Mutex::new(VecDeque::new()));
        assert!(drain_commands(&queue).is_empty());
    }

    #[test]
    fn hotkey_port_post_fails_when_no_thread_is_ready() {
        // thread_id == 0 means "no thread ready" (startup not finished, or
        // the thread has already exited); post() must reject the command
        // without touching the queue or calling PostThreadMessageW at all.
        let thread_id = Arc::new(AtomicU32::new(0));
        let queue: HotkeyCommandQueue = Arc::new(Mutex::new(VecDeque::new()));
        let mut port = HotkeyPortImpl::new(thread_id, queue.clone());

        assert!(port.suspend().is_err());
        assert!(
            queue.lock().unwrap().is_empty(),
            "a rejected post must not leave a command behind for a later thread to pick up"
        );
    }

    #[test]
    fn hotkey_port_post_fails_for_a_thread_id_that_does_not_exist() {
        // A thread id that was never issued by the OS: PostThreadMessageW
        // must fail with ERROR_INVALID_THREAD_ID, and post() must surface
        // that as an Err rather than silently succeeding — and must not
        // strand the command in the queue for some later wake to pick up.
        let thread_id = Arc::new(AtomicU32::new(u32::MAX));
        let queue: HotkeyCommandQueue = Arc::new(Mutex::new(VecDeque::new()));
        let mut port = HotkeyPortImpl::new(thread_id, queue.clone());

        assert!(port.resume().is_err());
        assert!(
            queue.lock().unwrap().is_empty(),
            "a post whose wake failed must not leave a command behind for a later thread to pick up"
        );
    }

    /// Poisons a queue by panicking while its guard is held.
    fn poisoned_queue() -> HotkeyCommandQueue {
        let queue: HotkeyCommandQueue = Arc::new(Mutex::new(VecDeque::new()));
        let victim = Arc::clone(&queue);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = victim.lock().unwrap();
            panic!("poison the queue");
        }));
        assert!(queue.is_poisoned(), "setup must actually poison the mutex");
        queue
    }

    #[test]
    fn post_on_a_poisoned_queue_still_enqueues() {
        // The queue is plain data, so poisoning is recovered rather than
        // reported: a panic elsewhere must not silently stop rebinds from
        // reaching the hotkey thread. The wake still fails on the bogus tid,
        // which is what rolls the command back out again.
        let queue = poisoned_queue();
        let thread_id = Arc::new(AtomicU32::new(u32::MAX));
        let mut port = HotkeyPortImpl::new(thread_id, Arc::clone(&queue));

        assert!(
            port.resume().is_err(),
            "the bogus thread id must still surface the wake failure"
        );
        assert!(
            !matches!(port.resume(), Err(BrightnessError::ChannelSend)),
            "poisoning must not be reported as a closed channel"
        );
    }

    #[test]
    fn drain_commands_on_a_poisoned_queue_still_drains() {
        let queue = poisoned_queue();
        queue
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push_back(HotkeyThreadCommand::Resume);

        let drained = drain_commands(&queue);

        assert_eq!(drained.len(), 1, "a poisoned queue must still drain");
    }
}
