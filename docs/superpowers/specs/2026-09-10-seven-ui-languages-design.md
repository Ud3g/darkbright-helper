# Design: seven more UI languages

_Date: 2026-09-10_

## Problem

The interface ships in English and German. The string table (`src/core/i18n.rs`), the language
picker with live switching, and the measured settings-window layout with its CI gate are all in
place, so a further language whose script Segoe UI covers natively and which runs left to right
is a translation, not an engineering project.

The obstacle is quality, not code. The maintainer can judge the German table and cannot judge
most of the languages worth adding. The translations are therefore produced and checked by LLMs,
and the process has to make up for the missing native-speaker review as far as a process can:
independent passes that catch shifted meaning and unidiomatic wording separately, mechanical
checks for everything that can be checked mechanically, a visual pass for what CI cannot see, and
an honest note in the README.

## Scope

**In scope:**

1. Seven languages: French (`fr`), Spanish (`es`, neutral), Portuguese (`pt`, Brazilian),
   Russian (`ru`), Indonesian (`id`), Turkish (`tr`), Vietnamese (`vi`).
2. Splitting the string tables into one file per language.
3. A multi-pass LLM translation and review process, piloted on French before the other six.
4. New tests: fields identical to English must be declared; French punctuation carries its
   no-break space; the OSD error row fits its window; the hotkey status line's fixed texts fit on
   one line. All in every language.
5. The OSD error row measured instead of estimated.
6. Shortening existing English and German status-line texts where the new status-line gate fails.
7. A screenshot pass over all seven languages.
8. README, CHANGELOG (including the entries missing for already-merged, unreleased work),
   `docs/architecture.md` and `CLAUDE.md`.

**Out of scope:**

- Scripts that need font or layout work: Chinese (needs a per-language font face, since Segoe UI's
  font link reaches Meiryo UI, a Japanese face, before Microsoft YaHei UI), Japanese (renders
  through that same link, but the layout gate would first need a check that the fallback font
  exists on the CI runner), Hindi and Bengali (Nirmala UI is not in Segoe UI's font link, and
  their stacked marks need vertical room the planner does not measure), Arabic and Urdu (right to
  left: mirrored window, custom-painted controls, menu and message-box flags, bidi isolation of
  embedded Latin text).
- Separate regional tables (`pt-PT`, `es-ES`). `pt-PT` and every other `pt-*` tag resolve to the
  Brazilian table through the existing lookup.
- Locale-specific digits or percent formatting; `{percent}%` stays as it is.
- A translation issue template, a translations section in `CONTRIBUTING.md`, or an in-app
  "machine translated" marker.
- Two texts that stay unmeasured: the capture field's prompt, which ellipsizes rather than
  clipping and is looked at in the screenshot pass only, and the tray menu's column widths.
- The status-line text that embeds variable error details
  (`hotkey_status_restore_also_failed_fmt`): its length depends on the English detail it carries,
  so no fixed budget can hold it.

## Languages

| Tag | Picker name | Variant and notes |
|---|---|---|
| `fr` | Français | Standard French; no-break spaces before `:` `;` `!` `?` as French typography requires, enforced by a test |
| `es` | Español | Neutral: avoids Spain/Latin-America splits (`equipo`, not `ordenador`/`computadora`), infinitive commands avoid the `tú`/`usted` choice |
| `pt` | Português (Brasil) | Brazilian (`arquivo`, `configurações`, `tela`); the suffix says so because `pt-PT` users receive this table too |
| `ru` | Русский | |
| `id` | Bahasa Indonesia | |
| `tr` | Türkçe | Rust's case mapping is locale-independent and no UI text is case-mapped, so the dotted/dotless `i` needs no handling |
| `vi` | Tiếng Việt | Stacked diacritics; checked for vertical clipping in the screenshot pass |

Tags stay generic and lowercase; `Lang::lookup` is unchanged.

**Picker order.** "System default" first, then `Lang::ALL` sorted by native name, which for these
nine languages is lowercase code-point order: Bahasa Indonesia, Deutsch, English, Español,
Français, Português (Brasil), Tiếng Việt, Türkçe, Русский. A test asserts `Lang::ALL` stays in
that order. Reordering is safe: `config.json` stores the tag, and the index is only the combo
position and the `wparam` of the language message. `Lang::English.index()` moves from 0 to 2;
no code reads `Lang::ALL[0]` or relies on English being first.

## Code structure

`core/i18n.rs` keeps `Lang`, `LanguageSetting`, `LanguageSource`, `Strings`, `TextKey` and
`strings()`. The tables move beside it, following the `controller.rs` + `controller/` precedent:

```
src/core/i18n.rs
src/core/i18n/en.rs  de.rs  fr.rs  es.rs  pt.rs  ru.rs  id.rs  tr.rs  vi.rs
src/core/i18n/tests.rs
```

`ENGLISH` and `GERMAN` are re-exported from `i18n.rs`, so the existing imports (all in test
modules of `error.rs`, `hotkey.rs`, `capture.rs`, `tray.rs` and `window.rs`) do not change.

Each language file opens with a `//!` header recording the decisions a later correction needs:
the variant, a glossary of the core terms (brightness, hotkey, on-screen display, log, settings,
resync, restore defaults, and the like, English → target), the key-name convention with its
source (the platform's accelerator labels and that language's keyboard caps), typography rules
where they apply, and whether the wording follows Microsoft's published style guide for the
language or model knowledge.

Two comments on `Strings` change with the split. The German key-name convention (Pos1, Entf,
Einfg, …) moves from the key-names group comment into `de.rs`'s header; the group comment keeps
only the general rule that each language follows its own platform convention. "Keep the groups
and their order identical in every language table so the two can be diffed" becomes "so any two
can be diffed".

**Tests that change.**
- Tests using `fr` as the example of an unshipped language (`i18n.rs` lookup, preference and parse
  tests; `config.rs`'s invalid-language test) switch to `ja`.
- Tests pinning the language list or its order follow the new list:
  `every_language_variant_appears_in_all`, `german_exists_with_its_tag_and_native_name`,
  `window.rs`'s combo-entries test. `tray.rs`'s `lang_from_wparam` test stays green by coincidence
  (German remains index 1) and is left as it is.
- `layout.rs`'s `the_language_row_leads_the_general_section_and_shifts_the_rest_by_one_row` pins
  the language combo at `(250, 42, 120)` and follows whatever width it gets.
- Completeness, placeholder, link-span, log-token, tooltip and layout tests iterate `Lang::ALL`
  and cover the new tables without change.

**Language picker width.** `ID_LANGUAGE` is 120 logical px wide, justified in `layout.rs` by the
English "System default". The planner cannot widen combos, so its authored `w` must hold the
widest entry any language produces — the native names and every translation of
`language_system_default` — and the combo-entry gate says whether it does.

The width has a budget. The window's width is the maximum of its 400-px floor and, among other
terms, the control column's widest run (`plan_layout`: `edge_b + control_run(Col::B) + margin`),
and today the language combo is that run. Up to `w = 138` the English window stays on its floor;
beyond it, English grows too.

- `w` is set to the smallest value, at most 138, that passes the gate in every language.
- If 138 is not enough, `language_system_default` is shortened first in the languages that need
  it. Native names are not shortened.
- Only if a native name alone needs more than 138 does `w` grow past it. The change then also
  updates `plan.rs`'s `rows_keep_their_authored_vertical_geometry_and_english_its_window_size` and
  `a_short_version_string_leaves_the_window_at_its_floor`, and the 400-px floor figures in
  `docs/architecture.md` §14 and its high-DPI manual procedure.

The comment above `ID_LANGUAGE` is updated to the entry that now sets its width.

## Translation process

**Pilot.** French runs through the whole pipeline first. It has the most hazards at once:
keyboard names (`Maj`, `Suppr`, `Échap`), typography (no-break space before `:`), and long
strings. The pilot also settles one open point for all languages: whether a subagent can retrieve
Microsoft's published localization style guide for the language; if it can, every translator
uses it, otherwise all work from model knowledge. The pilot's review report and the changes
applied go to the maintainer; the brief is sharpened from what the pilot surfaced; then the
remaining six languages run in parallel.

**Brief** (carried in the implementation plan, given to every pass):

- Sources: the English table, the German table (a second source resolves ambiguity: `Close` is a
  verb, `Level:` is a log level), and each field's `///` doc comment.
- Verbatim: `{name}`, `{path}`, `{error}`, `{restore_error}`; both `<a>`…`</a>` spans; the log-level
  tokens; the product name `darkbright-helper`, including inside sentences; the `⚠` prefix and the
  "state — action" shape of the tray warning rows.
- Style: concise Windows UI wording in the target language, per the style reference the pilot
  settled on.
- Units and key names as Windows and the language's keyboards show them.
- Length budgets, because these slots cannot grow:
  - captions in `Anchor::Stretch` rows and log-level combo entries (the plan lists the fields from
    `CONTROLS`);
  - the language picker's entries within a 138-px combo;
  - the hotkey status line's fixed texts on one line (`capture_reject_no_modifier`,
    `capture_reject_unnameable_key`, `capture_reject_duplicate`, `hotkey_status_unreachable`,
    `hotkey_status_no_response`, `hotkey_status_unknown_error`,
    `hotkey_notice_interception_unavailable`).

