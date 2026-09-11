//! The Spanish string table.
//!
//! Variant: neutral Spanish (`es`) for every region, `equipo` for computer, infinitive commands.
//! Style reference: Microsoft style guide at <https://aka.ms/spanish-neutral-styleguide>.
//!
//! Glossary (English → Spanish):
//! - brightness → brillo
//! - hotkey → atajo
//! - on-screen display → indicador en pantalla
//! - monitor → monitor
//! - log → registro
//! - log file → archivo de registro
//! - log folder → carpeta de registros
//! - settings → configuración
//! - resync → resincronizar
//! - inactivity → inactividad
//! - restore defaults → restaurar valores predeterminados
//! - intercept (keys) → interceptar
//! - start with Windows → iniciar con Windows
//! - config file → archivo de configuración
//! - hotkey thread → subproceso de atajos
//! - DDC → DDC
//!
//! Key names: the style guide's key and shortcut tables and Microsoft's Spanish Windows
//! shortcut page (Mayús, Supr, Re Pág, Av Pág, Entrar, Retroceso); Ctrl, Alt, Tab and Win
//! as accelerator labels and key caps print them.
//! Typography: opening `¿` on questions; no space before an ellipsis.

use super::Strings;

/// The Spanish strings.
pub(crate) const SPANISH: Strings = Strings {
    osd_ddc_error: "Error de DDC - No se pudo ajustar",

    tray_tip_ddc_unavailable: "DDC no disponible",
    tray_tip_monitor_unresponsive: "monitor sin respuesta",
    tray_tip_hotkeys_stopped: "atajos detenidos",
    tray_tip_hotkey_change_failed: "error al cambiar el atajo",
    tray_tip_file_logging_off: "sin archivo de registro",

    tray_warn_ddc_unavailable: "⚠ DDC no disponible — presionar un atajo de brillo para volver a intentarlo",
    tray_warn_monitor_unresponsive: "⚠ El monitor no responde — reiniciar la aplicación si el problema persiste",
    tray_warn_hotkeys_stopped: "⚠ Los atajos dejaron de funcionar — reiniciar la aplicación",
    tray_warn_hotkey_change_failed: "⚠ Error al cambiar el atajo — probar otra combinación",
    tray_warn_file_logging_failed: "⚠ No se pudo abrir el archivo de registro — verificar que se puede escribir en la carpeta de registros",

    tray_usage_heading: "Colocar el puntero sobre un monitor y luego:",
    tray_usage_brighter: "Subir brillo",
    tray_usage_dimmer: "Bajar brillo",
    tray_menu_settings: "Configuración",
    tray_menu_open_log_folder: "Abrir carpeta de registros",
    tray_menu_quit_fmt: "Salir de {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Mayús",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Flecha arriba",
    key_down: "Flecha abajo",
    key_left: "Flecha izquierda",
    key_right: "Flecha derecha",
    key_page_up: "Re Pág",
    key_page_down: "Av Pág",
    key_home: "Inicio",
    key_end: "Fin",
    key_insert: "Insertar",
    key_delete: "Supr",
    key_space: "Barra espaciadora",
    key_tab: "Tab",
    key_enter: "Entrar",
    key_escape: "Esc",
    key_backspace: "Retroceso",

    header_general: "General",
    label_language: "Idioma",
    language_system_default: "Idioma del sistema",
    autostart: "Iniciar con Windows",
    label_step: "Incremento de brillo por pulsación",
    unit_percent_step: "%",
    header_hotkeys: "Atajos",
    label_hotkey_up: "Subir brillo",
    label_hotkey_down: "Bajar brillo",
    intercept: "Tratar de interceptar las teclas de brillo",
    hint_intercept: "(puede no funcionar con todos los teclados; algunos antivirus alertan sobre los hooks de bajo nivel)",
    header_osd: "Indicador en pantalla",
    label_timeout: "Duración en pantalla",
    unit_milliseconds: "ms",
    label_opacity: "Opacidad",
    unit_percent_opacity: "%",
    header_advanced: "Avanzado",
    resync_check: "Resincronizar el brillo cada",
    unit_seconds_resync: "s",
    inactivity_check: "Resincronizar tras una inactividad de",
    unit_seconds_inactivity: "s",
    log_check: "Crear archivo de registro",
    label_log_level: "Nivel:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(los cambios de registro se aplican al reiniciar; debug y trace registran los números de serie de los monitores y las rutas)",
    footer_links: "<a>Abrir archivo de configuración</a> \u{b7} <a>Abrir carpeta de registros</a>",
    button_restore_defaults: "Restaurar valores predeterminados",
    button_close: "Cerrar",
    window_title: "Configuración de darkbright-helper",

    capture_prompt: "Presionar una combinación de teclas… (Esc para cancelar)",
    capture_reject_no_modifier: "Agregar Ctrl, Alt o Win (no basta solo con Mayús)",
    capture_reject_unnameable_key: "No se puede usar esta tecla en un atajo",
    capture_reject_duplicate: "Combinación ya asignada al otro atajo de brillo",

    hotkey_status_unreachable: "No se pudo acceder al subproceso de atajos",
    hotkey_status_no_response: "El subproceso de atajos no respondió",
    hotkey_status_unknown_error: "error desconocido",
    hotkey_status_restore_also_failed_fmt: "{error}; tampoco se pudo restaurar: {restore_error}",
    hotkey_notice_interception_unavailable: "Interceptación no disponible; los atajos sí funcionan",

    msgbox_already_running: "darkbright-helper ya se está ejecutando.",
    msgbox_title_startup_error: "Error de inicio",
    msgbox_title_hotkey_error: "Error de atajo",
    msgbox_title_autostart: "Inicio automático",
    msgbox_title_restore_defaults: "Restaurar valores predeterminados",
    msgbox_startup_failed_lead: "No se pudo iniciar darkbright-helper:",
    msgbox_thread_spawn_advice: "El sistema no pudo iniciar un subproceso, lo que suele indicar falta de recursos. Cerrar algunas aplicaciones o reiniciar el equipo, y volver a intentarlo.",
    msgbox_hotkey_failed_lead: "No se pudieron registrar los atajos:",
    msgbox_hotkey_advice_fmt: "Posibles soluciones:\n• Cerrar otras aplicaciones que puedan estar usando estos atajos\n• Cambiar la configuración de los atajos en:\n  {path}\n• Reiniciar la aplicación después de hacer los cambios",
    msgbox_autostart_failed_fmt: "No se pudo actualizar la entrada de inicio de Windows:\n{error}",
    msgbox_restore_defaults_question: "¿Restaurar los valores predeterminados de toda la configuración? Los atajos se aplican de inmediato.",
    msgbox_config_file_fallback: "archivo de configuración",
};
