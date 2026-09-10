//! The user-visible string table.
//!
//! Every string a user can read lives here, once per language, so the UI can
//! be switched between languages without hunting literals across the platform
//! layer. Log messages are deliberately absent: they stay English so that a
//! pasted log is readable by whoever is diagnosing it.
//!
//! [`Strings`] is a plain struct rather than a map or an external catalog
//! because that makes completeness a compile-time property. Adding a field
//! breaks every language's initializer until it is filled in, and removing one
//! breaks every reference. No catalog format offers that.
//!
//! Each language's table lives in its own file under `i18n/`, named by its
//! tag, with a header recording the choices a later correction needs.
//!
//! The product name `darkbright-helper` is not in here — a product name is not
//! translated.

mod de;
mod en;

pub(crate) use de::GERMAN;
pub(crate) use en::ENGLISH;

/// A language the user interface can be displayed in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    /// English, the fallback for every unmatched locale.
    #[default]
    English,
    /// German.
    German,
}

impl Lang {
    /// Every language the app can display, in the order the picker shows them.
    pub const ALL: &'static [Lang] = &[Lang::English, Lang::German];

    /// The BCP-47 tag identifying this language: what `config.json` stores
    /// for a fixed choice and what locale matching compares against. Always
    /// lowercase and generic (`de`, never `de-DE`).
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::German => "de",
        }
    }

    /// The language's name in itself, for the picker: a user who cannot read
    /// the current UI language can still find their own.
    #[must_use]
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::German => "Deutsch",
        }
    }

    /// Position in [`Lang::ALL`]. Used to carry a language through a Win32
    /// message `wparam` and as the picker's combo index.
    ///
    /// # Panics
    ///
    /// Never in practice: every `Lang` variant is listed in [`Lang::ALL`].
    #[must_use]
    pub fn index(self) -> usize {
        Lang::ALL
            .iter()
            .position(|&l| l == self)
            .expect("every Lang variant is listed in Lang::ALL")
    }

    /// Inverse of [`Lang::index`]; `None` for an index outside [`Lang::ALL`].
    #[must_use]
    pub fn from_index(index: usize) -> Option<Lang> {
        Lang::ALL.get(index).copied()
    }

    /// RFC 4647 §3.4 lookup of one language tag against the shipped
    /// languages: the whole tag first, then with subtags removed from the
    /// right until something matches. After each removal a trailing
    /// single-character subtag (an extension or private-use singleton) is
    /// removed too. Case-insensitive. `None` when nothing matches.
    #[must_use]
    pub(crate) fn lookup(tag: &str) -> Option<Lang> {
        let lowered = tag.to_ascii_lowercase();
        let mut subtags: Vec<&str> = lowered.split('-').collect();
        loop {
            let candidate = subtags.join("-");
            if let Some(&lang) = Lang::ALL.iter().find(|l| l.tag() == candidate) {
                return Some(lang);
            }
            subtags.pop()?;
            if subtags.last().is_some_and(|s| s.len() == 1) {
                subtags.pop();
            }
            if subtags.is_empty() {
                return None;
            }
        }
    }

    /// The first entry of an ordered preference list (most preferred first)
    /// that `Lang::lookup` resolves, or English when none does. The
    /// documented fallback: English is the table every other language is a
    /// translation of, so it is the one language that always exists.
    #[must_use]
    pub fn from_preferences(preferred: &[String]) -> Lang {
        preferred
            .iter()
            .find_map(|tag| Lang::lookup(tag))
            .unwrap_or(Lang::English)
    }
}

/// The `language` config value that means "follow the OS display language".
pub(crate) const SYSTEM_LANGUAGE: &str = "system";

/// The parsed `language` config value: follow the OS, or one fixed language.
///
/// Only the choice is stored, never the language it resolved to, so a config
/// carried to another machine follows that machine's OS language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LanguageSetting {
    /// Follow the OS display language (`SYSTEM_LANGUAGE` in the file).
    #[default]
    System,
    /// Always this language, whatever the OS says.
    Fixed(Lang),
}

