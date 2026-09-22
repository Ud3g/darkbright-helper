//! The Ukrainian string table.
//!
//! Variant: Standard Ukrainian (`uk-UA` conventions), Cyrillic script, formal «ви» address.
//! Style reference: Microsoft style guide at <https://aka.ms/ukrainian-styleguide>.
//!
//! Glossary (English → Ukrainian):
//! - brightness → яскравість
//! - hotkey → сполучення клавіш
//! - on-screen display → екранний індикатор
//! - monitor → монітор
//! - log → журнал
//! - log file → файл журналу
//! - log folder → папка журналу
//! - settings → параметри
//! - resync → синхронізувати
//! - inactivity → бездіяльність
//! - restore defaults → відновити стандартні параметри
//! - intercept (keys) → перехоплювати
//! - start with Windows → запускати разом із Windows
//! - config file → файл конфігурації
//! - hotkey thread → потік сполучень клавіш
//! - DDC → DDC
//!
//! Key names: the guide's key table names the arrows in Ukrainian (Стрілка вгору) and
//! keeps every other key in English, as Ukrainian key caps print it. Пробіл is written in
//! mixed case, the form accelerator labels show, not the guide's all-caps running-text form.
//! Typography: curly apostrophe ’ (U+2019); the guide's spaced en dash as the sentence dash,
//! except in the tray warning rows, whose source shape keeps a spaced em dash; U+2026 ellipsis.
//! Default settings are «стандартні параметри», never «параметри за замовчуванням».
//! The seconds unit is Cyrillic «с» (U+0441), not Latin «s».

use super::Strings;

/// The Ukrainian strings.
pub(crate) const UKRAINIAN: Strings = Strings {
    osd_ddc_error: "Помилка DDC – яскравість не змінено",

    tray_tip_ddc_unavailable: "DDC недоступний",
    tray_tip_monitor_unresponsive: "монітор не відповідає",
    tray_tip_hotkeys_stopped: "сполучення клавіш не діють",
    tray_tip_hotkey_change_failed: "сполучення клавіш не змінено",
    tray_tip_file_logging_off: "файл журналу недоступний",

    tray_warn_ddc_unavailable: "⚠ DDC недоступний — натисніть сполучення клавіш яскравості, щоб повторити спробу",
    tray_warn_monitor_unresponsive: "⚠ Монітор не відповідає — якщо це триватиме, перезапустіть програму",
    tray_warn_hotkeys_stopped: "⚠ Сполучення клавіш перестали діяти — перезапустіть програму",
    tray_warn_hotkey_change_failed: "⚠ Не вдалося змінити сполучення клавіш — спробуйте інше",
    tray_warn_file_logging_failed: "⚠ Не вдалося почати запис журналу у файл — перевірте, чи доступна папка журналу для запису",

    tray_usage_heading: "Наведіть вказівник миші на монітор, потім:",
    tray_usage_brighter: "Збільшити яскравість",
    tray_usage_dimmer: "Зменшити яскравість",
    tray_menu_settings: "Параметри",
    tray_menu_open_log_folder: "Відкрити папку журналу",
    tray_menu_quit_fmt: "Закрити {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Shift",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Стрілка вгору",
    key_down: "Стрілка вниз",
    key_left: "Стрілка вліво",
    key_right: "Стрілка вправо",
    key_page_up: "Page Up",
    key_page_down: "Page Down",
    key_home: "Home",
    key_end: "End",
    key_insert: "Insert",
    key_delete: "Delete",
    key_space: "Пробіл",
    key_tab: "Tab",
    key_enter: "Enter",
    key_escape: "Esc",
    key_backspace: "Backspace",

    header_general: "Загальні",
    label_language: "Мова",
    language_system_default: "Мова системи",
    autostart: "Запускати разом із Windows",
    label_step: "Крок яскравості за натискання",
    unit_percent_step: "%",
    header_hotkeys: "Сполучення клавіш",
    label_hotkey_up: "Збільшити яскравість",
    label_hotkey_down: "Зменшити яскравість",
    intercept: "Намагатися перехоплювати клавіші яскравості",
    hint_intercept: "(може працювати не з усіма клавіатурами; деякі антивіруси вважають низькорівневі перехоплювачі підозрілими)",
    header_osd: "Екранний індикатор",
    label_timeout: "Тривалість показу",
    unit_milliseconds: "мс",
    label_opacity: "Непрозорість",
    unit_percent_opacity: "%",
    header_advanced: "Додатково",
    resync_check: "Синхронізувати яскравість кожні",
    unit_seconds_resync: "с",
    inactivity_check: "Синхронізувати після бездіяльності",
    unit_seconds_inactivity: "с",
    log_check: "Записувати журнал у файл",
    label_log_level: "Рівень:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(зміни параметрів журналу застосовуються після перезапуску; на рівнях debug і нижче записуються серійні номери моніторів і шляхи)",
    footer_links: "<a>Відкрити файл конфігурації</a> \u{b7} <a>Відкрити папку журналу</a>",
    button_restore_defaults: "Відновити стандартні параметри",
    button_close: "Закрити",
    window_title: "Параметри darkbright-helper",

    capture_prompt: "Натисніть сполучення клавіш… (Esc – скасувати)",
    capture_reject_no_modifier: "Додайте Ctrl, Alt або Win (самого Shift замало)",
    capture_reject_unnameable_key: "Ця клавіша не підходить для сполучення клавіш",
    capture_reject_duplicate: "Уже призначено іншому сполученню клавіш яскравості",

    hotkey_status_unreachable: "Не вдалося зв’язатися з потоком сполучень клавіш",
    hotkey_status_no_response: "Потік сполучень клавіш не відповів",
    hotkey_status_unknown_error: "невідома помилка",
    hotkey_status_restore_also_failed_fmt: "{error}; відновити попередні сполучення також не вдалося: {restore_error}",
    hotkey_notice_interception_unavailable: "Перехоплення недоступне; сполучення клавіш діють",

    msgbox_already_running: "darkbright-helper уже запущено.",
    msgbox_title_startup_error: "Помилка запуску",
    msgbox_title_hotkey_error: "Помилка сполучення клавіш",
    msgbox_title_autostart: "Автозавантаження",
    msgbox_title_restore_defaults: "Відновити стандартні параметри",
    msgbox_startup_failed_lead: "Не вдалося запустити darkbright-helper:",
    msgbox_thread_spawn_advice: "Системі не вдалося запустити потік; зазвичай це означає, що бракує ресурсів. Закрийте кілька програм або перезавантажте комп’ютер і повторіть спробу.",
    msgbox_hotkey_failed_lead: "Не вдалося зареєструвати сполучення клавіш:",
    msgbox_hotkey_advice_fmt: "Можливі рішення:\n• Закрийте інші програми, які можуть використовувати ці сполучення клавіш\n• Змініть сполучення клавіш у файлі:\n  {path}\n• Після змін перезапустіть програму",
    msgbox_autostart_failed_fmt: "Не вдалося оновити запис автозавантаження Windows:\n{error}",
    msgbox_restore_defaults_question: "Відновити стандартні значення всіх параметрів? Сполучення клавіш буде застосовано одразу.",
    msgbox_config_file_fallback: "файл конфігурації",
};
