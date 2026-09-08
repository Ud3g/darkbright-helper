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
//! The product name `darkbright-helper` is not in here — a product name is not
//! translated.

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
}

/// Every user-visible string, for one language.
///
/// Fields are grouped by the surface they appear on. Keep the groups and their
/// order identical in every language table so the two can be diffed.
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
    // name in every language and have no field. German follows the wording
    // Windows itself uses in accelerator labels and on the German key cap
    // (Pos1, Entf, Einfg, Bild auf, Rücktaste, Nach-Oben); a later language
    // should follow its own platform convention the same way.
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
    // Display only. The value written to config.json is always the English
    // token, resolved from the combo's selected index — never from this text.
    // A translation appends its own wording, as in "warn (Warnung)", so the
    // stored value stays visible to anyone editing the file by hand.
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
    // Non-test code doesn't construct this yet; the picker control that will
    // is a later change, and `#[expect]` can't be used because the test
    // target already constructs it (in the `KEYS` array below) while the
    // non-test target doesn't. Remove this attribute once the picker lands.
    #[allow(dead_code)]
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

/// The English strings. Every other language is a translation of this table.
pub(crate) const ENGLISH: Strings = Strings {
    osd_ddc_error: "DDC Error - Adjustment failed",

    tray_tip_ddc_unavailable: "DDC unavailable",
    tray_tip_monitor_unresponsive: "monitor not responding",
    tray_tip_hotkeys_stopped: "hotkeys stopped",
    tray_tip_hotkey_change_failed: "hotkey change failed",
    tray_tip_file_logging_off: "file logging off",

    tray_warn_ddc_unavailable: "⚠ DDC unavailable — press a brightness hotkey to retry",
    tray_warn_monitor_unresponsive: "⚠ Monitor not responding — restart the app if this persists",
    tray_warn_hotkeys_stopped: "⚠ Hotkeys stopped working — restart the app",
    tray_warn_hotkey_change_failed: "⚠ Hotkey change failed — try another combination",
    tray_warn_file_logging_failed: "⚠ File logging failed to start — check the log folder is writable",

    tray_usage_heading: "Point mouse at a monitor, then:",
    tray_usage_brighter: "Brighter",
    tray_usage_dimmer: "Dimmer",
    tray_menu_settings: "Settings",
    tray_menu_open_log_folder: "Open Log Folder",
    tray_menu_quit_fmt: "Quit {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Shift",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Up",
    key_down: "Down",
    key_left: "Left",
    key_right: "Right",
    key_page_up: "PageUp",
    key_page_down: "PageDown",
    key_home: "Home",
    key_end: "End",
    key_insert: "Insert",
    key_delete: "Delete",
    key_space: "Space",
    key_tab: "Tab",
    key_enter: "Enter",
    key_escape: "Escape",
    key_backspace: "Backspace",

    header_general: "General",
    label_language: "Language",
    language_system_default: "System default",
    autostart: "Start with Windows",
    label_step: "Brightness step per keypress",
    unit_percent_step: "%",
    header_hotkeys: "Hotkeys",
    label_hotkey_up: "Brightness up",
    label_hotkey_down: "Brightness down",
    intercept: "Try to intercept dedicated brightness keys",
    hint_intercept: "(may not work with all keyboards; some antivirus software flags low-level hooks)",
    header_osd: "On-screen display",
    label_timeout: "Display duration",
    unit_milliseconds: "ms",
    label_opacity: "Opacity",
    unit_percent_opacity: "%",
    header_advanced: "Advanced",
    resync_check: "Resync brightness every",
    unit_seconds_resync: "s",
    inactivity_check: "Resync after inactivity of",
    unit_seconds_inactivity: "s",
    log_check: "Write log file",
    label_log_level: "Level:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(logging changes take effect after restart; debug and below log monitor serials and paths)",
    footer_links: "<a>Open config file</a> \u{b7} <a>Open log folder</a>",
    button_restore_defaults: "Restore defaults",
    button_close: "Close",
    window_title: "darkbright-helper Settings",

    capture_prompt: "Press a key combination… (Esc to cancel)",
    capture_reject_no_modifier: "Add Ctrl, Alt, or Win (Shift alone isn't enough)",
    capture_reject_unnameable_key: "That key can't be used as a hotkey",
    capture_reject_duplicate: "Already assigned to the other brightness hotkey",

    hotkey_status_unreachable: "Could not reach the hotkey thread",
    hotkey_status_no_response: "Hotkey thread did not respond",
    hotkey_status_unknown_error: "unknown error",
    hotkey_status_restore_also_failed_fmt: "{error}; restore also failed: {restore_error}",
    hotkey_notice_interception_unavailable: "Brightness-key interception unavailable; using plain key registration",

    msgbox_already_running: "darkbright-helper is already running.",
    msgbox_title_startup_error: "Startup Error",
    msgbox_title_hotkey_error: "Hotkey Error",
    msgbox_title_autostart: "Autostart",
    msgbox_title_restore_defaults: "Restore Defaults",
    msgbox_startup_failed_lead: "darkbright-helper could not start:",
    msgbox_thread_spawn_advice: "The system would not start a thread, which usually means it is out of resources. Close some applications, or restart the computer, and try again.",
    msgbox_hotkey_failed_lead: "Failed to register hotkeys:",
    msgbox_hotkey_advice_fmt: "Possible solutions:\n• Close other applications that might be using these hotkeys\n• Change the hotkey configuration in:\n  {path}\n• Restart the application after making changes",
    msgbox_autostart_failed_fmt: "Couldn't update the Windows startup entry:\n{error}",
    msgbox_restore_defaults_question: "Reset all settings to their defaults? Hotkeys are applied immediately.",
    msgbox_config_file_fallback: "config file",
};

