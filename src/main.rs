#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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

use darkbright_helper::core::config::{Config, ConfigLoadOutcome};
use darkbright_helper::core::controller::Controller;
use darkbright_helper::core::state::BrightnessMessage;
use darkbright_helper::platform::windows::CursorLocator;
use darkbright_helper::platform::windows::hotkey::{
    BRIGHTNESS_DOWN_ALT_ID, BRIGHTNESS_DOWN_ID, BRIGHTNESS_UP_ALT_ID, BRIGHTNESS_UP_ID,
    HotkeyManager, VK_BRIGHTNESS_DOWN, VK_BRIGHTNESS_UP, parse_hotkey,
};
use darkbright_helper::platform::windows::osd::OsdWindow;
use darkbright_helper::platform::windows::overlay::OverlayManager;
use darkbright_helper::platform::windows::single_instance::{self, InstanceLock, SingleInstance};
use darkbright_helper::platform::windows::{
    DdcSupervisor, PowerEventListener, TrayIcon, UsageWindow,
};
use darkbright_helper::platform::windows::{show_error_message_box, show_info_message_box};
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
        log::debug!(path:% = path.display(); "ShellExecuteW failed for file");
        // The error ends up in error-level logs; embed only the file name,
        // since the absolute path contains the user name.
        Err(BrightnessError::config_file_open(
            path.file_name().map_or_else(
                || path.display().to_string(),
                |n| n.to_string_lossy().into_owned(),
            ),
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

/// Opens or focuses the usage instructions window (shell side effect).
fn open_usage(window: &mut Option<UsageWindow>, config: &Config) {
    if let Some(w) = window {
        if w.is_valid() {
            log::debug!("Usage window already open, bringing to front");
            w.bring_to_front();
            return;
        }
    }
    match UsageWindow::new(
        &config.hotkeys.brightness_up,
        &config.hotkeys.brightness_down,
    ) {
        Ok(w) => {
            log::info!("Usage window opened");
            *window = Some(w);
        }
        Err(e) => {
            log::error!(error:% = e; "Failed to create usage window");
        }
    }
}

/// Opens the config file in the system default editor (shell side effect).
fn open_settings() {
    log::debug!("TrayOpenSettings received");
    if let Some(path) = Config::default_path() {
        if let Err(e) = open_with_default_app(&path) {
            log::error!(error:% = e; "Failed to open config file");
        }
    } else {
        log::error!("Could not determine config file path");
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
        Some(path) if path.exists() => {
            // Absolute config paths contain the user name; log them at debug only.
            log::debug!(path:% = path.display(); "Loading configuration");
            let (cfg, outcome) = Config::load_or_recover(path);
            match outcome {
                ConfigLoadOutcome::Loaded => {
                    log::info!("Configuration loaded from file");
                }
                ConfigLoadOutcome::RecoveredFromBackup { primary_error } => {
                    log::warn!(
                        error:% = primary_error;
                        "Config file corrupt; settings recovered from backup — fix or delete config.json to stop this warning"
                    );
                }
                ConfigLoadOutcome::DefaultsSubstituted {
                    primary_error,
                    backup_error,
                } => {
                    log::error!(
                        error:% = primary_error,
                        backup_error:? = backup_error.map(|e| e.to_string());
                        "Failed to parse config and no usable backup, using defaults"
                    );
                }
            }
            cfg
        }
        Some(path) => {
            log::debug!(path:% = path.display(); "Config file not found, creating default");
            let config = Config::default();
            if let Err(e) = config.save_to(path) {
                log::warn!(error:% = e; "Failed to save default config file");
            } else {
                log::info!("Default config file created");
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

// Sequential startup wiring (DPI, config, threads, controller, main loop,
// cleanup) is inherently long and reads clearest kept in one place.
#[allow(clippy::too_many_lines)]
fn main() {
    // Declare DPI awareness before creating any windows.
    // This prevents Windows from bitmap-stretching our UI at non-100% scaling.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    init_logging();

    // Enforce a single instance per logon session before spawning any worker,
    // window, or hotkey. A second launch informs the user and exits, so it
    // leaves no duplicate tray icon, overlay, or failed hotkey registration.
    let _instance_guard: Option<SingleInstance> = match single_instance::acquire() {
        Ok(InstanceLock::Acquired(guard)) => Some(guard),
        Ok(InstanceLock::AlreadyRunning) => {
            log::info!("Another instance is already running; exiting");
            show_info_message_box(
                "Brightness Control",
                "Brightness Control is already running.",
            );
            return;
        }
        Err(e) => {
            // Fail open: an unexpected guard failure must not block the user's
            // only instance.
            log::error!(error:% = e; "Single-instance check failed; continuing without guard");
            None
        }
    };

    // Load configuration
    let config = load_config();

    // Create channels
    // Main channel for BrightnessMessage (hotkey thread -> main, DDC worker -> main)
    let (tx, rx) = mpsc::channel();

    // Spawn the supervised DDC worker.
    let supervisor = DdcSupervisor::spawn(tx.clone());
    log::info!("DDC worker thread spawned");

    // Create controller: OSD is created here (its failure path), then injected.
    let osd = match OsdWindow::new(config.osd.opacity, config.osd.timeout_ms) {
        Ok(osd) => osd,
        Err(e) => {
            log::error!(error:% = e; "Failed to create OSD window");
            return;
        }
    };
    let mut controller = Controller::new(
        config.clone(),
        osd,
        OverlayManager::default(),
        supervisor,
        CursorLocator,
        Instant::now(),
    );

    // Request initial monitor enumeration from DDC worker
    controller.handle_refresh(Instant::now());

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

    // Main Loop
    log::info!("Entering main event loop");
    let mut usage_window: Option<UsageWindow> = None;
    loop {
        // Pump Windows messages (for OSD WM_PAINT, WM_TIMER, etc.)
        pump_windows_messages();

        let now = Instant::now();
        controller.check_periodic_refresh(now);
        controller.supervise_and_watchdog(now);

        // Check for brightness messages with a short timeout
        match rx.recv_timeout(Duration::from_millis(16)) {
            Ok(msg) => {
                log::debug!(message:? = msg; "Main loop received message");
                match msg {
                    // Shell side effects stay out of the core controller.
                    BrightnessMessage::TrayOpenUsage => open_usage(&mut usage_window, &config),
                    BrightnessMessage::TrayOpenSettings => open_settings(),
                    other => match controller.handle_message(other, Instant::now()) {
                        Ok(should_continue) => {
                            if !should_continue {
                                break;
                            }
                        }
                        Err(e) => {
                            log::error!(error:% = e; "Error processing message");
                        }
                    },
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

    // Cleanup
    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), FALSE);
    }

    // Ask the DDC worker to shut down, then destroy windows.
    log::debug!("Sending shutdown command to DDC worker");
    controller.shutdown_worker();

    // Explicitly drop controller to ensure windows are destroyed before exit.
    drop(controller);

    log::info!("Brightness Control Tool Stopped");
}
