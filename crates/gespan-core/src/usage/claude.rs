use super::{now_ms, timestamp, window, Amount, FetchError, Snapshot, Source, Window};
use serde_json::Value;
use std::{io::Read, path::PathBuf, time::Duration};

pub(crate) struct Claude;
impl Source for Claude {
    fn probe(
        &self,
        profile: &crate::accounts::Profile,
        _model: Option<&str>,
    ) -> Result<Snapshot, FetchError> {
        fetch_for(profile)
    }
    fn parse(&self, raw: &str) -> Result<Snapshot, FetchError> {
        let value: Value = serde_json::from_str(raw.trim())
            .map_err(|_| FetchError::failed("Claude usage response was not JSON."))?;
        parse(&value)
    }
}

fn fetch_for(profile: &crate::accounts::Profile) -> Result<Snapshot, FetchError> {
    let named = profile
        .directory(crate::provider::Kind::Claude)
        .map_err(FetchError::failed)?;
    let token = named
        .is_none()
        .then(crate::accounts::claude_oauth_token)
        .flatten();
    let credentials = sign_in(named, token).ok_or_else(|| {
        FetchError::unavailable("Sign in with claude auth login, then refresh usage.")
    })?;
    let oauth = credentials.get("claudeAiOauth").unwrap_or(&credentials);
    let token = oauth["accessToken"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| FetchError::unavailable("Claude is not signed in."))?;
    let expiry = oauth["expiresAt"]
        .as_i64()
        .or_else(|| oauth["expiresAt"].as_str()?.parse::<i64>().ok());
    // The CLI owns its rotating refresh token. Never refresh or write it here.
    let expired = || expired_error(oauth);
    if expiry.is_some_and(|at| at <= now_ms()) {
        return Err(expired());
    }
    let response = crate::net::agent()
        .timeout(Duration::from_secs(10))
        .build()
        .get("https://api.anthropic.com/api/oauth/usage")
        .set("Authorization", &format!("Bearer {token}"))
        .set("anthropic-beta", "oauth-2025-04-20")
        .set("User-Agent", "claude-code/2.1.0")
        .call()
        .map_err(|error| match error {
            ureq::Error::Status(401, _) => expired(),
            ureq::Error::Status(403, _) => {
                FetchError::unavailable("Claude usage is unavailable for this account.")
            }
            ureq::Error::Status(status, _) => {
                FetchError::failed(format!("Claude usage request failed ({status})."))
            }
            error => {
                // The transport error names the cause (timeout, DNS, TLS); the
                // footer keeps the short message.
                tracing::warn!(%error, "could not reach Claude usage");
                FetchError::failed("Could not reach Claude usage. Try refreshing again.")
            }
        })?;
    let mut body = String::new();
    response
        .into_reader()
        .take(1024 * 1024)
        .read_to_string(&mut body)
        .map_err(|_| FetchError::failed("Could not read Claude usage response."))?;
    let value = serde_json::from_str(&body)
        .map_err(|_| FetchError::failed("Claude usage response was not JSON."))?;
    parse(&value)
}

pub(super) fn parse(value: &Value) -> Result<Snapshot, FetchError> {
    if !value.is_object() {
        return Err(FetchError::failed("Claude usage response was unexpected."));
    }
    let mut windows = [("five_hour", 300), ("seven_day", 10_080)]
        .into_iter()
        .filter_map(|(key, minutes)| {
            window(
                &value[key],
                minutes,
                &["utilization", "used_percentage", "usedPercent"],
            )
        })
        .collect::<Vec<_>>();
    let mut extra_usage = claude_spend(&value["spend"]);
    // Enterprise seats have no plan windows: their monthly spend cap is the limit.
    if windows.is_empty() {
        if let Some(cap) = extra_usage.take_if(|spend| spend.limit.is_some_and(|l| l > 0.)) {
            let limit = cap.limit.unwrap_or_default();
            let percent = value["spend"]["percent"]
                .as_f64()
                .filter(|n| n.is_finite())
                .unwrap_or(cap.used / limit * 100.)
                .clamp(0., 100.);
            let mut window = Window::new(percent, 43_200, None);
            window.amount = Some(cap);
            windows.push(window);
        }
    }
    if windows.is_empty() {
        return Err(FetchError::unavailable(
            "No Claude subscription usage available for this account.",
        ));
    }
    // A model's own weekly cap, such as Fable's, sits beside the plan's.
    for limit in value["limits"].as_array().into_iter().flatten() {
        if limit["kind"] != "weekly_scoped" {
            continue;
        }
        let Some(name) = limit["scope"]["model"]["display_name"]
            .as_str()
            .filter(|name| !name.trim().is_empty())
        else {
            continue;
        };
        if let Some(mut window) = window(limit, 10_080, &["percent"]) {
            window.scope = Some(name.into());
            windows.push(window);
        }
    }
    Ok(Snapshot {
        windows,
        updated_at: now_ms(),
        extra_usage,
        ..Default::default()
    })
}
fn claude_spend(spend: &Value) -> Option<Amount> {
    if spend["enabled"] != true {
        return None;
    }
    let money = |raw: &Value| {
        let minor = raw["amount_minor"].as_f64().filter(|n| n.is_finite())?;
        let exponent = raw["exponent"].as_i64().unwrap_or(2).clamp(0, 6) as i32;
        Some(minor / 10f64.powi(exponent))
    };
    Some(Amount {
        used: money(&spend["used"])?,
        limit: money(&spend["limit"]),
        unit: spend["used"]["currency"].as_str().unwrap_or("USD").into(),
    })
}
/// The windows in Claude Code's `rate_limit_event`. Its utilization is a
/// fraction, where the usage API reports a percent.
pub fn live_windows(info: &Value) -> Vec<Window> {
    let mut windows: Vec<Window> = [("five_hour", 300), ("seven_day", 10_080)]
        .into_iter()
        .filter_map(|(key, minutes)| {
            let raw = &info["unifiedWindows"][key];
            let fraction = raw["utilization"].as_f64().filter(|n| n.is_finite())?;
            Some(Window::new(
                (fraction * 100.).clamp(0., 100.),
                minutes,
                timestamp(&raw["resetsAt"]),
            ))
        })
        .collect();
    windows.sort_by_key(|w| w.minutes);
    windows
}