/// The German strings.
pub(crate) const GERMAN: Strings = Strings {
    osd_ddc_error: "DDC-Fehler - Anpassung fehlgeschlagen",

    tray_tip_ddc_unavailable: "DDC nicht verfügbar",
    tray_tip_monitor_unresponsive: "Monitor antwortet nicht",
    tray_tip_hotkeys_stopped: "Hotkeys gestoppt",
    tray_tip_hotkey_change_failed: "Hotkey-Änderung fehlgeschlagen",
    tray_tip_file_logging_off: "Dateiprotokoll aus",

    tray_warn_ddc_unavailable: "⚠ DDC nicht verfügbar — Helligkeits-Hotkey drücken, um es erneut zu versuchen",
    tray_warn_monitor_unresponsive: "⚠ Monitor antwortet nicht — App neu starten, falls das anhält",
    tray_warn_hotkeys_stopped: "⚠ Hotkeys funktionieren nicht mehr — App neu starten",
    tray_warn_hotkey_change_failed: "⚠ Hotkey-Änderung fehlgeschlagen — andere Kombination versuchen",
    tray_warn_file_logging_failed: "⚠ Dateiprotokoll konnte nicht gestartet werden — prüfen, ob der Protokollordner beschreibbar ist",

    tray_usage_heading: "Maus auf einen Monitor zeigen, dann:",
    tray_usage_brighter: "Heller",
    tray_usage_dimmer: "Dunkler",
    tray_menu_settings: "Einstellungen",
    tray_menu_open_log_folder: "Protokollordner öffnen",
    tray_menu_quit_fmt: "{name} beenden",

    key_mod_ctrl: "Strg",
    key_mod_alt: "Alt",
    key_mod_shift: "Umschalt",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Nach-Oben",
    key_down: "Nach-Unten",
    key_left: "Nach-Links",
    key_right: "Nach-Rechts",
    key_page_up: "Bild auf",
    key_page_down: "Bild ab",
    key_home: "Pos1",
    key_end: "Ende",
    key_insert: "Einfg",
    key_delete: "Entf",
    key_space: "Leertaste",
    key_tab: "Tab",
    key_enter: "Eingabe",
    key_escape: "Esc",
    key_backspace: "Rücktaste",

    header_general: "Allgemein",
    label_language: "Sprache",
    language_system_default: "Systemstandard",
    autostart: "Mit Windows starten",
    label_step: "Helligkeitsschritt pro Tastendruck",
    unit_percent_step: "%",
    header_hotkeys: "Hotkeys",
    label_hotkey_up: "Helligkeit erhöhen",
    label_hotkey_down: "Helligkeit verringern",
    intercept: "Versuchen, dedizierte Helligkeitstasten abzufangen",
    hint_intercept: "(funktioniert nicht mit allen Tastaturen; manche Antivirenprogramme melden Low-Level-Hooks)",
    header_osd: "Bildschirmanzeige",
    label_timeout: "Anzeigedauer",
    unit_milliseconds: "ms",
    label_opacity: "Deckkraft",
    unit_percent_opacity: "%",
    header_advanced: "Erweitert",
    resync_check: "Helligkeit abgleichen alle",
    unit_seconds_resync: "s",
    inactivity_check: "Abgleich nach Inaktivität von",
    unit_seconds_inactivity: "s",
    log_check: "Protokolldatei schreiben",
    label_log_level: "Stufe:",
    log_level_error: "error (Fehler)",
    log_level_warn: "warn (Warnung)",
    log_level_info: "info (Info)",
    log_level_debug: "debug (Debug)",
    log_level_trace: "trace (Ablaufverfolgung)",
    hint_logging: "(Protokolländerungen gelten nach dem Neustart; debug und darunter protokollieren Monitor-Seriennummern und Pfade)",
    footer_links: "<a>Konfigurationsdatei öffnen</a> \u{b7} <a>Protokollordner öffnen</a>",
    button_restore_defaults: "Standardwerte wiederherstellen",
    button_close: "Schließen",
    window_title: "darkbright-helper Einstellungen",

    capture_prompt: "Tastenkombination drücken… (Esc zum Abbrechen)",
    capture_reject_no_modifier: "Strg, Alt oder Win hinzufügen (Umschalt allein reicht nicht)",
    capture_reject_unnameable_key: "Diese Taste kann nicht als Hotkey verwendet werden",
    capture_reject_duplicate: "Bereits dem anderen Helligkeits-Hotkey zugewiesen",

    hotkey_status_unreachable: "Hotkey-Thread nicht erreichbar",
    hotkey_status_no_response: "Hotkey-Thread hat nicht geantwortet",
    hotkey_status_unknown_error: "unbekannter Fehler",
    hotkey_status_restore_also_failed_fmt: "{error}; Wiederherstellen ebenfalls fehlgeschlagen: {restore_error}",
    hotkey_notice_interception_unavailable: "Abfangen der Helligkeitstasten nicht verfügbar; einfache Tastenregistrierung wird verwendet",

    msgbox_already_running: "darkbright-helper läuft bereits.",
    msgbox_title_startup_error: "Startfehler",
    msgbox_title_hotkey_error: "Hotkey-Fehler",
    msgbox_title_autostart: "Autostart",
    msgbox_title_restore_defaults: "Standardwerte wiederherstellen",
    msgbox_startup_failed_lead: "darkbright-helper konnte nicht gestartet werden:",
    msgbox_thread_spawn_advice: "Das System hat keinen Thread gestartet, was meist bedeutet, dass die Ressourcen knapp sind. Einige Anwendungen schließen oder den Computer neu starten und es erneut versuchen.",
    msgbox_hotkey_failed_lead: "Hotkeys konnten nicht registriert werden:",
    msgbox_hotkey_advice_fmt: "Mögliche Lösungen:\n• Andere Anwendungen schließen, die diese Hotkeys verwenden könnten\n• Die Hotkey-Konfiguration ändern in:\n  {path}\n• Die Anwendung nach der Änderung neu starten",
    msgbox_autostart_failed_fmt: "Der Windows-Autostarteintrag konnte nicht aktualisiert werden:\n{error}",
    msgbox_restore_defaults_question: "Alle Einstellungen auf die Standardwerte zurücksetzen? Hotkeys werden sofort übernommen.",
    msgbox_config_file_fallback: "Konfigurationsdatei",
};

