# Design: DDC worker supervision & state watchdog

_Date: 2026-07-04 · Addresses architecture-review findings #2 (primary), #1, and #3 (touched scope)_

## Problem

`docs/architecture-review.md` finding #2: **worker-thread death causes silent, permanent
degradation — no supervision or watchdog.** Two independent latches, both rooted in the same
cause (a DDC result that never arrives corrupts *both* state and function at once):

- **Stuck optimistic value:** `handle_adjust` sets `pending_brightness` optimistically
  (`main.rs:406`), then `ddc_cmd_tx.send(...)` only logs on `Err` (`main.rs:432`). If the worker
  is gone, no `DdcSetResult` ever returns, so `confirm`/`revert` never runs. The OSD shows a value
  never written to hardware — violating the documented "strict error handling / red OSD on
  failure" contract in exactly the case it is meant to cover.
- **`refresh_in_progress` latches forever:** set in `handle_refresh` (`main.rs:463`), cleared only
  by a returning `DdcRefreshResult` (`main.rs:326`). If that result never arrives (worker panic,
  or message dropped on shutdown), all future periodic/inactivity refreshes are silently
  suppressed for the process lifetime.

Root cause: workers are `thread::spawn`ed detached with no `catch_unwind`/supervision; the main
loop only exits on channel `Disconnected` (`main.rs:863`). Because the hotkey/tray/power threads
still hold `tx` clones, `rx` never disconnects, so a panicked DDC worker leaves the app running
degraded indefinitely.

Two adjacent findings are pulled into scope because they are entangled with the fix:

- **#1 — DDC set-results aren't correlated with their request** (`handle_ddc_set_result`,
  `main.rs:267` ignores `value` and acts on whatever `pending_brightness` currently is). Under
  rapid input, an earlier command's revert clears the `pending` belonging to a later in-flight
  command. The watchdog needs request correlation anyway, so this is fixed here.
- **#3 — the confirm/revert state machine is untested** and partly lives in the binary crate
  (`main.rs`), unreachable from integration tests. This design extracts the pure reconciliation
  logic into `core/` and unit-tests it.

## Scope

