# Architecture Review: darkbright-helper

_Review date: 2026-07-26 · Reviewed at v0.8.0 + post-review polish (commit `e04ebf4`) · Independent
pass. The prior cycle (2026-07-25, all 27 rows resolved) and the `windows` 0.52→0.62 migration that
followed it are treated as done: resolved rows were deliberately not re-litigated. Investigation was
inline over the source, git history, and CI config, cross-checked against a parallel-agent digest
from a concurrent session; **every finding below was re-derived from the code**. Unlike the previous
review, the test suite was executed this time: `cargo test` → **191 tests, all green**._

## Repo profile

- **Artefact type:** Standalone Windows desktop utility — long-running tray/hotkey background
  process, single binary. Signals: `windows_subsystem="windows"` in release (`main.rs:1`),
  tray/OSD/overlay UI, `publish = false` over an explicitly internal `lib.rs` surface, no server,
  no network I/O anywhere in the tree.
- **Maturity:** Active product. v0.8.0, 408 commits, 6 semver tags, maintained CHANGELOG, CI on
  `windows-latest` gating fmt + clippy (`all`+`pedantic`, `-D warnings`, `--all-targets`) + tests +
  release-profile check, plus a separate MSRV job and a weekly `cargo audit` cron.
- **Size & team:** ~11.5k LOC Rust across 24 modules, single author (`Ud3g`, 408/408 commits). A
  large share of the biggest files is in-module tests (`controller.rs`: ~810 logic + ~1030 test).
  Calibration: solo-maintainer bar — change-safety and simplicity over ceremony.
- **Ecosystem:** Rust 2024, MSRV 1.88. `windows` 0.62, `thiserror` 2, `serde`/`serde_json`,
  `log`+`env_logger` (structured kv), `winres` (build). No async runtime — single-owner state on the
  main thread plus MPSC channels across five threads (main / hotkey / ddc_worker / power / tray).
- **Ambiguities resolved:** none — classification was unambiguous.

## Overview

The architectural health established by the last cycle holds, and this pass re-verified the load-
bearing claims rather than trusting them:

- The core/platform boundary is real by import, not by assertion: zero `windows::` paths and zero
  `unsafe` under `src/core/`, strict platform→core direction, no cycles outside the documented
  `osd`↔`osd_render` pair.
- The GDI paths — the usual leak site for an app that repaints on every keypress — are RAII-balanced
  throughout (`osd_render.rs` guards restore *and* delete font, bitmap and memory DC; `fill_rect`
  deletes its brush; `tray.rs` pairs `GetDC`/`ReleaseDC` and guards the DIB). The one unpaired
  `CreateSolidBrush` (`osd.rs:272`) is a window-class background behind a `OnceLock`, i.e.
  process-lifetime, not a leak.
- `power.rs` is correct post-fix: the sent-message path is handled inside the wnd_proc via a
  thread-local sender, and the message-only window is explicitly subscribed through
  `RegisterSuspendResumeNotification`.
- Optimistic-set reconciliation is sound and its load-bearing assumption is written down at the
  point of use (`state.rs:290-295`): the no-pending→`GroundTruth` branch is safe only because one
  serial worker delivers results in issue order.
- The `unsafe impl Send for PhysicalMonitor` invariant (`ddc.rs:99`) matches actual usage —
  worker-confined, deliberately not `Sync`, not `Clone`.

The findings cluster into three themes: **an undocumented DDC protocol assumption at the read
boundary** (C1, I1 — one fix pass, same code path), **degraded states with no real exit or no
visible signal** (I2, I3, I4), and **residue the compiler can no longer see** (I5, I6, I7).

## Findings

### Critical

**C1. The DDC brightness scale is assumed to be 0–100; the monitor's own reported maximum is read
and thrown away.**
`src/platform/windows/ddc.rs:311`, `:323-327`; `src/platform/windows/ddc_worker.rs:190-191`;
`docs/architecture.md:150`.

`get_vcp_feature` correctly retrieves both values from `GetVCPFeatureAndVCPFeatureReply` and
documents its return as `(current_value, maximum_value)` — then the only production caller discards
the max: `let (current, _) = get_vcp_feature(...)`. Verified by grep: `max_value` is produced by
exactly one function and consumed by nothing. Three consequences compound:

1. **Reads are misinterpreted.** The raw VCP value is treated as a percentage. VCP continuous values
   are 16-bit on the wire and MCCS does not require max == 100.
2. **The value is then truncated, not clamped.** `ddc_worker.rs:191` is `brightness as u8` under
   `#[allow(clippy::cast_possible_truncation)]`. A monitor reporting max=1000/current=500 (50%)
   yields `500 as u8` = 244, which `MonitorState` then clamps with `.min(100)` (`state.rs:246`,
   `:297`, `:323`) → displays **100%**. The wraparound lands *inside* the plausible range, so
   nothing looks wrong in the OSD, the tray, or the log.
3. **Writes are wrong in the same direction.** `set_brightness` passes the 0–100 percentage straight
   through (`ddc.rs:324`), so on a max=255 monitor "100%" sets ≈39% actual backlight; on a max<100
   monitor the top of the range is unreachable or rejected.

The docs reinforce the wrong model rather than owning the assumption: `architecture.md:150` presents
the 1:1 mapping as a *perceptual* choice ("a linear mapping is chosen for simplicity… 50% means
exactly 50% backlight power") — a statement about gamma, not about DDC scaling. Neither
`architecture.md` nor `improvement-ideas.md` mentions the reported maximum at all.

Rated Critical because it is a silent correctness failure on the app's single core operation, with
no diagnostic that would ever surface it, in a binary now published to third parties — and the data
needed to fix it is already fetched and dropped.

**Direction:** carry `max` into `DdcMonitor` (already returned), scale on both read and write, and
use `u8::try_from(...)` instead of the allowed truncation. Log the reported max once per monitor at
`debug` — that alone makes the whole class of problem diagnosable from a field log. Then correct
`architecture.md:150` to state the scaling explicitly.

**Condition:** misbehaves only on monitors whose VCP 0x10 max ≠ 100. That the tool works well on the
maintainer's hardware is consistent with those monitors reporting 100, the common case. The
frequency is unverified; the defect is not.

**✅ RESOLVED** — 2026-07-26, commit `7a1607a`. **Open decision settled: scale to percent at the
platform boundary**, not store-native-and-convert-on-display. Everything above the seam is already
percent — `DdcCommand::SetBrightness { value: u8 }`, `MonitorState`, `BRIGHTNESS_MIN/MAX`, the
overlay math, `config.max_brightness` — and `calculate_adjustment`'s `delta: i8` step is only
meaningful in percent (a step of 5 on a max=65535 panel is imperceptible). Storing native would have
pushed a per-monitor scale into state, config and the OSD; percent-at-the-boundary confined the
change to `ddc.rs` + `ddc_worker.rs` and added no state above the seam. `DdcMonitor` is now
explicitly documented as that boundary: it learns `reported_max` on each successful read, converts
both directions, and no raw value escapes it. The `as u8` truncation and its `#[allow]` are gone.

Fallback when no read has yet succeeded (the enumerable-but-unreadable monitor of I1): assume max
100, which is *identical* to the pre-fix behaviour, so that monitor is no worse off. A reported max
of 0 is rejected the same way rather than trusted — it is not a scale, and it would divide by zero.

TDD, in `core/` so the math is host-testable: 7 tests written first. Two-step red — the functions
did not exist (compile error), so stubs encoding the *current* pass-through/truncate semantics went
in next, making the red output the review's own worked example verbatim (`left: 100, right: 50` for
500 of 1000). That also confirms empirically the claim that the wraparound lands inside the
plausible range. Coverage: both scaling directions, the unknown/zero-max fallback, a monitor
reporting a current value above its own maximum, a sub-100 range (percentages collapse onto raw
values — inherent, now documented), and `u32::MAX` to pin the u64 widening.