/// The string table for `lang`.
#[must_use]
pub fn strings(lang: Lang) -> &'static Strings {
    match lang {
        Lang::English => &ENGLISH,
        Lang::German => &GERMAN,
    }
}

#[cfg(test)]
mod tests {
    use super::{ENGLISH, Lang, Strings, TextKey, strings};

    #[test]
    fn every_language_tag_is_unique_and_lowercase() {
        let mut seen = Vec::new();
        for lang in Lang::ALL {
            let tag = lang.tag();
            assert!(!tag.is_empty(), "{lang:?} has an empty tag");
            assert_eq!(tag, tag.to_lowercase(), "{lang:?} tag is not lowercase");
            assert!(!seen.contains(&tag), "duplicate tag {tag}");
            seen.push(tag);
        }
    }

    /// Every field of the table, paired with its name.
    ///
    /// The destructuring binding carries no rest pattern, so a field added to
    /// [`Strings`] without an entry here is a compile error (`E0027`) rather
    /// than something a reviewer has to catch. The pattern is packed several
    /// names to a line, and the function is exempt from rustfmt, because one
    /// name per line puts it past the line-count lint.
    #[rustfmt::skip]
    fn every_field(s: &Strings) -> impl IntoIterator<Item = (&'static str, &'static str)> {
        let Strings {
            osd_ddc_error, tray_tip_ddc_unavailable, tray_tip_monitor_unresponsive,
            tray_tip_hotkeys_stopped, tray_tip_hotkey_change_failed, tray_tip_file_logging_off,
            tray_warn_ddc_unavailable, tray_warn_monitor_unresponsive, tray_warn_hotkeys_stopped,
            tray_warn_hotkey_change_failed, tray_warn_file_logging_failed, tray_usage_heading,
            tray_usage_brighter, tray_usage_dimmer, tray_menu_settings, tray_menu_open_log_folder,
            tray_menu_quit_fmt, key_mod_ctrl, key_mod_alt, key_mod_shift, key_mod_win,
            key_separator, key_up, key_down, key_left, key_right, key_page_up, key_page_down,
            key_home, key_end, key_insert, key_delete, key_space, key_tab, key_enter, key_escape,
            key_backspace, header_general, label_language, language_system_default, autostart,
            label_step, unit_percent_step, header_hotkeys,
            label_hotkey_up, label_hotkey_down, intercept, hint_intercept, header_osd,
            label_timeout, unit_milliseconds, label_opacity, unit_percent_opacity, header_advanced,
            resync_check, unit_seconds_resync, inactivity_check, unit_seconds_inactivity, log_check,
            label_log_level, log_level_error, log_level_warn, log_level_info, log_level_debug,
            log_level_trace, hint_logging, footer_links, button_restore_defaults, button_close,
            window_title, capture_prompt, capture_reject_no_modifier, capture_reject_unnameable_key,
            capture_reject_duplicate, hotkey_status_unreachable, hotkey_status_no_response,
            hotkey_status_unknown_error, hotkey_status_restore_also_failed_fmt,
            hotkey_notice_interception_unavailable, msgbox_already_running,
            msgbox_title_startup_error, msgbox_title_hotkey_error, msgbox_title_autostart,
            msgbox_title_restore_defaults, msgbox_startup_failed_lead, msgbox_thread_spawn_advice,
            msgbox_hotkey_failed_lead, msgbox_hotkey_advice_fmt, msgbox_autostart_failed_fmt,
            msgbox_restore_defaults_question, msgbox_config_file_fallback,
        } = *s;
        [
            ("osd_ddc_error", osd_ddc_error),
            ("tray_tip_ddc_unavailable", tray_tip_ddc_unavailable),
            ("tray_tip_monitor_unresponsive", tray_tip_monitor_unresponsive),
            ("tray_tip_hotkeys_stopped", tray_tip_hotkeys_stopped),
            ("tray_tip_hotkey_change_failed", tray_tip_hotkey_change_failed),
            ("tray_tip_file_logging_off", tray_tip_file_logging_off),
            ("tray_warn_ddc_unavailable", tray_warn_ddc_unavailable),
            ("tray_warn_monitor_unresponsive", tray_warn_monitor_unresponsive),
            ("tray_warn_hotkeys_stopped", tray_warn_hotkeys_stopped),
            ("tray_warn_hotkey_change_failed", tray_warn_hotkey_change_failed),
            ("tray_warn_file_logging_failed", tray_warn_file_logging_failed),
            ("tray_usage_heading", tray_usage_heading),
            ("tray_usage_brighter", tray_usage_brighter), ("tray_usage_dimmer", tray_usage_dimmer),
            ("tray_menu_settings", tray_menu_settings),
            ("tray_menu_open_log_folder", tray_menu_open_log_folder),
            ("tray_menu_quit_fmt", tray_menu_quit_fmt), ("key_mod_ctrl", key_mod_ctrl),
            ("key_mod_alt", key_mod_alt), ("key_mod_shift", key_mod_shift),
            ("key_mod_win", key_mod_win), ("key_separator", key_separator),
            ("key_up", key_up), ("key_down", key_down), ("key_left", key_left),
            ("key_right", key_right), ("key_page_up", key_page_up), ("key_page_down", key_page_down),
            ("key_home", key_home), ("key_end", key_end), ("key_insert", key_insert),
            ("key_delete", key_delete), ("key_space", key_space), ("key_tab", key_tab),
            ("key_enter", key_enter), ("key_escape", key_escape), ("key_backspace", key_backspace),
            ("header_general", header_general), ("label_language", label_language),
            ("language_system_default", language_system_default),
            ("autostart", autostart),
            ("label_step", label_step), ("unit_percent_step", unit_percent_step),
            ("header_hotkeys", header_hotkeys), ("label_hotkey_up", label_hotkey_up),
            ("label_hotkey_down", label_hotkey_down), ("intercept", intercept),
            ("hint_intercept", hint_intercept), ("header_osd", header_osd),
            ("label_timeout", label_timeout), ("unit_milliseconds", unit_milliseconds),
            ("label_opacity", label_opacity), ("unit_percent_opacity", unit_percent_opacity),
            ("header_advanced", header_advanced), ("resync_check", resync_check),
            ("unit_seconds_resync", unit_seconds_resync), ("inactivity_check", inactivity_check),
            ("unit_seconds_inactivity", unit_seconds_inactivity), ("log_check", log_check),
            ("label_log_level", label_log_level), ("log_level_error", log_level_error),
            ("log_level_warn", log_level_warn), ("log_level_info", log_level_info),
            ("log_level_debug", log_level_debug), ("log_level_trace", log_level_trace),
            ("hint_logging", hint_logging), ("footer_links", footer_links),
            ("button_restore_defaults", button_restore_defaults), ("button_close", button_close),
            ("window_title", window_title), ("capture_prompt", capture_prompt),
            ("capture_reject_no_modifier", capture_reject_no_modifier),
            ("capture_reject_unnameable_key", capture_reject_unnameable_key),
            ("capture_reject_duplicate", capture_reject_duplicate),
            ("hotkey_status_unreachable", hotkey_status_unreachable),
            ("hotkey_status_no_response", hotkey_status_no_response),
            ("hotkey_status_unknown_error", hotkey_status_unknown_error),
            ("hotkey_status_restore_also_failed_fmt", hotkey_status_restore_also_failed_fmt),
            ("hotkey_notice_interception_unavailable", hotkey_notice_interception_unavailable),
            ("msgbox_already_running", msgbox_already_running),
            ("msgbox_title_startup_error", msgbox_title_startup_error),
            ("msgbox_title_hotkey_error", msgbox_title_hotkey_error),
            ("msgbox_title_autostart", msgbox_title_autostart),
            ("msgbox_title_restore_defaults", msgbox_title_restore_defaults),
            ("msgbox_startup_failed_lead", msgbox_startup_failed_lead),
            ("msgbox_thread_spawn_advice", msgbox_thread_spawn_advice),
            ("msgbox_hotkey_failed_lead", msgbox_hotkey_failed_lead),
            ("msgbox_hotkey_advice_fmt", msgbox_hotkey_advice_fmt),
            ("msgbox_autostart_failed_fmt", msgbox_autostart_failed_fmt),
            ("msgbox_restore_defaults_question", msgbox_restore_defaults_question),
            ("msgbox_config_file_fallback", msgbox_config_file_fallback),
        ]
    }

