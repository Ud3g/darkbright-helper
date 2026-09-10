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
independent passes that catch meaning errors and unidiomatic wording separately, mechanical
checks for everything that can be checked mechanically, a visual pass for what CI cannot see, and
an honest note in the README.

## Scope

**In scope:**

1. Seven languages: French (`fr`), Spanish (`es`, neutral), Portuguese (`pt`, Brazilian),
   Russian (`ru`), Indonesian (`id`), Turkish (`tr`), Vietnamese (`vi`).
2. Splitting the string tables into one file per language.
3. A multi-pass LLM translation and review process, piloted on French before the other six.
4. Two new tests: fields identical to English must be declared, and the OSD error row must fit
   its window in every language.
5. The OSD error row measured instead of estimated.
6. A screenshot pass over all seven languages.
7. README, CHANGELOG (including the entries missing for already-merged, unreleased work),
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
- The follow-ups the layout work parked: `capture_prompt` strings are not gated (they ellipsize),
  and the tray menu's column widths are unmeasured. The screenshot pass looks at the capture
  prompt, but no gate is added.

## Languages

| Tag | Picker name | Variant and notes |
|---|---|---|
| `fr` | Français | Standard French; no-break spaces before `:` `;` `!` `?` as French typography requires, recorded in the language file header |
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
position and the `wparam` of the language message.

## Code structure

`core/i18n.rs` keeps `Lang`, `LanguageSetting`, `LanguageSource`, `Strings`, `TextKey` and
`strings()`. The tables move beside it, following the `controller.rs` + `controller/` precedent:

```
src/core/i18n.rs
src/core/i18n/en.rs  de.rs  fr.rs  es.rs  pt.rs  ru.rs  id.rs  tr.rs  vi.rs
src/core/i18n/tests.rs
```

`ENGLISH` and `GERMAN` are re-exported from `i18n.rs`, so the existing imports in `error.rs`,
`hotkey.rs`, `capture.rs`, `tray.rs` and `window.rs` do not change.

Each language file opens with a `//!` header recording the decisions a later correction needs:
the variant, a glossary of the core terms (brightness, hotkey, on-screen display, log, settings,
resync, restore defaults, and the like, English → target), the key-name convention with its
source (the platform's accelerator labels and that language's keyboard caps, as German already
does), typography rules where they apply, and which style reference the wording follows.

