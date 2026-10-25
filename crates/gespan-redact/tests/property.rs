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

/// Token shapes distinctive enough to catch even when glued after `_`.
fn distinctive_token() -> impl Strategy<Value = String> {
    prop_oneof![
        "gh[pousr]_[A-Za-z0-9]{20,40}",
        "(AKIA|ASIA)[0-9A-Z]{16,20}",
        "glpat-[A-Za-z0-9_-]{20,40}",
        "AIza[0-9A-Za-z_-]{35}",
    ]
}

fn token() -> impl Strategy<Value = String> {
    prop_oneof![
        "sk-[A-Za-z0-9_-]{20,40}",
        "sk-ant-api03-[A-Za-z0-9_-]{20,60}",
        "(sk|rk)_(live|test)_[A-Za-z0-9]{16,40}",
        "npm_[A-Za-z0-9]{36}",
        "hf_[A-Za-z0-9]{30,40}",
        "xapp-[0-9]-[A-Za-z0-9-]{10,40}",
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
    fn distinctive_tokens_never_survive_even_glued(
        prefix in "[A-Za-z0-9 ]{0,10}", glue in prop::sample::select(vec!["_", "/", "=", ":", " "]),
        token in distinctive_token(), suffix in any::<String>(),
    ) {
        let input = format!("{prefix}{glue}{token} {suffix}");
        let out = Redactor::new(Config::default()).redact_text(&input);
        prop_assert!(!out.contains(&token), "{token:?} survived in {out:?}");
    }

    #[test]
    fn secret_values_with_a_scheme_never_survive(
        key in "(authorization|Authorization|auth_token|proxy_authorization)",
        scheme in "(Bearer|Basic|token)", value in "[A-Za-z0-9+/=._-]{8,40}",
        sep in prop::sample::select(vec!["=", ": "]),
    ) {
        let input = format!("x {key}{sep}{scheme} {value} y");
        let out = Redactor::new(Config::default()).redact_text(&input);
        prop_assert!(!out.contains(&value), "{value:?} survived in {out:?}");
    }

    #[test]
    fn quoted_secret_values_never_survive(
        key in "[a-z_]{0,6}(password|token|secret|api_key)",
        value in "[A-Za-z0-9 !#$%&()*+,./:;<=>?@^_~-]{1,40}",
        form in 0..3usize, suffix in any::<String>(),
    ) {
        let pair = match form {
            0 => format!("{key}=\"{value}\""),
            1 => format!("{key}: \"{value}\""),
            _ => format!("\"{key}\": \"{value}\""),
        };
        let input = format!("{pair} {suffix}");
        let out = Redactor::new(Config::default()).redact_text(&input);
        prop_assert!(!out.contains(&pair), "{pair:?} survived in {out:?}");
        prop_assert!(value.trim().len() < 3 || !out.contains(value.trim()) || suffix.contains(value.trim()),
            "{value:?} survived in {out:?}");
    }

    #[test]
    fn scrubbing_twice_changes_nothing(
        parts in prop::collection::vec(prop_oneof![
            any::<String>(),
            token(),
            Just("/Users/alice/code/app/src/a.rs".to_string()),
            Just("C:\\Users\\Alice\\x".to_string()),
            Just("token=".to_string()),
            Just("password: ".to_string()),
            Just("\"prompt\":".to_string()),
            Just("Bearer ".to_string()),
            Just("bob@example.com".to_string()),
            Just("https://u:".to_string()),
            Just("@host/".to_string()),
            Just("\"".to_string()),
            Just("\\\"".to_string()),
            Just("{".to_string()),
        ], 1..12),
    ) {
        let r = Redactor::new(Config {
            home: Some("/Users/alice".into()),
            projects: vec!["/Users/alice/code/app".into()],
            secrets: vec!["kc-Secret-Value-123".into()],
        });
        let once = r.redact_text(&parts.concat());
        prop_assert_eq!(r.redact_text(&once), once.clone());
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
