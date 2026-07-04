# Design: DDC worker supervision & state watchdog

_Date: 2026-07-04 · Addresses architecture-review findings #2 (primary), #1, and #3 (touched scope)_

_Revised 2026-07-04 after a cold adversarial review — see "Review amendments" at the end._

## Problem

`docs/architecture-review.md` finding #2: **worker-thread death causes silent, permanent
degradation — no supervision or watchdog.** Two independent latches, both rooted in the same
cause (a DDC result that never arrives corrupts *both* state and function at once):

- **Stuck optimistic value:** `handle_adjust` sets `pending_brightness` optimistically
  (`main.rs:406`), then `ddc_cmd_tx.send(...)` only logs on `Err` (`main.rs:428–432`). Whether the
  worker is already gone (send fails) or dies mid-command (send succeeds, no result returns),
  `pending_brightness` is left set and `confirm`/`revert` never runs. The OSD shows a value never
  written to hardware — violating the documented "strict error handling / red OSD on failure"
  contract in exactly the case it is meant to cover.
- **`refresh_in_progress` latches:** set in `handle_refresh` (`main.rs:463`); it *is* cleared on an
  immediate `send` error (`main.rs:466–469`), so a worker that is already dead at send time does
  **not** latch the refresh flag. The latch occurs only when the send **succeeds** and no
  `DdcRefreshResult` ever returns (`main.rs:326`) — i.e. the worker panics mid-command, or the
  result is dropped on shutdown. In that case all future periodic/inactivity refreshes are silently
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

### Leitidee: two *separate* mechanisms, and what each one is actually for

The core insight is that today a single missing result breaks **both** state and function. The
design decouples these — and, crucially, assigns each mechanism the failure mode it is *precisely*
good at:

1. **Death detection — the fast, precise path for a *dead* worker.** `JoinHandle::is_finished()`
   (polled every ~250 ms) plus `send()` errors detect a worker that has panicked or returned.
   This is what heals both latches in the common failure case, in ~250 ms, without waiting on any
   timeout. On confirmed death it force-reverts every stuck pending and respawns the worker.
2. **State watchdog — a slow *backstop* for a *hung* worker.** A worker that is alive but stuck
   forever inside a blocking Win32 DDC call is invisible to `is_finished()`. Only a wall-clock
   deadline catches it. This deadline is therefore **generous** (see `SET_TIMEOUT` below): it must
   not fire on a set that is merely *queued behind a refresh*, only on one that is genuinely never
   going to complete. It touches only main-thread state (revert + red OSD, clear the refresh flag)
   and never the worker, so it is always safe.

**Safety property:** respawn is triggered only by confirmed death (`is_finished`/`send`-Err),
**never by a timeout alone.** Respawning a merely-hung worker would run two threads against the
same physical-monitor handles — worse than the disease. A hung worker is instead reconciled by the
watchdog and, after repeated timeouts, latched into the `ddc_disabled` degraded state.

### Component 1 — sequence-id correlation (core, platform-agnostic)

`MonitorState.pending_brightness: Option<u8>` becomes `pending: Option<PendingSet>`:

```rust
pub struct PendingSet {
    pub value: u8,
    pub seq: u64,
    pub sent_at: Instant,
}
```

A `u64` sequence counter lives in the controller; each `SetBrightness` takes the next value.
`DdcCommand::SetBrightness` and `BrightnessMessage::DdcSetResult` both carry `seq` (echoed back by
the worker). `u64` never realistically wraps.

Methods on `MonitorState`:

- `set_pending(value, seq, now)` — records an optimistic set with its sequence id and send time.
- `apply_set_result(seq, value, success) -> SetOutcome` — the single reconciliation entry point,
  replacing the old blind `confirm`/`revert`. Cases:
  - `pending.seq == seq` → **matched**: on success commit (`cached = value`), on failure revert
    (`pending = None`). Normal path.
  - `pending.seq > seq` (a newer set is already in flight) → **stale**: ignore entirely; the newer
    set's own result defines the final state.
  - `pending == None` → the pending was already resolved (e.g. by the watchdog's `force_revert`,
    Component 4). On **success** this is a *late but authoritative* result — the hardware really did
    change — so apply it as ground truth via `cached = value` (equivalent to a refresh datum). On
    **failure**, ignore (nothing changed). This is finding #2.
