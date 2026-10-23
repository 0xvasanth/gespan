use std::path::{Path, PathBuf};

use base64::Engine as _;
use serde::Serialize;
use serde_json::Value;
use tauri::AppHandle;

use crate::dirs_home;

#[derive(Serialize, Default, Debug, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProviderAccountIdentity {
    pub email: Option<String>,
    pub name: Option<String>,
    pub plan: Option<String>,
    pub organization: Option<String>,
}

/// Read the signed-in identity a provider CLI already cached on disk, so
/// no token is sent anywhere. Returns `None` when the profile is not signed in.
#[tauri::command]
pub async fn provider_account_identity(
    app: AppHandle,
    provider: String,
    account_id: Option<String>,
) -> Result<Option<ProviderAccountIdentity>, String> {
    let dir = crate::harness::provider_account_dir(&app, &provider, account_id.as_deref())?;
    tauri::async_runtime::spawn_blocking(move || match provider.as_str() {
        "claude" => Ok(claude_identity(dir)),
        "codex" => Ok(codex_identity(dir)),
        _ => Err("Account identity is not supported for this provider".into()),
    })
    .await
    .map_err(|e| e.to_string())?
}

fn home() -> Option<PathBuf> {
    dirs_home().map(PathBuf::from)
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

fn claude_identity(dir: Option<PathBuf>) -> Option<ProviderAccountIdentity> {
    claude_identity_with_env_token(dir, std::env::var("CLAUDE_CODE_OAUTH_TOKEN").ok())
}

/// The default account can be signed in with `CLAUDE_CODE_OAUTH_TOKEN` alone,
/// in which case there is no `.claude.json` to read. It is connected, just
/// anonymous. Secondary accounts always have their own config directory, so
/// the environment token never speaks for them.
fn claude_identity_with_env_token(
    dir: Option<PathBuf>,
    env_token: Option<String>,
) -> Option<ProviderAccountIdentity> {
    if dir.is_none() && env_token.is_some_and(|token| !token.trim().is_empty()) {
        return Some(ProviderAccountIdentity::default());
    }
    let path = match dir {
        Some(dir) => dir.join(".claude.json"),
        None => home()?.join(".claude.json"),
    };
    read_json(&path).as_ref().and_then(parse_claude_identity)
}

/// Parse the `oauthAccount` block Claude Code writes to `.claude.json`.
fn parse_claude_identity(config: &Value) -> Option<ProviderAccountIdentity> {
    let account = config.get("oauthAccount")?;
    // organizationType is e.g. "claude_max", "claude_pro", "claude_team".
    let plan = text(account, "organizationType")
        .map(|kind| capitalize(kind.strip_prefix("claude_").unwrap_or(&kind)));
    Some(ProviderAccountIdentity {
        email: text(account, "emailAddress"),
        name: text(account, "displayName").or_else(|| text(account, "fullName")),
        plan,
        organization: text(account, "organizationName"),
    })
}

fn codex_identity(dir: Option<PathBuf>) -> Option<ProviderAccountIdentity> {
    let dir = match dir {
        Some(dir) => dir,
        None => std::env::var_os("CODEX_HOME")
            .map(PathBuf::from)
            .or_else(|| home().map(|home| home.join(".codex")))?,
    };
    parse_codex_identity(&read_json(&dir.join("auth.json"))?)
}

/// Parse the claims of the `id_token` in Codex's `auth.json`.
fn parse_codex_identity(auth: &Value) -> Option<ProviderAccountIdentity> {
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
    Some(ProviderAccountIdentity {
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn identity(
        email: Option<&str>,
        name: Option<&str>,
        plan: Option<&str>,
        organization: Option<&str>,
    ) -> Option<ProviderAccountIdentity> {
        Some(ProviderAccountIdentity {
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

    fn claude_config_dir(label: &str, config: &Value) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "monocode-claude-identity-{label}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".claude.json"), config.to_string()).unwrap();
        dir
    }

    #[test]
    fn claude_env_token_marks_the_default_account_connected_without_a_config() {
        assert_eq!(
            claude_identity_with_env_token(None, Some("sk-ant-oat01-secret".into())),
            Some(ProviderAccountIdentity::default())
        );
    }

    #[test]
    fn claude_env_token_never_speaks_for_a_secondary_account() {
        let dir = claude_config_dir(
            "secondary",
            &json!({ "oauthAccount": { "emailAddress": "grace@example.com" } }),
        );
        let found =
            claude_identity_with_env_token(Some(dir.clone()), Some("sk-ant-oat01-x".into()));
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(found, identity(Some("grace@example.com"), None, None, None));
    }

    #[test]
    fn claude_blank_env_token_is_ignored() {
        let missing = std::env::temp_dir().join("monocode-claude-identity-missing");
        for token in [None, Some(String::new()), Some("  \n".into())] {
            assert_eq!(
                claude_identity_with_env_token(Some(missing.clone()), token),
                None
            );
        }
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
}
