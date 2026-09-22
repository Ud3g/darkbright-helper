//! The Polish string table.
//!
//! Variant: Standard Polish (Poland), `pl-PL` conventions, second-person imperative address.
//! Style reference: Microsoft style guide at <https://aka.ms/polish-styleguide>.
//!
//! Glossary (English → Polish):
//! - brightness → jasność
//! - hotkey → klawisz skrótu (short form "skrót" where the slot is too narrow)
//! - on-screen display → wskaźnik ekranowy
//! - monitor → monitor
//! - log → dziennik
//! - log file → plik dziennika
//! - log folder → folder dziennika
//! - settings → ustawienia
//! - resync → synchronizować
//! - inactivity → bezczynność
//! - restore defaults → przywrócić ustawienia domyślne
//! - intercept (keys) → przechwytywać
//! - start with Windows → uruchamiać z systemem Windows
//! - config file → plik konfiguracyjny
//! - hotkey thread → wątek klawiszy skrótów
//! - DDC → DDC
//!
//! Key names: the style guide's key table (Strzałka w górę, Spacja, Esc, Page Up); every
//! other key keeps the English name Polish key caps print. Win as in `de`/`fr`/`ru`.
//! Typography: spaced em dash (also in place of the source's hyphen), U+2026 ellipsis,
//! `\u{a0}` after one-letter prepositions and conjunctions in text that wraps (hints, message
//! boxes), sentence case throughout — Polish capitalizes only the first word of a title or label.

use super::Strings;

/// The Polish strings.
pub(crate) const POLISH: Strings = Strings {
    osd_ddc_error: "Błąd DDC — nie można zmienić jasności",

    tray_tip_ddc_unavailable: "DDC niedostępny",
    tray_tip_monitor_unresponsive: "monitor nie odpowiada",
    tray_tip_hotkeys_stopped: "skróty nie działają",
    tray_tip_hotkey_change_failed: "nie udało się zmienić skrótu",
    tray_tip_file_logging_off: "brak pliku dziennika",

    tray_warn_ddc_unavailable: "⚠ DDC niedostępny — naciśnij klawisz skrótu jasności, aby spróbować ponownie",
    tray_warn_monitor_unresponsive: "⚠ Monitor nie odpowiada — jeśli to się powtarza, uruchom ponownie aplikację",
    tray_warn_hotkeys_stopped: "⚠ Klawisze skrótów przestały działać — uruchom ponownie aplikację",
    tray_warn_hotkey_change_failed: "⚠ Nie można zmienić klawisza skrótu — wypróbuj inną kombinację",
    tray_warn_file_logging_failed: "⚠ Nie można rozpocząć zapisu dziennika — sprawdź, czy folder dziennika jest dostępny do zapisu",

    tray_usage_heading: "Wskaż myszą monitor, a następnie:",
    tray_usage_brighter: "Zwiększ jasność",
    tray_usage_dimmer: "Zmniejsz jasność",
    tray_menu_settings: "Ustawienia",
    tray_menu_open_log_folder: "Otwórz folder dziennika",
    tray_menu_quit_fmt: "Zakończ {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Shift",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Strzałka w górę",
    key_down: "Strzałka w dół",
    key_left: "Strzałka w lewo",
    key_right: "Strzałka w prawo",
    key_page_up: "Page Up",
    key_page_down: "Page Down",
    key_home: "Home",
    key_end: "End",
    key_insert: "Insert",
    key_delete: "Delete",
    key_space: "Spacja",
    key_tab: "Tab",
    key_enter: "Enter",
    key_escape: "Esc",
    key_backspace: "Backspace",

    header_general: "Ogólne",
    label_language: "Język",
    language_system_default: "Język systemu",
    autostart: "Uruchamiaj z systemem Windows",
    label_step: "Krok jasności na naciśnięcie",
    unit_percent_step: "%",
    header_hotkeys: "Klawisze skrótów",
    label_hotkey_up: "Zwiększ jasność",
    label_hotkey_down: "Zmniejsz jasność",
    intercept: "Próbuj przechwytywać klawisze jasności",
    hint_intercept: "(może nie działać ze wszystkimi klawiaturami; niektóre programy antywirusowe uznają zaczepy niskiego poziomu za podejrzane)",
    header_osd: "Wskaźnik ekranowy",
    label_timeout: "Czas wyświetlania",
    unit_milliseconds: "ms",
    label_opacity: "Nieprzezroczystość",
    unit_percent_opacity: "%",
    header_advanced: "Zaawansowane",
    resync_check: "Synchronizuj jasność co",
    unit_seconds_resync: "s",
    inactivity_check: "Synchronizuj po bezczynności",
    unit_seconds_inactivity: "s",
    log_check: "Zapisuj plik dziennika",
    label_log_level: "Poziom:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(zmiany ustawień dziennika zaczynają obowiązywać po ponownym uruchomieniu; poziomy debug i\u{a0}trace zapisują numery seryjne monitorów i\u{a0}ścieżki)",
    footer_links: "<a>Otwórz plik konfiguracyjny</a> \u{b7} <a>Otwórz folder dziennika</a>",
    button_restore_defaults: "Przywróć ustawienia domyślne",
    button_close: "Zamknij",
    window_title: "darkbright-helper — Ustawienia",

    capture_prompt: "Naciśnij klawisze… (Esc anuluje)",
    capture_reject_no_modifier: "Dodaj Ctrl, Alt lub Win (sam Shift nie wystarczy)",
    capture_reject_unnameable_key: "Tego klawisza nie można użyć jako skrótu",
    capture_reject_duplicate: "Ta kombinacja jest już przypisana do drugiego skrótu",

    hotkey_status_unreachable: "Nie można połączyć się z wątkiem klawiszy skrótów",
    hotkey_status_no_response: "Wątek klawiszy skrótów nie odpowiedział",
    hotkey_status_unknown_error: "nieznany błąd",
    hotkey_status_restore_also_failed_fmt: "{error}; przywrócenie również się nie powiodło: {restore_error}",
    hotkey_notice_interception_unavailable: "Przechwytywanie niedostępne; skróty nadal działają",

    msgbox_already_running: "Program darkbright-helper jest już uruchomiony.",
    msgbox_title_startup_error: "Błąd uruchamiania",
    msgbox_title_hotkey_error: "Błąd klawisza skrótu",
    msgbox_title_autostart: "Autostart",
    msgbox_title_restore_defaults: "Przywracanie ustawień domyślnych",
    msgbox_startup_failed_lead: "Nie można uruchomić programu darkbright-helper:",
    msgbox_thread_spawn_advice: "System nie uruchomił wątku, co zwykle oznacza brak wolnych zasobów. Zamknij niektóre aplikacje lub uruchom ponownie komputer, a\u{a0}potem spróbuj jeszcze raz.",
    msgbox_hotkey_failed_lead: "Nie można zarejestrować klawiszy skrótów:",
    msgbox_hotkey_advice_fmt: "Możliwe rozwiązania:
• Zamknij inne aplikacje, które mogą używać tych klawiszy skrótów
• Zmień konfigurację klawiszy skrótów w\u{a0}pliku:
  {path}
• Po wprowadzeniu zmian uruchom ponownie aplikację",
    msgbox_autostart_failed_fmt: "Nie można zaktualizować wpisu autostartu systemu Windows:\n{error}",
    msgbox_restore_defaults_question: "Czy przywrócić wszystkie ustawienia domyślne? Klawisze skrótów zostaną zastosowane natychmiast.",
    msgbox_config_file_fallback: "plik konfiguracyjny",
};