One test — the 505-case round trip over max ∈ {100, 101, 255, 1000, 65535} — passed on the stub
(identity round-trips trivially), so it was verified to be load-bearing rather than trusted:
replacing the write's round-half-up with truncation fails it at 51% / max 101, which would drift the
OSD by a step on every refresh. Rounding, not truncation, is what makes read-back stable, and that
is the test that holds it.

**Hardware:** the probe now prints `current of max → percent`, and on this machine reports
`45 of 100 → 45%` — so the scaling is a verified no-op here, which is exactly why the defect
survived to a published binary. **Still unverified:** behaviour on a monitor that actually reports a
maximum other than 100. No such panel is available; the conversion is covered by unit tests, but the
end-to-end path on non-100 hardware is untested by construction.

### Important

**I1. A monitor that enumerates but whose brightness read fails never gets state — so it is
permanently uncontrollable, silently.**
`src/platform/windows/ddc_worker.rs:198-205`; `src/core/controller.rs:285-311`, `:319-321`,
`:460-471`; `src/main.rs:713-715`.

The worker deliberately keeps the `DdcMonitor` when a read fails, with an explicit rationale:
*"Unreadable is often transient (standby, KVM, DDC hiccup). Keep the handle so sets are still
attempted instead of failing with 'monitor not found'"*. It reports that monitor in `enumerated` but
not in `monitors`. The controller creates `MonitorState` **only** from `monitors` (`:285`);
`enumerated` feeds absence bookkeeping and nothing else (`:319-321`). The intent is therefore
defeated one module over: no state → `handle_adjust` returns `MonitorNotFound` (`:460`) →
`main.rs:714` logs "Error processing message" → and because the error path returns *before* any OSD
call, the user gets no feedback at all. A dead keypress.

The transient case masks it: each such press also dispatches a refresh, so once a read succeeds the
monitor works. The sharp case is a panel that persistently NAKs the VCP *read* but honours the
*write* — permanently uncontrollable despite a live worker handle, and never pruned either, since it
is present in `enumerated`. The enumerable-but-unreadable shape is explicitly modelled and tested
for the refresh cadence (`periodic_refresh_gates_on_enumerated_not_readable`, `controller.rs:1022`)
— thought through for pruning, missed for state creation.

**This was expected to scope C1** — both defects sit at the same boundary, so seeding state from
`enumerated` and carrying the reported `max` looked like one pass over `handle_ddc_refresh_result`
and the `(MonitorId, u8)` result tuple. **It did not turn out that way** (see C1's note, resolved
`7a1607a`): carrying the max needed no change to either, because the conversion belongs one layer
down, inside `DdcMonitor`. The result tuple still carries a percentage and its shape is untouched, so
this finding stands unchanged and unblocked as a controller-side change on its own.

**Direction:** seed `MonitorState` from `enumerated` as well (last-known or default value plus an
"unknown" marker), or at minimum surface the dead end on the OSD.

**✅ RESOLVED** — 2026-07-26, commit `5c8a653`. **Open decision settled on all three counts.**

*What value.* `UNREAD_BRIGHTNESS_SEED = 50` — not an invented number: `MonitorState`'s existing
`Default` already used 50, so this makes a latent choice explicit. The midpoint is the right one
because the first adjustment writes `seed ± step` and the write *succeeds* in the sharp case, so
reality snaps to our model after one press. The only cost is that one jump, and the midpoint bounds
it to ≤50 in either direction; seeding at either end would make one of the two hotkeys lurch the
full range the wrong way.

*What marker.* `MonitorState.brightness_known: bool`, false only while the value is a seed. Cleared
by the first evidence either way — a successful read (`update_from_ddc`) or a write the hardware
accepted (`apply_set_result`, both the `Confirmed` and `GroundTruth` branches). Seeding is
**insert-only**, so a monitor that was readable before keeps its real last-known value; the
"last-known or default" of the direction is therefore *both*, resolved by which one exists.

*Whether the OSD shows "unknown".* **No — the tray does instead**, and this is a deliberate
inversion of the direction's fallback. The OSD is the wrong surface: by the time it appears, the
adjustment has already been written, so after the first press the value is authoritative and a
warning styling would flash exactly once and never return. A standing condition needs a standing
signal, so the tray menu prefixes the value with `~` until it is established. It also costs no
`osd_render` work, which the OSD option would have.

**The pre-existing docs were already right; the code was half-built.** `architecture.md:601` and
`CHANGELOG.md`'s 0.8.0 entry both promised that unreadable monitors "stay set-capable … instead of
failing with 'monitor not found'". The worker half shipped (it keeps the handle); the controller
half never did, and the doc's closing clause — "the main thread retains its cached value meanwhile"
— quietly assumes a cached value that a never-read monitor does not have. Worth noting for the other
open rows: a documented behaviour is not evidence of an implemented one.

TDD: 5 tests written first. Three went red on the real defect, all three with
`MonitorNotFound("DEL U2722D")` — the dead keypress, reproduced exactly. Two passed against the stub
(they guard the *new* logic rather than the old bug), so both were verified by injecting the
regression they exist to catch: making the seed unconditional fails
`seeding_never_overwrites_a_monitor_that_already_has_state` with `left: 50, right: 80`, and dropping
the `brightness_known` flag from `update_from_ddc` fails `a_later_successful_read_replaces_the_seed`.

**Side effect on row 13 (DOC-4):** the `MonitorState` sample in `architecture.md` gained
`brightness_known` and lost the stale `last_refresh` line, which is half of that row. The `CLAUDE.md`
half (`pending_brightness` vs the real `pending: Option<PendingSet>`) is untouched and still open.

**I2. A hung-but-alive DDC worker has no recovery path, and the tray tells the user to try one that
cannot work.**
`src/core/controller.rs:427-429`, `:395-402`, `:640`, `:681-687`;
`src/platform/windows/tray.rs:122`; `docs/architecture.md:741-744`.

