use super::Profile;
use crate::provider::Kind;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
#[derive(Clone, Serialize, Deserialize, Default, Debug, PartialEq)]
pub struct Identity {
    pub email: Option<String>,
    pub name: Option<String>,
    pub plan: Option<String>,
    pub organization: Option<String>,
}
pub(super) fn read(profile: &Profile, kind: Kind) -> Option<Identity> {
    let dir = profile.directory(kind).ok()?;
    match kind {
        Kind::Claude => claude_identity(dir, super::claude_oauth_token()),
        Kind::Codex => codex_identity(dir),
        // Both keep one sign-in per machine, so only the default account
        // has one to read.
        Kind::Cursor if dir.is_none() => parse_cursor_identity(&read_json(&cursor_config()?)?),
        Kind::Kiro if dir.is_none() => kiro_identity(profile),
        _ => None,
    }
}
fn home() -> Option<PathBuf> {
    super::home()
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

fn text(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

fn capitalize(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn claude_identity(dir: Option<PathBuf>, token: Option<String>) -> Option<Identity> {
    // Claude Code prefers the token, so the default account is signed in
    // with it whatever `.claude.json` holds. The token names no one.
    if dir.is_none() && token.is_some() {
        return Some(Identity::default());
    }
    let path = match dir {
        Some(dir) => dir.join(".claude.json"),
        None => home()?.join(".claude.json"),
    };
    parse_claude_identity(&read_json(&path)?)
}

/// Parse the `oauthAccount` block Claude Code writes to `.claude.json`.
fn parse_claude_identity(config: &Value) -> Option<Identity> {
    let account = config.get("oauthAccount")?;
    // organizationType is e.g. "claude_max", "claude_pro", "claude_team".
    let plan = text(account, "organizationType")
        .map(|kind| capitalize(kind.strip_prefix("claude_").unwrap_or(&kind)));
    Some(Identity {
        email: text(account, "emailAddress"),
        name: text(account, "displayName").or_else(|| text(account, "fullName")),
        plan,
        organization: text(account, "organizationName"),
    })
}

fn codex_identity(dir: Option<PathBuf>) -> Option<Identity> {
    let dir = match dir {
        Some(dir) => dir,
        None => std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|home| home.join(".codex")))?,
    };
    parse_codex_identity(&read_json(&dir.join("auth.json"))?)
}

/// Parse the claims of the `id_token` in Codex's `auth.json`.
fn parse_codex_identity(auth: &Value) -> Option<Identity> {
    let id_token = auth.get("tokens")?.get("id_token")?.as_str()?;
    let payload = id_token.split('.').nth(1)?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload.trim_end_matches('='))
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    let openai = claims.get("https://api.openai.com/auth");
    let organization = openai
        .and_then(|auth| auth.get("organizations"))
        .and_then(Value::as_array)
        .and_then(|orgs| {
            orgs.iter()
                .find(|org| org.get("is_default").and_then(Value::as_bool) == Some(true))
        })
        .and_then(|org| text(org, "title"));
    Some(Identity {
        email: text(&claims, "email").or_else(|| {
            claims
                .get("https://api.openai.com/profile")
                .and_then(|profile| text(profile, "email"))
        }),
        name: text(&claims, "name"),
        plan: openai
            .and_then(|auth| text(auth, "chatgpt_plan_type"))
            .map(|plan| capitalize(&plan)),
        organization,
    })
}

/// Where Cursor's CLI keeps `cli-config.json`, by the rule the CLI itself
/// follows: `CURSOR_CONFIG_DIR`, then `$XDG_CONFIG_HOME/cursor`, then
/// `~/.cursor`.
fn cursor_config() -> Option<PathBuf> {
    let set = |var: &str| {
        std::env::var_os(var)
            .filter(|v| !v.to_string_lossy().trim().is_empty())
            .map(PathBuf::from)
    };
    let dir = set("CURSOR_CONFIG_DIR")
        .or_else(|| set("XDG_CONFIG_HOME").map(|dir| dir.join("cursor")))
        .or_else(|| home().map(|home| home.join(".cursor")))?;
    Some(dir.join("cli-config.json"))
}

/// Parse the `authInfo` block `agent login` writes to `cli-config.json`.
fn parse_cursor_identity(config: &Value) -> Option<Identity> {
    let auth = config.get("authInfo")?;
    let identity = Identity {
        email: text(auth, "email"),
        name: text(auth, "displayName"),
        ..Default::default()
    };
    (identity.email.is_some() || identity.name.is_some()).then_some(identity)
}

fn kiro_identity(profile: &Profile) -> Option<Identity> {
    let output =
        super::login::cli_output(Kind::Kiro, &["whoami", "--format", "json"], profile).ok()?;
    parse_kiro_identity(&serde_json::from_str(output.trim()).ok()?)
}

