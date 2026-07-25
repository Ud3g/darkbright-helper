//! Implementation of the dimming overlay window for Windows.

use std::collections::HashMap;
use std::sync::OnceLock;

use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BLACK_BRUSH, GetMonitorInfoW, GetStockObject, HBRUSH, HMONITOR, MONITORINFO,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, HWND_TOPMOST,
    LWA_ALPHA, RegisterClassExW, SW_HIDE, SW_SHOW, SWP_NOACTIVATE, SetLayeredWindowAttributes,
    SetWindowPos, ShowWindow, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TOPMOST,
    WS_EX_TRANSPARENT, WS_POPUP,
};
use windows::core::{PCWSTR, w};

use super::{SafeHwnd, hmonitor_from_isize, last_error_as_brightness_error};
use crate::core::state::MonitorId;
use crate::error::{BrightnessError, Result};

/// The class name for the overlay window.
const OVERLAY_CLASS_NAME: PCWSTR = w!("DarkBrightOverlayClass");

/// Ensures the window class is registered exactly once.
static REGISTER_CLASS_ONCE: OnceLock<Result<()>> = OnceLock::new();

/// Window procedure for the overlay window.
///
/// Since the overlay is a passive visual element (click-through),
/// we delegate almost everything to the default window procedure.
unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Registers the window class for the overlay window if not already registered.
///
/// # Returns
///
/// Returns the class name `PCWSTR` on success.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if `GetModuleHandleW` or `RegisterClassExW` fails.
pub fn ensure_overlay_class_registered() -> Result<PCWSTR> {
    // Initialize the registration once.
    REGISTER_CLASS_ONCE
        .get_or_init(|| {
            unsafe {
                let hinstance = GetModuleHandleW(None).map_err(|e| {
                    BrightnessError::windows_api("GetModuleHandleW", e.code().0.cast_unsigned())
                })?;

                // Use the stock BLACK_BRUSH for the background.
                // This ensures the window is black by default, which is what we want for dimming.
                let black_brush = HBRUSH(GetStockObject(BLACK_BRUSH).0);

                let wnd_class = WNDCLASSEXW {
                    cbSize: u32::try_from(std::mem::size_of::<WNDCLASSEXW>()).unwrap_or(0),
                    style: CS_HREDRAW | CS_VREDRAW,
                    lpfnWndProc: Some(wnd_proc),
                    hInstance: hinstance.into(),
                    hCursor: windows::Win32::UI::WindowsAndMessaging::HCURSOR::default(),
                    hbrBackground: black_brush,
                    lpszClassName: OVERLAY_CLASS_NAME,
                    ..Default::default()
                };

                if RegisterClassExW(&raw const wnd_class) == 0 {
                    return Err(last_error_as_brightness_error("RegisterClassExW"));
                }
            }
            Ok(())
        })
        .as_ref()
        .map_err(|_| {
            // Since we cannot move out of the OnceLock reference and BrightnessError
            // does not implement Clone, we reconstruct a representative error.
            // If initialization failed once, it will continue to fail.
            BrightnessError::windows_api("ensure_overlay_class_registered", 0)
        })?;

    Ok(OVERLAY_CLASS_NAME)
}

