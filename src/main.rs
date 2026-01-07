#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, mpsc};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{BOOL, FALSE, TRUE};
use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, SW_SHOWNORMAL, TranslateMessage,
};

use darkbright_helper::core::brightness::calculate_adjustment;
use darkbright_helper::core::config::Config;
use darkbright_helper::core::state::{
    BrightnessMessage, DdcCommand, MonitorId, MonitorState, TrayMenuData, TrayMonitorInfo,
    generate_display_names,
};
use darkbright_helper::platform::windows::ddc::get_monitor_id;
use darkbright_helper::platform::windows::get_monitor_under_cursor;
use darkbright_helper::platform::windows::hotkey::{
    BRIGHTNESS_DOWN_ALT_ID, BRIGHTNESS_DOWN_ID, BRIGHTNESS_UP_ALT_ID, BRIGHTNESS_UP_ID,
    HotkeyManager, VK_BRIGHTNESS_DOWN, VK_BRIGHTNESS_UP, parse_hotkey,
};
use darkbright_helper::platform::windows::osd::OsdWindow;
use darkbright_helper::platform::windows::overlay::OverlayManager;
use darkbright_helper::platform::windows::show_error_message_box;
use darkbright_helper::platform::windows::{DdcWorker, PowerEventListener, TrayIcon, UsageWindow};
use darkbright_helper::{BrightnessError, Result};

static SHUTDOWN_SENDER: LazyLock<Mutex<Option<mpsc::Sender<BrightnessMessage>>>> =
    LazyLock::new(|| Mutex::new(None));

