//! The Turkish string table.
//!
//! Variant: Standard Turkish (Türkiye), `tr-TR` and Turkish Q keyboard conventions, "siz" address.
//! Style reference: Microsoft style guide at <https://aka.ms/turkish-styleguide>.
//!
//! Glossary (English → Turkish):
//! - brightness → parlaklık
//! - hotkey → kısayol tuşu
//! - on-screen display → ekran göstergesi
//! - monitor → monitör
//! - log → günlük
//! - log file → günlük dosyası
//! - log folder → günlük klasörü
//! - settings → ayarlar
//! - resync → yeniden eşitle
//! - inactivity → boşta kalma
//! - restore defaults → varsayılanları geri yükle
//! - intercept (keys) → yakala
//! - start with Windows → Windows ile başlat
//! - config file → yapılandırma dosyası
//! - hotkey thread → kısayol tuşu iş parçacığı
//! - DDC → DDC
//!
//! Key names: the style guide's key table (Geri Al, Sekme, Ara Çubuğu, Yukarı Ok, Page Up);
//! Ctrl as its shortcut table and Turkish Q key caps print it, not its prose "Control"; Win as
//! in `de`/`fr`.
//! Typography: straight apostrophe and quotes; em dash only in the kept `tray_warn_*` shape;
//! full-sentence bullets end with a period.

use super::Strings;

/// The Turkish strings.
pub(crate) const TURKISH: Strings = Strings {
    osd_ddc_error: "DDC hatası - Ayar yapılamadı",

    tray_tip_ddc_unavailable: "DDC kullanılamıyor",
    tray_tip_monitor_unresponsive: "monitör yanıt vermiyor",
    tray_tip_hotkeys_stopped: "kısayol tuşları durdu",
    tray_tip_hotkey_change_failed: "kısayol tuşu değiştirilemedi",
    tray_tip_file_logging_off: "günlük dosyası açılamadı",

    tray_warn_ddc_unavailable: "⚠ DDC kullanılamıyor — yeniden denemek için bir parlaklık kısayol tuşuna basın",
    tray_warn_monitor_unresponsive: "⚠ Monitör yanıt vermiyor — sorun devam ederse uygulamayı yeniden başlatın",
    tray_warn_hotkeys_stopped: "⚠ Kısayol tuşları çalışmıyor — uygulamayı yeniden başlatın",
    tray_warn_hotkey_change_failed: "⚠ Kısayol tuşu değiştirilemedi — başka bir tuş bileşimi deneyin",
    tray_warn_file_logging_failed: "⚠ Günlük dosyası açılamadı — günlük klasörünün yazılabilir olduğunu kontrol edin",

    tray_usage_heading: "Fareyi bir monitörün üzerine getirin, sonra:",
    tray_usage_brighter: "Parlaklığı artır",
    tray_usage_dimmer: "Parlaklığı azalt",
    tray_menu_settings: "Ayarlar",
    tray_menu_open_log_folder: "Günlük klasörünü aç",
    tray_menu_quit_fmt: "{name} uygulamasından çık",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Shift",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Yukarı Ok",
    key_down: "Aşağı Ok",
    key_left: "Sol Ok",
    key_right: "Sağ Ok",
    key_page_up: "Page Up",
    key_page_down: "Page Down",
    key_home: "Home",
    key_end: "End",
    key_insert: "Insert",
    key_delete: "Delete",
    key_space: "Ara Çubuğu",
    key_tab: "Sekme",
    key_enter: "Enter",
    key_escape: "Esc",
    key_backspace: "Geri Al",

    header_general: "Genel",
    label_language: "Dil",
    language_system_default: "Sistem dili",
    autostart: "Windows ile başlat",
    label_step: "Tuşa her basışta parlaklık adımı",
    unit_percent_step: "%",
    header_hotkeys: "Kısayol tuşları",
    label_hotkey_up: "Parlaklığı artır",
    label_hotkey_down: "Parlaklığı azalt",
    intercept: "Özel parlaklık tuşlarını yakalamayı dene",
    hint_intercept: "(tüm klavyelerde çalışmayabilir; bazı virüsten koruma yazılımları alt düzey kancaları şüpheli bulur)",
    header_osd: "Ekran göstergesi",
    label_timeout: "Gösterim süresi",
    unit_milliseconds: "ms",
    label_opacity: "Opaklık",
    unit_percent_opacity: "%",
    header_advanced: "Gelişmiş",
    resync_check: "Parlaklığı şu aralıklarla yeniden eşitle:",
    unit_seconds_resync: "sn",
    inactivity_check: "Boşta kalma süresi dolunca yeniden eşitle:",
    unit_seconds_inactivity: "sn",
    log_check: "Günlük dosyasına yaz",
    label_log_level: "Düzey:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(günlük kaydı değişiklikleri yeniden başlattıktan sonra geçerli olur; debug ve trace düzeyleri monitör seri numaralarını ve yolları günlüğe kaydeder)",
    footer_links: "<a>Yapılandırma dosyasını aç</a> \u{b7} <a>Günlük klasörünü aç</a>",
    button_restore_defaults: "Varsayılanları geri yükle",
    button_close: "Kapat",
    window_title: "darkbright-helper Ayarları",

    capture_prompt: "Bir tuş bileşimine basın… (iptal için Esc)",
    capture_reject_no_modifier: "Ctrl, Alt veya Win ekleyin (yalnızca Shift yetmez)",
    capture_reject_unnameable_key: "Bu tuş kısayol tuşu olarak kullanılamaz",
    capture_reject_duplicate: "Diğer parlaklık kısayol tuşuna zaten atanmış",

    hotkey_status_unreachable: "Kısayol tuşu iş parçacığına ulaşılamadı",
    hotkey_status_no_response: "Kısayol tuşu iş parçacığı yanıt vermedi",
    hotkey_status_unknown_error: "bilinmeyen hata",
    hotkey_status_restore_also_failed_fmt: "{error}; önceki kısayol tuşları da geri yüklenemedi: {restore_error}",
    hotkey_notice_interception_unavailable: "Tuş yakalama kullanılamıyor; kısayol tuşları çalışıyor",

    msgbox_already_running: "darkbright-helper zaten çalışıyor.",
    msgbox_title_startup_error: "Başlatma Hatası",
    msgbox_title_hotkey_error: "Kısayol Tuşu Hatası",
    msgbox_title_autostart: "Windows ile Başlat",
    msgbox_title_restore_defaults: "Varsayılanları Geri Yükle",
    msgbox_startup_failed_lead: "darkbright-helper başlatılamadı:",
    msgbox_thread_spawn_advice: "Sistem yeni bir iş parçacığı başlatamadı. Bu genellikle sistem kaynaklarının yetersiz olduğu anlamına gelir. Bazı uygulamaları kapatın veya bilgisayarı yeniden başlatın, sonra tekrar deneyin.",
    msgbox_hotkey_failed_lead: "Kısayol tuşları etkinleştirilemedi:",
    msgbox_hotkey_advice_fmt: "Olası çözümler:\n• Bu kısayol tuşlarını kullanıyor olabilecek diğer uygulamaları kapatın.\n• Kısayol tuşu yapılandırmasını şu dosyada değiştirin:\n  {path}\n• Değişiklik yaptıktan sonra uygulamayı yeniden başlatın.",
    msgbox_autostart_failed_fmt: "Windows ile başlatma girdisi güncelleştirilemedi:\n{error}",
    msgbox_restore_defaults_question: "Tüm ayarlar için varsayılanlar geri yüklensin mi? Kısayol tuşları hemen uygulanır.",
    msgbox_config_file_fallback: "yapılandırma dosyası",
};
