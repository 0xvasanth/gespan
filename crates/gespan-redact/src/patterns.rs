//! Tokens, `key=value` secrets and emails, found by pattern alone.

use std::sync::LazyLock;

use regex::Regex;

use crate::{EMAIL, REDACTED};

/// Well-known token shapes. Each needs a word boundary before it and a
/// minimum length, so words like `risk-assessment-framework` survive.
static TOKENS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"\b(?:",
        r"sk-[A-Za-z0-9_-]{20,}",
        r"|gh[pousr]_[A-Za-z0-9]{20,}",
        r"|github_pat_[A-Za-z0-9_]{20,}",
        r"|xox[abposr]-[A-Za-z0-9-]{10,}",
        r"|(?:AKIA|ASIA)[0-9A-Z]{16,}",
        r"|eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*",
        r")",
    ))
    .expect("token pattern")
});

/// `password=…`, `GITHUB_TOKEN="…"`, `?access_token=…`. The value is quoted
/// (JSON-escaped quotes too, and it may span lines) or runs to whitespace,
/// `&`, `;` or `,`.
static KEY_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(?i)\b([A-Za-z0-9_-]*(?:password|passwd|token|secret|api[_-]?key))=",
        r#"(?:\\"[^"]*?\\"|"[^"]*"|'[^']*'|[^\s&;,"']+)"#,
    ))
    .expect("key=value pattern")
});

static BEARER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(bearer\s+)[A-Za-z0-9._~+/=-]{8,}").expect("bearer pattern")
});

static EMAILS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[\w.%+'-]+@[\w-]+(?:\.[\w-]+)*\.\w{2,}").expect("email pattern"));

/// `sk-…`, `ghp_…`, `xoxb-…`, `AKIA…`, JWTs, `Bearer …` and secret-named
/// `key=value` pairs become `<redacted>`; the key or `Bearer` stays.
pub(crate) fn redact_tokens(input: &str) -> String {
    let out = TOKENS.replace_all(input, REDACTED);
    let out = KEY_VALUE.replace_all(&out, format!("${{1}}={REDACTED}"));
    BEARER
        .replace_all(&out, format!("${{1}}{REDACTED}"))
        .into_owned()
}

/// Email addresses become `<email>`.
pub(crate) fn redact_emails(input: &str) -> String {
    EMAILS.replace_all(input, EMAIL).into_owned()
}

/// A key whose value is a secret: anything ending in `password`, `passwd`,
/// `token`, `secret` or `api_key`, in any case.
pub(crate) fn is_secret_key(key: &str) -> bool {
    let key: String = key
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .collect::<String>()
        .to_ascii_lowercase();
    ["password", "passwd", "token", "secret", "apikey"]
        .iter()
        .any(|suffix| key.ends_with(suffix))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gone(input: &str, secret: &str) -> String {
        let out = redact_tokens(input);
        assert!(!out.contains(secret), "{secret:?} survived in {out:?}");
        out
    }

    #[test]
    fn openai_and_anthropic_keys() {
        let ant = "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
        assert_eq!(gone(&format!("key {ant} used"), ant), "key <redacted> used");
        let proj = "sk-proj-a1B2c3D4e5F6g7H8i9J0k1L2";
        assert_eq!(gone(&format!("({proj})"), proj), "(<redacted>)");
    }

    #[test]
    fn github_tokens() {
        let ghp = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";
        assert_eq!(gone(&format!("auth {ghp}"), ghp), "auth <redacted>");
        let pat = "github_pat_11ABCDEFG0123456789_abcdefghijklmnop";
        assert_eq!(gone(pat, pat), "<redacted>");
        let gho = "gho_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123";
        gone(gho, gho);
    }

    #[test]
    fn slack_and_aws_keys() {
        let slack = "xoxb-1234567890-0987654321-AbCdEfGh";
        assert_eq!(gone(&format!("slack={slack}"), slack), "slack=<redacted>");
        assert_eq!(
            gone("aws AKIAIOSFODNN7EXAMPLE,", "AKIAIOSFODNN7EXAMPLE"),
            "aws <redacted>,"
        );
        // Longer than the usual 20 characters still goes as a whole.
        gone("ASIAIOSFODNN7EXAMPLEXYZ", "EXAMPLE");
    }

    #[test]
    fn jwts() {
        let jwt =
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0In0.dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(gone(&format!("jwt: {jwt}\n"), jwt), "jwt: <redacted>\n");
        // An unsigned token has an empty third part.
        let unsigned = "eyJhbGciOiJub25lIn0.eyJzdWIiOiIxMjM0In0.";
        gone(unsigned, "eyJzdWIiOiIxMjM0In0");
    }

    #[test]
    fn bearer_keeps_the_scheme() {
        assert_eq!(
            redact_tokens("Authorization: Bearer abc.def-123456"),
            "Authorization: Bearer <redacted>"
        );
        assert_eq!(redact_tokens("bearer\tAbCdEfGh12=="), "bearer\t<redacted>");
    }

    #[test]
    fn secret_named_key_values() {
        assert_eq!(
            redact_tokens("password=hunter22&user=x"),
            "password=<redacted>&user=x"
        );
        assert_eq!(
            redact_tokens("GET /cb?code=1&access_token=abc123; next"),
            "GET /cb?code=1&access_token=<redacted>; next"
        );
        assert_eq!(
            redact_tokens("GITHUB_TOKEN=\"a b\nc\" x"),
            "GITHUB_TOKEN=<redacted> x"
        );
        assert_eq!(
            redact_tokens("Client_Secret='s3'"),
            "Client_Secret=<redacted>"
        );
        assert_eq!(
            redact_tokens("x-api-key=k1 api_key=k2"),
            "x-api-key=<redacted> api_key=<redacted>"
        );
        // JSON-escaped quotes inside raw log text.
        assert_eq!(
            redact_tokens(r#"{"msg":"token=\"t0p s3cret\" ok"}"#),
            r#"{"msg":"token=<redacted> ok"}"#
        );
    }

    #[test]
    fn empty_secret_values_are_left_alone() {
        assert_eq!(redact_tokens("token= missing"), "token= missing");
    }

    #[test]
    fn ordinary_words_are_not_tokens() {
        for text in [
            "risk-assessment-framework-v2",
            "feat/task-runner-improvements-and-more",
            "the bearer of bad news",
            "token: missing",
            "sk-short",
            "desk-1234567890abcdefghijkl",
            "AKIA",
        ] {
            assert_eq!(redact_tokens(text), text);
        }
    }

    #[test]
    fn emails() {
        assert_eq!(
            redact_emails("from alice.o'neil+x@example.co.uk, cc bob@ex.io."),
            "from <email>, cc <email>."
        );
        assert_eq!(redact_emails("zoë@exämple.de"), "<email>");
        assert_eq!(redact_emails("git@github.com:org/repo"), "<email>:org/repo");
        assert_eq!(redact_emails("@types/node and a@b"), "@types/node and a@b");
    }

    #[test]
    fn secret_keys() {
        for key in [
            "password",
            "Token",
            "access_token",
            "GITHUB_TOKEN",
            "client_secret",
            "apiKey",
            "api-key",
            "passwd",
        ] {
            assert!(is_secret_key(key), "{key}");
        }
        for key in [
            "tokens_used",
            "password_hint",
            "secretary",
            "id",
            "token_count",
        ] {
            assert!(!is_secret_key(key), "{key}");
        }
    }
}
