# Design: UI language selection, live switching, and the German translation

_Date: 2026-09-08_

## Problem

The string table exists (`src/core/i18n.rs`, `docs/architecture.md` §16), but nothing reads a
preference: every lookup ends at `Lang::English`, and §16 lists the five places that decide the
language today. Four of them hard-code the default; the fifth, `OsdRenderState.lang` in the OSD,
has no writer at all. There is no config field, no OS-language detection, no picker, and no second
language.

This cycle adds all of that. It deliberately does **not** make the settings window survive the
longer German labels; that is the next cycle, and this one produces its input.

## Scope

**In scope:**

1. A `language` config field: `"system"` or a BCP-47 tag, default `"system"`, validated like every
   other field.
2. OS-language detection behind a seam in `core/`, implemented on Windows with
   `GetUserPreferredUILanguages(MUI_LANGUAGE_NAME)`.
3. RFC 4647 lookup in `core/`: preference list against the shipped languages, truncating
   `de-AT` to `de`, case-insensitive, English as the documented fallback.
4. A "Language" picker in the settings window, modelled on the log-level combo: first entry
   "System default", then every shipped language in its own name.
5. Live switching at every one of the five places §16 names, including a writer for
   `OsdRenderState.lang`.
6. `Lang::German` and a complete German table.
7. The first production callers of `ParsedHotkey::display_text`, with translated modifier *and*
   key names.
8. A written record of which settings-window labels overflow in German, as the input for the
   layout-hardening cycle.

**Out of scope:**

- **Layout hardening.** The settings window stays a hand-measured 400×(now 654) px table with
  no text measurement. German will overflow it; that is known, measured, and the next cycle.
- **A release tag.** None until the hardening lands.
- Right-to-left, CJK font selection, locale-aware percent formatting, a third language, a
  contributor guide for translations.
- Log messages, `BrightnessError`'s `Display`, the hotkey wire format, the log-level tokens, the
  config field names, the product name. All stay English permanently, as §16 already records.

## Design

### 1. Config field and resolution (`core/`)

**Config.** `Config` gains `language: String`, `#[serde(default = "default_language")]`, default
`"system"`. The field sits at the root of the file, next to `version`, because it is not a
property of any existing section. `SettingsDirty` gains `language`, and every hand-maintained
place that enumerates its flags follows: `SettingsDirty::any()` (an OR over the flags with no
compile-time completeness; missing it would make a language-only session skip its close-time
flush and, on the merge path, never write the field at all), `restore_defaults` (resets it to
`"system"`), `overlay_dirty` (copies it when flagged), the `SettingsDirty` literal in
`handle_restore_defaults` (must flag it so the reset reaches disk), and the doc comments that
say "the ten user-facing settings", which become eleven.

**Validation is strict.** `validate_and_fix` accepts exactly two shapes: the literal `"system"`
(case-insensitive) or a tag that the lookup below resolves to a shipped language. Anything else,
including a well-formed but unshipped tag such as `"fr"`, produces
`ConfigNotice::Unparseable { field: "language", .. }` and is replaced by `"system"` in memory.
This mirrors `logging.file_level`: a typo is reported, never silently honoured, and like every
other repaired field the replacement reaches disk with the next full save. The raw string is not
preserved anywhere; the in-memory `Config` is the only source of truth for the language.

**Types in `core/i18n.rs`:**

- `Lang::German` joins the enum, `tag()` returns `"de"`, and `Lang::ALL` lists English first,
  then German, which is also the picker order.
- `Lang::native_name(self) -> &'static str`: `"English"`, `"Deutsch"`. Each language names itself
  in itself, so a user who cannot read the current UI language can still find their own.
- `enum LanguageSetting { System, Fixed(Lang) }`, the parsed form of the config value.
  `LanguageSetting::parse(&str) -> Option<Self>` implements the validation rule above;
  `wire(&self) -> &'static str` returns `"system"` or `Lang::tag()`. A value read as `"de-AT"`
  parses to `Fixed(German)` and, once the dialog re-saves it, is written back as `"de"`.
