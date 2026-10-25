//! Scrub diagnostics before anyone sees or sends them. Shared by the Tauri app
//! and the native app, so both follow the same rules and the same fixtures
//! (`fixtures/redaction.json`).
//!
//! The rules: the home directory becomes `~`; each project root becomes
//! `<project-N>`, keeping the path inside it; emails become `<email>`; tokens
//! and known secrets become `<redacted>`; deny-listed JSON fields are dropped.

mod fields;
#[cfg(feature = "fixtures")]
pub mod fixtures;
mod pairs;
mod paths;
mod patterns;
mod value;

use std::path::PathBuf;

use aho_corasick::{AhoCorasick, MatchKind};

use paths::PathRule;
pub use value::DENY_LIST;

/// Replaces the home directory.
pub const HOME: &str = "~";
/// Replaces an email address.
pub const EMAIL: &str = "<email>";
/// Replaces a token, key or secret value.
pub const REDACTED: &str = "<redacted>";

/// Extra passes `redact_text` may take to reach stable output.
const MAX_PASSES: usize = 4;

/// What the redactor knows about this machine.
#[derive(Clone, Debug, Default)]
pub struct Config {
    /// The user's home directory.
    pub home: Option<PathBuf>,
    /// Project roots, in a stable order: the first is `<project-1>`.
    pub projects: Vec<PathBuf>,
    /// Secret values known to this machine (keychain, account store). Each
    /// is replaced wherever it appears; values under 8 characters are
    /// ignored so a short one can't shred ordinary text.
    pub secrets: Vec<String>,
}

/// Scrubs text and JSON. Build it once and reuse it.
#[derive(Debug)]
pub struct Redactor {
    /// Project roots longest first, then home, so a project under home wins.
    paths: Vec<PathRule>,
    /// Known secret values in every encoding, longest match first.
    secrets: Option<AhoCorasick>,
}

impl Redactor {
    pub fn new(config: Config) -> Self {
        let mut paths: Vec<PathRule> = config
            .projects
            .iter()
            .enumerate()
            .filter_map(|(i, root)| PathRule::new(root, format!("<project-{}>", i + 1)))
            .collect();
        // Longest root first, so a nested project beats the one around it.
        paths.sort_by_key(|rule| std::cmp::Reverse(rule.len));
        paths.extend(
            config
                .home
                .as_deref()
                .and_then(|home| PathRule::new(home, HOME.to_string())),
        );
        // A secret that is part of a placeholder would be found again inside
        // its own replacement on every pass.
        let placeholders: Vec<String> = [HOME.to_string(), EMAIL.to_string(), REDACTED.to_string()]
            .into_iter()
            .chain((1..=config.projects.len()).map(|n| format!("<project-{n}>")))
            .collect();
        // Each secret as written, JSON-escaped and URL-encoded every common way.
        let mut variants: Vec<String> = config
            .secrets
            .iter()
            .filter(|secret| secret.chars().count() >= 8)
            .filter(|secret| !placeholders.iter().any(|p| p.contains(secret.as_str())))
            .flat_map(|secret| {
                [
                    secret.clone(),
                    json_escaped(secret),
                    percent_encoded(secret, false, false),
                    percent_encoded(secret, true, false),
                    percent_encoded(secret, false, true),
                    percent_encoded(secret, true, true),
                ]
            })
            .collect();
        variants.sort();
        variants.dedup();
        let secrets = (!variants.is_empty()).then(|| {
            AhoCorasick::builder()
                .match_kind(MatchKind::LeftmostLongest)
                .build(&variants)
                .expect("literal secrets build an automaton")
        });
        Self { paths, secrets }
    }

    /// Scrubs free text: a log message, a panic, a stack trace. The output
    /// is already scrubbed: scrubbing it again changes nothing.
    pub fn redact_text(&self, input: &str) -> String {
        // One rule's placeholder can expose a match for another (a URL
        // password inside a `password:` value), so repeat until stable.
        let mut out = self.redact_once(input);
        for _ in 0..MAX_PASSES {
            let next = self.redact_once(&out);
            if next == out {
                break;
            }
            out = next;
        }
        out
    }

    fn redact_once(&self, input: &str) -> String {
        let mut out = fields::blank_denied_fields(input);
        if let Some(secrets) = &self.secrets {
            let mut replaced = String::with_capacity(out.len());
            secrets.replace_all_with(&out, &mut replaced, |_, _, dst| {
                dst.push_str(REDACTED);
                true
            });
            out = replaced;
        }
        for rule in &self.paths {
            out = rule.apply(&out);
        }
        patterns::redact_emails(&patterns::redact_tokens(&out))
    }
}

fn json_escaped(text: &str) -> String {
    let quoted = serde_json::to_string(text).expect("a string serializes");
    quoted[1..quoted.len() - 1].to_string()
}