impl LanguageSetting {
    /// Parses a config value. Accepts [`SYSTEM_LANGUAGE`] (any case) or any
    /// tag [`Lang::lookup`] resolves; everything else, a well-formed but
    /// unshipped tag included, is `None`, so the config loader reports it
    /// like any other unparseable field.
    #[must_use]
    pub(crate) fn parse(value: &str) -> Option<Self> {
        if value.eq_ignore_ascii_case(SYSTEM_LANGUAGE) {
            return Some(Self::System);
        }
        Lang::lookup(value).map(Self::Fixed)
    }

    /// The value written to the config file: [`SYSTEM_LANGUAGE`] or the
    /// language's tag. A value read as `de-AT` is written back as `de`.
    #[must_use]
    pub(crate) fn wire(self) -> &'static str {
        match self {
            Self::System => SYSTEM_LANGUAGE,
            Self::Fixed(lang) => lang.tag(),
        }
    }

    /// The language to display, given the OS preference list.
    #[must_use]
    pub fn resolve(self, preferred: &[String]) -> Lang {
        match self {
            Self::System => Lang::from_preferences(preferred),
            Self::Fixed(lang) => lang,
        }
    }
}

/// Seam for the OS's ordered UI-language preference list.
///
/// Read once at startup: Windows applies a change of the display language to
/// already-running processes only after a sign-out, so the list read at
/// process start matches what the rest of the desktop shows for the process
/// lifetime. The binary calls it and hands the list to the controller as
/// data; nothing in `core/` holds the source itself.
pub trait LanguageSource {
    /// Language tags, most preferred first. Empty when the OS cannot say,
    /// which resolves to English.
    fn preferred_languages(&self) -> Vec<String>;
}

/// Every user-visible string, for one language.
///
/// Fields are grouped by the surface they appear on. Keep the groups and their
/// order identical in every language table so any two can be diffed.
#[derive(Debug, Clone, Copy)]
pub struct Strings {
    // --- On-screen display ---
    /// Shown in the OSD's error row when a DDC write failed.
    pub osd_ddc_error: &'static str,

    // --- Tray tooltip ---
    /// Tooltip fragment: the DDC worker is not running.
    pub tray_tip_ddc_unavailable: &'static str,
    /// Tooltip fragment: the DDC worker is not answering.
    pub tray_tip_monitor_unresponsive: &'static str,
    /// Tooltip fragment: the hotkey thread stopped.
    pub tray_tip_hotkeys_stopped: &'static str,
    /// Tooltip fragment: a rebind did not take effect.
    pub tray_tip_hotkey_change_failed: &'static str,
    /// Tooltip fragment: the log file could not be opened.
    pub tray_tip_file_logging_off: &'static str,

    // --- Tray warning rows ---
    /// Warning row shown when the DDC worker is not running.
    pub tray_warn_ddc_unavailable: &'static str,
    /// Warning row shown when a monitor stopped answering.
    pub tray_warn_monitor_unresponsive: &'static str,
    /// Warning row shown when the hotkey thread died for good.
    pub tray_warn_hotkeys_stopped: &'static str,
    /// Warning row shown when a hotkey rebind failed.
    pub tray_warn_hotkey_change_failed: &'static str,
    /// Warning row shown when file logging could not start.
    pub tray_warn_file_logging_failed: &'static str,

    // --- Tray usage rows and commands ---
    /// Heading above the two hotkey usage rows.
    pub tray_usage_heading: &'static str,
    /// Usage row label for the brightness-up hotkey.
    pub tray_usage_brighter: &'static str,
    /// Usage row label for the brightness-down hotkey.
    pub tray_usage_dimmer: &'static str,
    /// Menu command opening the settings window.
    pub tray_menu_settings: &'static str,
    /// Menu command opening the log folder in Explorer.
    pub tray_menu_open_log_folder: &'static str,
    /// Menu command that exits the app. Takes the product name, so the
    /// translation must contain `{name}`.
    pub tray_menu_quit_fmt: &'static str,

