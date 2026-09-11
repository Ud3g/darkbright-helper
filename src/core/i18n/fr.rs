//! The French string table.
//!
//! Variant: Standard French (France), `fr-FR` conventions, formal "vous" address.
//! Style reference: Microsoft style guide at <https://aka.ms/french-france-styleguide>.
//!
//! Glossary (English → French):
//! - brightness → luminosité
//! - hotkey → raccourci
//! - on-screen display → affichage à l’écran
//! - monitor → écran
//! - log → journal
//! - log file → fichier journal
//! - log folder → dossier des journaux
//! - settings → paramètres
//! - resync → resynchroniser
//! - inactivity → inactivité
//! - restore defaults → rétablir les valeurs par défaut
//! - intercept (keys) → intercepter
//! - start with Windows → démarrer avec Windows
//! - config file → fichier de configuration
//! - hotkey thread → thread des raccourcis
//! - DDC → DDC
//!
//! Key names: the style guide's key table and French shortcut column (Maj, Échap, Suppr,
//! Pg préc, Origine); Ctrl and Win as French accelerator labels and key caps print them.
//! Typography: `\u{a0}` before `:`, `\u{202f}` before `;` `!` `?`, curly apostrophe (’).

use super::Strings;

/// The French strings.
pub(crate) const FRENCH: Strings = Strings {
    osd_ddc_error: "Erreur DDC - Échec du réglage",

    tray_tip_ddc_unavailable: "DDC non disponible",
    tray_tip_monitor_unresponsive: "écran sans réponse",
    tray_tip_hotkeys_stopped: "raccourcis arrêtés",
    tray_tip_hotkey_change_failed: "raccourci non appliqué",
    tray_tip_file_logging_off: "fichier journal inactif",

    tray_warn_ddc_unavailable: "⚠ DDC non disponible — appuyez sur un raccourci de luminosité pour réessayer",
    tray_warn_monitor_unresponsive: "⚠ L’écran ne répond pas — redémarrez l’application si le problème persiste",
    tray_warn_hotkeys_stopped: "⚠ Les raccourcis ne fonctionnent plus — redémarrez l’application",
    tray_warn_hotkey_change_failed: "⚠ Échec de la modification du raccourci — essayez une autre combinaison",
    tray_warn_file_logging_failed: "⚠ Impossible d’ouvrir le fichier journal — vérifiez que le dossier des journaux est accessible en écriture",

    tray_usage_heading: "Pointez la souris sur un écran, puis\u{a0}:",
    tray_usage_brighter: "Augmenter la luminosité",
    tray_usage_dimmer: "Diminuer la luminosité",
    tray_menu_settings: "Paramètres",
    tray_menu_open_log_folder: "Ouvrir le dossier des journaux",
    tray_menu_quit_fmt: "Quitter {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Maj",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Haut",
    key_down: "Bas",
    key_left: "Gauche",
    key_right: "Droite",
    key_page_up: "Pg préc",
    key_page_down: "Pg suiv",
    key_home: "Origine",
    key_end: "Fin",
    key_insert: "Inser",
    key_delete: "Suppr",
    key_space: "Espace",
    key_tab: "Tab",
    key_enter: "Entrée",
    key_escape: "Échap",
    key_backspace: "Retour arrière",

    header_general: "Général",
    label_language: "Langue",
    language_system_default: "Langue du système",
    autostart: "Démarrer avec Windows",
    label_step: "Incrément de luminosité par appui",
    unit_percent_step: "%",
    header_hotkeys: "Raccourcis",
    label_hotkey_up: "Augmenter la luminosité",
    label_hotkey_down: "Diminuer la luminosité",
    intercept: "Essayer d’intercepter les touches de luminosité",
    hint_intercept: "(ne fonctionne pas avec tous les claviers\u{202f}; certains antivirus signalent les hooks de bas niveau)",
    header_osd: "Affichage à l’écran",
    label_timeout: "Durée d’affichage",
    unit_milliseconds: "ms",
    label_opacity: "Opacité",
    unit_percent_opacity: "%",
    header_advanced: "Avancé",
    resync_check: "Resynchroniser la luminosité toutes les",
    unit_seconds_resync: "s",
    inactivity_check: "Resynchroniser après une inactivité de",
    unit_seconds_inactivity: "s",
    log_check: "Écrire dans un fichier journal",
    label_log_level: "Niveau\u{a0}:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(les modifications de journalisation s’appliquent au redémarrage\u{202f}; debug et trace journalisent les numéros de série des écrans et les chemins)",
    footer_links: "<a>Fichier de configuration</a> \u{b7} <a>Dossier des journaux</a>",
    button_restore_defaults: "Rétablir les valeurs par défaut",
    button_close: "Fermer",
    window_title: "Paramètres de darkbright-helper",

    capture_prompt: "Appuyez sur un raccourci… (Échap pour annuler)",
    capture_reject_no_modifier: "Ajoutez Ctrl, Alt ou Win (Maj seule ne suffit pas)",
    capture_reject_unnameable_key: "Impossible d’utiliser cette touche comme raccourci",
    capture_reject_duplicate: "Combinaison déjà attribuée à l’autre raccourci",

    hotkey_status_unreachable: "Impossible de joindre le thread des raccourcis",
    hotkey_status_no_response: "Le thread des raccourcis n’a pas répondu",
    hotkey_status_unknown_error: "erreur inconnue",
    hotkey_status_restore_also_failed_fmt: "{error}\u{202f}; la restauration a également échoué\u{a0}: {restore_error}",
    hotkey_notice_interception_unavailable: "Interception non disponible, raccourcis toujours actifs",

    msgbox_already_running: "darkbright-helper est déjà en cours d’exécution.",
    msgbox_title_startup_error: "Erreur de démarrage",
    msgbox_title_hotkey_error: "Erreur de raccourci",
    msgbox_title_autostart: "Démarrage automatique",
    msgbox_title_restore_defaults: "Rétablir les valeurs par défaut",
    msgbox_startup_failed_lead: "Impossible de démarrer darkbright-helper\u{a0}:",
    msgbox_thread_spawn_advice: "Le système n’a pas pu démarrer de thread, ce qui signifie généralement qu’il manque de ressources. Fermez quelques applications ou redémarrez l’ordinateur, puis réessayez.",
    msgbox_hotkey_failed_lead: "Impossible d’activer les raccourcis\u{a0}:",
    msgbox_hotkey_advice_fmt: "Solutions possibles\u{a0}:\n• Fermer les autres applications qui pourraient utiliser ces raccourcis\n• Modifier la configuration des raccourcis dans\u{a0}:\n  {path}\n• Redémarrer l’application après les modifications",
    msgbox_autostart_failed_fmt: "Impossible de mettre à jour l’entrée de démarrage de Windows\u{a0}:\n{error}",
    msgbox_restore_defaults_question: "Rétablir les valeurs par défaut de tous les paramètres\u{202f}? Les raccourcis s’appliquent immédiatement.",
    msgbox_config_file_fallback: "fichier de configuration",
};
