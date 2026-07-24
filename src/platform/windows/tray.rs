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
use std::sync::OnceLock;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, TRUE, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateBitmap, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GdiFlush, GetDC, HBITMAP, HDC, HGDIOBJ, ReleaseDC,
    SelectObject,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Shell::{
    NIF_ICON, NIF_MESSAGE, NIF_SHOWTIP, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFY_ICON_DATA_FLAGS, NOTIFYICONDATAW, Shell_NotifyIconW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateIconIndirect, CreatePopupMenu, CreateWindowExW, DI_NORMAL, DefWindowProcW,
    DestroyMenu, DispatchMessageW, DrawIconEx, GetCursorPos, GetMessageW, HICON, HWND_MESSAGE,
    ICONINFO, IMAGE_ICON, LR_DEFAULTSIZE, LR_LOADFROMFILE, LR_SHARED, LoadImageW, MF_GRAYED,
    MF_SEPARATOR, MF_STRING, MSG, PostMessageW, PostQuitMessage, RegisterClassExW,
    SetForegroundWindow, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON,
    TrackPopupMenu, TranslateMessage, WINDOW_EX_STYLE, WM_DESTROY, WM_NULL, WNDCLASSEXW,
    WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

use crate::core::state::{BrightnessMessage, HealthWarnings, TrayMenuData};
use crate::error::{BrightnessError, Result};

use super::{SafeHwnd, last_error_as_brightness_error};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Unique identifier for the tray icon (used with `Shell_NotifyIconW`).
const TRAY_ICON_ID: u32 = 1;

/// Custom window message for tray icon callbacks.
/// Using `WM_APP` + offset to avoid conflicts with system messages.
const WM_TRAY_CALLBACK: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 100;

/// Custom message posted by the main thread when degraded-state warnings
/// change; wparam bit 0 = DDC degraded, bit 1 = hotkeys lost.
const WM_TRAY_STATUS: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 101;

// ─────────────────────────────────────────────────────────────────────────────
// Menu Item IDs
// ─────────────────────────────────────────────────────────────────────────────

/// Menu item ID for the "Usage" option (shows hotkey instructions).
const MENU_ID_USAGE: u32 = 1000;

/// Menu item ID for the "Settings" option.
const MENU_ID_SETTINGS: u32 = 1001;

/// Menu item ID for the "Quit" option.
const MENU_ID_QUIT: u32 = 1002;

/// Menu item ID for the grayed version line.
const MENU_ID_VERSION: u32 = 1003;

/// Base ID for monitor info rows (non-clickable).
/// Each monitor uses `MENU_ID_MONITOR_BASE` + index.
const MENU_ID_MONITOR_BASE: u32 = 2000;

/// Base ID for degraded-subsystem warning rows (non-clickable).
const MENU_ID_WARNING_BASE: u32 = 3000;

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
// Warning Presentation (pure helpers)
// ─────────────────────────────────────────────────────────────────────────────

/// Composes the tray tooltip text from the active warnings.
fn compose_tooltip(warnings: HealthWarnings) -> String {
    let mut parts: Vec<&str> = Vec::new();
    if warnings.ddc_degraded {
        parts.push("DDC unavailable");
    }
    if warnings.hotkeys_lost {
        parts.push("hotkeys stopped");
    }
    if parts.is_empty() {
        TRAY_TOOLTIP.to_string()
    } else {
        format!("{TRAY_TOOLTIP} – {}", parts.join(", "))
    }
}

/// Grayed warning lines shown at the top of the tray menu.
fn warning_menu_lines(warnings: HealthWarnings) -> Vec<&'static str> {
    let mut lines = Vec::new();
    if warnings.ddc_degraded {
        // User activity is the recovery signal for a degraded DDC state.
        lines.push("⚠ DDC unavailable — press a brightness hotkey to retry");
    }
    if warnings.hotkeys_lost {
        // The give-up latch only clears with a fresh process.
        lines.push("⚠ Hotkeys stopped working — restart the app");
    }
    lines
}

