//! Scrub diagnostics before anyone sees or sends them. Shared by the Tauri app
//! and the native app, so both follow the same rules and the same fixtures
//! (`fixtures/redaction.json`).
//!
//! The rules: the home directory becomes `~`; each project root becomes
//! `<project-N>`, keeping the path inside it; emails become `<email>`; tokens
//! and known secrets become `<redacted>`; deny-listed JSON fields are dropped.

mod paths;
mod patterns;

use std::path::PathBuf;

use paths::PathRule;

/// Replaces the home directory.
pub const HOME: &str = "~";
/// Replaces an email address.
pub const EMAIL: &str = "<email>";
/// Replaces a token, key or secret value.
pub const REDACTED: &str = "<redacted>";

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
    /// Known secret values, longest first.
    secrets: Vec<String>,
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
        let mut secrets: Vec<String> = config
            .secrets
            .into_iter()
            .filter(|secret| secret.chars().count() >= 8)
            .collect();
        secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
        Self { paths, secrets }
    }

    /// Scrubs free text: a log message, a panic, a stack trace.
    pub fn redact_text(&self, input: &str) -> String {
        let mut out = input.to_string();
        for rule in &self.paths {
            out = rule.apply(&out);
        }
        for secret in &self.secrets {
            out = out.replace(secret.as_str(), REDACTED);
        }
        patterns::redact_emails(&patterns::redact_tokens(&out))
    }
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
