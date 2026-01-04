//! System tray icon and menu implementation for Windows.
//!
//! This module provides a system tray icon with a context menu that allows users to:
//! - View current brightness/overlay levels for all monitors
//! - Open the settings (config.json) file
//! - Quit the application
//!
//! The tray icon runs its own message loop on a dedicated thread and communicates
//! with the main thread via `BrightnessMessage` channels.

use std::cell::RefCell;
use std::sync::mpsc::{self, Sender};
use std::sync::OnceLock;
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NOTIFYICONDATAW,
    NOTIFY_ICON_DATA_FLAGS, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu, DispatchMessageW,
    GetCursorPos, GetMenuItemRect, GetMessageW, HICON, HMENU, HWND_MESSAGE, IMAGE_ICON,
    LR_DEFAULTSIZE, LR_LOADFROMFILE, LR_SHARED, LoadImageW, MF_GRAYED, MF_SEPARATOR, MF_STRING,
    MSG, PostMessageW, PostQuitMessage, RegisterClassExW, SendMessageW, SetForegroundWindow,
    TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WM_DESTROY, WM_EXITMENULOOP, WM_MENUSELECT, WM_NULL, WM_USER, WNDCLASSEXW,
    WS_EX_TOPMOST, WS_OVERLAPPED, WS_POPUP,
};
use windows::Win32::Foundation::RECT;
use windows::core::{PCWSTR, w};

use crate::core::state::{BrightnessMessage, TrayMenuData};
use crate::error::{BrightnessError, Result};

use super::{SafeHwnd, last_error_as_brightness_error};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

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

/// Tooltip text shown when hovering over the tray icon.
const TRAY_TOOLTIP: &str = "Brightness Control";

/// Resource ID for the embedded application icon.
/// Must match the ID defined in build.rs when embedding resources.
const IDI_APP_ICON: u16 = 1;

/// Application name displayed in the tray menu.
const APP_NAME: &str = "Brightness Control";

/// Timeout for waiting for menu data from the main thread.
const MENU_DATA_TIMEOUT: Duration = Duration::from_millis(500);

// ─────────────────────────────────────────────────────────────────────────────
// Tooltip Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Tooltip window class name.
const TOOLTIPS_CLASS: &str = "tooltips_class32";

/// Tooltip style: Always show tip even if window is not active.
const TTS_ALWAYSTIP: u32 = 0x01;

/// Tooltip style: Prevent prefix characters from being stripped.
const TTS_NOPREFIX: u32 = 0x02;

/// Tooltip message: Set maximum width (enables multi-line).
const TTM_SETMAXTIPWIDTH: u32 = WM_USER + 24;

/// Tooltip message: Activate/deactivate tracking tooltip.
const TTM_TRACKACTIVATE: u32 = WM_USER + 17;

/// Tooltip message: Set position of tracking tooltip.
const TTM_TRACKPOSITION: u32 = WM_USER + 18;

/// Tooltip message: Add a tool (Unicode).
const TTM_ADDTOOLW: u32 = WM_USER + 50;

/// Tooltip message: Update tooltip text (Unicode).
const TTM_UPDATETIPTEXTW: u32 = WM_USER + 57;

/// Tool flag: Tooltip is positioned by TTM_TRACKPOSITION.
const TTF_TRACK: u32 = 0x0020;

/// Tool flag: Position is absolute screen coordinates.
const TTF_ABSOLUTE: u32 = 0x0080;

/// Tool flag: uId is an HWND.
const TTF_IDISHWND: u32 = 0x0001;

/// Maximum tooltip width in pixels.
const TOOLTIP_MAX_WIDTH: i32 = 350;

// ─────────────────────────────────────────────────────────────────────────────
// Thread-Local Sender Storage
// ─────────────────────────────────────────────────────────────────────────────

thread_local! {
    /// Thread-local storage for the message sender.
    /// This allows the window procedure to send messages to the main thread.
    static TRAY_SENDER: RefCell<Option<Sender<BrightnessMessage>>> = const { RefCell::new(None) };

    /// Thread-local storage for the tooltip window handle.
    static TRAY_TOOLTIP_HWND: RefCell<Option<HWND>> = const { RefCell::new(None) };

    /// Thread-local storage for the tooltip text (built from hotkeys).
    static TRAY_TOOLTIP_TEXT: RefCell<String> = const { RefCell::new(String::new()) };

    /// Thread-local storage for the currently active menu handle.
    static TRAY_ACTIVE_MENU: RefCell<Option<HMENU>> = const { RefCell::new(None) };
}

