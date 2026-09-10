use super::{ENGLISH, Lang, LanguageSetting, SYSTEM_LANGUAGE, Strings, TextKey, strings};

/// `Lang::index` panics on a variant missing from [`Lang::ALL`], and it
/// runs on the tray and settings language-push paths, so the list has to
/// stay complete. The `match` names every variant with no wildcard arm, so
/// a new one fails to compile here until it is added to both.
#[test]
fn every_language_variant_appears_in_all() {
    for lang in [
        Lang::English,
        Lang::German,
        Lang::French,
        Lang::Portuguese,
        Lang::Spanish,
        Lang::Indonesian,
        Lang::Russian,
        Lang::Turkish,
        Lang::Vietnamese,
    ] {
        let listed = match lang {
            Lang::English => Lang::ALL.contains(&Lang::English),
            Lang::German => Lang::ALL.contains(&Lang::German),
            Lang::French => Lang::ALL.contains(&Lang::French),
            Lang::Portuguese => Lang::ALL.contains(&Lang::Portuguese),
            Lang::Spanish => Lang::ALL.contains(&Lang::Spanish),
            Lang::Indonesian => Lang::ALL.contains(&Lang::Indonesian),
            Lang::Russian => Lang::ALL.contains(&Lang::Russian),
            Lang::Turkish => Lang::ALL.contains(&Lang::Turkish),
            Lang::Vietnamese => Lang::ALL.contains(&Lang::Vietnamese),
        };
        assert!(listed, "{lang:?} is missing from Lang::ALL");
    }
}

#[test]
fn every_language_tag_is_unique_and_lowercase() {
    let mut seen = Vec::new();
    for lang in Lang::ALL {
        let tag = lang.tag();
        assert!(!tag.is_empty(), "{lang:?} has an empty tag");
        assert_eq!(tag, tag.to_lowercase(), "{lang:?} tag is not lowercase");
        assert!(!seen.contains(&tag), "duplicate tag {tag}");
        seen.push(tag);
    }
}

