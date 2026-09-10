//! The Vietnamese string table.
//!
//! Variant: Standard Vietnamese (`vi-VN`), "bạn" address, NFC-precomposed characters only.
//! Style reference: Microsoft style guide at <https://aka.ms/vietnamese-styleguide>.
//!
//! Glossary (English → Vietnamese):
//! - brightness → độ sáng
//! - hotkey → phím tắt
//! - on-screen display → hiển thị trên màn hình
//! - monitor → màn hình
//! - log → nhật ký
//! - log file → tệp nhật ký
//! - log folder → thư mục nhật ký
//! - settings → cài đặt
//! - resync → đồng bộ lại
//! - inactivity → không hoạt động
//! - restore defaults → khôi phục mặc định
//! - intercept (keys) → chặn (phím)
//! - start with Windows → khởi động cùng Windows
//! - config file → tệp cấu hình
//! - hotkey thread → luồng phím tắt
//! - DDC → DDC
//!
//! Key names: the style guide's key table (Mũi tên Lên, Page Up, Esc), but the space bar takes
//! Microsoft's Vietnamese shortcut notation (Ctrl+Phím cách); Ctrl, Shift, Alt stay English.
//! Typography: no comma before và or hoặc in a list; no space before a colon or semicolon.

use super::Strings;

/// The Vietnamese strings.
pub(crate) const VIETNAMESE: Strings = Strings {
    osd_ddc_error: "Lỗi DDC - Không điều chỉnh được",

    tray_tip_ddc_unavailable: "DDC không khả dụng",
    tray_tip_monitor_unresponsive: "màn hình không phản hồi",
    tray_tip_hotkeys_stopped: "phím tắt ngừng hoạt động",
    tray_tip_hotkey_change_failed: "không đổi được phím tắt",
    tray_tip_file_logging_off: "không ghi được tệp nhật ký",

    tray_warn_ddc_unavailable: "⚠ DDC không khả dụng — nhấn một phím tắt độ sáng để thử lại",
    tray_warn_monitor_unresponsive: "⚠ Màn hình không phản hồi — khởi động lại ứng dụng nếu vẫn còn sự cố",
    tray_warn_hotkeys_stopped: "⚠ Phím tắt đã ngừng hoạt động — khởi động lại ứng dụng",
    tray_warn_hotkey_change_failed: "⚠ Không đổi được phím tắt — thử tổ hợp phím khác",
    tray_warn_file_logging_failed: "⚠ Không ghi được tệp nhật ký — kiểm tra quyền ghi vào thư mục nhật ký",

    tray_usage_heading: "Trỏ chuột vào màn hình, sau đó:",
    tray_usage_brighter: "Tăng độ sáng",
    tray_usage_dimmer: "Giảm độ sáng",
    tray_menu_settings: "Cài đặt",
    tray_menu_open_log_folder: "Mở thư mục nhật ký",
    tray_menu_quit_fmt: "Thoát {name}",

    key_mod_ctrl: "Ctrl",
    key_mod_alt: "Alt",
    key_mod_shift: "Shift",
    key_mod_win: "Win",
    key_separator: "+",

    key_up: "Mũi tên Lên",
    key_down: "Mũi tên Xuống",
    key_left: "Mũi tên Trái",
    key_right: "Mũi tên Phải",
    key_page_up: "Page Up",
    key_page_down: "Page Down",
    key_home: "Home",
    key_end: "End",
    key_insert: "Insert",
    key_delete: "Delete",
    key_space: "Phím cách",
    key_tab: "Tab",
    key_enter: "Enter",
    key_escape: "Esc",
    key_backspace: "Backspace",

    header_general: "Chung",
    label_language: "Ngôn ngữ",
    language_system_default: "Mặc định hệ thống",
    autostart: "Khởi động cùng Windows",
    label_step: "Bước độ sáng mỗi lần nhấn phím",
    unit_percent_step: "%",
    header_hotkeys: "Phím tắt",
    label_hotkey_up: "Tăng độ sáng",
    label_hotkey_down: "Giảm độ sáng",
    intercept: "Thử chặn các phím độ sáng chuyên dụng",
    hint_intercept: "(có thể không hoạt động với mọi bàn phím; một số phần mềm chống vi-rút cảnh báo về hook cấp thấp)",
    header_osd: "Hiển thị trên màn hình",
    label_timeout: "Thời gian hiển thị",
    unit_milliseconds: "ms",
    label_opacity: "Độ mờ đục",
    unit_percent_opacity: "%",
    header_advanced: "Nâng cao",
    resync_check: "Đồng bộ lại độ sáng mỗi",
    unit_seconds_resync: "s",
    inactivity_check: "Đồng bộ lại sau khi không hoạt động trong",
    unit_seconds_inactivity: "s",
    log_check: "Ghi tệp nhật ký",
    label_log_level: "Mức:",
    log_level_error: "error",
    log_level_warn: "warn",
    log_level_info: "info",
    log_level_debug: "debug",
    log_level_trace: "trace",
    hint_logging: "(thay đổi cài đặt nhật ký có hiệu lực sau khi khởi động lại; debug và trace ghi lại số sê-ri màn hình và đường dẫn)",
    footer_links: "<a>Mở tệp cấu hình</a> \u{b7} <a>Mở thư mục nhật ký</a>",
    button_restore_defaults: "Khôi phục mặc định",
    button_close: "Đóng",
    window_title: "Cài đặt darkbright-helper",

    capture_prompt: "Nhấn tổ hợp phím… (Esc để hủy)",
    capture_reject_no_modifier: "Thêm Ctrl, Alt hoặc Win (chỉ Shift thì chưa đủ)",
    capture_reject_unnameable_key: "Không dùng được phím này làm phím tắt",
    capture_reject_duplicate: "Đã gán cho phím tắt độ sáng còn lại",

    hotkey_status_unreachable: "Không kết nối được với luồng phím tắt",
    hotkey_status_no_response: "Luồng phím tắt không phản hồi",
    hotkey_status_unknown_error: "lỗi không xác định",
    hotkey_status_restore_also_failed_fmt: "{error}; cũng không khôi phục được: {restore_error}",
    hotkey_notice_interception_unavailable: "Không chặn được phím; phím tắt vẫn hoạt động",

    msgbox_already_running: "darkbright-helper hiện đang chạy.",
    msgbox_title_startup_error: "Lỗi khởi động",
    msgbox_title_hotkey_error: "Lỗi phím tắt",
    msgbox_title_autostart: "Khởi động cùng Windows",
    msgbox_title_restore_defaults: "Khôi phục mặc định",
    msgbox_startup_failed_lead: "Không khởi động được darkbright-helper:",
    msgbox_thread_spawn_advice: "Hệ thống không tạo được luồng mới, thường là do đã hết tài nguyên. Hãy đóng bớt ứng dụng hoặc khởi động lại máy tính rồi thử lại.",
    msgbox_hotkey_failed_lead: "Không đăng ký được phím tắt:",
    msgbox_hotkey_advice_fmt: "Cách khắc phục:\n• Đóng các ứng dụng khác có thể đang dùng các phím tắt này\n• Thay đổi cấu hình phím tắt trong:\n  {path}\n• Khởi động lại ứng dụng sau khi thay đổi",
    msgbox_autostart_failed_fmt: "Không cập nhật được mục khởi động cùng Windows:\n{error}",
    msgbox_restore_defaults_question: "Bạn có muốn khôi phục mọi cài đặt về mặc định không? Phím tắt sẽ được áp dụng ngay.",
    msgbox_config_file_fallback: "tệp cấu hình",
};
