//! The Greek string table.
//!
//! Variant: Standard Modern Greek (`el-GR`), monotonic, precomposed (NFC) letters, tonos only.
//! Style reference: Microsoft style guide at <https://aka.ms/greek-styleguide>.
//!
//! Glossary (English → Greek):
//! - brightness → φωτεινότητα
//! - hotkey → συντόμευση
//! - on-screen display → ένδειξη οθόνης
//! - monitor → οθόνη
//! - log → καταγραφή
//! - log file → αρχείο καταγραφής
//! - log folder → φάκελος καταγραφής
//! - settings → ρυθμίσεις
//! - resync → συγχρονισμός
//! - inactivity → αδράνεια
//! - restore defaults → επαναφορά προεπιλογών
//! - intercept (keys) → δέσμευση (πλήκτρων)
//! - start with Windows → εκκίνηση με τα Windows
//! - config file → αρχείο διαμόρφωσης
//! - hotkey thread → νήμα συντομεύσεων
//! - DDC → DDC
//!
//! Key names: the style guide's key table localizes only the arrows (Επάνω βέλος) and the
//! space bar; every other key keeps its English name, with Page Up and Page Down spelled as
//! two words, as that table spells them. Modifiers take the guide's own
//! shortcut-key spelling (Ctrl+Shift+Esc), not the running-text form «Control», and the
//! space bar the short combo form «Διάστημα» rather than «Πλήκτρο διαστήματος», which no
//! key-combination label has room for.
//! Typography: the Greek question mark is «;»; the Greek semicolon (ano teleia) is barred in
//! software by the guide, so clauses the English joins with «;» are split or joined with
//! «και»; U+2026 for the ellipsis; the em dash of the source rows kept, spaced, and used for
//! the hyphen the English OSD error row breaks with. The U+00B7 in `footer_links` is the
//! source's pinned link separator, not punctuation.

use super::Strings;