/// Opens a file with the system's default application.
///
/// Uses `ShellExecuteW` with the "open" verb to launch the default handler
/// for the file type (e.g., Notepad or VS Code for `.json` files).
///
/// # Arguments
///
/// * `path` - Path to the file to open.
///
/// # Errors
///
/// Returns `BrightnessError::ConfigFileOpen` if the shell operation fails.
fn open_with_default_app(path: &std::path::Path) -> Result<()> {
    use windows::core::w;

    let path_wide: Vec<u16> = path
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    let result = unsafe {
        ShellExecuteW(
            None,
            w!("open"),
            windows::core::PCWSTR(path_wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };

    // ShellExecuteW returns a value > 32 on success.
    // Values <= 32 indicate various error conditions.
    if result.0 as isize > 32 {
        log::debug!(path:% = path.display(); "Opened file with default application");
        Ok(())
    } else {
        // The return value can be interpreted as an error code for low values
        // ShellExecuteW error codes fit in i32; truncation is safe here
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let error_code = result.0 as i32;
        Err(BrightnessError::config_file_open(
            path.display().to_string(),
            std::io::Error::from_raw_os_error(error_code),
        ))
    }
}

unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> BOOL {
    if ctrl_type == CTRL_C_EVENT || ctrl_type == CTRL_BREAK_EVENT {
        log::info!("Shutdown signal received");
        if let Ok(guard) = SHUTDOWN_SENDER.lock()
            && let Some(tx) = &*guard
        {
            let _ = tx.send(BrightnessMessage::Shutdown);
            return TRUE;
        }
    }
    FALSE
}

/// Main controller for brightness management.
///
/// This struct manages monitor states, dimming overlays, and the OSD.
/// DDC/CI communication is delegated to the DDC worker thread.
struct BrightnessController {
    /// Current state (brightness, overlay) per monitor.
    states: HashMap<MonitorId, MonitorState>,
    /// Manager for the dimming overlay windows.
    overlay_manager: OverlayManager,
    /// The on-screen display for showing changes.
    osd: OsdWindow,
    /// The loaded configuration.
    config: Config,
    /// Cache for mapping Windows handles to monitor IDs (performance optimization).
    id_cache: HashMap<isize, MonitorId>,
    /// Channel to send commands to the DDC worker thread.
    ddc_cmd_tx: mpsc::Sender<DdcCommand>,
    /// Timestamp of last user-initiated brightness adjustment.
    last_activity: Instant,
    /// Timestamp of last completed DDC refresh.
    last_refresh: Instant,
    /// Flag to prevent overlapping refresh requests.
    refresh_in_progress: bool,
    /// Whether the last refresh found any monitors.
    last_refresh_successful: bool,
    /// Handle to the currently open usage window (if any).
    usage_window: Option<UsageWindow>,
}

impl BrightnessController {
    /// Creates a new `BrightnessController` instance.
    ///
    /// # Arguments
    ///
    /// * `config` - The loaded configuration.
    /// * `ddc_cmd_tx` - Channel sender for DDC worker commands.
    ///
    /// # Errors
    ///
    /// Returns an error if the OSD window cannot be created.
    fn new(config: Config, ddc_cmd_tx: mpsc::Sender<DdcCommand>) -> Result<Self> {
        let osd = OsdWindow::new(config.osd.opacity, config.osd.timeout_ms)?;

        let now = Instant::now();
        Ok(Self {
            states: HashMap::new(),
            overlay_manager: OverlayManager::default(),
            osd,
            config,
            id_cache: HashMap::new(),
            ddc_cmd_tx,
            last_activity: now,
            last_refresh: now,
            refresh_in_progress: false,
            last_refresh_successful: true,
            usage_window: None,
        })
    }

    /// Processes a brightness control message.
    ///
    /// Returns `Ok(true)` if the application should continue running,
    /// or `Ok(false)` if shutdown was requested.
    ///
    /// # Errors
    ///
    /// Returns an error if message processing fails.
    fn handle_message(&mut self, message: BrightnessMessage) -> Result<bool> {
        match message {
            BrightnessMessage::Adjust { monitor_id, delta } => {
                self.handle_adjust(monitor_id, delta)?;
            }
            BrightnessMessage::SetAbsolute { monitor_id, value } => {
                self.handle_set_absolute(monitor_id, value)?;
            }
            BrightnessMessage::Refresh => {
                self.handle_refresh();
            }
            BrightnessMessage::SystemResumed => {
                log::info!(reason = "system_resume"; "Triggering refresh");
                self.handle_refresh();
            }
            // ── Tray Icon Messages ───────────────────────────────────────
            BrightnessMessage::TrayOpenUsage => {
                self.handle_open_usage();
            }
            BrightnessMessage::TrayOpenSettings => {
                log::debug!("TrayOpenSettings received");
                if let Some(path) = Config::default_path() {
                    if let Err(e) = open_with_default_app(&path) {
                        log::error!(error:% = e; "Failed to open config file");
                    }
                } else {
                    log::error!("Could not determine config file path");
                }
            }
            BrightnessMessage::TrayRequestQuit => {
                log::info!("Quit requested from tray menu");
                self.handle_shutdown()?;
                return Ok(false);
            }
            BrightnessMessage::TrayMenuOpening { reply_tx } => {
                log::debug!("TrayMenuOpening received");
                let menu_data = self.build_tray_menu_data();
                if let Err(e) = reply_tx.send(menu_data) {
                    log::warn!(error:? = e; "Failed to send tray menu data");
                }
            }
            BrightnessMessage::DdcSetResult {
                monitor_id,
                value,
                success,
                error,
            } => {
                self.handle_ddc_set_result(&monitor_id, value, success, error.as_deref())?;
            }
            BrightnessMessage::DdcRefreshResult { monitors } => {
                self.handle_ddc_refresh_result(monitors);
            }
            BrightnessMessage::Shutdown => {
                self.handle_shutdown()?;
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Handles the shutdown process.
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    fn handle_shutdown(&mut self) -> Result<()> {
        Ok(())
    }

    /// Opens or focuses the usage instructions window.
    ///
    /// If a usage window is already open and valid, it is brought to the front.
    /// Otherwise, a new window is created with the configured hotkey information.
    fn handle_open_usage(&mut self) {
        // Check if we already have a valid usage window
        if let Some(ref window) = self.usage_window {
            if window.is_valid() {
                log::debug!("Usage window already open, bringing to front");
                window.bring_to_front();
                return;
            }
        }

        // Create a new usage window
        match UsageWindow::new(
            &self.config.hotkeys.brightness_up,
            &self.config.hotkeys.brightness_down,
        ) {
            Ok(window) => {
                log::info!("Usage window opened");
                self.usage_window = Some(window);
            }
            Err(e) => {
                log::error!(error:% = e; "Failed to create usage window");
            }
        }
    }

    /// Handles the result of a DDC brightness set operation.
    ///
    /// On success, confirms the pending brightness. On failure, reverts to
    /// the cached value and shows an error indicator in the OSD.
    fn handle_ddc_set_result(
        &mut self,
        monitor_id: &MonitorId,
        value: u8,
        success: bool,
        error: Option<&str>,
    ) -> Result<()> {
        let Some(state) = self.states.get_mut(monitor_id) else {
            log::warn!(monitor_id:% = monitor_id; "Received DDC result for unknown monitor");
            return Ok(());
        };

        if success {
            state.confirm_brightness();
            log::debug!(monitor_id:% = monitor_id, brightness = value; "DDC confirmed brightness");

            // Update OSD to confirm (removes any error coloring)
            if self.osd.is_visible() {
                self.osd.update(state)?;
            }
        } else {
            let error_msg = error.unwrap_or("unknown error");
            log::error!(monitor_id:% = monitor_id, target_brightness = value, error = error_msg; "DDC failed to set brightness");

            state.revert_pending();

            // Show OSD error state
            if self.osd.is_visible() {
                self.osd.update_error(state)?;
            }
        }

        Ok(())
    }

    /// Handles the result of a DDC refresh operation.
    ///
    /// Updates existing monitor states and creates new entries for
    /// newly detected monitors.
    fn handle_ddc_refresh_result(&mut self, monitors: Vec<(MonitorId, u8)>) {
        let found_monitors = !monitors.is_empty();

        if found_monitors {
            log::info!(count = monitors.len(); "DDC refresh complete");
        } else {
            log::warn!("DDC refresh completed with no monitors found");
        }

        for (monitor_id, brightness) in monitors {
            log::debug!(monitor_id:% = monitor_id, brightness = brightness; "Monitor found during refresh");

            self.states
                .entry(monitor_id)
                .and_modify(|s| s.update_from_ddc(brightness))
                .or_insert_with(|| MonitorState::new(brightness));
        }

        // Update refresh tracking state
        self.last_refresh = Instant::now();
        self.refresh_in_progress = false;
        self.last_refresh_successful = found_monitors;
    }

    /// Applies a relative brightness adjustment.
    ///
    /// Determines the target monitor (mouse position), calculates new values,
    /// shows the OSD immediately with optimistic update, and sends the DDC
    /// command to the worker thread (non-blocking).
    fn handle_adjust(&mut self, monitor_id: Option<MonitorId>, delta: i8) -> Result<()> {
        // Check if we need an inactivity-based refresh before processing
        // (must be checked BEFORE updating last_activity)
        self.check_inactivity_refresh();

        // If last refresh failed, trigger a new one (user activity indicates they're back)
        if !self.last_refresh_successful && !self.refresh_in_progress {
            log::debug!("Triggering refresh on user activity (last refresh found no monitors)");
            self.handle_refresh();
        }

        // Update activity timestamp for inactivity-based refresh tracking
        self.last_activity = Instant::now();

        // 1. Determine target monitor and handle
        // We need the HMONITOR handle for OSD and overlay positioning.
        let hmonitor = get_monitor_under_cursor()?;

        // If no ID was provided, identify the monitor under the cursor.
        // We use a cache to avoid slow registry accesses (EDID lookup).
        let target_id = match monitor_id {
            Some(id) => id,
            None => {
                if let Some(id) = self.id_cache.get(&hmonitor.0) {
                    id.clone()
                } else {
                    let id = get_monitor_id(hmonitor)?;
                    self.id_cache.insert(hmonitor.0, id.clone());
                    id
                }
            }
        };

        // 2. Find state for this monitor
        let state = self
            .states
            .get_mut(&target_id)
            .ok_or_else(|| BrightnessError::MonitorNotFound(target_id.to_string()))?;

        // 3. Calculate new brightness
        let old_hardware = state.effective_brightness();
        let old_overlay = state.overlay_opacity;

        let adjustment = calculate_adjustment(old_hardware, old_overlay, delta);
        let new_hardware = adjustment.hardware_brightness;
        let new_overlay = adjustment.overlay_opacity;

        let changed = (old_hardware != new_hardware) || (old_overlay != new_overlay);

        if !changed {
            log::trace!(monitor_id:% = target_id, hardware = old_hardware, overlay = old_overlay; "No brightness change needed");
            // Still show/update OSD to reset timer and provide feedback
            if self.osd.is_visible() {
                self.osd.update(state)?;
            } else {
                self.osd.show(hmonitor, state)?;
            }
            return Ok(());
        }

        log::trace!(
            monitor_id:% = target_id,
            old_hw = old_hardware,
            new_hw = new_hardware,
            old_overlay = old_overlay,
            new_overlay = new_overlay;
            "Attempting brightness adjustment"
        );

        // 4. Optimistic update (only set pending if hardware is changing)
        if new_hardware != old_hardware {
            state.set_pending(new_hardware);
        }
        state.overlay_opacity = new_overlay;

        // 5. Update overlay (software layer is immediately effective)
        if new_overlay != old_overlay {
            self.overlay_manager
                .update(&target_id, hmonitor, new_overlay)?;
        }

        // 6. Show or update OSD with optimistic values
        if self.osd.is_visible() {
            self.osd.update(state)?;
        } else {
            self.osd.show(hmonitor, state)?;
        }

        // 7. Send DDC command to worker (non-blocking)
        if new_hardware == old_hardware {
            log::debug!(monitor_id:% = target_id, old_overlay = old_overlay, new_overlay = new_overlay; "Adjusting overlay only");
        } else {
            log::debug!(monitor_id:% = target_id, old_hw = old_hardware, new_hw = new_hardware; "Sending DDC command");
            if let Err(e) = self.ddc_cmd_tx.send(DdcCommand::SetBrightness {
                monitor_id: target_id,
                value: new_hardware,
            }) {
                log::error!(error:% = e; "Failed to send DDC command");
            }
            // Confirmation/revert happens when we receive DdcSetResult
        }

        Ok(())
    }

    /// Sets an absolute brightness value for a monitor.
    ///
    /// # Arguments
    ///
    /// * `monitor_id` - Target monitor (None = monitor under cursor).
    /// * `value` - Target brightness (0-100).
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    fn handle_set_absolute(&mut self, _monitor_id: Option<MonitorId>, _value: u8) -> Result<()> {
        // Placeholder for future extensions (e.g., fixed brightness via CLI command)
        Ok(())
    }

    /// Requests a refresh of monitor list and brightness values.
    ///
    /// Sends a `RefreshAll` command to the DDC worker. The actual state
    /// update happens when `DdcRefreshResult` is received.
    fn handle_refresh(&mut self) {
        log::info!("Requesting monitor refresh from DDC worker");

        // Clear ID cache since handles may change after refresh
        self.id_cache.clear();

        // Mark refresh in progress to prevent overlapping requests
        self.refresh_in_progress = true;

        // Send refresh command to worker (non-blocking)
        if let Err(e) = self.ddc_cmd_tx.send(DdcCommand::RefreshAll) {
            log::error!(error:% = e; "Failed to send refresh command to DDC worker");
            self.refresh_in_progress = false;
        }
    }

    /// Checks if a periodic refresh is due and triggers it if needed.
    ///
    /// This is called from the main loop to keep monitor state in sync
    /// with external changes (e.g., physical monitor buttons, other apps).
    fn check_periodic_refresh(&mut self) {
        let periodic_seconds = self.config.refresh.periodic_seconds;

        // Skip if periodic refresh is disabled (0) or refresh already in progress
        if periodic_seconds == 0 || self.refresh_in_progress {
            return;
        }

        // Skip if last refresh found no monitors (will retry on user activity or resume)
        if !self.last_refresh_successful {
            log::debug!("Skipping periodic refresh (last refresh found no monitors)");
            return;
        }

        let elapsed = self.last_refresh.elapsed();
        let interval = Duration::from_secs(u64::from(periodic_seconds));

        if elapsed >= interval {
            log::debug!(elapsed_seconds = elapsed.as_secs(); "Periodic refresh triggered");
            self.handle_refresh();
        }
    }

    /// Builds the data needed to populate the tray menu.
    ///
    /// Generates display names with duplicate suffixes (e.g., "Dell U2722D #1")
    /// when multiple monitors with identical manufacturer and model are connected.
    fn build_tray_menu_data(&self) -> TrayMenuData {
        // Collect monitor IDs and generate unique display names
        let monitor_ids: Vec<MonitorId> = self.states.keys().cloned().collect();
        let display_names = generate_display_names(&monitor_ids);

        let monitors: Vec<TrayMonitorInfo> = self
            .states
            .iter()
            .map(|(monitor_id, state)| {
                let display_name = display_names
                    .get(monitor_id)
                    .cloned()
                    .unwrap_or_else(|| monitor_id.base_display_name());

                TrayMonitorInfo {
                    display_name,
                    hardware_brightness: state.effective_brightness(),
                    overlay_opacity: state.overlay_opacity,
                }
            })
            .collect();

        TrayMenuData {
            monitors,
            hotkey_up: self.config.hotkeys.brightness_up.clone(),
            hotkey_down: self.config.hotkeys.brightness_down.clone(),
        }
    }

    /// Checks if a refresh is needed due to inactivity and triggers it if so.
    ///
    /// This is called at the start of `handle_adjust()` to resync with
    /// external changes before applying a brightness adjustment after
    /// the user has been inactive for a configured duration.
    ///
    /// Uses non-blocking approach: triggers refresh but proceeds with
    /// optimistic adjustment. Values reconcile when `DdcRefreshResult` arrives.
    fn check_inactivity_refresh(&mut self) {
        let inactivity_seconds = self.config.refresh.inactivity_seconds;

        // Skip if inactivity refresh is disabled (0) or refresh already in progress
        if inactivity_seconds == 0 || self.refresh_in_progress {
            return;
        }

        let elapsed = self.last_activity.elapsed();
        let threshold = Duration::from_secs(u64::from(inactivity_seconds));

        if elapsed >= threshold {
            log::debug!(elapsed_seconds = elapsed.as_secs(); "Inactivity refresh triggered");
            self.handle_refresh();
        }
    }
}

/// Pumps all pending Windows messages for the current thread.
///
/// This is necessary because the main thread owns OSD and overlay windows
/// that need to receive `WM_PAINT` and `WM_TIMER` messages.
fn pump_windows_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&raw mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&raw const msg);
            DispatchMessageW(&raw const msg);
        }
    }
}

/// Loads the application configuration.
///
/// Attempts to load from the default path. If the file doesn't exist,
/// creates a default config file. If parsing fails, uses defaults.
fn load_config() -> Config {
    let config_path = Config::default_path();
    match &config_path {
        Some(path) if path.exists() => match Config::load_from(path) {
            Ok(cfg) => {
                log::info!(path:% = path.display(); "Configuration loaded from file");
                cfg
            }
            Err(e) => {
                log::error!(path:% = path.display(), error:% = e; "Failed to parse config, using defaults");
                Config::default()
            }
        },
        Some(path) => {
            log::info!(path:% = path.display(); "Config file not found, creating default");
            let config = Config::default();
            if let Err(e) = config.save_to(path) {
                log::warn!(path:% = path.display(), error:% = e; "Failed to save default config file");
            } else {
                log::info!(path:% = path.display(); "Default config file created");
            }
            config
        }
        None => {
            log::warn!("Could not determine config directory, using defaults");
            Config::default()
        }
    }
}

/// Initializes the logging subsystem.
///
/// Uses `RUST_LOG` environment variable if set, otherwise defaults to
/// "debug" for debug builds and "info" for release builds.
fn init_logging() {
    let default_level = if cfg!(debug_assertions) {
        "debug"
    } else {
        "info"
    };

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_level))
        .init();

    log::info!("Brightness Control Tool Starting");
}