The design deliberately never respawns a *hung* (as opposed to dead) worker — "that would run two
threads against the same physical-monitor handles" — which is sound. §12 then says the resulting
disabled state "recovers on user activity (a hotkey adjustment) or on system resume". But
`clear_degraded()` only resets `ddc_disabled`, the respawn history and the timeout counter. It
cannot unstick a thread blocked inside a Win32 DDC call. Traced end to end: press → `handle_adjust`
clears the flag (`:427`) → sends another command that queues behind the stuck one → 8 s
`SET_TIMEOUT` → revert + counter++ → three of those → `ddc_disabled` again. The user watches the
tray warning flicker off and back indefinitely; the only real exit is restarting the app.

Two things sharpen it. `ddc_disabled` never gates dispatch — all ten uses gate *respawn* and drive
the tray badge only, so the name overstates what the flag does. And the tray text is a false
affordance exactly where its sibling gets it right: `"⚠ DDC unavailable — press a brightness hotkey
to retry"` (`:122`) versus `"⚠ Hotkeys stopped working — restart the app"` (`:126`).

**Direction:** cheapest honest fix is to distinguish the two causes in `HealthWarnings` and say
"restart the app" for the hang case. Real recovery would mean abandoning rather than respawning the
worker (leak the thread, drop its handles, spawn a replacement) — a genuine design decision, worth
recording either way.

**✅ RESOLVED** — 2026-07-26, commit `43bec28`. **Open decision settled: honest message *and*
automatic recovery, but no abandon-and-respawn.**

*One correction to the finding.* The tray text is not wrong twice — it is right for one of the two
causes. On the backoff path `clear_degraded()` calls `clear_backoff()`, which empties the respawn
history, so the next 250 ms supervision tick sees `!is_alive()` and respawns: "press a brightness
hotkey to retry" describes exactly what happens. Only the hang path is a false affordance. That is
why the fix is a split rather than a rewrite.

*The direction's binary framing missed the option that mattered.* **The proof of life already
arrives and was already being thrown away.** When a blocked DDC call finally returns, the worker
sends its result (`ddc_worker.rs:121`) — that is conclusive evidence the thread is not stuck. The
main thread receives it, and then: only the `Confirmed|GroundTruth` branch reset the timeout counter,
the `Ignored` branch reset nothing, a `DdcRefreshResult` reset nothing at all, and **no result of any
kind cleared the degraded flag**. So recovery from a bounded hang needed no new mechanism, no second
thread and no leak — just acting on a message that was already being delivered. Every worker result
now counts, including failures (a worker reporting a NAK is answering; a hang is about not answering)
and results the reconciler discards as stale.

*Why the keypress recovery had to go, not just the text.* It is what makes the badge lie: the press
clears a flag it cannot fix, so the warning vanishes and returns ~24 s later when the next set times
out. A standing condition needs a standing signal — the same reasoning as row 2's tray marker.
System resume stays a recovery trigger, because a resume plausibly *does* unstick the hardware (the
panel that stalled the bus was likely asleep); a keypress adds no information to the system.

*Abandon-and-respawn: deliberately not built, and the finding under-states its cost.* The design's
stated hazard — "two threads against the same physical-monitor handles" — is not the real one: a
fresh worker calls `GetPhysicalMonitorsFromHMONITOR` again and gets its *own* handles, so aliasing
never arises. The real hazard is the I²C bus underneath. If the old thread is stuck mid-transaction,
the replacement can stall on the same bus, and the result is two hung threads instead of one. Add
permanently leaked `PhysicalMonitor` handles per abandon, a latch to stop an abandon loop, and thus a
*third* respawn policy beside the two STR-1 (row 19) already flags as duplicate — against a failure
mode this review lists under "could not verify". Not worth it at this project's calibration.

*Dispatch is deliberately still not gated while degraded.* Suppressing sets in the hang state was
considered and rejected: a false hang diagnosis would turn a merely slow worker into a dead one, and
the queue is harmless because sets are seq-correlated, so a late-delivered backlog reconciles to the
correct final value.

TDD: 8 tests red first. The most telling was `set_timeout_reverts_counts_and_disables_after_limit`
failing `left: WorkerDead, right: WorkerHung` — the two causes collapsing onto one state, which is
the finding stated as a value.

**A gap the finding did not name, caught by the red run.** Removing the keypress recovery would have
stranded the app: `supervise_worker` returns early for any degraded state, so a worker diagnosed as
hung that *subsequently died* would never be respawned, and nothing else clears that state. Before
this row a keypress covered it by accident. `a_hung_worker_that_later_dies_is_respawned_without_a_
keypress` failed `left: 0, right: 1` (no respawn); death now supersedes the hang diagnosis, since the
reason not to respawn went with the thread. Net result there is better than before: what used to
need a keypress is now automatic.

One test — the `wparam` bit round trip — passed on first write, so it was checked rather than
trusted: shifting the hotkey bit by one decodes "hotkeys lost" as "monitor hung", which is precisely
the silent cross-thread misreport it exists to catch.

**Still unverified:** the hang itself, unchanged from this review's own note. Every rule added here
is testable at the `DdcPort` seam and is tested; whether a real Win32 DDC call ever blocks
unboundedly is not something this codebase can answer.

**I3. `id_cache` is keyed by a handle the OS may recycle, and nothing watches for topology changes.**
`src/core/controller.rs:129`, `:215`, `:356`, `:449-455`.

`MonitorHandle(isize)` carries an `HMONITOR` value; Win32 does not guarantee stability across
display changes. The cache is cleared only at `handle_refresh` begin (`:215`) and pruned per-id
(`:356`). Grep for `WM_DISPLAYCHANGE`/`WM_DEVICECHANGE`: **zero hits** in `src/` — `power.rs`
handles only suspend/resume. So topology change is noticed only by the periodic refresh, and
`refresh.periodic_seconds = 0` is an explicitly *valid* configuration, as is `inactivity_seconds = 0`.
With both disabled, nothing bounds how long a stale mapping survives.

A cache *miss* is self-healing (fresh `resolve_id` → `MonitorNotFound` → refresh). Only a collision
is silent, and it is the bad kind: the OSD appears on the monitor under the cursor while a
*different* monitor's state and hardware are adjusted. The cache itself is well justified —
`get_monitor_id` runs `GetMonitorInfoW` → `EnumDisplayDevicesW` → SetupAPI enumeration → registry
EDID read, far too slow for the hotkey path — so removing it is the wrong fix.

**Direction:** subscribe to `WM_DISPLAYCHANGE` in the existing power message window and send
`Refresh`; that closes the window regardless of refresh config. Cheaper alternative: a TTL on
`id_cache` entries independent of `periodic_seconds`.

**✅ RESOLVED** — 2026-07-26, commit `d194bd2`. **The direction above was wrong as written** and
implementing it literally would have produced dead code: the listener's window was *message-only*
(`HWND_MESSAGE`), and message-only windows are excluded from broadcast messages — which
`WM_DISPLAYCHANGE` is. That is precisely the trap that cost this module its resume detection in the
previous cycle, reproduced one message later. The window is therefore now a hidden **top-level**
window (`WS_POPUP | WS_EX_TOOLWINDOW`, never shown), and `RegisterSuspendResumeNotification` is
retained as the documented delivery guarantee for `WM_POWERBROADCAST`, so resume detection now has
two independent paths rather than one.