    #[test]
    fn no_field_in_any_language_is_empty() {
        for &lang in Lang::ALL {
            for (name, value) in every_field(strings(lang)) {
                assert!(!value.is_empty(), "{lang:?} has an empty {name}");
            }
        }
    }

    #[test]
    fn english_is_the_table_returned_for_english() {
        assert_eq!(strings(Lang::English).osd_ddc_error, ENGLISH.osd_ddc_error);
    }

    #[test]
    fn the_quit_command_keeps_its_product_name_placeholder() {
        for &lang in Lang::ALL {
            assert!(
                strings(lang).tray_menu_quit_fmt.contains("{name}"),
                "{lang:?} dropped the {{name}} placeholder from the quit command"
            );
        }
    }

    #[test]
    fn every_key_resolves_to_a_non_empty_string_in_every_language() {
        const KEYS: &[TextKey] = &[
            TextKey::HeaderGeneral,
            TextKey::LabelLanguage,
            TextKey::Autostart,
            TextKey::LabelStep,
            TextKey::UnitPercentStep,
            TextKey::HeaderHotkeys,
            TextKey::LabelHotkeyUp,
            TextKey::LabelHotkeyDown,
            TextKey::Intercept,
            TextKey::HintIntercept,
            TextKey::HeaderOsd,
            TextKey::LabelTimeout,
            TextKey::UnitMilliseconds,
            TextKey::LabelOpacity,
            TextKey::UnitPercentOpacity,
            TextKey::HeaderAdvanced,
            TextKey::ResyncCheck,
            TextKey::UnitSecondsResync,
            TextKey::InactivityCheck,
            TextKey::UnitSecondsInactivity,
            TextKey::LogCheck,
            TextKey::LabelLogLevel,
            TextKey::HintLogging,
            TextKey::FooterLinks,
            TextKey::ButtonRestoreDefaults,
            TextKey::ButtonClose,
        ];
        for &lang in Lang::ALL {
            let s = strings(lang);
            for &key in KEYS {
                assert!(!s.get(key).is_empty(), "{lang:?} has no text for {key:?}");
            }
        }
    }