/// Spawns the power event listener thread.
///
/// The listener detects system sleep/resume events and sends
/// `BrightnessMessage::SystemResumed` to trigger a monitor refresh.
///
/// # Arguments
///
/// * `tx` - Channel sender to notify the main thread of power events.
fn spawn_power_listener(tx: mpsc::Sender<BrightnessMessage>) {
    std::thread::spawn(move || {
        match PowerEventListener::new(tx) {
            Ok(listener) => {
                log::info!("Power event listener started");
                listener.run_message_loop();
            }
            Err(e) => {
                log::error!(error:% = e; "Failed to create power event listener");
                // Non-fatal: app works without resume detection
            }
        }
    });
}

/// Spawns the system tray icon thread.
///
/// The tray icon provides a menu for accessing settings and quitting the application.
/// The thread runs until the application shuts down (tray icon is cleaned up via RAII).
///
/// # Arguments
///
/// * `tx` - Channel sender to notify the main thread of tray events.
fn spawn_tray_thread(tx: mpsc::Sender<BrightnessMessage>) {
    std::thread::spawn(move || {
        match TrayIcon::new(tx) {
            Ok(tray) => {
                log::info!("System tray icon created");
                if let Err(e) = tray.run_message_loop() {
                    log::error!(error:% = e; "Tray message loop error");
                }
            }
            Err(e) => {
                // Non-fatal: app works without tray icon
                log::warn!(error:% = e; "Failed to create system tray icon, continuing without it");
            }
        }
    });
}

