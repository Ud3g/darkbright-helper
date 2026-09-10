//! The Indonesian string table.
//!
//! Variant: Standard Bahasa Indonesia (`id-ID`), the formal register Windows uses.
//! Style reference: Microsoft style guide at <https://aka.ms/indonesian-styleguide>.
//!
//! Glossary (English → Indonesian):
//! - brightness → kecerahan
//! - hotkey → tombol pintas
//! - on-screen display → tampilan di layar
//! - monitor → monitor
//! - log → log
//! - log file → file log
//! - log folder → folder log
//! - settings → pengaturan
//! - resync → sinkronkan ulang
//! - inactivity → tidak aktif
//! - restore defaults → pulihkan default
//! - intercept (keys) → cegat (noun: pencegatan)
//! - start with Windows → mulai bersama Windows
//! - config file → file konfigurasi
//! - hotkey thread → thread tombol pintas
//! - DDC → DDC
//!
//! Key names: the style guide leaves key names untranslated except the arrow keys (Panah Kiri);
//! its shortcut table keeps Ctrl, Alt, Esc and Backspace in English and writes Alt+Spasi.

use super::Strings;

/// The Indonesian strings.
pub(crate) const INDONESIAN: Strings = Strings {
    osd_ddc_error: "Kesalahan DDC - Penyesuaian gagal",

    tray_tip_ddc_unavailable: "DDC tidak tersedia",
    tray_tip_monitor_unresponsive: "monitor tidak merespons",
    tray_tip_hotkeys_stopped: "tombol pintas tidak berfungsi",
    tray_tip_hotkey_change_failed: "perubahan tombol pintas gagal",
    tray_tip_file_logging_off: "file log nonaktif",

    tray_warn_ddc_unavailable: "⚠ DDC tidak tersedia — tekan tombol pintas kecerahan untuk mencoba lagi",
    tray_warn_monitor_unresponsive: "⚠ Monitor tidak merespons — mulai ulang aplikasi jika masalah berlanjut",
    tray_warn_hotkeys_stopped: "⚠ Tombol pintas tidak lagi berfungsi — mulai ulang aplikasi",
    tray_warn_hotkey_change_failed: "⚠ Perubahan tombol pintas gagal — coba kombinasi lain",
    tray_warn_file_logging_failed: "⚠ File log tidak dapat dibuka — pastikan folder log dapat ditulis",

    tray_usage_heading: "Arahkan mouse ke monitor, lalu:",
    tray_usage_brighter: "Naikkan kecerahan",
    tray_usage_dimmer: "Turunkan kecerahan",
    tray_menu_settings: "Pengaturan",
    tray_menu_open_log_folder: "Buka Folder Log",
    tray_menu_quit_fmt: "Keluar dari {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Shift",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Panah Atas",
    key_down: "Panah Bawah",
    key_left: "Panah Kiri",
    key_right: "Panah Kanan",
    key_page_up: "PageUp",
    key_page_down: "PageDown",
    key_home: "Home",
    key_end: "End",
    key_insert: "Insert",
    key_delete: "Delete",
    key_space: "Spasi",
    key_tab: "Tab",
    key_enter: "Enter",
    key_escape: "Esc",
    key_backspace: "Backspace",

    header_general: "Umum",
    label_language: "Bahasa",
    language_system_default: "Default sistem",
    autostart: "Mulai bersama Windows",
    label_step: "Langkah kecerahan per tekanan tombol",
    unit_percent_step: "%",
    header_hotkeys: "Tombol pintas",
    label_hotkey_up: "Naikkan kecerahan",
    label_hotkey_down: "Turunkan kecerahan",
    intercept: "Coba cegat tombol kecerahan khusus",
    hint_intercept: "(mungkin tidak berfungsi di semua keyboard; beberapa antivirus menandai hook tingkat rendah)",
    header_osd: "Tampilan di layar",
    label_timeout: "Durasi tampilan",
    unit_milliseconds: "ms",
    label_opacity: "Opasitas",
    unit_percent_opacity: "%",
    header_advanced: "Tingkat lanjut",
    resync_check: "Sinkronkan ulang kecerahan setiap",
    unit_seconds_resync: "dtk",
    inactivity_check: "Sinkronkan ulang setelah tidak aktif selama",
    unit_seconds_inactivity: "dtk",
    log_check: "Tulis file log",
    label_log_level: "Level:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(pengaturan log berlaku setelah aplikasi dimulai ulang; debug dan di bawahnya mencatat nomor seri monitor dan jalur)",
    footer_links: "<a>Buka file konfigurasi</a> \u{b7} <a>Buka folder log</a>",
    button_restore_defaults: "Pulihkan default",
    button_close: "Tutup",
    window_title: "Pengaturan darkbright-helper",

    capture_prompt: "Tekan kombinasi tombol… (Esc untuk membatalkan)",
    capture_reject_no_modifier: "Tambahkan Ctrl, Alt, atau Win (Shift saja tidak cukup)",
    capture_reject_unnameable_key: "Tombol ini tidak dapat digunakan sebagai tombol pintas",
    capture_reject_duplicate: "Sudah ditetapkan ke tombol pintas kecerahan lainnya",

    hotkey_status_unreachable: "Tidak dapat menghubungi thread tombol pintas",
    hotkey_status_no_response: "Thread tombol pintas tidak merespons",
    hotkey_status_unknown_error: "kesalahan tidak diketahui",
    hotkey_status_restore_also_failed_fmt: "{error}; pemulihan juga gagal: {restore_error}",
    hotkey_notice_interception_unavailable: "Pencegatan tidak tersedia; tombol pintas tetap berfungsi",

    msgbox_already_running: "darkbright-helper sudah berjalan.",
    msgbox_title_startup_error: "Kesalahan Saat Memulai",
    msgbox_title_hotkey_error: "Kesalahan Tombol Pintas",
    msgbox_title_autostart: "Mulai Otomatis",
    msgbox_title_restore_defaults: "Pulihkan Default",
    msgbox_startup_failed_lead: "darkbright-helper tidak dapat dimulai:",
    msgbox_thread_spawn_advice: "Sistem tidak dapat memulai thread, biasanya karena kehabisan sumber daya. Tutup beberapa aplikasi atau mulai ulang komputer, lalu coba lagi.",
    msgbox_hotkey_failed_lead: "Gagal mendaftarkan tombol pintas:",
    msgbox_hotkey_advice_fmt: "Kemungkinan solusi:\n• Tutup aplikasi lain yang mungkin menggunakan tombol pintas ini\n• Ubah konfigurasi tombol pintas di:\n  {path}\n• Mulai ulang aplikasi setelah melakukan perubahan",
    msgbox_autostart_failed_fmt: "Tidak dapat memperbarui entri mulai otomatis Windows:\n{error}",
    msgbox_restore_defaults_question: "Pulihkan semua pengaturan ke default? Tombol pintas akan langsung diterapkan.",
    msgbox_config_file_fallback: "file konfigurasi",
};
