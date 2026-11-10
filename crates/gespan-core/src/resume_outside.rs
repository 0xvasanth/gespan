//! Continuing a Gespan session outside Gespan: the agent CLI's own command
//! that resumes it, quoted for a shell, and a system terminal that runs it.
use std::process::{Command, Stdio};

use crate::provider::Kind;
use crate::scm::shell::{apply_gui_env, resolve_gui_binary};

/// The shell a pasted line is quoted for: a POSIX shell on macOS and Linux,
/// `cmd.exe` on Windows (the dock's shell there, from `COMSPEC`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellStyle {
    Posix,
    Cmd,
}

impl ShellStyle {
    pub fn native() -> Self {
        if cfg!(windows) {
            Self::Cmd
        } else {
            Self::Posix
        }
    }
}

/// What a Claude Code session sets for the programs it starts. A Gespan
/// started from inside one hands them on to its terminals, and a `claude`
/// that sees them runs as a child session that saves no transcript.
pub const INHERITED_SESSION_MARKERS: &[&str] = &[
    "CLAUDECODE",
    "CLAUDE_CODE_CHILD_SESSION",
    "CLAUDE_CODE_SESSION_ID",
    "CLAUDE_CODE_ENTRYPOINT",
];

/// How a session continues outside Gespan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resume {
    /// The agent's CLI resumes the session with `program args`.
    Command {
        program: &'static str,
        args: Vec<String>,
    },
    /// The agent has its own session id, but no CLI command Gespan has
    /// checked takes it.
    IdOnly(String),
    /// The agent has not started the session yet: nothing to resume.
    NotStarted,
}

/// The way out for a session on `kind` whose agent id is `provider_id`.
/// Only commands checked against each CLI are offered; every other agent
/// gets its id to copy rather than a guess.
pub fn resume(kind: Kind, provider_id: Option<&str>) -> Resume {
    let Some(id) = provider_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Resume::NotStarted;
    };
    // The id comes from the agent's own output: anything but a plain token
    // (or one the CLI would read as a flag) is offered to copy, never run.
    if !plain_id(id) {
        return Resume::IdOnly(id.to_owned());
    }
    let flags: &[&str] = match kind {
        // Gespan starts Claude with `--session-id`, so its id is Claude's.
        Kind::Claude => &["--resume"],
        // The app-server thread id is the rollout's session id.
        Kind::Codex => &["resume"],
        // Cursor's ACP session id names its chat store (cursor_store.rs).
        Kind::Cursor => &["--resume"],
        Kind::Kiro => &["chat", "--resume-id"],
        Kind::OpenCode => &["--session"],
        // The flags Gespan itself resumes the Pi family with (pi.rs).
        Kind::Pi => &["--session"],
        Kind::Omp => &["--resume"],
        Kind::Grok | Kind::Antigravity | Kind::Fx | Kind::Hermes | Kind::Devin => {
            return Resume::IdOnly(id.to_owned());
        }
    };
    Resume::Command {
        program: kind.binary_name(),
        args: flags
            .iter()
            .map(|flag| (*flag).to_owned())
            .chain([id.to_owned()])
            .collect(),
    }
}

/// Session ids as the checked CLIs write them: UUIDs, `sess_…`, `ses_…`,
/// and the Pi family's session paths.
fn plain_id(id: &str) -> bool {
    id.len() <= 512
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._:/@+-~".contains(c))
}