/// Creates a new overlay window.
///
/// The window is created with the following attributes:
/// - Layered (for opacity control)
/// - Transparent (click-through)
/// - Tool window (hidden from taskbar)
/// - Topmost (always on top)
/// - Popup (no border/caption)
///
/// The window is initially hidden and has 0 size.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if `CreateWindowExW` or class registration fails.
pub fn create_overlay_window() -> Result<SafeHwnd> {
    let class_name = ensure_overlay_class_registered()?;

    unsafe {
        // WS_EX_LAYERED: Allows transparency/opacity
        // WS_EX_TRANSPARENT: Click-through (events pass to window underneath)
        // WS_EX_TOOLWINDOW: Hides from taskbar and Alt-Tab
        // WS_EX_TOPMOST: Keeps window above others
        let ex_style = WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_TOPMOST;

        // WS_POPUP: No border, title bar, etc.
        let style = WS_POPUP;

        let hwnd = CreateWindowExW(
            ex_style,
            class_name,
            w!("DarkBrightOverlay"), // Window title (not visible)
            style,
            CW_USEDEFAULT, // x
            CW_USEDEFAULT, // y
            0,             // width (will be resized later)
            0,             // height
            None,          // Parent
            None,          // Menu
            Some(GetModuleHandleW(None).unwrap_or_default().into()),
            None, // lpParam
        )
        .map_err(|e| BrightnessError::windows_api("CreateWindowExW", e.code().0.cast_unsigned()))?;

        // Wrap in SafeHwnd for automatic cleanup
        Ok(SafeHwnd::new_owned(hwnd))
    }
}

/// Positions the overlay window to cover the specified monitor.
///
/// This function moves and resizes the window to match the monitor's bounds
/// and ensures it is topmost.
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if `GetMonitorInfoW` or `SetWindowPos` fails.
pub fn position_window_fullscreen(hwnd: HWND, hmonitor: HMONITOR) -> Result<()> {
    let mut mi = MONITORINFO {
        cbSize: u32::try_from(std::mem::size_of::<MONITORINFO>()).unwrap_or(0),
        ..Default::default()
    };

    unsafe {
        if !GetMonitorInfoW(hmonitor, &raw mut mi).as_bool() {
            return Err(last_error_as_brightness_error("GetMonitorInfoW"));
        }

        let rect = mi.rcMonitor;
        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            rect.left,
            rect.top,
            width,
            height,
            SWP_NOACTIVATE,
        )
        .map_err(|e| BrightnessError::windows_api("SetWindowPos", e.code().0.cast_unsigned()))?;
    }

    Ok(())
}

/// A single dimming overlay window covering one monitor.
///
/// The current opacity is not tracked here: `MonitorState.overlay_opacity`
/// in core is the single source of truth; this type only drives the window.
pub struct WindowsOverlay {
    hwnd: SafeHwnd,
    visible: bool,
}

impl WindowsOverlay {
    /// Creates a new overlay window for the specified monitor.
    ///
    /// # Errors
    ///
    /// Returns an error if window creation, positioning, or opacity setting fails.
    pub fn new(hmonitor: HMONITOR) -> Result<Self> {
        let hwnd = create_overlay_window()?;
        position_window_fullscreen(hwnd.as_raw(), hmonitor)?;

        // Initialize as transparent
        set_window_opacity(hwnd.as_raw(), 0.0)?;

        Ok(Self {
            hwnd,
            visible: false,
        })
    }

    /// Sets the overlay opacity (0.0 = invisible, 1.0 = fully black).
    ///
    /// # Errors
    ///
    /// Returns an error if the window attribute cannot be set.
    fn set_opacity(&mut self, opacity: f32) -> Result<()> {
        set_window_opacity(self.hwnd.as_raw(), opacity)
    }

    /// Shows the overlay window.
    ///
    /// # Errors
    ///
    /// Currently infallible; returns `Result` for call-site consistency.
    fn show(&mut self) -> Result<()> {
        show_window(self.hwnd.as_raw())?;
        self.visible = true;
        Ok(())
    }

    /// Hides the overlay window.
    ///
    /// # Errors
    ///
    /// Currently infallible; returns `Result` for call-site consistency.
    fn hide(&mut self) -> Result<()> {
        hide_window(self.hwnd.as_raw())?;
        self.visible = false;
        Ok(())
    }

    /// Returns true if the overlay window is currently shown.
    fn is_visible(&self) -> bool {
        self.visible
    }
}

/// Shows the overlay window.
///
/// # Errors
///
/// This method is currently infallible but returns `Result` for consistency.
pub fn show_window(hwnd: HWND) -> Result<()> {
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOW);
    }
    Ok(())
}