/// The access token lasts about 8 hours. While a refresh token is present the
/// CLI renews it on its next run, so that is not a sign-out: a failed (not
/// unavailable) result keeps the last usage and hides the sign-in panel.
fn expired_error(oauth: &Value) -> FetchError {
    if oauth["refreshToken"]
        .as_str()
        .is_some_and(|s| !s.trim().is_empty())
    {
        FetchError::failed("Claude usage updates after your next Claude message.")
    } else {
        FetchError::unavailable("Claude sign-in expired. Run claude auth login, then refresh.")
    }
}

fn config_dir() -> Option<PathBuf> {
    std::env::var_os("CLAUDE_CONFIG_DIR")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}
fn credentials(config: Option<PathBuf>) -> Option<Value> {
    // On macOS the CLI keeps its login in Keychain only. An item that is there
    // but blank (the CLI clears it when a refresh fails) means signed out; an
    // old .credentials.json must not stand in for it, or the footer would
    // promise a renewal with a long-dead refresh token and hide Sign in.
    #[cfg(target_os = "macos")]
    {
        if let Some(raw) = keychain(config.as_deref()) {
            return credential_blob(&raw);
        }
    }
    let dir = config.or_else(|| {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(|home| PathBuf::from(home).join(".claude"))
    })?;
    let raw = std::fs::read_to_string(dir.join(".credentials.json")).ok()?;
    credential_blob(&raw)
}
/// A named account's stored sign-in, or for the default account its
/// `CLAUDE_CODE_OAUTH_TOKEN` (no expiry) ahead of the stored one, as Claude
/// Code itself prefers it.
fn sign_in(named: Option<PathBuf>, token: Option<String>) -> Option<Value> {
    match (named, token) {
        (Some(dir), _) => credentials(Some(dir)),
        (None, Some(token)) => {
            Some(serde_json::json!({ "claudeAiOauth": { "accessToken": token } }))
        }
        (None, None) => credentials(config_dir()),
    }
}
fn credential_blob(raw: &str) -> Option<Value> {
    let value: Value = serde_json::from_str(raw.trim()).ok()?;
    value.get("claudeAiOauth").unwrap_or(&value)["accessToken"]
        .as_str()
        .filter(|s| !s.trim().is_empty())?;
    Some(value)
}