    #[test]
    fn english_key_names_equal_the_wire_names() {
        // In English the display name of every named key is its wire name,
        // which is what keeps `english_display_text_matches_the_stored_format`
        // in hotkey.rs true after key names are routed through this table.
        let s = strings(Lang::English);
        assert_eq!(s.key_up, "Up");
        assert_eq!(s.key_down, "Down");
        assert_eq!(s.key_left, "Left");
        assert_eq!(s.key_right, "Right");
        assert_eq!(s.key_page_up, "PageUp");
        assert_eq!(s.key_page_down, "PageDown");
        assert_eq!(s.key_home, "Home");
        assert_eq!(s.key_end, "End");
        assert_eq!(s.key_insert, "Insert");
        assert_eq!(s.key_delete, "Delete");
        assert_eq!(s.key_space, "Space");
        assert_eq!(s.key_tab, "Tab");
        assert_eq!(s.key_enter, "Enter");
        assert_eq!(s.key_escape, "Escape");
        assert_eq!(s.key_backspace, "Backspace");
        assert_eq!(s.label_language, "Language");
        assert_eq!(s.language_system_default, "System default");
    }

    #[test]
    fn the_footer_link_row_keeps_both_link_spans() {
        for &lang in Lang::ALL {
            let text = strings(lang).get(TextKey::FooterLinks);
            assert_eq!(
                text.matches("<a>").count(),
                2,
                "{lang:?} footer must keep exactly two <a> spans, or SysLink indices shift"
            );
            assert_eq!(text.matches("</a>").count(), 2);
        }
    }

