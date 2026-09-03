//! DDC worker thread for non-blocking monitor communication.
//!
//! This module provides a dedicated worker thread that handles all DDC/CI
//! operations, keeping the main thread responsive for UI updates.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, SendError, Sender};
use std::thread::JoinHandle;
use std::time::Instant;

use windows::Win32::Graphics::Gdi::HMONITOR;

use crate::core::reconcile::{
    RESPAWN_MAX, RESPAWN_WINDOW, RespawnDecision, RespawnGate, RespawnOutcome,
};
use crate::core::state::{BrightnessMessage, DdcCommand, MonitorId};
use crate::platform::windows::ddc::{
    DdcMonitor, enumerate_monitors, get_monitor_id, get_physical_monitors,
};

/// Worker thread that handles all DDC/CI communication.
///
/// The worker owns all `DdcMonitor` instances and processes commands
/// from the main thread, sending results back via the response channel.
pub(crate) struct DdcWorker {
    /// DDC monitors indexed by their `MonitorId`.
    ///
    /// Commands address monitors by identity, never by platform handle, so the
    /// worker needs no handle→id mapping of its own; resolving a cursor
    /// position to an identity happens on the main thread.
    monitors: HashMap<MonitorId, DdcMonitor>,
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
    pub(crate) fn new(cmd_rx: Receiver<DdcCommand>, resp_tx: Sender<BrightnessMessage>) -> Self {
        Self {
            monitors: HashMap::new(),
            cmd_rx,
            resp_tx,
        }
    }