**In scope:** full supervision + automatic respawn of the DDC worker; a state watchdog that
self-heals both latches; sequence-id correlation of DDC set results (#1); extraction of the pure
reconciliation logic into a testable `core/` unit with unit tests (#3, in the touched scope).

**Out of scope (non-goals):** supervising the hotkey/tray/power threads (only the DDC worker has
the latch consequences; the pattern can be extended later); findings #4–#7. No config knobs for
the new timeouts (constants — YAGNI).

## Design

### Leitidee: two *separate* mechanisms

The core insight is that today a single missing result breaks **both** state and function. The
design decouples these:

1. **State watchdog (safe; heals latches).** Purely main-thread-side. When a `pending` or
   `refresh` passes a deadline, only the *state* is reconciled (revert + red OSD, or clear the
   refresh flag). It never touches the worker, so it is always safe, and it covers **every**
   "result never arrives" case uniformly (panic, dropped message, hang).
2. **Worker supervision (heals function).** Respawns the worker only on **confirmed death** —
   `JoinHandle::is_finished()` **or** a `send()` error. A merely *slow* worker is **never**
   respawned (that would run two threads against the same physical-monitor handles); its state is
   reconciled by mechanism 1, and its late result is discarded by sequence-id mismatch.

Safety property: **respawn is triggered only by confirmed death, never by a timeout alone.**

### Component 1 — sequence-id correlation (core, platform-agnostic)

`MonitorState.pending_brightness: Option<u8>` becomes `pending: Option<PendingSet>`:

```rust
pub struct PendingSet {
    pub value: u8,
    pub seq: u64,
    pub sent_at: Instant,
}
```

Methods on `MonitorState`:

- `set_pending(value, seq, now)` — records an optimistic set with its sequence id and send time.
- `confirm(seq) -> bool` / `revert(seq) -> bool` — **seq-guarded**: act only when `seq` matches
  the current pending's seq; a stale result (whose pending was already superseded) is ignored
  rather than wrongly clearing the newer pending. This is the fix for #1.
- `pending_timed_out(now, timeout) -> bool` — pure decision, `now` passed in.
- `force_revert()` — unconditional clear (used by the watchdog on timeout).
- `effective_brightness()` — unchanged semantics (`pending.value` else `cached`).

A `u64` sequence counter lives in the controller; each `SetBrightness` takes the next value.
`DdcCommand::SetBrightness` and `BrightnessMessage::DdcSetResult` both carry `seq` (echoed back by
the worker). `RefreshAll` stays unstamped — only one refresh is ever in flight, tracked by the
`RefreshTracker` below; a late refresh result carries fresh data and is harmless to apply.

### Component 2 — testable reconciliation units (core, `src/core/reconcile.rs`)

- **`RefreshTracker`** — replaces the loose `refresh_in_progress` / `last_refresh` /
  `last_refresh_successful` fields on the controller:

  ```rust
  pub struct RefreshTracker {
      in_progress: bool,
      started_at: Option<Instant>,
      last_refresh: Instant,
      last_successful: bool,
  }
  ```

  Methods: `begin(now)`, `complete(now, found)`, `timed_out(now, timeout) -> bool`, `abort()`,
  plus read accessors the controller needs (`in_progress()`, `last_successful()`, elapsed since
  `last_refresh`).

- **`respawn_allowed(recent: &[Instant], now, window, max) -> bool`** — pure backoff decision:
  `true` if fewer than `max` respawns fall within `window` before `now`.

All time-based decisions are pure functions taking `now: Instant`, so they are unit-testable
cross-platform. Respawn, FFI, and OSD side effects stay in `main.rs` / `platform`.

### Component 3 — `DdcSupervisor` (supervision/respawn, `platform/windows/ddc_worker.rs`)

Owns the worker lifecycle. Replaces the raw `ddc_cmd_tx` on the controller.

```rust
pub struct DdcSupervisor {
    cmd_tx: Sender<DdcCommand>,
    handle: JoinHandle<()>,
    resp_tx: Sender<BrightnessMessage>,  // persistent clone, for respawns
    recent_respawns: Vec<Instant>,
}

pub enum RespawnOutcome { Respawned, BackoffExceeded }
```

- `spawn(resp_tx) -> Self` — builds a fresh cmd channel, spawns the worker, stores the handle.
- `send(cmd) -> Result<(), SendError<DdcCommand>>` — delegates to `cmd_tx`.
- `is_alive() -> bool` — `!handle.is_finished()`.
- `respawn(now) -> RespawnOutcome` — consults `respawn_allowed`; on allow, rebuilds the cmd
  channel + spawns a new worker + records the timestamp; on deny, returns `BackoffExceeded`.
- `shutdown(self)` — sends `DdcCommand::Shutdown`.

Controller additionally holds: `RefreshTracker`, `next_seq: u64`, `last_health_check: Instant`,
and `ddc_disabled: bool` (degraded latch after backoff is exceeded).

### Component 4 — main-loop integration

Throttled to roughly every 250 ms (via `last_health_check`), each tick:

- **`supervise_worker()`** — if `!supervisor.is_alive()` and not shutting down and not already
  disabled, call `respawn(now)`:
  - `Respawned` → `log::warn`; `force_revert()` every stuck pending + red OSD; `RefreshTracker.abort()`;
    then a fresh `handle_refresh()` (the new worker starts empty and must re-enumerate).
  - `BackoffExceeded` → `ddc_disabled = true`; `log::error`; red OSD.
- **`check_watchdogs(now)`** —
  - any monitor whose pending `pending_timed_out(now, SET_TIMEOUT)` → `force_revert()` + red OSD +
    `log::error`.
  - if `RefreshTracker.timed_out(now, REFRESH_TIMEOUT)` → `abort()` (clears `in_progress`, sets
    `last_successful = false` so the next user activity retries).

**Send path.** `handle_adjust` / `handle_refresh` use `supervisor.send(...)`. On `Err` (worker
already gone at send time) → immediate hard fail: `revert(seq)` + red OSD for a set, or abort the
refresh; the next `supervise_worker()` tick detects the death and respawns. This realises the
review's "send-failure = hard error, red OSD" direction. Both signals are kept: `send()`-Err gives
instant feedback; `is_finished()` catches a mid-command panic where `send` already succeeded.

**Recovery from `ddc_disabled`.** Reset on a manual tray *Refresh* and on `SystemResumed`, giving a
recovery path without auto-spinning the respawn loop.

### Component 5 — constants (core)

Named constants (not config knobs; YAGNI, may move to config later):

- `SET_TIMEOUT ≈ 1500 ms` — generous vs. the ~40–120 ms × up-to-3-attempts per set.
- `REFRESH_TIMEOUT ≈ 5000 ms` — enumeration touches every monitor.
- `RESPAWN_MAX = 3` within `RESPAWN_WINDOW = 60 s`.

### Startup wiring change

Today (`main.rs:774–790`): main creates `(ddc_cmd_tx, ddc_cmd_rx)`, a `ddc_shutdown_tx` clone,
spawns a detached worker, and hands `ddc_cmd_tx` to the controller. New: `DdcSupervisor::spawn(tx.clone())`
owns the cmd channel + handle; the controller holds the supervisor. Shutdown at exit becomes
`supervisor.shutdown()` (via the controller) instead of the standalone `ddc_shutdown_tx`.

## Error handling / contract

- A stuck or timed-out set surfaces `osd.update_error(state)` (the documented red OSD) +
  `log::error!` — honouring "strict error handling" in exactly the case it targets. Consistent with
  the existing rule that hardware failure for >0% does **not** fall back to overlay dimming.
- `ddc_disabled` (backoff exceeded) surfaces a red OSD on the next adjust + an error log; the app
  keeps running rather than spinning.

## Testing (core, cross-platform)

Unit tests in `core`:

- **Seq correlation:** matching `confirm(seq)` commits; matching `revert(seq)` clears; a stale
  `confirm`/`revert` (non-matching seq) is a no-op. Explicit repro for #1: `set_pending(v1, seq1)`,
  `set_pending(v2, seq2)`, `revert(seq1)` → pending is still `v2/seq2`.
- **Pending timeout:** `pending_timed_out` false before / true after the deadline; `force_revert`
  clears and `effective_brightness` falls back to `cached`.
- **`RefreshTracker`:** `begin` sets `in_progress`; `timed_out` after the deadline; `abort` clears
  and sets `last_successful = false`; `complete` resets and stamps `last_refresh`.
- **`respawn_allowed`:** `max` events within `window` → deny; spaced beyond `window` → allow.

`Instant`-based tests build a base `Instant::now()` and add `Duration`s. Respawn/FFI/OSD paths
remain manually verified on Windows (per the repo's manual-hardware-testing convention).

## Open questions

None blocking. Timeout/backoff constants are tunable in review; they are isolated in one place.