fn start_hotkey_thread(config: &Config, tx: mpsc::Sender<BrightnessMessage>) -> Result<()> {
    // Parse and validate primary hotkeys before spawning thread (fail fast on parse errors)
    let up_hotkey = parse_hotkey(&config.hotkeys.brightness_up)
        .map_err(|e| BrightnessError::config_invalid("hotkeys.brightness_up", e.to_string()))?;

    let down_hotkey = parse_hotkey(&config.hotkeys.brightness_down)
        .map_err(|e| BrightnessError::config_invalid("hotkeys.brightness_down", e.to_string()))?;

    // Channel to receive registration result from the hotkey thread.
    // Hotkeys MUST be registered on the same thread that runs the message loop,
    // because WM_HOTKEY messages are sent to the registering thread's queue.
    let (result_tx, result_rx) = mpsc::channel::<Result<()>>();

    let config_clone = config.clone();
    std::thread::spawn(move || {
        // Create hotkey manager on THIS thread (creates message window here)
        let mut hotkey_manager = match HotkeyManager::new(tx, config_clone.brightness.step_percent)
        {
            Ok(hm) => hm,
            Err(e) => {
                let _ = result_tx.send(Err(e));
                return;
            }
        };

        // Register primary hotkeys on THIS thread (fatal if they fail)
        if let Err(e) =
            hotkey_manager.register_hotkey(BRIGHTNESS_UP_ID, up_hotkey.modifiers, up_hotkey.vk_code)
        {
            let _ = result_tx.send(Err(BrightnessError::hotkey_registration(
                config_clone.hotkeys.brightness_up.clone(),
                e.to_string(),
            )));
            return;
        }

        if let Err(e) = hotkey_manager.register_hotkey(
            BRIGHTNESS_DOWN_ID,
            down_hotkey.modifiers,
            down_hotkey.vk_code,
        ) {
            let _ = result_tx.send(Err(BrightnessError::hotkey_registration(
                config_clone.hotkeys.brightness_down.clone(),
                e.to_string(),
            )));
            return;
        }

        // Signal success to main thread
        let _ = result_tx.send(Ok(()));

        // Brightness key interception: either via low-level hook or RegisterHotKey
        if config_clone.hotkeys.intercept_brightness_keys {
            // Use low-level keyboard hook to intercept brightness keys before Shell
            match hotkey_manager.install_brightness_hook() {
                Ok(()) => {
                    log::info!("Low-level keyboard hook installed for brightness keys");
                }
                Err(e) => {
                    log::warn!(error:% = e; "Failed to install brightness key hook");
                }
            }
        } else {
            log::debug!("Brightness key interception disabled by config");

            // Register secondary (opportunistic) hotkeys - non-fatal
            if let Err(e) = hotkey_manager.register_hotkey(
                BRIGHTNESS_UP_ALT_ID,
                HOT_KEY_MODIFIERS(0),
                VK_BRIGHTNESS_UP,
            ) {
                log::debug!(error:% = e; "Secondary brightness up hotkey not registered");
            }

            if let Err(e) = hotkey_manager.register_hotkey(
                BRIGHTNESS_DOWN_ALT_ID,
                HOT_KEY_MODIFIERS(0),
                VK_BRIGHTNESS_DOWN,
            ) {
                log::debug!(error:% = e; "Secondary brightness down hotkey not registered");
            }
        }

        // Run message loop (blocks until thread ends)
        hotkey_manager.run_message_loop();
    });

    // Wait for registration result from hotkey thread
    result_rx.recv().map_err(|_| BrightnessError::ChannelRecv)?
}

