# Design: Live monitor rows in the tray menu

_Date: 2026-08-31_

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

**Out of scope (non-goals):**

- **Rows appearing or disappearing while the menu is open** — warning lines (§13), monitor hot-plug,
  and pruning all change the *number* of rows and therefore the menu's height. Those keep today's
  behaviour: they show up the next time the menu is opened.
- **Live hotkey-binding rows.** A rebind from the settings window still surfaces on the next open.
- **Any change to the OSD or overlay.** They are already live.
- **A configuration knob for the poll cadence.** YAGNI; the interval is an implementation detail
  with no user-visible trade-off worth exposing.

## Measurements

The design rests on four facts about Win32 popup menus that are not covered by documentation. All
four were measured on this machine (Windows 11 Pro, build 22621) with a throwaway probe that
registered a window class, opened a real `TrackPopupMenu`, mutated a row from inside the modal loop
and captured the screen. **Each is stated here with its result, because the design is short only
insofar as these hold.**

| Question | Result |
|---|---|
| Does `WM_TIMER` from a timer set on the owner window reach the wndproc while the modal menu loop is running? | **Yes**, repeatedly and evenly — 8 ticks at a 700 ms interval across a 5.8 s menu session, no drift or coalescing |
| Does a visible popup repaint after `SetMenuItemInfoW(MIIM_STRING)`? | **Yes, on its own.** A run with the explicit repaint suppressed still showed the new text within one screenshot interval |
| What happens when the new text is *wider* than the popup was when it opened? | **No clipping** — the popup grows to fit (measured 304 → 312 px on the item rect) |
| And when it becomes narrower again? | The width **stays** at the maximum reached. The menu grows monotonically within one session; it does not jitter |

The second result is the load-bearing one. It removes the entire mechanism this design would
otherwise have needed: locating the popup's `#32768` window via `GetMenuItemRect` +
`WindowFromPoint`, confirming it with an undocumented `MN_GETHMENU` (0x01E1), and forcing
`InvalidateRect`/`UpdateWindow` on a window the process does not own. None of that is written.
The third result likewise removes a planned padding scheme that would have right-aligned the digits
to a fixed width so the text could never grow.

**If a future Windows build stops auto-repainting**, the failure is benign and needs no error path:
the row keeps its old text, which is exactly today's behaviour. Nothing crashes, nothing is
inconsistent beyond what it already is.

## Design

### Mechanism — the tray thread polls itself while its menu is open

`show_context_menu` starts a timer immediately before `TrackPopupMenu` and kills it immediately
after:

```rust
let _ = SetTimer(Some(hwnd), MENU_REFRESH_TIMER_ID, MENU_REFRESH_INTERVAL_MS, None);
let cmd = TrackPopupMenu(/* … unchanged … */);
let _ = KillTimer(Some(hwnd), MENU_REFRESH_TIMER_ID);
```

The timer therefore exists only for the seconds a menu is actually open — there is no periodic work
in the idle process. Each `WM_TIMER` calls the **existing** `request_menu_data()`, the same
`TrayMenuOpening` round trip the menu already performs on open, and writes back only the rows whose
text actually changed.

This adds no new inter-thread message, no new handle, and no new API in `core/`. `TrayMenuOpening`
is already a pull; this is that same pull, repeated while the menu is on screen. Every component
outside `tray.rs` remains unaware that a menu can be open at all.

### Rows are addressed by ID, not by position

The monitor rows already carry unique command IDs (`MENU_ID_MONITOR_BASE + index`,
`tray.rs:82`). The update uses `SetMenuItemInfoW(hmenu, MENU_ID_MONITOR_BASE + i, false, &info)` —
by ID, not by position — so the code never has to track how many warning rows happen to sit above
the monitor block.

### Menu session state

One thread-local, alive only while a menu is open:

```rust
struct MenuSession {
    hmenu: isize,      // the live popup; an HMENU is not Send, and never leaves this thread
    rows: Vec<String>, // last text written per monitor row, in ID order
}
```

