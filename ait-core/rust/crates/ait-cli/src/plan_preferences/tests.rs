use super::*;

#[test]
fn language_tags_preserve_script_and_region_without_names_or_inference() {
    for (input, expected) in [
        ("ZH-tw", "zh-TW"),
        ("zh-hANT-tw", "zh-Hant-TW"),
        ("zh-Hans", "zh-Hans"),
        ("zh-HK", "zh-HK"),
        ("zh", "zh"),
        ("EN", "en"),
        ("en-us", "en-US"),
        ("ja-JP", "ja-JP"),
        ("ko-KR", "ko-KR"),
        ("es-419", "es-419"),
        ("fil", "fil"),
    ] {
        assert_eq!(normalize_language(input).unwrap(), expected, "{input}");
    }
    for input in [
        "",
        " ",
        " en",
        "en ",
        "a",
        "auto",
        "中文",
        "zh_TW",
        "en.UTF-8",
        "en-",
        "en--US",
        "en-US-Latn",
        "en-Latn-US-extra",
        "en-x-test",
        "en-u-ca-gregory",
        "en-1234",
        "12",
        "en-１２３",
        "en-\nUS",
        "en`",
        "en-🇺🇸",
    ] {
        assert!(normalize_language(input).is_err(), "{input:?}");
    }
}

#[test]
fn explicit_preferences_are_strict_and_missing_values_have_stable_sources() {
    let defaults = PlanPreferences::from_config(&JsonMap::new()).unwrap();
    assert_eq!(
        defaults.language_summary(),
        json!({"value":"en","source":"built_in"})
    );
    assert_eq!(
        defaults.style_summary(),
        json!({"value":"concise","source":"built_in"})
    );
    for key in ["plan_language", "plan_style"] {
        for value in [
            JsonValue::Null,
            json!(3),
            json!(false),
            json!(""),
            json!("auto"),
        ] {
            let mut config = JsonMap::new();
            config.insert(key.to_string(), value);
            assert!(PlanPreferences::from_config(&config).is_err(), "{key}");
        }
    }
    assert!(normalize_style("Concise").is_err());
    assert!(normalize_style(" detailed").is_err());
}

#[test]
fn os_locale_normalization_keeps_script_semantics() {
    for (input, expected) in [
        ("zh_TW.UTF-8", Some("zh-TW")),
        ("ja_JP.utf8", Some("ja-JP")),
        ("sr_RS@latin", Some("sr-Latn-RS")),
        ("sr@cyrillic", Some("sr-Cyrl")),
        ("sr_Latn_RS.UTF-8@latin", Some("sr-Latn-RS")),
        ("sr_Latn_RS@cyrillic", None),
        ("sr_RS@unknown", None),
        ("C", Some("en")),
        ("C.UTF-8", Some("en")),
        ("POSIX", Some("en")),
        ("", None),
        ("not_a_locale", None),
    ] {
        assert_eq!(normalize_os_locale(input).as_deref(), expected, "{input}");
    }
}

fn linux(vars: &[(&str, &str)]) -> String {
    linux_language(|key| {
        vars.iter()
            .find(|(name, _)| *name == key)
            .map(|(_, value)| value.to_string())
    })
}

#[test]
fn linux_selection_honors_message_precedence_language_lists_and_c_locale() {
    assert_eq!(linux(&[]), "en");
    assert_eq!(linux(&[("LANG", "zh_TW.UTF-8")]), "zh-TW");
    assert_eq!(
        linux(&[
            ("LANG", "en_US.UTF-8"),
            ("LC_MESSAGES", "ja_JP"),
            ("LC_ALL", "")
        ]),
        "ja-JP"
    );
    assert_eq!(
        linux(&[
            ("LANG", "en_US"),
            ("LC_MESSAGES", "ja_JP"),
            ("LC_ALL", "ko_KR")
        ]),
        "ko-KR"
    );
    assert_eq!(
        linux(&[
            ("LANG", "en_US"),
            ("LANGUAGE", ":bad_value_here:zh_Hant_TW:ko_KR")
        ]),
        "zh-Hant-TW"
    );
    assert_eq!(
        linux(&[("LANG", "ja_JP"), ("LANGUAGE", "bad_value_here:")]),
        "ja-JP"
    );
    assert_eq!(linux(&[("LANG", "ja_JP"), ("LANGUAGE", "C:ko_KR")]), "en");
    for value in ["C", "C.UTF-8", "POSIX", "POSIX.UTF-8", "invalid value"] {
        assert_eq!(
            linux(&[("LC_ALL", value), ("LANG", "ja_JP"), ("LANGUAGE", "ko_KR")]),
            "en"
        );
    }
    assert_eq!(linux(&[("LANGUAGE", "ko_KR")]), "en");
}

#[test]
fn native_language_lists_skip_unusable_entries_and_preserve_priority() {
    let values = ["???", "ko-KR", "ja-JP"].map(String::from);
    assert_eq!(preferred_language(&values), "ko-KR");
    assert_eq!(preferred_language(&[]), "en");
    assert_eq!(preferred_language(&["???".into()]), "en");
    let utf16: Vec<_> = "ja-JP\0ko-KR\0\0".encode_utf16().collect();
    assert_eq!(decode_windows_languages(&utf16), ["ja-JP", "ko-KR"]);
    assert!(decode_windows_languages(&utf16[..utf16.len() - 1]).is_empty());
    assert_eq!(
        decode_windows_languages(&[0xd800, 0, b'e' as u16, b'n' as u16, 0, 0]),
        ["en"]
    );
}

#[test]
fn all_languages_and_styles_share_identical_english_guidance() {
    let mut body = None;
    for language in [
        "zh-TW",
        "zh-CN",
        "zh-Hant-TW",
        "en",
        "ja-JP",
        "ko-KR",
        "fr-FR",
    ] {
        for style in ["concise", "detailed"] {
            let config = json!({"plan_language":language,"plan_style":style});
            let guidance = PlanPreferences::from_config(config.as_object().unwrap())
                .unwrap()
                .guidance();
            assert!(guidance.is_ascii());
            assert_eq!(
                guidance.lines().next().unwrap(),
                format!("Plan prose defaults: language=`{language}` (BCP 47); style=`{style}`.")
            );
            let rest = guidance.split_once('\n').unwrap().1.to_string();
            if let Some(expected) = &body {
                assert_eq!(&rest, expected);
            } else {
                body = Some(rest);
            }
        }
    }
}

#[test]
fn native_locale_source_smoke() {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    eprintln!(
        "Native preferred language API: {:?}",
        native_preferred_languages()
    );
    let detected = detect_language();
    eprintln!("Detected Plan language: {detected}");
    assert_eq!(normalize_language(&detected).unwrap(), detected);
}
