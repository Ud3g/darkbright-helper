//! Binary entry point: startup wiring and the shell side effects that go
//! with it. Everything reusable lives in the library crate — this file
//! spawns the threads, builds the controller from the platform
//! implementations, and runs the message loop.
//!
//! Release builds are linked as a Windows subsystem binary, so no console
//! window appears; debug builds keep one, which is where the log output of a
//! `cargo run` goes.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::VecDeque;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, LazyLock, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

use windows::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT, SetConsoleCtrlHandler};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    DispatchMessageW, MSG, PM_REMOVE, PeekMessageW, SW_SHOWNORMAL, TranslateMessage,
};
use windows::core::BOOL;

use darkbright_helper::core::config::{Config, ConfigLoadOutcome};
use darkbright_helper::core::controller::Controller;
use darkbright_helper::core::logfile::{LOG_FILE_NAME, LOG_MAX_BYTES, RotatingFileWriter};
use darkbright_helper::core::panic_hook;
use darkbright_helper::core::reconcile::{
    RESPAWN_MAX, RESPAWN_WINDOW, RespawnDecision, RespawnGate,
};
use darkbright_helper::core::state::{BrightnessMessage, HealthWarnings};
use darkbright_helper::core::version::version_string;
use darkbright_helper::platform::windows::CursorLocator;
use darkbright_helper::platform::windows::hotkey::{
    HotkeyCommandQueue, HotkeyPortImpl, parse_hotkey, run_hotkey_thread,
};
use darkbright_helper::platform::windows::osd::OsdWindow;
use darkbright_helper::platform::windows::overlay::OverlayManager;
use darkbright_helper::platform::windows::single_instance::{self, InstanceLock, SingleInstance};
use darkbright_helper::platform::windows::{
    DdcSupervisor, PowerEventListener, SettingsSinkImpl, TrayIcon, TrayStatusHandle,
    WindowsConfigStore,
};
use darkbright_helper::platform::windows::{show_error_message_box, show_info_message_box};
use darkbright_helper::{BrightnessError, Result};

/// Channel to the main loop, published so the console control handler can
/// reach it. `None` until the controller is wired up, and a `static` because
/// [`ctrl_handler`] is an `extern "system"` callback that can capture nothing.
static SHUTDOWN_SENDER: LazyLock<Mutex<Option<mpsc::Sender<BrightnessMessage>>>> =
    LazyLock::new(|| Mutex::new(None));

