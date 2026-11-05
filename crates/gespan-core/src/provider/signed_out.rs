//! A provider CLI that lost its sign-in says so in its own structured terms.
//! Only those signals count; wording alone never turns another failure into a
//! sign-in prompt.
use super::Kind;
use serde_json::Value;

const PREFIX: &str = "Signed out of ";
/// ACP's `auth_required` error code.
const ACP_AUTH_REQUIRED: i64 = -32000;

/// The turn error for a signed-out provider. The chat recognises a saved one
/// by this shape after a restart.
pub(crate) fn message(kind: Kind, detail: &str) -> String {
    let detail = detail.trim();
    if detail.is_empty() {
        format!("{PREFIX}{}.", kind.label())
    } else {
        format!("{PREFIX}{}. {detail}", kind.label())
    }
}

/// The provider a saved turn error says the user is signed out of.
pub fn provider_of(error: &str) -> Option<Kind> {
    let rest = error.trim_start().strip_prefix(PREFIX)?;
    Kind::ALL
        .iter()
        .copied()
        .find(|kind| rest.starts_with(&format!("{}.", kind.label())))
}

/// Claude's synthetic assistant frame for a missing, invalid or expired login.
pub(crate) fn claude(frame: &Value) -> bool {
    frame["error"] == "authentication_failed"
}

