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
    use super::{ENGLISH, Lang, strings};

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
}
