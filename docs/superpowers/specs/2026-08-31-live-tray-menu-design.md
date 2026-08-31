# Design: Live monitor rows in the tray menu

_Date: 2026-08-31_

_Revised 2026-08-31 after a cold adversarial review — see "Review amendments" at the end._

## Problem

The tray menu's monitor status rows (`Dell U2722D: 🕶0% 🔆60%`) are built exactly once, when the
menu opens: `show_context_menu` sends `TrayMenuOpening { reply_tx }` to the main thread, waits for
the `TrayMenuData` reply, and appends the rows (`src/platform/windows/tray.rs:658`).

Brightness hotkeys keep working while that menu is open — measured, not assumed: the modal
`TrackPopupMenu` loop runs on the tray thread, the hotkey thread is unaffected, and the OSD (a
topmost layered window owned by the main thread) draws over the menu. So a user who presses a
hotkey with the menu open sees **two numbers on screen that disagree**: the OSD showing the new
value, and the menu row still showing the value from when the menu opened.

This is a consistency defect, not a missing-feedback one. The OSD already provides the immediate
feedback; the menu row is simply wrong until the menu is reopened.

## Scope

**In scope:** the monitor status rows update while the menu is open, from any source that changes
the displayed values (hotkey, periodic refresh, resume, a DDC result reconciled to ground truth).