/// What Claude printed for the failed sign-in, rather than its error code.
pub(crate) fn claude_detail(frame: &Value) -> String {
    frame["message"]["content"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|block| block["text"].as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A Codex error that will not be retried: the token could not be refreshed,
/// or the API rejected the request as unauthenticated.
pub(crate) fn codex(error: &Value) -> bool {
    let info = &error["codexErrorInfo"];
    info == "unauthorized"
        || info
            .as_object()
            .into_iter()
            .flat_map(|info| info.values())
            .any(|detail| detail["httpStatusCode"] == 401)
}

/// Kiro answers requests with ACP's auth-required code once signed out. The
/// code alone is not enough: JSON-RPC lets servers use it for other errors.
pub(crate) fn kiro(error: &Value) -> bool {
    error["code"] == ACP_AUTH_REQUIRED
        && error["message"]
            .as_str()
            .is_some_and(|m| m.to_ascii_lowercase().contains("authentication required"))
}

#[cfg(test)]
mod tests {
    use super::super::{Protocol, Request};
    use super::*;
    use serde_json::json;

    fn protocol(kind: Kind) -> Protocol {
        Protocol::new(Request {
            kind,
            binary: "unused".into(),
            cwd: "/tmp".into(),
            prompt: "Hello".into(),
            provider_id: None,
            model: None,
            settings: Default::default(),
        })
    }
    /// Replays captured frames until one ends the turn.
    fn replay(p: &mut Protocol, frames: &str) -> Result<(), String> {
        let mut out = Vec::new();
        for line in frames.lines().filter(|l| !l.trim().is_empty()) {
            p.frame(serde_json::from_str(line).unwrap(), &mut out)?;
        }
        Ok(())
    }

    #[test]
    fn claude_captures_end_as_signed_out_with_the_cli_wording() {
        for (frames, detail) in [
            (
                include_str!("fixtures/signed_out/claude-loggedout.jsonl"),
                "Not logged in · Please run /login",
            ),
            (
                include_str!("fixtures/signed_out/claude-badkey.jsonl"),
                "Invalid API key · Fix external API key",
            ),
            (
                include_str!("fixtures/signed_out/claude-badoauth.jsonl"),
                "Failed to authenticate. API Error: 401 OAuth access token is invalid.",
            ),
        ] {
            let mut p = protocol(Kind::Claude);
            let error = replay(&mut p, frames).unwrap_err();
            assert_eq!(error, format!("Signed out of Claude Code. {detail}"));
            assert!(p.signed_out);
            assert_eq!(provider_of(&error), Some(Kind::Claude));
        }
    }
    #[test]
    fn codex_captures_end_as_signed_out() {
        for (frames, detail) in [
            (
                include_str!("fixtures/signed_out/codex-expired.jsonl"),
                "Your access token could not be refreshed. Please log out and sign in again.",
            ),
            (
                include_str!("fixtures/signed_out/codex-loggedout.jsonl"),
                "unexpected status 401 Unauthorized: Missing bearer or basic authentication in header",
            ),
        ] {
            let mut p = protocol(Kind::Codex);
            let error = replay(&mut p, frames).unwrap_err();
            assert!(
                error.starts_with(&format!("Signed out of Codex. {detail}")),
                "{error}"
            );
            assert!(p.signed_out);
        }
    }
    #[test]
    fn codex_reconnects_on_401_are_not_yet_a_sign_out() {
        // Only the final, unretried error decides the turn.
        let frames = include_str!("fixtures/signed_out/codex-loggedout.jsonl");
        let retries: String = frames
            .lines()
            .filter(|line| line.contains("\"willRetry\":true"))
            .map(|line| format!("{line}\n"))
            .collect();
        assert!(!retries.is_empty());
        let mut p = protocol(Kind::Codex);
        assert_eq!(replay(&mut p, &retries), Ok(()));
        assert!(!p.signed_out);
    }
    #[test]
    fn kiro_capture_reports_auth_required() {
        let frames = include_str!("fixtures/signed_out/kiro-acp.jsonl");
        let errors: Vec<Value> = frames
            .lines()
            .map(|line| serde_json::from_str::<Value>(line).unwrap())
            .filter_map(|frame| frame.get("error").cloned())
            .collect();
        assert_eq!(errors.len(), 2);
        assert!(errors.iter().all(kiro));
        // Kiro's stderr names the same failure for the existing stderr check.
        let stderr = include_str!("fixtures/signed_out/kiro-stderr.txt");
        assert!(stderr
            .lines()
            .any(|line| super::super::acp::kiro_stderr_auth(line).is_some()));
    }
    #[test]
    fn other_claude_and_codex_failures_stay_ordinary_errors() {
        let mut p = protocol(Kind::Claude);
        let error = replay(
            &mut p,
            r#"{"type":"assistant","error":"rate_limit","message":{"content":[{"type":"text","text":"API Error: 529 Overloaded"}]}}"#,
        )
        .unwrap_err();
        assert!(!p.signed_out);
        assert_eq!(provider_of(&error), None);

        let mut p = protocol(Kind::Claude);
        let error = replay(
            &mut p,
            r#"{"type":"result","is_error":true,"result":"Not logged in · Please run /login"}"#,
        )
        .unwrap_err();
        // A bare result has no structured code: it stays the CLI's own text.
        assert!(!p.signed_out, "{error}");

        for error in [
            json!({"message":"Usage limit reached","codexErrorInfo":"usageLimitExceeded"}),
            json!({"message":"stream disconnected","codexErrorInfo":{"responseStreamDisconnected":{"httpStatusCode":502}}}),
            json!({"message":"unauthorized-looking words but no code"}),
        ] {
            let mut p = protocol(Kind::Codex);
            let frame = json!({"method":"error","params":{"error":error,"willRetry":false}});
            let message = replay(&mut p, &frame.to_string()).unwrap_err();
            assert!(!p.signed_out, "{message}");
            assert_eq!(provider_of(&message), None);
        }
    }

    #[test]
    fn saved_errors_name_their_provider() {
        for kind in [Kind::Claude, Kind::Codex, Kind::Kiro] {
            let error = message(kind, "Not logged in");
            assert_eq!(provider_of(&error), Some(kind), "{error}");
        }
        assert_eq!(message(Kind::Codex, " "), "Signed out of Codex.");
        assert_eq!(provider_of("Signed out of Codex."), Some(Kind::Codex));
    }
    #[test]
    fn other_errors_name_no_provider() {
        for error in [
            "Not logged in · Please run /login",
            "Usage limit reached: try again later",
            "Signed out of the VPN, retrying",
            "Signed out of Codexy. Nope",
            "Provider exited before completing the turn. Signed out of Codex.",
            "",
        ] {
            assert_eq!(provider_of(error), None, "{error}");
        }
    }
    #[test]
    fn kiro_auth_needs_both_the_code_and_the_reason() {
        assert!(kiro(
            &json!({"code":-32000,"message":"Authentication required. Not logged in."})
        ));
        assert!(!kiro(&json!({"code":-32000,"message":"Server busy"})));
        assert!(!kiro(
            &json!({"code":-32603,"message":"Authentication required"})
        ));
    }
    #[test]
    fn codex_auth_is_unauthorized_or_http_401() {
        assert!(codex(&json!({"codexErrorInfo":"unauthorized"})));
        assert!(codex(
            &json!({"codexErrorInfo":{"httpConnectionFailed":{"httpStatusCode":401}}})
        ));
        for info in [
            json!("usageLimitExceeded"),
            json!({"httpConnectionFailed":{"httpStatusCode":500}}),
            json!({"responseStreamDisconnected":{"httpStatusCode":403}}),
            json!(null),
        ] {
            assert!(!codex(&json!({"codexErrorInfo":info})), "{info}");
        }
    }
}
