//! No input that contains a token survives scrubbing, whatever surrounds it.

use gespan_redact::{Config, Redactor, DENY_LIST};
use proptest::prelude::*;
use serde_json::json;

/// Characters that end a word and are not part of any token shape.
fn separator() -> impl Strategy<Value = &'static str> {
    prop::sample::select(vec![
        " ", "\n", "\t", "(", ")", "\"", "'", ",", ";", "[", "|",
    ])
}

fn token() -> impl Strategy<Value = String> {
    prop_oneof![
        "sk-[A-Za-z0-9_-]{20,40}",
        "sk-ant-api03-[A-Za-z0-9_-]{20,60}",
        "gh[pousr]_[A-Za-z0-9]{20,40}",
        "github_pat_[A-Za-z0-9_]{20,60}",
        "xox[abposr]-[A-Za-z0-9-]{10,40}",
        "(AKIA|ASIA)[0-9A-Z]{16,20}",
        "eyJ[A-Za-z0-9_-]{5,30}\\.[A-Za-z0-9_-]{5,30}\\.[A-Za-z0-9_-]{0,40}",
    ]
}

fn surround(prefix: &str, open: &str, middle: &str, close: &str, suffix: &str) -> String {
    format!("{prefix}{open}{middle}{close}{suffix}")
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn tokens_never_survive(
        prefix in any::<String>(), open in separator(), token in token(),
        close in separator(), suffix in any::<String>(),
    ) {
        let input = surround(&prefix, open, &token, close, &suffix);
        let out = Redactor::new(Config::default()).redact_text(&input);
        prop_assert!(!out.contains(&token), "{token:?} survived in {out:?}");
        prop_assert!(out.contains("<redacted>"));
    }

    #[test]
    fn bearer_values_never_survive(
        prefix in any::<String>(), open in separator(),
        value in "[A-Za-z0-9._~+/=-]{8,40}", close in separator(), suffix in any::<String>(),
    ) {
        let input = surround(&prefix, open, &format!("Bearer {value}"), close, &suffix);
        let out = Redactor::new(Config::default()).redact_text(&input);
        prop_assert!(!out.contains(&format!("Bearer {value}")), "survived in {out:?}");
    }

    #[test]
    fn secret_key_values_never_survive(
        prefix in any::<String>(), open in separator(),
        key in "[A-Za-z0-9_]{0,8}(password|passwd|token|secret|api_key|apiKey|API-KEY)",
        value in "[A-Za-z0-9!#$%()*+./:=?@_~-]{1,40}", close in separator(), suffix in any::<String>(),
    ) {
        let pair = format!("{key}={value}");
        let input = surround(&prefix, open, &pair, close, &suffix);
        let out = Redactor::new(Config::default()).redact_text(&input);
        prop_assert!(!out.contains(&pair), "{pair:?} survived in {out:?}");
        prop_assert!(out.contains(&format!("{key}=<redacted>")), "{out:?}");
    }

    #[test]
    fn deny_listed_fields_are_always_dropped(
        index in 0..DENY_LIST.len(), upper in any::<bool>(), value in any::<String>(),
    ) {
        let key = if upper { DENY_LIST[index].to_uppercase() } else { DENY_LIST[index].to_string() };
        let input = json!({"events": [{key: value, "keep": 1}], "level": "info"});
        let out = Redactor::new(Config::default()).redact_value(input);
        prop_assert_eq!(out, json!({"events": [{"keep": 1}], "level": "info"}));
    }
}
