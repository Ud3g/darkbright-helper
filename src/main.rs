use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, mpsc};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{BOOL, FALSE, TRUE};
use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, TranslateMessage,
};

use darkbright_helper::core::brightness::calculate_adjustment;
use darkbright_helper::core::config::Config;
use darkbright_helper::core::state::{BrightnessMessage, DdcCommand, MonitorId, MonitorState};
use darkbright_helper::platform::windows::ddc::get_monitor_id;
use darkbright_helper::platform::windows::get_monitor_under_cursor;
use darkbright_helper::platform::windows::hotkey::{
    BRIGHTNESS_DOWN_ALT_ID, BRIGHTNESS_DOWN_ID, BRIGHTNESS_UP_ALT_ID, BRIGHTNESS_UP_ID,
    HotkeyManager, VK_BRIGHTNESS_DOWN, VK_BRIGHTNESS_UP, parse_hotkey,
};
use darkbright_helper::platform::windows::osd::OsdWindow;
use darkbright_helper::platform::windows::overlay::OverlayManager;
use darkbright_helper::platform::windows::show_error_message_box;
use darkbright_helper::platform::windows::DdcWorker;
use darkbright_helper::{BrightnessError, Result};

static SHUTDOWN_SENDER: LazyLock<Mutex<Option<mpsc::Sender<BrightnessMessage>>>> =
    LazyLock::new(|| Mutex::new(None));

unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> BOOL {
    if ctrl_type == CTRL_C_EVENT || ctrl_type == CTRL_BREAK_EVENT {
        log::info!("Shutdown signal received.");
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
            BrightnessMessage::DdcSetResult {
                monitor_id,
                value,
                success,
                error,
            } => {
                self.handle_ddc_set_result(&monitor_id, value, success, error)?;
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

    /// Handles the result of a DDC brightness set operation.
    ///
    /// On success, confirms the pending brightness. On failure, reverts to
    /// the cached value and shows an error indicator in the OSD.
    fn handle_ddc_set_result(
        &mut self,
        monitor_id: &MonitorId,
        value: u8,
        success: bool,
        error: Option<String>,
    ) -> Result<()> {
        let Some(state) = self.states.get_mut(monitor_id) else {
            log::warn!("Received DDC result for unknown monitor: {monitor_id}");
            return Ok(());
        };

        if success {
            state.confirm_brightness();
            log::debug!("{monitor_id}: DDC confirmed brightness at {value}%");

            // Update OSD to confirm (removes any error coloring)
            if self.osd.is_visible() {
                self.osd.update(state)?;
            }
        } else {
            let error_msg = error.as_deref().unwrap_or("unknown error");
            log::error!("{monitor_id}: DDC failed to set brightness to {value}%: {error_msg}");

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
        log::info!("DDC refresh complete, {} monitor(s) found", monitors.len());

        for (monitor_id, brightness) in monitors {
            log::debug!("{monitor_id}: brightness = {brightness}%");

            self.states
                .entry(monitor_id)
                .and_modify(|s| s.update_from_ddc(brightness))
                .or_insert_with(|| MonitorState::new(brightness));
        }

        // Update refresh timestamp and clear in-progress flag
        self.last_refresh = Instant::now();
        self.refresh_in_progress = false;
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
            log::trace!("{target_id}: no change (hw={old_hardware}%, overlay={old_overlay}%)");
            // Still show/update OSD to reset timer and provide feedback
            if self.osd.is_visible() {
                self.osd.update(state)?;
            } else {
                self.osd.show(hmonitor, state)?;
            }
            return Ok(());
        }

        log::trace!(
            "{target_id}: attempting hw {old_hardware}%→{new_hardware}%, overlay {old_overlay}%→{new_overlay}%"
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
        if new_hardware != old_hardware {
            log::debug!(
                "{target_id}: sending DDC command hw {old_hardware}%→{new_hardware}%"
            );
            if let Err(e) = self.ddc_cmd_tx.send(DdcCommand::SetBrightness {
                monitor_id: target_id,
                value: new_hardware,
            }) {
                log::error!("Failed to send DDC command: {e}");
            }
            // Confirmation/revert happens when we receive DdcSetResult
        } else {
            log::debug!(
                "{target_id}: overlay only {old_overlay}%→{new_overlay}%"
            );
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
        log::info!("Requesting monitor refresh from DDC worker...");

        // Clear ID cache since handles may change after refresh
        self.id_cache.clear();

        // Mark refresh in progress to prevent overlapping requests
        self.refresh_in_progress = true;

        // Send refresh command to worker (non-blocking)
        if let Err(e) = self.ddc_cmd_tx.send(DdcCommand::RefreshAll) {
            log::error!("Failed to send refresh command to DDC worker: {e}");
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

        let elapsed = self.last_refresh.elapsed();
        let interval = Duration::from_secs(u64::from(periodic_seconds));

        if elapsed >= interval {
            log::debug!(
                "Periodic refresh triggered ({}s since last refresh)",
                elapsed.as_secs()
            );
            self.handle_refresh();
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
            log::debug!(
                "Inactivity refresh triggered ({}s since last activity)",
                elapsed.as_secs()
            );
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

fn start_hotkey_thread(config: Config, tx: mpsc::Sender<BrightnessMessage>) -> Result<()> {
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
        let mut hotkey_manager = match HotkeyManager::new(tx, config_clone.brightness.step_percent) {
            Ok(hm) => hm,
            Err(e) => {
                let _ = result_tx.send(Err(e));
                return;
            }
        };

        // Register primary hotkeys on THIS thread (fatal if they fail)
        if let Err(e) = hotkey_manager.register_hotkey(
            BRIGHTNESS_UP_ID,
            up_hotkey.modifiers,
            up_hotkey.vk_code,
        ) {
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

        // Register secondary (opportunistic) hotkeys - non-fatal
        if let Err(e) = hotkey_manager.register_hotkey(
            BRIGHTNESS_UP_ALT_ID,
            HOT_KEY_MODIFIERS(0),
            VK_BRIGHTNESS_UP,
        ) {
            log::debug!("Secondary brightness up hotkey not registered: {e}");
        }

        if let Err(e) = hotkey_manager.register_hotkey(
            BRIGHTNESS_DOWN_ALT_ID,
            HOT_KEY_MODIFIERS(0),
            VK_BRIGHTNESS_DOWN,
        ) {
            log::debug!("Secondary brightness down hotkey not registered: {e}");
        }

        // Run message loop (blocks until thread ends)
        hotkey_manager.run_message_loop();
    });

    // Wait for registration result from hotkey thread
    result_rx
        .recv()
        .map_err(|_| BrightnessError::ChannelRecv)?
}

fn main() {
    // Phase 6, Step 41: Initialize logging
    // Default to "info" in release and "debug" in debug builds if RUST_LOG is not set.
    let default_level = if cfg!(debug_assertions) {
        "debug"
    } else {
        "info"
    };

    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_level))
        .init();

    log::info!("Brightness Control Tool Starting...");

    // Phase 6, Step 42: Load configuration
    let config = match Config::load() {
        Ok(cfg) => {
            log::info!("Configuration loaded successfully.");
            cfg
        }
        Err(e) => {
            log::error!("Failed to load configuration: {e}. Using defaults.");
            Config::default()
        }
    };

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
            log::error!("Failed to initialize BrightnessController: {e}");
            return;
        }
    };

    // Request initial monitor enumeration from DDC worker
    controller.handle_refresh();

    // Phase 6, Step 44 & 45: Register hotkeys and start hotkey thread

    // Register Ctrl+C handler
    if let Ok(mut guard) = SHUTDOWN_SENDER.lock() {
        *guard = Some(tx.clone());
    }

    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), TRUE);
    }

    if let Err(e) = start_hotkey_thread(config, tx) {
        log::error!("Fatal error during hotkey registration: {e}");
        let config_path = Config::default_path()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "config file".to_string());
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
    log::info!("Entering main event loop...");
    loop {
        // Pump Windows messages (for OSD WM_PAINT, WM_TIMER, etc.)
        pump_windows_messages();

        // Check if periodic refresh is due
        controller.check_periodic_refresh();

        // Check for brightness messages with a short timeout
        match rx.recv_timeout(Duration::from_millis(16)) {
            Ok(msg) => {
                log::debug!("Main loop received message: {msg:?}");
                match controller.handle_message(msg) {
                    Ok(should_continue) => {
                        if !should_continue {
                            break;
                        }
                    }
                    Err(e) => {
                        log::error!("Error processing message: {e}");
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                // No message received, continue pumping Windows messages
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                log::info!("Channel disconnected, shutting down.");
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

    log::info!("Brightness Control Tool Stopped.");
}
