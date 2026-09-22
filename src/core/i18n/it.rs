//! The Italian string table.
//!
//! Variant: Standard Italian (Italy), `it-IT` conventions, second-person singular address.
//! Style reference: Microsoft Italian style guide (`ITA-ITA-STYLEGUIDE.PDF`), listed at
//! <https://learn.microsoft.com/globalization/reference/microsoft-style-guides>.
//!
//! Glossary (English → Italian):
//! - brightness → luminosità
//! - hotkey → scelta rapida
//! - on-screen display → indicatore su schermo
//! - monitor → monitor
//! - log → registro
//! - log file → file di registro
//! - log folder → cartella dei registri
//! - settings → impostazioni
//! - resync → risincronizzare
//! - inactivity → inattività
//! - restore defaults → ripristina valori predefiniti
//! - intercept (keys) → intercettare
//! - start with Windows → avvia con Windows
//! - config file → file di configurazione
//! - hotkey thread → thread delle scelte rapide
//! - DDC → DDC
//!
//! Key names: the style guide's key table and Microsoft's Italian keyboard-shortcut pages
//! (Maiusc, Canc, Ins, Fine, Invio, Esc, Pag su, Pag giù, Freccia su); Ctrl, Alt, Win, Tab,
//! Home and Backspace keep their English names there too. Sentence case rather than the
//! guide's all caps, so a binding reads like the rest of the window.
//! Typography: curly apostrophe (’), no space before punctuation, hyphen where English uses
//! a dash — except the tray warning rows, whose shared "state — action" shape sets that dash.

use super::Strings;

/// The Italian strings.
pub(crate) const ITALIAN: Strings = Strings {
    osd_ddc_error: "Errore DDC - Regolazione non riuscita",

    tray_tip_ddc_unavailable: "DDC non disponibile",
    tray_tip_monitor_unresponsive: "monitor senza risposta",
    tray_tip_hotkeys_stopped: "scelte rapide arrestate",
    tray_tip_hotkey_change_failed: "scelta rapida non applicata",
    tray_tip_file_logging_off: "nessun file di registro",

    tray_warn_ddc_unavailable: "⚠ DDC non disponibile — premi una scelta rapida di luminosità per riprovare",
    tray_warn_monitor_unresponsive: "⚠ Il monitor non risponde — riavvia l’app se il problema persiste",
    tray_warn_hotkeys_stopped: "⚠ Le scelte rapide non funzionano più — riavvia l’app",
    tray_warn_hotkey_change_failed: "⚠ Modifica della scelta rapida non riuscita — prova un’altra combinazione",
    tray_warn_file_logging_failed: "⚠ Non è possibile aprire il file di registro — verifica che la cartella dei registri sia scrivibile",

    tray_usage_heading: "Punta il mouse su un monitor, quindi:",
    tray_usage_brighter: "Aumenta luminosità",
    tray_usage_dimmer: "Riduci luminosità",
    tray_menu_settings: "Impostazioni",
    tray_menu_open_log_folder: "Apri cartella dei registri",
    tray_menu_quit_fmt: "Esci da {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Maiusc",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Freccia su",
    key_down: "Freccia giù",
    key_left: "Freccia sinistra",
    key_right: "Freccia destra",
    key_page_up: "Pag su",
    key_page_down: "Pag giù",
    key_home: "Home",
    key_end: "Fine",
    key_insert: "Ins",
    key_delete: "Canc",
    key_space: "Barra spaziatrice",
    key_tab: "Tab",
    key_enter: "Invio",
    key_escape: "Esc",
    key_backspace: "Backspace",

    header_general: "Generale",
    label_language: "Lingua",
    language_system_default: "Lingua di sistema",
    autostart: "Avvia con Windows",
    label_step: "Variazione luminosità per pressione",
    unit_percent_step: "%",
    header_hotkeys: "Scelte rapide",
    label_hotkey_up: "Aumenta luminosità",
    label_hotkey_down: "Riduci luminosità",
    intercept: "Prova a intercettare i tasti di luminosità",
    hint_intercept: "(potrebbe non funzionare con tutte le tastiere; alcuni antivirus segnalano gli hook di basso livello)",
    header_osd: "Indicatore su schermo",
    label_timeout: "Durata di visualizzazione",
    unit_milliseconds: "ms",
    label_opacity: "Opacità",
    unit_percent_opacity: "%",
    header_advanced: "Avanzate",
    resync_check: "Risincronizza la luminosità ogni",
    unit_seconds_resync: "s",
    inactivity_check: "Risincronizza dopo inattività di",
    unit_seconds_inactivity: "s",
    log_check: "Scrivi il file di registro",
    label_log_level: "Livello:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(le modifiche alla registrazione hanno effetto dopo il riavvio; debug e trace registrano numeri di serie dei monitor e percorsi)",
    footer_links: "<a>File di configurazione</a> \u{b7} <a>Cartella dei registri</a>",
    button_restore_defaults: "Ripristina valori predefiniti",
    button_close: "Chiudi",
    window_title: "Impostazioni di darkbright-helper",

    capture_prompt: "Premi i tasti… (Esc per annullare)",
    capture_reject_no_modifier: "Aggiungi Ctrl, Alt o Win (Maiusc da solo non basta)",
    capture_reject_unnameable_key: "Tasto non utilizzabile come scelta rapida",
    capture_reject_duplicate: "Combinazione già assegnata all’altra scelta rapida",

    hotkey_status_unreachable: "Thread delle scelte rapide non raggiungibile",
    hotkey_status_no_response: "Il thread delle scelte rapide non ha risposto",
    hotkey_status_unknown_error: "errore sconosciuto",
    hotkey_status_restore_also_failed_fmt: "{error}; anche il ripristino non è riuscito: {restore_error}",
    hotkey_notice_interception_unavailable: "Intercettazione non disponibile; scelte rapide attive",

    msgbox_already_running: "darkbright-helper è già in esecuzione.",
    msgbox_title_startup_error: "Errore di avvio",
    msgbox_title_hotkey_error: "Errore di scelta rapida",
    msgbox_title_autostart: "Avvio automatico",
    msgbox_title_restore_defaults: "Ripristina valori predefiniti",
    msgbox_startup_failed_lead: "Non è possibile avviare darkbright-helper:",
    msgbox_thread_spawn_advice: "Il sistema non ha avviato un thread, il che in genere indica una mancanza di risorse. Chiudi alcune applicazioni oppure riavvia il computer, quindi riprova.",
    msgbox_hotkey_failed_lead: "Non è possibile registrare le scelte rapide:",
    msgbox_hotkey_advice_fmt: "Possibili soluzioni:\n• Chiudi le altre applicazioni che potrebbero usare queste scelte rapide\n• Modifica la configurazione delle scelte rapide in:\n  {path}\n• Riavvia l’applicazione dopo le modifiche",
    msgbox_autostart_failed_fmt: "Non è possibile aggiornare la voce di avvio di Windows:\n{error}",
    msgbox_restore_defaults_question: "Vuoi ripristinare i valori predefiniti di tutte le impostazioni? Le scelte rapide vengono applicate subito.",
    msgbox_config_file_fallback: "file di configurazione",
};
