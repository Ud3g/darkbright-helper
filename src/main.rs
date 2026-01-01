use std::collections::HashMap;
use std::sync::{LazyLock, Mutex, mpsc};

use windows::Win32::Foundation::{BOOL, FALSE, TRUE};
use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};

use darkbright_helper::core::brightness::calculate_adjustment;
use darkbright_helper::core::config::Config;
use darkbright_helper::core::state::{BrightnessMessage, MonitorId, MonitorState};
use darkbright_helper::platform::windows::ddc::{
    DdcMonitor, enumerate_monitors, get_monitor_id, get_physical_monitors,
};
use darkbright_helper::platform::windows::hotkey::{
    BRIGHTNESS_DOWN_ALT_ID, BRIGHTNESS_DOWN_ID, BRIGHTNESS_UP_ALT_ID, BRIGHTNESS_UP_ID,
    HotkeyManager, parse_hotkey, VK_BRIGHTNESS_DOWN, VK_BRIGHTNESS_UP,
};
use darkbright_helper::platform::windows::get_monitor_under_cursor;
use darkbright_helper::platform::windows::osd::OsdWindow;
use darkbright_helper::platform::windows::overlay::OverlayManager;
use darkbright_helper::{BrightnessError, Result};

static SHUTDOWN_SENDER: LazyLock<Mutex<Option<mpsc::Sender<BrightnessMessage>>>> =
    LazyLock::new(|| Mutex::new(None));

unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> BOOL {
    if ctrl_type == CTRL_C_EVENT || ctrl_type == CTRL_BREAK_EVENT {
        log::info!("Shutdown signal received.");
        if let Ok(guard) = SHUTDOWN_SENDER.lock() {
            if let Some(tx) = &*guard {
                let _ = tx.send(BrightnessMessage::Shutdown);
                return TRUE;
            }
        }
    }
    FALSE
}

/// Main controller for brightness management.
///
/// This struct manages the detected monitors, their states,
/// the dimming overlays, and the on-screen display (OSD).
struct BrightnessController {
    /// List of detected DDC/CI monitors.
    monitors: Vec<DdcMonitor>,
    /// Current state (brightness, overlay) per monitor.
    states: HashMap<MonitorId, MonitorState>,
    /// Manager for the dimming overlay windows.
    overlay_manager: OverlayManager,
    /// The on-screen display for showing changes.
    osd: OsdWindow,
    /// The loaded configuration.
    #[allow(dead_code)]
    config: Config,
    /// Cache for mapping Windows handles to monitor IDs (performance optimization).
    id_cache: HashMap<isize, MonitorId>,
}

impl BrightnessController {
    /// Creates a new `BrightnessController` instance.
    ///
    /// Initializes the OSD with values from the configuration.
    /// Monitors and states are populated during application startup (Phase 6).
    fn new(config: Config) -> Result<Self> {
        let osd = OsdWindow::new(config.osd.opacity, config.osd.timeout_ms)?;

        Ok(Self {
            monitors: Vec::new(),
            states: HashMap::new(),
            overlay_manager: OverlayManager::default(),
            osd,
            config,
            id_cache: HashMap::new(),
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
                self.handle_refresh()?;
            }
            BrightnessMessage::Shutdown => {
                self.handle_shutdown()?;
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Handles the shutdown process.
    fn handle_shutdown(&mut self) -> Result<()> {
        Ok(())
    }

    /// Applies a relative brightness adjustment.
    ///
    /// Determines the target monitor (mouse position), calculates new values,
    /// shows the OSD immediately, and then performs the DDC update.
    fn handle_adjust(&mut self, monitor_id: Option<MonitorId>, delta: i8) -> Result<()> {
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

        // 2. Find state and monitor object
        let state = self
            .states
            .get_mut(&target_id)
            .ok_or_else(|| BrightnessError::MonitorNotFound(target_id.to_string()))?;

        let monitor = self
            .monitors
            .iter_mut()
            .find(|m| m.id() == &target_id)
            .ok_or_else(|| BrightnessError::MonitorNotFound(target_id.to_string()))?;

        // 3. Calculate new brightness
        let adjustment =
            calculate_adjustment(state.effective_brightness(), state.overlay_opacity, delta);

        // 4. Optimistic update
        state.set_pending(adjustment.hardware_brightness);
        let old_overlay = state.overlay_opacity;
        state.overlay_opacity = adjustment.overlay_opacity;

        // Update overlay (software layer is immediately effective)
        if state.overlay_opacity != old_overlay {
            self.overlay_manager
                .update(&target_id, hmonitor, state.overlay_opacity)?;
        }

        // Show or update OSD
        if self.osd.is_visible() {
            self.osd.update(state)?;
        } else {
            self.osd.show(hmonitor, state)?;
        }

        // 5. Hardware update via DDC (blocking in controller thread)
        log::debug!(
            "Setting DDC brightness for {}: {}%",
            target_id,
            adjustment.hardware_brightness
        );

        match monitor.set_brightness(u32::from(adjustment.hardware_brightness)) {
            Ok(()) => {
                state.confirm_brightness();
                // Update OSD to confirm (removes error coloring if present)
                self.osd.update(state)?;
            }
            Err(e) => {
                log::error!("DDC error for {target_id}: {e}");
                // 6. Error rollback
                state.revert_pending();
                state.overlay_opacity = old_overlay;

                // Revert overlay to old value
                let _ = self
                    .overlay_manager
                    .update(&target_id, hmonitor, old_overlay);

                // Set OSD to error state
                self.osd.update_error(state)?;
                return Err(e);
            }
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

    /// Refreshes the monitor list and re-reads their states.
    ///
    /// # Errors
    ///
    /// Returns an error if monitor enumeration or DDC communication fails.
    fn handle_refresh(&mut self) -> Result<()> {
        log::info!("Refreshing monitor list and brightness levels...");

        // 1. Re-enumerate physical monitors
        let hmonitors = enumerate_monitors()?;
        let mut new_monitors = Vec::new();

        // Clear ID cache since handles may have changed
        self.id_cache.clear();

        for hmonitor in hmonitors {
            // Determine MonitorId (and cache it)
            let monitor_id = get_monitor_id(hmonitor)?;
            self.id_cache.insert(hmonitor.0, monitor_id.clone());

            // Physical handles for DDC/CI
            let physicals = get_physical_monitors(hmonitor)?;

            for p_mon in physicals {
                let mut ddc_mon = DdcMonitor::new(p_mon, monitor_id.clone());

                // Read current brightness via DDC
                match ddc_mon.get_brightness() {
                    Ok(val) => {
                        #[allow(clippy::cast_possible_truncation)]
                        let val_u8 = val as u8;
                        // Update or create state
                        self.states
                            .entry(monitor_id.clone())
                            .and_modify(|s| s.update_from_ddc(val_u8))
                            .or_insert_with(|| MonitorState::new(val_u8));

                        new_monitors.push(ddc_mon);
                    }
                    Err(e) => log::warn!("Could not read brightness for {monitor_id}: {e}"),
                }
            }
        }

        self.monitors = new_monitors;
        Ok(())
    }
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
            log::error!("Failed to load configuration: {}. Using defaults.", e);
            Config::default()
        }
    };

    // Phase 6, Step 43: Enumerate monitors and initialize state
    let mut controller = match BrightnessController::new(config.clone()) {
        Ok(c) => c,
        Err(e) => {
            log::error!("Failed to initialize BrightnessController: {}", e);
            return;
        }
    };

    if let Err(e) = controller.handle_refresh() {
        log::error!("Initial monitor enumeration failed: {}", e);
    }

    // Phase 6, Step 44 & 45: Register hotkeys and start hotkey thread
    let (tx, rx) = mpsc::channel();

    // Register Ctrl+C handler
    if let Ok(mut guard) = SHUTDOWN_SENDER.lock() {
        *guard = Some(tx.clone());
    }

    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), TRUE);
    }

