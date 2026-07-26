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

## Resolution plan

**Progress: rows 1 and 3–5 resolved 2026-07-26** on branch `arch-review-fixes-2026-07-26`
(`a2a9896`, `fc9c65b`, `d194bd2`, `7a1607a`) — fmt, clippy (`-D warnings`, `--all-targets`) and the
full suite green at each commit; 200 tests now (+2 for the display-change path, +7 for VCP scaling,
the DDC probe moved to ignored-by-default). Rows 2 and 6+ remain open.

Two lessons worth carrying, both from writing the test first. Row 5's stated direction was wrong and
only the red run exposed it (see I3). Row 1's round-trip test *passed* on the deliberately-wrong
stub, so it was checked against an injected rounding regression instead of being trusted (see C1) —
a test that has never failed is not yet evidence of anything.

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
| 2  | I1    | Seed state for enumerable-but-unreadable monitors (`controller.rs:285`)  | Important | S  | Needs decision | What value/marker to seed; whether the OSD shows "unknown" |
| 3  | I5    | Delete dead `handle_cache` + both fictional comments (`ddc_worker.rs:33`)| Important | XS | Clear        | ✅ resolved `a2a9896` |
| 4  | I7    | Mark the DDC probe manual-only (`tests/ddc_test.rs`)                    | Important | XS | Clear        | ✅ resolved `fc9c65b` |
| 5  | I3    | `WM_DISPLAYCHANGE` → `Refresh` in the power window (`controller.rs:129`) | Important | S  | Clear        | ✅ resolved `d194bd2` — direction was wrong; window had to become top-level |
| 6  | I4    | Surface a failed file-log attach in release (`main.rs:552`)              | Important | S  | Mostly clear | Tray degraded-state channel vs. one-time message box |
| 7  | I6    | Demote the lib's incidental `pub` surface (`lib.rs`)                    | Important | S  | Clear        | — |
| 8  | I8    | Adaptive main-loop cadence + document it (`main.rs:699`)                 | Important | XS | Mostly clear | Adaptive timeout vs. accept-and-document |
| 9  | I2    | Hung-worker recovery: honest message or real abandon (`tray.rs:122`)     | Important | S–M| Needs decision | Fix the message only, or implement abandon-and-respawn |
| 10 | DEP-3 | Drop the dead `Win32_UI_Controls` feature                               | Cosmetic  | XS | Clear        | — |
| 11 | DOC-1 | CHANGELOG: add the hotkey-startup-wait fix                              | Cosmetic  | XS | Clear        | — |
| 12 | DOC-3 | Add `panic_hook.rs` to the module tree                                  | Cosmetic  | XS | Clear        | — |
| 13 | DOC-4 | Fix `MonitorState` sample drift in architecture.md + CLAUDE.md          | Cosmetic  | XS | Clear        | — |
| 14 | DOC-5 | Mark "Automated testing suite" partially implemented                    | Cosmetic  | XS | Clear        | — |
| 15 | DOC-2 | Reconcile the `monitors` round-trip claim with `MonitorConfig`           | Cosmetic  | S  | Mostly clear | Loosen the type vs. soften the doc |
| 16 | UX-3  | Decide and record the hand-rolled-vs-crates question; drop the scratch   | Cosmetic  | XS | Needs decision | Whether to pursue crate replacement at all |
| 17 | DEP-2 | Confirm `winres` upstream status; swap to `winresource` or pin-note      | Cosmetic  | S  | Needs decision | Requires a crates.io check first |
| 18 | DEP-1 | Add Dependabot (weekly, grouped, PRs only)                              | Cosmetic  | XS | Mostly clear | — |
| 19 | STR-1 | Fold `DdcSupervisor` onto `RespawnGate`; test the real wiring            | Cosmetic  | S  | Clear        | — |
| 20 | UX-1  | One sentence on config snapshots / no live reload                       | Cosmetic  | XS | Clear        | — |
| 21 | UX-2  | Checksum in the release body                                            | Cosmetic  | XS | Clear        | — |
| 22 | STR-2 | Give the two no-assert tests assertions or delete them                   | Cosmetic  | XS | Clear        | — |

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
- **I3's premise** that Windows actually recycles `HMONITOR` values on the dock/undock transitions
  this app sees. The hazard follows from the API contract; the decisive check is a manual
  dock/undock with `refresh.periodic_seconds = 0`.
- **I8's magnitude** — no profiling of idle CPU or energy impact was done.
- **`winres`/`winresource` upstream status and dates** (DEP-2) — asserted from recollection, no
  network check.
- **Live RustSec advisory state** — `cargo audit` is not installed locally; CI's weekly job is the
  standing authority.
- **The outstanding manual passes the project's own docs call for**: the sleep/wake System Resume
  Test and the full hardware pass for the 0.62 migration (DDC, OSD, overlay, tray, hotkeys, power
  events).
- `tray.rs` (1069 lines) and `hotkey.rs` (808) were pattern-swept for FFI red flags and read at the
  seams, not line-audited end to end.