#[cfg(target_os = "macos")]
pub(crate) fn delete_keychain(config: &std::path::Path) -> Result<(), String> {
    use crate::process;
    use std::{
        process::{Command, Stdio},
        time::Instant,
    };
    let service = keychain_service(Some(config));
    let mut command = Command::new("/usr/bin/security");
    command
        .args(["delete-generic-password", "-s", &service])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    process::isolate(&mut command);
    let mut child = process::spawn(&mut command).map_err(|e| e.to_string())?;
    let _registration = process::register(child.id());
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() || status.code() == Some(44) => break Ok(()),
            Ok(Some(_)) => {
                break Err("Could not remove the account's Claude Keychain credentials".into());
            }
            Err(e) => break Err(e.to_string()),
            Ok(None) if Instant::now() >= deadline => {
                break Err("Removing Claude credentials timed out".into());
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    process::terminate(&mut child);
    result
}

#[cfg(target_os = "macos")]
fn keychain_service(config: Option<&std::path::Path>) -> String {
    use sha2::{Digest, Sha256};
    use unicode_normalization::UnicodeNormalization;
    config
        .map(|dir| {
            let selector: String = dir.to_string_lossy().nfc().collect();
            let digest: String = Sha256::digest(selector.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            format!("Claude Code-credentials-{}", &digest[..8])
        })
        .unwrap_or_else(|| "Claude Code-credentials".into())
}

#[cfg(target_os = "macos")]
/// The raw Keychain secret, or None when there is no item (or it cannot be read).
fn keychain(config: Option<&std::path::Path>) -> Option<String> {
    use crate::process;
    use std::{
        process::{Command, Stdio},
        sync::mpsc,
        thread,
        time::Instant,
    };

    let service = keychain_service(config);
    let mut command = Command::new("/usr/bin/security");
    command
        .args(["find-generic-password", "-s", &service, "-w"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    process::isolate(&mut command);
    let mut child = process::spawn(&mut command).ok()?;
    let _registration = process::register(child.id());
    let output = child.stdout.take()?;
    // Drain concurrently: waiting before reading can fill the pipe and hang.
    let (tx, rx) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut body = String::new();
        let result = output.take(1024 * 1024).read_to_string(&mut body);
        let _ = tx.send(result.ok().map(|_| body));
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => {
                break rx
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                    .ok()
                    .flatten();
            }
            Ok(Some(_)) | Err(_) => break None,
            Ok(None) if Instant::now() >= deadline => break None,
            Ok(None) => thread::sleep(Duration::from_millis(20)),
        }
    };
    process::terminate(&mut child);
    drop(rx);
    let _ = reader.join();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "macos")]
    #[test]
    fn custom_config_dir_selects_claudes_hashed_keychain_service() {
        // Claude Code names the Keychain item with the first eight lowercase hex
        // characters of the selector's SHA-256; any drift loses the login.
        assert_eq!(
            keychain_service(Some(std::path::Path::new("/tmp/profile"))),
            "Claude Code-credentials-902e721c"
        );
        assert_eq!(keychain_service(None), "Claude Code-credentials");
    }
    #[test]
    fn an_expired_access_token_with_a_refresh_token_is_not_a_sign_out() {
        let renewable = serde_json::json!({"accessToken":"a","refreshToken":"r","expiresAt":1});
        assert!(!expired_error(&renewable).unavailable);
        // Claude Code blanks both tokens when a refresh fails.
        let wiped = serde_json::json!({"accessToken":"","refreshToken":"","expiresAt":0});
        assert!(expired_error(&wiped).unavailable);
        assert!(expired_error(&serde_json::json!({"accessToken":"a"})).unavailable);
    }
    #[test]
    fn accepts_nested_and_legacy_credentials_without_logging_secrets() {
        assert!(
            credential_blob(r#"{"claudeAiOauth":{"accessToken":"test","expiresAt":123}}"#)
                .is_some()
        );
        assert!(credential_blob(r#"{"accessToken":"test"}"#).is_some());
        assert!(credential_blob(r#"{"accessToken":" "}"#).is_none());
        assert!(credential_blob("invalid").is_none());
    }

    #[test]
    fn usage_uses_the_oauth_token_only_for_the_default_account() {
        let token = || Some("sk-ant-oat01-test".to_owned());
        let default = sign_in(None, token()).unwrap();
        assert_eq!(default["claudeAiOauth"]["accessToken"], "sk-ant-oat01-test");
        assert!(
            default["claudeAiOauth"]["expiresAt"].is_null(),
            "a token never reads as expired"
        );
        let dir = std::env::temp_dir().join(format!("gespan-claude-usage-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A named account keeps its own sign-in, whatever the token.
        std::fs::write(
            dir.join(".credentials.json"),
            r#"{"claudeAiOauth":{"accessToken":"stored","expiresAt":4102444800000}}"#,
        )
        .unwrap();
        let named = sign_in(Some(dir.clone()), token()).unwrap();
        assert_eq!(named["claudeAiOauth"]["accessToken"], "stored");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn an_enterprise_spend_cap_without_plan_windows_is_its_usage() {
        // Enterprise seats have no 5h or weekly windows; the monthly spend cap is
        // the limit, so it must not read as signed out.
        let value: Value =
            serde_json::from_str(include_str!("fixtures/claude-enterprise-usage.json")).unwrap();
        let snapshot = parse(&value).unwrap();
        let [window] = snapshot.windows.as_slice() else {
            panic!("one spend window: {:?}", snapshot.windows)
        };
        assert_eq!(window.label(), "mo");
        assert_eq!(window.used_percent, 25.);
        assert_eq!(
            window.amount,
            Some(Amount {
                used: 25.,
                limit: Some(100.),
                unit: "USD".into()
            })
        );
        assert!(snapshot.extra_usage.is_none(), "the cap is not shown twice");
    }
    #[test]
    fn no_windows_and_no_spend_cap_is_still_unavailable() {
        let value = serde_json::json!({"five_hour":null,"seven_day":null,"limits":[],"spend":{"enabled":false}});
        assert!(parse(&value).unwrap_err().unavailable);
    }
}