TDD: two tests written failing first — `wnd_proc_forwards_display_change_as_refresh` (dispatch), and
`listener_window_is_top_level_so_broadcasts_arrive`, which pins the window's `GA_PARENT` ancestor to
the desktop. The second test is the durable one: it caught that `GetParent` is *not* a usable
discriminator here (it reports the owner, null for both window kinds), and its red run printed the
proof — parent `HWND(0x10012)` versus desktop `HWND(0x10010)`, two different pseudo-desktops.
Absence evidence is deliberately not reset on this trigger, so a monitor genuinely unplugged still
ages out and is pruned. architecture.md §9 gained the trigger row, a behaviour bullet, and a
correction to its own stale "message-only" claim.

**Still unverified on hardware:** that the refresh actually fires on a real plug/unplug, and the
sleep/wake resume test — now doubly relevant, since this commit changed the window type that path
depends on.

**I4. If file logging fails to attach at startup, the warning goes only to the console that release
builds hide.**
`src/main.rs:552-562`, `:311-330`, `:263-269`.

`build_file_logger` failure is reported with `log::warn!` — but the file sink is by definition not
attached at that point, so `TeeLogger` has nowhere to tee it. In release the console is hidden, so a
user who set `logging.file_enabled: true` to diagnose a problem gets a silently empty log directory:
unwritable `%APPDATA%`, a locked file, or a full disk all present as "the feature just doesn't
work". This is the one path in an otherwise well-built logging subsystem where the failure of the
diagnostic channel is reported *through* that same channel. Mid-run rotation failures, by contrast,
are handled and tested (`logfile.rs`).

**Direction:** surface it through a channel that exists in release — the tray degraded-state path
already carries exactly this kind of news — or a one-time message box.

**✅ RESOLVED** — 2026-07-26, commit `311560d`. **Open decision settled: the tray, not a message
box.** `HealthWarnings` gains `file_log_failed`; the controller latches it via
`set_file_log_failed()`, and `main.rs` carries the attach outcome past the controller's construction
(the failure at `:552` predates both controller and tray by design — the file log must attach as
early as possible so startup lines land in it).

*Why not the message box.* Both existing startup message boxes (single-instance, hotkey registration)
announce conditions where the app **cannot do its job**, and both `return` immediately after. Here
the app runs normally. A modal box also blocks the startup path of something typically launched at
logon, would reappear every boot for a transient cause such as a scanner holding the file, and is
flash-not-standing — the same property that decided rows 2 and 9 the other way. Three consecutive
rows asking "transient signal or standing one?" should not get three different answers.

*Why the tray is the right surface specifically.* The affected user is, by definition, someone who
set `file_enabled = true` and later goes looking for the log — and the tray menu already carries
"Open Log Folder", so it sits on that user's guaranteed path. The warning line therefore points at
the folder rather than explaining an I/O error.

*One sub-decision the finding did not raise: the badge.* A failed file log is the one warning that
deliberately does **not** raise the icon's amber badge, which was factored out of `handle_status_update`
into a testable `wants_warning_badge`. The badge means the app cannot do its job; a missing
diagnostic log does not stop a single adjustment, and letting it in would weaken the signal for the
two conditions that do. Menu and tooltip only.

**The finding missed an inconsistency inside the module.** Right next door, `logfile.rs:56-61`
handles a failed *rotation* by degrading to an oversized file and retrying on the next over-cap
write — "logging never stops". The *attach* is the one place in the file-log design that gives up
permanently after a single try (`TeeLogger.file` is a `OnceLock`, `build_file_logger` is called
once), so a transient lock costs the whole session's file log. **Deliberately not fixed here:** a
retry addresses the transient causes, while this row is about the persistent ones (unwritable
directory, missing `APPDATA`) presenting as silence — and a periodic filesystem probe sits awkwardly
beside row 8, which questions this app's wake cadence in the first place. Recorded as the right
follow-up if transient attach failures are ever actually observed.

TDD: 5 tests red first, including the round trip losing the new state across the `wparam` hop
(`file_log_failed: true` in, `false` out). The badge test passed on the stub — it asserts an
exclusion, and the stub excluded by ignorance — so it was checked by injection: adding
`|| warnings.file_log_failed` to `wants_warning_badge` fails it.

**Still unverified:** the release-build behaviour on a genuinely unwritable `%APPDATA%`. The
warning-line and badge logic are pure functions and tested; the `main.rs` wiring that feeds them is
three lines in the binary and is covered only by the manual step added to architecture.md §14.

**I5. A dead field carries a comment instructing future maintainers to maintain an invariant that
does not exist.**
`src/platform/windows/ddc_worker.rs:33`, `:51`, `:140`, `:178`, `:29-32`;
`src/core/controller.rs:125-128`.

`DdcWorker.handle_cache` is declared, initialised, cleared, inserted into, and asserted empty by one
test — and **never read**. There are exactly five occurrences and none is a lookup.

The aggravating half is the comment. Both files carry mirrored text asserting that *"The core
controller / DDC worker keeps its own independent handle→id cache … each side invalidating on
refresh under its own rules. **Changes to handle→identity mapping must cover both.**"* Half of that
is fiction: `Controller.id_cache` is the only handle→id cache. A future maintainer touching identity
mapping — exactly what I3 requires — is told to keep two things in sync when one is a no-op. On the
change-safety axis that is worse than an uncommented dead field, which is why it ranks here rather
than in the cosmetics. Last cycle's dead-state prune missed it.

**Direction:** delete the field and both paired comments.

**✅ RESOLVED** — 2026-07-26, commit `a2a9896`: field, its initialiser, its `clear`, its `insert`,
the now-unused `hmonitor_to_isize` import, and the test assertion all deleted. The worker's struct
doc now states the actual invariant — commands address monitors by identity, never by platform
handle, so the worker needs no handle mapping of its own — and `Controller.id_cache`'s comment says
it is the only one, why it exists (a display-device enumeration plus a registry EDID read is far too
slow for the hotkey path), and when it is invalidated.

**I6. The `publish = false` lib façade disables `dead_code` across the whole platform layer.**
`src/lib.rs`; `src/platform/windows/osd.rs:553`, `:653`; `tray.rs:950`, `:956`; `mod.rs:180`, `:202`.

Everything `pub` in the lib is reachable as far as rustc is concerned, so the compiler never reports
unused items across ~24 modules — while the lib exists only so `core/` is host-testable and three
integration tests can reach the platform layer. Confirmed orphans: `OsdWindow::show_error` (zero
callers — and it was *retired by design*, since the controller deliberately never spontaneously
shows a hidden OSD in error state, `controller.rs:610-613`), `OsdWindow::hwnd`, `TrayIcon::hwnd`,
`TrayIcon::sender`, plus `SafeHwnd::new_borrowed`/`into_raw` used only by their own unit tests. This
is one root cause behind several residues rather than six separate nits — including I5.

