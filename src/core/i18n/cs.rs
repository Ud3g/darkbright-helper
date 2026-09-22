//! The Czech string table.
//!
//! Variant: Standard Czech (`cs-CZ`), neutral second-person plural address (vy).
//! Style reference: Microsoft style guide at <https://aka.ms/czech-styleguide>.
//!
//! Glossary (English → Czech):
//! - brightness → jas
//! - hotkey → klávesová zkratka (bare *zkratka* only where a length budget forces it:
//!   tray tooltip, status line)
//! - on-screen display → zobrazení na obrazovce
//! - monitor → monitor
//! - log → protokol
//! - log file → soubor protokolu
//! - log folder → složka protokolů
//! - settings → nastavení
//! - resync → znovu synchronizovat
//! - inactivity → nečinnost
//! - restore defaults → obnovit výchozí nastavení
//! - intercept (keys) → zachytávat
//! - start with Windows → spouštět s Windows
//! - config file → konfigurační soubor
//! - hotkey thread → vlákno klávesových zkratek
//! - DDC → DDC
//!
//! Key names: the style guide's shortcut table (Esc, Alt+Enter, Alt+Mezerník, Ctrl+Backspace);
//! arrows are named in Czech (Šipka nahoru), every other key keeps the English name that Czech
//! key caps print. Commands are infinitives, the Czech Windows menu convention.
//! Typography: `\u{a0}` after one-letter prepositions and conjunctions, U+2026 ellipsis; Czech
//! would set the `tray_warn_*` dash as an en dash, the shared em dash is kept for the row shape.

use super::Strings;

/// The Czech strings.
pub(crate) const CZECH: Strings = Strings {
    osd_ddc_error: "Chyba DDC: jas se nezměnil",

    tray_tip_ddc_unavailable: "DDC není k\u{a0}dispozici",
    tray_tip_monitor_unresponsive: "monitor neodpovídá",
    tray_tip_hotkeys_stopped: "klávesové zkratky nefungují",
    tray_tip_hotkey_change_failed: "změna zkratky se nepovedla",
    tray_tip_file_logging_off: "zápis protokolu vypnutý",

    tray_warn_ddc_unavailable: "⚠ DDC není k\u{a0}dispozici — zkuste to znovu stisknutím klávesové zkratky jasu",
    tray_warn_monitor_unresponsive: "⚠ Monitor neodpovídá — pokud potíže trvají, restartujte aplikaci",
    tray_warn_hotkeys_stopped: "⚠ Klávesové zkratky přestaly fungovat — restartujte aplikaci",
    tray_warn_hotkey_change_failed: "⚠ Změna klávesové zkratky se nepovedla — zkuste jinou kombinaci",
    tray_warn_file_logging_failed: "⚠ Zápis protokolu se nepovedlo spustit — ověřte, že do složky protokolů jde zapisovat",

    tray_usage_heading: "Najeďte myší na monitor, potom:",
    tray_usage_brighter: "Zvýšit jas",
    tray_usage_dimmer: "Snížit jas",
    tray_menu_settings: "Nastavení",
    tray_menu_open_log_folder: "Otevřít složku protokolů",
    tray_menu_quit_fmt: "Ukončit {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Shift",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Šipka nahoru",
    key_down: "Šipka dolů",
    key_left: "Šipka doleva",
    key_right: "Šipka doprava",
    key_page_up: "Page Up",
    key_page_down: "Page Down",
    key_home: "Home",
    key_end: "End",
    key_insert: "Insert",
    key_delete: "Delete",
    key_space: "Mezerník",
    key_tab: "Tab",
    key_enter: "Enter",
    key_escape: "Esc",
    key_backspace: "Backspace",

    header_general: "Obecné",
    label_language: "Jazyk",
    language_system_default: "Jazyk systému",
    autostart: "Spouštět s\u{a0}Windows",
    label_step: "Krok jasu na stisk klávesy",
    unit_percent_step: "%",
    header_hotkeys: "Klávesové zkratky",
    label_hotkey_up: "Zvýšit jas",
    label_hotkey_down: "Snížit jas",
    intercept: "Zkusit zachytávat vyhrazené klávesy jasu",
    hint_intercept: "(nemusí fungovat se všemi klávesnicemi; některé antivirové programy označují nízkoúrovňové háky za podezřelé)",
    header_osd: "Zobrazení na obrazovce",
    label_timeout: "Doba zobrazení",
    unit_milliseconds: "ms",
    label_opacity: "Neprůhlednost",
    unit_percent_opacity: "%",
    header_advanced: "Upřesnit",
    resync_check: "Znovu synchronizovat jas v\u{a0}intervalu",
    unit_seconds_resync: "s",
    inactivity_check: "Znovu synchronizovat po nečinnosti",
    unit_seconds_inactivity: "s",
    log_check: "Zapisovat soubor protokolu",
    label_log_level: "Úroveň:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(změny protokolování se projeví po restartu; úrovně debug a\u{a0}trace zapisují sériová čísla monitorů a\u{a0}cesty)",
    footer_links: "<a>Otevřít konfigurační soubor</a> \u{b7} <a>Otevřít složku protokolů</a>",
    button_restore_defaults: "Obnovit výchozí nastavení",
    button_close: "Zavřít",
    window_title: "Nastavení aplikace darkbright-helper",

    capture_prompt: "Stiskněte kombinaci kláves… (Esc zruší)",
    capture_reject_no_modifier: "Přidejte Ctrl, Alt nebo Win (Shift sám nestačí)",
    capture_reject_unnameable_key: "Tuto klávesu nejde použít v\u{a0}klávesové zkratce",
    capture_reject_duplicate: "Tuto kombinaci už používá druhá zkratka jasu",

    hotkey_status_unreachable: "Nejde se spojit s\u{a0}vláknem klávesových zkratek",
    hotkey_status_no_response: "Vlákno klávesových zkratek neodpovědělo",
    hotkey_status_unknown_error: "neznámá chyba",
    hotkey_status_restore_also_failed_fmt: "{error}; obnovit předchozí zkratky se také nepovedlo: {restore_error}",
    hotkey_notice_interception_unavailable: "Zachytávání kláves není dostupné; zkratky fungují dál",

    msgbox_already_running: "darkbright-helper už běží.",
    msgbox_title_startup_error: "Chyba při spuštění",
    msgbox_title_hotkey_error: "Chyba klávesové zkratky",
    msgbox_title_autostart: "Spouštění s\u{a0}Windows",
    msgbox_title_restore_defaults: "Obnovení výchozího nastavení",
    msgbox_startup_failed_lead: "darkbright-helper se nepovedlo spustit:",
    msgbox_thread_spawn_advice: "Systému se nepovedlo spustit vlákno. Obvykle to znamená, že docházejí systémové prostředky. Zavřete některé aplikace nebo restartujte počítač a\u{a0}zkuste to znovu.",
    msgbox_hotkey_failed_lead: "Nepovedlo se zaregistrovat klávesové zkratky:",
    msgbox_hotkey_advice_fmt: "Možná řešení:\n• Zavřete ostatní aplikace, které tyto klávesové zkratky mohou používat\n• Změňte nastavení klávesových zkratek v\u{a0}souboru:\n  {path}\n• Po změnách aplikaci restartujte",
    msgbox_autostart_failed_fmt: "Položku spouštění s\u{a0}Windows se nepovedlo aktualizovat:\n{error}",
    msgbox_restore_defaults_question: "Obnovit výchozí hodnoty všech nastavení? Klávesové zkratky se použijí ihned.",
    msgbox_config_file_fallback: "konfigurační soubor",
};
