//! The Russian string table.
//!
//! Variant: Standard Russian (`ru-RU` conventions), formal «вы» address, «е» written for «ё».
//! Style reference: Microsoft style guide at <https://aka.ms/russian-styleguide>.
//!
//! Glossary (English → Russian):
//! - brightness → яркость
//! - hotkey → сочетание клавиш
//! - on-screen display → экранный индикатор
//! - monitor → монитор
//! - log → журнал
//! - log file → файл журнала
//! - log folder → папка журналов
//! - settings → параметры
//! - resync → синхронизировать
//! - inactivity → бездействие
//! - restore defaults → восстановить значения по умолчанию
//! - intercept (keys) → перехватывать
//! - start with Windows → запускать вместе с Windows
//! - config file → файл конфигурации
//! - hotkey thread → поток сочетаний клавиш
//! - DDC → DDC
//!
//! Key names: the style guide's key table (Стрелка вверх, Ввод, Пробел); every other key
//! keeps the English name Russian key caps print. Mixed case, as Russian accelerator
//! labels show it (Ctrl+Z), not the guide's all-caps form for keys named in running text.
//! Typography: spaced em dash (also in place of the source's hyphen), U+2026 ellipsis.

use super::Strings;

/// The Russian strings.
pub(crate) const RUSSIAN: Strings = Strings {
    osd_ddc_error: "Ошибка DDC — яркость не изменена",

    tray_tip_ddc_unavailable: "DDC недоступен",
    tray_tip_monitor_unresponsive: "монитор не отвечает",
    tray_tip_hotkeys_stopped: "сочетания клавиш не работают",
    tray_tip_hotkey_change_failed: "сочетание клавиш не изменено",
    tray_tip_file_logging_off: "файл журнала недоступен",

    tray_warn_ddc_unavailable: "⚠ DDC недоступен — нажмите сочетание клавиш яркости, чтобы попробовать еще раз",
    tray_warn_monitor_unresponsive: "⚠ Монитор не отвечает — если проблема не исчезнет, перезапустите приложение",
    tray_warn_hotkeys_stopped: "⚠ Сочетания клавиш перестали работать — перезапустите приложение",
    tray_warn_hotkey_change_failed: "⚠ Не удалось изменить сочетание клавиш — попробуйте другое сочетание",
    tray_warn_file_logging_failed: "⚠ Не удалось начать запись журнала в файл — проверьте, доступна ли папка журналов для записи",

    tray_usage_heading: "Наведите указатель мыши на монитор, затем:",
    tray_usage_brighter: "Увеличить яркость",
    tray_usage_dimmer: "Уменьшить яркость",
    tray_menu_settings: "Параметры",
    tray_menu_open_log_folder: "Открыть папку журналов",
    tray_menu_quit_fmt: "Закрыть {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Shift",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Стрелка вверх",
    key_down: "Стрелка вниз",
    key_left: "Стрелка влево",
    key_right: "Стрелка вправо",
    key_page_up: "Page Up",
    key_page_down: "Page Down",
    key_home: "Home",
    key_end: "End",
    key_insert: "Insert",
    key_delete: "Delete",
    key_space: "Пробел",
    key_tab: "Tab",
    key_enter: "Ввод",
    key_escape: "Esc",
    key_backspace: "Backspace",

    header_general: "Общие",
    label_language: "Язык",
    language_system_default: "Язык системы",
    autostart: "Запускать вместе с Windows",
    label_step: "Шаг яркости за нажатие",
    unit_percent_step: "%",
    header_hotkeys: "Сочетания клавиш",
    label_hotkey_up: "Увеличить яркость",
    label_hotkey_down: "Уменьшить яркость",
    intercept: "Пытаться перехватывать клавиши яркости",
    hint_intercept: "(работает не со всеми клавиатурами, и некоторые антивирусы считают низкоуровневые перехватчики подозрительными)",
    header_osd: "Экранный индикатор",
    label_timeout: "Время показа",
    unit_milliseconds: "мс",
    label_opacity: "Непрозрачность",
    unit_percent_opacity: "%",
    header_advanced: "Дополнительно",
    resync_check: "Синхронизировать яркость каждые",
    unit_seconds_resync: "с",
    inactivity_check: "Синхронизировать после бездействия в течение",
    unit_seconds_inactivity: "с",
    log_check: "Записывать журнал в файл",
    label_log_level: "Уровень:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(изменения параметров журнала применяются после перезапуска, а на уровнях debug и trace в журнал записываются серийные номера мониторов и пути)",
    footer_links: "<a>Открыть файл конфигурации</a> \u{b7} <a>Открыть папку журналов</a>",
    button_restore_defaults: "Восстановить значения по умолчанию",
    button_close: "Закрыть",
    window_title: "Параметры darkbright-helper",

    capture_prompt: "Нажмите сочетание клавиш… (Esc для отмены)",
    capture_reject_no_modifier: "Добавьте Ctrl, Alt или Win (одного Shift недостаточно)",
    capture_reject_unnameable_key: "Эта клавиша не поддерживается в сочетаниях клавиш",
    capture_reject_duplicate: "Уже назначено другой команде яркости",

    hotkey_status_unreachable: "Не удалось связаться с потоком сочетаний клавиш",
    hotkey_status_no_response: "Поток сочетаний клавиш не ответил",
    hotkey_status_unknown_error: "неизвестная ошибка",
    hotkey_status_restore_also_failed_fmt: "{error}; восстановить тоже не удалось: {restore_error}",
    hotkey_notice_interception_unavailable: "Перехват недоступен, но сочетания клавиш работают",

    msgbox_already_running: "Приложение darkbright-helper уже запущено.",
    msgbox_title_startup_error: "Ошибка запуска",
    msgbox_title_hotkey_error: "Ошибка сочетания клавиш",
    msgbox_title_autostart: "Автозагрузка",
    msgbox_title_restore_defaults: "Восстановить значения по умолчанию",
    msgbox_startup_failed_lead: "Не удалось запустить darkbright-helper:",
    msgbox_thread_spawn_advice: "Системе не удалось запустить поток. Обычно это означает, что не хватает ресурсов. Закройте несколько приложений или перезагрузите компьютер и попробуйте еще раз.",
    msgbox_hotkey_failed_lead: "Не удалось зарегистрировать сочетания клавиш:",
    msgbox_hotkey_advice_fmt: "Возможные решения:\n• Закройте другие приложения, которые могут использовать эти сочетания клавиш\n• Измените сочетания клавиш здесь:\n  {path}\n• После изменений перезапустите приложение",
    msgbox_autostart_failed_fmt: "Не удалось обновить запись автозагрузки Windows:\n{error}",
    msgbox_restore_defaults_question: "Вы хотите восстановить значения по умолчанию для всех параметров? Сочетания клавиш применятся сразу.",
    msgbox_config_file_fallback: "файл конфигурации",
};