/// Paints an amber warning badge (filled circle in the lower-right quadrant)
/// into a top-down 32-bit BGRA pixel buffer of `size`×`size` pixels.
fn paint_warning_badge(pixels: &mut [u8], size: usize) {
    /// Amber #FFB300, fully opaque, in BGRA byte order.
    const BADGE: [u8; 4] = [0x00, 0xB3, 0xFF, 0xFF];
    let center = (size * 3) / 4;
    let radius = size / 4;
    for y in center.saturating_sub(radius)..size.min(center + radius + 1) {
        for x in center.saturating_sub(radius)..size.min(center + radius + 1) {
            let dx = x.abs_diff(center);
            let dy = y.abs_diff(center);
            if dx * dx + dy * dy <= radius * radius {
                let i = (y * size + x) * 4;
                pixels[i..i + 4].copy_from_slice(&BADGE);
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Thread-Local Sender Storage
// ─────────────────────────────────────────────────────────────────────────────

thread_local! {
    /// Thread-local storage for the message sender.
    /// This allows the window procedure to send messages to the main thread.
    static TRAY_SENDER: RefCell<Option<Sender<BrightnessMessage>>> = const { RefCell::new(None) };
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
            log::warn!(error:% = e; "Failed to receive menu data");
            None
        }
    }
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
            BrightnessError::tray_icon_creation(format!("LoadImageW (resource) failed: {e}"))
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
                "LoadImageW (file '{path_str}') failed: {e}"
            ))
        })?;

        Ok(HICON(handle.0))
    }
}

/// Pixel edge length of the generated warning icon.
const WARNING_ICON_SIZE: usize = 32;

/// Base and warning-badged tray icons, chosen from on status updates.
#[derive(Clone, Copy)]
struct StatusIcons {
    normal: HICON,
    warning: HICON,
}

thread_local! {
    /// Icon pair for status updates; set once by `TrayIcon::new` (tray thread).
    static STATUS_ICONS: RefCell<Option<StatusIcons>> = const { RefCell::new(None) };
}

/// RAII: memory DC with a 32-bit top-down DIB section selected into it.
/// On drop it restores the old bitmap, deletes the DIB, then deletes the DC.
struct BadgeCanvas {
    dc: HDC,
    dib: HBITMAP,
    old: HGDIOBJ,
    bits: *mut u8,
}

impl BadgeCanvas {
    /// Allocates a `size`×`size` BGRA canvas. Returns `None` on GDI failure.
    fn new(size: i32) -> Option<Self> {
        unsafe {
            let screen = GetDC(None);
            let dc = CreateCompatibleDC(screen);
            ReleaseDC(None, screen);
            if dc.is_invalid() {
                return None;
            }

            let bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: u32::try_from(std::mem::size_of::<BITMAPINFOHEADER>()).unwrap_or(0),
                    biWidth: size,
                    // Negative height = top-down rows, so buffer row 0 is the
                    // top of the image and the badge quadrant is bottom-right.
                    biHeight: -size,
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };

            let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
            let Ok(dib) =
                CreateDIBSection(dc, &raw const bmi, DIB_RGB_COLORS, &raw mut bits, None, 0)
            else {
                let _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, dib);
            Some(Self {
                dc,
                dib,
                old,
                bits: bits.cast(),
            })
        }
    }
}

impl Drop for BadgeCanvas {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.dc, self.old);
            let _ = DeleteObject(self.dib);
            let _ = DeleteDC(self.dc);
        }
    }
}