`show_context_menu` fills it after building the menu and clears it after `TrackPopupMenu` returns.
A `WM_TIMER` arriving with no session (which cannot normally happen, since the timer's lifetime is
the menu's) is discarded.

`rows` is what makes the "only when changed" check possible, and it is the *only* change detection
in the design — comparing the string about to be written against the string last written. A
brightness that has not moved produces no API call and therefore no repaint.

### Row formatting is extracted

The row text is composed inline in `show_context_menu` today (`tray.rs:694`), including the `~`
prefix that marks a seeded rather than measured brightness. It moves to a pure function:

```rust
fn monitor_menu_line(monitor: &TrayMonitorInfo) -> String
```

used by both the build path and the refresh path. This is not incidental tidying: two independent
copies of this formatting that could drift apart is precisely the bug this feature would otherwise
introduce, and as a pure function it is unit-testable, which the surrounding Win32 code is not.

### When the monitor set changes mid-menu

If a refresh returns a different number of monitors than the session recorded (hot-plug, pruning),
the row IDs no longer line up with what is on screen. The refresh is then **skipped entirely** for
the rest of that menu session rather than guessing: the rows freeze at their last values, which is
today's behaviour and the documented non-goal above.

### When the main thread does not answer

`request_menu_data()` already tolerates a timeout (`MENU_DATA_TIMEOUT`, 500 ms) by returning `None`.
Under polling, a wedged main thread would otherwise stall the menu and log a warning every tick. So
the first timed-out refresh **ends refreshing for that menu session** — the timer is killed, one
warning is logged, and the menu behaves exactly as it does today. A single stall must not become a
stutter.

### Cadence

`MENU_REFRESH_INTERVAL_MS = 250`, defined in `tray.rs` beside the existing `MENU_DATA_TIMEOUT`
rather than in `core/reconcile.rs`: it is a property of this menu's presentation, not of the
refresh/respawn timing scheme that module holds. Four polls per second, for the few seconds a menu
is open, against a main loop that already wakes every 16 ms and answers the request immediately.

250 ms is chosen against the OSD, not against human reaction time — the OSD is the instant feedback,
and the menu row only has to stop being *wrong* before the eye moves to it.

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

## Verification

**Automated:**

- Unit test for `monitor_menu_line`: known brightness, seeded brightness (the `~` marker), and a
  non-zero overlay opacity.

**Manual (hardware, tray):** added to the manual checklist in `docs/architecture.md`.

1. Open the menu, press a brightness hotkey → the row follows within ~250 ms while the menu stays
   open, and matches the OSD.
2. Hold the hotkey through key-repeat → the row counts along without flicker; the menu may widen
   once and must not shrink back or jitter.
3. Hover "Settings" while the rows update → the highlight is not disturbed.
4. Dark mode → the updated row keeps the system menu colours.
5. Two monitors, cursor over one → only that row moves.
6. Unplug a monitor with the menu open → rows freeze, nothing crashes, next open is correct.
7. Close the menu → confirm the timer is gone (no `WM_TIMER` in the debug log after
   `TrackPopupMenu` returns) and idle cost is unchanged.

## Documentation impact

- `docs/architecture.md` §13, "Monitor Status Rows": the bullet "Updated each time the menu is
  opened via `TrayMenuOpening` request/response" becomes the two-phase description (built on open,
  then refreshed on a timer while open), and carries the measured facts above — the reason the code
  is as short as it is belongs in the document, not only in the commit message.
- The §13 "Degraded-State Indicator" push/pull description stays correct as written: warnings remain
  pull-on-open plus push-to-icon. The new path is a third one and is described where the monitor
  rows are, not there.
- `CLAUDE.md`'s architecture summary needs no change: no module boundary, seam, or message moves.

## Risks

| Risk | Assessment |
|---|---|
| A Windows build that does not auto-repaint after `SetMenuItemInfoW` | Degrades to today's behaviour (stale row). No error path needed. Fallback if it ever matters: the reopen alternative above |
| Polling stalls the menu when the main thread is slow | Bounded by `MENU_DATA_TIMEOUT` and limited to one occurrence per menu session by design |
| `SetMenuItemInfoW` by ID behaves differently from by position | Plain documented API; the probe exercised the by-position form. Confirm on the first hardware pass |
| Menu widens mid-session and looks unsteady | Measured: it grows once and holds. Under key-repeat the width settles at the widest value reached |