/// Hides the overlay window.
///
/// # Errors
///
/// This method is currently infallible but returns `Result` for consistency.
pub fn hide_window(hwnd: HWND) -> Result<()> {
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
    Ok(())
}

/// Sets the opacity of the overlay window.
///
/// # Arguments
///
/// * `hwnd` - The window handle.
/// * `opacity` - Opacity value from 0.0 (invisible) to 1.0 (fully opaque).
///
/// # Errors
///
/// Returns `BrightnessError::WindowsApi` if `SetLayeredWindowAttributes` fails.
pub fn set_window_opacity(hwnd: HWND, opacity: f32) -> Result<()> {
    // Clamp opacity to 0.0 - 1.0
    let opacity = opacity.clamp(0.0, 1.0);

    // Convert to 0-255 safely
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let alpha = (opacity * 255.0).round() as u8;

    unsafe {
        SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA).map_err(|e| {
            BrightnessError::windows_api("SetLayeredWindowAttributes", e.code().0.cast_unsigned())
        })?;
    }

    Ok(())
}

/// Manages overlay windows for multiple monitors.
pub struct OverlayManager {
    overlays: HashMap<MonitorId, WindowsOverlay>,
}

impl Default for OverlayManager {
    fn default() -> Self {
        Self::new()
    }
}

impl OverlayManager {
    /// Creates a new overlay manager.
    #[must_use]
    pub fn new() -> Self {
        Self {
            overlays: HashMap::new(),
        }
    }

    /// Updates the overlay for a specific monitor.
    ///
    /// # Arguments
    ///
    /// * `monitor_id` - The unique identifier of the monitor.
    /// * `hmonitor` - The Windows monitor handle (used for positioning).
    /// * `opacity` - The desired opacity (0-100).
    ///
    /// # Errors
    ///
    /// Returns an error if window creation, positioning, or opacity setting fails.
    ///
    /// # Panics
    ///
    /// This method may panic if the internal state is inconsistent during insertion or retrieval.
    pub fn update(
        &mut self,
        monitor_id: &MonitorId,
        hmonitor: HMONITOR,
        opacity: u8,
    ) -> Result<()> {
        // If opacity is 0, we just need to hide it if it exists.
        if opacity == 0 {
            if let Some(overlay) = self.overlays.get_mut(monitor_id) {
                overlay.hide()?;
            }
            return Ok(());
        }

        // Create overlay if it doesn't exist
        if !self.overlays.contains_key(monitor_id) {
            let overlay = WindowsOverlay::new(hmonitor)?;
            self.overlays.insert(monitor_id.clone(), overlay);
        }

        let overlay = self.overlays.get_mut(monitor_id).expect("Just inserted");

        // Ensure correct position (topology might have changed)
        position_window_fullscreen(overlay.hwnd.as_raw(), hmonitor)?;

        // Update opacity and visibility
        overlay.set_opacity(f32::from(opacity) / 100.0)?;
        if !overlay.is_visible() {
            overlay.show()?;
        }

        Ok(())
    }

    /// Removes and destroys a monitor's overlay window, if one exists.
    ///
    /// Dropping the overlay destroys its window via the RAII handle; used
    /// when a monitor is pruned so the orphaned fullscreen window cannot
    /// migrate onto a surviving monitor.
    pub fn remove(&mut self, monitor_id: &MonitorId) {
        if self.overlays.remove(monitor_id).is_some() {
            log::debug!(monitor_id:% = monitor_id; "Overlay removed");
        }
    }
}

impl crate::core::controller::OverlaySink for OverlayManager {
    fn update(
        &mut self,
        id: &MonitorId,
        handle: crate::core::controller::MonitorHandle,
        opacity: u8,
    ) -> Result<()> {
        OverlayManager::update(self, id, hmonitor_from_isize(handle.0), opacity)
    }
    fn remove(&mut self, id: &MonitorId) {
        OverlayManager::remove(self, id);
    }
}