/// Sets the thread-local sender for the tray icon callbacks.
fn set_tray_sender(sender: Sender<BrightnessMessage>) {
    TRAY_SENDER.with(|s| {
        *s.borrow_mut() = Some(sender);
    });
}

/// Executes a closure with access to the thread-local sender.
fn with_tray_sender<R>(f: impl FnOnce(&Sender<BrightnessMessage>) -> R) -> Option<R> {
    TRAY_SENDER.with(|s| s.borrow().as_ref().map(f))
}

/// Requests menu data from the main thread.
///
/// Sends a `TrayMenuOpening` message and waits for the response with a timeout.
/// Returns `None` if the request times out or fails.
fn request_menu_data() -> Option<TrayMenuData> {
    let (reply_tx, reply_rx) = mpsc::channel();

    let sent = with_tray_sender(|sender| {
        sender
            .send(BrightnessMessage::TrayMenuOpening { reply_tx })
            .is_ok()
    })
    .unwrap_or(false);

    if !sent {
        log::warn!("Failed to send TrayMenuOpening message");
        return None;
    }

    match reply_rx.recv_timeout(MENU_DATA_TIMEOUT) {
        Ok(data) => Some(data),
        Err(e) => {
            log::warn!("Failed to receive menu data: {e}");
            None
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Tooltip Support
// ─────────────────────────────────────────────────────────────────────────────

/// TOOLINFOW structure for tooltip operations.
/// Using raw struct since windows crate may not expose it directly.
#[repr(C)]
#[allow(non_snake_case)]
struct TOOLINFOW {
    cbSize: u32,
    uFlags: u32,
    hwnd: HWND,
    uId: usize,
    rect: RECT,
    hinst: windows::Win32::Foundation::HINSTANCE,
    lpszText: *mut u16,
    lParam: LPARAM,
    lpReserved: *mut std::ffi::c_void,
}

/// Creates a tracking tooltip window.
///
/// # Arguments
///
/// * `parent` - Parent window handle for the tooltip.
///
/// # Returns
///
/// The tooltip window handle, or `None` if creation failed.
fn create_tooltip(parent: HWND) -> Option<HWND> {
    let class_wide: Vec<u16> = TOOLTIPS_CLASS.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST,
            PCWSTR(class_wide.as_ptr()),
            PCWSTR::null(),
            WS_POPUP | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(TTS_ALWAYSTIP | TTS_NOPREFIX),
            0, 0, 0, 0,
            parent,
            None,
            GetModuleHandleW(None).ok()?,
            None,
        );

        if hwnd.0 == 0 {
            log::warn!("Failed to create tooltip window");
            return None;
        }

        // Set maximum width to enable multi-line tooltips
        SendMessageW(hwnd, TTM_SETMAXTIPWIDTH, WPARAM(0), LPARAM(TOOLTIP_MAX_WIDTH as isize));

        // Add a tool for the parent window
        let mut ti = TOOLINFOW {
            cbSize: u32::try_from(std::mem::size_of::<TOOLINFOW>()).unwrap_or(0),
            uFlags: TTF_IDISHWND | TTF_TRACK | TTF_ABSOLUTE,
            hwnd: parent,
            uId: parent.0 as usize,
            rect: RECT::default(),
            hinst: windows::Win32::Foundation::HINSTANCE::default(),
            lpszText: std::ptr::null_mut(),
            lParam: LPARAM(0),
            lpReserved: std::ptr::null_mut(),
        };

        SendMessageW(
            hwnd,
            TTM_ADDTOOLW,
            WPARAM(0),
            LPARAM(&raw mut ti as isize),
        );

        log::debug!("Tooltip window created");
        Some(hwnd)
    }
}

/// Stores the tooltip text (built from hotkeys) for later use.
fn set_tooltip_text(hotkey_up: &str, hotkey_down: &str) {
    let text = format!(
        "1. Move mouse to desired monitor\n2. Press {} (brighter) or {} (dimmer)",
        hotkey_up, hotkey_down
    );
    TRAY_TOOLTIP_TEXT.with(|t| {
        *t.borrow_mut() = text;
    });
}

/// Shows the tooltip at the specified screen position.
///
/// # Arguments
///
/// * `tooltip_hwnd` - The tooltip window handle.
/// * `parent` - Parent window handle.
/// * `x`, `y` - Screen coordinates to position the tooltip.
fn show_tooltip_at(tooltip_hwnd: HWND, parent: HWND, x: i32, y: i32) {
    TRAY_TOOLTIP_TEXT.with(|text_cell| {
        let text = text_cell.borrow();
        if text.is_empty() {
            return;
        }

        let mut text_wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();

        unsafe {
            // Update the tooltip text
            let mut ti = TOOLINFOW {
                cbSize: u32::try_from(std::mem::size_of::<TOOLINFOW>()).unwrap_or(0),
                uFlags: TTF_IDISHWND | TTF_TRACK | TTF_ABSOLUTE,
                hwnd: parent,
                uId: parent.0 as usize,
                rect: RECT::default(),
                hinst: windows::Win32::Foundation::HINSTANCE::default(),
                lpszText: text_wide.as_mut_ptr(),
                lParam: LPARAM(0),
                lpReserved: std::ptr::null_mut(),
            };

            SendMessageW(
                tooltip_hwnd,
                TTM_UPDATETIPTEXTW,
                WPARAM(0),
                LPARAM(&raw mut ti as isize),
            );

            // Position the tooltip
            let coords = ((y as u32) << 16) | (x as u32 & 0xFFFF);
            SendMessageW(
                tooltip_hwnd,
                TTM_TRACKPOSITION,
                WPARAM(0),
                LPARAM(coords as isize),
            );

            // Activate tracking
            ti.lpszText = std::ptr::null_mut(); // Not needed for activate
            SendMessageW(
                tooltip_hwnd,
                TTM_TRACKACTIVATE,
                WPARAM(1), // TRUE = activate
                LPARAM(&raw mut ti as isize),
            );
        }
    });
}

/// Hides the tracking tooltip.
///
/// # Arguments
///
/// * `tooltip_hwnd` - The tooltip window handle.
/// * `parent` - Parent window handle.
fn hide_tooltip(tooltip_hwnd: HWND, parent: HWND) {
    unsafe {
        let ti = TOOLINFOW {
            cbSize: u32::try_from(std::mem::size_of::<TOOLINFOW>()).unwrap_or(0),
            uFlags: TTF_IDISHWND | TTF_TRACK | TTF_ABSOLUTE,
            hwnd: parent,
            uId: parent.0 as usize,
            rect: RECT::default(),
            hinst: windows::Win32::Foundation::HINSTANCE::default(),
            lpszText: std::ptr::null_mut(),
            lParam: LPARAM(0),
            lpReserved: std::ptr::null_mut(),
        };

        SendMessageW(
            tooltip_hwnd,
            TTM_TRACKACTIVATE,
            WPARAM(0), // FALSE = deactivate
            LPARAM(&raw const ti as isize),
        );
    }
}

/// Handles menu item hover events for tooltip display.
///
/// # Arguments
///
/// * `hwnd` - The tray window handle.
/// * `wparam` - Contains menu item index and flags.
fn handle_menu_select(hwnd: HWND, wparam: WPARAM) {
    // Extract item index and flags from wparam
    #[allow(clippy::cast_possible_truncation)]
    let item_index = (wparam.0 & 0xFFFF) as u16;
    #[allow(clippy::cast_possible_truncation)]
    let flags = ((wparam.0 >> 16) & 0xFFFF) as u16;

    // MF_MOUSESELECT flag indicates mouse hover
    const MF_HILITE: u16 = 0x0080;
    const MF_POPUP: u16 = 0x0010;

    // Check if this is a valid item selection (not popup, not closed)
    let is_valid_selection = (flags & MF_HILITE) != 0 && (flags & MF_POPUP) == 0 && flags != 0xFFFF;

    // Get the actual menu item ID by checking if it's in the monitor range
    let menu_id = item_index as u32;
    let is_monitor = menu_id >= MENU_ID_MONITOR_BASE && menu_id < MENU_ID_MONITOR_BASE + 100;

    TRAY_TOOLTIP_HWND.with(|tooltip_cell| {
        let tooltip_opt = tooltip_cell.borrow();
        let Some(tooltip_hwnd) = *tooltip_opt else {
            return;
        };

        if is_monitor && is_valid_selection {
            // Show tooltip near the menu item
            TRAY_ACTIVE_MENU.with(|menu_cell| {
                let menu_opt = menu_cell.borrow();
                let Some(hmenu) = *menu_opt else {
                    return;
                };

                // Get menu item rectangle
                let mut rect = RECT::default();
                unsafe {
                    if GetMenuItemRect(hwnd, hmenu, item_index as u32, &raw mut rect).is_ok() {
                        // Position tooltip to the right of the menu item
                        show_tooltip_at(tooltip_hwnd, hwnd, rect.right + 5, rect.top);
                    }
                }
            });
        } else {
            // Hide tooltip when not hovering a monitor item
            hide_tooltip(tooltip_hwnd, hwnd);
        }
    });
}

/// Cleans up tooltip when menu closes.
fn handle_menu_exit(hwnd: HWND) {
    TRAY_TOOLTIP_HWND.with(|tooltip_cell| {
        let tooltip_opt = tooltip_cell.borrow();
        if let Some(tooltip_hwnd) = *tooltip_opt {
            hide_tooltip(tooltip_hwnd, hwnd);
        }
    });

    TRAY_ACTIVE_MENU.with(|menu_cell| {
        *menu_cell.borrow_mut() = None;
    });
}

// ─────────────────────────────────────────────────────────────────────────────
// Icon Loading
// ─────────────────────────────────────────────────────────────────────────────

/// Loads the application icon for the tray.
///
/// Tries to load from embedded resource first (for release builds),
/// then falls back to loading from file (for development).
///
/// # Errors
///
/// Returns `BrightnessError::TrayIconCreation` if the icon cannot be loaded
/// from either source.
fn load_tray_icon() -> Result<HICON> {
    // Try loading from embedded resource first
    if let Ok(icon) = load_icon_from_resource() {
        log::debug!("Loaded tray icon from embedded resource");
        return Ok(icon);
    }

    // Fall back to loading from file (development mode)
    if let Ok(icon) = load_icon_from_file() {
        log::debug!("Loaded tray icon from file");
        return Ok(icon);
    }

    Err(BrightnessError::tray_icon_creation(
        "Failed to load icon from resource or file",
    ))
}

/// Attempts to load the icon from embedded Windows resource.
fn load_icon_from_resource() -> Result<HICON> {
    unsafe {
        let hinstance = GetModuleHandleW(None).map_err(|e| {
            BrightnessError::tray_icon_creation(format!("GetModuleHandleW failed: {}", e.code().0))
        })?;

        // Load icon from resource by ID
        let handle = LoadImageW(
            hinstance,
            PCWSTR(IDI_APP_ICON as *const u16),
            IMAGE_ICON,
            0, // Use default width
            0, // Use default height
            LR_DEFAULTSIZE | LR_SHARED,
        )
        .map_err(|e| {
            BrightnessError::tray_icon_creation(format!("LoadImageW (resource) failed: {}", e))
        })?;

        Ok(HICON(handle.0))
    }
}

/// Attempts to load the icon from the res/icon.ico file.
fn load_icon_from_file() -> Result<HICON> {
    use std::path::PathBuf;

    // Try to find the icon file relative to the executable
    let icon_path = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .map(|dir| dir.join("res").join("icon.ico"))
        .or_else(|| Some(PathBuf::from("res/icon.ico")))
        .unwrap();

    // Convert path to wide string
    let path_str = icon_path.to_string_lossy();
    let wide_path: Vec<u16> = path_str.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let handle = LoadImageW(
            None,
            PCWSTR(wide_path.as_ptr()),
            IMAGE_ICON,
            0,
            0,
            LR_LOADFROMFILE | LR_DEFAULTSIZE,
        )
        .map_err(|e| {
            BrightnessError::tray_icon_creation(format!(
                "LoadImageW (file '{}') failed: {}",
                path_str, e
            ))
        })?;

        Ok(HICON(handle.0))
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shell Notification Icon
// ─────────────────────────────────────────────────────────────────────────────

/// Creates the `NOTIFYICONDATAW` structure for tray icon operations.
///
/// # Arguments
///
/// * `hwnd` - Window handle for receiving tray messages.
/// * `icon` - Icon handle to display in the tray.
/// * `flags` - Which fields are valid in the structure.
fn create_notify_icon_data(hwnd: HWND, icon: HICON, flags: NOTIFY_ICON_DATA_FLAGS) -> NOTIFYICONDATAW {
    let mut nid = NOTIFYICONDATAW {
        cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(0),
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        uFlags: flags,
        uCallbackMessage: WM_TRAY_CALLBACK,
        hIcon: icon,
        ..Default::default()
    };

    // Set tooltip text (szTip is a fixed-size array)
    let tooltip_wide: Vec<u16> = TRAY_TOOLTIP.encode_utf16().collect();
    let copy_len = tooltip_wide.len().min(nid.szTip.len() - 1);
    nid.szTip[..copy_len].copy_from_slice(&tooltip_wide[..copy_len]);
    // Null terminator is already set by Default

    nid
}

/// Adds the tray icon to the system notification area.
///
/// # Arguments
///
/// * `hwnd` - Window handle for receiving tray messages.
/// * `icon` - Icon handle to display.
///
/// # Errors
///
/// Returns `BrightnessError::TrayIconCreation` if `Shell_NotifyIconW` fails.
fn add_tray_icon(hwnd: HWND, icon: HICON) -> Result<()> {
    let nid = create_notify_icon_data(
        hwnd,
        icon,
        NIF_ICON | NIF_MESSAGE | NIF_TIP | NIF_SHOWTIP,
    );

    unsafe {
        if !Shell_NotifyIconW(NIM_ADD, &raw const nid).as_bool() {
            return Err(last_error_as_brightness_error("Shell_NotifyIconW(NIM_ADD)"));
        }
    }

    log::debug!("Tray icon added to notification area");
    Ok(())
}

/// Removes the tray icon from the system notification area.
///
/// # Arguments
///
/// * `hwnd` - Window handle associated with the tray icon.
fn remove_tray_icon(hwnd: HWND) {
    let nid = NOTIFYICONDATAW {
        cbSize: u32::try_from(std::mem::size_of::<NOTIFYICONDATAW>()).unwrap_or(0),
        hWnd: hwnd,
        uID: TRAY_ICON_ID,
        ..Default::default()
    };

    unsafe {
        if !Shell_NotifyIconW(NIM_DELETE, &raw const nid).as_bool() {
            log::warn!("Failed to remove tray icon");
        } else {
            log::debug!("Tray icon removed from notification area");
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Context Menu
// ─────────────────────────────────────────────────────────────────────────────

/// Shows the tray icon context menu at the current cursor position.
///
/// # Arguments
///
/// * `hwnd` - Window handle for menu ownership and message routing.
fn show_context_menu(hwnd: HWND) {
    unsafe {
        // Create popup menu
        let Ok(hmenu) = CreatePopupMenu() else {
            log::error!("Failed to create popup menu");
            return;
        };

        // Request current monitor data from main thread
        let menu_data = request_menu_data();

        // Create tooltip if we have monitor data with hotkeys
        if let Some(ref data) = menu_data {
            // Store tooltip text from hotkeys
            set_tooltip_text(&data.hotkey_up, &data.hotkey_down);

            // Create tooltip window if not exists
            TRAY_TOOLTIP_HWND.with(|tooltip_cell| {
                let mut tooltip_opt = tooltip_cell.borrow_mut();
                if tooltip_opt.is_none() {
                    *tooltip_opt = create_tooltip(hwnd);
                }
            });
        }

        // Store menu handle for GetMenuItemRect calls
        TRAY_ACTIVE_MENU.with(|menu_cell| {
            *menu_cell.borrow_mut() = Some(hmenu);
        });

        // Add monitor info rows at the top (disabled/non-clickable)
        if let Some(ref data) = menu_data {
            for (index, monitor) in data.monitors.iter().enumerate() {
                let monitor_text = format!(
                    "{}: 🔆{}% 🕶{}%\0",
                    monitor.display_name, monitor.hardware_brightness, monitor.overlay_opacity
                );
                let monitor_wide: Vec<u16> = monitor_text.encode_utf16().collect();

                #[allow(clippy::cast_possible_truncation)]
                let menu_id = MENU_ID_MONITOR_BASE + index as u32;

                let _ = AppendMenuW(
                    hmenu,
                    MF_STRING | MF_GRAYED,
                    menu_id as usize,
                    PCWSTR(monitor_wide.as_ptr()),
                );
            }

            // Add separator after monitors (only if we have monitors)
            if !data.monitors.is_empty() {
                let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
            }
        }

        // Settings item
        let settings_wide: Vec<u16> = "Settings\0".encode_utf16().collect();
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            MENU_ID_SETTINGS as usize,
            PCWSTR(settings_wide.as_ptr()),
        );

        // Quit item
        let quit_text = format!("Quit {APP_NAME}\0");
        let quit_wide: Vec<u16> = quit_text.encode_utf16().collect();
        let _ = AppendMenuW(hmenu, MF_STRING, MENU_ID_QUIT as usize, PCWSTR(quit_wide.as_ptr()));

        // Get cursor position for menu placement
        let mut cursor_pos = POINT::default();
        if GetCursorPos(&raw mut cursor_pos).is_err() {
            log::warn!("GetCursorPos failed, using default position");
            cursor_pos = POINT { x: 0, y: 0 };
        }

        // Required: Set foreground window before showing menu
        // This ensures the menu dismisses when clicking outside
        let _ = SetForegroundWindow(hwnd);

        // Show menu and wait for selection
        let cmd = TrackPopupMenu(
            hmenu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RETURNCMD,
            cursor_pos.x,
            cursor_pos.y,
            0,
            hwnd,
            None,
        );

        // Clean up menu
        let _ = DestroyMenu(hmenu);

        // Send a null message to ensure the window processes the menu dismissal
        // This is a Windows quirk required for proper tray menu behavior
        let _ = PostMessageW(hwnd, WM_NULL, WPARAM(0), LPARAM(0));

        // Handle selection
        handle_menu_selection(cmd.0 as u32);
    }
}

/// Handles a menu item selection.
///
/// # Arguments
///
/// * `cmd` - The menu item ID that was selected (0 if menu was dismissed).
fn handle_menu_selection(cmd: u32) {
    match cmd {
        0 => {
            // Menu was dismissed without selection
            log::debug!("Tray menu dismissed");
        }
        MENU_ID_SETTINGS => {
            log::debug!("Settings menu item clicked");
            with_tray_sender(|sender| {
                if let Err(e) = sender.send(BrightnessMessage::TrayOpenSettings) {
                    log::error!("Failed to send TrayOpenSettings: {e}");
                }
            });
        }
        MENU_ID_QUIT => {
            log::debug!("Quit menu item clicked");
            with_tray_sender(|sender| {
                if let Err(e) = sender.send(BrightnessMessage::TrayRequestQuit) {
                    log::error!("Failed to send TrayRequestQuit: {e}");
                }
            });
        }
        _ => {
            log::debug!("Unknown menu item: {cmd}");
        }
    }
}

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
            WM_MENUSELECT => {
                handle_menu_select(hwnd, wparam);
                DefWindowProcW(hwnd, msg, wparam, lparam)
            }
            WM_EXITMENULOOP => {
                handle_menu_exit(hwnd);
                DefWindowProcW(hwnd, msg, wparam, lparam)
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
/// * `hwnd` - The window handle, used for menu ownership.
/// * `lparam` - Contains the mouse message (e.g., `WM_RBUTTONUP`).
fn handle_tray_callback(hwnd: HWND, lparam: LPARAM) {
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_RBUTTONUP};

    // The low word of lparam contains the mouse message
    #[allow(clippy::cast_possible_truncation)]
    let mouse_msg = (lparam.0 & 0xFFFF) as u32;

    match mouse_msg {
        WM_RBUTTONUP => {
            log::debug!("Tray icon right-clicked");
            show_context_menu(hwnd);
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
    /// Kept alive to prevent Windows from releasing the icon while the tray is active.
    #[allow(dead_code)]
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

        // Store sender in thread-local storage for window procedure access
        set_tray_sender(sender.clone());

        // Load the application icon
        let icon_handle = load_tray_icon()?;

        // Register the tray icon with the shell
        add_tray_icon(hwnd, icon_handle)?;

        Ok(Self {
            hwnd: unsafe { SafeHwnd::new_owned(hwnd) },
            sender,
            icon_handle,
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
        // Remove tray icon from notification area
        remove_tray_icon(self.hwnd.as_raw());
        log::debug!("TrayIcon dropped");
    }
}