**Direction:** demote to `pub(crate)`/`pub(super)` everything the binary and `tests/` do not name;
that restores the compiler as the dead-code detector.

**✅ RESOLVED** — 2026-07-26, commit `6acda0c`. 259 demotable items → **116 `pub`, 131
`pub(crate)`**, net 158 lines removed. **This row was under-sized at S**; it is an M. The reason is
a mechanic the finding did not state: demoting a *module* buys nothing for a type that module
re-exports, because a reachable type keeps every one of its `pub` methods reachable. Narrowing the
five non-externally-named modules (`edid`, `ddc_worker`, `power`, `tray`, `usage`) produced **zero**
new warnings on its own. Detection only came back with per-item demotion.

Method: bulk-demote everything, then let rustc name the real surface — it is the only authority here,
since the binary is a *separate crate* and `pub(crate)` is invisible to it. Four error shapes had to
be handled to converge (E0603 private item, E0624 private method, E0616 private field, and the
span-less "type … is private"). Verified two ways: `RUSTFLAGS="-W unreachable_pub" cargo clippy
--lib` reports nothing, so no item is gratuitously `pub`; and re-demoting `RefreshConfig` /
`MonitorConfig` proved they were an over-restore on my part, now corrected.

**Found and deleted (~15, none predicted beyond the six in the finding):** `OsdWindow::show_error`
(retired by design), `::hide` (the auto-hide timer in the window proc owns hiding) and `::hwnd`;
`TrayIcon`'s `sender` field plus both accessors — the window procedure reaches the channel through a
thread-local, so the struct's copy was unreachable, and dropping it made the constructor's `.clone()`
needless; `DdcMonitor::id`, `::cached_brightness` **and the backing field** — closing out the
write-only field flagged in row 1's note; `HotkeyManager::unregister_hotkey`; `Config::load`
(superseded by `load_or_recover`); the `TrayMenuCreation` error variant and its constructor;
`BRIGHTNESS_MIN`; `TrayMenuData`'s `hotkey_up`/`hotkey_down` (dead payload — the menu offers a
"Usage" item that opens the usage window instead); and `SafeHwnd::new_borrowed` / `into_raw`.

Two judgement calls worth flagging. **`SafeHwnd` lost its `owned` flag**: with borrowed construction
gone, every wrapped window is one this crate created, so the flag encoded a distinction that no
longer existed — the same hazard as row 3's fictional invariant. Its test was rewritten against
`new_owned` (a null handle is inert on drop) and the `into_raw` test deleted, since it exercised only
the deleted method. **Two of the four pre-existing `#[allow(dead_code)]` were masking real dead
code** (`SafeHook::as_raw`/`is_valid`, one carrying a doc comment claiming a `CallNextHookEx` caller
that does not exist) and are gone; the other two are genuine RAII keep-alives with clear comments and
stayed.

**Bonus, and the reason the row is worth more than its size:** `clippy::unnecessary_wraps` skips
`pub` functions because the signature is API, so narrowing surfaced three infallible functions
returning `Result`. `overlay::show_window`/`hide_window` and their two callers now return `()`;
`OverlayManager::update` stays fallible through `set_opacity`, so the cascade stopped at one level.

**One trap for next time, now documented in `code-conventions.md`:** `cargo clippy --all-targets`
does **not** compile doc examples. The sweep passed clippy and the whole test suite except the
`lib.rs` doctest, which is a consumer of the public API like any other — `calculate_adjustment` and
`BrightnessAdjustment` are `pub` because that example names them, nothing else.

**I7. The only DDC integration test cannot fail.**
`tests/ddc_test.rs`.

Every branch `println!`s; there is not one assertion in 57 lines. It returns `Ok(())` whether EDID
parsing worked, physical-monitor handles opened, or brightness reads failed. It passed in this
session's run and it passes on CI's headless `windows-latest` VM, contributing to a green tally
while verifying nothing. The project's policy — DDC is verified manually — is correct and
documented; the problem is that the file's *existence* implies automated coverage of the riskiest
subsystem, discoverable as a no-op only by reading it.

**Direction:** `#[ignore]` plus a how-to-run comment, and either assert what *is* assertable on any
Windows host (that identification succeeds for every enumerated monitor) or rename it to say it is a
manual probe.

**✅ RESOLVED** — 2026-07-26, commit `fc9c65b`: both, rather than either. Renamed to
`ddc_hardware_probe`, `#[ignore]`d with the run line in the reason string and a module doc
explaining that `--nocapture` *is* the point (the printed per-monitor breakdown is the diagnostic
value). Identification is deliberately **not** asserted per monitor — a virtual or RDP display can
legitimately fail it. The two assertions are the ones that would make the probe itself meaningless:
nothing enumerated, or not one monitor answering a VCP 0x10 read, the latter with a message naming
the usual causes (monitor DDC/CI setting, cable, KVM).

Verified both ways: `cargo test` reports it ignored; `cargo test --test ddc_test -- --ignored
--nocapture` passes and prints. **That run also produced a real data point for C1** — the attached
Philips 346B1C reports `Max: 100`, so C1 does not currently misfire on this machine. It confirms
the defect is hardware-conditional, and it is the first empirical evidence either way.

**I8. The steady-state wake cadence of an always-on background app is an unowned decision.**
`src/main.rs:642-727` (`recv_timeout(Duration::from_millis(16))`).

The main loop wakes ~62×/s for the entire process lifetime — for a tool whose actual work is a few
keypresses a day. The 16 ms figure only matters while the OSD is visible; the rest of the loop is
throttled internally anyway (`supervise_and_watchdog` self-limits to 250 ms, `controller.rs:630`).
Nothing in `architecture.md` mentions the cadence or its cost, so it reads as accreted rather than
chosen. For a laptop tray utility, "how often do we wake when idle" deserves a sentence.

**Magnitude unmeasured** — no profiling was done, and per-wake work is trivial; this is a
design-ownership finding, not a measured regression. **Direction:** an adaptive timeout (16 ms while
the OSD is visible, ~250 ms otherwise) is a few lines and changes nothing observable; either way,
record the choice.

**✅ RESOLVED** — 2026-07-26, commit `b09ff80`. **Open decision settled: accept and document, not
adapt.** No behaviour change; `architecture.md` §"Main-Loop Cadence" now carries the reasoning and
the number, and `main.rs` points at it from the `recv_timeout` site.

*The magnitude is no longer unmeasured, and that is what decided it.* A release instance had been
running 11.9 h: **0.72 s of CPU total — 0.0017 % of one core**, ~60 ms per hour, and that includes
startup, EDID parsing, window creation and every DDC refresh in the window. Sampled over 30 s of
idle the accumulated CPU time did not advance at all; ~0.27 µs per wake is below the scheduler's
accounting granularity. The adaptive variant is real and would cut idle wakes 15×, but the saving it
buys is **~1.2 s of CPU per day**, against a predicate that can silently rot — a window added to the
main thread later has to be remembered in it, and forgetting produces a sluggish UI, which no test
catches. Before the measurement the adaptive option looked like the obvious trade; the number
inverted it.