    #[test]
    fn message_box_formats_keep_their_placeholders() {
        for &lang in Lang::ALL {
            let s = strings(lang);
            assert!(s.msgbox_hotkey_advice_fmt.contains("{path}"), "{lang:?}");
            assert!(
                s.msgbox_autostart_failed_fmt.contains("{error}"),
                "{lang:?}"
            );
        }
    }

    #[test]
    fn the_rebind_restore_failure_keeps_both_error_placeholders() {
        for &lang in Lang::ALL {
            let text = strings(lang).hotkey_status_restore_also_failed_fmt;
            assert!(text.contains("{error}"), "{lang:?}");
            assert!(text.contains("{restore_error}"), "{lang:?}");
        }
    }

    #[test]
    fn german_exists_with_its_tag_and_native_name() {
        assert_eq!(Lang::German.tag(), "de");
        assert_eq!(Lang::German.native_name(), "Deutsch");
        assert_eq!(Lang::English.native_name(), "English");
        assert_eq!(Lang::ALL, &[Lang::English, Lang::German]);
        assert_eq!(strings(Lang::German).button_close, "Schließen");
    }

    #[test]
    fn native_names_are_non_empty_and_unique() {
        let mut seen = Vec::new();
        for lang in Lang::ALL {
            let name = lang.native_name();
            assert!(!name.is_empty(), "{lang:?} has an empty native name");
            assert!(!seen.contains(&name), "duplicate native name {name}");
            seen.push(name);
        }
    }