/// Every field of the table, paired with its name.
///
/// The destructuring binding carries no rest pattern, so a field added to
/// [`Strings`] without an entry here is a compile error (`E0027`) rather
/// than something a reviewer has to catch. The pattern is packed several
/// names to a line, and the function is exempt from rustfmt, because one
/// name per line puts it past the line-count lint.
#[rustfmt::skip]
fn every_field(s: &Strings) -> impl IntoIterator<Item = (&'static str, &'static str)> {
    let Strings {
        osd_ddc_error, tray_tip_ddc_unavailable, tray_tip_monitor_unresponsive,
        tray_tip_hotkeys_stopped, tray_tip_hotkey_change_failed, tray_tip_file_logging_off,
        tray_warn_ddc_unavailable, tray_warn_monitor_unresponsive, tray_warn_hotkeys_stopped,
        tray_warn_hotkey_change_failed, tray_warn_file_logging_failed, tray_usage_heading,
        tray_usage_brighter, tray_usage_dimmer, tray_menu_settings, tray_menu_open_log_folder,
        tray_menu_quit_fmt, key_mod_ctrl, key_mod_alt, key_mod_shift, key_mod_win,
        key_separator, key_up, key_down, key_left, key_right, key_page_up, key_page_down,
        key_home, key_end, key_insert, key_delete, key_space, key_tab, key_enter, key_escape,
        key_backspace, header_general, label_language, language_system_default, autostart,
        label_step, unit_percent_step, header_hotkeys,
        label_hotkey_up, label_hotkey_down, intercept, hint_intercept, header_osd,
        label_timeout, unit_milliseconds, label_opacity, unit_percent_opacity, header_advanced,
        resync_check, unit_seconds_resync, inactivity_check, unit_seconds_inactivity, log_check,
        label_log_level, log_level_error, log_level_warn, log_level_info, log_level_debug,
        log_level_trace, hint_logging, footer_links, button_restore_defaults, button_close,
        window_title, capture_prompt, capture_reject_no_modifier, capture_reject_unnameable_key,
        capture_reject_duplicate, hotkey_status_unreachable, hotkey_status_no_response,
        hotkey_status_unknown_error, hotkey_status_restore_also_failed_fmt,
        hotkey_notice_interception_unavailable, msgbox_already_running,
        msgbox_title_startup_error, msgbox_title_hotkey_error, msgbox_title_autostart,
        msgbox_title_restore_defaults, msgbox_startup_failed_lead, msgbox_thread_spawn_advice,
        msgbox_hotkey_failed_lead, msgbox_hotkey_advice_fmt, msgbox_autostart_failed_fmt,
        msgbox_restore_defaults_question, msgbox_config_file_fallback,
    } = *s;
    [
        ("osd_ddc_error", osd_ddc_error),
        ("tray_tip_ddc_unavailable", tray_tip_ddc_unavailable),
        ("tray_tip_monitor_unresponsive", tray_tip_monitor_unresponsive),
        ("tray_tip_hotkeys_stopped", tray_tip_hotkeys_stopped),
        ("tray_tip_hotkey_change_failed", tray_tip_hotkey_change_failed),
        ("tray_tip_file_logging_off", tray_tip_file_logging_off),
        ("tray_warn_ddc_unavailable", tray_warn_ddc_unavailable),
        ("tray_warn_monitor_unresponsive", tray_warn_monitor_unresponsive),
        ("tray_warn_hotkeys_stopped", tray_warn_hotkeys_stopped),
        ("tray_warn_hotkey_change_failed", tray_warn_hotkey_change_failed),
        ("tray_warn_file_logging_failed", tray_warn_file_logging_failed),
        ("tray_usage_heading", tray_usage_heading),
        ("tray_usage_brighter", tray_usage_brighter), ("tray_usage_dimmer", tray_usage_dimmer),
        ("tray_menu_settings", tray_menu_settings),
        ("tray_menu_open_log_folder", tray_menu_open_log_folder),
        ("tray_menu_quit_fmt", tray_menu_quit_fmt), ("key_mod_ctrl", key_mod_ctrl),
        ("key_mod_alt", key_mod_alt), ("key_mod_shift", key_mod_shift),
        ("key_mod_win", key_mod_win), ("key_separator", key_separator),
        ("key_up", key_up), ("key_down", key_down), ("key_left", key_left),
        ("key_right", key_right), ("key_page_up", key_page_up), ("key_page_down", key_page_down),
        ("key_home", key_home), ("key_end", key_end), ("key_insert", key_insert),
        ("key_delete", key_delete), ("key_space", key_space), ("key_tab", key_tab),
        ("key_enter", key_enter), ("key_escape", key_escape), ("key_backspace", key_backspace),
        ("header_general", header_general), ("label_language", label_language),
        ("language_system_default", language_system_default),
        ("autostart", autostart),
        ("label_step", label_step), ("unit_percent_step", unit_percent_step),
        ("header_hotkeys", header_hotkeys), ("label_hotkey_up", label_hotkey_up),
        ("label_hotkey_down", label_hotkey_down), ("intercept", intercept),
        ("hint_intercept", hint_intercept), ("header_osd", header_osd),
        ("label_timeout", label_timeout), ("unit_milliseconds", unit_milliseconds),
        ("label_opacity", label_opacity), ("unit_percent_opacity", unit_percent_opacity),
        ("header_advanced", header_advanced), ("resync_check", resync_check),
        ("unit_seconds_resync", unit_seconds_resync), ("inactivity_check", inactivity_check),
        ("unit_seconds_inactivity", unit_seconds_inactivity), ("log_check", log_check),
        ("label_log_level", label_log_level), ("log_level_error", log_level_error),
        ("log_level_warn", log_level_warn), ("log_level_info", log_level_info),
        ("log_level_debug", log_level_debug), ("log_level_trace", log_level_trace),
        ("hint_logging", hint_logging), ("footer_links", footer_links),
        ("button_restore_defaults", button_restore_defaults), ("button_close", button_close),
        ("window_title", window_title), ("capture_prompt", capture_prompt),
        ("capture_reject_no_modifier", capture_reject_no_modifier),
        ("capture_reject_unnameable_key", capture_reject_unnameable_key),
        ("capture_reject_duplicate", capture_reject_duplicate),
        ("hotkey_status_unreachable", hotkey_status_unreachable),
        ("hotkey_status_no_response", hotkey_status_no_response),
        ("hotkey_status_unknown_error", hotkey_status_unknown_error),
        ("hotkey_status_restore_also_failed_fmt", hotkey_status_restore_also_failed_fmt),
        ("hotkey_notice_interception_unavailable", hotkey_notice_interception_unavailable),
        ("msgbox_already_running", msgbox_already_running),
        ("msgbox_title_startup_error", msgbox_title_startup_error),
        ("msgbox_title_hotkey_error", msgbox_title_hotkey_error),
        ("msgbox_title_autostart", msgbox_title_autostart),
        ("msgbox_title_restore_defaults", msgbox_title_restore_defaults),
        ("msgbox_startup_failed_lead", msgbox_startup_failed_lead),
        ("msgbox_thread_spawn_advice", msgbox_thread_spawn_advice),
        ("msgbox_hotkey_failed_lead", msgbox_hotkey_failed_lead),
        ("msgbox_hotkey_advice_fmt", msgbox_hotkey_advice_fmt),
        ("msgbox_autostart_failed_fmt", msgbox_autostart_failed_fmt),
        ("msgbox_restore_defaults_question", msgbox_restore_defaults_question),
        ("msgbox_config_file_fallback", msgbox_config_file_fallback),
    ]
}