/// Creates the warning variant of the tray icon: the base icon with an amber
/// badge painted into the lower-right quadrant.
///
/// # Errors
///
/// Returns `BrightnessError::TrayIconCreation` if any GDI/icon call fails.
fn create_warning_icon(base: HICON) -> Result<HICON> {
    let size = i32::try_from(WARNING_ICON_SIZE)
        .map_err(|_| BrightnessError::tray_icon_creation("icon size exceeds i32"))?;

    let canvas = BadgeCanvas::new(size)
        .ok_or_else(|| BrightnessError::tray_icon_creation("badge canvas allocation failed"))?;

    unsafe {
        DrawIconEx(canvas.dc, 0, 0, base, size, size, 0, None, DI_NORMAL)
            .map_err(|e| BrightnessError::tray_icon_creation(format!("DrawIconEx failed: {e}")))?;
        // Flush pending GDI drawing before touching the DIB bits directly.
        let _ = GdiFlush();

        let pixels =
            std::slice::from_raw_parts_mut(canvas.bits, WARNING_ICON_SIZE * WARNING_ICON_SIZE * 4);
        paint_warning_badge(pixels, WARNING_ICON_SIZE);

        // A monochrome mask is required by ICONINFO even though the 32-bit
        // color bitmap's alpha channel is what actually shapes the icon.
        let mask = CreateBitmap(size, size, 1, 1, None);
        if mask.is_invalid() {
            return Err(BrightnessError::tray_icon_creation("mask bitmap failed"));
        }

        let info = ICONINFO {
            fIcon: TRUE,
            xHotspot: 0,
            yHotspot: 0,
            hbmMask: mask,
            hbmColor: canvas.dib,
        };
        // CreateIconIndirect copies both bitmaps; canvas + mask can be freed.
        let icon = CreateIconIndirect(&raw const info);
        let _ = DeleteObject(mask);
        icon.map_err(|e| {
            BrightnessError::tray_icon_creation(format!("CreateIconIndirect failed: {e}"))
        })
    }
}