- `Lang::lookup(tag: &str) -> Option<Lang>`: RFC 4647 §3.4 lookup of one tag against
  `Lang::ALL`. Lowercases, then compares the whole tag, then strips subtags from the right
  (`de-AT-1901` → `de-AT` → `de`) until a shipped tag matches or nothing is left. After each
  strip, if the new trailing subtag is a single character (an extension or private-use singleton
  such as `x`), it is stripped as well before the next comparison: `de-x-foo` → `de-x` → `de`,
  and `x-private` → `None`.
- `Lang::from_preferences(preferred: &[String]) -> Lang`: the first entry in the list whose
  lookup succeeds, else `Lang::English`. Every entry is tried in order; a list whose first entry
  is unshipped and second is `de` yields German.
- `LanguageSetting::resolve(&self, preferred: &[String]) -> Lang`: `Fixed(l)` → `l`, `System` →
  `from_preferences`.

**Tag convention.** Shipped tags are generic, lowercase, hyphenated (`de`, never `de-DE` or
`de_DE`). A regional variant is only ever added as a separate `Lang` when it needs different
text.

### 2. OS-language detection

**Seam.** `pub trait LanguageSource { fn preferred_languages(&self) -> Vec<String>; }` in
`core/i18n.rs`, next to the types it feeds. It is a seam in the same sense as `MonitorLocator`:
a platform capability with a platform-agnostic consumer. Unlike the controller seams, nothing
in `core/` calls it repeatedly, so it is not a type parameter of `Controller`; `main.rs` calls
it once at startup and hands the resulting list to `Controller::new` as data.

**Once at startup, by design.** The preferred-UI-language list itself can change while the
process runs, but Windows applies such a change to already-running processes only after a
sign-out, and every Windows program the user has open keeps its old language until then. A list
read at process start therefore matches what the rest of the desktop shows for the process
lifetime. A future platform that changes the UI language live would re-read and post a message;
nothing here precludes that, and nothing builds it.

**Windows implementation.** New file `src/platform/windows/locale.rs`, `pub struct
WindowsLanguageSource;`, re-exported from `platform/windows/mod.rs` for the binary. It calls
`GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, ..)` twice, the documented pattern: once with
`None` for the buffer and a size of 0 on input, which makes the call report the required size in
UTF-16 units including both terminating NULs; then with a buffer of that size. The result is a
double-NUL-terminated multi-string, split into tags at each NUL. The `windows` 0.62 binding
returns `Result<()>`, so the crate's usual `?` + `map_err` shape applies. Any failure returns an
empty list and logs one `warn` line; an empty list resolves to English. `Cargo.toml` gains the
`Win32_Globalization` feature. `LOCALE_NAME_MAX_LENGTH` is not needed, since the call reports
its own buffer size.

### 3. Controller

The controller owns the resolved language, as it owns every other piece of runtime state.

- New fields: `os_languages: Vec<String>` (from `Controller::new`) and `lang: Lang`, resolved at
  construction from `config.language` and the list.
- `pub fn lang(&self) -> Lang` for `main.rs` (see the tray below).
- `SettingChange::Language(LanguageSetting)`: sets `config.language = setting.wire()`, flags
  `dirty.language`, arms the debounced save, re-resolves. **If the resolved `Lang` changed**, it
  calls `self.osd.set_language(lang)` and `self.settings.set_language(lang)`. If it did not
  change (switching from "System default" to "Deutsch" on a German Windows), nothing is pushed:
  the config value still changes and is saved, but no relabelling runs.
- `SettingsSnapshot` gains two fields: `language: LanguageSetting`, which the picker displays,
  and `lang: Lang`, which every label in the window is resolved in. Both come from the
  controller, so the window can never disagree with it.