    let hotkey_config = config.clone();

    std::thread::spawn(move || {
        let mut hotkey_manager = match HotkeyManager::new(tx, hotkey_config.brightness.step_percent)
        {
            Ok(hm) => hm,
            Err(e) => {
                log::error!("Failed to initialize HotkeyManager: {}", e);
                return;
            }
        };

        // Register primary hotkeys from config
        match parse_hotkey(&hotkey_config.hotkeys.brightness_up) {
            Ok(p) => {
                if let Err(e) =
                    hotkey_manager.register_hotkey(BRIGHTNESS_UP_ID, p.modifiers, p.vk_code)
                {
                    log::error!("Failed to register primary brightness up hotkey: {}", e);
                }
            }
            Err(e) => log::error!("Invalid brightness_up hotkey in config: {}", e),
        }

        match parse_hotkey(&hotkey_config.hotkeys.brightness_down) {
            Ok(p) => {
                if let Err(e) =
                    hotkey_manager.register_hotkey(BRIGHTNESS_DOWN_ID, p.modifiers, p.vk_code)
                {
                    log::error!("Failed to register primary brightness down hotkey: {}", e);
                }
            }
            Err(e) => log::error!("Invalid brightness_down hotkey in config: {}", e),
        }

        // Register secondary (opportunistic) hotkeys
        use windows::Win32::UI::Input::KeyboardAndMouse::HOT_KEY_MODIFIERS;

        if let Err(e) = hotkey_manager.register_hotkey(
            BRIGHTNESS_UP_ALT_ID,
            HOT_KEY_MODIFIERS(0),
            VK_BRIGHTNESS_UP,
        ) {
            log::debug!("Secondary brightness up hotkey not registered: {}", e);
        }

        if let Err(e) = hotkey_manager.register_hotkey(
            BRIGHTNESS_DOWN_ALT_ID,
            HOT_KEY_MODIFIERS(0),
            VK_BRIGHTNESS_DOWN,
        ) {
            log::debug!("Secondary brightness down hotkey not registered: {}", e);
        }

        // Run message loop (blocks until thread ends)
        hotkey_manager.run_message_loop();
    });

    // Phase 6, Step 46: Main Loop
    log::info!("Entering main event loop...");
    for msg in rx {
        match controller.handle_message(msg) {
            Ok(should_continue) => {
                if !should_continue {
                    break;
                }
            }
            Err(e) => {
                log::error!("Error processing message: {}", e);
            }
        }
    }

    // Phase 6, Step 48: Cleanup
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), FALSE);
    }

    // Explicitly drop controller to ensure windows are destroyed before exit
    drop(controller);

    log::info!("Brightness Control Tool Stopped.");
}