- `force_revert()` — unconditional clear (used by the watchdog / respawn path on timeout/death).
- `pending_timed_out(now, timeout) -> bool` — pure decision, `now` passed in.
- `effective_brightness()` — unchanged semantics (`pending.value` else `cached`).
- `update_from_ddc(value)` (refresh datum) — updates `cached_brightness`/`last_refresh` but
  **preserves a live `pending`.** A live optimistic set out-ranks a refresh read that was enqueued
  earlier; it resolves via its own result or the watchdog. (Change from today's unconditional
  clear at `state.rs:223–227`.) This is finding #9.

### Component 2 — testable reconciliation units (core, `src/core/reconcile.rs`)

- **`RefreshTracker`** — replaces the loose `refresh_in_progress` / `last_refresh` /
  `last_refresh_successful` fields on the controller, and enforces the "one refresh in flight"
  invariant that the current code only *asserts*:

  ```rust
  pub struct RefreshTracker {
      in_progress: bool,
      generation: u64,          // bumped on each begin(); echoed by the result
      started_at: Option<Instant>,
      last_refresh: Instant,
      last_successful: bool,
  }
  ```

  Methods: `begin(now) -> u64` (bumps `generation`, returns it), `complete(generation, now, found)`
  (**ignores stale results** whose `generation` ≠ the current one — finding #3), `timed_out(now,
  timeout) -> bool`, `abort()`, plus read accessors (`in_progress()`, `last_successful()`, elapsed
  since `last_refresh`). `RefreshAll` is stamped: `DdcCommand::RefreshAll { generation }` and
  `BrightnessMessage::DdcRefreshResult { generation, monitors }` carry it.

- **`respawn_allowed(recent: &[Instant], now, window, max) -> bool`** — pure backoff decision:
  `true` if fewer than `max` respawns fall within `window` before `now`.

All time-based decisions are pure functions taking `now: Instant`, so they are unit-testable
cross-platform (`Instant::now() + Duration`; no need to construct an arbitrary `Instant`). Respawn,
FFI, and OSD side effects stay in `main.rs` / `platform`.

### Component 3 — `DdcSupervisor` (supervision/respawn, `platform/windows/ddc_worker.rs`)

Owns the worker lifecycle. Replaces the raw `ddc_cmd_tx` on the controller.

```rust
pub struct DdcSupervisor {
    cmd_tx: Sender<DdcCommand>,
    handle: JoinHandle<()>,
    resp_tx: Sender<BrightnessMessage>,  // persistent clone, for respawns
    recent_respawns: Vec<Instant>,       // pruned to RESPAWN_WINDOW on each attempt
}