/// Applies a posted status update: swaps the tray icon and tooltip to match
/// the active warnings.
fn handle_status_update(hwnd: HWND, wparam: WPARAM) {
    let warnings = HealthWarnings {
        ddc_degraded: wparam.0 & 0b01 != 0,
        hotkeys_lost: wparam.0 & 0b10 != 0,
    };

    let Some(icons) = STATUS_ICONS.with(|s| *s.borrow()) else {
        return;
    };
    let icon = if warnings.ddc_degraded || warnings.hotkeys_lost {
        icons.warning
    } else {
        icons.normal
    };

    let tooltip = compose_tooltip(warnings);
    let nid = create_notify_icon_data(hwnd, icon, &tooltip, NIF_ICON | NIF_TIP | NIF_SHOWTIP);
    unsafe {
        if !Shell_NotifyIconW(NIM_MODIFY, &raw const nid).as_bool() {
            log::warn!(error_code = super::get_last_error_code(); "Failed to update tray status");
            return;
        }
    }
    log::debug!(
        ddc_degraded = warnings.ddc_degraded,
        hotkeys_lost = warnings.hotkeys_lost;
        "Tray status updated"
    );
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
/// * `tooltip` - Tooltip text (truncated to the fixed `szTip` capacity).
/// * `flags` - Which fields are valid in the structure.
fn create_notify_icon_data(
    hwnd: HWND,
    icon: HICON,
    tooltip: &str,
    flags: NOTIFY_ICON_DATA_FLAGS,
) -> NOTIFYICONDATAW {
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
    let tooltip_wide: Vec<u16> = tooltip.encode_utf16().collect();
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
        TRAY_TOOLTIP,
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
        if Shell_NotifyIconW(NIM_DELETE, &raw const nid).as_bool() {
            log::debug!("Tray icon removed from notification area");
        } else {
            log::warn!(error_code = super::get_last_error_code(); "Failed to remove tray icon");
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
            log::error!(error_code = super::get_last_error_code(); "Failed to create popup menu");
            return;
        };

        // Request current monitor data from main thread
        let menu_data = request_menu_data();

        // Add monitor info rows at the top (disabled/non-clickable)
        if let Some(ref data) = menu_data {
            // Degraded-subsystem warnings come first so they cannot be missed.
            let warn_lines = warning_menu_lines(data.warnings);
            for (index, line) in warn_lines.iter().enumerate() {
                let line_text = format!("{line}\0");
                let line_wide: Vec<u16> = line_text.encode_utf16().collect();

                // Menu IDs are u32; the warning count is at most 2
                #[allow(clippy::cast_possible_truncation)]
                let menu_id = MENU_ID_WARNING_BASE + (index as u32);

                let _ = AppendMenuW(
                    hmenu,
                    MF_STRING | MF_GRAYED,
                    menu_id as usize,
                    PCWSTR(line_wide.as_ptr()),
                );
            }
            if !warn_lines.is_empty() {
                let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
            }

            for (index, monitor) in data.monitors.iter().enumerate() {
                let monitor_text = format!(
                    "{}: 🕶{}% 🔆{}%\0",
                    monitor.display_name, monitor.overlay_opacity, monitor.hardware_brightness
                );
                let monitor_wide: Vec<u16> = monitor_text.encode_utf16().collect();

                // Menu IDs are u32; index won't exceed monitor count (typically < 10)
                #[allow(clippy::cast_possible_truncation)]
                let menu_id = MENU_ID_MONITOR_BASE + (index as u32);

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

        // Usage item (shows hotkey instructions window)
        let usage_wide: Vec<u16> = "Usage\0".encode_utf16().collect();
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            MENU_ID_USAGE as usize,
            PCWSTR(usage_wide.as_ptr()),
        );

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
        let _ = AppendMenuW(
            hmenu,
            MF_STRING,
            MENU_ID_QUIT as usize,
            PCWSTR(quit_wide.as_ptr()),
        );

        // Version line (grayed, informational)
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
        let version_text = format!("{APP_NAME} v{}\0", env!("CARGO_PKG_VERSION"));
        let version_wide: Vec<u16> = version_text.encode_utf16().collect();
        let _ = AppendMenuW(
            hmenu,
            MF_STRING | MF_GRAYED,
            MENU_ID_VERSION as usize,
            PCWSTR(version_wide.as_ptr()),
        );

        // Get cursor position for menu placement
        let mut cursor_pos = POINT::default();
        if GetCursorPos(&raw mut cursor_pos).is_err() {
            log::warn!(error_code = super::get_last_error_code(); "GetCursorPos failed, using default position");
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

        // Handle selection (menu item IDs are non-negative)
        #[allow(clippy::cast_sign_loss)]
        let menu_cmd = cmd.0 as u32;
        handle_menu_selection(menu_cmd);
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
        MENU_ID_USAGE => {
            log::debug!("Usage menu item clicked");
            with_tray_sender(|sender| {
                if let Err(e) = sender.send(BrightnessMessage::TrayOpenUsage) {
                    log::error!(error:% = e; "Failed to send TrayOpenUsage");
                }
            });
        }
        MENU_ID_SETTINGS => {
            log::debug!("Settings menu item clicked");
            with_tray_sender(|sender| {
                if let Err(e) = sender.send(BrightnessMessage::TrayOpenSettings) {
                    log::error!(error:% = e; "Failed to send TrayOpenSettings");
                }
            });
        }
        MENU_ID_QUIT => {
            log::debug!("Quit menu item clicked");
            with_tray_sender(|sender| {
                if let Err(e) = sender.send(BrightnessMessage::TrayRequestQuit) {
                    log::error!(error:% = e; "Failed to send TrayRequestQuit");
                }
            });
        }
        _ => {
            log::debug!(menu_id = cmd; "Unknown menu item");
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
        .map_err(|e| {
            BrightnessError::tray_icon_creation(format!("Class registration failed: {e}"))
        })?;

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
            WM_TRAY_STATUS => {
                handle_status_update(hwnd, wparam);
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
/// * `hwnd` - The window handle, used for menu ownership.
/// * `lparam` - Contains the mouse message (e.g., `WM_RBUTTONUP`).
fn handle_tray_callback(hwnd: HWND, lparam: LPARAM) {
    use windows::Win32::UI::WindowsAndMessaging::{WM_LBUTTONUP, WM_RBUTTONUP};

    // The low word of lparam contains the mouse message
    // Masking with 0xFFFF ensures the value fits in u32
    #[allow(clippy::cast_possible_truncation)]
    let mouse_msg = (lparam.0 & 0xFFFF).cast_unsigned() as u32;

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
                WINDOW_EX_STYLE::default(), // No extended styles needed
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

        // Derive the warning variant; on failure the base icon doubles as the
        // warning icon (the tooltip and menu still carry the warning text).
        let warning_icon = match create_warning_icon(icon_handle) {
            Ok(icon) => icon,
            Err(e) => {
                log::warn!(error:% = e; "Failed to create warning tray icon, using base icon");
                icon_handle
            }
        };
        STATUS_ICONS.with(|s| {
            *s.borrow_mut() = Some(StatusIcons {
                normal: icon_handle,
                warning: warning_icon,
            });
        });

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

    /// Returns a cross-thread handle for posting status updates.
    #[must_use]
    pub fn status_handle(&self) -> TrayStatusHandle {
        TrayStatusHandle(self.hwnd.as_raw().0)
    }
}

/// Cross-thread handle for posting degraded-state updates to the tray window.
///
/// Wraps the raw window handle value; `PostMessageW` may be called from any
/// thread and fails harmlessly once the window is gone.
#[derive(Debug, Clone, Copy)]
pub struct TrayStatusHandle(isize);

impl TrayStatusHandle {
    /// Posts the current warnings to the tray thread (fire-and-forget).
    pub fn notify(self, warnings: HealthWarnings) {
        let bits = usize::from(warnings.ddc_degraded) | (usize::from(warnings.hotkeys_lost) << 1);
        unsafe {
            if let Err(e) = PostMessageW(HWND(self.0), WM_TRAY_STATUS, WPARAM(bits), LPARAM(0)) {
                log::debug!(error:% = e; "Tray status post failed (tray window gone?)");
            }
        }
    }
}

impl Drop for TrayIcon {
    fn drop(&mut self) {
        // Remove tray icon from notification area
        remove_tray_icon(self.hwnd.as_raw());
        log::debug!("TrayIcon dropped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tooltip_plain_when_healthy() {
        assert_eq!(
            compose_tooltip(HealthWarnings::default()),
            "Brightness Control"
        );
    }

    #[test]
    fn tooltip_lists_active_warnings() {
        let ddc = HealthWarnings {
            ddc_degraded: true,
            hotkeys_lost: false,
        };
        assert_eq!(compose_tooltip(ddc), "Brightness Control – DDC unavailable");

        let keys = HealthWarnings {
            ddc_degraded: false,
            hotkeys_lost: true,
        };
        assert_eq!(
            compose_tooltip(keys),
            "Brightness Control – hotkeys stopped"
        );

        let both = HealthWarnings {
            ddc_degraded: true,
            hotkeys_lost: true,
        };
        assert_eq!(
            compose_tooltip(both),
            "Brightness Control – DDC unavailable, hotkeys stopped"
        );
    }

    #[test]
    fn menu_warning_lines_match_active_warnings() {
        assert!(warning_menu_lines(HealthWarnings::default()).is_empty());

        let both = HealthWarnings {
            ddc_degraded: true,
            hotkeys_lost: true,
        };
        let lines = warning_menu_lines(both);
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("DDC"));
        assert!(lines[1].contains("Hotkeys"));
    }

    #[test]
    fn warning_badge_paints_amber_circle_bottom_right() {
        const SIZE: usize = 32;
        let mut pixels = vec![0u8; SIZE * SIZE * 4];
        paint_warning_badge(&mut pixels, SIZE);

        // Badge center at (3/4, 3/4) of the icon is amber, fully opaque (BGRA).
        let center = (24 * SIZE + 24) * 4;
        assert_eq!(&pixels[center..center + 4], &[0x00, 0xB3, 0xFF, 0xFF]);

        // Top-left corner (base image area) stays untouched.
        assert_eq!(&pixels[0..4], &[0, 0, 0, 0]);

        // Inside the badge bounding box but outside the circle: untouched.
        let outside = (31 * SIZE + 16) * 4;
        assert_eq!(&pixels[outside..outside + 4], &[0, 0, 0, 0]);
    }
}