*Two corrections to the finding, in opposite directions.* "The 16 ms figure only matters while the
OSD is visible" is wrong twice over. It **over-counts the overlay**, which needs no pumping at all —
a layered `LWA_ALPHA` window with no `WM_PAINT` handler, composited by the DWM — and the overlay is
precisely the long-lived one, so an adaptive predicate would have had to *exclude* it deliberately or
keep the fast path running for hours through the app's headline feature. And it **misses the usage
window** entirely: created on the main thread (`main.rs:126`), no message loop of its own, and the
only main-thread window that takes input at all — the OSD and overlay are both `WS_EX_TRANSPARENT`.
So the one surface that genuinely justifies a fast pump is the one the finding does not mention.

*What the interval actually governs, which reframes the whole row.* Not input latency: any `send()`
wakes `recv_timeout` immediately, so hotkeys, DDC results and the tray's menu-data request never wait
it out, and the first OSD paint happens on the next loop top rather than after a timeout. It bounds
only how late an *unsolicited* window message is noticed. Its bounds were already fixed by two
existing decisions — `OSD_TIMEOUT_MIN = 100 ms` below (an auto-hide must not visibly overshoot) and
Windows' 5 s hang-detection above — which leaves roughly 25 ms…1 s admissible. That the value sits at
the responsive end of a legitimate range is a much smaller charge than "accreted rather than chosen".

*The event-driven option was rejected with a sharper reason than expected.* `std::sync::mpsc` exposes
no waitable handle, so `MsgWaitForMultipleObjects` needs a wake-post from all four sender threads,
and a forgotten post delays a message silently. But the decisive point is that supervision still
needs a ~250 ms timer, so a full transport rewrite lands at **the same wake rate as the ten-line
adaptive variant**.

### Cosmetic / nice-to-have

**Docs & changelog** _(DOC-1 … DOC-5)_
- `CHANGELOG.md`'s `[Unreleased] → Fixed` omits `922bc21` (bounding the hotkey-thread startup wait
  to 5 s) — a real reliability fix, and the one commit since the `0.8.0` tag not represented.
  `RELEASING.md` makes the changelog the source for release notes.
- `architecture.md:261` promises hand-written `monitors` entries "are preserved and round-trip
  through saves", but `MonitorConfig` (`config.rs:100-107`) hard-codes three fields, so any other
  sub-key is dropped by serde — and `refresh_backup` (`config.rs:485`, called unconditionally from
  `load_or_recover`) writes the reshaped form to the `.bak`. The same paragraph already hedges that
  the value shape "is not a contract yet", so this is an internal contradiction rather than a broken
  promise. Either loosen the type to `serde_json::Value` until the schema lands, or drop the
  round-trip sentence.
- `architecture.md`'s module tree omits `core/panic_hook.rs` (`edid.rs` and `reconcile.rs` are
  listed).
- Sample drift on the same struct in two docs: `architecture.md`'s `MonitorState` sample still lists
  `last_refresh` (pruned; it lives on `RefreshTracker`, `reconcile.rs:131`), and `CLAUDE.md`'s
  summary says `pending_brightness` where the field is `pending: Option<PendingSet>`.
- `improvement-ideas.md`'s "Automated testing suite" bullet is framed as if none exists, next to two
  entries that correctly carry a "partially implemented" note. 191 tests exist.

**Dependencies** _(DEP-1 … DEP-3)_
- No dependency-freshness mechanism: no Dependabot or Renovate config; CI's weekly `cargo audit`
  covers advisories only. The repo's own history says freshness, not advisories, is the failure mode
  — the `windows` pin drifted from Nov 2023 to 2026-07 and moved only because a review asked, and
  `thiserror` sat on 1.0 for ~18 months and was fixed as a *cosmetic* item. Both caught by human
  review, never tooling.
- `winres` 0.1.12 is the stalest thing in the tree and the one dependency not covered by the
  documented maintenance posture, though it is load-bearing (`build.rs` does
  `res.compile().unwrap()`). It also drags in `toml 0.5`. Upstream is believed unmaintained with
  `winresource` as the maintained drop-in — **not verified against crates.io; confirm before
  acting**, then either swap or record a deliberate-pin note beside the others.
- `Cargo.toml` enables the `Win32_UI_Controls` feature; zero `UI::Controls` paths in `src/` or
  `tests/`. It was added for `SS_LEFT`, which was later removed. Compile-only change — it needs none
  of the hardware pass the manifest reserves for version bumps.

**Structure** _(STR-1 … STR-2)_
- Two implementations of one respawn policy: `RespawnGate` (`reconcile.rs:70-109`, sliding window +
  latching `gave_up`, 4 unit tests) versus `DdcSupervisor::respawn` (`ddc_worker.rs:286-299`, its
  own `recent_respawns` Vec, no latch — `Controller.ddc_disabled` plays that role). `RespawnGate`
  was the later generalisation for the hotkey thread and was never retrofitted to the original
  client. Consequence: `FakeDdc` returns a canned `RespawnOutcome`, so the real DDC backoff wiring
  has no test — only the primitives do.
- Two tests assert nothing (`mod.rs:276`, self-documented "just verify it doesn't panic", and
  `ddc_worker.rs:365`). 2 of 191; listed only for completeness with I7.

**Config & release UX** _(UX-1 … UX-3)_
- Three config snapshots, no reload path: `main.rs:581` moves a clone into the Controller, `:424`
  clones another into the hotkey thread, main keeps the original. Immutable after load, so no race —
  but the tray's "Settings" item opens `config.json` for editing (`:141-150`) and nothing reloads
  it, with no hint at the menu item. This is what has to change first if the settings GUI on the
  roadmap lands.
- `release.yml` publishes an unsigned `.exe` with no checksum. Fine for a solo tool; a `sha256` line
  in the release body is cheap if download-integrity ever comes up.
- `docs/research-findings.md` (untracked) is a scratch file of research *prompts* about replacing the
  hand-rolled tray/hotkey/window/config/power/EDID code with crates (`tray-icon`, `global-hotkey`,
  `winit`/`tao`, `edid-rs`), sizing the target at ~2,000 lines of Win32 boilerplate. No decision is
  recorded anywhere in the docs. Either record the decision in `improvement-ideas.md` or delete it —
  and note that a file named "findings" containing none will mislead if it is ever committed.

  **✅ RESOLVED** — 2026-07-26, commit `df1992b`: deleted, with no note kept. The warning about
  committing it turned out to be overtaken — it *had* been committed, accidentally, by a `git add -A`
  in `6acda0c` earlier in this same cycle.

  Dated 2026-01-10 and stale in five of its six sections: it has config resolving paths through
  `dirs_next` (removed in 0.8.0), EDID parsing inside `ddc.rs` (extracted to `core/edid.rs`), the
  usage window inside `mod.rs` (now `usage.rs`), `osd.rs` at ~800 lines (573, since `osd_render.rs`
  was split out) and the tray at ~500 (1064). Its last 30 lines are a raw transcript dump from
  another tool — token counts, cost, colour markup.

  No decision note was kept, deliberately: the six months since were themselves the decision. Not one
  of the prompts was ever run, and the codebase moved the other way — extracting `core/edid.rs`,
  splitting `osd_render.rs`, introducing the seam traits, upgrading `windows` to 0.62, *reducing*
  third-party dependencies. Writing "we considered crates and declined" would have documented a
  deliberation that never happened.

