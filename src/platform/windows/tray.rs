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
    DestroyMenu, DispatchMessageW, DrawIconEx, GetCursorPos, GetMessageW, HICON, HMENU,
    HWND_MESSAGE, ICONINFO, IMAGE_ICON, KillTimer, LR_DEFAULTSIZE, LR_LOADFROMFILE, LR_SHARED,
    LoadImageW, MENU_ITEM_FLAGS, MENUITEMINFOW, MF_GRAYED, MF_SEPARATOR, MF_STRING, MIIM_STRING,
    MSG, PostMessageW, PostQuitMessage, RegisterClassExW, SetForegroundWindow, SetMenuItemInfoW,
    SetTimer, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RETURNCMD, TPM_RIGHTBUTTON, TrackPopupMenu,
    TranslateMessage, WINDOW_EX_STYLE, WM_DESTROY, WM_NULL, WM_TIMER, WNDCLASSEXW, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, PWSTR, w};

use crate::core::state::{
    BrightnessMessage, DdcHealth, HealthWarnings, TrayMenuData, changed_rows, monitor_menu_line,
};
use crate::core::version::version_string;
use crate::error::{BrightnessError, Result};

use super::theme;
use super::{SafeHwnd, hwnd_from_isize, hwnd_to_isize, last_error_as_brightness_error};

// ─────────────────────────────────────────────────────────────────────────────
// Constants
// ─────────────────────────────────────────────────────────────────────────────

/// Unique identifier for the tray icon (used with `Shell_NotifyIconW`).
const TRAY_ICON_ID: u32 = 1;

/// Custom window message for tray icon callbacks.
/// Using `WM_APP` + offset to avoid conflicts with system messages.
const WM_TRAY_CALLBACK: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 100;

/// Custom message posted by the main thread when degraded-state warnings
/// change; the payload is packed by [`warnings_to_bits`].
const WM_TRAY_STATUS: u32 = windows::Win32::UI::WindowsAndMessaging::WM_APP + 101;

// ─────────────────────────────────────────────────────────────────────────────
// Menu Item IDs
// ─────────────────────────────────────────────────────────────────────────────

/// Menu item ID for the "Settings" option.
const MENU_ID_SETTINGS: u32 = 1001;

/// Menu item ID for the "Quit" option.
const MENU_ID_QUIT: u32 = 1002;

/// Menu item ID for the grayed version line.
const MENU_ID_VERSION: u32 = 1003;

/// Menu item ID for the "Open Log Folder" option.
const MENU_ID_OPEN_LOGS: u32 = 1004;

/// Base ID for monitor info rows (non-clickable).
/// Each monitor uses `MENU_ID_MONITOR_BASE` + index.
const MENU_ID_MONITOR_BASE: u32 = 2000;

/// Base ID for degraded-subsystem warning rows (non-clickable).
const MENU_ID_WARNING_BASE: u32 = 3000;

/// Base ID for the usage instruction rows (non-clickable).
const MENU_ID_USAGE_BASE: u32 = 4000;

/// Tooltip text shown when hovering over the tray icon.
const TRAY_TOOLTIP: &str = "Brightness Control";

/// Resource ID for the embedded application icon.
/// Must match the ID defined in build.rs when embedding resources.
const IDI_APP_ICON: u16 = 1;

/// Application name displayed in the tray menu.
const APP_NAME: &str = "Brightness Control";

/// Timeout for waiting for menu data from the main thread.
const MENU_DATA_TIMEOUT: Duration = Duration::from_millis(500);

/// How often an open menu re-reads its monitor rows.
///
/// Chosen against the OSD rather than against reaction time: the OSD is the
/// instant feedback for a hotkey press, so the row only has to stop being
/// wrong before the eye moves to it.
const MENU_REFRESH_INTERVAL_MS: u32 = 250;

/// Timeout for one refresh round trip.
///
/// Deliberately far below [`MENU_DATA_TIMEOUT`]: on this path the tray thread
/// is inside the menu's modal loop and pumps nothing while it waits, so the
/// wait *is* a frozen menu. The main loop wakes on the send and answers within
/// its 16 ms tick, so 50 ms is already generous.
const MENU_POLL_TIMEOUT: Duration = Duration::from_millis(50);

/// Consecutive unanswered refreshes before an open menu stops refreshing.
///
/// One hiccup must not kill the feature; a wedged main thread must not stutter
/// the menu four times a second.
const MENU_POLL_MISS_LIMIT: u32 = 2;