    // --- Hotkey display ---
    //
    // These are display-only. The canonical hotkey format written to
    // config.json is always English and lives in `ParsedHotkey`'s `Display`
    // impl; translating these never changes what is stored.
    /// Display name of the Ctrl modifier.
    pub key_mod_ctrl: &'static str,
    /// Display name of the Alt modifier.
    pub key_mod_alt: &'static str,
    /// Display name of the Shift modifier.
    pub key_mod_shift: &'static str,
    /// Display name of the Windows modifier.
    pub key_mod_win: &'static str,
    /// Separator placed between modifiers and the key name.
    pub key_separator: &'static str,

    // --- Key names ---
    //
    // Display names for the keys `VK_TO_NAME` in `platform/windows/hotkey.rs`
    // can name, one field per key a translation may render differently.
    // Function keys, `Plus`, `Minus`, letters and digits keep their wire
    // name in every language and have no field. Each language follows its
    // own platform convention — the wording Windows uses in that language's
    // accelerator labels and on its keyboard caps — recorded in the header of
    // its table file.
    /// Display name of the Up arrow key.
    pub key_up: &'static str,
    /// Display name of the Down arrow key.
    pub key_down: &'static str,
    /// Display name of the Left arrow key.
    pub key_left: &'static str,
    /// Display name of the Right arrow key.
    pub key_right: &'static str,
    /// Display name of Page Up.
    pub key_page_up: &'static str,
    /// Display name of Page Down.
    pub key_page_down: &'static str,
    /// Display name of Home.
    pub key_home: &'static str,
    /// Display name of End.
    pub key_end: &'static str,
    /// Display name of Insert.
    pub key_insert: &'static str,
    /// Display name of Delete.
    pub key_delete: &'static str,
    /// Display name of the space bar.
    pub key_space: &'static str,
    /// Display name of Tab.
    pub key_tab: &'static str,
    /// Display name of Enter.
    pub key_enter: &'static str,
    /// Display name of Escape.
    pub key_escape: &'static str,
    /// Display name of Backspace.
    pub key_backspace: &'static str,

    // --- Settings window ---
    /// Section header above the general settings.
    pub header_general: &'static str,
    /// Label before the language picker.
    pub label_language: &'static str,
    /// First entry of the language picker: follow the OS display language.
    pub language_system_default: &'static str,
    /// Checkbox enabling the Windows startup entry.
    pub autostart: &'static str,
    /// Label for the per-keypress brightness step.
    pub label_step: &'static str,
    /// Percent unit after the brightness step field.
    pub unit_percent_step: &'static str,
    /// Section header above the hotkey settings.
    pub header_hotkeys: &'static str,
    /// Label for the brightness-up hotkey field.
    pub label_hotkey_up: &'static str,
    /// Label for the brightness-down hotkey field.
    pub label_hotkey_down: &'static str,
    /// Checkbox enabling the low-level keyboard hook.
    pub intercept: &'static str,
    /// Hint below the interception checkbox.
    pub hint_intercept: &'static str,
    /// Section header above the on-screen display settings.
    pub header_osd: &'static str,
    /// Label for the OSD display duration.
    pub label_timeout: &'static str,
    /// Millisecond unit after the duration field.
    pub unit_milliseconds: &'static str,
    /// Label for the OSD opacity.
    pub label_opacity: &'static str,
    /// Percent unit after the opacity field.
    pub unit_percent_opacity: &'static str,
    /// Section header above the advanced settings.
    pub header_advanced: &'static str,
    /// Checkbox enabling the periodic brightness resync.
    pub resync_check: &'static str,
    /// Second unit after the resync interval field.
    pub unit_seconds_resync: &'static str,
    /// Checkbox enabling the resync after inactivity.
    pub inactivity_check: &'static str,
    /// Second unit after the inactivity field.
    pub unit_seconds_inactivity: &'static str,
    /// Checkbox enabling the log file.
    pub log_check: &'static str,
    /// Label before the log level picker.
    pub label_log_level: &'static str,
    // --- Log level picker ---
    //
    // Display only; the value written to config.json is resolved from the
    // combo's selected index, never from this text. Every language shows the
    // bare English token: a translation in parentheses is permitted by the
    // surrounding rule, but the longest one needs roughly twice the combo's
    // width, and widening the combo would cost the window more room than a
    // diagnostic picker is worth.
    /// Log level picker entry for `error`.
    pub log_level_error: &'static str,
    /// Log level picker entry for `warn`.
    pub log_level_warn: &'static str,
    /// Log level picker entry for `info`.
    pub log_level_info: &'static str,
    /// Log level picker entry for `debug`.
    pub log_level_debug: &'static str,
    /// Log level picker entry for `trace`.
    pub log_level_trace: &'static str,
    /// Hint below the logging settings.
    pub hint_logging: &'static str,
    /// The footer link row, containing two `<a>` link spans.
    pub footer_links: &'static str,
    /// Button restoring the default settings.
    pub button_restore_defaults: &'static str,
    /// Button closing the settings window.
    pub button_close: &'static str,
    /// The settings window's title bar text.
    pub window_title: &'static str,

