//! The Dutch string table.
//!
//! Variant: Netherlands Dutch, `nl-NL` conventions, formal "u" address, commands as infinitives;
//! message-box advice in direct imperative sentences (Sluit…, Wijzig…, Start…).
//! Style reference: Microsoft style guide at <https://aka.ms/dutch-styleguide>.
//!
//! Glossary (English → Dutch):
//! - brightness → helderheid
//! - hotkey → sneltoets
//! - on-screen display → schermweergave
//! - monitor → beeldscherm
//! - log → logboek
//! - log file → logboekbestand
//! - log folder → logboekmap
//! - file logging → logboekregistratie
//! - settings → instellingen
//! - resync → synchroniseren
//! - inactivity → inactiviteit
//! - restore defaults → standaardwaarden herstellen
//! - intercept (keys) → onderscheppen
//! - start with Windows → starten met Windows
//! - config file → configuratiebestand
//! - hotkey thread → thread voor sneltoetsen
//! - DDC → DDC
//!
//! Key names: the style guide's key table (Pijl-omhoog, Pijl-omlaag, Spatiebalk, Esc, Page Up);
//! Ctrl, Alt, Shift, Home, End, Insert, Delete, Tab, Enter and Backspace stay English there too.
//! `Win` keeps the key-cap abbreviation; the guide's "Windows-toets" is a running-text noun,
//! too long for a combination label.
//! Typography: compounds with an acronym or a brand name take a hyphen (DDC-fout,
//! Windows-opstartitem); the guide replaces en/em dashes, but the tray rows keep the table-wide
//! "state — action" shape; no apostrophes are needed, so none appear.

use super::Strings;

/// The Dutch strings.
pub(crate) const DUTCH: Strings = Strings {
    osd_ddc_error: "DDC-fout - Aanpassing mislukt",

    tray_tip_ddc_unavailable: "DDC niet beschikbaar",
    tray_tip_monitor_unresponsive: "beeldscherm reageert niet",
    tray_tip_hotkeys_stopped: "sneltoetsen gestopt",
    tray_tip_hotkey_change_failed: "sneltoets niet gewijzigd",
    tray_tip_file_logging_off: "logboekbestand uit",

    tray_warn_ddc_unavailable: "⚠ DDC niet beschikbaar — druk op een helderheidssneltoets om het opnieuw te proberen",
    tray_warn_monitor_unresponsive: "⚠ Beeldscherm reageert niet — start de app opnieuw als dit aanhoudt",
    tray_warn_hotkeys_stopped: "⚠ Sneltoetsen werken niet meer — start de app opnieuw",
    tray_warn_hotkey_change_failed: "⚠ Het wijzigen van de sneltoets is mislukt — probeer een andere combinatie",
    tray_warn_file_logging_failed: "⚠ Logboekregistratie kan niet worden gestart — controleer of de logboekmap beschrijfbaar is",

    tray_usage_heading: "Plaats de muisaanwijzer op een beeldscherm, vervolgens:",
    tray_usage_brighter: "Helderheid verhogen",
    tray_usage_dimmer: "Helderheid verlagen",
    tray_menu_settings: "Instellingen",
    tray_menu_open_log_folder: "Logboekmap openen",
    tray_menu_quit_fmt: "{name} afsluiten",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Shift",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Pijl-omhoog",
    key_down: "Pijl-omlaag",
    key_left: "Pijl-links",
    key_right: "Pijl-rechts",
    key_page_up: "Page Up",
    key_page_down: "Page Down",
    key_home: "Home",
    key_end: "End",
    key_insert: "Insert",
    key_delete: "Delete",
    key_space: "Spatiebalk",
    key_tab: "Tab",
    key_enter: "Enter",
    key_escape: "Esc",
    key_backspace: "Backspace",

    header_general: "Algemeen",
    label_language: "Taal",
    language_system_default: "Systeemstandaard",
    autostart: "Starten met Windows",
    label_step: "Helderheidsstap per toetsaanslag",
    unit_percent_step: "%",
    header_hotkeys: "Sneltoetsen",
    label_hotkey_up: "Helderheid verhogen",
    label_hotkey_down: "Helderheid verlagen",
    intercept: "Helderheidstoetsen proberen te onderscheppen",
    hint_intercept: "(werkt mogelijk niet met alle toetsenborden; bepaalde antivirussoftware meldt hooks op laag niveau)",
    header_osd: "Schermweergave",
    label_timeout: "Weergaveduur",
    unit_milliseconds: "ms",
    label_opacity: "Dekking",
    unit_percent_opacity: "%",
    header_advanced: "Geavanceerd",
    resync_check: "Helderheid synchroniseren elke",
    unit_seconds_resync: "s",
    inactivity_check: "Synchroniseren na inactiviteit van",
    unit_seconds_inactivity: "s",
    log_check: "Logboekbestand schrijven",
    label_log_level: "Niveau:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(logboekinstellingen gelden pas na opnieuw opstarten; debug en trace registreren serienummers van beeldschermen en paden)",
    footer_links: "<a>Configuratiebestand openen</a> \u{b7} <a>Logboekmap openen</a>",
    button_restore_defaults: "Standaardwaarden herstellen",
    button_close: "Sluiten",
    window_title: "Instellingen van darkbright-helper",

    capture_prompt: "Druk op een sneltoets… (Esc om te annuleren)",
    capture_reject_no_modifier: "Voeg Ctrl, Alt of Win toe (Shift alleen volstaat niet)",
    capture_reject_unnameable_key: "Deze toets kan niet als sneltoets worden gebruikt",
    capture_reject_duplicate: "Al toegewezen aan de andere sneltoets",

    hotkey_status_unreachable: "De thread voor sneltoetsen is niet bereikbaar",
    hotkey_status_no_response: "De thread voor sneltoetsen heeft niet gereageerd",
    hotkey_status_unknown_error: "onbekende fout",
    hotkey_status_restore_also_failed_fmt: "{error}; herstellen is ook mislukt: {restore_error}",
    hotkey_notice_interception_unavailable: "Onderscheppen niet mogelijk; sneltoetsen werken wel",

    msgbox_already_running: "darkbright-helper wordt al uitgevoerd.",
    msgbox_title_startup_error: "Opstartfout",
    msgbox_title_hotkey_error: "Sneltoetsfout",
    msgbox_title_autostart: "Automatisch starten",
    msgbox_title_restore_defaults: "Standaardwaarden herstellen",
    msgbox_startup_failed_lead: "darkbright-helper kan niet worden gestart:",
    msgbox_thread_spawn_advice: "Het systeem kon geen thread starten, wat meestal betekent dat er onvoldoende systeembronnen beschikbaar zijn. Sluit enkele toepassingen af of start de computer opnieuw op en probeer het opnieuw.",
    msgbox_hotkey_failed_lead: "De sneltoetsen kunnen niet worden geregistreerd:",
    msgbox_hotkey_advice_fmt: "Mogelijke oplossingen:
• Sluit andere toepassingen af die deze sneltoetsen mogelijk gebruiken
• Wijzig de sneltoetsinstellingen in:
  {path}
• Start de toepassing opnieuw na het wijzigen",
    msgbox_autostart_failed_fmt: "Het Windows-opstartitem kan niet worden bijgewerkt:\n{error}",
    msgbox_restore_defaults_question: "Wilt u voor alle instellingen de standaardwaarden herstellen? Sneltoetsen worden meteen toegepast.",
    msgbox_config_file_fallback: "configuratiebestand",
};
