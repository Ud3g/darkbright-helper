//! DDC worker thread for non-blocking monitor communication.
//!
//! This module provides a dedicated worker thread that handles all DDC/CI
//! operations, keeping the main thread responsive for UI updates.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender};

use windows::Win32::Graphics::Gdi::HMONITOR;

use crate::core::state::{BrightnessMessage, DdcCommand, MonitorId};
use crate::platform::windows::ddc::{
    DdcMonitor, enumerate_monitors, get_monitor_id, get_physical_monitors,
};

/// Worker thread that handles all DDC/CI communication.
///
/// The worker owns all `DdcMonitor` instances and processes commands
/// from the main thread, sending results back via the response channel.
pub struct DdcWorker {
    /// DDC monitors indexed by their `MonitorId`.
    monitors: HashMap<MonitorId, DdcMonitor>,
    /// Maps `HMONITOR` handles to `MonitorId` for quick lookup.
    handle_cache: HashMap<isize, MonitorId>,
    /// Receiver for commands from the main thread.
    cmd_rx: Receiver<DdcCommand>,
    /// Sender for results back to the main thread.
    resp_tx: Sender<BrightnessMessage>,
}

impl DdcWorker {
    /// Creates a new DDC worker.
    ///
    /// # Arguments
    ///
    /// * `cmd_rx` - Receiver for `DdcCommand` messages from the main thread.
    /// * `resp_tx` - Sender for `BrightnessMessage` results back to the main thread.
    #[must_use]
    pub fn new(cmd_rx: Receiver<DdcCommand>, resp_tx: Sender<BrightnessMessage>) -> Self {
        Self {
            monitors: HashMap::new(),
            handle_cache: HashMap::new(),
            cmd_rx,
            resp_tx,
        }
    }

    /// Runs the worker loop, processing commands until shutdown.
    ///
    /// This method blocks until a `DdcCommand::Shutdown` is received
    /// or the command channel is disconnected.
    pub fn run(mut self) {
        log::info!("DDC worker thread started");

        loop {
            let Ok(cmd) = self.cmd_rx.recv() else {
                log::info!("DDC worker command channel disconnected, shutting down");
                break;
            };

            log::trace!(command:? = cmd; "Received command");
            match cmd {
                DdcCommand::SetBrightness {
                    monitor_id,
                    value,
                    seq,
                } => {
                    self.handle_set_brightness(&monitor_id, value, seq);
                }
                DdcCommand::RefreshAll => {
                    self.handle_refresh_all();
                }
                DdcCommand::Shutdown => {
                    log::info!("DDC worker received shutdown command");
                    break;
                }
            }
        }

        log::info!("DDC worker thread stopped");
    }

    /// Handles a `SetBrightness` command.
    fn handle_set_brightness(&mut self, monitor_id: &MonitorId, value: u8, seq: u64) {
        let result = if let Some(monitor) = self.monitors.get_mut(monitor_id) {
            monitor.set_brightness(u32::from(value))
        } else {
            log::warn!(monitor_id:% = monitor_id; "Monitor not found");
            Err(crate::BrightnessError::MonitorNotFound(
                monitor_id.to_string(),
            ))
        };

        let (success, error) = match result {
            Ok(()) => {
                log::debug!(monitor_id:% = monitor_id, brightness = value; "Set brightness");
                (true, None)
            }
            Err(e) => {
                log::error!(monitor_id:% = monitor_id, error:% = e; "Failed to set brightness");
                (false, Some(e.to_string()))
            }
        };

        // Send result back to main thread
        let msg = BrightnessMessage::DdcSetResult {
            monitor_id: monitor_id.clone(),
            value,
            seq,
            success,
            error,
        };

        if let Err(e) = self.resp_tx.send(msg) {
            log::error!(error:% = e; "Failed to send DDC result");
        }
    }

    /// Handles a `RefreshAll` command.
    ///
    /// Enumerates all monitors, reads their brightness values,
    /// and sends the results back to the main thread.
    fn handle_refresh_all(&mut self) {
        log::debug!("Refreshing all monitors");

        // Clear existing state
        self.monitors.clear();
        self.handle_cache.clear();

        let mut results: Vec<(MonitorId, u8)> = Vec::new();

        // Enumerate monitors
        let hmonitors = match enumerate_monitors() {
            Ok(h) => h,
            Err(e) => {
                log::error!(error:% = e; "Failed to enumerate monitors");
                // Send empty result
                self.send_refresh_result(results);
                return;
            }
        };

        for hmonitor in hmonitors {
            if let Err(e) = self.process_monitor(hmonitor, &mut results) {
                log::warn!(error:% = e; "Failed to process monitor");
            }
        }

        self.send_refresh_result(results);
    }

    /// Processes a single monitor during refresh.
    fn process_monitor(
        &mut self,
        hmonitor: HMONITOR,
        results: &mut Vec<(MonitorId, u8)>,
    ) -> crate::Result<()> {
        // Get monitor ID from EDID
        let monitor_id = get_monitor_id(hmonitor)?;
        self.handle_cache.insert(hmonitor.0, monitor_id.clone());

        // Get physical monitors for DDC
        let physical_monitors = get_physical_monitors(hmonitor)?;

        for p_mon in physical_monitors {
            let mut ddc_mon = DdcMonitor::new(p_mon, monitor_id.clone());

            // Read current brightness
            match ddc_mon.get_brightness() {
                Ok(brightness) => {
                    #[allow(clippy::cast_possible_truncation)]
                    let brightness_u8 = brightness as u8;

                    log::debug!(monitor_id:% = monitor_id, brightness = brightness_u8; "Read monitor brightness");

                    results.push((monitor_id.clone(), brightness_u8));
                    self.monitors.insert(monitor_id.clone(), ddc_mon);
                }
                Err(e) => {
                    log::warn!(monitor_id:% = monitor_id, error:% = e; "Could not read brightness");
                }
            }
        }

        Ok(())
    }

    /// Sends refresh results back to the main thread.
    fn send_refresh_result(&self, monitors: Vec<(MonitorId, u8)>) {
        let msg = BrightnessMessage::DdcRefreshResult { monitors };

        if let Err(e) = self.resp_tx.send(msg) {
            log::error!(error:% = e; "Failed to send refresh result");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn test_ddc_worker_creation() {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (resp_tx, _resp_rx) = mpsc::channel();

        let worker = DdcWorker::new(cmd_rx, resp_tx);

        // Worker should start with empty state
        assert!(worker.monitors.is_empty());
        assert!(worker.handle_cache.is_empty());

        // Clean up
        drop(cmd_tx);
    }

    #[test]
    fn test_ddc_worker_shutdown() {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (resp_tx, _resp_rx) = mpsc::channel();

        let worker = DdcWorker::new(cmd_rx, resp_tx);

        // Send shutdown command
        cmd_tx.send(DdcCommand::Shutdown).unwrap();

        // Worker should exit cleanly
        worker.run();
    }

    #[test]
    fn test_ddc_worker_channel_disconnect() {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (resp_tx, _resp_rx) = mpsc::channel();

        let worker = DdcWorker::new(cmd_rx, resp_tx);

        // Drop sender to disconnect channel
        drop(cmd_tx);

        // Worker should exit cleanly on disconnect
        worker.run();
    }
}