#[test]
fn no_field_in_any_language_is_empty() {
    for &lang in Lang::ALL {
        for (name, value) in every_field(strings(lang)) {
            assert!(!value.is_empty(), "{lang:?} has an empty {name}");
        }
    }
}

#[test]
fn english_is_the_table_returned_for_english() {
    assert_eq!(strings(Lang::English).osd_ddc_error, ENGLISH.osd_ddc_error);
}

#[test]
fn the_quit_command_keeps_its_product_name_placeholder() {
    for &lang in Lang::ALL {
        assert!(
            strings(lang).tray_menu_quit_fmt.contains("{name}"),
            "{lang:?} dropped the {{name}} placeholder from the quit command"
        );
    }
}

#[test]
fn every_key_resolves_to_a_non_empty_string_in_every_language() {
    const KEYS: &[TextKey] = &[
        TextKey::HeaderGeneral,
        TextKey::LabelLanguage,
        TextKey::Autostart,
        TextKey::LabelStep,
        TextKey::UnitPercentStep,
        TextKey::HeaderHotkeys,
        TextKey::LabelHotkeyUp,
        TextKey::LabelHotkeyDown,
        TextKey::Intercept,
        TextKey::HintIntercept,
        TextKey::HeaderOsd,
        TextKey::LabelTimeout,
        TextKey::UnitMilliseconds,
        TextKey::LabelOpacity,
        TextKey::UnitPercentOpacity,
        TextKey::HeaderAdvanced,
        TextKey::ResyncCheck,
        TextKey::UnitSecondsResync,
        TextKey::InactivityCheck,
        TextKey::UnitSecondsInactivity,
        TextKey::LogCheck,
        TextKey::LabelLogLevel,
        TextKey::HintLogging,
        TextKey::FooterLinks,
        TextKey::ButtonRestoreDefaults,
        TextKey::ButtonClose,
    ];
    for &lang in Lang::ALL {
        let s = strings(lang);
        for &key in KEYS {
            assert!(!s.get(key).is_empty(), "{lang:?} has no text for {key:?}");
        }
    }
}

#[test]
fn english_key_names_equal_the_wire_names() {
    // In English the display name of every named key is its wire name,
    // which is what keeps `english_display_text_matches_the_stored_format`
    // in hotkey.rs true after key names are routed through this table.
    let s = strings(Lang::English);
    assert_eq!(s.key_up, "Up");
    assert_eq!(s.key_down, "Down");
    assert_eq!(s.key_left, "Left");
    assert_eq!(s.key_right, "Right");
    assert_eq!(s.key_page_up, "PageUp");
    assert_eq!(s.key_page_down, "PageDown");
    assert_eq!(s.key_home, "Home");
    assert_eq!(s.key_end, "End");
    assert_eq!(s.key_insert, "Insert");
    assert_eq!(s.key_delete, "Delete");
    assert_eq!(s.key_space, "Space");
    assert_eq!(s.key_tab, "Tab");
    assert_eq!(s.key_enter, "Enter");
    assert_eq!(s.key_escape, "Escape");
    assert_eq!(s.key_backspace, "Backspace");
    assert_eq!(s.label_language, "Language");
    assert_eq!(s.language_system_default, "System default");
}

