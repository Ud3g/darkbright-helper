# Design: measured layout for the settings window

_Date: 2026-09-09_

## Problem

The settings window is a hand-measured 400×654 logical-pixel table (`CONTROLS` in
`src/platform/windows/settings/layout.rs`). Every `x`, `y`, `w` and `h` was chosen by eye
against the English captions, and `layout()` does nothing but scale those numbers to the
current DPI. No text is ever measured — there is no `DT_CALCRECT` and no
`GetTextExtentPoint32W` anywhere in the crate.

That held for exactly one language. German shipped in 0.10.0 and produced three visible
defects, recorded in the manual pass of 2026-09-08:

1. `Standardwerte wiederherstellen` needs 171 px in a 110 px button. The button is centred, so
   it is clipped at *both* ends and renders as `ardwerte wiederhers` — corruption, not
   truncation. It cannot simply grow: `ID_CLOSE` starts at 308 and the version line occupies
   everything to its left.
2. The log-level combo shows `warn (War`. `ID_LOG_LEVEL` is 76 px wide, chosen so its right
   edge aligns with the spinner rows at x=326; the German entries run to 136 px.
3. `Protokolldatei schreiben` (134 px plus the checkbox indicator) is cut and collides with the
   `Stufe:` label hard-placed at x=170, with only 146 px in front of it.

A fourth was found by measurement rather than by eye: the footer `SysLink` needs 278 px against
its declared 250, while 126 px of the window sit unused to its right.

The diagnostic that was supposed to catch all of this, `report_label_overflow`, caught only the
first. It never measures combo *entries* — they are not `CONTROLS` rows — and it grants checkbox
captions the control's full width, ignoring the indicator the caption does not get to use. Until
it sees what a person saw, it cannot be a gate.

This cycle turns `layout()` from a table of chosen numbers into measure-then-place, fixes the
four defects, and arms the repaired diagnostic in CI. After it, a third language is a
translation, not a layout project.

## Scope

**In scope:**

1. A text-measurement seam (`DrawTextW` + `DT_CALCRECT`, the dialog font selected into the DC)
   and a pure layout planner that consumes it.
2. Horizontal placement derived from measurement: two label columns, the control column, the
   right-aligned footer chain, and the window width.
3. Vertical growth for the only two controls that can grow — the wrapping hint statics — with
   every following row pushed down by the difference.
4. `BeginDeferWindowPos`-bundled application of the plan.
5. The four defects, by the means each one needs: one German string, one dropped display
   convention, one corrected table value, and — for the third — nothing at all.
6. The repaired diagnostic promoted to a hard CI gate over every language × four DPI values.
7. An exhaustive test that the tray tooltip fits `szTip`'s 127 UTF-16 units in every language.
8. The layout-adjacent items from the language-selection follow-up list.

**Out of scope:**

- Full vertical flow (`y` computed cumulatively, `BASE_WINDOW_HEIGHT` derived). Rows keep their
  authored `y`; only the two wrapping hints move anything.
- A third language, pseudo-localisation, right-to-left, CJK font selection.
- The OSD's unmeasured `draw_error_message` (§6) and the tray menu's column widths. Both are
  named in the 2026-09-07 review; neither is a defect today.
- Re-theming the open window on a system light/dark change, and `MessageBoxW`'s
  Windows-supplied button captions. Both were examined in the manual pass and accepted.

## Measurements

All at 96 DPI, Segoe UI 9 pt, through `DrawTextW`/`DT_CALCRECT` with the window's own fonts —
the same mechanism this design puts into production.

| Quantity | English | German | Available today |
|---|---:|---:|---|
| Widest label, hotkey column (x=24) | 94 | 113 | 140 |
| Widest label, spinner column (x=24) | 154 | 181 | 220 |
| `ID_RESTORE` | 90 | 171 | 110 |
| `ID_LOG_CHECK` caption + 13 px indicator | 90 | 151 | 146 |
| `ID_LINK_CONFIG`, `<a>` markup stripped | 183 | 278 | 250 |
| Widest log-level entry | 40 | 136 | 76 |
| `Strg+Umschalt+Nach-Oben` | 80 | 155 | 218 |
| Tooltip, all four warnings (UTF-16 units) | 99 | 113 | 127 |

Two facts follow, and they set this design's shape.

**German does not need a wider window.** Its widest label leaves 39 px of slack in the column
that binds. All four defects are local errors in hard-wired constants, not a window that ran out
of room. The derived width earns its keep at language three: Russian scales
`Brightness step per keypress` to roughly 293 px (2026-09-07 review §4.1), which does break the
column.