/// How long to wait for the hotkey thread to report its registration result.
/// A healthy spawn (message window + `RegisterHotKey`) takes milliseconds;
/// the bound only exists so a hung spawn cannot freeze the main loop.
const HOTKEY_START_TIMEOUT: Duration = Duration::from_secs(5);

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

    let result_value = result.0.expose_provenance();

    // ShellExecuteW returns a value > 32 on success.
    // Values <= 32 indicate various error conditions.
    if result_value > 32 {
        log::debug!(path:% = path.display(); "Opened file with default application");
        Ok(())
    } else {
        // The return value doubles as an error code for low values.
        // Failure values are always <= 32, so this never truncates; the
        // fallback just keeps the conversion infallible without an `as` cast.
        let error_code = i32::try_from(result_value).unwrap_or(i32::MAX);
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

/// Console control handler: turns Ctrl+C and Ctrl+Break into an orderly
/// shutdown message instead of letting Windows terminate the process, so the
/// tray icon and the DDC handles are released.
///
/// Returns `TRUE` only when the shutdown message was actually handed to the
/// main loop. Every other case — a different control event, or Ctrl+C before
/// [`SHUTDOWN_SENDER`] has been wired up — returns `FALSE` and lets the
/// default handler terminate the process.
///
/// # Safety
///
/// This is a Windows callback, invoked on a control-handler thread of the
/// OS's choosing. It touches only [`SHUTDOWN_SENDER`], which is
/// synchronised.
unsafe extern "system" fn ctrl_handler(ctrl_type: u32) -> BOOL {
    if ctrl_type == CTRL_C_EVENT || ctrl_type == CTRL_BREAK_EVENT {
        log::info!("Shutdown signal received");
        // Neither holder of this lock can panic while holding it, so poisoning
        // is not reachable today; recovering rather than skipping keeps the
        // one policy the process uses (see architecture.md, Thread Conventions)
        // and means a future holder that can panic does not silently turn
        // shutdown into a no-op.
        let guard = SHUTDOWN_SENDER
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(tx) = &*guard {
            let _ = tx.send(BrightnessMessage::Shutdown);
            return BOOL::from(true);
        }
    }
    BOOL::from(false)
}

/// Opens the config file in the system default editor (shell side effect).
///
/// Used by the settings dialog's "Open config file" footer link.
fn open_config_file() {
    log::debug!("OpenConfigFile received");
    if let Some(path) = Config::default_path() {
        if let Err(e) = open_with_default_app(&path) {
            log::error!(error:% = e; "Failed to open config file");
        }
    } else {
        log::error!("Could not determine config file path");
    }
}

/// Opens the app data directory (config + logs) in Explorer (shell side effect).
fn open_log_folder() {
    log::debug!("TrayOpenLogFolder received");
    if let Some(dir) = Config::default_dir() {
        // The directory may not exist yet (fresh install, logging never on).
        let _ = std::fs::create_dir_all(&dir);
        if let Err(e) = open_with_default_app(&dir) {
            log::error!(error:% = e; "Failed to open log folder");
        }
    } else {
        log::error!("Could not determine log folder path");
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
/// Invalid hotkey strings are repaired to defaults, never fatal.
fn load_config() -> Config {
    let config_path = Config::default_path();
    let mut config = match &config_path {
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
    };
    // Hotkey validity needs the platform parser, so the repair runs here
    // rather than inside core's validate_and_fix.
    config.repair_hotkeys(|s| parse_hotkey(s).is_ok());
    config
}

/// Console logger plus an optionally attached rolling-file logger.
///
/// The file half cannot exist at logger-installation time: whether it is
/// wanted, and at which level, comes from the config file — whose loading
/// itself produces log lines. The tee is therefore installed console-only and
/// the file sink attached right after config load; only the config-loading
/// lines themselves are console-only.
struct TeeLogger {
    console: env_logger::Logger,
    file: OnceLock<env_logger::Logger>,
}

impl TeeLogger {
    /// Attaches the file logger and raises the global max level to match.
    fn attach_file(&self, logger: env_logger::Logger) {
        let max = self.console.filter().max(logger.filter());
        if self.file.set(logger).is_ok() {
            log::set_max_level(max);
        }
    }
}

impl log::Log for TeeLogger {
    fn enabled(&self, metadata: &log::Metadata) -> bool {
        self.console.enabled(metadata) || self.file.get().is_some_and(|f| f.enabled(metadata))
    }

    fn log(&self, record: &log::Record) {
        // Each env_logger instance applies its own level filter internally.
        self.console.log(record);
        if let Some(file) = self.file.get() {
            file.log(record);
        }
    }

    fn flush(&self) {
        self.console.flush();
        if let Some(file) = self.file.get() {
            file.flush();
        }
    }
}

/// Initializes the logging subsystem (console immediately, file attachable).
///
/// The console half uses the `RUST_LOG` environment variable if set,
/// otherwise "debug" for debug builds and "info" for release builds.
fn init_logging() -> &'static TeeLogger {
    let default_level = if cfg!(debug_assertions) {
        "debug"
    } else {
        "info"
    };

    let console =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(default_level))
            .build();
    let max = console.filter();

    let tee: &'static TeeLogger = Box::leak(Box::new(TeeLogger {
        console,
        file: OnceLock::new(),
    }));
    if log::set_logger(tee).is_ok() {
        log::set_max_level(max);
    }

    log::info!(version = version_string(); "Brightness Control Tool Starting");
    tee
}