    #[test]
    fn index_round_trips_through_all() {
        for (i, &lang) in Lang::ALL.iter().enumerate() {
            assert_eq!(lang.index(), i);
            assert_eq!(Lang::from_index(i), Some(lang));
        }
        assert_eq!(Lang::from_index(Lang::ALL.len()), None);
    }

    #[test]
    fn every_format_field_keeps_the_english_placeholders() {
        // Each `_fmt` field and the placeholders it must carry. A translation
        // that drops or misspells one would leave `{path}` literal on screen.
        type FormatCase = (
            &'static str,
            fn(&Strings) -> &'static str,
            &'static [&'static str],
        );
        let formats: &[FormatCase] = &[
            ("tray_menu_quit_fmt", |s| s.tray_menu_quit_fmt, &["{name}"]),
            (
                "hotkey_status_restore_also_failed_fmt",
                |s| s.hotkey_status_restore_also_failed_fmt,
                &["{error}", "{restore_error}"],
            ),
            (
                "msgbox_hotkey_advice_fmt",
                |s| s.msgbox_hotkey_advice_fmt,
                &["{path}"],
            ),
            (
                "msgbox_autostart_failed_fmt",
                |s| s.msgbox_autostart_failed_fmt,
                &["{error}"],
            ),
        ];
        for &lang in Lang::ALL {
            let s = strings(lang);
            for (name, get, placeholders) in formats {
                for placeholder in *placeholders {
                    assert!(
                        get(s).contains(placeholder),
                        "{lang:?}: {name} lost {placeholder}"
                    );
                }
            }
        }
    }

    #[test]
    fn log_level_entries_lead_with_the_stored_token_in_every_language() {
        for &lang in Lang::ALL {
            let s = strings(lang);
            for (token, shown) in [
                ("error", s.log_level_error),
                ("warn", s.log_level_warn),
                ("info", s.log_level_info),
                ("debug", s.log_level_debug),
                ("trace", s.log_level_trace),
            ] {
                assert!(
                    shown.starts_with(token),
                    "{lang:?}: log level entry {shown:?} must start with the stored token {token}"
                );
            }
        }
    }
}