**The checkbox indicator is measurable.** `GetThemePartSize(BUTTON, BP_CHECKBOX, TS_TRUE)`
returns 13×13 here. The repaired diagnostic asks Windows rather than carrying the 17 px rule of
thumb from the manual pass.

## Design

### 1. Three parts where there was one

`layout()` splits along the line between "needs a device context" and "is arithmetic":

| Part | Location | Character |
|---|---|---|
| `TextMeasure` trait + `GdiMeasure` | new `settings/measure.rs` | All the `unsafe`. Builds the two dialog fonts and a memory DC, selects the right font, calls `DrawTextW`, queries the themed indicator size. `Drop` frees fonts and DC. |
| `plan_layout(lang, dpi, version_text, &mut impl TextMeasure) -> Plan` | new `settings/plan.rs` | Pure arithmetic over `CONTROLS` plus measurements. No `HWND`. |
| `apply(hwnd, &Plan)` | `layout.rs` | `BeginDeferWindowPos`/`DeferWindowPos`/`EndDeferWindowPos`, then the frame resize. |

`layout(hwnd, dpi)` becomes the three-line orchestrator of those.

The port is what makes the CI gate trustworthy rather than flaky: the gate measures through the
same `GdiMeasure` the window uses, so it compares a measured text against a slot computed from
that same measurement. Nothing is asserted against a frozen pixel count, and a CI runner whose
font metrics differ slightly from a developer's machine stays green for the right reason.

A `GdiMeasure` needs no window: it builds its fonts from `build_font(dpi, weight)` — the same
call `window.rs` makes — and draws into a `CreateCompatibleDC(None)` memory DC. That is why the
planner is testable and why `plan_layout` can run before any control exists.

`layout.rs` gives work away rather than taking it on; it keeps `CONTROLS`, the ids, the styles,
`scale_dimension`, `RANGE_SPECS`, the combo-height logic and `compute_placement`.

### 2. Horizontal: one new field, no new coordinate system

`x`, `y`, `w` and `h` stay in `CONTROLS` as the authored 96-DPI baseline. `ControlSpec` gains
one field saying how the control responds when text grows:

| Anchor | Meaning |
|---|---|
| `Fixed` | `x`, `w` as authored. |
| `LabelA` / `LabelB` | Label column A (hotkey rows) or B (spinner and log rows). Width measured; the column's width is the maximum over its rows. |
| `CheckboxA` / `CheckboxB` | As above plus the indicator width and its text gap. |
| `InlineLabel` | Right-aligned immediately before the control column (`Stufe:`). Width measured. |
| `ControlColumn` | `x` = the column's computed left edge; `w` as authored. |
| `AfterControl` | `x` = column left edge + the authored offset (spinner buttons, unit suffixes). |
| `Stretch` | Keeps the authored right margin: `w = win_w - x - (400 - x - w_authored)`. |
| `FooterButton` | Part of the right-aligned footer chain; width measured plus padding, floored. |
| `FooterFill` | The version line; takes what the buttons leave. |

`Stretch` needs no new constant: the right margin a control should keep is already implicit in
today's table, and is simply preserved.

**Two label columns, not one.** The table already has two: hotkey rows end at 170, spinner and
log rows at 250. Each is measured separately and **floored at today's value**. English therefore
renders pixel-for-pixel as it does now, German too (its hotkey labels reach 113 against a 140
floor and its spinner labels 181 against 220), and a future language moves only the column that
needs it. `COL_GAP` is 6 px, read off the existing table rather than invented — it is the gap
both the `w:140` and the `w:220` labels leave before their control.

**The log row is a composite, and defect 3 dissolves in it.** Its label-side requirement is not
one caption but the whole run before the control column:

```
indicator(13) + text_gap(4) + "Protokolldatei schreiben"(134) + INLINE_GAP(8) + "Stufe:"(36) = 195
column B available at its floor: 250 - 24 - 6 = 220
```

That run enters column B's maximum as a single number. It fits in German with 25 px to spare,
with no word changed. In a language where it does not fit, the control column moves right and
the row still reads correctly — which is the point: no future language can reproduce this
collision, because nothing in the row is placed by hand any more.

### 3. Window width