impl Resume {
    /// The command alone, for a terminal already in the session's folder.
    /// `None` when there is no command or a word cannot be quoted safely.
    pub fn command_line(&self, style: ShellStyle) -> Option<String> {
        let Self::Command { program, args } = self else {
            return None;
        };
        let words = std::iter::once(*program)
            .chain(args.iter().map(String::as_str))
            .map(|word| quote(word, style))
            .collect::<Option<Vec<_>>>()?;
        Some(words.join(" "))
    }
    /// The command for a terminal Gespan opens, which shares Gespan's own
    /// environment: any inherited session marker `present` reports is
    /// cleared first, so the resumed session saves its turns.
    pub fn clean_command_line(
        &self,
        style: ShellStyle,
        present: impl Fn(&str) -> bool,
    ) -> Option<String> {
        let command = self.command_line(style)?;
        let markers: Vec<&str> = INHERITED_SESSION_MARKERS
            .iter()
            .copied()
            .filter(|name| present(name))
            .collect();
        if markers.is_empty() {
            return Some(command);
        }
        Some(match style {
            ShellStyle::Posix => {
                let unset: Vec<String> = markers.iter().map(|name| format!("-u {name}")).collect();
                format!("env {} {command}", unset.join(" "))
            }
            ShellStyle::Cmd => {
                let unset: Vec<String> = markers
                    .iter()
                    .map(|name| format!("set \"{name}=\""))
                    .collect();
                format!("{} && {command}", unset.join(" && "))
            }
        })
    }
    /// The line to paste in any terminal: into `cwd` first, since each CLI
    /// finds its sessions by folder, then the command.
    pub fn line(&self, cwd: &str, style: ShellStyle) -> Option<String> {
        let command = self.command_line(style)?;
        let cwd = quote(cwd, style)?;
        Some(match style {
            ShellStyle::Posix => format!("cd {cwd} && {command}"),
            ShellStyle::Cmd => format!("cd /d {cwd} && {command}"),
        })
    }
}

/// `word` as one shell word: bare when it is plainly safe, quoted otherwise,
/// and `None` when no quoting keeps it literal. Control characters are never
/// passed on; in `cmd`, `%` and `!` expand even inside quotes and `"` ends
/// them, so a word holding any of those gets no command.
pub fn quote(word: &str, style: ShellStyle) -> Option<String> {
    if word.chars().any(char::is_control) {
        return None;
    }
    Some(match style {
        ShellStyle::Posix => {
            let safe = !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
            if safe {
                word.to_owned()
            } else {
                format!("'{}'", word.replace('\'', r"'\''"))
            }
        }
        ShellStyle::Cmd => {
            let safe = !word.is_empty()
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || r"_@+=:,./\-".contains(c));
            if safe {
                word.to_owned()
            } else if word.contains(['%', '!', '"']) {
                return None;
            } else {
                format!("\"{word}\"")
            }
        }
    })
}

/// The operating systems a system terminal is opened on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Os {
    Mac,
    Windows,
    Linux,
}

impl Os {
    pub fn native() -> Self {
        if cfg!(target_os = "macos") {
            Self::Mac
        } else if cfg!(windows) {
            Self::Windows
        } else {
            Self::Linux
        }
    }
}

/// Linux terminals tried in order after `$TERMINAL`, each with the
/// arguments that make it run a program.
const LINUX_TERMINALS: &[(&str, &[&str])] = &[
    ("x-terminal-emulator", &["-e"]),
    ("gnome-terminal", &["--"]),
    ("konsole", &["-e"]),
    ("xfce4-terminal", &["-x"]),
    ("kitty", &[]),
    ("alacritty", &["-e"]),
    ("wezterm", &["start", "--"]),
    ("xterm", &["-e"]),
];

/// The program and arguments that open a new system terminal window in the
/// session's folder running the agent: `line` (which moves into the folder
/// first) on macOS and Linux, `command` alone on Windows, whose `start` opens
/// the window in the launcher's own folder instead, since `cmd /C` would
/// split a line at its `&&`. On Linux the first terminal `installed` finds
/// is used, `$TERMINAL` (`terminal_env`) first. The window stays open in a
/// shell once the agent exits.
pub fn terminal_launch(
    os: Os,
    line: &str,
    command: &str,
    terminal_env: Option<&str>,
    installed: impl Fn(&str) -> bool,
) -> Option<(String, Vec<String>)> {
    match os {
        Os::Mac => {
            let script = line.replace('\\', r"\\").replace('"', "\\\"");
            Some((
                "/usr/bin/osascript".into(),
                vec![
                    "-e".into(),
                    format!("tell application \"Terminal\" to do script \"{script}\""),
                    "-e".into(),
                    "tell application \"Terminal\" to activate".into(),
                ],
            ))
        }
        // `start` opens a new console window; `/K` keeps it open after.
        Os::Windows => Some((
            "cmd".into(),
            vec![
                "/C".into(),
                "start".into(),
                "\"\"".into(),
                "cmd".into(),
                "/K".into(),
                command.into(),
            ],
        )),
        Os::Linux => {
            let script = format!("{line}; exec \"${{SHELL:-sh}}\"");
            let from_env = terminal_env
                .map(str::trim)
                .filter(|name| !name.is_empty() && installed(name))
                .map(|name| (name, &["-e"][..]));
            let (program, flags) = from_env.or_else(|| {
                LINUX_TERMINALS
                    .iter()
                    .find(|(name, _)| installed(name))
                    .map(|(name, flags)| (*name, *flags))
            })?;
            let mut args: Vec<String> = flags.iter().map(|flag| (*flag).to_owned()).collect();
            args.extend(["sh".into(), "-c".into(), script]);
            Some((program.to_owned(), args))
        }
    }
}