    /// Runs the worker loop, processing commands until shutdown.
    ///
    /// This method blocks until a `DdcCommand::Shutdown` is received
    /// or the command channel is disconnected.
    pub(crate) fn run(mut self) {
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
                DdcCommand::RefreshAll { generation } => {
                    self.handle_refresh_all(generation);
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
            monitor.set_brightness(value)
        } else {
            log::warn!(monitor_id:% = monitor_id; "Monitor not found");
            Err(crate::BrightnessError::MonitorNotFound(
                monitor_id.to_string(),
            ))
        };

        let (success, error) = match result {
            Ok(()) => {
                log::debug!(monitor_id:% = monitor_id.full_identity(), brightness = value; "Set brightness");
                (true, None)
            }
            Err(e) => {
                // The controller is the handling point and logs this failure
                // at error level (the text travels in the result message);
                // this line only keeps the worker-side timeline visible.
                log::debug!(monitor_id:% = monitor_id, error:% = e; "Failed to set brightness");
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
    fn handle_refresh_all(&mut self, generation: u64) {
        log::debug!("Refreshing all monitors");

        // Clear existing state
        self.monitors.clear();

        let mut results: Vec<(MonitorId, u8)> = Vec::new();
        let mut enumerated: Vec<MonitorId> = Vec::new();

        // Enumerate monitors
        let hmonitors = match enumerate_monitors() {
            Ok(h) => h,
            Err(e) => {
                log::error!(error:% = e; "Failed to enumerate monitors");
                // Send empty result
                self.send_refresh_result(generation, results, enumerated);
                return;
            }
        };

        for hmonitor in hmonitors {
            if let Err(e) = self.process_monitor(hmonitor, &mut results, &mut enumerated) {
                log::warn!(error:% = e; "Failed to process monitor");
            }
        }

        self.send_refresh_result(generation, results, enumerated);
    }

    /// Processes a single monitor during refresh.
    fn process_monitor(
        &mut self,
        hmonitor: HMONITOR,
        results: &mut Vec<(MonitorId, u8)>,
        enumerated: &mut Vec<MonitorId>,
    ) -> crate::Result<()> {
        // Get monitor ID from EDID
        let monitor_id = get_monitor_id(hmonitor)?;
        // Identified ⇒ physically present. Push before opening the physical
        // handle: a handle-open or brightness-read failure below must count
        // as unreadable, not as absent from the topology.
        enumerated.push(monitor_id.clone());

        // Get physical monitors for DDC
        let physical_monitors = get_physical_monitors(hmonitor)?;

        for p_mon in physical_monitors {
            let mut ddc_mon = DdcMonitor::new(p_mon, monitor_id.clone());

            // Read current brightness
            match ddc_mon.get_brightness() {
                Ok(brightness) => {
                    // The monitor's declared range travels with the value: a
                    // maximum other than 100 is the one condition that makes
                    // every brightness number in a log suspect, and a field log
                    // is the only place it can be observed.
                    log::debug!(monitor_id:% = monitor_id.full_identity(), brightness = brightness, reported_max:? = ddc_mon.reported_max(); "Read monitor brightness");

                    results.push((monitor_id.clone(), brightness));
                    self.monitors.insert(monitor_id.clone(), ddc_mon);
                }
                Err(e) => {
                    // Unreadable is often transient (standby, KVM, DDC
                    // hiccup). Keep the handle so sets are still attempted
                    // instead of failing with "monitor not found" until the
                    // next refresh; only the reported value is missing.
                    log::warn!(monitor_id:% = monitor_id, error:% = e; "Could not read brightness; keeping handle for sets");
                    self.monitors.insert(monitor_id.clone(), ddc_mon);
                }
            }
        }

        Ok(())
    }

    /// Sends refresh results back to the main thread.
    fn send_refresh_result(
        &self,
        generation: u64,
        monitors: Vec<(MonitorId, u8)>,
        enumerated: Vec<MonitorId>,
    ) {
        let msg = BrightnessMessage::DdcRefreshResult {
            generation,
            monitors,
            enumerated,
        };

        if let Err(e) = self.resp_tx.send(msg) {
            log::error!(error:% = e; "Failed to send refresh result");
        }
    }
}

/// Owns the DDC worker thread and can respawn it after a confirmed death.
///
/// The supervisor holds the command-channel sender, the worker's join handle,
/// and a persistent response-channel sender used to wire each new worker.
///
/// The restart *policy* is not implemented here: it is [`RespawnGate`], the
/// same crash-loop backoff the hotkey thread uses, so there is one place to
/// change how often a supervised thread may be restarted. This type decides
/// only what a decision means for a worker — spawn a replacement, or report
/// that none was spawned — and translates it into [`RespawnOutcome`], which is
/// what the controller actually needs to know.
pub struct DdcSupervisor {
    cmd_tx: Sender<DdcCommand>,
    handle: JoinHandle<()>,
    resp_tx: Sender<BrightnessMessage>,
    gate: RespawnGate,
}

impl DdcSupervisor {
    /// Spawns the initial worker and returns its supervisor.
    ///
    /// `resp_tx` is the channel the worker sends results on; the supervisor
    /// keeps a clone so it can wire replacement workers.
    ///
    /// # Panics
    ///
    /// Panics if the OS refuses the initial worker thread. There is no useful
    /// degraded startup: the worker is the only path to the hardware, and this
    /// runs before any window exists to report the failure in. A worker refused
    /// *later* is not fatal — see [`DdcSupervisor::respawn`].
    #[must_use]
    pub fn spawn(resp_tx: Sender<BrightnessMessage>) -> Self {
        let (cmd_tx, handle) =
            Self::spawn_worker(&resp_tx).expect("OS refused to start the DDC worker thread");
        Self {
            cmd_tx,
            handle,
            resp_tx,
            gate: RespawnGate::new(RESPAWN_WINDOW, RESPAWN_MAX),
        }
    }

    /// Creates a fresh command channel and spawns a worker draining it.
    ///
    /// Returns `None` if the OS refuses the thread; callers decide whether that
    /// is fatal (startup) or a degraded state to report (respawn).
    fn spawn_worker(
        resp_tx: &Sender<BrightnessMessage>,
    ) -> Option<(Sender<DdcCommand>, JoinHandle<()>)> {
        let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<DdcCommand>();
        let worker = DdcWorker::new(cmd_rx, resp_tx.clone());
        let handle = std::thread::Builder::new()
            .name("ddc".to_string())
            .spawn(move || worker.run())
            .map_err(|e| {
                log::error!(error:% = e; "Failed to spawn DDC worker thread");
            })
            .ok()?;
        Some((cmd_tx, handle))
    }

    /// Sends a command to the worker.
    ///
    /// # Errors
    ///
    /// Returns `SendError` if the worker's receiver has been dropped (the
    /// worker has died) — callers treat this as a hard failure.
    pub(crate) fn send(&self, cmd: DdcCommand) -> Result<(), SendError<DdcCommand>> {
        self.cmd_tx.send(cmd)
    }

    /// Whether the worker thread is still running.
    #[must_use]
    pub(crate) fn is_alive(&self) -> bool {
        !self.handle.is_finished()
    }

    /// Attempts to respawn a dead worker, honouring the backoff window.
    pub(crate) fn respawn(&mut self, now: Instant) -> RespawnOutcome {
        match self.gate.on_death(now) {
            RespawnDecision::Attempt => {
                // A refused thread is reported as an exhausted budget rather
                // than panicking on the main thread: the caller already has a
                // degraded mode for a worker that will not come back.
                let Some((cmd_tx, handle)) = Self::spawn_worker(&self.resp_tx) else {
                    return RespawnOutcome::BackoffExceeded;
                };
                self.cmd_tx = cmd_tx;
                self.handle = handle;
                RespawnOutcome::Respawned
            }
            RespawnDecision::GaveUpNow | RespawnDecision::AlreadyGaveUp => {
                RespawnOutcome::BackoffExceeded
            }
        }
    }

    /// Clears the respawn history so recovery can retry immediately.
    pub(crate) fn clear_backoff(&mut self) {
        self.gate.reset();
    }

    /// Asks the worker to shut down (best-effort; does not join).
    pub(crate) fn shutdown(&self) {
        let _ = self.cmd_tx.send(DdcCommand::Shutdown);
    }
}

impl crate::core::controller::DdcPort for DdcSupervisor {
    fn send(&mut self, cmd: DdcCommand) -> crate::Result<()> {
        DdcSupervisor::send(self, cmd).map_err(|_| crate::BrightnessError::ChannelSend)
    }
    fn is_alive(&self) -> bool {
        DdcSupervisor::is_alive(self)
    }
    fn respawn(&mut self, now: Instant) -> RespawnOutcome {
        DdcSupervisor::respawn(self, now)
    }
    fn clear_backoff(&mut self) {
        DdcSupervisor::clear_backoff(self);
    }
    fn shutdown(&self) {
        DdcSupervisor::shutdown(self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// Spawns a supervisor whose results nobody reads.
    ///
    /// The receiver is returned rather than dropped: dropping it would make
    /// every `resp_tx.send` fail, which is a different scenario than the
    /// backoff tests intend to exercise.
    fn supervisor() -> (DdcSupervisor, mpsc::Receiver<BrightnessMessage>) {
        let (resp_tx, resp_rx) = mpsc::channel();
        (DdcSupervisor::spawn(resp_tx), resp_rx)
    }

    #[test]
    fn respawn_is_rate_limited_to_the_budget_within_the_window() {
        let (mut sup, _resp_rx) = supervisor();
        let base = Instant::now();
        let budget = u64::try_from(RESPAWN_MAX).expect("respawn budget fits in u64");

        // Deaths one second apart: all inside the sliding window.
        for secs in 0..budget {
            assert_eq!(
                sup.respawn(base + Duration::from_secs(secs)),
                RespawnOutcome::Respawned,
                "respawn {secs} is within budget"
            );
        }
        assert_eq!(
            sup.respawn(base + Duration::from_secs(budget)),
            RespawnOutcome::BackoffExceeded,
            "one past the budget is a crash loop, not a restart"
        );
    }

    #[test]
    fn clear_backoff_reopens_an_exhausted_gate() {
        // The whole recovery path depends on this. Once the backoff is
        // exhausted the controller stops calling respawn at all, and the only
        // way back is a keypress or a system resume clearing it. A policy that
        // latched permanently would leave the app with no worker — and no way
        // to get one — for the rest of the session.
        let (mut sup, _resp_rx) = supervisor();
        let base = Instant::now();
        let budget = u64::try_from(RESPAWN_MAX).expect("respawn budget fits in u64");

        for secs in 0..budget {
            let _ = sup.respawn(base + Duration::from_secs(secs));
        }
        assert_eq!(
            sup.respawn(base + Duration::from_secs(budget)),
            RespawnOutcome::BackoffExceeded
        );

        sup.clear_backoff();

        assert_eq!(
            sup.respawn(base + Duration::from_secs(budget)),
            RespawnOutcome::Respawned,
            "clearing the backoff must reopen the gate at the same instant"
        );
    }

    #[test]
    fn deaths_spaced_beyond_the_window_never_exhaust_the_budget() {
        // A worker that dies once a day is not a crash loop. Only deaths
        // clustered inside the window count, so the history has to age out.
        let (mut sup, _resp_rx) = supervisor();
        let base = Instant::now();

        for i in 1..=6 {
            assert_eq!(
                sup.respawn(base + RESPAWN_WINDOW * 2 * i),
                RespawnOutcome::Respawned,
                "isolated death {i} must still be recoverable"
            );
        }
    }

    #[test]
    fn a_respawned_worker_answers_on_the_result_channel() {
        let (mut sup, resp_rx) = supervisor();

        assert_eq!(sup.respawn(Instant::now()), RespawnOutcome::Respawned);
        assert!(sup.is_alive(), "the replacement worker is running");

        // A set for a monitor that does not exist needs no hardware: the worker
        // resolves it to MonitorNotFound and answers. That answer is the proof
        // the replacement is draining the *new* command channel and still holds
        // a live clone of the result sender — the wiring between respawn and
        // the main thread, which a canned RespawnOutcome cannot exercise.
        let id = MonitorId::new("TST", "PROBE", None);
        sup.send(DdcCommand::SetBrightness {
            monitor_id: id.clone(),
            value: 42,
            seq: 7,
        })
        .expect("the command channel points at the replacement worker");

        match resp_rx.recv_timeout(Duration::from_secs(5)) {
            Ok(BrightnessMessage::DdcSetResult {
                monitor_id,
                seq,
                success,
                ..
            }) => {
                assert_eq!(monitor_id, id);
                assert_eq!(seq, 7, "the sequence number survives the round trip");
                assert!(!success, "no such monitor exists");
            }
            other => panic!("expected a set result from the respawned worker, got {other:?}"),
        }
    }

    #[test]
    fn test_ddc_worker_creation() {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (resp_tx, _resp_rx) = mpsc::channel();

        let worker = DdcWorker::new(cmd_rx, resp_tx);

        // Worker should start with empty state
        assert!(worker.monitors.is_empty());

        // Clean up
        drop(cmd_tx);
    }

    /// Asserts the worker has exited for the right reason.
    ///
    /// `run()` returning is the headline property — a worker that failed to
    /// stop would hang the test rather than fail it — but on its own that
    /// proves nothing about *how* it stopped. Exit must also be silent (no
    /// result may be emitted for a command that is not a brightness operation)
    /// and it must have released its end of the result channel, which only a
    /// consumed `run(self)` does. One `try_recv` distinguishes all three: a
    /// value means something was emitted, `Empty` means the sender outlived the
    /// call, `Disconnected` means a clean exit with nothing sent.
    fn assert_exited_silently(resp_rx: &mpsc::Receiver<BrightnessMessage>) {
        match resp_rx.try_recv() {
            Err(mpsc::TryRecvError::Disconnected) => {}
            Ok(msg) => panic!("worker emitted a result while shutting down: {msg:?}"),
            Err(mpsc::TryRecvError::Empty) => {
                panic!("worker returned but its result sender is still alive");
            }
        }
    }

    #[test]
    fn shutdown_command_exits_the_worker_silently() {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (resp_tx, resp_rx) = mpsc::channel();

        let worker = DdcWorker::new(cmd_rx, resp_tx);
        cmd_tx.send(DdcCommand::Shutdown).unwrap();
        worker.run();

        assert_exited_silently(&resp_rx);
    }

    #[test]
    fn a_disconnected_command_channel_exits_the_worker_silently() {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (resp_tx, resp_rx) = mpsc::channel();

        let worker = DdcWorker::new(cmd_rx, resp_tx);
        // A main thread that died without sending Shutdown must not strand the
        // worker: dropping the command sender has to end the loop too.
        drop(cmd_tx);
        worker.run();

        assert_exited_silently(&resp_rx);
    }
}