**In scope as a prerequisite:** making `generate_display_names` deterministic (see "Identity, not
count" below). It is a one-line fix to an existing defect that this design's safety guard depends on.

**Out of scope (non-goals):**

- **Rows appearing or disappearing while the menu is open** — warning lines (§13), monitor hot-plug,
  and pruning all change the *number* of rows and therefore the menu's height. Those keep today's
  behaviour: they show up the next time the menu is opened.
- **Live hotkey-binding rows.** A rebind from the settings window still surfaces on the next open.
- **Any change to the OSD or overlay.** They are already live.
- **Screen-reader parity.** `SetMenuItemInfoW` raises no MSAA/UIA name-change notification (not
  verified either way; assumed absent), so a Narrator user keeps hearing the value captured when the
  popup was created. That is today's behaviour for everyone, and this design does not worsen it.
- **A configuration knob for the poll cadence.** YAGNI; an implementation detail with no
  user-visible trade-off worth exposing.

## Measurements

The design rests on facts about Win32 popup menus that documentation does not cover. All were
measured on this machine (Windows 11 Pro, build 22621, 3440×1380 work area, system dark mode) with a
throwaway probe that registered a window class, opened a real `TrackPopupMenu`, mutated a row from
inside the modal loop and captured the screen. **Each is stated with its result, because the design
is short only insofar as these hold.**

| # | Question | Result |
|---|---|---|
| 1 | Does `WM_TIMER` reach the wndproc while the modal menu loop runs? | **Yes**, evenly. 25 ticks at 250 ms across a 6.3 s session |
| 2 | Does a visible popup repaint after `SetMenuItemInfoW(MIIM_STRING)`? | **Yes, on its own.** A run with the explicit repaint suppressed still showed the new text |
| 3 | Does addressing the row **by command ID** (`fByPosition = false`) work the same? | **Yes.** 4/4 `Ok(())`, 0 failures, repaint confirmed on screen |
| 4 | What happens when the changed row becomes wider than the popup? | The popup **grows**; no clipping. Width is the maximum over all items, so a row only widens the popup when it becomes the widest |
| 5 | And at the right work-area edge, where the real tray menu opens? | It **grows leftward**: `l=3141 r=3437 w=296` → `l=3131 r=3443 w=312`. No clipping, no reposition — the popup only ever gains area, never moves out from under the cursor |
| 6 | Does it shrink back when the text narrows? | No. Width holds at the maximum reached in that session — it grows monotonically, it does not jitter |
| 7 | Does the dark-menu path behave the same? | **Yes.** Re-run with the app's own uxtheme opt-in (ordinals 135/104/136/133) on a dark system: repaint correct, menu uniformly dark, no mixed-theme row |
| 8 | Is `WM_TIMER` starved by a busy queue, being the lowest-priority message? | **No.** With synthetic cursor motion over the popup every 12 ms: 25 ticks, identical to idle |

Result 2 is the load-bearing one. It removes the mechanism this design would otherwise have needed:
locating the popup's `#32768` window via `GetMenuItemRect` + `WindowFromPoint`, confirming it with
an undocumented `MN_GETHMENU` (0x01E1), and forcing `InvalidateRect`/`UpdateWindow` on a window the
process does not own. None of that is written. Results 4–6 likewise remove a planned padding scheme
that would have right-aligned the digits to a fixed width so the text could never grow.

Incidentally observed during run 7: a grayed row does take the hover highlight, and that highlight
survived an update to a neighbouring row.

**If a future Windows build stops auto-repainting**, the failure is benign and needs no error path:
the row keeps its old text, which is exactly today's behaviour.

## Design

### Mechanism — the tray thread polls itself while its menu is open

`show_context_menu` starts a timer immediately before `TrackPopupMenu` and kills it immediately
after:

```rust
if SetTimer(Some(hwnd), MENU_REFRESH_TIMER_ID, MENU_REFRESH_INTERVAL_MS, None) == 0 {
    log::warn!(error_code = super::get_last_error_code(); "Menu refresh timer not started");
}
let cmd = TrackPopupMenu(/* … unchanged … */);
let _ = KillTimer(Some(hwnd), MENU_REFRESH_TIMER_ID);
// …then clear the session, then DestroyMenu.
```

`SetTimer` returns `usize` (0 on failure), not a `Result` — so the return value is checked, not
discarded, or the feature could die silently and the manual checklist would have nothing to point at.

The timer exists only for the seconds a menu is actually open; there is no periodic work in the idle
process. Each `WM_TIMER` requests fresh `TrayMenuData` over the same `TrayMenuOpening` round trip the
menu already performs on open, and writes back only the rows whose text actually changed.

This adds no new inter-thread message, no new handle, and no new API in `core/`. `TrayMenuOpening`
is already a pull; this is that same pull, repeated while the menu is on screen. Every component
outside `tray.rs` remains unaware that a menu can be open at all.

### Ordering at teardown, and why the no-session guard is required

`KillTimer` **does not** purge `WM_TIMER` messages already posted to the queue. A tick can therefore
still be dispatched after the kill, which makes discarding a tick that finds no session a *required*
guard rather than a defensive one. The teardown order is fixed:

1. `KillTimer` — stop producing new ticks
2. clear the thread-local session — any straggler tick now finds `None` and returns
3. `DestroyMenu(hmenu)` — only now is the handle allowed to become invalid

Reversing 2 and 3 would let a queued tick call `SetMenuItemInfoW` on a freed `HMENU`.

### Rows are addressed by ID, not by position

The monitor rows already carry unique command IDs (`MENU_ID_MONITOR_BASE + index`,
`tray.rs:78`). The update uses `SetMenuItemInfoW(hmenu, MENU_ID_MONITOR_BASE + i, false, &info)`, so
the code never has to track how many warning rows happen to sit above the monitor block. Measured
equivalent to the by-position form (measurement 3).

The `MENUITEMINFOW` is built with `cbSize` set, `fMask = MIIM_STRING` alone (never `MIIM_TYPE`,
which would clobber `fType`), and the same wide-buffer pattern as `append_menu_item`
(`tray.rs:640`) — a `Vec<u16>` from `format!("{text}\0").encode_utf16()` that outlives the call.
`SetMenuItemInfoW` returns `Result<()>`; an `Err` is logged and ends refreshing for that menu
session rather than being discarded.

### Menu session state

One thread-local, alive only while a menu is open:

```rust
struct MenuSession {
    hmenu: isize,       // the live popup; an HMENU is not Send, and never leaves this thread
    names: Vec<String>, // display names, in row order — the identity guard
    rows: Vec<String>,  // last text written per row, same order
    refreshing: bool,   // cleared when a refresh must not be retried this session
}
```

`show_context_menu` fills it after building the menu and clears it during teardown, in the order
above.

### Identity, not count

A refresh may only write row *i* if row *i* still means the same monitor. Comparing counts is not
enough: `generate_display_names` (`state.rs:166`) assigns the `#1`/`#2` suffixes that distinguish
identical models **in the order of the slice it is given**, and that slice comes from
`self.states.keys()` — a `HashMap`, whose iteration order changes when the map is mutated. Two
identical panels can therefore swap which one is `#1` without the count changing.

That is a defect in today's code, visible only as a puzzling swap between two openings. Polling
would make it a swap *in front of the user*. So:

- **Prerequisite fix:** sort the `Vec<MonitorId>` before passing it to `generate_display_names`, so
  the suffix follows a stable order rather than hash order.
- **Guard:** the refresh compares the display-name vector against the session's. Any difference —
  count, order, or naming — ends refreshing for that session and leaves the rows as they are, which
  is the documented non-goal.

### When the main thread does not answer

The open path's `MENU_DATA_TIMEOUT` (500 ms) is wrong for polling: the tray thread pumps nothing
while it waits, so the wait is a frozen menu, and today that cost is paid once *before* the popup
exists. The poll path uses its own, much shorter `MENU_POLL_TIMEOUT` (50 ms) — generous against a
main loop that ticks every 16 ms and answers immediately.

A miss skips that tick and nothing else. Two consecutive misses end refreshing for the session with
one `warn`. A single hiccup must not kill the feature; a wedged main thread must not stutter the
menu four times a second.

### Row formatting and the diff are pure and testable

Two pure functions live in `core/state.rs`, beside `TrayMonitorInfo` and the existing
`generate_display_names` — which is already presentation logic in that module, so this follows the
established placement rather than inventing one:

```rust
pub(crate) fn monitor_menu_line(monitor: &TrayMonitorInfo) -> String;
pub(crate) fn changed_rows(prev: &[String], next: &[String]) -> Vec<(usize, String)>;
```

The formatter is composed inline in `show_context_menu` today (`tray.rs:691`), including the `~`
prefix that marks a seeded rather than measured brightness. It now has two callers — the build path
and the refresh path — and two copies of it drifting apart is precisely the bug this feature would
otherwise introduce. `changed_rows` is the only change detection in the design. Both are host-testable, which the surrounding Win32 code
is not — `tray.rs` is `#[cfg(windows)]`-gated, so tests placed there do not run on the Linux/WSL
host path `CLAUDE.md` documents.

### Re-entrancy

`show_context_menu` has no guard against being entered while a menu is already up
(`WM_TRAY_CALLBACK` → `handle_tray_callback` → `show_context_menu`, `tray.rs:917`), and posted
messages *are* dispatched inside the modal loop — measured. Today a nested entry is merely wasteful;
with a single timer ID and a single session it would silently kill the outer menu's refresh. So the
session doubles as the guard: `show_context_menu` returns early if one already exists, and logs it
at `debug`.

### Cadence

`MENU_REFRESH_INTERVAL_MS = 250`, defined in `tray.rs` beside the existing `MENU_DATA_TIMEOUT`
rather than in `core/reconcile.rs`: it is a property of this menu's presentation, not of the
refresh/respawn timing scheme that module holds. Four polls per second, for the few seconds a menu
is open, against a main loop that already wakes every 16 ms. Measured to hold under sustained mouse
motion (measurement 8).

250 ms is chosen against the OSD, not against human reaction time — the OSD is the instant feedback,
and the menu row only has to stop being *wrong* before the eye moves to it.

### Logging

The main loop logs every received message at `debug` (`main.rs:667`) and the controller logs
`"TrayMenuOpening received"` (`controller.rs:1505`) — two lines per tick, eight per second while a
menu is open, into a 1 MB rolling file log. Every manual test in `docs/architecture.md` starts with
`RUST_LOG=debug`, so this would bury the lines a tester came to read and rotate away the history a
bug report carries. Both lines drop to `trace` for `TrayMenuOpening` specifically. What stays at
`debug`: the timer failing to start, a re-entrant menu open, and the two conditions that end
refreshing.

## Rejected alternatives

- **Push from the main thread** (post a `WM_APP` nudge when the displayed values change, mirroring
  the existing `TrayStatusHandle`/`WM_TRAY_STATUS` path for health warnings). Event-driven and with
  no polling, but it needs a new message, a handle extension, a change to the main loop, and a new
  observable on the controller (a fingerprint or revision counter) whose only purpose is to let the
  tray notice. It also forces the main thread to post when no menu is open, because it cannot know.
  Four files against one, two change-detection mechanisms against one, and the controller would gain
  API that exists solely for a menu it should not know about. This was the recommendation until the
  measurement showed the repaint was free — that removed the shared expensive part the comparison
  had rested on, leaving the trigger as the entire cost, which reverses the ranking.
- **Close and reopen the menu on every change** (`EndMenu` + a fresh `TrackPopupMenu` at the same
  position). Robust and needs no undocumented behaviour, and it would also cover the non-goals, but
  it flickers, drops hover and keyboard selection, and strobes under hotkey key-repeat. Retained as
  the fallback if a future Windows build stops repainting, but not worth its cost today.
- **Owner-drawn menu items** (`MFT_OWNERDRAW` + `WM_DRAWITEM`). Would give full control over the
  row, at the price of painting highlight and dark-mode colours by hand — which §13 deliberately
  leaves to the system.
- **A custom popup window instead of `TrackPopupMenu`.** Discards system theming, dark mode and
  accessibility for a cosmetic gain.
- **Fixed-width digit padding** so the row can never widen. Measurements 4–6 make it unnecessary,
  and it would put a cosmetic compromise in the one place the user reads a number.

## Verification

**Automated:** unit tests for `monitor_menu_line` (known brightness, seeded brightness with the `~`
marker, non-zero overlay opacity) and for `changed_rows` (no change, one change, several changes,
and — separately — that a differing name vector is what stops a refresh, not a differing length).

**Manual (hardware, tray):** added to the manual checklist in `docs/architecture.md`.

1. Open the menu, press a brightness hotkey → the row follows within ~250 ms while the menu stays
   open, and matches the OSD.
2. Hold the hotkey through key-repeat → the row counts along without flicker; the menu may widen
   once and must not shrink back or jitter.
3. Hover "Settings" while the rows update → the highlight is not disturbed.
4. Arrow-key onto a monitor row, then press the hotkey → the keyboard highlight survives the update.
5. Dark mode → the updated row keeps the system menu colours (probe says yes; confirm in the app).
6. Two monitors, cursor over one → only that row moves.
7. Two *identical* monitors → the `#1`/`#2` rows keep their monitors for the whole session.
8. Unplug a monitor with the menu open → rows freeze, nothing crashes, next open is correct.
9. Close the menu, then check the log: no refresh activity after `TrackPopupMenu` returns, and the
   idle main-loop cadence is unchanged.

## Documentation impact

- `docs/architecture.md` §13, "Monitor Status Rows": the bullet "Updated each time the menu is
  opened via `TrayMenuOpening` request/response" becomes the two-phase description (built on open,
  then refreshed on a timer while open), and carries the measured facts above — the reason the code
  is as short as it is belongs in the document, not only in the commit message.