- The six `strings(Lang::English)` lookups the controller makes today read `self.lang`.
- `handle_restore_defaults` already resets the config and refreshes the window; it additionally
  re-resolves the language and pushes it when it changed, exactly like a `Language` change. The
  refresh goes out first, the language push second; the window handles that order correctly
  because a refresh never touches its language (see "Settings window" below).

**Language changes only ever originate in the settings window.** Both `SettingChange::Language`
and `RestoreDefaults` are posted by the dialog, so a push can never race the window's own
creation: by the time one is sent, the window exists and its `hwnd` slot is live. The
`OPENING` sentinel path in `SettingsSinkImpl` therefore needs no special case for the language
message; a post that finds no window is dropped like any other.

**The hotkey thread stops composing user-facing text.** Today it builds the "rebind failed and
restore also failed" message itself, resolving the table inline because it has no language of
its own. Instead, `BrightnessMessage::HotkeyRebindResult` carries `error: Option<String>` and
a new `restore_error: Option<String>`, both raw `BrightnessError` `Display` output; the
controller joins them with `hotkey_status_restore_also_failed_fmt` in its own language. After
this, the only `i18n` item `platform/windows/hotkey.rs` uses outside tests is the `Strings`
parameter of `display_text`; the `strings()`/`Lang` lookup at the composition site goes away.

### 4. Receivers

**OSD.** `OsdSink` gains `fn set_language(&mut self, lang: Lang)`. `OsdWindow`'s implementation
writes `OsdRenderState.lang` through the same `OSD_STATE.with` path the brightness fields use.
That is the missing writer §16 describes. The next paint renders in the new language; an OSD
that is visible at the moment of the switch is not repainted, since the error row it might be
showing disappears within the timeout anyway. `OsdWindow::new` takes the initial `Lang` so the
first paint is right without a separate call.

**Tray.** The tray is not a controller seam; `main.rs` already diffs
`controller.health_warnings()` once per loop tick and pushes changes through
`TrayStatusHandle::notify`. The language follows the same pattern: `main.rs` keeps `last_lang`,
and `TrayStatusHandle` gains `set_language(lang)`, posting a second private message
(`WM_TRAY_LANG = WM_APP + 102`, the slot after `WM_TRAY_STATUS`; `wparam` = index into
`Lang::ALL`, an out-of-range index is logged and ignored, never used to index). On receipt the
tray thread calls `set_tray_lang`, its first caller outside `TrayIcon::new`, and re-issues the
tooltip via `NIM_MODIFY` from the warnings it last received, which it now remembers in a
thread-local next to `STATUS_ICONS`. The menu is rebuilt on every open, so it picks the
language up the next time it is shown.

The tray's status handle arrives on a channel some ticks after the tray thread starts, and
`main.rs` already re-pushes `last_warnings` when it does. The language needs no such catch-up:
`spawn_tray_thread` takes the initial `Lang`, so the first tooltip is right, and a language
change cannot precede the handle because it can only come from the settings window, which is
opened from the tray menu, which exists only once the tray thread is up.

**Settings window.** `WindowState.lang` becomes `Cell<Lang>`, set from the snapshot's `lang`
at creation rather than from `Lang::default()`. **That cell has exactly two writers:** window
creation, and the `WM_APP_SETTINGS_LANG` handler below. `apply_snapshot` ignores `snap.lang`
on a refresh: if it updated the cell, `window_strings()` would answer in the new language while
every label set by `SetWindowTextW` still showed the old one. A refresh therefore never changes
the language, and the separate push always relabels in full.

`WM_APP_SETTINGS_LANG` is `WM_APP + 6`, the sixth entry in the contiguous `WM_APP_SETTINGS_*`
range. That range is pinned by a `const` assertion in `settings/window.rs` and is the span
`drain_pending_payload_messages` filters on when reclaiming boxed payloads; both are extended
to include the new constant, deliberately, as the comment there asks. The message carries its
`Lang` as an index into `Lang::ALL` in `wparam` with no heap payload, so it has nothing to
reclaim, and an out-of-range index is logged and ignored. `SettingsSinkImpl::set_language`
posts it. Its handler:

1. stores the new `Lang` in `WindowState.lang`;
2. walks `CONTROLS` and calls `SetWindowTextW` on every control with a `TextKey`;
3. sets the window title from `window_title`;
4. re-fills both combos (log level, language) under `SUPPRESS_NOTIFICATIONS`, preserving the
   selected index, so a re-fill is never mistaken for a user selection;
5. invalidates the two hotkey capture fields so they repaint through `display_text`.

`layout()` does **not** run again: control positions come from the constant table and do not
depend on text. A hotkey status line already on screen keeps its old-language text until the
next hotkey event replaces it; the line is transient and this is accepted. The window's own
message boxes (autostart failure, restore-defaults confirmation) already resolve through
`window_strings()` and follow the cell.

**Startup (`main.rs`).** The `let s = strings(..)` binding moves below `load_config()` and
resolves from the loaded config and the detected list. The one message box that precedes config
loading, "already running", uses the OS language directly: a second instance must not read the
config file, because the first instance may be saving it, and the single-instance guard has to
come before anything else. A fixed language choice that differs from the OS language therefore
does not reach that one box. This is a decision made here, recorded in §16 by this cycle, and
it means the OS-language read is the one thing that now precedes the single-instance guard: a
kernel32 call with no side effects, which the startup-order notes in §8 and §12 record.

### 5. The picker

A new first row in the "General" section: a label (`TextKey::LabelLanguage`,
`label_language`) at x 24 and a `COMBOBOX` at x 250, dropped height sized for the entries, ids
`ID_LABEL_LANGUAGE` and `ID_LANGUAGE`. Every row below shifts down by one 30 px row and
`BASE_WINDOW_HEIGHT` goes from 624 to 654; the hand-measured alignment note on the log-level
row moves with it unchanged, since only `y` changes.

**Width 120, not the log-level combo's 76.** The wider box is not a concession to German: the
*English* entry "System default" plus the 17 px dropdown arrow does not fit 76 px at 9 pt. The
right edge lands at 370, inside the 388 px content column. The two combos in the same column
now end at different x positions; that mismatch is accepted here and listed in the overflow
record so the hardening cycle aligns the column properly rather than this cycle guessing a
width for both.

Entries, in order: index 0 is `language_system_default` from the table ("System default",
translated into the current UI language); index `i + 1` is `Lang::ALL[i].native_name()`. The
selection is resolved from `CB_GETCURSEL` to a `LanguageSetting` by index, never from the text,
exactly as the log-level combo does. `apply_snapshot` selects the index for `snap.language`.
`CBN_SELCHANGE` posts `SettingChange::Language`.

The dark-mode subclass is installed by window class in `create_controls`, so the new combo gets
it for free. `configure_combo_height` is not free: it names `ID_LOG_LEVEL` directly and is
generalised to run over both combos.

### 6. Hotkey display text

`ParsedHotkey::display_text` gets its first production callers. §16 deferred this because in
English it would only re-case a hand-edited `ctrl+shift+up` on screen; with a second language
the display genuinely differs, and that objection no longer applies.

- **Capture fields.** The control's window text stays the canonical wire string: that is the
  *value*, read back by `bindings_conflict` and re-posted on change. The parse and the
  `display_text` call go into `capture_display_text`, the pure, host-tested function that
  already decides what the field shows, not into `paint_capture`, which is only its GDI
  wrapper; the idle branch parses the wire text and renders it, falling back to the raw text if
  parsing fails (it should not, the value came from the config validator or the capture
  itself). On a language switch the fields are invalidated and repaint.
- **Tray usage rows.** `usage_menu_lines` receives the wire strings as today and renders them
  through `parse_hotkey` + `display_text`, falling back to the wire string on a parse error.
- `preview_text` in the capture control already uses the table.