**Element kinds.** The back-translation receives each string with one of these kinds instead of
its field name: OSD error message; tray tooltip fragment; tray warning row; tray menu heading;
tray menu item; key or modifier name; settings section header; settings field label; checkbox
caption; hint text; unit suffix; dropdown entry; button caption; link row; window title; capture
prompt; status-line message; message-box title; message-box body.

**Pipeline per language:**

1. **Translate** (Opus, fresh context). First fixes the glossary, then fills the table. Returns
   the language file with its header and a list of decisions it was unsure about.
2. **Blind back-translation** (Sonnet). Receives only the translated strings under neutral ids,
   each with its element kind. No field names and no doc comments, since both give the English
   away. Returns English.
3. **Review** (Opus, fresh context). Receives English, German, the translation, the
   back-translation, the glossary, the brief and the translator's doubts. Checks (a) meaning drift
   between source and back-translation and (b), as a native localizer, naturalness, register,
   Windows terminology, glossary consistency, key names and typography. Returns per string: `ok`
   or `change`, with suggestion, reason, category (meaning, terminology, style, typography) and
   confidence (high, medium, low).
4. **Apply.** The orchestrating session applies every `change`, whatever its confidence. Where a
   suggestion contradicts a decision the translator recorded, one further Opus run receives both
   rationales and decides; there is no second round. Changes the reviewer marked low-confidence
   are listed in the pull request description.

**What each pass can and cannot catch.** The back-translation exposes shifted or lost meaning. It
cannot expose a wrong term that translates back to the same English word — a technically correct
but non-Windows term for "settings" round-trips cleanly. Terminology and register rest on the
review pass alone.

**Where decisions go.** Settled, reasoned choices (variant, glossary, key names) go into the
language file header. Doubts nobody could resolve go into the pull request description as open
questions, not into the repository.

No subagent runs on Fable.

## Verification

**Existing gates** (unchanged, now over nine languages): table completeness at compile time,
no empty field, placeholders, the footer's two link spans, log-level tokens, the tray tooltip
within 127 UTF-16 units for every warning combination, and the settings layout at 96/120/144/192
DPI (caption overflow, combo entries, capture-field hotkey text, overlap, client-rect containment,
window bounds).

**New: declared identity with English.** A test in `core/i18n/tests.rs` walks every field of
every non-English table; a field equal to its English value must appear in that language's
allowlist in the test, or the test fails naming it. German's list is `header_hotkeys`,
`key_mod_alt`, `key_mod_win`, `key_separator`, `key_tab`, the five unit fields and
`msgbox_title_autostart`; the log-level tokens are exempt everywhere because another test already
requires them to equal English. This catches a string left untranslated, now and in later
corrections, and makes every identical field a recorded decision.

**New: French no-break spaces.** A test in `core/i18n/tests.rs` requires every `:`, `;`, `!` and
`?` in the French table to be preceded by U+00A0 or U+202F.

**New: the hotkey status line fits.** `ID_HK_ERROR` is a one-line `SS_LEFT` static (`h: 16`): a
text wider than the line wraps, and the wrapped part is not visible. Its text is set at run time,
so it has no `TextKey` and the caption gate skips it. A new gate in `plan.rs`, alongside the
others, measures the seven fixed status-line texts listed in the brief for every language at the
four gate DPIs against the planned width of `ID_HK_ERROR`. English and German are expected to
fail it today (`hotkey_notice_interception_unavailable` is 70 characters in English), so their
failing texts are shortened as part of this change; the maintainer checks the German wording.

**New: the OSD error row fits.** `draw_error_message` (`osd_render.rs`) centres its text using an
estimate of `round(0.38 × font size)` px per UTF-16 unit, 7 px at 96 DPI. The OSD is 360 px wide
with 10 px padding, and Russian or Vietnamese renderings of `osd_ddc_error` run near 40 units, so
a wrong estimate shifts the text off centre and, for a long string, clips its right end.