/// URL encoding in upper- or lower-case hex, with spaces as `%20` or (form
/// encoding) `+`.
fn percent_encoded(text: &str, lower: bool, plus: bool) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (byte as char).to_string()
            }
            b' ' if plus => "+".to_string(),
            _ if lower => format!("%{byte:02x}"),
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redactor(home: &str, projects: &[&str]) -> Redactor {
        Redactor::new(Config {
            home: Some(home.into()),
            projects: projects.iter().map(PathBuf::from).collect(),
            ..Config::default()
        })
    }

    #[test]
    fn known_secrets_are_replaced_and_short_ones_ignored() {
        let r = Redactor::new(Config {
            secrets: vec!["hunter2hunter2".into(), "abcd".into(), String::new()],
            ..Config::default()
        });
        assert_eq!(
            r.redact_text("login hunter2hunter2 failed for abcd"),
            "login <redacted> failed for abcd"
        );
    }

    #[test]
    fn known_secrets_are_replaced_before_paths_and_in_encoded_forms() {
        let r = Redactor::new(Config {
            home: Some("/Users/alice".into()),
            secrets: vec!["x9/Users/alice/k".into(), "p@ss w0rd\"x".into()],
            ..Config::default()
        });
        assert_eq!(r.redact_text("pw x9/Users/alice/k"), "pw <redacted>");
        assert_eq!(
            r.redact_text(r#"{"v":"p@ss w0rd\"x"}"#),
            r#"{"v":"<redacted>"}"#
        );
        assert_eq!(
            r.redact_text("?pw=p%40ss%20w0rd%22x&a=1"),
            "?pw=<redacted>&a=1"
        );
    }

    #[test]
    fn known_secrets_in_lower_case_and_form_encoding() {
        let r = Redactor::new(Config {
            secrets: vec!["p@ss w0rd/x".into()],
            ..Config::default()
        });
        assert_eq!(r.redact_text("a=p%40ss%20w0rd%2fx"), "a=<redacted>");
        assert_eq!(r.redact_text("a=p%40ss+w0rd%2Fx"), "a=<redacted>");
    }

    #[test]
    fn deny_listed_fields_inside_text_are_blanked() {
        let r = Redactor::new(Config::default());
        assert_eq!(
            r.redact_text(
                r#"{"level":"info","fields":{"prompt":"my private prompt","env":{"A":"b}"},"n":1}}"#
            ),
            r#"{"level":"info","fields":{"prompt":"<redacted>","env":"<redacted>","n":1}}"#
        );
        // The first line of a log tail is usually cut short.
        assert_eq!(
            r.redact_text(r#"{"prompt":"my private prompt cut"#),
            r#"{"prompt":"<redacted>""#
        );
    }

    #[test]
    fn dotted_user_names_are_other_users() {
        let r = redactor("/Users/al", &[]);
        assert_eq!(
            r.redact_text("/Users/al.smith/Documents/x"),
            "/Users/al.smith/Documents/x"
        );
        assert_eq!(r.redact_text("saved to /Users/al."), "saved to ~.");
    }

    #[test]
    fn output_is_already_scrubbed() {
        // The URL rule's output exposes a `password:` value the colon rule
        // had passed over; one more pass would change it.
        let r = Redactor::new(Config::default());
        let once = r.redact_text(
            "password: https://u:token=\u{3a3}\"bob@example.comsk-_aaaa_0-A0_AaA0AaAA0",
        );
        assert_eq!(r.redact_text(&once), once);
    }

    #[test]
    fn repeated_counter_keys_stay_linear() {
        // Each rejected key used to re-read the same long value: 48 KB of
        // `tokens_used=` took 13.7 s in a release build.
        let r = Redactor::new(Config::default());
        for input in [
            "tokens_used=".repeat(4000),
            "token_count:".repeat(4000),
            "input_tokens=1,output_tokens=2,".repeat(1600),
            "token: ".repeat(6000),
        ] {
            let started = std::time::Instant::now();
            let out = r.redact_text(&input);
            assert!(out.len() <= input.len() + 16);
            assert!(
                started.elapsed() < std::time::Duration::from_secs(2),
                "{:?} took {:?}",
                &input[..12],
                started.elapsed()
            );
        }
    }

    #[test]
    fn cut_off_deny_listed_values_stop_at_the_end_of_the_line() {
        let r = Redactor::new(Config::default());
        assert_eq!(
            r.redact_text("{\"env\":{\"A\":\"b\n{\"level\":\"info\",\"msg\":\"keep me\"}\n"),
            "{\"env\":\"<redacted>\"\n{\"level\":\"info\",\"msg\":\"keep me\"}\n"
        );
        assert_eq!(
            r.redact_text("{\"prompt\":\"cut\n{\"level\":\"info\",\"msg\":\"keep\"}"),
            "{\"prompt\":\"<redacted>\"\n{\"level\":\"info\",\"msg\":\"keep\"}"
        );
        // A complete value may still span lines.
        assert_eq!(
            r.redact_text("{\"env\": {\n  \"A\": \"b\"\n}, \"n\": 1}"),
            "{\"env\": \"<redacted>\", \"n\": 1}"
        );
    }

    #[test]
    fn known_secrets_inside_placeholders_are_ignored() {
        let r = Redactor::new(Config {
            projects: vec!["/w/app".into()],
            secrets: vec![
                "redacted".into(),
                "project-1".into(),
                "kc-Secret-Value-123".into(),
            ],
            ..Config::default()
        });
        let once = r.redact_text("x redacted y kc-Secret-Value-123 /w/app/a");
        assert_eq!(once, "x redacted y <redacted> <project-1>/a");
        assert_eq!(r.redact_text(&once), once);
    }

    #[test]
    fn the_longest_known_secret_wins() {
        let r = Redactor::new(Config {
            secrets: vec!["abcdefgh".into(), "abcdefghijkl".into()],
            ..Config::default()
        });
        assert_eq!(
            r.redact_text("[abcdefghijkl] [abcdefgh]"),
            "[<redacted>] [<redacted>]"
        );
    }

    #[test]
    fn every_rule_applies_to_one_line() {
        let r = redactor("/Users/alice", &["/Users/alice/me@work.dev"]);
        assert_eq!(
            r.redact_text(
                "push /Users/alice/me@work.dev/a.rs as bob@x.io with ghp_16C7e42F292c6912E7710c838347Ae178B4a from /Users/alice/tmp"
            ),
            "push <project-1>/a.rs as <email> with <redacted> from ~/tmp"
        );
    }

    #[test]
    fn home_becomes_tilde_on_macos() {
        let r = redactor("/Users/alice", &[]);
        assert_eq!(
            r.redact_text("wrote /Users/alice/Library/Logs/x.jsonl"),
            "wrote ~/Library/Logs/x.jsonl"
        );
    }

    #[test]
    fn home_becomes_tilde_on_linux_at_end_of_text() {
        let r = redactor("/home/alice/", &[]);
        assert_eq!(r.redact_text("cwd=/home/alice"), "cwd=~");
        assert_eq!(
            r.redact_text("a /home/alice/.local/state b /home/alice/x"),
            "a ~/.local/state b ~/x"
        );
    }

    #[test]
    fn home_on_windows_ignores_case_and_separator_style() {
        let r = redactor(r"C:\Users\Alice", &[]);
        assert_eq!(r.redact_text(r"c:/users/ALICE\AppData\x"), r"~\AppData\x");
        // JSON-escaped text doubles every backslash.
        assert_eq!(
            r.redact_text(r#"{"path":"C:\\Users\\Alice\\x.log"}"#),
            r#"{"path":"~\\x.log"}"#
        );
    }

    #[test]
    fn home_that_prefixes_another_user_is_left_alone() {
        let r = redactor("/Users/al", &[]);
        assert_eq!(r.redact_text("/Users/alice/x"), "/Users/alice/x");
        assert_eq!(
            r.redact_text("/Users/al_2/x /Users/al-b"),
            "/Users/al_2/x /Users/al-b"
        );
        assert_eq!(r.redact_text("/Users/al/x"), "~/x");
    }

    #[test]
    fn project_root_keeps_the_path_inside_it() {
        let r = redactor("/Users/alice", &["/Users/alice/code/app"]);
        assert_eq!(
            r.redact_text("error in /Users/alice/code/app/src/main.rs:12"),
            "error in <project-1>/src/main.rs:12"
        );
        assert_eq!(
            r.redact_text("/Users/alice/code/other/a.rs"),
            "~/code/other/a.rs"
        );
    }

    #[test]
    fn nested_projects_match_longest_root_first_and_keep_list_numbers() {
        let r = redactor("/home/a", &["/w/app", "/w/app/sub"]);
        assert_eq!(r.redact_text("/w/app/sub/a.rs"), "<project-2>/a.rs");
        assert_eq!(r.redact_text("/w/app/b.rs"), "<project-1>/b.rs");
        assert_eq!(r.redact_text("/w/application/b.rs"), "/w/application/b.rs");
    }

    #[test]
    fn root_and_empty_paths_are_ignored() {
        let r = redactor("/", &["", r"C:\"]);
        assert_eq!(r.redact_text("/etc/hosts C:\\x"), "/etc/hosts C:\\x");
    }

    #[test]
    fn unicode_paths() {
        let r = redactor("/Users/zoë", &["/Users/zoë/项目/日本"]);
        assert_eq!(
            r.redact_text("/Users/zoë/项目/日本/ファイル.rs and /Users/zoë/Música"),
            "<project-1>/ファイル.rs and ~/Música"
        );
        // A letter outside ASCII still counts as part of the name.
        assert_eq!(r.redact_text("/Users/zoëy/x"), "/Users/zoëy/x");
    }
}