    // --- Hotkey capture field ---
    //
    // The prompt and the three rejections a capture control shows in place of
    // a binding. The binding it produces is always the canonical English wire
    // format, so none of this reaches `config.json`.
    /// Placeholder shown while a capture field waits for the first key.
    pub capture_prompt: &'static str,
    /// Rejection: the combination has no Ctrl, Alt or Win modifier.
    pub capture_reject_no_modifier: &'static str,
    /// Rejection: the pressed key has no name the config format can store.
    pub capture_reject_unnameable_key: &'static str,
    /// Rejection: the combination is already bound to the other hotkey.
    pub capture_reject_duplicate: &'static str,

    // --- Hotkey status line ---
    //
    // Shown in the settings window's inline status line. Composed in the
    // controller, which is platform-agnostic and resolves the table inline
    // rather than holding a language of its own.
    /// Status: the hotkey thread could not be reached at all.
    pub hotkey_status_unreachable: &'static str,
    /// Status: the hotkey thread was reached but never acknowledged.
    pub hotkey_status_no_response: &'static str,
    /// Status: the hotkey thread reported a failure with no detail.
    pub hotkey_status_unknown_error: &'static str,
    /// Status: a rebind failed and putting the previous bindings back failed
    /// too. Takes both error details, so the translation must contain
    /// `{error}` and `{restore_error}`.
    pub hotkey_status_restore_also_failed_fmt: &'static str,
    /// Status: the low-level keyboard hook could not be installed, so plain
    /// registration is in use. A notice, not an error — the binding works.
    pub hotkey_notice_interception_unavailable: &'static str,

    // --- Message boxes ---
    //
    // Titles carry the product name, which is not translated; the translation
    // supplies only the part after the dash. The bodies are not all name-free:
    // `msgbox_already_running` and `msgbox_startup_failed_lead` spell the
    // product name out inside the sentence, and a translation must keep it
    // rather than replacing it with a translated noun. `Display` on
    // `BrightnessError` stays English for logs — see
    // `BrightnessError::user_message`.
    /// Message box body shown when a second instance is started.
    pub msgbox_already_running: &'static str,
    /// Title suffix for a fatal startup failure.
    pub msgbox_title_startup_error: &'static str,
    /// Title suffix for a hotkey registration failure.
    pub msgbox_title_hotkey_error: &'static str,
    /// Title suffix for the autostart registry failure.
    pub msgbox_title_autostart: &'static str,
    /// Title suffix for the restore-defaults confirmation.
    pub msgbox_title_restore_defaults: &'static str,
    /// Lead line of a fatal startup failure, above the error detail.
    pub msgbox_startup_failed_lead: &'static str,
    /// Advice shown when the OS refused to start a thread.
    pub msgbox_thread_spawn_advice: &'static str,
    /// Lead line of a hotkey registration failure, above the error detail.
    pub msgbox_hotkey_failed_lead: &'static str,
    /// Advice shown when hotkey registration failed. Takes the config file
    /// path, so the translation must contain `{path}`.
    pub msgbox_hotkey_advice_fmt: &'static str,
    /// Body of the autostart failure. Takes the error detail, so the
    /// translation must contain `{error}`.
    pub msgbox_autostart_failed_fmt: &'static str,
    /// Body of the restore-defaults confirmation.
    pub msgbox_restore_defaults_question: &'static str,
    /// Placeholder used when the config path cannot be determined.
    pub msgbox_config_file_fallback: &'static str,
}