fn main() {
    // Declare DPI awareness before creating any windows.
    // This prevents Windows from bitmap-stretching our UI at non-100% scaling.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    init_logging();

    // Load configuration
    let config = load_config();

    // Phase 6, Step 43: Create channels
    // Main channel for BrightnessMessage (hotkey thread -> main, DDC worker -> main)
    let (tx, rx) = mpsc::channel();

    // DDC command channel (main -> DDC worker)
    let (ddc_cmd_tx, ddc_cmd_rx) = mpsc::channel::<DdcCommand>();

    // Keep a clone for sending shutdown command on exit
    let ddc_shutdown_tx = ddc_cmd_tx.clone();

    // Spawn DDC worker thread
    let ddc_worker = DdcWorker::new(ddc_cmd_rx, tx.clone());
    std::thread::spawn(move || ddc_worker.run());
    log::info!("DDC worker thread spawned");

    // Create controller
    let mut controller = match BrightnessController::new(config.clone(), ddc_cmd_tx) {
        Ok(c) => c,
        Err(e) => {
            log::error!(error:% = e; "Failed to initialize BrightnessController");
            return;
        }
    };

    // Request initial monitor enumeration from DDC worker
    controller.handle_refresh();

    // Spawn power event listener thread (for sleep/resume detection)
    spawn_power_listener(tx.clone());

    // Spawn system tray icon thread
    spawn_tray_thread(tx.clone());

    // Register hotkeys and start hotkey thread

    // Register Ctrl+C handler
    if let Ok(mut guard) = SHUTDOWN_SENDER.lock() {
        *guard = Some(tx.clone());
    }

    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), TRUE);
    }

    if let Err(e) = start_hotkey_thread(&config, tx) {
        log::error!(error:% = e; "Fatal error during hotkey registration");
        let config_path = Config::default_path().map_or_else(
            || "config file".to_string(),
            |p| p.to_string_lossy().to_string(),
        );
        let message = format!(
            "Failed to register hotkeys:\n\n\
             {e}\n\n\
             Possible solutions:\n\
             • Close other applications that might be using these hotkeys\n\
             • Change the hotkey configuration in:\n  {config_path}\n\
             • Restart the application after making changes"
        );
        show_error_message_box("Brightness Control - Hotkey Error", &message);
        return;
    }

    // Phase 6, Step 46: Main Loop
    log::info!("Entering main event loop");
    loop {
        // Pump Windows messages (for OSD WM_PAINT, WM_TIMER, etc.)
        pump_windows_messages();

        // Check if periodic refresh is due
        controller.check_periodic_refresh();

        // Check for brightness messages with a short timeout
        match rx.recv_timeout(Duration::from_millis(16)) {
            Ok(msg) => {
                log::debug!(message:? = msg; "Main loop received message");
                match controller.handle_message(msg) {
                    Ok(should_continue) => {
                        if !should_continue {
                            break;
                        }
                    }
                    Err(e) => {
                        log::error!(error:% = e; "Error processing message");
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No message received, continue pumping Windows messages
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::info!("Channel disconnected, shutting down");
                break;
            }
        }
    }

    // Phase 6, Step 48: Cleanup
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), FALSE);
    }

    // Send shutdown command to DDC worker
    log::debug!("Sending shutdown command to DDC worker");
    let _ = ddc_shutdown_tx.send(DdcCommand::Shutdown);

    // Explicitly drop controller to ensure windows are destroyed before exit
    drop(controller);

    log::info!("Brightness Control Tool Stopped");
}