/// Timer id for the open menu's refresh.
///
/// Timers are keyed by window *and* id, so this cannot collide with the OSD's
/// timer, which lives on another window owned by another thread.
const MENU_REFRESH_TIMER_ID: usize = 1;

// ─────────────────────────────────────────────────────────────────────────────
// Warning Presentation (pure helpers)
// ─────────────────────────────────────────────────────────────────────────────

/// Composes the tray tooltip text from the active warnings.
fn compose_tooltip(warnings: HealthWarnings) -> String {
    let mut parts: Vec<&str> = Vec::new();
    match warnings.ddc {
        DdcHealth::Ok => {}
        DdcHealth::WorkerDead => parts.push("DDC unavailable"),
        DdcHealth::WorkerHung => parts.push("monitor not responding"),
    }
    if warnings.hotkeys_lost {
        parts.push("hotkeys stopped");
    }
    if warnings.hotkeys_degraded {
        parts.push("hotkey change failed");
    }
    if warnings.file_log_failed {
        parts.push("file logging off");
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
    match warnings.ddc {
        DdcHealth::Ok => {}
        // A keypress clears the respawn backoff, so this retry is real.
        DdcHealth::WorkerDead => {
            lines.push("⚠ DDC unavailable — press a brightness hotkey to retry");
        }
        // Nothing the user can press unsticks a blocked DDC call. It often
        // frees itself, so restarting is advice, not an instruction.
        DdcHealth::WorkerHung => {
            lines.push("⚠ Monitor not responding — restart the app if this persists");
        }
    }
    if warnings.hotkeys_lost {
        // The give-up latch only clears with a fresh process.
        lines.push("⚠ Hotkeys stopped working — restart the app");
    }
    if warnings.hotkeys_degraded {
        // Unlike hotkeys_lost, this clears on the next successful rebind —
        // no restart needed.
        lines.push("⚠ Hotkey change failed — try another combination");
    }
    if warnings.file_log_failed {
        // "Open Log Folder" sits a few items below in this same menu, which is
        // what someone hunting for a missing log clicks — so point at the
        // folder rather than explain an I/O error nobody can act on.
        lines.push("⚠ File logging failed to start — check the log folder is writable");
    }
    lines
}

// ─────────────────────────────────────────────────────────────────────────────
// Usage Presentation (pure helper)
// ─────────────────────────────────────────────────────────────────────────────

/// Composes the grayed rows that teach the core interaction.
///
/// The block reads as one sentence finished by two key bindings. Everything
/// after a `\t` is drawn right-aligned in the menu's shortcut column — the
/// place a reader already scans for a key combination, and the reason the rows
/// stay narrower than they would as prose.
///
/// The bindings come from the running configuration, so a user who rebound
/// them is taught the keys that actually work.
fn usage_menu_lines(hotkey_up: &str, hotkey_down: &str) -> [String; 3] {
    [
        "Point mouse at a monitor, then:".to_string(),
        format!("Brighter\t{hotkey_up}"),
        format!("Dimmer\t{hotkey_down}"),
    ]
}

/// Whether the tray icon should carry the amber warning badge.
///
/// Covers both a permanently lost hotkey thread (`hotkeys_lost`, latched
/// until restart) and a degraded one (`hotkeys_degraded`, cleared by the next
/// successful rebind/resume): either way the app is not responding to
/// hotkeys right now, and that must never sit behind a healthy-looking icon.
/// Deliberately excludes a failed file log — a missing diagnostic log does
/// not stop a single adjustment, and letting it light the badge would weaken
/// the signal for the conditions that genuinely mean something is broken.
fn wants_warning_badge(warnings: HealthWarnings) -> bool {
    warnings.ddc.is_degraded() || warnings.hotkeys_lost || warnings.hotkeys_degraded
}

/// Packs warnings into the `wparam` payload of a [`WM_TRAY_STATUS`] post.
///
/// The tray runs on its own thread, so the state has to survive a trip through
/// a window message; low two bits carry the DDC condition, bit 2 `hotkeys_lost`,
/// bit 3 `file_log_failed`, bit 4 `hotkeys_degraded`.
fn warnings_to_bits(warnings: HealthWarnings) -> usize {
    let ddc = match warnings.ddc {
        DdcHealth::Ok => 0,
        DdcHealth::WorkerDead => 1,
        DdcHealth::WorkerHung => 2,
    };
    ddc | (usize::from(warnings.hotkeys_lost) << 2)
        | (usize::from(warnings.file_log_failed) << 3)
        | (usize::from(warnings.hotkeys_degraded) << 4)
}

/// Unpacks what [`warnings_to_bits`] wrote. Unknown bit patterns decode as
/// healthy — a garbled post must not invent a warning.
fn warnings_from_bits(bits: usize) -> HealthWarnings {
    HealthWarnings {
        ddc: match bits & 0b11 {
            1 => DdcHealth::WorkerDead,
            2 => DdcHealth::WorkerHung,
            _ => DdcHealth::Ok,
        },
        hotkeys_lost: bits & 0b100 != 0,
        file_log_failed: bits & 0b1000 != 0,
        hotkeys_degraded: bits & 0b1_0000 != 0,
    }
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

    /// Last hotkey pair received from the main thread, kept as a fallback for
    /// the usage rows when a menu open's `TrayMenuOpening` request times out.
    static LAST_HOTKEYS: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
}

/// Bookkeeping for the popup menu currently on screen.
///
/// Lives on the tray thread only, which is why it can hold an `HMENU` — that
/// handle is not `Send` and never leaves this thread.
struct MenuSession {
    /// The live popup.
    hmenu: HMENU,
    /// Display names in row order. A refresh may only rewrite row `i` while
    /// this still matches, or two identical panels could swap their rows
    /// under the reader.
    names: Vec<String>,
    /// Last text written to each row, in the same order.
    rows: Vec<String>,
    /// Cleared once refreshing this menu must not be retried.
    refreshing: bool,
    /// Consecutive refreshes the main thread did not answer.
    misses: u32,
}

thread_local! {
    /// The open menu, or `None` when none is open. Doubles as the re-entrancy
    /// guard: the shell can post a second right-click callback that the modal
    /// loop dispatches, and a nested menu would take over this slot.
    static MENU_SESSION: RefCell<Option<MenuSession>> = const { RefCell::new(None) };
}

/// Runs `f` against the open menu's session, if there is one.
fn with_session<R>(f: impl FnOnce(&mut MenuSession) -> R) -> Option<R> {
    MENU_SESSION.with(|cell| cell.borrow_mut().as_mut().map(f))
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
/// Sends a `TrayMenuOpening` message and waits for the response.
/// Returns `None` if the request times out or fails. The timeout is the
/// caller's because the two callers differ in what a wait costs: before the
/// popup exists it is a delayed menu, inside the modal loop it is a frozen one.
fn request_menu_data(timeout: Duration) -> Option<TrayMenuData> {
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

    match reply_rx.recv_timeout(timeout) {
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
            Some(hinstance.into()),
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
            let dc = CreateCompatibleDC(Some(screen));
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
            let Ok(dib) = CreateDIBSection(
                Some(dc),
                &raw const bmi,
                DIB_RGB_COLORS,
                &raw mut bits,
                None,
                0,
            ) else {
                let _ = DeleteDC(dc);
                return None;
            };
            let old = SelectObject(dc, dib.into());
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
            let _ = DeleteObject(self.dib.into());
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
        let _ = DeleteObject(mask.into());
        icon.map_err(|e| {
            BrightnessError::tray_icon_creation(format!("CreateIconIndirect failed: {e}"))
        })
    }
}

/// Applies a posted status update: swaps the tray icon and tooltip to match
/// the active warnings.
fn handle_status_update(hwnd: HWND, wparam: WPARAM) {
    let warnings = warnings_from_bits(wparam.0);

    let Some(icons) = STATUS_ICONS.with(|s| *s.borrow()) else {
        return;
    };
    let icon = if wants_warning_badge(warnings) {
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
        ddc:? = warnings.ddc,
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

/// Appends one text menu item; a failure degrades the menu by one row rather
/// than breaking it, so the result is ignored.
fn append_menu_item(hmenu: HMENU, flags: MENU_ITEM_FLAGS, id: u32, text: &str) {
    let wide: Vec<u16> = format!("{text}\0").encode_utf16().collect();
    unsafe {
        let _ = AppendMenuW(hmenu, flags, id as usize, PCWSTR(wide.as_ptr()));
    }
}

/// Rewrites one already-appended menu item's text.
///
/// Addressed by command id rather than by position, so the number of warning
/// rows above the monitor block does not have to be tracked. Windows repaints
/// the visible popup by itself after this call, and widens it if the new text
/// needs more room.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if the item cannot be updated, which
/// for a live menu means the id is gone or the menu is no longer valid.
fn set_menu_item_text(hmenu: HMENU, menu_id: u32, text: &str) -> Result<()> {
    let mut wide: Vec<u16> = format!("{text}\0").encode_utf16().collect();
    let info = MENUITEMINFOW {
        cbSize: u32::try_from(std::mem::size_of::<MENUITEMINFOW>()).unwrap_or(0),
        // MIIM_STRING alone: MIIM_TYPE would also rewrite fType and drop
        // MFT_STRING along with the item's grayed appearance.
        fMask: MIIM_STRING,
        dwTypeData: PWSTR(wide.as_mut_ptr()),
        ..Default::default()
    };
    // SAFETY: `info` describes a string update only, and `wide` is a
    // NUL-terminated buffer that outlives the call.
    unsafe { SetMenuItemInfoW(hmenu, menu_id, false, &raw const info) }
        .map_err(|e| BrightnessError::windows_api("SetMenuItemInfoW", e.code().0.cast_unsigned()))
}

/// Re-reads the monitor rows of the open menu and writes back what changed.
///
/// Runs inside `TrackPopupMenu`'s modal loop. Anything slow here is a menu
/// that does not respond to the mouse, which is why the round trip is on a
/// short leash and why every giving-up condition latches for the rest of the
/// menu rather than being retried.
fn refresh_open_menu() {
    let Some((hmenu, names, rows)) = with_session(|session| {
        session
            .refreshing
            .then(|| (session.hmenu, session.names.clone(), session.rows.clone()))
    })
    .flatten() else {
        return;
    };

    let Some(data) = request_menu_data(MENU_POLL_TIMEOUT) else {
        let gave_up = with_session(|session| {
            session.misses += 1;
            let gave_up = session.misses >= MENU_POLL_MISS_LIMIT;
            if gave_up {
                session.refreshing = false;
            }
            gave_up
        })
        .unwrap_or(false);
        if gave_up {
            log::warn!("Main thread did not answer; tray menu rows stay as they are");
        }
        return;
    };

    let next_names: Vec<String> = data
        .monitors
        .iter()
        .map(|monitor| monitor.display_name.clone())
        .collect();
    let next_rows: Vec<String> = data.monitors.iter().map(monitor_menu_line).collect();

    let Some(updates) = changed_rows(&names, &rows, &next_names, &next_rows) else {
        let _ = with_session(|session| session.refreshing = false);
        log::debug!("Monitor set changed while the tray menu was open; rows frozen");
        return;
    };

    for (index, text) in updates {
        // Menu IDs are u32; index won't exceed monitor count (typically < 10)
        #[allow(clippy::cast_possible_truncation)]
        let menu_id = MENU_ID_MONITOR_BASE + (index as u32);
        if let Err(e) = set_menu_item_text(hmenu, menu_id, &text) {
            let _ = with_session(|session| session.refreshing = false);
            log::warn!(error:% = e; "Tray menu row update failed; rows frozen");
            return;
        }
    }

    let _ = with_session(|session| {
        session.rows = next_rows;
        session.misses = 0;
    });
}

/// Appends a separator line.
fn append_separator(hmenu: HMENU) {
    unsafe {
        let _ = AppendMenuW(hmenu, MF_SEPARATOR, 0, PCWSTR::null());
    }
}

/// Shows the tray icon context menu at the current cursor position.
///
/// # Arguments
///
/// * `hwnd` - Window handle for menu ownership and message routing.
// Building the menu is one straight-line sequence of rows in display order;
// splitting it into helpers would only scatter that order.
#[allow(clippy::too_many_lines)]
fn show_context_menu(hwnd: HWND) {
    // The shell can post a second right-click callback that the modal loop
    // dispatches; a nested menu would take over the session slot and the
    // shared timer, silently killing the outer menu's refresh.
    if MENU_SESSION.with(|cell| cell.borrow().is_some()) {
        log::debug!("Tray menu already open; ignoring the second open");
        return;
    }

    // The system light/dark setting may have changed since the last menu.
    theme::refresh_menu_theme();

    unsafe {
        // Create popup menu
        let Ok(hmenu) = CreatePopupMenu() else {
            log::error!(error_code = super::get_last_error_code(); "Failed to create popup menu");
            return;
        };

        // Request current monitor data from main thread
        let menu_data = request_menu_data(MENU_DATA_TIMEOUT);

        let mut row_names: Vec<String> = Vec::new();
        let mut row_texts: Vec<String> = Vec::new();

        if let Some(ref data) = menu_data {
            // Degraded-subsystem warnings come first so they cannot be missed.
            let warn_lines = warning_menu_lines(data.warnings);
            for (index, line) in warn_lines.iter().enumerate() {
                // Menu IDs are u32; the warning count is at most 2
                #[allow(clippy::cast_possible_truncation)]
                let menu_id = MENU_ID_WARNING_BASE + (index as u32);
                append_menu_item(hmenu, MF_STRING | MF_GRAYED, menu_id, line);
            }
            if !warn_lines.is_empty() {
                append_separator(hmenu);
            }

            // Monitor info rows (disabled/non-clickable)
            for (index, monitor) in data.monitors.iter().enumerate() {
                let monitor_text = monitor_menu_line(monitor);
                // Menu IDs are u32; index won't exceed monitor count (typically < 10)
                #[allow(clippy::cast_possible_truncation)]
                let menu_id = MENU_ID_MONITOR_BASE + (index as u32);
                append_menu_item(hmenu, MF_STRING | MF_GRAYED, menu_id, &monitor_text);
                row_names.push(monitor.display_name.clone());
                row_texts.push(monitor_text);
            }
            if !data.monitors.is_empty() {
                append_separator(hmenu);
            }
        }

        // How to use the app, stated rather than hidden behind a click: this
        // is the only place a first-time user is told what to press. The
        // bindings come with each `TrayMenuOpening` reply, so a rebind in the
        // settings dialog shows up the next time this menu opens. If the
        // request above timed out, the last pair this thread did receive
        // stands in rather than showing nothing.
        let hotkeys = match &menu_data {
            Some(data) => {
                let pair = (data.hotkey_up.clone(), data.hotkey_down.clone());
                LAST_HOTKEYS.with(|last| *last.borrow_mut() = Some(pair.clone()));
                Some(pair)
            }
            None => LAST_HOTKEYS.with(|last| last.borrow().clone()),
        };
        if let Some((hotkey_up, hotkey_down)) = hotkeys {
            let lines = usage_menu_lines(&hotkey_up, &hotkey_down);
            for (index, line) in lines.iter().enumerate() {
                // Menu IDs are u32; this block is exactly three rows.
                #[allow(clippy::cast_possible_truncation)]
                let menu_id = MENU_ID_USAGE_BASE + (index as u32);
                append_menu_item(hmenu, MF_STRING | MF_GRAYED, menu_id, line);
            }
            append_separator(hmenu);
        }

        append_menu_item(hmenu, MF_STRING, MENU_ID_SETTINGS, "Settings");
        append_menu_item(hmenu, MF_STRING, MENU_ID_OPEN_LOGS, "Open Log Folder");
        append_menu_item(hmenu, MF_STRING, MENU_ID_QUIT, &format!("Quit {APP_NAME}"));

        // Version line (grayed, informational)
        append_separator(hmenu);
        let version_text = format!("{APP_NAME} v{}", version_string());
        append_menu_item(hmenu, MF_STRING | MF_GRAYED, MENU_ID_VERSION, &version_text);

        // Get cursor position for menu placement
        let mut cursor_pos = POINT::default();
        if GetCursorPos(&raw mut cursor_pos).is_err() {
            log::warn!(error_code = super::get_last_error_code(); "GetCursorPos failed, using default position");
            cursor_pos = POINT { x: 0, y: 0 };
        }

        // Required: Set foreground window before showing menu
        // This ensures the menu dismisses when clicking outside
        let _ = SetForegroundWindow(hwnd);

        MENU_SESSION.with(|cell| {
            *cell.borrow_mut() = Some(MenuSession {
                hmenu,
                names: row_names,
                rows: row_texts,
                refreshing: true,
                misses: 0,
            });
        });
        // The timer lives exactly as long as the popup, so an idle process
        // does no periodic work at all.
        if SetTimer(
            Some(hwnd),
            MENU_REFRESH_TIMER_ID,
            MENU_REFRESH_INTERVAL_MS,
            None,
        ) == 0
        {
            log::warn!(
                error_code = super::get_last_error_code();
                "Tray menu refresh timer not started; rows stay as opened"
            );
        }

        // Show menu and wait for selection
        let cmd = TrackPopupMenu(
            hmenu,
            TPM_RIGHTBUTTON | TPM_BOTTOMALIGN | TPM_LEFTALIGN | TPM_RETURNCMD,
            cursor_pos.x,
            cursor_pos.y,
            None,
            hwnd,
            None,
        );

        // Order matters. `KillTimer` does not purge WM_TIMER messages that are
        // already queued, so the session has to be gone before the handle is:
        // a straggler tick then finds no session and returns, instead of
        // writing to a destroyed menu.
        let _ = KillTimer(Some(hwnd), MENU_REFRESH_TIMER_ID);
        MENU_SESSION.with(|cell| *cell.borrow_mut() = None);
        let _ = DestroyMenu(hmenu);

        // Send a null message to ensure the window processes the menu dismissal
        // This is a Windows quirk required for proper tray menu behavior
        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));

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
        MENU_ID_SETTINGS => {
            log::debug!("Settings menu item clicked");
            with_tray_sender(|sender| {
                if let Err(e) = sender.send(BrightnessMessage::TrayOpenSettings) {
                    log::error!(error:% = e; "Failed to send TrayOpenSettings");
                }
            });
        }
        MENU_ID_OPEN_LOGS => {
            log::debug!("Open Log Folder menu item clicked");
            with_tray_sender(|sender| {
                if let Err(e) = sender.send(BrightnessMessage::TrayOpenLogFolder) {
                    log::error!(error:% = e; "Failed to send TrayOpenLogFolder");
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
            WM_TIMER => {
                if wparam.0 == MENU_REFRESH_TIMER_ID {
                    refresh_open_menu();
                }
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

        // Before the first menu exists: without this the context menu is drawn
        // light even when the system asks for dark.
        theme::init_dark_menus();

        // Create message-only window (HWND_MESSAGE parent = no visible window)
        let hwnd = unsafe {
            let hinstance = GetModuleHandleW(None).map_err(|e| {
                BrightnessError::tray_icon_creation(format!(
                    "GetModuleHandleW failed: {}",
                    e.code().0
                ))
            })?;

            CreateWindowExW(
                WINDOW_EX_STYLE::default(), // No extended styles needed
                class_name,
                w!("BrightnessControlTray"),
                WS_OVERLAPPED, // Minimal style for message-only window
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE), // Message-only window
                None,
                Some(hinstance.into()),
                None,
            )
            .map_err(|e| {
                BrightnessError::windows_api("CreateWindowExW", e.code().0.cast_unsigned())
            })?
        };

        log::debug!("Tray message window created");

        // The menu belongs to this window, so the window has to be dark-mode
        // aware as well for the popup to pick the dark theme up.
        theme::allow_dark_mode_for_window(hwnd);

        // Store sender in thread-local storage for window procedure access
        set_tray_sender(sender);

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
            let result = unsafe { GetMessageW(&raw mut msg, None, 0, 0) };

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

    /// Returns a cross-thread handle for posting status updates.
    #[must_use]
    pub fn status_handle(&self) -> TrayStatusHandle {
        TrayStatusHandle(hwnd_to_isize(self.hwnd.as_raw()))
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
        let bits = warnings_to_bits(warnings);
        unsafe {
            if let Err(e) = PostMessageW(
                Some(hwnd_from_isize(self.0)),
                WM_TRAY_STATUS,
                WPARAM(bits),
                LPARAM(0),
            ) {
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

    /// Warnings with only the DDC condition set.
    fn ddc_only(ddc: DdcHealth) -> HealthWarnings {
        HealthWarnings {
            ddc,
            ..HealthWarnings::default()
        }
    }

    #[test]
    fn tooltip_lists_active_warnings() {
        assert_eq!(
            compose_tooltip(ddc_only(DdcHealth::WorkerDead)),
            "Brightness Control – DDC unavailable"
        );

        let keys = HealthWarnings {
            hotkeys_lost: true,
            ..HealthWarnings::default()
        };
        assert_eq!(
            compose_tooltip(keys),
            "Brightness Control – hotkeys stopped"
        );

        let both = HealthWarnings {
            ddc: DdcHealth::WorkerDead,
            hotkeys_lost: true,
            hotkeys_degraded: false,
            file_log_failed: false,
        };
        assert_eq!(
            compose_tooltip(both),
            "Brightness Control – DDC unavailable, hotkeys stopped"
        );
    }

    #[test]
    fn tooltip_names_a_failed_hotkey_change() {
        let degraded = HealthWarnings {
            hotkeys_degraded: true,
            ..HealthWarnings::default()
        };
        assert_eq!(
            compose_tooltip(degraded),
            "Brightness Control – hotkey change failed"
        );
    }

    #[test]
    fn tooltip_says_not_responding_for_a_hung_worker() {
        assert_eq!(
            compose_tooltip(ddc_only(DdcHealth::WorkerHung)),
            "Brightness Control – monitor not responding"
        );
    }

    #[test]
    fn tooltip_names_a_failed_file_log() {
        let logging = HealthWarnings {
            file_log_failed: true,
            ..HealthWarnings::default()
        };
        assert_eq!(
            compose_tooltip(logging),
            "Brightness Control – file logging off"
        );
    }

    #[test]
    fn menu_warning_lines_match_active_warnings() {
        assert!(warning_menu_lines(HealthWarnings::default()).is_empty());

        let all = HealthWarnings {
            ddc: DdcHealth::WorkerDead,
            hotkeys_lost: true,
            hotkeys_degraded: true,
            file_log_failed: true,
        };
        let lines = warning_menu_lines(all);
        assert_eq!(lines.len(), 4);
        assert!(lines[0].contains("DDC"));
        assert!(lines[1].contains("Hotkeys"));
        assert!(lines[2].contains("try another combination"));
        assert!(lines[3].contains("logging"));
    }

    #[test]
    fn only_a_dead_worker_is_advertised_as_hotkey_recoverable() {
        // A keypress clears the respawn backoff, so the advice is sound here…
        let dead = ddc_only(DdcHealth::WorkerDead);
        assert!(warning_menu_lines(dead)[0].contains("hotkey"));

        // …but nothing the user can press unsticks a blocked DDC call, so
        // offering the same retry would be a false affordance.
        let line = warning_menu_lines(ddc_only(DdcHealth::WorkerHung))[0];
        assert!(!line.contains("hotkey"), "must not promise a hotkey retry");
        assert!(line.contains("restart"));
    }

    #[test]
    fn a_failed_file_log_is_announced_where_the_user_goes_looking() {
        let logging = HealthWarnings {
            file_log_failed: true,
            ..HealthWarnings::default()
        };
        let lines = warning_menu_lines(logging);
        assert_eq!(lines.len(), 1);
        // The same menu carries "Open Log Folder" a few items down, which is
        // what someone hunting for a missing log clicks — so the line points
        // at the folder rather than explaining an I/O error.
        assert!(lines[0].contains("log folder"), "got: {}", lines[0]);
    }

    #[test]
    fn a_failed_file_log_does_not_raise_the_warning_badge() {
        // The badge means the app cannot do its job. A missing diagnostic log
        // does not stop a single brightness adjustment, and diluting the badge
        // would weaken the signal for the two conditions that do.
        let logging = HealthWarnings {
            file_log_failed: true,
            ..HealthWarnings::default()
        };
        assert!(!wants_warning_badge(logging));

        assert!(wants_warning_badge(ddc_only(DdcHealth::WorkerHung)));
        assert!(wants_warning_badge(HealthWarnings {
            hotkeys_lost: true,
            ..HealthWarnings::default()
        }));
    }

    #[test]
    fn status_bits_round_trip_every_warning_combination() {
        for ddc in [DdcHealth::Ok, DdcHealth::WorkerDead, DdcHealth::WorkerHung] {
            for hotkeys_lost in [false, true] {
                for hotkeys_degraded in [false, true] {
                    for file_log_failed in [false, true] {
                        let warnings = HealthWarnings {
                            ddc,
                            hotkeys_lost,
                            hotkeys_degraded,
                            file_log_failed,
                        };
                        assert_eq!(
                            warnings_from_bits(warnings_to_bits(warnings)),
                            warnings,
                            "lost across the window-message hop"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn usage_lines_show_the_configured_hotkeys_in_the_shortcut_column() {
        let lines = usage_menu_lines("Alt+F1", "Alt+F2");

        // The header names the mouse step; the hotkeys never appear in it.
        assert!(lines[0].contains("monitor"));
        assert!(!lines[0].contains('\t'));

        // Everything after the tab lands in the menu's shortcut column.
        assert_eq!(lines[1], "Brighter\tAlt+F1");
        assert_eq!(lines[2], "Dimmer\tAlt+F2");
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