#[test]
fn the_footer_link_row_keeps_both_link_spans() {
    for &lang in Lang::ALL {
        let text = strings(lang).get(TextKey::FooterLinks);
        assert_eq!(
            text.matches("<a>").count(),
            2,
            "{lang:?} footer must keep exactly two <a> spans, or SysLink indices shift"
        );
        assert_eq!(text.matches("</a>").count(), 2);
    }
}

#[test]
fn message_box_formats_keep_their_placeholders() {
    for &lang in Lang::ALL {
        let s = strings(lang);
        assert!(s.msgbox_hotkey_advice_fmt.contains("{path}"), "{lang:?}");
        assert!(
            s.msgbox_autostart_failed_fmt.contains("{error}"),
            "{lang:?}"
        );
    }
}

#[test]
fn the_rebind_restore_failure_keeps_both_error_placeholders() {
    for &lang in Lang::ALL {
        let text = strings(lang).hotkey_status_restore_also_failed_fmt;
        assert!(text.contains("{error}"), "{lang:?}");
        assert!(text.contains("{restore_error}"), "{lang:?}");
    }
}

#[test]
fn each_language_has_its_tag_and_native_name() {
    for (lang, tag, name) in [
        (Lang::German, "de", "Deutsch"),
        (Lang::English, "en", "English"),
        (Lang::French, "fr", "Français"),
        (Lang::Portuguese, "pt", "Português (Brasil)"),
        (Lang::Spanish, "es", "Español"),
        (Lang::Indonesian, "id", "Bahasa Indonesia"),
        (Lang::Russian, "ru", "Русский"),
        (Lang::Turkish, "tr", "Türkçe"),
        (Lang::Vietnamese, "vi", "Tiếng Việt"),
    ] {
        assert_eq!(lang.tag(), tag);
        assert_eq!(lang.native_name(), name);
    }
    assert_eq!(strings(Lang::German).button_close, "Schließen");
}

#[test]
fn all_is_sorted_by_native_name() {
    let names: Vec<String> = Lang::ALL
        .iter()
        .map(|lang| lang.native_name().to_lowercase())
        .collect();
    let mut sorted = names.clone();
    sorted.sort();
    assert_eq!(names, sorted, "Lang::ALL must stay in native-name order");
}

/// Fields a language deliberately leaves identical to English. A field that
/// matches English without being listed here is most likely one nobody
/// translated. The `match` has no wildcard arm, so a new language fails to
/// compile here until its list is written.
// One arm per shipped language; the list only grows as languages are added.
#[expect(clippy::too_many_lines)]
fn same_as_english(lang: Lang) -> &'static [&'static str] {
    match lang {
        Lang::English => &[],
        Lang::German => &[
            "header_hotkeys",
            "key_mod_alt",
            "key_mod_win",
            "key_separator",
            "key_tab",
            "unit_percent_step",
            "unit_milliseconds",
            "unit_percent_opacity",
            "unit_seconds_resync",
            "unit_seconds_inactivity",
            "msgbox_title_autostart",
        ],
        Lang::French => &[
            "key_mod_ctrl",
            "key_mod_alt",
            "key_mod_win",
            "key_separator",
            "key_tab",
            "unit_percent_step",
            "unit_milliseconds",
            "unit_percent_opacity",
            "unit_seconds_resync",
            "unit_seconds_inactivity",
        ],
        Lang::Portuguese | Lang::Vietnamese => &[
            "key_mod_ctrl",
            "key_mod_alt",
            "key_mod_shift",
            "key_mod_win",
            "key_separator",
            "key_home",
            "key_end",
            "key_insert",
            "key_delete",
            "key_tab",
            "key_enter",
            "key_backspace",
            "unit_percent_step",
            "unit_milliseconds",
            "unit_percent_opacity",
            "unit_seconds_resync",
            "unit_seconds_inactivity",
        ],
        Lang::Spanish => &[
            "header_general",
            "key_mod_ctrl",
            "key_mod_alt",
            "key_mod_win",
            "key_separator",
            "key_tab",
            "unit_percent_step",
            "unit_milliseconds",
            "unit_percent_opacity",
            "unit_seconds_resync",
            "unit_seconds_inactivity",
        ],
        Lang::Indonesian => &[
            "key_mod_ctrl",
            "key_mod_alt",
            "key_mod_shift",
            "key_mod_win",
            "key_separator",
            "key_page_up",
            "key_page_down",
            "key_home",
            "key_end",
            "key_insert",
            "key_delete",
            "key_tab",
            "key_enter",
            "key_backspace",
            "unit_percent_step",
            "unit_milliseconds",
            "unit_percent_opacity",
            "label_log_level",
        ],
        Lang::Russian => &[
            "key_mod_ctrl",
            "key_mod_alt",
            "key_mod_shift",
            "key_mod_win",
            "key_separator",
            "key_home",
            "key_end",
            "key_insert",
            "key_delete",
            "key_tab",
            "key_backspace",
            "unit_percent_step",
            "unit_percent_opacity",
        ],
        Lang::Turkish => &[
            "key_mod_ctrl",
            "key_mod_alt",
            "key_mod_shift",
            "key_mod_win",
            "key_separator",
            "key_home",
            "key_end",
            "key_insert",
            "key_delete",
            "key_enter",
            "unit_percent_step",
            "unit_milliseconds",
            "unit_percent_opacity",
        ],
    }
}