- The §13 "Degraded-State Indicator" push/pull description stays correct as written: warnings remain
  pull-on-open plus push-to-icon. The new path is a third one and is described where the monitor
  rows are, not there.
- `CLAUDE.md`'s tray-thread bullet says the pull happens "when the menu opens" and must gain "and
  again on a timer while it stays open" — by the repo's own rule, a summary that contradicts
  `docs/architecture.md` is a bug in that file.

## Risks

| Risk | Assessment |
|---|---|
| A Windows build that does not auto-repaint after `SetMenuItemInfoW` | Degrades to today's behaviour (stale row). No error path needed. Fallback if it ever matters: the reopen alternative above |
| Polling stalls the menu when the main thread is slow | Bounded at 50 ms per tick and to two ticks per menu session |
| A queued `WM_TIMER` outliving `DestroyMenu` | Prevented by the fixed teardown order; the no-session discard is a required guard, not a defensive one |
| Identical monitors swapping `#1`/`#2` mid-session | Prevented by the name-vector guard, and the underlying non-determinism is fixed rather than worked around |
| Screen-reader users do not get the live value | Accepted and documented as a non-goal; no regression against today |

## Review amendments

A cold adversarial review of the first draft returned fifteen findings. Eleven were accepted and are
folded into the sections above: the poll-path timeout (previously the 500 ms open-path one), the
re-entrancy guard, the corrected `KillTimer` reasoning and teardown order, a wrong line citation, the
`CLAUDE.md` update this design does require, extracting the diff as well as the formatter, the
logging noise and an unperformable verification step, the display-name non-determinism, the
unchecked `SetTimer` return, the incomplete `MENUITEMINFOW` sketch, and the discarded
`SetMenuItemInfoW` result.

Four could not be settled by reading and were measured instead; all four came back in the design's
favour and are now rows 3, 5, 7 and 8 of the Measurements table — by-ID addressing, growth at the
work-area edge, the dark-menu rendering path, and `WM_TIMER` under a busy queue. Two of those had
been raised as major risks; the review was right that the first draft asserted them without evidence,
and wrong that they would fail.

One finding was rejected on reasoning: that a partial repaint might draw one row with light-mode
colours inside a dark menu. The theme is established before the menu is built and does not change
within a session, so a repaint uses the same theme data as the rest of the popup — and measurement 7
confirms it.