**✅ RESOLVED (DEP-3, DOC-1, DOC-3, DOC-4, DOC-5, UX-1, UX-2, STR-2)** — 2026-07-26, commit
`59e29e5`, one pass over the eight mechanical rows. fmt, clippy and 216 tests green. Four of them
turned out to carry more than the finding described.

*STR-2 is three tests, not two* — `test_ddc_worker_shutdown` asserts as little as its two named
siblings. More usefully, the property all three were reaching for is **termination**, which no
assertion can express: a worker that fails to stop hangs the run rather than failing it. What *is*
assertable is how it stopped, and one `try_recv` after `run(self)` separates every outcome — a value
means a result was emitted during shutdown, `Empty` means the sender outlived the call, `Disconnected`
means a clean silent exit. Both worker tests now go through that check.

*The third one was worth keeping, not deleting.* `get_last_error_code` reinterprets a signed OS error
as unsigned, and Windows' HRESULT-shaped codes have the high bit set — so the plausible "safer"
rewrite, `u32::try_from(..).unwrap_or(0)`, would turn a real failure code into *no error*. The test
now pins that round trip, and it was verified by injecting exactly that rewrite: `left: 0, right:
2147942405`. A test asserting nothing sat on top of a genuine trap.

*DOC-4's `CLAUDE.md` half was understated.* Beyond the named `pending_brightness`, the sample was also
missing `brightness_known` (added by row 2) and `missing_since`. The drift was three fields, not one.

*DOC-5 is not just a label change.* With 216 tests the entry needed a statement of what actually
remains, and it is not laziness: end-to-end coverage across Windows versions and monitor models is
unavailable to a headless CI runner, which has no DDC-capable display. That is the same constraint
that makes `tests/ddc_test.rs` an ignored manual probe and keeps a hardware checklist in the docs, so
the three now say one consistent thing.

*Verification limits.* DEP-3's removal is proven by a clean build — the feature gated bindings nothing
imported. UX-2 cannot be exercised until a tag is pushed; the PowerShell that builds the release body
was run locally against a stand-in file and the workflow YAML was parsed, so what remains untested is
the `gh release create` invocation itself, which the first release will settle either way.

## Resolution plan

**Progress: every Critical and Important row resolved 2026-07-26** on branch
`arch-review-fixes-2026-07-26` (`a2a9896`, `fc9c65b`, `d194bd2`, `7a1607a`, `6acda0c`, `5c8a653`,
`43bec28`, `311560d`, `b09ff80`) — fmt, clippy (`-D warnings`, `--all-targets`) and the full suite
green at each commit; 216 tests now (+2 for the display-change path, +7 for VCP scaling, +5 for the
unreadable-monitor path, +9 for the split DDC health state, +4 for the file-log warning, −1 for a
test that only exercised a deleted method; the DDC probe is ignored by default). Row 8 alone landed
as documentation with no code change, on a measurement. Of the cosmetics, row 16 (`df1992b`) and the
eight mechanical rows (`59e29e5`) are also done, leaving **four: 15, 17, 18 and 19** — the two that
need a decision, the Dependabot config, and the one real code change among them.

Five lessons worth carrying. Row 5's stated direction was wrong and only the red run exposed it
(see I3). Row 1's round-trip test *passed* on the deliberately-wrong stub, so it was checked against
an injected rounding regression rather than trusted (see C1) — a test that has never failed is not
yet evidence of anything; row 9's bit round trip was checked the same way for the same reason. Row 7
was sized S when it is an M, because the finding named the symptom (`pub mod` suppresses
`dead_code`) without the mechanic that makes it expensive to fix (a re-exported type keeps all its
methods reachable, so module-level demotion alone changes nothing). And row 9 shows the cost of a
finding that frames its remedy as a binary: "fix the message or build abandon-and-respawn" hid a
third option that needed neither, because the evidence the expensive one would have manufactured was
already being delivered and discarded. And row 8 is the one row where **measuring first changed the
answer**: ten lines for a 15× cut in idle wakes reads as an obvious trade until the thing being cut
turns out to be 0.0017 % of a core. A finding that says its own magnitude is unmeasured should be
measured before it is implemented, not after.

**On this review's own accuracy:** every finding acted on has been real, but four of eight carried a
wrong, incomplete or too-narrow *direction* (rows 5, 7, 8 and 9) and one under-sized the work. Three
rows also turned up something adjacent to the defect described — row 2 found the docs promising
behaviour that was only half-built, row 9 found that removing the useless recovery would strand a
hung-then-dead worker, row 6 found the attach to be the only part of the file log that gives up
after one try while its neighbour retries forever. The findings have held up better than the
remedies attached to them, and the code *around* a finding has been worth reading as carefully as the
finding itself.

Ordered tackle list. **Complexity**: XS ≈ minutes, one edit site · S ≈ an hour-ish, one module plus
tests/docs · M ≈ multi-site change needing real design care. **Clarity**: Clear = mechanical,
direction fully specified · Mostly clear = one small implementation choice with an obvious default ·
Needs decision = genuine open question to settle before implementation. Ordering: severity first;
within severity, clear quick wins before larger items, decision-blocked items last.

C1 and I1 were listed adjacent on the expectation of a single pass. In the event they proved
separable: C1 resolved entirely inside `ddc.rs`/`ddc_worker.rs` without touching
`handle_ddc_refresh_result` or the `(MonitorId, u8)` result-tuple shape, which is where I1 lives.
Row 2 is therefore a standalone controller-side change, not the second half of row 1.

