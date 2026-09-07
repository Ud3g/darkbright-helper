# Design: UI string centralization (i18n foundation)

_Date: 2026-09-07_

## Problem

Every user-visible string in the app is an English literal at its point of use. There are roughly
95 of them, spread across the settings window's control table, the tray menu and tooltip, the OSD,
and four message boxes. None live in `src/core/`; all of them sit in `src/platform/windows/`.

That is fine for a single-language app, but it blocks a selectable UI language entirely, and it
buries two couplings that would corrupt user data the moment a translation is introduced:

- **The hotkey display text is also the storage format.** `ParsedHotkey`'s `Display` impl
  (`src/platform/windows/hotkey.rs:774`) concatenates `"Ctrl"`, `"Alt"`, `"Shift"`, `"Win"` and a
  key name from `VK_TO_NAME` (`hotkey.rs:1099`), and that exact string is what `config.json` stores
  in `hotkeys.brightness_up` and what `parse_hotkey` reads back. The settings dialog's capture
  preview (`settings/capture.rs:269`) builds the same string from its own copy of those literals.
  Translating the displayed text without separating the two formats would write
  `"Strg+Umschalt+Auf"` into the config file and break every hand-edited config and every
  round trip.
- **The log-level combo entries are also the stored values.** `LOG_LEVELS`
  (`settings/window.rs:253`) feeds both `CB_ADDSTRING` and `logging.file_level`, which is parsed
  by `log::LevelFilter`.

A prior code-conventions review considered centralizing UI strings and declined, on the grounds
that it was cosmetic with no i18n plans. That reasoning no longer holds.

This design covers the centralization only. Language selection, live switching, layout hardening
under long translations, and the first actual translation are separate cycles that build on it.

## Scope

**In scope:**

- A new platform-agnostic `src/core/i18n.rs` holding the language enum, the string table type, and
  the English table.
- Every user-visible string sourced from that table instead of a literal at its call site.
- Separating hotkey **display text** from the **canonical wire format** that `config.json` stores.
- Separating log-level **display text** from the stored value.
- A translatable user-facing message for each `BrightnessError` variant that can reach a message
  box, while the `Display` impl stays English for logs.
- Freeing the settings window title from its compile-time `w!()` literal.

**Out of scope (later cycles):**

- **Any second language.** The table ships with English only. Its shape must make adding a language
  a mechanical edit, but no translation is written here.
- **The `language` config field, OS locale detection, and the picker control.** Nothing in this
  cycle reads a preference; every call site resolves to `Lang::English`.
- **Live language switching.** The plumbing must not preclude it, but no re-labelling path is built.
- **Layout hardening.** The settings window keeps its hand-measured 400x624 pixel table and its
  fixed columns. German labels measure up to 2.5x their English width, so this will overflow — that
  is the next cycle's problem, and deliberately not this one's.
- **Right-to-left support, CJK font selection, locale-aware percent formatting.**
- **Log messages.** All logging stays English, permanently and by design. A German log pasted into
  an issue is worse than an English one, and log strings are not a user interface.

## Design

### Where it lives

`src/core/i18n.rs`, platform-agnostic, compiles and unit-tests on any host. It holds:

- `enum Lang` — English only for now, with `Lang::ALL` and a BCP-47 tag per variant so the next
  cycle can add variants without touching consumers.
- `struct Strings` — one `&'static str` field per user-visible string, roughly 100 fields, grouped
  by surface with section comments.
- `const ENGLISH: Strings`.
- `fn strings(lang: Lang) -> &'static Strings`.
- `enum TextKey` plus `Strings::get(&self, key: TextKey) -> &'static str`.

### Why a struct, and why a key enum on top of it

The struct is what makes completeness a compile-time property. A Rust struct literal has no
partial-initialization escape hatch: adding a field breaks every language table with `E0063` until
it is filled in, and removing one breaks with `E0560`. No catalog format — JSON, FTL, YAML — gives
that guarantee, which is the reason this design takes no i18n dependency at all.

The key enum exists for one reason: the settings window's `CONTROLS` table is a `const` array, so
it cannot hold a resolved `&str` that depends on a runtime language. `ControlSpec.text` becomes
`Option<TextKey>`, and the text is resolved when the control is created or re-labelled.

`Strings::get` is an exhaustive `match` over `TextKey`, so a new key forces both a new field in
every language table and a new arm in `get`. The two checks are independent and both are compile
errors.

**Everywhere outside the control table, call sites use the field directly** — `s.tray_settings`
rather than `s.get(TextKey::TraySettings)`. Direct access reads better and needs no enum entry, so
`TextKey` covers only the roughly 26 control-table labels rather than all 100 strings.

