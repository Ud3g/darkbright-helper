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
///
/// Only English exists today. The enum is here so that consumers already take
/// a language parameter and adding a second one touches no call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    /// English, the fallback for every unmatched locale.
    #[default]
    English,
}

impl Lang {
    /// Every language the app can display, in the order the picker shows them.
    pub const ALL: &'static [Lang] = &[Lang::English];

    /// The BCP-47 tag identifying this language, for locale matching and for
    /// the value stored in the config file.
    #[must_use]
    pub fn tag(self) -> &'static str {
        match self {
            Lang::English => "en",
        }
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

    // --- Settings window ---
    /// Section header above the general settings.
    pub header_general: &'static str,
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

    // --- Message boxes ---
    //
    // Titles carry the product name, which is not translated; the translation
    // supplies only the part after the dash. `Display` on `BrightnessError`
    // stays English for logs — see `BrightnessError::user_message`.
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
pub enum TextKey {
    /// Section header above the general settings.
    HeaderGeneral,
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
    /// The settings window's title bar text.
    WindowTitle,
}

impl Strings {
    /// The text for `key`.
    ///
    /// Exhaustive by construction: a new [`TextKey`] variant fails to compile
    /// here until it is given a field, and a new field fails to compile in
    /// every language table until it is translated.
    #[must_use]
    pub fn get(&self, key: TextKey) -> &'static str {
        match key {
            TextKey::HeaderGeneral => self.header_general,
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
            TextKey::WindowTitle => self.window_title,
        }
    }
}

/// The English strings. Every other language is a translation of this table.
pub const ENGLISH: Strings = Strings {
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

    header_general: "General",
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

/// The string table for `lang`.
#[must_use]
pub fn strings(lang: Lang) -> &'static Strings {
    match lang {
        Lang::English => &ENGLISH,
    }
}

#[cfg(test)]
mod tests {
    use super::{ENGLISH, Lang, TextKey, strings};

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

    #[test]
    fn every_language_resolves_to_a_table_with_no_empty_fields() {
        for &lang in Lang::ALL {
            let s = strings(lang);
            assert!(
                !s.osd_ddc_error.is_empty(),
                "{lang:?} has an empty osd_ddc_error"
            );
            assert!(
                !s.tray_tip_ddc_unavailable.is_empty(),
                "{lang:?} has an empty tray_tip_ddc_unavailable"
            );
            assert!(
                !s.tray_tip_monitor_unresponsive.is_empty(),
                "{lang:?} has an empty tray_tip_monitor_unresponsive"
            );
            assert!(
                !s.tray_tip_hotkeys_stopped.is_empty(),
                "{lang:?} has an empty tray_tip_hotkeys_stopped"
            );
            assert!(
                !s.tray_tip_hotkey_change_failed.is_empty(),
                "{lang:?} has an empty tray_tip_hotkey_change_failed"
            );
            assert!(
                !s.tray_tip_file_logging_off.is_empty(),
                "{lang:?} has an empty tray_tip_file_logging_off"
            );
            assert!(
                !s.tray_warn_ddc_unavailable.is_empty(),
                "{lang:?} has an empty tray_warn_ddc_unavailable"
            );
            assert!(
                !s.tray_warn_monitor_unresponsive.is_empty(),
                "{lang:?} has an empty tray_warn_monitor_unresponsive"
            );
            assert!(
                !s.tray_warn_hotkeys_stopped.is_empty(),
                "{lang:?} has an empty tray_warn_hotkeys_stopped"
            );
            assert!(
                !s.tray_warn_hotkey_change_failed.is_empty(),
                "{lang:?} has an empty tray_warn_hotkey_change_failed"
            );
            assert!(
                !s.tray_warn_file_logging_failed.is_empty(),
                "{lang:?} has an empty tray_warn_file_logging_failed"
            );
            assert!(
                !s.tray_usage_heading.is_empty(),
                "{lang:?} has an empty tray_usage_heading"
            );
            assert!(
                !s.tray_usage_brighter.is_empty(),
                "{lang:?} has an empty tray_usage_brighter"
            );
            assert!(
                !s.tray_usage_dimmer.is_empty(),
                "{lang:?} has an empty tray_usage_dimmer"
            );
            assert!(
                !s.tray_menu_settings.is_empty(),
                "{lang:?} has an empty tray_menu_settings"
            );
            assert!(
                !s.tray_menu_open_log_folder.is_empty(),
                "{lang:?} has an empty tray_menu_open_log_folder"
            );
            assert!(
                !s.tray_menu_quit_fmt.is_empty(),
                "{lang:?} has an empty tray_menu_quit_fmt"
            );
            assert!(
                !s.key_mod_ctrl.is_empty(),
                "{lang:?} has an empty key_mod_ctrl"
            );
            assert!(
                !s.key_mod_alt.is_empty(),
                "{lang:?} has an empty key_mod_alt"
            );
            assert!(
                !s.key_mod_shift.is_empty(),
                "{lang:?} has an empty key_mod_shift"
            );
            assert!(
                !s.key_mod_win.is_empty(),
                "{lang:?} has an empty key_mod_win"
            );
            assert!(
                !s.key_separator.is_empty(),
                "{lang:?} has an empty key_separator"
            );
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
            TextKey::WindowTitle,
        ];
        for &lang in Lang::ALL {
            let s = strings(lang);
            for &key in KEYS {
                assert!(!s.get(key).is_empty(), "{lang:?} has no text for {key:?}");
            }
        }
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
}