/// Opens a new system terminal window in `cwd` running the agent; `line` and
/// `command` as [`terminal_launch`] takes them.
pub fn open_in_terminal(cwd: &str, line: &str, command: &str) -> Result<(), String> {
    let terminal_env = std::env::var("TERMINAL").ok();
    let (program, args) = terminal_launch(
        Os::native(),
        line,
        command,
        terminal_env.as_deref(),
        |name| resolve_gui_binary(name).is_some(),
    )
    .ok_or_else(|| "No terminal app found. Copy the command instead.".to_string())?;
    let mut command = Command::new(&program);
    command.current_dir(cwd);
    #[cfg(windows)]
    {
        // cmd parses its own command line; Rust's quoting would wrap the
        // already-quoted line again.
        use std::os::windows::process::CommandExt;
        command.raw_arg(args.join(" "));
    }
    #[cfg(not(windows))]
    command.args(&args);
    apply_gui_env(&mut command);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("Could not open a terminal: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "7f3c9a2e-41b8-4c0d-9e55-2a6f0c1d8b37";

    fn posix_line(kind: Kind) -> Option<String> {
        resume(kind, Some(ID)).line("/work/gespan", ShellStyle::Posix)
    }

    #[test]
    fn each_checked_agent_resumes_with_its_own_cli() {
        let cases = [
            (Kind::Claude, format!("claude --resume {ID}")),
            (Kind::Codex, format!("codex resume {ID}")),
            (Kind::Cursor, format!("cursor-agent --resume {ID}")),
            (Kind::Kiro, format!("kiro-cli chat --resume-id {ID}")),
            (Kind::OpenCode, format!("opencode --session {ID}")),
            (Kind::Pi, format!("pi --session {ID}")),
            (Kind::Omp, format!("omp --resume {ID}")),
        ];
        for (kind, command) in cases {
            assert_eq!(
                posix_line(kind),
                Some(format!("cd /work/gespan && {command}")),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn agents_without_a_checked_command_offer_only_their_id() {
        for kind in [
            Kind::Grok,
            Kind::Antigravity,
            Kind::Fx,
            Kind::Hermes,
            Kind::Devin,
        ] {
            let resume = resume(kind, Some(ID));
            assert_eq!(resume, Resume::IdOnly(ID.into()), "{kind:?}");
            assert_eq!(resume.line("/work", ShellStyle::Posix), None);
            assert_eq!(resume.command_line(ShellStyle::Cmd), None);
        }
    }

    #[test]
    fn every_agent_is_either_a_command_or_an_id() {
        for kind in Kind::ALL {
            assert!(
                !matches!(resume(kind, Some(ID)), Resume::NotStarted),
                "{kind:?}"
            );
        }
    }

    #[test]
    fn a_session_the_agent_never_started_has_nothing_to_resume() {
        // Gespan clears the id when the agent or folder changes, and a new
        // chat has none until its first turn.
        for id in [None, Some(""), Some("   ")] {
            assert_eq!(resume(Kind::Claude, id), Resume::NotStarted, "{id:?}");
        }
        assert_eq!(
            resume(Kind::Claude, None).line("/work", ShellStyle::Posix),
            None
        );
    }

    #[test]
    fn stray_whitespace_around_a_stored_id_is_dropped() {
        assert_eq!(
            resume(Kind::Claude, Some(&format!(" {ID}\n"))).command_line(ShellStyle::Posix),
            Some(format!("claude --resume {ID}"))
        );
    }

    #[test]
    fn an_id_that_reads_as_a_flag_is_never_put_in_a_command() {
        assert_eq!(
            resume(Kind::Codex, Some("--last")),
            Resume::IdOnly("--last".into())
        );
    }

    #[test]
    fn only_plain_ids_go_into_a_command() {
        // The id is the agent's own output; anything shell-like is copy-only.
        for id in [
            "x\"&calc&\"",
            "$(touch /tmp/x)",
            "a b",
            "%COMSPEC%",
            "id;rm",
            "a\nb",
        ] {
            assert_eq!(
                resume(Kind::Claude, Some(id)),
                Resume::IdOnly(id.into()),
                "{id:?}"
            );
        }
        // Real ids from each checked CLI stay commands.
        for id in [
            ID,
            "sess_d521fa54-b3a8-462d-b2ca-8d1b2e5d053c",
            "ses_3f2a9c",
            "/Users/me/.pi/sessions/2026-11-11.jsonl",
        ] {
            assert!(
                matches!(resume(Kind::Kiro, Some(id)), Resume::Command { .. }),
                "{id}"
            );
        }
    }

    #[test]
    fn folders_with_spaces_and_quotes_stay_one_word() {
        let resume = resume(Kind::Claude, Some(ID));
        assert_eq!(
            resume.line("/Users/me/My Projects/app", ShellStyle::Posix),
            Some(format!(
                "cd '/Users/me/My Projects/app' && claude --resume {ID}"
            ))
        );
        assert_eq!(
            resume.line("/work/it's mine", ShellStyle::Posix),
            Some(format!(r"cd '/work/it'\''s mine' && claude --resume {ID}"))
        );
        // `$`, backticks and `~` must reach `cd` literally, not expand.
        assert_eq!(
            resume.line("/work/$HOME `x` ~", ShellStyle::Posix),
            Some(format!("cd '/work/$HOME `x` ~' && claude --resume {ID}"))
        );
    }

    #[test]
    fn windows_lines_change_drive_and_quote_for_cmd() {
        let resume = resume(Kind::Codex, Some(ID));
        assert_eq!(
            resume.line(r"C:\work\gespan", ShellStyle::Cmd),
            Some(format!(r"cd /d C:\work\gespan && codex resume {ID}"))
        );
        assert_eq!(
            resume.line(r"D:\My Projects\a&b", ShellStyle::Cmd),
            Some(format!(
                r#"cd /d "D:\My Projects\a&b" && codex resume {ID}"#
            ))
        );
    }

    #[test]
    fn a_terminal_gespan_opens_clears_only_the_markers_it_inherited() {
        let resume = resume(Kind::Claude, Some(ID));
        let present = |name: &str| matches!(name, "CLAUDECODE" | "CLAUDE_CODE_CHILD_SESSION");
        assert_eq!(
            resume.clean_command_line(ShellStyle::Posix, present),
            Some(format!(
                "env -u CLAUDECODE -u CLAUDE_CODE_CHILD_SESSION claude --resume {ID}"
            ))
        );
        assert_eq!(
            resume.clean_command_line(ShellStyle::Cmd, present),
            Some(format!(
                r#"set "CLAUDECODE=" && set "CLAUDE_CODE_CHILD_SESSION=" && claude --resume {ID}"#
            ))
        );
    }

    #[test]
    fn without_inherited_markers_the_command_is_unchanged() {
        let resume = resume(Kind::Codex, Some(ID));
        assert_eq!(
            resume.clean_command_line(ShellStyle::Posix, |_| false),
            resume.command_line(ShellStyle::Posix)
        );
        // Nothing to run, nothing to clean.
        assert_eq!(
            Resume::NotStarted.clean_command_line(ShellStyle::Posix, |_| true),
            None
        );
    }

    #[test]
    fn quoting_handles_empty_and_odd_words() {
        assert_eq!(quote("", ShellStyle::Posix).as_deref(), Some("''"));
        assert_eq!(quote("", ShellStyle::Cmd).as_deref(), Some("\"\""));
        assert_eq!(
            quote("a;rm -rf /", ShellStyle::Posix).as_deref(),
            Some("'a;rm -rf /'")
        );
        // `&`, `|` and `^` are literal inside cmd's quotes.
        assert_eq!(
            quote("a&b|c^d", ShellStyle::Cmd).as_deref(),
            Some("\"a&b|c^d\"")
        );
    }

    #[test]
    fn words_no_quoting_keeps_literal_get_no_command() {
        // cmd expands %VAR% and !VAR! inside quotes, and a " ends them.
        for word in ["C:\\%PATH%", "C:\\a!b!", "a\"b"] {
            assert_eq!(quote(word, ShellStyle::Cmd), None, "{word}");
        }
        // A newline would run whatever follows it once pasted.
        for style in [ShellStyle::Posix, ShellStyle::Cmd] {
            assert_eq!(quote("/work/a\nrm -rf ~", style), None);
        }
        let resume = resume(Kind::Claude, Some(ID));
        assert_eq!(resume.line(r"C:\work\%USERNAME%", ShellStyle::Cmd), None);
        assert_eq!(resume.line("/work/a\nb", ShellStyle::Posix), None);
        // The same folder is fine where it is literal.
        assert!(resume.line("/work/100%", ShellStyle::Posix).is_some());
    }

    #[test]
    fn mac_opens_terminal_app_with_the_line_escaped_for_applescript() {
        let line = r#"cd '/work/a "b" \c' && claude --resume x"#;
        let (program, args) =
            terminal_launch(Os::Mac, line, "claude --resume x", None, |_| false).unwrap();
        assert_eq!(program, "/usr/bin/osascript");
        assert_eq!(
            args[1],
            r#"tell application "Terminal" to do script "cd '/work/a \"b\" \\c' && claude --resume x""#
        );
        assert_eq!(args[3], "tell application \"Terminal\" to activate");
    }

    #[test]
    fn windows_opens_a_console_in_the_folder_running_the_command_alone() {
        // The outer `cmd /C` would split a `cd … && …` line at its `&&`, so
        // the window starts in the folder and runs only the command.
        let (program, args) = terminal_launch(
            Os::Windows,
            "cd /d C:\\w && codex resume x",
            "codex resume x",
            None,
            |_| false,
        )
        .unwrap();
        assert_eq!(program, "cmd");
        assert_eq!(args.join(" "), r#"/C start "" cmd /K codex resume x"#);
        assert!(!args.iter().any(|a| a.contains("&&")));
    }

    #[test]
    fn linux_prefers_terminal_env_then_the_first_installed() {
        let line = "cd /w && claude --resume x";
        let script = format!("{line}; exec \"${{SHELL:-sh}}\"");
        let (program, args) =
            terminal_launch(Os::Linux, line, "claude --resume x", Some("foot"), |name| {
                name == "foot"
            })
            .unwrap();
        assert_eq!(program, "foot");
        assert_eq!(args, ["-e", "sh", "-c", script.as_str()]);

        // An uninstalled $TERMINAL falls through to the known list.
        let (program, args) =
            terminal_launch(Os::Linux, line, "claude --resume x", Some("nope"), |name| {
                matches!(name, "konsole" | "xterm")
            })
            .unwrap();
        assert_eq!(program, "konsole");
        assert_eq!(args[0], "-e");

        let (program, args) = terminal_launch(Os::Linux, line, "claude --resume x", None, |name| {
            name == "gnome-terminal"
        })
        .unwrap();
        assert_eq!(program, "gnome-terminal");
        assert_eq!(args[..3], ["--", "sh", "-c"]);
    }

    #[test]
    fn linux_without_any_terminal_has_no_launch() {
        assert_eq!(
            terminal_launch(Os::Linux, "x", "x", Some(""), |_| false),
            None
        );
    }
}