**Tests that change.** Tests using `fr` as the example of an unshipped language
(`i18n.rs` lookup, preference and parse tests; `config.rs`'s invalid-language test) switch to
`ja`. Tests pinning the language list or its order (`every_language_variant_appears_in_all`,
`german_exists_with_its_tag_and_native_name`, `window.rs`'s combo-entries test, `tray.rs`'s
`lang_from_wparam` test) follow the new list. Completeness, placeholder, link-span, log-token,
tooltip and layout tests iterate `Lang::ALL` and cover the new tables without change.

**Language picker width.** `ID_LANGUAGE` is 120 logical px wide, justified in `layout.rs` by the
English "System default". "Português (Brasil)", "Bahasa Indonesia" and longer translations of
"System default" are expected to fail the combo-entry gate. The planner cannot widen combos, so
the authored `w` grows to what the gate measures, and the comment above it is updated.

## Translation process

**Pilot.** French runs through the whole pipeline first. It has the most hazards at once:
keyboard names (`Maj`, `Suppr`, `Échap`), typography (no-break space before `:`), and long
strings. The pilot's review report and the changes applied go to the maintainer; the brief is
sharpened from what the pilot surfaced; then the remaining six languages run in parallel.

**Brief** (carried in the implementation plan, given to every pass):

- Sources: the English table, the German table (a second source resolves ambiguity: `Close` is a
  verb, `Level:` is a log level), and each field's `///` doc comment.
- Verbatim: `{name}`, `{path}`, `{error}`, `{restore_error}`; both `<a>`…`</a>` spans; the log-level
  tokens; the product name `darkbright-helper`, including inside sentences; the `⚠` prefix and the
  "state — action" shape of the tray warning rows.
- Style: concise Windows UI wording in the target language, following Microsoft's published
  localization style guide for that language when it can be retrieved, otherwise model knowledge;
  the header records which.
- Units and key names as Windows and the language's keyboards show them.
- Length: captions in `Anchor::Stretch` rows and combo entries as short as the meaning allows,
  because the planner cannot move them. The plan lists those fields from `CONTROLS`.

**Pipeline per language:**

1. **Translate** (Opus, fresh context). First fixes the glossary, then fills the table. Returns
   the language file with its header and a list of decisions it was unsure about.
2. **Blind back-translation** (Sonnet). Receives only the translated strings under neutral ids,
   each with its element kind (menu item, checkbox caption, tooltip fragment, message-box body,
   …). No field names and no doc comments, since both give the English away. Returns English.
3. **Review** (Opus, fresh context). Receives English, German, the translation, the
   back-translation, the glossary, the brief and the translator's doubts. Checks (a) meaning drift
   between source and back-translation and (b), as a native localizer, naturalness, register,
   Windows terminology, glossary consistency, key names and typography. Returns per string: `ok`
   or `change`, with suggestion, reason, category (meaning, terminology, style, typography) and
   confidence.
4. **Apply.** The orchestrating session applies the review. Where a suggestion contradicts a
   decision the translator recorded, one further Opus run receives both rationales and decides.
   There is no second round.

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

**New: the OSD error row fits.** `draw_error_message` (`osd_render.rs`) centres its text using an
estimate of `round(0.38 × font size)` px per UTF-16 unit, 7 px at 96 DPI. The OSD is 360 px wide
with 10 px padding, and Russian or Vietnamese renderings of `osd_ddc_error` run near 40 units, so
a wrong estimate shifts the text off centre and, for a long string, clips its right end. The estimate is replaced by
`GetTextExtentPoint32W` on the paint DC with the text font selected, through one helper that the
paint path and a new test share. The test measures `osd_ddc_error` for every language at the four
gate DPIs with `OsdMetrics::for_dpi` and asserts it fits `width − 2 × padding`.

**When a gate fails:**

- A string too long for its slot, the tooltip or the OSD: shorten it, then run a short review
  (Opus, fresh context, only the changed strings, same brief), so the shortened wording has been
  reviewed too.
- The language or log-level combo too narrow: widen its authored `w`.

**Screenshot pass.** For what CI cannot see: missing glyphs, vertically clipped diacritics, and
the ungated capture prompt.

- Stop the installed instance (the maintainer confirms immediately before), run the dev build,
  and cycle the seven languages through the picker, which switches live, so one run covers all.
- Per language: the settings window, a hotkey field in capture mode, the tray menu, and the
  Restore-defaults confirmation (answered with Cancel).
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

The internal-only merged changes (string centralization, dark-mode brush sharing, the hotkey
command sequence numbers, test and refactoring work) get no entry, per `CONTRIBUTING.md`.

**`docs/architecture.md`.**
- §16: where the tables live; "Adding a language" extended with the language file header, the
  English-identity allowlist, the picker order, and the translation and review procedure in
  plain terms, including the re-review of shortened strings.
- The OSD error indicator: the row is centred by measured width, and a test keeps every
  language's text inside the window.
- Language Switching Test: the unshipped-tag example `"fr"` becomes `"ja"`; a new step cycles
  every language through the picker and checks glyphs, diacritics, the capture prompt, the tray
  menu and the Restore-defaults confirmation.

**`CLAUDE.md`.** The module map names the per-language files under `core/i18n/`.

Commits and the pull request describe the change in domain terms, without internal step or tier
labels.

## Risks

- **Residual wording quality.** The pipeline catches meaning errors reliably and unidiomatic
  wording often, not always. The README says so and says where to report it.
- **Vietnamese diacritics.** Segoe UI supports Vietnamese, but the planner measures widths only;
  stacked marks clipped by a fixed control height would show only in the screenshot pass.
- **Runner fonts.** The layout gate measures with real GDI on `windows-latest`. All seven scripts
  are covered by Segoe UI itself, so no fallback font is involved; this is why the scripts that
  would need one are out of scope.