pub enum RespawnOutcome { Respawned, BackoffExceeded }
```

- `spawn(resp_tx) -> Self` — builds a fresh cmd channel, spawns the worker, stores the handle.
- `send(cmd) -> Result<(), SendError<DdcCommand>>` — delegates to `cmd_tx`.
- `is_alive() -> bool` — `!handle.is_finished()`.
- `respawn(now) -> RespawnOutcome` — first prunes `recent_respawns` older than `RESPAWN_WINDOW`,
  then consults `respawn_allowed`; on allow, rebuilds the cmd channel + spawns a new worker +
  records the timestamp; on deny, returns `BackoffExceeded`.
- `clear_backoff()` — empties `recent_respawns` (used by the recovery path, finding #5).
- `shutdown(self)` — sends `DdcCommand::Shutdown`.

Controller additionally holds: `RefreshTracker`, `next_seq: u64`, `last_health_check: Instant`,
`consecutive_set_timeouts: u32` (hung-worker escalation, finding #6), and `ddc_disabled: bool`
(degraded latch after backoff is exceeded or the worker is diagnosed hung).

### Component 4 — main-loop integration

Throttled to roughly every 250 ms (via `last_health_check`), each tick:

- **`supervise_worker()`** — if `!supervisor.is_alive()` and not shutting down and not already
  disabled, call `respawn(now)`:
  - `Respawned` → `log::warn`; `force_revert()` every stuck pending (red OSD per the visibility
    rule below); `RefreshTracker.abort()`; then a fresh `handle_refresh()` (the new worker starts
    empty and must re-enumerate — `id_cache`/overlay state survive because recovery routes through
    the normal refresh path). Reset `consecutive_set_timeouts = 0`.
  - `BackoffExceeded` → `ddc_disabled = true`; `log::error`; red OSD.
- **`check_watchdogs(now)`** —
  - any monitor whose pending `pending_timed_out(now, SET_TIMEOUT)` → `force_revert()`, red OSD,
    `log::error`, and **increment `consecutive_set_timeouts`**. If the worker is still
    `is_alive()` (i.e. hung, not dead) and `consecutive_set_timeouts >= HUNG_TIMEOUT_LIMIT` →
    `ddc_disabled = true` with a **distinct** log ("worker hung: N consecutive set timeouts, DDC
    disabled until restart"). Recovery is app-restart / the recovery triggers below. No respawn
    (handle contention). This is finding #6.
  - if `RefreshTracker.timed_out(now, REFRESH_TIMEOUT)` → `abort()` (clears `in_progress`, sets
    `last_successful = false` so the next user activity retries).

Any **matched successful** set result resets `consecutive_set_timeouts = 0`.

**Red OSD visibility rule (finding #8).** The watchdog surfaces the error via
`osd.update_error(state)` **only if `osd.is_visible()`**, matching the existing convention at
`main.rs:293–296`. It never spontaneously shows a new OSD — the watchdog fires seconds after the
keypress, by which time the OSD has usually auto-hidden, and `cached_brightness` is authoritative
so the next interaction shows the correct value. When the respawn path force-reverts several
monitors at once, only the monitor whose OSD is currently visible is restyled; the rest revert
silently.

**Send path.** `handle_adjust` / `handle_refresh` use `supervisor.send(...)`. On `Err` (worker
already gone at send time) → immediate hard fail: `apply_set_result(seq, _, false)` (revert) + red
OSD for a set, or abort the refresh; the next `supervise_worker()` tick detects the death and
respawns. This realises the review's "send-failure = hard error, red OSD" direction. Both signals
are kept: `send()`-Err gives instant feedback; `is_finished()` catches a mid-command panic where
`send` already succeeded.

**Recovery from `ddc_disabled` (findings #4, #5).** Reset on **user activity (a hotkey adjust in
`handle_adjust`)** and on **`SystemResumed`** — both already call `handle_refresh()`. There is *no*
manual tray "Refresh" action in the codebase, so it is not a trigger. The reset **must also call
`supervisor.clear_backoff()`** and set `consecutive_set_timeouts = 0`; otherwise, if recovery
fires within `RESPAWN_WINDOW` of the last respawn, the next tick re-consults the same ≥`max`
timestamps and immediately re-latches (finding #5).

### Component 5 — constants (core)

Named constants (not config knobs; YAGNI, may move to config later):

- `SET_TIMEOUT ≈ 8000 ms` — a **hung-worker backstop**, deliberately *larger* than
  `REFRESH_TIMEOUT + a set budget`. A set may sit in the worker's serial queue behind an
  in-flight refresh (up to `REFRESH_TIMEOUT`), because `handle_adjust` can enqueue a `RefreshAll`
  immediately before the set (`main.rs:338–344`). The pending's `sent_at` starts at enqueue time,
  so a tight deadline would false-fire on a perfectly healthy post-inactivity / post-resume
  hotkey press. A *dead* worker is healed in ~250 ms by death detection regardless of this value,
  so making it generous costs nothing on the dead-worker path and only affects the rare genuinely
  hung worker. This is the fix for finding #1.
- `REFRESH_TIMEOUT ≈ 5000 ms` — enumeration touches every monitor (EDID registry read +
  `get_brightness` at 40–120 ms × up to 3 attempts each).
- `RESPAWN_MAX = 3` within `RESPAWN_WINDOW = 60 s`.
- `HUNG_TIMEOUT_LIMIT = 3` consecutive set timeouts before diagnosing a hung worker.

### Startup wiring change

Today (`main.rs:774–790`): main creates `(ddc_cmd_tx, ddc_cmd_rx)`, a `ddc_shutdown_tx` clone,
spawns a detached worker, and hands `ddc_cmd_tx` to the controller. New: `DdcSupervisor::spawn(tx.clone())`
owns the cmd channel + handle; the controller holds the supervisor. Shutdown at exit becomes
`supervisor.shutdown()` (via the controller) instead of the standalone `ddc_shutdown_tx`.

## Error handling / contract

- A stuck or timed-out set surfaces `osd.update_error(state)` (the documented red OSD, only if
  visible) + `log::error!` — honouring "strict error handling" in exactly the case it targets.
  Consistent with the existing rule that hardware failure for >0% does **not** fall back to overlay
  dimming.
- `ddc_disabled` (backoff exceeded, or worker diagnosed hung) surfaces a red OSD on the next adjust
  + an error log; the app keeps running rather than spinning, and recovers on user activity /
  resume.

## Testing (core, cross-platform)

Unit tests in `core`:

- **Seq correlation & late results (`apply_set_result`):** matched success commits; matched
  failure reverts; a *stale* result (`pending.seq > seq`) is a no-op. Explicit repro for #1:
  `set_pending(v1, seq1)`, `set_pending(v2, seq2)`, then the `seq1` failure result → pending is
  still `v2/seq2`. Finding #2: `force_revert()` then a `seq`-matching **success** with `pending ==
  None` → `cached = value` (ground truth); a `pending == None` **failure** → no-op.
- **`update_from_ddc` vs a live pending (#9):** with a live `PendingSet`, a refresh datum updates
  `cached` but leaves `pending` intact; `effective_brightness` still returns the pending value.
- **Pending timeout:** `pending_timed_out` false before / true after the deadline; `force_revert`
  clears and `effective_brightness` falls back to `cached`.
- **`RefreshTracker`:** `begin` sets `in_progress` and bumps `generation`; a `complete` with a
  **stale generation** is ignored (#3); a current-generation `complete` resets and stamps
  `last_refresh`/`last_successful`; `timed_out` after the deadline; `abort` clears and sets
  `last_successful = false`.
- **`respawn_allowed`:** `max` events within `window` → deny; spaced beyond `window` → allow;
  after `clear_backoff` (empty slice) → allow.

`Instant`-based tests build a base `Instant::now()` and add `Duration`s. Respawn/FFI/OSD paths
remain manually verified on Windows (per the repo's manual-hardware-testing convention).

## Open questions

None blocking. Timeout/backoff constants are tunable in review; they are isolated in one place.

## Review amendments

A cold adversarial review (no prior context) verified all file:line citations and the two-mechanism
architecture, and surfaced nine issues, all incorporated above:

1. **(Critical)** A set queued behind an in-flight refresh would false-fire a tight `SET_TIMEOUT`
   on the most common paths (post-inactivity / post-resume hotkey). → `SET_TIMEOUT` reframed as a
   hung-worker backstop, `≈ 8 s > REFRESH_TIMEOUT + set budget`; dead workers still heal in ~250 ms
   via death detection (Leitidee, Component 5).
2. A late *successful* set result after `force_revert` was silently discarded, losing ground truth.
   → `apply_set_result` applies a `pending == None` success as authoritative `cached` (Component 1).
3. "One refresh in flight" was asserted, not enforced; a stale result could corrupt the tracker.
   → refreshes are generation-stamped; `RefreshTracker.complete` ignores stale generations
   (Component 2).
4. `ddc_disabled` recovery named a tray "Refresh" action that does not exist in the code. →
   recovery triggers are user activity + `SystemResumed` (Component 4).
5. Resetting `ddc_disabled` without clearing the respawn history re-latched immediately. → recovery
   also calls `clear_backoff()`; `recent_respawns` is pruned (Components 3, 4).
6. A hung (not dead) worker was permanent silent loss. → after `HUNG_TIMEOUT_LIMIT` consecutive set
   timeouts with the worker still alive, latch `ddc_disabled` with a distinct log (Component 4).
7. The problem statement overstated the refresh latch (send-Err already clears the flag today). →
   problem statement corrected.
8. The watchdog "red OSD" was under-specified. → `update_error` only if visible, no spontaneous
   show, single-monitor rule on multi-revert (Component 4).
9. `update_from_ddc` vs a live `PendingSet` was unspecified. → refresh preserves a live pending; a
   test pins it (Components 1, testing).