- The estimate is replaced by `GetTextExtentPoint32W` on the paint DC with the text font
  selected: `TextOutW` draws one unformatted line, and that call measures exactly that line
  (the settings measurer's `DrawTextW` with `DT_CALCRECT` answers for formatted text instead).
- One helper computes the width for both the paint path and the test. If measurement fails, the
  text starts at the left padding instead of being centred.
- The test creates its own memory DC (`CreateCompatibleDC(None)`), selects the OSD text font at
  `OsdMetrics::for_dpi(dpi).font_size` through the renderer's existing font guard, and asserts
  `osd_ddc_error` fits `width − 2 × padding` for every language at the four gate DPIs. The
  settings window's `GdiMeasure` is not reused: it builds the 9-pt dialog font, not the OSD's.

**When a gate fails:**

- A string too long for its slot, the tooltip, the status line or the OSD: shorten it, then run a
  short review (Opus, fresh context, only the changed strings, same brief), so the shortened
  wording has been reviewed too.
- The language combo: as described under "Language picker width". The log-level combo: widen its
  authored `w`.

**Screenshot pass.** For what CI cannot see: missing glyphs, vertically clipped diacritics, and
the ungated capture prompt.

- Stop the installed instance (the maintainer confirms immediately before), run the dev build,
  and cycle the seven languages through the picker, which switches live, so one run covers all.
- Per language: the settings window, a hotkey field in capture mode, a rejection on the status
  line (press Shift alone in the capture field), the tray menu, and the Restore-defaults
  confirmation (answered with Cancel).
- All languages at the current scaling; Vietnamese and whichever language yields the widest
  window also at 200 %. CI already covers geometry at every gate DPI.
- The screenshots are inspected for missing glyphs, clipping and ellipsis; anything notable goes
  into the pull request description with its image.
- Restore the installed instance afterwards.

## Documentation and changelog

**README.**
- The configuration paragraph's "(English and German today)" becomes the list of nine languages.
- "How this project was built" gains one statement: the translations are LLM-generated as well;
  the maintainer checked German; the others went through a multi-pass LLM review (translation,
  blind back-translation, independent review) but no native speaker; corrections are welcome in
  GitHub Discussions, category General. (The bug template requires monitor, brightness-path and
  debug-log fields and blank issues are disabled, so it is the wrong place for a wording report.)

**CHANGELOG `[Unreleased]`.** Nothing merged since 0.10.0 other than the product-name change has
an entry. This change adds:

- *Added* — interface language selection: follows the Windows display language, can be pinned in
  Settings (`language` field), switches the tray, settings window and OSD live, German
  translation included; the settings window sizes itself to the chosen language's text, so no
  caption is cut off.
- *Added* — French, Spanish, Brazilian Portuguese, Russian, Indonesian, Turkish and Vietnamese.
- *Fixed* — config repairs, backup recovery and a failed single-instance check are now written to
  `darkbright.log`; before, everything logged before the file log attached reached only the
  hidden console.
- *Fixed* — Shift+Tab from "Restore defaults" reaches the footer links again after they were left
  forward with Tab; it used to skip them.
- *Fixed* — the dialog shown when Windows refuses to start a worker thread no longer shows stray
  indentation and gaps in the middle of its sentence.
- *Fixed* — longer messages on the settings window's hotkey status line are no longer cut off
  after the first line.

The internal-only merged changes (dark-mode brush sharing, the hotkey command sequence numbers,
test, refactoring and dependency work) get no entry, per `CONTRIBUTING.md`.

**`docs/architecture.md`.**
- §16: the opening sentence that places the table in `src/core/i18n.rs` describes the split;
  "Adding a language" is extended with the language file header, the English-identity allowlist,
  the picker order and the length budgets, and the translation and review procedure in plain
  terms, including the re-review of shortened strings.
- §14: the list of layout gates gains the hotkey status line.
- The OSD error indicator: the row is centred by measured width, and a test keeps every
  language's text inside the window.
- Language Switching Test: the unshipped-tag example `"fr"` becomes `"ja"`; a new step cycles
  every language through the picker and checks glyphs, diacritics, the capture prompt, a
  status-line rejection, the tray menu and the Restore-defaults confirmation.

**`CLAUDE.md`.** The module map names the per-language files under `core/i18n/`.

Commits and the pull request describe the change in domain terms, without internal step or tier
labels.

## Risks

- **Residual wording quality.** Shifted meaning is caught by the back-translation and the review;
  a wrong term or register is caught only if the reviewer sees it. The README says the
  translations had no native-speaker review and where to report problems.
- **Vietnamese diacritics.** Segoe UI supports Vietnamese, but the planner measures widths only;
  stacked marks clipped by a fixed control height would show only in the screenshot pass.
- **Runner fonts.** The layout gate measures with real GDI on `windows-latest`. All seven scripts
  are covered by Segoe UI itself, so no fallback font is involved; this is why the scripts that
  would need one are out of scope.