/// A label in the settings window's control table.
///
/// The table is a `const`, so it cannot hold a string that depends on the
/// current language. It holds one of these instead, resolved when a control is
/// created or re-labelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextKey {
    /// Section header above the general settings.
    HeaderGeneral,
    /// Label before the language picker.
    LabelLanguage,
    /// Checkbox enabling the Windows startup entry.
    Autostart,
    /// Label for the per-keypress brightness step.
    LabelStep,
    /// Percent unit after the brightness step field.
    UnitPercentStep,
    /// Section header above the hotkey settings.
    HeaderHotkeys,
    /// Label for the brightness-up hotkey field.
    LabelHotkeyUp,
    /// Label for the brightness-down hotkey field.
    LabelHotkeyDown,
    /// Checkbox enabling the low-level keyboard hook.
    Intercept,
    /// Hint below the interception checkbox.
    HintIntercept,
    /// Section header above the on-screen display settings.
    HeaderOsd,
    /// Label for the OSD display duration.
    LabelTimeout,
    /// Millisecond unit after the duration field.
    UnitMilliseconds,
    /// Label for the OSD opacity.
    LabelOpacity,
    /// Percent unit after the opacity field.
    UnitPercentOpacity,
    /// Section header above the advanced settings.
    HeaderAdvanced,
    /// Checkbox enabling the periodic brightness resync.
    ResyncCheck,
    /// Second unit after the resync interval field.
    UnitSecondsResync,
    /// Checkbox enabling the resync after inactivity.
    InactivityCheck,
    /// Second unit after the inactivity field.
    UnitSecondsInactivity,
    /// Checkbox enabling the log file.
    LogCheck,
    /// Label before the log level picker.
    LabelLogLevel,
    /// Hint below the logging settings.
    HintLogging,
    /// The footer link row, containing two `<a>` link spans.
    FooterLinks,
    /// Button restoring the default settings.
    ButtonRestoreDefaults,
    /// Button closing the settings window.
    ButtonClose,
}

impl Strings {
    /// The text for `key`.
    ///
    /// Exhaustive by construction: a new [`TextKey`] variant fails to compile
    /// here until it is given a field, and a new field fails to compile in
    /// every language table until it is translated.
    #[must_use]
    pub(crate) fn get(&self, key: TextKey) -> &'static str {
        match key {
            TextKey::HeaderGeneral => self.header_general,
            TextKey::LabelLanguage => self.label_language,
            TextKey::Autostart => self.autostart,
            TextKey::LabelStep => self.label_step,
            TextKey::UnitPercentStep => self.unit_percent_step,
            TextKey::HeaderHotkeys => self.header_hotkeys,
            TextKey::LabelHotkeyUp => self.label_hotkey_up,
            TextKey::LabelHotkeyDown => self.label_hotkey_down,
            TextKey::Intercept => self.intercept,
            TextKey::HintIntercept => self.hint_intercept,
            TextKey::HeaderOsd => self.header_osd,
            TextKey::LabelTimeout => self.label_timeout,
            TextKey::UnitMilliseconds => self.unit_milliseconds,
            TextKey::LabelOpacity => self.label_opacity,
            TextKey::UnitPercentOpacity => self.unit_percent_opacity,
            TextKey::HeaderAdvanced => self.header_advanced,
            TextKey::ResyncCheck => self.resync_check,
            TextKey::UnitSecondsResync => self.unit_seconds_resync,
            TextKey::InactivityCheck => self.inactivity_check,
            TextKey::UnitSecondsInactivity => self.unit_seconds_inactivity,
            TextKey::LogCheck => self.log_check,
            TextKey::LabelLogLevel => self.label_log_level,
            TextKey::HintLogging => self.hint_logging,
            TextKey::FooterLinks => self.footer_links,
            TextKey::ButtonRestoreDefaults => self.button_restore_defaults,
            TextKey::ButtonClose => self.button_close,
        }
    }
}

/// The string table for `lang`.
#[must_use]
pub fn strings(lang: Lang) -> &'static Strings {
    match lang {
        Lang::English => &ENGLISH,
        Lang::German => &GERMAN,
    }
}

#[cfg(test)]
mod tests;
