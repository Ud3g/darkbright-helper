//! The Brazilian Portuguese string table.
//!
//! Variant: Brazilian Portuguese (`pt-BR`), user addressed as "você"; commands (menu items,
//! buttons, checkboxes) as infinitives, instructions in messages as imperatives.
//! Style reference: Microsoft style guide at <https://aka.ms/portuguese-brazil-styleguide>.
//!
//! Glossary (English → Brazilian Portuguese):
//! - brightness → brilho
//! - hotkey → tecla de atalho
//! - on-screen display → exibição na tela
//! - monitor → monitor
//! - log → log
//! - log file → arquivo de log
//! - log folder → pasta de logs
//! - settings → configurações
//! - resync → ressincronizar
//! - inactivity → inatividade
//! - restore defaults → restaurar padrões
//! - intercept (keys) → interceptar
//! - start with Windows → iniciar com o Windows
//! - config file → arquivo de configuração
//! - hotkey thread → thread das teclas de atalho
//! - DDC → DDC
//!
//! Key names: the style guide's key table (Seta para cima, Barra de espaços, Page Up, Esc);
//! Ctrl as the guide's shortcut tables and ABNT2 key caps print it (its key table says Control);
//! Win, as the other tables use it, since the caps show only the Windows logo.
//! Typography: no comma before `ou` in a series; the source's capitalization kept on UI options
//! (menu commands, the restore-defaults title), sentence case on error text and
//! error message-box titles, as the style guide prescribes.

use super::Strings;

/// The Brazilian Portuguese strings.
pub(crate) const PORTUGUESE: Strings = Strings {
    osd_ddc_error: "Erro de DDC - Falha no ajuste",

    tray_tip_ddc_unavailable: "DDC não disponível",
    tray_tip_monitor_unresponsive: "monitor não responde",
    tray_tip_hotkeys_stopped: "teclas de atalho inativas",
    tray_tip_hotkey_change_failed: "falha ao alterar tecla de atalho",
    tray_tip_file_logging_off: "sem arquivo de log",

    tray_warn_ddc_unavailable: "⚠ DDC não disponível — pressione uma tecla de atalho de brilho para tentar novamente",
    tray_warn_monitor_unresponsive: "⚠ O monitor não responde — reinicie o aplicativo se o problema persistir",
    tray_warn_hotkeys_stopped: "⚠ As teclas de atalho pararam de funcionar — reinicie o aplicativo",
    tray_warn_hotkey_change_failed: "⚠ Falha ao alterar a tecla de atalho — tente outra combinação",
    tray_warn_file_logging_failed: "⚠ Falha ao abrir o arquivo de log — verifique se é possível gravar na pasta de logs",

    tray_usage_heading: "Aponte o mouse para um monitor e use:",
    tray_usage_brighter: "Aumentar brilho",
    tray_usage_dimmer: "Diminuir brilho",
    tray_menu_settings: "Configurações",
    tray_menu_open_log_folder: "Abrir Pasta de Logs",
    tray_menu_quit_fmt: "Sair do {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Shift",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Seta para cima",
    key_down: "Seta para baixo",
    key_left: "Seta para a esquerda",
    key_right: "Seta para a direita",
    key_page_up: "Page Up",
    key_page_down: "Page Down",
    key_home: "Home",
    key_end: "End",
    key_insert: "Insert",
    key_delete: "Delete",
    key_space: "Barra de espaços",
    key_tab: "Tab",
    key_enter: "Enter",
    key_escape: "Esc",
    key_backspace: "Backspace",

    header_general: "Geral",
    label_language: "Idioma",
    language_system_default: "Padrão do sistema",
    autostart: "Iniciar com o Windows",
    label_step: "Incremento de brilho por tecla",
    unit_percent_step: "%",
    header_hotkeys: "Teclas de atalho",
    label_hotkey_up: "Aumentar brilho",
    label_hotkey_down: "Diminuir brilho",
    intercept: "Tentar interceptar as teclas de brilho dedicadas",
    hint_intercept: "(pode não funcionar com todos os teclados; alguns antivírus sinalizam hooks de baixo nível)",
    header_osd: "Exibição na tela",
    label_timeout: "Tempo de exibição",
    unit_milliseconds: "ms",
    label_opacity: "Opacidade",
    unit_percent_opacity: "%",
    header_advanced: "Avançado",
    resync_check: "Ressincronizar o brilho a cada",
    unit_seconds_resync: "s",
    inactivity_check: "Ressincronizar após inatividade de",
    unit_seconds_inactivity: "s",
    log_check: "Gravar arquivo de log",
    label_log_level: "Nível:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(as alterações de log entram em vigor após reiniciar; debug e trace registram números de série de monitores e caminhos)",
    footer_links: "<a>Abrir arquivo de configuração</a> \u{b7} <a>Abrir pasta de logs</a>",
    button_restore_defaults: "Restaurar padrões",
    button_close: "Fechar",
    window_title: "Configurações do darkbright-helper",

    capture_prompt: "Pressione uma combinação de teclas… (Esc para cancelar)",
    capture_reject_no_modifier: "Adicione Ctrl, Alt ou Win (Shift sozinho não basta)",
    capture_reject_unnameable_key: "Esta tecla não pode ser usada como tecla de atalho",
    capture_reject_duplicate: "Já atribuída à outra tecla de atalho de brilho",

    hotkey_status_unreachable: "Thread das teclas de atalho inacessível",
    hotkey_status_no_response: "O thread das teclas de atalho não respondeu",
    hotkey_status_unknown_error: "erro desconhecido",
    hotkey_status_restore_also_failed_fmt: "{error}; a restauração também falhou: {restore_error}",
    hotkey_notice_interception_unavailable: "Interceptação não disponível; teclas de atalho ativas",

    msgbox_already_running: "O darkbright-helper já está em execução.",
    msgbox_title_startup_error: "Erro de inicialização",
    msgbox_title_hotkey_error: "Erro de tecla de atalho",
    msgbox_title_autostart: "Inicialização automática",
    msgbox_title_restore_defaults: "Restaurar Padrões",
    msgbox_startup_failed_lead: "Não foi possível iniciar o darkbright-helper:",
    msgbox_thread_spawn_advice: "O sistema não conseguiu iniciar um thread, o que geralmente indica falta de recursos. Feche alguns aplicativos ou reinicie o computador e tente novamente.",
    msgbox_hotkey_failed_lead: "Falha ao registrar as teclas de atalho:",
    msgbox_hotkey_advice_fmt: "Possíveis soluções:\n• Feche outros aplicativos que possam estar usando essas teclas de atalho\n• Altere a configuração das teclas de atalho em:\n  {path}\n• Reinicie o aplicativo depois de fazer as alterações",
    msgbox_autostart_failed_fmt: "Não foi possível atualizar a entrada de inicialização do Windows:\n{error}",
    msgbox_restore_defaults_question: "Restaurar os padrões de todas as configurações? As teclas de atalho são aplicadas imediatamente.",
    msgbox_config_file_fallback: "arquivo de configuração",
};