| #  | ID    | Issue (anchor)                                                          | Severity  | Cx | Clarity      | Open decision |
|----|-------|-------------------------------------------------------------------------|-----------|----|--------------|---------------|
| 1  | C1    | Scale VCP 0x10 by reported max; stop truncating (`ddc.rs:311`)           | Critical  | M  | Mostly clear | ✅ resolved `7a1607a` — settled on scale-to-percent at the boundary |
| 2  | I1    | Seed state for enumerable-but-unreadable monitors (`controller.rs:285`)  | Important | S  | Needs decision | ✅ resolved `5c8a653` — seed 50 + `brightness_known`; marker in the tray, not the OSD |
| 3  | I5    | Delete dead `handle_cache` + both fictional comments (`ddc_worker.rs:33`)| Important | XS | Clear        | ✅ resolved `a2a9896` |
| 4  | I7    | Mark the DDC probe manual-only (`tests/ddc_test.rs`)                    | Important | XS | Clear        | ✅ resolved `fc9c65b` |
| 5  | I3    | `WM_DISPLAYCHANGE` → `Refresh` in the power window (`controller.rs:129`) | Important | S  | Clear        | ✅ resolved `d194bd2` — direction was wrong; window had to become top-level |
| 6  | I4    | Surface a failed file-log attach in release (`main.rs:552`)              | Important | S  | Mostly clear | ✅ resolved `311560d` — tray menu line + tooltip, deliberately no icon badge |
| 7  | I6    | Demote the lib's incidental `pub` surface (`lib.rs`)                    | Important | S→M| Clear        | ✅ resolved `6acda0c` — sized S, was an M |
| 8  | I8    | Adaptive main-loop cadence + document it (`main.rs:699`)                 | Important | XS | Mostly clear | ✅ resolved `b09ff80` — measured at 0.0017 % of a core, so accept-and-document; adaptive declined |
| 9  | I2    | Hung-worker recovery: honest message or real abandon (`tray.rs:122`)     | Important | S–M| Needs decision | ✅ resolved `43bec28` — split the state, act on proof of life; abandon-and-respawn declined with reasons |
| 10 | DEP-3 | Drop the dead `Win32_UI_Controls` feature                               | Cosmetic  | XS | Clear        | ✅ resolved `59e29e5` |
| 11 | DOC-1 | CHANGELOG: add the hotkey-startup-wait fix                              | Cosmetic  | XS | Clear        | ✅ resolved `59e29e5` |
| 12 | DOC-3 | Add `panic_hook.rs` to the module tree                                  | Cosmetic  | XS | Clear        | ✅ resolved `59e29e5` |
| 13 | DOC-4 | Fix `MonitorState` sample drift in architecture.md + CLAUDE.md          | Cosmetic  | XS | Clear        | ✅ resolved `5c8a653` + `59e29e5` — CLAUDE.md drifted on three fields, not one |
| 14 | DOC-5 | Mark "Automated testing suite" partially implemented                    | Cosmetic  | XS | Clear        | ✅ resolved `59e29e5` — names what CI structurally cannot cover |
| 15 | DOC-2 | Reconcile the `monitors` round-trip claim with `MonitorConfig`           | Cosmetic  | S  | Mostly clear | Loosen the type vs. soften the doc |
| 16 | UX-3  | Decide and record the hand-rolled-vs-crates question; drop the scratch   | Cosmetic  | XS | Needs decision | ✅ resolved `df1992b` — file deleted, no note kept; six months of opposite-direction work were the decision |
| 17 | DEP-2 | Confirm `winres` upstream status; swap to `winresource` or pin-note      | Cosmetic  | S  | Needs decision | Requires a crates.io check first |
| 18 | DEP-1 | Add Dependabot (weekly, grouped, PRs only)                              | Cosmetic  | XS | Mostly clear | — |
| 19 | STR-1 | Fold `DdcSupervisor` onto `RespawnGate`; test the real wiring            | Cosmetic  | S  | Clear        | — |
| 20 | UX-1  | One sentence on config snapshots / no live reload                       | Cosmetic  | XS | Clear        | ✅ resolved `59e29e5` |
| 21 | UX-2  | Checksum in the release body                                            | Cosmetic  | XS | Clear        | ✅ resolved `59e29e5` — untested until the first tag |
| 22 | STR-2 | Give the two no-assert tests assertions or delete them                   | Cosmetic  | XS | Clear        | ✅ resolved `59e29e5` — three, not two; all three kept and given assertions |

## Categories reviewed

| Category | Status | Reason / scope |
|---|---|---|
| Structure & Modularity | Applicable | Reviewed; boundary grep-verified real. Source of I6, STR-1. |
| Dependencies | Applicable | Reviewed; 6 direct runtime deps + 1 build-dep, all used, 49-package lock tree. Source of DEP-1…3. |
| Data Flow & State | Conditional → applicable | The app's central concern is shared mutable multi-thread state. Source of I1, I3, I5. |
| Interfaces & Contracts | Conditional → narrow | Only external contracts: `config.json`, hotkey/tray UX, single-instance mutex name. No code consumers ⇒ no API/SemVer review. |
| Error Handling & Resilience | Applicable | Both halves reviewed — DDC I/O fails routinely. Source of I2. |
| Testability & Tests | Applicable | Reviewed **and executed** (191 green). Source of I7. |
| Configuration & Secrets | Applicable | Reviewed; never-fatal policy holds, atomic writes, `.bak` chain, unknown-key diff. |
| Operations & Observability | Conditional → applicable | Long-running background process. Source of I4, I8. |
| Security | Conditional → local-app depth | No network or service surface ⇒ no AuthN/AuthZ review. Trust boundaries reviewed: EDID, registry, config file, window messages, opt-in LL hook, single-instance object. |
| Documentation & Evolvability | Applicable | Reviewed; doc discipline is genuinely strong, which is why the drift items are worth naming. |

## What I could not verify

- **Hardware behaviour**, which is C1's frequency question: whether any monitor in the author's
  fleet reports a VCP 0x10 max ≠ 100. The defect is static; the blast radius is not. _Partially
  answered 2026-07-26 by the reworked manual probe: the attached Philips 346B1C reports `Max: 100`,
  so C1 is latent, not active, on this machine. Any other monitor remains unchecked — re-run
  `cargo test --test ddc_test -- --ignored --nocapture` when one is attached._
- **I1's sharp case** — a panel that persistently NAKs VCP reads while honouring writes — is reasoned
  from the worker's own rationale for keeping the handle; no evidence such a panel is in use.
- **I2's hang** is reasoned from code, not reproduced; no test can model a truly blocking Win32 call.
  _Unchanged by the row-9 fix (`43bec28`): every rule it adds is tested at the `DdcPort` seam, but
  whether a real DDC call ever blocks unboundedly — the question that decides whether declining
  abandon-and-respawn was right — remains open._
- **I3's premise** that Windows actually recycles `HMONITOR` values on the dock/undock transitions
  this app sees. The hazard follows from the API contract; the decisive check is a manual
  dock/undock with `refresh.periodic_seconds = 0`.
- ~~**I8's magnitude** — no profiling of idle CPU or energy impact was done.~~ _Answered 2026-07-26:
  a release instance at 11.9 h uptime had used 0.72 s of CPU, i.e. **0.0017 % of one core**, with no
  measurable advance over a 30 s idle sample. That number decided row 8. The **energy** half stands
  open — CPU time does not capture the cost of denying the core deeper idle states, and no battery
  profiling was done; it is bounded by the wake count and unremarkable beside any GUI process on the
  same machine, but it is an argument, not a measurement._
- **`winres`/`winresource` upstream status and dates** (DEP-2) — asserted from recollection, no
  network check.
- **Live RustSec advisory state** — `cargo audit` is not installed locally; CI's weekly job is the
  standing authority.
- **The outstanding manual passes the project's own docs call for**: the sleep/wake System Resume
  Test and the full hardware pass for the 0.62 migration (DDC, OSD, overlay, tray, hotkeys, power
  events).
- `tray.rs` (1069 lines) and `hotkey.rs` (808) were pattern-swept for FFI red flags and read at the
  seams, not line-audited end to end.