### How strings reach the UI

Each UI component gains a `lang: Lang` field, fixed at `Lang::English` in this cycle, and resolves
`strings(self.lang)` where it builds text. The free text-composing functions in the tray
(`compose_tooltip`, `warning_menu_lines`, `usage_menu_lines`) take `&'static Strings` as a
parameter, which keeps them pure and host-testable exactly as they are today.

No global, no `OnceLock`, no atomic. The app's architecture is single-owner state with message
passing and no shared mutable globals; a global string table would add a concurrency question to a
design that deliberately has none, and would force the currently parallel unit tests to serialize.
The next cycle sets `lang` from config at construction time.

### Hotkey display and storage

`ParsedHotkey`'s `Display` impl stays exactly as it is and becomes the explicitly documented
**canonical wire format**: English, stable across languages and versions, the only thing written to
or read from `config.json`. `bindings_conflict` keeps comparing wire format.

A new `ParsedHotkey::display_text(&self, s: &Strings) -> String` produces the **display** form,
built from `s.mod_ctrl`, `s.mod_alt`, `s.mod_shift`, `s.mod_win` and a translatable key name. In
English it renders identically to today, so this cycle changes nothing a user sees.

`VK_TO_NAME` stays the wire-format table and stays English. Key names get display entries in
`Strings`; a language is free to leave them identical to English, and function keys and single
characters always will be. `GetKeyNameTextW` is deliberately **not** adopted: it localizes by
keyboard layout rather than display language, produces very long all-caps German names such as
`EINGABE (ZEHNERTASTATUR)`, and carries known Num Lock and Pause defects.

`settings/capture.rs`'s `preview_text` switches to the same display path, removing its duplicate
copy of the modifier literals.

### Log levels

`LOG_LEVELS` stays the stored-value array, indexed as it is today. The combo displays
`s.log_level_error` and friends. English entries read `error`, `warn`, `info`, `debug`, `trace`; a
translation renders the English token with its translation appended, as in `warn (Warnung)`, so the
value that lands in `config.json` stays visible to anyone editing the file by hand. Selection
continues to resolve through `CB_GETCURSEL` to an index, never through the displayed text.

### Error messages

`BrightnessError`'s `Display` impl stays English and unchanged. It is the logging representation and
several variants embed file paths and Win32 error codes that belong in a log, not a dialog.

A separate `fn user_message(&self, s: &Strings) -> String` covers the variants that can actually
reach a message box: the ones returned by `DdcSupervisor::spawn`, by hotkey registration, and by the
autostart registry write. Those three call sites (`src/main.rs:609`, `src/main.rs:695`,
`settings/window.rs:1218`) today wrap `{e}` in hand-written English advice prose; that prose moves
into `Strings` as well. Variants that cannot reach a dialog fall through to the English `Display`
output, which keeps the mapping honest rather than inventing user-facing text for messages no user
will see.

### Window title

`w!("darkbright-helper Settings")` (`settings/window.rs:1813`) becomes a runtime `wide()` call over
`s.settings_window_title`. Every other user-visible string already flows through a runtime UTF-16
conversion, so this is the only mechanical obstacle in the codebase.

## Testing

Host-testable in `core/`:

- Every `TextKey` resolves to a non-empty string for every `Lang`.
- `Lang::ALL` covers every variant and every tag is unique and lowercase.

Windows-only, in-module:

- Every `ControlSpec` with an `Option<TextKey>` resolves; every control that previously had a
  non-empty `text` still has one.
- `ParsedHotkey` round-trips through the wire format unchanged, and `display_text` equals the wire
  format under `Lang::English` — this is the regression guard for the config-corruption risk.
- The log-level combo maps each index to the correct stored value regardless of display text.

The existing tests that assert on English literals (`hotkey.rs:1365` and the settings layout fit
tests) keep passing unchanged, which is the cycle's main safety property: **no user-visible
behaviour changes.**

## Risks

- **Wide, mechanical diff.** Roughly 95 call sites across seven files. Mitigated by shipping it as
  one reviewable cycle with no behaviour change, so the diff can be read as a mapping rather than as
  logic.
- **A missed literal.** Nothing catches a string that stays hard-coded. A grep-based sweep of
  string literals in the platform module is part of the work, and the pseudo-language planned for
  the layout cycle will surface any survivors immediately, because untranslated text stays plain
  ASCII while everything else is visibly accented.
- **`Strings` grows to ~100 fields.** Acceptable: it is a flat data table with section comments, and
  keeping it in one file is what makes the completeness guarantee legible.

## Non-goals restated

This cycle makes the app translatable. It does not make it translated, and it does not make the
settings window survive a translation. Both are known, both are planned, and neither belongs here.