```
win_w = max( 400,
             24 + colA + COL_GAP + capture_w + 12,
             24 + colB + COL_GAP + ctrl_w + suffix_w + 12,
             12 + version_w + FOOTER_GAP + restore_w + FOOTER_GAP + close_w + 12 )
```

The 400 floor is what keeps today's appearance exactly as it is; every other term is measured.
Button widths are `max(BUTTON_MIN_W, measured + BUTTON_TEXT_PAD)`, both constants read off the
current table (`Close` at 35 px in an 80 px button gives the floor; `Restore defaults` at 90 px
in 110 gives the padding).

The footer therefore participates in the width, which resolves the `ID_RESTORE` conflict without
a compromise in either direction:

- **Release build.** `version_string()` is `0.10.0`, about 40 px. The footer needs 326 px, the
  400 floor binds, and the shipped German window is identical to today's — with 114 px of room
  for a 40 px string.
- **Development build.** The worst realistic string, `0.10.0+64.g0e4d436.dirty (dev)`, measures
  165 px (§14 already records this as 165 against 172 px of room, a 7 px margin). The footer then
  needs 451 px and the window grows to it. Nothing is truncated.

That is the intended behaviour, not a side effect: the version line is the one control whose
caption is a runtime value, so the window that must display it is a runtime question.

### 4. Vertical: only what can grow

Rows keep their authored `y`. The only controls whose height depends on their text are the two
wrapping hint statics, and they are measured with `DT_CALCRECT | DT_WORDBREAK` against their
computed width:

```
delta  = max(0, measured_h - authored_h)
y_off += delta            // applies to every later control
win_h  = 654 + total_y_off
```

Growth only, never shrinkage. A wider window makes hints wrap onto fewer lines, and a window
that got *shorter* because a translation was terse would be more surprise than gain.

Keeping `y` authored also preserves the one piece of hard-won empirical geometry in the table:
the comment beside `ID_LABEL_LOG_LEVEL` explaining why `Stufe:` sits at `y:512` rather than the
`y:515` the textbook vertical-centering model predicts, measured on hardware at 125% DPI. A flow
layout would have to re-derive that; this design does not disturb it.

### 5. One path for creation, DPI change and language change

The window is created hidden, so the plan can be computed after the controls exist and before
anything is shown:

1. `create_controls` (unchanged).
2. `layout(hwnd, dpi)` — plan, `apply`, then size the frame with `AdjustWindowRectExForDpi` and
   `SetWindowPos`, re-clamping the position into the work area.
3. `configure_updowns`, `configure_combo_height`, `apply_snapshot`, `ShowWindow`.

The work-area clamp is lifted out of `compute_placement` into a shared helper, because it now
has two callers: initial placement and every later resize.

`WM_DPICHANGED` already re-runs `layout()`. **`WM_APP_SETTINGS_LANG` does not, and must.** Today
it relabels in place, which is correct only because every language shares one set of slots. After
this cycle a relabel changes the measurements, so the handler re-plans and re-applies. Omitting
this would reintroduce exactly the class of defect the cycle removes.

### 6. The four defects

| Defect | Treatment | Cost |
|---|---|---|
| `ID_RESTORE` | German becomes `Auf Standard zurücksetzen` (146 px); the button sizes itself; the window grows only in a development build | one string |
| `ID_LOG_LEVEL` | The parenthetical convention is dropped: every language shows `error`/`warn`/`info`/`debug`/`trace` | five strings, one §16 paragraph |
| `ID_LOG_CHECK` | None. The measured composite row resolves it | — |
| `ID_LINK_CONFIG` | Authored `w` corrected 250 → 376, the full width its row always had; `Stretch` carries that margin forward | one number |

Dropping the parentheses restores §16's own logic rather than contradicting it. §16 lists the
log-level tokens under what stays untranslated, because they round-trip through `config.json`;
appending a translation was the exception that undercut the reason. The adjacent
`Stufe:`/`Level:` label carries the meaning, and the widest entry falls from 136 px to 40, so the
combo keeps `w:76` and its alignment with the spinner rows at x=326.

`log_level_entries_lead_with_the_stored_token_in_every_language` in `core/i18n.rs` becomes
vacuous and is replaced by the stronger statement it now can make: the entries *are* the stored
tokens, identical in every language.

### 7. The gate

The repaired diagnostic loses `#[ignore]` and runs in the existing `cargo test --locked` step on
`windows-latest`. No workflow change. For every `Lang` × DPI ∈ {96, 120, 144, 192}:

- every slot is at least as wide as its measured text, with the indicator subtracted for
  checkboxes and `<a>` markup stripped for the `SysLink`;