**Key names.** `Strings` gains one field per named key that a translation may render
differently: `key_up`, `key_down`, `key_left`, `key_right`, `key_page_up`, `key_page_down`,
`key_home`, `key_end`, `key_insert`, `key_delete`, `key_space`, `key_tab`, `key_enter`,
`key_escape`, `key_backspace`. `display_text` maps the virtual key to the field; function keys,
`Plus`, `Minus`, letters and digits keep their wire name, which no consulted project translates.
`VK_TO_NAME` is untouched and remains the wire table.

German follows the wording Windows itself uses in its accelerator labels and on the German
key cap: Nach-Oben, Nach-Unten, Nach-Links, Nach-Rechts, Bild auf, Bild ab, Pos1, Ende, Einfg,
Entf, Leertaste, Tab, Eingabe, Esc, Rücktaste. The arrow keys have no industry consensus
(Chromium: Aufwärtspfeil; Firefox: Pfeil nach oben; Qt: Hoch); the Explorer form was chosen
because it is what a German Windows user sees in every context menu, and because it contains
no space, which matters in the tray menu's tab-aligned column. These are key-cap and dictionary
words with no copyrightable expression; a comment above the block names the convention so a
later language can follow it. No third-party notice is added.

### 7. The German table

`GERMAN: Strings` fills every field. Conventions, all already recorded on the English table:

- Log levels show the English token with the German word appended: `warn (Warnung)`.
- Modifiers: Strg, Alt, Umschalt, Win; separator `+`.
- The product name stays `darkbright-helper` inside every sentence that carries it.
- Format fields keep their placeholders (`{name}`, `{path}`, `{error}`, `{restore_error}`)
  verbatim; only the surrounding text is translated.
- The footer link row keeps its two `<a>` spans.
- Percent, millisecond and second units stay `%`, `ms`, `s`.

`strings(Lang::German)` returns `&GERMAN`.

### 8. Recording the overflow

A `#[cfg(test)] #[ignore]` diagnostic test in `settings/layout.rs`, Windows-only, creates the
regular and bold fonts at 96 DPI, measures every `ControlSpec` label of every `Lang` with
`DrawTextW` + `DT_CALCRECT`, and prints one line per label whose measured width exceeds the
spec's `w`, with both numbers. It is ignored rather than asserting because German is known to
overflow today; the hardening cycle turns it into a gate once it passes. Running it, plus a
screenshot pass over the German window, tooltip and menu, produces
`personal/i18n-overflow-2026-09.md`: the list of overflowing labels with their measured and
available widths, the hotkey display lengths (`Strg+Umschalt+Nach-Rechts` is 25 characters
against 16 in English), and the two combo boxes' unequal right edges from the picker row. That
file is the hardening cycle's input.

### 9. Documentation

- `docs/architecture.md` §4: `language` row in the field table (`"system"` or a shipped tag,
  default `"system"`) and a sentence on strict validation.
- §8 startup-ordering note and §12 startup order: the OS-language read now precedes the
  single-instance guard.
- §14: the picker row in the control list, `WM_APP_SETTINGS_LANG` in the message list, and the
  `WM_APP_SETTINGS_*` range growing from five to six.
- §16: "Choosing the language" rewritten to describe the one owner and the push paths, including
  the "already running" box's OS-language exception; the "Not yet wired" paragraph and the
  `pub`-items exception updated, since `Lang::ALL`, `Lang::tag` and `display_text` now all have
  production callers; a new short "Adding a language" paragraph (add the variant, the tag, the
  native name, the table; the compiler lists the rest).
- "Integration Testing (Manual)": a "Language Switching Test" covering first start on a German
  Windows, the picker, live relabelling, the tooltip, the menu, the OSD error row, the "already
  running" box, and Restore Defaults switching the UI back to the OS language mid-dialog.
- `README.md`: one sentence under "Configuration" that the UI follows the Windows display
  language, currently English and German, and can be pinned in Settings.
- `CLAUDE.md` module map: `locale.rs` added.

## Testing

Host-testable in `core/`:

- Lookup: `de` → German, `DE` → German, `de-AT` → German, `de-AT-1901` → German, `de-x-foo`
  → German, `en-GB` → English, `fr` → `None`, `x-private` → `None`, `""` → `None`; preference
  lists `["fr", "de"]` → German, `[]` → English.
- `LanguageSetting::parse`: `"system"`, `"SYSTEM"`, `"de"`, `"de-CH"` accepted; `"fr"`,
  `"Deutsch"`, `"de_DE"`, `""` rejected. `wire()` round-trips `"system"` and `"de"`.
- Config: an invalid `language` yields `Unparseable { field: "language" }` and `"system"`;
  a valid one passes unchanged; `restore_defaults` resets it; `overlay_dirty` copies it only
  when flagged; `SettingsDirty::any()` is true for a language-only change; the existing "every
  field is defaulted / restored" tests extend to it.
- Controller: a `Language` change that alters the resolved language pushes to the OSD and
  settings fakes exactly once; one that does not alter it pushes nothing but still dirties and
  saves; a language-only session saves on close; `settings_snapshot` carries both new fields;
  `RestoreDefaults` with a fixed language pushes the OS language after the refresh; the
  restore-failed ack composes in the controller's language.
- `i18n`: the existing completeness test runs over `Lang::ALL`; the hand-written `KEYS` array in
  `every_key_resolves_to_a_non_empty_string_in_every_language` gains `LabelLanguage`; a new test
  checks that every `_fmt` field contains the same `{placeholder}` set in every language as in
  English; another that `native_name` is non-empty and unique; another that every language's
  `footer_links` contains exactly two `<a>`.

Windows-only, in-module:

- `display_text` under `GERMAN` renders `Ctrl+Shift+Up` as `Strg+Umschalt+Nach-Oben`, and the
  existing `english_display_text_matches_the_stored_format` still passes.
- `capture_display_text` renders a wire string through `display_text` in the idle state and
  falls back to the raw text for an unparseable one.
- The language combo maps each index to the right `LanguageSetting` and back, and a relabel
  posts no `SettingChange`.
- `WindowsLanguageSource::preferred_languages` on CI (`windows-latest`): the list is non-empty,
  and every entry starts with a two- or three-letter ASCII primary subtag followed by nothing or
  a hyphen. That second assertion is what pins `MUI_LANGUAGE_NAME`: the `MUI_LANGUAGE_ID` form
  returns hex `LANGID` strings such as `0409`, which a loose "parses as a tag" check would pass.
  The non-empty assertion assumes a runner account has a UI language, which every hosted
  Windows image does; if that ever fails, the test drops to well-formedness with a note, not to
  `#[ignore]`.
- The layout fit tests (`every_tabstop_control_fits_inside_the_client_rect`, the overlap
  check) still pass with the shifted rows.

Manual: the "Language Switching Test" above, plus the overflow record.

## Risks

- **Relabelling misses a control.** Any control created outside `CONTROLS` (the version line
  is the only one, and it has no `TextKey`) would keep its old text. Mitigated by walking the
  same table creation walks.
- **Combo re-fill fires a selection change.** Mitigated by `SUPPRESS_NOTIFICATIONS`, the same
  guard `apply_snapshot` uses; a test asserts no `SettingChange` is posted during a relabel.
- **German overflows.** Expected, measured, recorded, not fixed here. Nothing is clipped in a
  way that loses data: every value stays editable, only labels truncate.
- **`GetUserPreferredUILanguages` returns something unexpected** (an empty list on a stripped
  Windows image, a `LANGID` form if the flag were wrong). Falls back to English with a `warn`
  line; the CI test on `windows-latest` pins the flag's behaviour.
- **A hand-edited `"de-AT"` becomes `"de"` on the next dialog save.** Documented; the value
  the user meant is preserved, only its spelling normalises.

## Non-goals restated

This cycle makes the app translated and switchable. It does not make the settings window fit
the translation; it measures and records how badly it does not, so that the next cycle can.
