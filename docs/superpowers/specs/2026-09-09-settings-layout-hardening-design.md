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

1. `Standardwerte wiederherstellen` needs 165 px in a 110 px button. The button is centred, so
   it is clipped at *both* ends and renders as `ardwerte wiederhers` — corruption, not
   truncation. It cannot simply grow: `ID_CLOSE` starts at 308 and the version line occupies
   everything to its left.
2. The log-level combo shows `warn (War`. `ID_LOG_LEVEL` is 76 px wide, chosen so its right
   edge aligns with the spinner rows at x=326; the German entries run to 130 px.
3. `Protokolldatei schreiben` (a 128 px caption plus the checkbox's overhead) is cut and
   collides with the `Stufe:` label hard-placed at x=170, with only 146 px in front of it.

A fourth is visible only in measurement: the footer `SysLink` needs 272 px against its declared
250, while 126 px of the window sit unused to its right.

The diagnostic that was supposed to catch all of this, `report_label_overflow`, caught the first
and the fourth. It never measures combo *entries* — they are not `CONTROLS` rows — and it grants
checkbox captions the control's full width, ignoring the indicator the caption does not get to
use. Until it sees what a person saw, it cannot be a gate.

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

- Full vertical flow (`y` computed cumulatively for every row, `BASE_WINDOW_HEIGHT` derived).
  Rows keep their authored `y`; only the two wrapping hints can move anything.
- A third language, pseudo-localisation, right-to-left, CJK font selection.
- The OSD's unmeasured `draw_error_message` (§6) and the tray menu's column widths. Both are
  named in the 2026-09-07 review; neither is a defect today.
- Re-theming the open window on a system light/dark change, and `MessageBoxW`'s
  Windows-supplied button captions. Both were examined in the manual pass and accepted.
- The window's height against a 1080p work area at 150% scaling. It already does not fit, and
  did not before this cycle — see §7.

## Units

**The planner works in physical pixels at the DPI it is planning for.** Every authored constant
from `CONTROLS` enters through `scale_dimension(value, dpi)`, every measurement is taken with
`build_font(dpi, weight)`, and every system or theme query takes an explicit DPI argument.

This is stated first because getting it wrong is invisible: at 96 DPI logical and physical
pixels coincide, so a formula that mixes the two is correct at 96 and silently wrong everywhere
else — and a gate built on the same mixed arithmetic would check it against itself and pass.
Scaling a 96-DPI measurement is not a substitute for measuring at the target DPI. The checkbox
indicator makes the point without ambiguity: `OpenThemeDataForDpi` reports 13 / 16 / 16 / 16 px
at 96 / 120 / 144 / 192, which is neither constant nor proportional.

Every number quoted in this document is at 96 DPI, where the two units coincide, unless a
DPI is stated.

## Measurements

At 96 DPI, Segoe UI 9 pt, through `DrawTextW`/`DT_CALCRECT` with the window's own fonts — the
same mechanism this design puts into production.

**Measurement convention: the string is passed without its terminating NUL.** The existing
diagnostic passes `wide()`'s output, which is NUL-terminated, and the `windows` crate's
`DrawTextW` binding sends the slice's whole length as the character count — so the NUL is
measured as a character and inflates *every* width by a constant 6 px at this font and DPI
(verified). Independent confirmation that the NUL-free value is the true one: `debug` measures
34 px without it, and "~34px" is what the comment beside `ID_LOG_LEVEL` in `CONTROLS` has said
all along, from a measurement taken by other means. Every figure below is NUL-free, and so is
every figure elsewhere in this document — which means they are 6 px lower than the ones the
2026-09-08 overflow record carries. `docs/architecture.md` §14's version-line figure of 165 px
comes from the same inflated source; re-measured NUL-free it is 165 px again — the earlier
figure was for the bare version, while the control actually displays `"v"` + the version, and
the two errors happen to cancel.

| Quantity | English | German | Available today |
|---|---:|---:|---|
| Widest label, hotkey column (x=24) | 88 | 107 | 140 |
| Widest label, spinner column (x=24) | 148 | 175 | 220 |
| `ID_RESTORE` | 84 | 165 | 110 |
| `ID_LOG_CHECK` caption alone | 67 | 128 | 146 |
| `ID_LINK_CONFIG`, `<a>` markup stripped | 177 | 272 | 250 |
| Widest log-level entry (`debug` / `trace (…)`) | 34 | 130 | 76 |
| `Strg+Umschalt+Nach-Oben` | 74 | 149 | 218 |
| Tooltip, all four warnings (UTF-16 units) | 99 | 113 | 127 |

Three facts follow, and they set this design's shape.

**German does not need a wider window.** Its widest label leaves 45 px of slack in the column
that binds. All four defects are local errors in hard-wired constants, not a window that ran out
of room. The derived width earns its keep at language three: Russian scales
`Brightness step per keypress` to roughly 293 px (2026-09-07 review §4.1), which does break the
column.

**The checkbox indicator is measurable, and its total overhead must be calibrated, not
assumed.** `OpenThemeDataForDpi(None, "BUTTON", dpi)` plus `GetThemePartSize(BP_CHECKBOX,
TS_TRUE)` gives the indicator itself — 13 / 16 / 16 / 16 px at 96 / 120 / 144 / 192 — and
`dark.rs:596` already defines `CHECKBOX_TEXT_GAP = 4`, used at `dark.rs:649,680` to place the
caption at `glyph_rect.right + CHECKBOX_TEXT_GAP`. But indicator + gap is demonstrably *not* the
whole budget: under the corrected convention the German log caption is 128 px, which with an
overhead of 17 would come to 145 against 146 px of room — it would fit, and on hardware it
visibly did not. `BCM_GETIDEALSIZE` on a real control wants caption + 18, which lands exactly on
146 — no slack at all — and matches what was seen.

The overhead was therefore measured from live controls rather than derived from the indicator
size, at all four DPI values, and both the system-drawn control and the dark-mode painter were
checked, because the window renders in either theme and the tighter of the two binds:

| DPI | System control (`BCM_GETIDEALSIZE` − caption) | Dark painter (`TS_DRAW` glyph + gap) | `CHECKBOX_OVERHEAD` (the larger) |
|---:|---:|---:|---:|
| 96 | 18 | 13 + 4 = 17 | **18** |
| 120 | 19 | 16 + 4 = 20 | **20** |
| 144 | 20 | 16 + 4 = 20 | **20** |
| 192 | 21 | 26 + 4 = 30 | **30** |

The system figure is a *constant per DPI*: identical for all four sample captions, English and
German, short and long. It does not scale proportionally with DPI — proportional scaling of the
96-DPI value would give 18 / 22 / 27 / 36, and the measured series is 18 / 19 / 20 / 21, barely
moving at all. The dark painter's budget is the opposite shape: it tracks the themed glyph, which
is flat at 13 / 16 / 16 / 16 under `TS_TRUE` but jumps to 26 at 192 DPI under `TS_DRAW` — and
`TS_DRAW` is what `dark.rs` asks for, so that is the number that binds there. Which theme wins
therefore changes with DPI: the system control is tighter at 96, the dark painter at 120 and 192,
and they tie at 144. Neither may be assumed from the other, and neither may be scaled from 96 —
the planner queries at the DPI it is planning for, exactly as §Units requires.

**A pushbutton needs 8 px around its caption and the table gives it 26.** `BCM_GETIDEALSIZE` −
caption is 8 for every button caption at every one of the four DPI values, English and German —
that is what the control itself asks for. The authored table is more generous: `Restore defaults`
measures 84 px NUL-free inside a 110 px button, so the table's own padding is 26 px.
`BUTTON_TEXT_PAD` is that 26, which is what makes the English footer render pixel-for-pixel as it
does today, and it is comfortably above the 8 px the control needs. `BUTTON_MIN_W` (80, from
`Close`'s authored width) keeps a short caption from producing a cramped button: `Close` at 35 px
plus the padding is only 61.

**Version-line widths**, measured on the string the control actually shows, `"v"` + the version:
`v0.10.0` is 36 px at 96 DPI (45 / 57 / 74 at 120 / 144 / 192), and the worst realistic
development string `v0.10.0+64.gc4687e5.dirty (dev)` is 165 px at 96 DPI (209 / 255 / 337).

## Design

### 1. Three parts where there was one

`layout()` splits along the line between "needs a device context" and "is arithmetic":

| Part | Location | Character |
|---|---|---|
| `TextMeasure` trait + `GdiMeasure` | new `settings/measure.rs` | All the `unsafe`. Builds the two dialog fonts and a memory DC, selects the right font, calls `DrawTextW`, queries the themed indicator size and the DPI-scaled system metrics. `Drop` restores the DC's original font, then frees both fonts and the DC. |
| `plan_layout(lang, dpi, version_text, &mut impl TextMeasure) -> Plan` | new `settings/plan.rs` | Pure arithmetic over `CONTROLS` plus measurements. No `HWND`. |
| `apply(hwnd, &Plan)` | `layout.rs` | `BeginDeferWindowPos`/`DeferWindowPos`/`EndDeferWindowPos`, with a fallback. |

`layout(hwnd, dpi)` becomes the thin orchestrator of those.

The port is what makes the CI gate trustworthy rather than flaky: the gate measures through the
same `GdiMeasure` the window uses, so it compares a measured text against a slot computed from
that same measurement. Nothing is asserted against a frozen pixel count, and a CI runner whose
font metrics differ slightly from a developer's machine stays green for the right reason.

**That argument covers text, and only text.** The test process is DPI-*unaware*:
`SetProcessDpiAwarenessContext` is called in `main.rs:546`, in the binary, which no test links.
Every DPI-sensitive system or theme query is therefore virtualised to 96 in a test unless it
takes an explicit DPI argument. Text measurement is unaffected, because the font's `lfHeight`
comes from the `dpi` parameter rather than from the device. The rule that follows is absolute:
**only explicit-DPI APIs may enter the measurement port** — `GetSystemMetricsForDpi`,
`OpenThemeDataForDpi`. A plain `GetSystemMetrics` would return 17 for the scrollbar width at
every DPI (verified) and quietly make the gate's 120/144/192 rows check nothing.

A `GdiMeasure` needs no window: it builds its fonts from `build_font(dpi, weight)` — the same
call `window.rs` makes — and draws into a `CreateCompatibleDC(None)` memory DC. Measured widths
from that memory DC are identical to a screen DC's for every sample string at 96 and 144 DPI.
That is why the planner is testable, and why the plan can be computed before any window exists.

`layout.rs` gives work away rather than taking it on; it keeps `CONTROLS`, the ids, the styles,
`scale_dimension`, `RANGE_SPECS`, the combo-height logic and the placement code.

### 2. Horizontal: one new field, no new coordinate system

`x`, `y`, `w` and `h` stay in `CONTROLS` as the authored 96-DPI baseline. `ControlSpec` gains
one field saying how the control responds when text grows:

| Anchor | Applies to | Meaning |
|---|---|---|
| `Label(col)` | row captions | Width measured; the column's width is the maximum over its rows. |
| `Checkbox(col)` | checkboxes paired with a control | As `Label`, plus indicator and gap. |
| `CheckboxRun` | `ID_LOG_CHECK` | First member of the composite log row: width is its own requirement, not the column's. |
| `InlineLabel` | `ID_LABEL_LOG_LEVEL` | Right-aligned immediately before the control column; `w = max(authored, measured)`. |
| `Control(col)` | edits, combos | `x` = the column's computed left edge; `w` as authored. |
| `ControlStretch(col)` | the two capture fields | `x` = the column's left edge; `w` runs to the right margin. |
| `AfterControl(col)` | spinner buttons, unit suffixes | `x` = column left edge + the authored offset. |
| `Stretch` | headers, separators, hints, full-width checkboxes, the `SysLink` | Keeps the authored right margin: `w = win_w − x − (400 − x − w_authored)`. |
| `FooterButton` | `ID_RESTORE`, `ID_CLOSE` | Right-aligned chain; width measured plus padding, floored. |
| `FooterFill` | `ID_VERSION` | Takes what the buttons leave. |

`col` is `A` (hotkey rows) or `B` (spinner and log rows). `Stretch` needs no new constant: the
right margin a control should keep is already implicit in today's table, and is preserved.
`ID_AUTOSTART` and `ID_INTERCEPT` are full-width checkboxes belonging to no column, so they take
`Stretch` for their width; the gate still checks their caption against the drawable part.

**Two label columns, not one.** The table already has two: hotkey rows end at 170, spinner and
log rows at 250. Each is measured separately and **floored at today's value**. English therefore
renders pixel-for-pixel as it does now, German too (its hotkey labels reach 113 against a 140
floor and its spinner labels 181 against 220), and a future language moves only the column that
needs it. `COL_GAP` is 6 px, read off the existing table rather than invented — it is the gap
both the `w:140` and the `w:220` labels leave before their control.

**The log row is a composite, and defect 3 dissolves in it.** Its label-side requirement is not
one caption but the whole run before the control column:

```
checkbox_overhead(18 at 96 DPI) + "Protokolldatei schreiben"(128)
              + INLINE_GAP(6) + inline_label(max(authored 74, "Stufe:" 30) = 74)   = 226
English, for comparison:  18 + 67 + 6 + 74                                        = 165
column B floor: 250 − 24 − COL_GAP(6)                                             = 220
```

`INLINE_GAP` is 6, read off the table like `COL_GAP` (170 − 24 − 140). The inline label's
authored width acts as a **minimum**, which is what keeps English identical: `max(74, 36)` is
74, so it lands at `250 − 6 − 74 = 170`, exactly where it sits today. Sizing it purely by
measurement would right-align it to 208 and shift the English row 38 px — a regression dressed
as a refinement.

English's run is 165, well under the 220 floor, so column B does not move and the whole window
is unchanged. German's 226 pushes column B to 226 and its control edge from 250 to 256 — and the
window still does not grow, because the widest thing in that column is the 120 px language combo
and `256 + 120 + 12 = 388` is inside the 400 floor. So the collision is gone with no word
changed and nothing visibly moved but the log row itself. In a language where the run does not
fit, the control column keeps moving right and the row still reads correctly — which is the
point: no future language can reproduce this collision, because nothing in the row is placed by
hand any more.

### 3. Window width

```
win_w = max( 400,
             24 + colA + COL_GAP + capture_w + 12,
             24 + colB + COL_GAP + widest_control_run + 12,
             12 + version_w + FOOTER_GAP_VERSION + restore_w
                            + FOOTER_GAP_BUTTONS + close_w + 12 )
```

`widest_control_run` is the maximum, over the control column's rows, of that row's own extent:
a spinner row is `edit(60) + updown(16) + SUFFIX_GAP(6) + suffix_w(≤30)` = 112, the language
combo alone is 120. `SUFFIX_GAP` is 6 (332 − 326), read off the table like the other gaps. Writing it as a per-row run rather than a single control width matters
precisely in the language-three case the formula exists for — a bare `ctrl_w + suffix_w` would
drop the 6 px between the spinner and its unit.

The two footer gaps are read off the current table and differ: 6 px between the version line's
right edge (184) and `ID_RESTORE` (190), 8 px between `ID_RESTORE`'s right edge (300) and
`ID_CLOSE` (308). Button widths are `max(BUTTON_MIN_W, measured + BUTTON_TEXT_PAD)`, where
`BUTTON_MIN_W` is 80 — `ID_CLOSE`'s authored width, which the floor exists to preserve — and
`BUTTON_TEXT_PAD` is 26, the padding the authored table itself gives `ID_RESTORE`, so English
renders exactly as it does today. A live control's own ideal size wants only 8 px (§Measurements),
so 26 is comfortably more than the button needs rather than a figure it has to live within.

The 400 floor is what keeps today's appearance exactly as it is; every other term is measured.
The footer participating in the width is what resolves the `ID_RESTORE` conflict without a
compromise in either direction:

- **Release build.** The version line reads `v0.10.0`, 36 px. With `Auf Standard zurücksetzen`
  at 140 px the restore button becomes 166 and `Schließen` leaves `ID_CLOSE` on its 80 px floor,
  so the footer needs `12 + 36 + 6 + 166 + 8 + 80 + 12 = 320` px. The 400 floor binds, and the
  shipped German window is identical to today's, with 80 px to spare. English is 264.
- **Development build.** The worst realistic string, `v0.10.0+64.gc4687e5.dirty (dev)`, measures
  165 px. The footer then needs `12 + 165 + 6 + 166 + 8 + 80 + 12 = 449` px and the window grows
  to it. Nothing is truncated.

That is the intended behaviour, not a side effect: the version line is the one control whose
caption is a runtime value, so the window that must display it is a runtime question. It does
mean the manually tested window is not the shipped window — §Testing carries the consequence.

### 4. Vertical: only what can grow

Rows keep their authored `y`. The only controls whose height depends on their text are the two
wrapping hint statics, and they are measured with `DT_CALCRECT | DT_WORDBREAK` against their
computed width:

```
delta  = max(0, measured_h − authored_h)
y_off += delta            // applies to every later control
win_h  = 654 + total_y_off
```

**`y_off` is recomputed from scratch on every plan, never latched.** A plan for English and a
plan for German are independent; switching back from a language that grew the window returns it
to the shorter geometry. The `max(0, …)` clamps a single hint against its own authored height,
nothing more.

Growth only, never shrinkage below the authored height: a wider window makes hints wrap onto
fewer lines, and a window that got *shorter* because a translation was terse would be more
surprise than gain.

Today this machinery moves nothing. No hint overflows its authored height in either language —
the diagnostic confirms it — so `y_off` is 0 and every row sits where it sits now, including the
one piece of hard-won empirical geometry in the table: the comment beside `ID_LABEL_LOG_LEVEL`
explaining why `Stufe:` sits at `y:512` rather than the `y:515` the textbook vertical-centering
model predicts, measured on hardware at 125% DPI. The rule exists so that the language which
does wrap a hint onto a third line pushes the rows below it instead of overlapping them, and so
that CI stays green when it happens rather than requiring a person to re-tune the table.

### 5. One path for creation, DPI change and language change

`plan_layout` needs no window, so the plan is computed **before** `CreateWindowExW`, not after:

1. Resolve the target monitor and its DPI (today's `compute_placement`, split).
2. Build the fonts; `plan_layout(lang, dpi, version_string(), &mut GdiMeasure)`.
3. `AdjustWindowRectExForDpi` on the plan's client size, then clamp into the work area.
4. `CreateWindowExW` at that position and size; `create_controls`; `apply(hwnd, &plan)`.
5. `configure_updowns`, `configure_combo_height`, `apply_snapshot`, `ShowWindow`.

This is deliberately not "create, then resize": a window created centred for 400 px and then
grown to 449 would sit 25 px off-centre. Computing first makes the initial geometry correct in
one step and deletes the resize path from creation entirely.

`apply` bundles the moves through `BeginDeferWindowPos`. Two constraints on it. `SWP_NOZORDER`
must be on every `DeferWindowPos` call: z-order is tab order in this window (creation order,
`layout.rs:145-148`), and reordering it would be a silent regression nothing tests. And a null
`HDWP` from `BeginDeferWindowPos` or `DeferWindowPos` **discards the whole batch**, which at
creation — where every control starts at `(0,0,0,0)` — would be a blank window; `apply` falls
back to the per-control `SetWindowPos` loop in that case, degrading one control at a time as the
rest of this module does.

**`WM_DPICHANGED`** already applies Windows' suggested rect before calling `layout()`
(`window.rs:860-873`). The re-plan keeps that suggested *position* and overrides only the
*size*, and carries a re-entrancy guard: resizing inside `WM_DPICHANGED` can send another
`WM_DPICHANGED` when the window's majority crosses a monitor boundary.

**`WM_APP_SETTINGS_LANG` does not re-run `layout()` today, and must.** It relabels in place,
which is correct only while every language shares one set of slots. After this cycle a relabel
changes the measurements. A language switch **resizes but never moves**: the user may have
dragged the window, and a caption change is no reason to recentre it.

### 6. The four defects

| Defect | Treatment | Cost |
|---|---|---|
| `ID_RESTORE` | German becomes `Auf Standard zurücksetzen` (140 px); the button sizes itself; the window grows only in a development build | one string |
| `ID_LOG_LEVEL` | The parenthetical convention is dropped: every language shows `error`/`warn`/`info`/`debug`/`trace` | five strings, one `i18n.rs` comment |
| `ID_LOG_CHECK` | None. The measured composite row resolves it | — |
| `ID_LINK_CONFIG` | Authored `w` corrected 250 → 376, the full width its row always had; `Stretch` carries that margin forward | one number |

**The log-level change is a width decision, not a consistency one.** §16 explicitly permits a
gloss — it says the picker's entries are display-only and the stored value comes from the
combo's selected index, never from its text — so the parenthetical was sanctioned, not an
oversight. What it costs is room: German's widest entry is 130 px, plus the dropdown arrow (17)
and the two 4 px text insets `dark.rs:824` applies, is 155 px. At the control column's left edge
of 250 the combo would end at 405 against a 388 right margin, so keeping the gloss buys a German
window roughly 17 px wider and gives up the log combo's right-edge alignment with the spinner
rows at 326. `CB_SETDROPPEDWIDTH` does not help: it widens only the open list, so the closed
face still reads `warn (War`. The bare token costs nothing and the adjacent `Stufe:`/`Level:`
label already carries the meaning, so the gloss goes and the combo keeps `w:76`.

`log_level_entries_lead_with_the_stored_token_in_every_language` in `core/i18n.rs` becomes
vacuous and is replaced by the stronger statement it now can make: the entries *are* the stored
tokens, identical in every language.

The two combos keep their different widths — `ID_LANGUAGE` at 120 for "System default" plus its
arrow, `ID_LOG_LEVEL` at 76 to align with the spinner rows. The comment beside `ID_LANGUAGE`
deferring "aligning the column properly" to a later cycle is rewritten to record that as a
decision rather than a debt: each combo is sized to its own content, and there is no third edge
to align them to that would not break one of the two alignments that exist.

### 7. The gate

The repaired diagnostic loses `#[ignore]` and runs in the existing `cargo test --locked` step on
`windows-latest`. No workflow change: visual styles do apply to the test binary (the manifest
resource is linked into every binary of the package), and `CreateCompatibleDC`, `GetDC` and
`GetThemePartSize` all work in that session.

For every `Lang` × DPI ∈ {96, 120, 144, 192}, against the plan for that DPI:

- every slot is at least as wide as its measured text, compared against the **drawable** width,
  not the raw `w`: a combo loses `GetSystemMetricsForDpi(SM_CXVSCROLL, dpi)` (17/21/26/34) plus
  two `COMBO_TEXT_INSET`, a capture field two `CAPTURE_TEXT_INSET`, a checkbox its indicator plus
  `CHECKBOX_TEXT_GAP`, and the `SysLink` its `<a>` markup. Comparing against raw widths would let
  the gate pass on text that visibly clips;
- each hint's assigned height is at least its wrapped measured height;
- **every combo entry** fits, which is the hole defect 2 fell through;
- the capture fields fit the widest hotkey text `ParsedHotkey::display_text` can produce in that
  language (all modifiers plus the longest named key), the seam the tray usage rows share;
- no two controls overlap, carrying over the existing exemption for a combo's dropped-list `h`;
- the version line fits the worst-case version string re-measured under the NUL-free
  convention, not the runner's incidental build string;
- `win_w ≤ 560` and `win_h ≤ 700` logical.

**What the height ceiling is and is not.** It is not proof the window fits a screen. Measured:
the outer rect for a 654-logical client at 144 DPI is 1037 px, and a 1080p work area at 150%
scaling is about 1008 px once the 48-logical-pixel taskbar is taken off. **Today's window
already exceeds it by roughly 29 px**, which `compute_placement`'s clamp comment
(`layout.rs:1017-1022`) states outright and mitigates by pinning the top-left corner inside the
work area. The ceiling is therefore a do-not-get-worse bound — 700 logical is 7% above today's
654 — and the earlier framing of it as a fits-on-screen proof compared a client height against a
work area and was wrong twice over. The real limitation is pre-existing, out of scope here, and
now recorded in §14 rather than implied.

On failure the test prints the same table `report_label_overflow` printed, so the diagnostic's
only real value survives inside the gate; the ignored test itself is removed.

### 8. Tooltip

`szTip` holds 127 UTF-16 units and truncates silently past that (`tray.rs:716`). The tooltip has
no runtime content — its text is a function of `HealthWarnings` and the language — so the space
of possible strings is finite and a test over all of it is a proof, not a sample. German's worst
case is 113 units against 127, English's 99. The test asserts the bound over every combination ×
every language; §13 records the measured budget. No truncation path is added: there is no case
the test does not already exclude. It needs no Windows APIs and runs on any host.

### 9. Carried follow-ups

From the language-selection follow-up list, the items that live in the files this cycle opens:

- `window.rs`: the binding named `first` at the `SetFocus` call has not been the first tab stop
  since the language row was added — renamed to say what it is.
- `fill_combo(combo: HWND)` takes `(hwnd, id)` like `set_combo_index` and `combo_selected_index`,
  removing three `GetDlgItem` lookups per combo from the relabel path.
- The old diagnostic built both fonts before asserting its DC was valid, leaking them on a
  failing assert. It dissolves with the test, and `GdiMeasure` releases its handles through
  `Drop` on every exit path including a panic — restoring the DC's original font first, since a
  font still selected into a DC cannot be deleted. RAII, as `docs/code-conventions.md` requires.

## Testing

**Host-testable, no display needed.** `plan_layout` against a fake `TextMeasure` returning
proportional widths: column maxima, the composite log row, the footer chain, `Stretch` margin
preservation, the width floor, hint-driven `y` offsets, and that a re-plan in a narrower language
returns the earlier geometry (the not-latched property of §4). The tooltip bound of §8.

**Windows, in CI.** The gate of §7, plus the existing `layout.rs` tests reworked to assert
against the plan rather than the raw table where they overlap with it
(`no_two_controls_overlap`, `every_tabstop_control_fits_inside_the_client_rect`).

**Manual, on hardware.** German at 100%, 150% and 200% DPI, following the recipe in the
maintenance notes for swapping the installed instance. Beyond the standard pass: the version line
uncut in a development build, a live `de` ↔ `en` switch resizing the window with no repaint
residue and without moving it, `Auf Standard zurücksetzen` fully legible, and the log row
uncollided at every one of the three scalings.

Because the footer participates in the width, a German development build is 449 px wide and a
release build 400 — so the pass would otherwise validate a geometry no user receives. `plan_layout` takes
`version_text` as a parameter for exactly this reason: the pass includes one run with a
release-shaped version string, which is the shipped geometry. The procedure is updated in
`docs/architecture.md` in this cycle, as the "Integration Testing" rule requires.

No release tag until that pass is clean.

## Risks

- **Runner font metrics differ from a developer's machine.** Contained by construction for text:
  both sides of every comparison are measured through the same port. Only 560 and 700 are
  absolute, and each has room.
- **A DPI-sensitive query sneaks into the port without a DPI argument.** This is the sharpest
  edge in the design, because it fails silently and in the direction of a green gate. §1 states
  the rule; the review of this cycle should check every call in `measure.rs` against it.
- **A theme query fails on a runner.** `OpenThemeDataForDpi` returning nothing must fall back to
  13 px as `dark.rs` already does, not redden CI for an environment reason. The "fail loudly
  rather than skip" posture applies to the device context, whose failure means the gate measured
  nothing at all.
- **The window resizes during a live language switch.** Only when a language genuinely needs more
  room, which in a development build is today's `de` ↔ `en` — so the manual pass sees it rather
  than a user meeting it first.
- **`ControlSpec` grows a field that must be right for 40-odd rows.** Mitigated by rolling the
  anchors out one kind at a time, each against the English plan's authored geometry, and by the
  gate, which fails on any row whose anchor leaves it too small or overlapping. Every row ends up
  with an anchor that describes it — there is no do-nothing default left to hide a row that was
  never considered.

## Documentation

- §14: measured layout replaces the hand-measured table; the version-line budget restated as a
  constraint on width rather than a fixed 172 px; the window's height against a 1080p work area
  at 150% recorded as a known, pre-existing limitation with the work-area clamp as its
  mitigation.
- §16: a sentence recording that the log-level picker shows the bare token — permitted, not
  required, by the surrounding rule — because glossing it costs window width.
- §13: the tooltip's measured budget.
- The testing section: the new gate, and the manual German-at-three-scalings procedure including
  the release-shaped version string.
- `CLAUDE.md`: `measure.rs` and `plan.rs` in the module map.
