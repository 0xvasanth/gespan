use super::Profile;
use crate::{
    process,
    provider::{self, Kind},
};
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AuthAction {
    SignIn,
    SignOut,
}
pub fn arguments(kind: Kind, action: AuthAction) -> Option<&'static [&'static str]> {
    let login = kind.spec().login?;
    match action {
        AuthAction::SignIn => Some(login.sign_in),
        AuthAction::SignOut => login.sign_out,
    }
}
/// What a running sign-in asks of the user: the link the CLI printed in case
/// its browser did not open, a one-time code to confirm there, and whether it
/// waits for a code pasted back from the provider's page.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LoginPrompt {
    pub url: Option<String>,
    pub code: Option<String>,
    pub wants_code: bool,
}
impl LoginPrompt {
    /// Reads the CLI's output so far.
    pub fn read(output: &str) -> Self {
        static ANSI: OnceLock<regex::Regex> = OnceLock::new();
        static URL: OnceLock<regex::Regex> = OnceLock::new();
        static CODE: OnceLock<regex::Regex> = OnceLock::new();
        let text = ANSI
            .get_or_init(|| {
                regex::Regex::new(r"\x1b\[[0-9;?]*[A-Za-z]|\x1b\][^\x07]*\x07").unwrap()
            })
            .replace_all(output, "");
        let url_re = URL.get_or_init(|| regex::Regex::new(r#"https://[^\s<>"']+"#).unwrap());
        let url = url_re
            .find(&text)
            .map(|m| m.as_str().trim_end_matches(['.', ',', ')', ';']).to_owned());
        // Codes appear after the word "code"; links are dropped first so a
        // query string never reads as one.
        let words = url_re.replace_all(&text, " ");
        let code = CODE
            .get_or_init(|| {
                regex::Regex::new(r"(?i)\bcode\b[^\n]*?(?:\n\s*)?\b([A-Z0-9]{4,8}-[A-Z0-9]{4,8})\b")
                    .unwrap()
            })
            .captures(&words)
            .map(|c| c[1].to_owned());
        let wants_code = text.to_ascii_lowercase().contains("paste code");
        Self {
            url,
            code,
            wants_code,
        }
    }
}
/// A sign-in the user follows along with: `prompt` hears each change to what
/// the CLI asks, and codes from `codes` are typed into it. Neither is logged.
pub struct Interaction {
    pub prompt: Box<dyn FnMut(LoginPrompt) + Send>,
    pub codes: mpsc::Receiver<String>,
}
/// The CLI opens its browser and owns credential writes. Never relay its URLs,
/// OAuth codes or tokens to logs, clipboard, or the native account metadata.
pub fn authenticate(
    kind: Kind,
    profile: &Profile,
    action: AuthAction,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    authenticate_with(kind, profile, action, cancel, None)
}
/// Like [`authenticate`], with the CLI's link, code and code prompt shown to
/// the user. A sign-in then counts only once the CLI's own status agrees.
pub fn authenticate_with(
    kind: Kind,
    profile: &Profile,
    action: AuthAction,
    cancel: Arc<AtomicBool>,
    interaction: Option<Interaction>,
) -> Result<(), String> {
    let args = arguments(kind, action)
        .ok_or("This provider does not offer a single browser sign-in flow")?;
    let binary =
        provider::resolve(kind).ok_or_else(|| format!("{} CLI not installed", kind.label()))?;
    let mut command = Command::new(binary);
    crate::scm::shell::apply_gui_env(&mut command);
    profile.apply(kind, &mut command)?;
    let interactive = interaction.is_some();
    command
        .args(args)
        .stdin(if interactive {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(if interactive {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stderr(Stdio::piped());
    if let Some(home) = super::home() {
        command.current_dir(home);
    }
    process::isolate(&mut command);
    let mut child = process::spawn(&mut command)
        .map_err(|e| format!("Could not start {} sign-in: {e}", kind.label()))?;
    let _registration = process::register(child.id());
    let _account_process = super::track(child.id(), kind, profile);
    let mut stderr = child.stderr.take().unwrap();
    let mut stdin = child.stdin.take();
    let (mut prompt, codes) = match interaction {
        Some(Interaction { prompt, codes }) => (Some(prompt), Some(codes)),
        None => (None, None),
    };
    // Both streams feed one view of what the CLI asks; stderr alone also
    // keeps the failure tail.
    let shown = Arc::new(Mutex::new(String::new()));
    let (chunks, seen) = mpsc::channel::<()>();
    let stdout_reader = child.stdout.take().map(|mut stdout| {
        let shown = shown.clone();
        let chunks = chunks.clone();
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(n) = stdout.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                append_bounded(&mut shown.lock().unwrap(), &buffer[..n]);
                let _ = chunks.send(());
            }
        })
    });
    let stderr_shown = interactive.then(|| shown.clone());
    let (tx, rx) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        // Keep draining the pipe even after the bounded diagnostic tail fills.
        while let Ok(n) = stderr.read(&mut buffer) {
            if n == 0 {
                break;
            }
            if let Some(shown) = &stderr_shown {
                append_bounded(&mut shown.lock().unwrap(), &buffer[..n]);
                let _ = chunks.send(());
            }
            bytes.extend_from_slice(&buffer[..n]);
            if bytes.len() > 65_536 {
                bytes.drain(..bytes.len() - 65_536);
            }
        }
        let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    let deadline = Instant::now()
        + Duration::from_secs(if action == AuthAction::SignIn {
            600
        } else {
            30
        });
    let mut last = LoginPrompt::default();
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            break Err("Sign-in cancelled".into());
        }
        if let Some(prompt) = prompt.as_mut() {
            if seen.try_iter().count() > 0 {
                let next = LoginPrompt::read(&shown.lock().unwrap());
                if next != last {
                    last = next.clone();
                    prompt(next);
                }
            }
        }
        if let (Some(codes), Some(input)) = (codes.as_ref(), stdin.as_mut()) {
            for code in codes.try_iter() {
                let _ = writeln!(input, "{}", code.trim()).and_then(|_| input.flush());
            }
        }
        if Instant::now() >= deadline {
            break Err(format!(
                "{} sign-in timed out. Please try again.",
                kind.label()
            ));
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(_)) => {
                let detail = rx.recv_timeout(Duration::from_secs(1)).unwrap_or_default();
                let message = safe_detail(&detail);
                break Err(if message.is_empty() {
                    format!("{} sign-in exited unexpectedly", kind.label())
                } else {
                    message
                });
            }
            Err(e) => break Err(format!("Could not complete sign-in: {e}")),
            Ok(None) => thread::sleep(Duration::from_millis(50)),
        }
    };
    drop(stdin);
    process::terminate(&mut child);
    drop(rx);
    let _ = reader.join();
    if let Some(reader) = stdout_reader {
        let _ = reader.join();
    }
    if result.is_ok()
        && interactive
        && action == AuthAction::SignIn
        && signed_in(kind, profile) == Some(false)
    {
        return Err(format!(
            "{} still reports that you are signed out. Please try again.",
            kind.label()
        ));
    }
    result
}
fn append_bounded(text: &mut String, bytes: &[u8]) {
    text.push_str(&String::from_utf8_lossy(bytes));
    if text.len() > 65_536 {
        let mut cut = text.len() - 65_536;
        while !text.is_char_boundary(cut) {
            cut += 1;
        }
        text.drain(..cut);
    }
}
/// What the CLI's own status command says, when it says it plainly.
pub fn signed_in(kind: Kind, profile: &Profile) -> Option<bool> {
    let args: &[&str] = match kind {
        Kind::Claude => &["auth", "status"],
        Kind::Codex => &["login", "status"],
        Kind::Kiro => &["whoami"],
        _ => return None,
    };
    let (success, output) = status_output(kind, args, profile).ok()?;
    read_status(kind, success, &output)
}
fn read_status(kind: Kind, success: bool, output: &str) -> Option<bool> {
    match kind {
        Kind::Claude => {
            let start = output.find('{')?;
            serde_json::from_str::<serde_json::Value>(&output[start..])
                .ok()?
                .get("loggedIn")?
                .as_bool()
        }
        _ if output.to_ascii_lowercase().contains("not logged in") => Some(false),
        _ if success => Some(true),
        _ => None,
    }
}
/// A status command's exit and its stdout and stderr together, within 15s.
fn status_output(kind: Kind, args: &[&str], profile: &Profile) -> Result<(bool, String), String> {
    let binary =
        provider::resolve(kind).ok_or_else(|| format!("{} CLI not installed", kind.label()))?;
    let mut command = Command::new(binary);
    crate::scm::shell::apply_gui_env(&mut command);
    profile.apply(kind, &mut command)?;
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    process::isolate(&mut command);
    let mut child = process::spawn(&mut command).map_err(|e| e.to_string())?;
    let _registration = process::register(child.id());
    let output = Arc::new(Mutex::new(String::new()));
    let readers: Vec<_> = [
        child
            .stdout
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
        child
            .stderr
            .take()
            .map(|s| Box::new(s) as Box<dyn Read + Send>),
    ]
    .into_iter()
    .flatten()
    .map(|mut stream| {
        let output = output.clone();
        thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(n) = stream.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                append_bounded(&mut output.lock().unwrap(), &buffer[..n]);
            }
        })
    })
    .collect();
    let deadline = Instant::now() + Duration::from_secs(15);
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status.success()),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(25)),
            Ok(None) => break Err("Status check timed out".to_owned()),
            Err(e) => break Err(e.to_string()),
        }
    };
    process::terminate(&mut child);
    for reader in readers {
        let _ = reader.join();
    }
    let text = output.lock().unwrap().clone();
    status.map(|success| (success, text))
}
fn safe_detail(value: &str) -> String {
    let last = value
        .lines()
        .rfind(|line| !line.trim().is_empty())
        .unwrap_or("");
    let urls = regex::Regex::new(r"(?i)https?://\S+").unwrap();
    urls.replace_all(last, "sign-in link")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(240)
        .collect()
}
pub(crate) fn cli_output(kind: Kind, args: &[&str], profile: &Profile) -> Result<String, String> {
    let binary = provider::resolve(kind)
        .ok_or_else(|| kind.missing_binary_or(format!("{} CLI not found", kind.label())))?;
    let mut command = Command::new(binary);
    crate::scm::shell::apply_gui_env(&mut command);
    profile.apply(kind, &mut command)?;
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    process::isolate(&mut command);
    let mut child = process::spawn(&mut command).map_err(|e| e.to_string())?;
    let _registration = process::register(child.id());
    let _account_process = super::track(child.id(), kind, profile);
    let output = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = output
            .take(1_048_577)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = tx.send(result);
    });
    let result = rx
        .recv_timeout(Duration::from_secs(15))
        .map_err(|_| "Provider request timed out".to_string())
        .and_then(|r| r.map_err(|e| e.to_string()))
        .and_then(|bytes| {
            if bytes.len() > 1_048_576 {
                Err("Provider output exceeded its limit".into())
            } else {
                Ok(String::from_utf8_lossy(&bytes).into_owned())
            }
        });
    process::terminate(&mut child);
    drop(rx);
    let _ = reader.join();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn browser_auth_commands_and_error_details_match_the_provider_flow() {
        assert_eq!(
            arguments(Kind::Claude, AuthAction::SignIn),
            Some(["auth", "login"].as_slice())
        );
        assert_eq!(
            arguments(Kind::Codex, AuthAction::SignOut),
            Some(["logout"].as_slice())
        );
        assert_eq!(
            arguments(Kind::Grok, AuthAction::SignIn),
            Some(["login", "--oauth"].as_slice())
        );
        assert!(arguments(Kind::Pi, AuthAction::SignIn).is_none());
        assert_eq!(
            safe_detail("first line\n Complete at https://example.test/?code=private\n"),
            "Complete at sign-in link"
        );
        assert_eq!(safe_detail(&"a".repeat(300)).len(), 240);
        assert_eq!(
            arguments(Kind::Kiro, AuthAction::SignIn),
            Some(["login"].as_slice())
        );
    }
    #[test]
    fn login_prompts_read_each_cli_s_captured_output() {
        let claude = LoginPrompt::read(include_str!("fixtures/claude-login.txt"));
        assert!(claude
            .url
            .as_deref()
            .is_some_and(|url| url.starts_with("https://claude.com/cai/oauth/authorize?")));
        assert_eq!(claude.code, None);
        assert!(claude.wants_code);

        // The local callback is not the link to open, and the query string's
        // dashed values are not a code.
        let codex = LoginPrompt::read(include_str!("fixtures/codex-login.txt"));
        assert!(codex
            .url
            .as_deref()
            .is_some_and(|url| url.starts_with("https://auth.openai.com/oauth/authorize?")));
        assert_eq!(codex.code, None);
        assert!(!codex.wants_code);

        let device = LoginPrompt::read(include_str!("fixtures/codex-device.txt"));
        assert_eq!(
            device.url.as_deref(),
            Some("https://auth.openai.com/codex/device")
        );
        assert_eq!(device.code.as_deref(), Some("TEST-CODE1"));

        let kiro = LoginPrompt::read(include_str!("fixtures/kiro-device.txt"));
        assert_eq!(kiro.code.as_deref(), Some("TEST-CODE2"));
        assert!(kiro
            .url
            .as_deref()
            .is_some_and(|url| url.starts_with("https://view.awsapps.com/start/")));

        // Kiro's portal sign-in only opens the browser.
        assert_eq!(
            LoginPrompt::read(include_str!("fixtures/kiro-portal.txt")),
            LoginPrompt::default()
        );
        // Partial output reads as far as it goes.
        let partial = &include_str!("fixtures/codex-device.txt")[..120];
        assert_eq!(LoginPrompt::read(partial).code, None);
    }
    #[test]
    fn status_reads_only_what_the_cli_says_plainly() {
        let out = r#"{"loggedIn": false, "authMethod": "none"}"#;
        assert_eq!(read_status(Kind::Claude, false, out), Some(false));
        let out = "{\n  \"loggedIn\": true,\n  \"authMethod\": \"claude.ai\"\n}";
        assert_eq!(read_status(Kind::Claude, true, out), Some(true));
        assert_eq!(read_status(Kind::Claude, true, "unknown command"), None);
        assert_eq!(
            read_status(Kind::Codex, true, "Logged in using ChatGPT"),
            Some(true)
        );
        assert_eq!(
            read_status(Kind::Codex, false, "Not logged in"),
            Some(false)
        );
        assert_eq!(
            read_status(Kind::Kiro, true, "Logged in with Google\n"),
            Some(true)
        );
        assert_eq!(
            read_status(Kind::Kiro, false, "Not logged in\n"),
            Some(false)
        );
        // A failing status command that says nothing clear does not block.
        assert_eq!(read_status(Kind::Kiro, false, "network error"), None);
    }
}