- each hint's assigned height is at least its wrapped measured height;
- **every combo entry** fits its combo's width less the dropdown arrow (`SM_CXVSCROLL`) — the
  hole defect 2 fell through;
- the capture fields fit the widest hotkey text `ParsedHotkey::display_text` can produce in that
  language (all modifiers plus the longest named key), which is the seam the tray usage rows
  share;
- no two controls overlap, carrying over the existing exemption for a combo's dropped-list `h`;
- the version line fits the *documented* worst case of 165 px, not the runner's incidental build
  string;
- `win_w ≤ 560` and `win_h ≤ 690`.

The height ceiling is derived, not picked: at 150% scaling 690 logical px is 1035 physical
against roughly 1040 usable on a 1080p work area. Today's window is 654. The width ceiling is
the looser "still reads as a dialog" bound.

On failure the test prints the same table `report_label_overflow` printed, so the diagnostic's
only real value survives inside the gate; the ignored test itself is removed.

### 8. Tooltip

`szTip` holds 127 UTF-16 units and truncates silently past that (`tray.rs`). The tooltip has no
runtime content — its text is a function of `HealthWarnings` and the language — so the space of
possible strings is finite and a test over all of it is a proof, not a sample. German's worst
case is 113 units against 127, English's 99. The test asserts the bound over every combination ×
every language; §13 records the measured budget. No truncation path is added: there is no case
the test does not already exclude.

### 9. Carried follow-ups

From the language-selection follow-up list, the items that live in the files this cycle opens:

- `window.rs`: the binding named `first` at the `SetFocus` call has not been the first tab stop
  since the language row was added — renamed to say what it is.
- `fill_combo(combo: HWND)` takes `(hwnd, id)` like `set_combo_index` and `combo_selected_index`,
  removing three `GetDlgItem` lookups per combo from the relabel path.
- The old diagnostic built both fonts before asserting its DC was valid, leaking them on a
  failing assert. It dissolves with the test, and `GdiMeasure` frees its fonts and DC through
  `Drop` on every exit path including a panic — RAII, as `docs/code-conventions.md` requires of
  handles.

## Testing

**Host-testable, no display needed.** `plan_layout` against a fake `TextMeasure` returning
proportional widths: column maxima, the composite log row, the footer chain, `Stretch` margin
preservation, the width floor, hint-driven `y` offsets.

**Windows, in CI.** The gate of §7, plus the existing `layout.rs` tests reworked to assert
against the plan rather than the raw table where they overlap with it
(`no_two_controls_overlap`, `every_tabstop_control_fits_inside_the_client_rect`).

**Host-testable.** The tooltip bound of §8.

**Manual, on hardware.** German at 100%, 150% and 200% DPI, following the recipe in the
maintenance notes for swapping the installed instance. Beyond the standard pass: the version line
uncut in a development build, a live `de` ↔ `en` switch resizing the window with no repaint
residue, `Auf Standard zurücksetzen` fully legible, and the log row uncollided at every one of
the three scalings. The procedure is updated in `docs/architecture.md` in this cycle, as the
"Integration Testing" rule requires.

No release tag until that pass is clean.

## Risks

- **Runner font metrics differ from a developer's machine.** Contained by construction: both
  sides of every comparison are measured through the same port. Only 560, 690 and 165 are
  absolute, and each has room.
- **`CreateCompatibleDC` in a headless CI session.** It works on `windows-latest`. If it ever
  does not, the gate must fail loudly rather than skip itself — a test that silently opts out is
  worse than no test, because it reads as green.
- **The window resizes during a live language switch.** Only when a language genuinely needs more
  room, which in a development build is today's `de` ↔ `en` — so the manual pass sees it rather
  than a user meeting it first.
- **`ControlSpec` grows a field that must be right for 40-odd rows.** Mitigated by making `Fixed`
  the do-nothing default and by the gate, which fails on any row whose anchor leaves it too small
  or overlapping.

## Documentation

- §14: measured layout replaces the hand-measured table; the version-line budget restated as a
  constraint on width rather than a fixed 172 px; the `ID_LANGUAGE` comment deferring column
  alignment "to the next cycle" is discharged.
- §16: the parenthetical log-level convention removed.
- §13: the tooltip's measured budget.
- The testing section: the new gate, and the manual German-at-three-scalings procedure.
- `CLAUDE.md`: `measure.rs` and `plan.rs` in the module map.