/// Parse `kiro-cli whoami --format json`: the email, and the sign-in
/// provider ("Google", "GitHub", ...) as the badge beside the account.
fn parse_kiro_identity(whoami: &Value) -> Option<Identity> {
    let email = text(whoami, "email")?;
    Some(Identity {
        email: Some(email),
        organization: text(whoami, "provider"),
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn identity(
        email: Option<&str>,
        name: Option<&str>,
        plan: Option<&str>,
        organization: Option<&str>,
    ) -> Option<Identity> {
        Some(Identity {
            email: email.map(String::from),
            name: name.map(String::from),
            plan: plan.map(String::from),
            organization: organization.map(String::from),
        })
    }

    fn codex_auth(claims: Value) -> Value {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(serde_json::to_vec(&claims).unwrap());
        json!({ "tokens": { "id_token": format!("header.{payload}.signature") } })
    }

    #[test]
    fn claude_reads_oauth_account() {
        let config = json!({
            "oauthAccount": {
                "emailAddress": "ada@example.com",
                "displayName": "Ada",
                "fullName": "Ada Lovelace",
                "organizationType": "claude_team",
                "organizationName": "Acme"
            }
        });
        assert_eq!(
            parse_claude_identity(&config),
            identity(
                Some("ada@example.com"),
                Some("Ada"),
                Some("Team"),
                Some("Acme")
            )
        );
    }

    #[test]
    fn claude_falls_back_to_full_name_and_keeps_missing_email_optional() {
        let config = json!({
            "oauthAccount": { "fullName": "Ada Lovelace", "organizationType": "claude_max" }
        });
        assert_eq!(
            parse_claude_identity(&config),
            identity(None, Some("Ada Lovelace"), Some("Max"), None)
        );
    }

    #[test]
    fn claude_without_oauth_account_is_signed_out() {
        assert_eq!(parse_claude_identity(&json!({ "numStartups": 3 })), None);
    }

    #[test]
    fn codex_reads_id_token_claims() {
        let auth = codex_auth(json!({
            "email": "ada@example.com",
            "name": "Ada",
            "https://api.openai.com/auth": {
                "chatgpt_plan_type": "plus",
                "organizations": [
                    { "title": "Other", "is_default": false },
                    { "title": "Personal", "is_default": true }
                ]
            }
        }));
        assert_eq!(
            parse_codex_identity(&auth),
            identity(
                Some("ada@example.com"),
                Some("Ada"),
                Some("Plus"),
                Some("Personal")
            )
        );
    }

    #[test]
    fn codex_falls_back_to_namespaced_profile_email() {
        let auth = codex_auth(json!({
            "https://api.openai.com/profile": { "email": "ada@example.com" }
        }));
        assert_eq!(
            parse_codex_identity(&auth),
            identity(Some("ada@example.com"), None, None, None)
        );
    }

    #[test]
    fn codex_rejects_malformed_tokens() {
        let token = |id_token: &str| json!({ "tokens": { "id_token": id_token } });
        assert_eq!(parse_codex_identity(&token("no-dots")), None);
        assert_eq!(parse_codex_identity(&token("header.!!!.signature")), None);
        let not_json = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("not json");
        assert_eq!(
            parse_codex_identity(&token(&format!("h.{not_json}.s"))),
            None
        );
        assert_eq!(
            parse_codex_identity(&json!({ "OPENAI_API_KEY": "sk-x" })),
            None
        );
    }

    #[test]
    fn cursor_reads_auth_info_and_ignores_the_rest_of_its_config() {
        let config = json!({
            "model": { "modelId": "auto" },
            "authInfo": {
                "email": "ada@example.com",
                "displayName": "Ada Lovelace",
                "userId": 509651823,
                "authId": "google-oauth2|1"
            }
        });
        assert_eq!(
            parse_cursor_identity(&config),
            identity(Some("ada@example.com"), Some("Ada Lovelace"), None, None)
        );
    }

    #[test]
    fn cursor_without_auth_info_is_signed_out() {
        assert_eq!(parse_cursor_identity(&json!({ "version": 1 })), None);
        // `agent logout` can leave the block behind with nothing in it.
        assert_eq!(
            parse_cursor_identity(&json!({ "authInfo": { "email": " ", "userId": 1 } })),
            None
        );
    }

    #[test]
    fn kiro_reads_whoami_email_and_sign_in_provider() {
        let whoami = json!({
            "accountType": "SocialGoogle",
            "provider": "Google",
            "email": "ada@example.com"
        });
        assert_eq!(
            parse_kiro_identity(&whoami),
            identity(Some("ada@example.com"), None, None, Some("Google"))
        );
        assert_eq!(
            parse_kiro_identity(&json!({ "email": "ada@example.com" })),
            identity(Some("ada@example.com"), None, None, None)
        );
    }

    #[test]
    fn kiro_without_an_email_is_signed_out() {
        assert_eq!(parse_kiro_identity(&json!({ "provider": "Google" })), None);
        assert_eq!(parse_kiro_identity(&json!({})), None);
    }

    #[test]
    fn claude_oauth_token_signs_in_only_the_default_account() {
        let dir = std::env::temp_dir().join(format!("gespan-claude-token-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let token = || Some("sk-ant-oat01-test".to_owned());
        // The default account is signed in by the token alone, before any
        // stored account is read.
        assert_eq!(claude_identity(None, token()), Some(Identity::default()));
        // A named account ignores it: no stored sign-in means signed out.
        assert_eq!(claude_identity(Some(dir.clone()), token()), None);
        std::fs::write(
            dir.join(".claude.json"),
            r#"{"oauthAccount":{"emailAddress":"ada@example.com","organizationType":"claude_pro"}}"#,
        )
        .unwrap();
        assert_eq!(
            claude_identity(Some(dir.clone()), token()),
            identity(Some("ada@example.com"), None, Some("Pro"), None)
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