/// Builds the rolling-file logger according to the loaded config.
///
/// The file level comes from `logging.file_level` only — `RUST_LOG` controls
/// just the console half.
fn build_file_logger(config: &Config) -> std::io::Result<env_logger::Logger> {
    let dir = Config::default_dir()
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "APPDATA not set"))?;
    std::fs::create_dir_all(&dir)?;
    let writer = RotatingFileWriter::open(dir.join(LOG_FILE_NAME), LOG_MAX_BYTES)?;

    // validate_and_fix guarantees the level parses; fall back defensively.
    let level = config
        .logging
        .file_level
        .parse()
        .unwrap_or(log::LevelFilter::Info);

    Ok(env_logger::Builder::new()
        .filter_level(level)
        .format_timestamp_millis()
        .write_style(env_logger::WriteStyle::Never)
        .target(env_logger::Target::Pipe(Box::new(writer)))
        .build())
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
    let spawned = std::thread::Builder::new()
        .name("power".to_string())
        .spawn(move || {
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
    if let Err(e) = spawned {
        // Non-fatal, same as a listener that fails to start: no resume detection.
        log::error!(error:% = e; "Failed to spawn power listener thread");
    }
}

/// Spawns the system tray icon thread.
///
/// The tray icon provides a menu for accessing settings and quitting the application.
/// The thread runs until the application shuts down (tray icon is cleaned up via RAII).
///
/// # Arguments
///
/// * `tx` - Channel sender to notify the main thread of tray events.
/// * `status_tx` - Hands the tray's status handle back to the main thread so
///   it can push degraded-state icon/tooltip updates.
fn spawn_tray_thread(
    tx: mpsc::Sender<BrightnessMessage>,
    status_tx: mpsc::Sender<TrayStatusHandle>,
) {
    let spawned = std::thread::Builder::new()
        .name("tray".to_string())
        .spawn(move || {
            match TrayIcon::new(tx) {
                Ok(tray) => {
                    log::info!("System tray icon created");
                    if let Err(e) = status_tx.send(tray.status_handle()) {
                        // Losing this costs every later degraded-state icon and
                        // tooltip update, for the rest of the run.
                        log::error!(error:% = e; "Failed to hand the tray status handle to the main thread");
                    }
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
    if let Err(e) = spawned {
        // Non-fatal, same as a tray icon that fails to create: no tray.
        log::error!(error:% = e; "Failed to spawn tray thread");
    }
}

/// Spawns the hotkey thread (running [`run_hotkey_thread`]) and returns its
/// `JoinHandle` for liveness supervision. Blocks until the thread reports
/// its registration result.
///
/// `thread_id`/`queue` are the shared cells [`HotkeyPortImpl`] posts through;
/// the same pair is reused across every spawn (initial and every supervised
/// respawn) so a rebind posted around a respawn ordinarily either reaches
/// the thread that is actually running or fails cleanly. The one exception
/// is a spawn abandoned on [`BrightnessError::HotkeyThreadUnresponsive`]
/// (deliberately leaked rather than joined): if that thread later finishes
/// registering on its own, it still publishes into these same cells.
fn start_hotkey_thread(
    up: String,
    down: String,
    intercept: bool,
    tx: mpsc::Sender<BrightnessMessage>,
    thread_id: Arc<AtomicU32>,
    queue: HotkeyCommandQueue,
) -> Result<std::thread::JoinHandle<()>> {
    // Named for the post-registration log line below; `run_hotkey_thread`
    // takes ownership of the originals.
    let brightness_up = up.clone();
    let brightness_down = down.clone();

    // Hotkeys MUST be registered on the same thread that runs the message
    // loop, because WM_HOTKEY messages are sent to the registering thread's
    // queue — hence the bounded rendezvous below instead of just spawning
    // and moving on.
    let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();

    let handle = std::thread::Builder::new()
        .name("hotkey".to_string())
        .spawn(move || {
            run_hotkey_thread(up, down, intercept, tx, thread_id, queue, ready_tx);
        })
        .map_err(|e| BrightnessError::ThreadSpawn {
            name: "hotkey",
            source: e,
        })?;

    // Wait for the registration result, but bounded: a spawn hung inside
    // window creation or registration must not block the main loop —
    // especially not on a supervised restart. On timeout the thread is
    // abandoned (deliberate leak): if it ever wakes it either exits or its
    // held registrations make the next restart fail loudly.
    match ready_rx.recv_timeout(HOTKEY_START_TIMEOUT) {
        Ok(result) => result?,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            return Err(BrightnessError::HotkeyThreadUnresponsive);
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            return Err(BrightnessError::ChannelRecv);
        }
    }
    // One info line naming the bound combos: the first thing to check on a
    // "hotkey does nothing" field report.
    log::info!(
        brightness_up:% = brightness_up,
        brightness_down:% = brightness_down;
        "Hotkeys registered"
    );
    Ok(handle)
}

/// Starts the application: claims the single-instance guard, loads the
/// config, brings up the worker threads and the tray, then runs the
/// controller's message loop until shutdown.
///
/// Returns no error, because there is nobody to return one to: a failure
/// that leaves the app unable to work — the DDC worker, the OSD window or
/// the hotkey thread refusing to start — shows a message box and returns
/// early. A second instance is not one of those; it exits the same way but
/// reports it as information, not an error. Anything the app can run
/// degraded without is logged and carried on with instead.
// Sequential startup wiring (DPI, config, threads, controller, main loop,
// cleanup) is inherently long and reads clearest kept in one place.
#[expect(clippy::too_many_lines)]
fn main() {
    // Declare DPI awareness before creating any windows.
    // This prevents Windows from bitmap-stretching our UI at non-100% scaling.
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let tee = init_logging();

    // With the release console hidden, an unlogged panic leaves no trace;
    // record panics through the logger before the default handler runs.
    panic_hook::install();

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

    // Attach the opt-in rolling file log now that the config is known. The
    // outcome outlives this block: a failure can only be reported once the
    // controller exists, since the log is the one channel it cannot use.
    let file_log_failed = config.logging.file_enabled
        && match build_file_logger(&config) {
            Ok(file_logger) => {
                tee.attach_file(file_logger);
                // First line in the file: identify the build being diagnosed.
                log::info!(version = version_string(); "File logging enabled");
                false
            }
            Err(e) => {
                log::warn!(error:% = e; "Failed to enable file logging, continuing without it");
                true
            }
        };

    // Create channels
    // Main channel for BrightnessMessage (hotkey thread -> main, DDC worker -> main)
    let (tx, rx) = mpsc::channel();

    // Spawn the supervised DDC worker. It is the only path to the hardware, so
    // a refusal here ends startup — but it ends it with an explanation, the way
    // a refused hotkey thread does below, not by vanishing.
    let supervisor = match DdcSupervisor::spawn(tx.clone()) {
        Ok(supervisor) => supervisor,
        Err(e) => {
            log::error!(error:% = e; "Fatal error starting the DDC worker");
            show_error_message_box(
                "Brightness Control - Startup Error",
                &format!(
                    "Brightness Control could not start:

                     {e}

                     The system would not start a thread, which usually means it                      is out of resources. Close some applications, or restart the                      computer, and try again."
                ),
            );
            return;
        }
    };
    log::info!("DDC worker thread spawned");

    // Create controller: OSD is created here (its failure path), then injected.
    let osd = match OsdWindow::new(config.osd.opacity, config.osd.timeout_ms) {
        Ok(osd) => osd,
        Err(e) => {
            log::error!(error:% = e; "Failed to create OSD window");
            return;
        }
    };
    // Shared with every hotkey thread spawn (initial and every supervised
    // respawn below): thread_id is 0 until that thread signals ready, and
    // reset to 0 when it exits, so a rebind posted through HotkeyPortImpl
    // ordinarily fails cleanly instead of talking to a thread that is not
    // there. Not an absolute guarantee: a spawn abandoned after
    // HOTKEY_START_TIMEOUT is deliberately leaked rather than joined, and if
    // it later finishes registering on its own it still publishes into
    // these same cells — see start_hotkey_thread's doc comment.
    let hotkey_thread_id: Arc<AtomicU32> = Arc::new(AtomicU32::new(0));
    let hotkey_queue: HotkeyCommandQueue = Arc::new(Mutex::new(VecDeque::new()));

    let mut controller = Controller::new(
        config.clone(),
        osd,
        OverlayManager::default(),
        supervisor,
        CursorLocator,
        SettingsSinkImpl::new(tx.clone()),
        HotkeyPortImpl::new(hotkey_thread_id.clone(), hotkey_queue.clone()),
        WindowsConfigStore::new(Config::default_path()),
        Instant::now(),
    );

    // The warning above went to the console, which release builds hide — and
    // to the file log, which is precisely what failed. The tray is the only
    // channel that exists either way.
    if file_log_failed {
        controller.set_file_log_failed();
    }

    // Request initial monitor enumeration from DDC worker
    controller.handle_refresh(Instant::now());

    // Spawn power event listener thread (for sleep/resume detection)
    spawn_power_listener(tx.clone());

    // Spawn system tray icon thread; it hands back a status handle for
    // pushing degraded-state icon/tooltip updates.
    let (tray_status_tx, tray_status_rx) = mpsc::channel();
    spawn_tray_thread(tx.clone(), tray_status_tx);
    let mut tray_status: Option<TrayStatusHandle> = None;
    let mut last_warnings = HealthWarnings::default();

    // Register hotkeys and start hotkey thread

    // Register Ctrl+C handler
    *SHUTDOWN_SENDER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(tx.clone());

    unsafe {
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), true);
    }

    let mut hotkey_handle = match start_hotkey_thread(
        config.hotkeys.brightness_up.clone(),
        config.hotkeys.brightness_down.clone(),
        config.hotkeys.intercept_brightness_keys,
        tx.clone(),
        hotkey_thread_id.clone(),
        hotkey_queue.clone(),
    ) {
        Ok(handle) => handle,
        Err(e) => {
            log::error!(error:% = e; "Fatal error during hotkey registration");
            // A refused thread is not a hotkey conflict: none of the advice
            // below applies to it, and the title would misattribute the cause.
            let (title, message) = if matches!(e, BrightnessError::ThreadSpawn { .. }) {
                (
                    "Brightness Control - Startup Error",
                    format!(
                        "Brightness Control could not start:\n\n\
                     {e}\n\n\
                     The system would not start a thread, which usually means it \
                     is out of resources. Close some applications, or restart the \
                     computer, and try again."
                    ),
                )
            } else {
                let config_path = Config::default_path().map_or_else(
                    || "config file".to_string(),
                    |p| p.to_string_lossy().to_string(),
                );
                (
                    "Brightness Control - Hotkey Error",
                    format!(
                        "Failed to register hotkeys:\n\n\
                         {e}\n\n\
                         Possible solutions:\n\
                         • Close other applications that might be using these hotkeys\n\
                         • Change the hotkey configuration in:\n  {config_path}\n\
                         • Restart the application after making changes"
                    ),
                )
            };
            show_error_message_box(title, &message);
            return;
        }
    };
    // Hotkeys are the app's primary input: if their thread dies (message
    // window destroyed, GetMessageW error), restart it — with the same
    // crash-loop backoff the DDC worker gets.
    let mut hotkey_gate = RespawnGate::new(RESPAWN_WINDOW, RESPAWN_MAX);

    // Main Loop
    log::info!("Entering main event loop");
    loop {
        // Pump Windows messages (for OSD WM_PAINT, WM_TIMER, etc.)
        pump_windows_messages();

        let now = Instant::now();
        controller.check_periodic_refresh(now);
        controller.check_pending_save(now);
        controller.supervise_and_watchdog(now);

        if hotkey_handle.is_finished() {
            match hotkey_gate.on_death(now) {
                RespawnDecision::Attempt => {
                    log::error!("Hotkey thread died; attempting restart");
                    // Live bindings, not the startup config: a rebind since
                    // startup must survive a respawn instead of silently
                    // reverting to what the process launched with. The
                    // controller is the sole owner of the runtime config.
                    let (up, down, intercept) = controller.hotkey_config();
                    match start_hotkey_thread(
                        up,
                        down,
                        intercept,
                        tx.clone(),
                        hotkey_thread_id.clone(),
                        hotkey_queue.clone(),
                    ) {
                        Ok(handle) => {
                            hotkey_handle = handle;
                            controller.hotkey_thread_respawned(now);
                            log::info!("Hotkey thread restarted");
                        }
                        Err(e) => {
                            hotkey_gate.record_spawn_failure();
                            controller.set_hotkeys_lost();
                            log::error!(
                                error:% = e;
                                "Hotkey thread restart failed; hotkeys unavailable until app restart"
                            );
                        }
                    }
                }
                RespawnDecision::GaveUpNow => {
                    controller.set_hotkeys_lost();
                    log::error!(
                        "Hotkey thread died repeatedly; giving up — hotkeys unavailable until app restart"
                    );
                }
                RespawnDecision::AlreadyGaveUp => {}
            }
        }

        // Push degraded-state changes to the tray (icon + tooltip). The menu
        // itself always pulls fresh data when opened; this is the passive path.
        if tray_status.is_none()
            && let Ok(handle) = tray_status_rx.try_recv()
        {
            tray_status = Some(handle);
            // A warning may have activated before the tray came up; sync it.
            if last_warnings != HealthWarnings::default() {
                handle.notify(last_warnings);
            }
        }
        let warnings = controller.health_warnings();
        if warnings != last_warnings {
            last_warnings = warnings;
            if let Some(handle) = tray_status {
                handle.notify(warnings);
            }
        }

        // Wait for the next message, but bounded: the OSD and overlay both
        // live on this thread with no message loop of their own, so the Win32
        // queue has to be polled — a thread cannot block on both it and an
        // MPSC channel. This interval is *not* input latency; any send
        // wakes the recv immediately. It only bounds how late an unsolicited
        // message is noticed, of which the OSD's auto-hide timer is the tightest
        // (osd.timeout_ms may be configured down to 100 ms). Measured idle cost
        // is ~0.0017% of a core; see the main-loop cadence notes in
        // docs/architecture.md before changing it.
        match rx.recv_timeout(Duration::from_millis(16)) {
            Ok(msg) => {
                // An open tray menu asks for its rows four times a second. At
                // debug level that buries the lines a manual test is reading.
                if matches!(msg, BrightnessMessage::TrayMenuOpening { .. }) {
                    log::trace!(message:? = msg; "Main loop received message");
                } else {
                    log::debug!(message:? = msg; "Main loop received message");
                }
                match msg {
                    // Shell side effects stay out of the core controller.
                    BrightnessMessage::TrayOpenLogFolder => open_log_folder(),
                    BrightnessMessage::OpenConfigFile => open_config_file(),
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
        let _ = SetConsoleCtrlHandler(Some(ctrl_handler), false);
    }

    // Ask the DDC worker to shut down, then destroy windows.
    log::debug!("Sending shutdown command to DDC worker");
    controller.shutdown_worker();

    // Explicitly drop controller to ensure windows are destroyed before exit.
    drop(controller);

    log::info!("Brightness Control Tool Stopped");
}