#[test]
fn a_field_equal_to_english_is_a_declared_decision() {
    let english: Vec<(&str, &str)> = every_field(&ENGLISH).into_iter().collect();
    let mut failures = Vec::new();
    for &lang in Lang::ALL {
        if lang == Lang::English {
            continue;
        }
        let declared = same_as_english(lang);
        for ((name, value), &(_, source)) in every_field(strings(lang)).into_iter().zip(&english) {
            // Config tokens, pinned to English by their own test.
            if name.starts_with("log_level_") {
                continue;
            }
            let identical = value == source;
            if identical != declared.contains(&name) {
                let relation = if identical {
                    "equals"
                } else {
                    "no longer equals"
                };
                failures.push(format!(
                    "{}: {name} {relation} English ({value:?})",
                    lang.tag()
                ));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn native_names_are_non_empty_and_unique() {
    let mut seen = Vec::new();
    for lang in Lang::ALL {
        let name = lang.native_name();
        assert!(!name.is_empty(), "{lang:?} has an empty native name");
        assert!(!seen.contains(&name), "duplicate native name {name}");
        seen.push(name);
    }
}

#[test]
fn index_round_trips_through_all() {
    for (i, &lang) in Lang::ALL.iter().enumerate() {
        assert_eq!(lang.index(), i);
        assert_eq!(Lang::from_index(i), Some(lang));
    }
    assert_eq!(Lang::from_index(Lang::ALL.len()), None);
}

#[test]
fn every_format_field_keeps_the_english_placeholders() {
    // Each `_fmt` field and the placeholders it must carry. A translation
    // that drops or misspells one would leave `{path}` literal on screen.
    type FormatCase = (
        &'static str,
        fn(&Strings) -> &'static str,
        &'static [&'static str],
    );
    let formats: &[FormatCase] = &[
        ("tray_menu_quit_fmt", |s| s.tray_menu_quit_fmt, &["{name}"]),
        (
            "hotkey_status_restore_also_failed_fmt",
            |s| s.hotkey_status_restore_also_failed_fmt,
            &["{error}", "{restore_error}"],
        ),
        (
            "msgbox_hotkey_advice_fmt",
            |s| s.msgbox_hotkey_advice_fmt,
            &["{path}"],
        ),
        (
            "msgbox_autostart_failed_fmt",
            |s| s.msgbox_autostart_failed_fmt,
            &["{error}"],
        ),
    ];
    for &lang in Lang::ALL {
        let s = strings(lang);
        for (name, get, placeholders) in formats {
            for placeholder in *placeholders {
                assert!(
                    get(s).contains(placeholder),
                    "{lang:?}: {name} lost {placeholder}"
                );
            }
        }
    }
}

#[test]
fn lookup_matches_whole_tags_case_insensitively() {
    assert_eq!(Lang::lookup("de"), Some(Lang::German));
    assert_eq!(Lang::lookup("DE"), Some(Lang::German));
    assert_eq!(Lang::lookup("en"), Some(Lang::English));
    assert_eq!(Lang::lookup("ja"), None);
    assert_eq!(Lang::lookup(""), None);
}

#[test]
fn lookup_truncates_subtags_from_the_right() {
    assert_eq!(Lang::lookup("de-AT"), Some(Lang::German));
    assert_eq!(Lang::lookup("de-AT-1901"), Some(Lang::German));
    assert_eq!(Lang::lookup("en-GB"), Some(Lang::English));
    assert_eq!(Lang::lookup("ja-JP"), None);
}

#[test]
fn lookup_drops_a_singleton_left_trailing_by_truncation() {
    // RFC 4647 §3.4: after removing the last subtag, a now-trailing
    // single-character subtag (an extension or private-use singleton)
    // is removed as well before the next comparison.
    assert_eq!(Lang::lookup("de-x-foo"), Some(Lang::German));
    assert_eq!(Lang::lookup("x-private"), None);
}

#[test]
fn from_preferences_takes_the_first_shipped_language() {
    let prefs = |tags: &[&str]| tags.iter().map(|t| (*t).to_string()).collect::<Vec<_>>();
    assert_eq!(Lang::from_preferences(&prefs(&["ja", "de"])), Lang::German);
    assert_eq!(
        Lang::from_preferences(&prefs(&["de-CH", "en"])),
        Lang::German
    );
    assert_eq!(Lang::from_preferences(&prefs(&["ja"])), Lang::English);
    assert_eq!(Lang::from_preferences(&[]), Lang::English);
}

#[test]
fn language_setting_parses_system_and_shipped_tags_only() {
    assert_eq!(
        LanguageSetting::parse("system"),
        Some(LanguageSetting::System)
    );
    assert_eq!(
        LanguageSetting::parse("SYSTEM"),
        Some(LanguageSetting::System)
    );
    assert_eq!(
        LanguageSetting::parse("de"),
        Some(LanguageSetting::Fixed(Lang::German))
    );
    assert_eq!(
        LanguageSetting::parse("de-CH"),
        Some(LanguageSetting::Fixed(Lang::German))
    );
    assert_eq!(LanguageSetting::parse("ja"), None);
    assert_eq!(LanguageSetting::parse("Deutsch"), None);
    assert_eq!(LanguageSetting::parse("de_DE"), None);
    assert_eq!(LanguageSetting::parse(""), None);
}

#[test]
fn language_setting_wire_round_trips() {
    assert_eq!(LanguageSetting::System.wire(), SYSTEM_LANGUAGE);
    assert_eq!(LanguageSetting::Fixed(Lang::German).wire(), "de");
    for &lang in Lang::ALL {
        let setting = LanguageSetting::Fixed(lang);
        assert_eq!(LanguageSetting::parse(setting.wire()), Some(setting));
    }
    assert_eq!(LanguageSetting::default(), LanguageSetting::System);
}

#[test]
fn language_setting_resolves_system_through_the_preferences() {
    let de = vec!["de-DE".to_string()];
    assert_eq!(LanguageSetting::System.resolve(&de), Lang::German);
    assert_eq!(
        LanguageSetting::Fixed(Lang::English).resolve(&de),
        Lang::English
    );
    assert_eq!(LanguageSetting::System.resolve(&[]), Lang::English);
}

#[test]
fn french_punctuation_keeps_its_no_break_space() {
    // French sets a no-break space before : ; ! ? so the mark never starts a
    // line on its own.
    let mut failures = Vec::new();
    for (name, value) in every_field(strings(Lang::French)) {
        let chars: Vec<char> = value.chars().collect();
        for (i, &c) in chars.iter().enumerate() {
            if matches!(c, ':' | ';' | '!' | '?')
                && !matches!(
                    i.checked_sub(1).map(|j| chars[j]),
                    Some('\u{a0}' | '\u{202f}')
                )
            {
                failures.push(format!("{name}: {c:?} at {i} in {value:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "\n{}", failures.join("\n"));
}

#[test]
fn log_level_entries_are_exactly_the_stored_tokens_in_every_language() {
    for &lang in Lang::ALL {
        let s = strings(lang);
        for (token, entry) in [
            ("error", s.log_level_error),
            ("warn", s.log_level_warn),
            ("info", s.log_level_info),
            ("debug", s.log_level_debug),
            ("trace", s.log_level_trace),
        ] {
            assert_eq!(
                entry,
                token,
                "{}'s log level entry must be the token that goes into config.json",
                lang.tag()
            );
        }
    }
}