/// The Greek strings.
pub(crate) const GREEK: Strings = Strings {
    osd_ddc_error: "Σφάλμα DDC — η αλλαγή απέτυχε",

    tray_tip_ddc_unavailable: "DDC μη διαθέσιμο",
    tray_tip_monitor_unresponsive: "η οθόνη δεν αποκρίνεται",
    tray_tip_hotkeys_stopped: "διακοπή συντομεύσεων",
    tray_tip_hotkey_change_failed: "αποτυχία αλλαγής συντόμευσης",
    tray_tip_file_logging_off: "χωρίς αρχείο καταγραφής",

    tray_warn_ddc_unavailable: "⚠ Το DDC δεν είναι διαθέσιμο — πατήστε μια συντόμευση φωτεινότητας για νέα προσπάθεια",
    tray_warn_monitor_unresponsive: "⚠ Η οθόνη δεν αποκρίνεται — αν συνεχιστεί, επανεκκινήστε την εφαρμογή",
    tray_warn_hotkeys_stopped: "⚠ Οι συντομεύσεις σταμάτησαν να λειτουργούν — επανεκκινήστε την εφαρμογή",
    tray_warn_hotkey_change_failed: "⚠ Η αλλαγή συντόμευσης απέτυχε — δοκιμάστε άλλο συνδυασμό",
    tray_warn_file_logging_failed: "⚠ Η καταγραφή σε αρχείο δεν ξεκίνησε — ελέγξτε αν επιτρέπεται η εγγραφή στον φάκελο καταγραφής",

    tray_usage_heading: "Τοποθετήστε τον δείκτη σε μια οθόνη και μετά:",
    tray_usage_brighter: "Αύξηση φωτεινότητας",
    tray_usage_dimmer: "Μείωση φωτεινότητας",
    tray_menu_settings: "Ρυθμίσεις",
    tray_menu_open_log_folder: "Άνοιγμα φακέλου καταγραφής",
    tray_menu_quit_fmt: "Έξοδος από {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Shift",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Επάνω βέλος",
    key_down: "Κάτω βέλος",
    key_left: "Αριστερό βέλος",
    key_right: "Δεξιό βέλος",
    key_page_up: "Page Up",
    key_page_down: "Page Down",
    key_home: "Home",
    key_end: "End",
    key_insert: "Insert",
    key_delete: "Delete",
    key_space: "Διάστημα",
    key_tab: "Tab",
    key_enter: "Enter",
    key_escape: "Esc",
    key_backspace: "Backspace",

    header_general: "Γενικά",
    label_language: "Γλώσσα",
    language_system_default: "Του συστήματος",
    autostart: "Εκκίνηση με τα Windows",
    label_step: "Βήμα φωτεινότητας ανά πάτημα",
    unit_percent_step: "%",
    header_hotkeys: "Συντομεύσεις",
    label_hotkey_up: "Αύξηση φωτεινότητας",
    label_hotkey_down: "Μείωση φωτεινότητας",
    intercept: "Απόπειρα δέσμευσης ειδικών πλήκτρων φωτεινότητας",
    hint_intercept: "(δεν λειτουργεί με όλα τα πληκτρολόγια και ορισμένα antivirus επισημαίνουν τα hooks χαμηλού επιπέδου)",
    header_osd: "Ένδειξη οθόνης",
    label_timeout: "Διάρκεια εμφάνισης",
    unit_milliseconds: "ms",
    label_opacity: "Αδιαφάνεια",
    unit_percent_opacity: "%",
    header_advanced: "Για προχωρημένους",
    resync_check: "Συγχρονισμός φωτεινότητας κάθε",
    unit_seconds_resync: "s",
    inactivity_check: "Συγχρονισμός μετά από αδράνεια",
    unit_seconds_inactivity: "s",
    log_check: "Εγγραφή αρχείου καταγραφής",
    label_log_level: "Επίπεδο:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(οι αλλαγές στην καταγραφή ισχύουν μετά την επανεκκίνηση και τα επίπεδα debug και trace καταγράφουν σειριακούς αριθμούς οθονών και διαδρομές)",
    footer_links: "<a>Άνοιγμα αρχείου διαμόρφωσης</a> \u{b7} <a>Άνοιγμα φακέλου καταγραφής</a>",
    button_restore_defaults: "Επαναφορά προεπιλογών",
    button_close: "Κλείσιμο",
    window_title: "Ρυθμίσεις darkbright-helper",

    capture_prompt: "Πατήστε συνδυασμό πλήκτρων… (Esc για ακύρωση)",
    capture_reject_no_modifier: "Προσθέστε Ctrl, Alt ή Win (μόνο το Shift δεν αρκεί)",
    capture_reject_unnameable_key: "Το πλήκτρο δεν υποστηρίζεται σε συντομεύσεις",
    capture_reject_duplicate: "Έχει ήδη αντιστοιχιστεί στην άλλη συντόμευση",

    hotkey_status_unreachable: "Δεν ήταν δυνατή η επικοινωνία με το νήμα συντομεύσεων",
    hotkey_status_no_response: "Το νήμα συντομεύσεων δεν απάντησε",
    hotkey_status_unknown_error: "άγνωστο σφάλμα",
    hotkey_status_restore_also_failed_fmt: "{error}. Απέτυχε και η επαναφορά: {restore_error}",
    hotkey_notice_interception_unavailable: "Δεν έγινε δέσμευση πλήκτρων, οι συντομεύσεις λειτουργούν",

    msgbox_already_running: "Το darkbright-helper εκτελείται ήδη.",
    msgbox_title_startup_error: "Σφάλμα εκκίνησης",
    msgbox_title_hotkey_error: "Σφάλμα συντόμευσης",
    msgbox_title_autostart: "Αυτόματη εκκίνηση",
    msgbox_title_restore_defaults: "Επαναφορά προεπιλογών",
    msgbox_startup_failed_lead: "Το darkbright-helper δεν μπόρεσε να ξεκινήσει:",
    msgbox_thread_spawn_advice: "Το σύστημα δεν ξεκίνησε ένα νήμα, πράγμα που συνήθως σημαίνει ότι οι πόροι του έχουν εξαντληθεί. Κλείστε μερικές εφαρμογές ή επανεκκινήστε τον υπολογιστή και δοκιμάστε ξανά.",
    msgbox_hotkey_failed_lead: "Η καταχώριση των συντομεύσεων απέτυχε:",
    msgbox_hotkey_advice_fmt: "Πιθανές λύσεις:\n• Κλείστε άλλες εφαρμογές που ίσως χρησιμοποιούν αυτές τις συντομεύσεις\n• Αλλάξτε τη διαμόρφωση των συντομεύσεων στο:\n  {path}\n• Επανεκκινήστε την εφαρμογή μετά τις αλλαγές",
    msgbox_autostart_failed_fmt: "Δεν ήταν δυνατή η ενημέρωση της καταχώρισης εκκίνησης των Windows:\n{error}",
    msgbox_restore_defaults_question: "Επαναφορά όλων των ρυθμίσεων στις προεπιλογές; Οι συντομεύσεις εφαρμόζονται αμέσως.",
    msgbox_config_file_fallback: "αρχείο διαμόρφωσης",
};
