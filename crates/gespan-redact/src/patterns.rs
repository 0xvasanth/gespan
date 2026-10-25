//! Tokens, `key=value` secrets and emails, found by pattern alone.

use std::sync::LazyLock;

use regex::Regex;

use crate::{pairs, EMAIL, REDACTED};

/// Well-known token shapes, each with a minimum length.
///
/// Prefixes that also turn up inside ordinary words (`sk-` in `risk-…`,
/// `sk_live_` in `task_live_…`, `eyJ`, `hf_`, `npm_`) need a character before
/// them that is not a letter or digit (`_` is fine, so `token_sk-…` is
/// caught). Prefixes that never do (`ghp_`, `AKIA`, `glpat-`, …) match
/// anywhere, even right after a digit.
static TOKENS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"(^|[^A-Za-z0-9])(?:",
        r"sk-[A-Za-z0-9_-]{20,}",
        r"|(?:sk|rk)_(?:live|test)_[A-Za-z0-9]{16,}",
        r"|npm_[A-Za-z0-9]{36,}",
        r"|hf_[A-Za-z0-9]{30,}",
        r"|eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]*",
        r")|(?:",
        r"gh[pousr]_[A-Za-z0-9]{20,}",
        r"|github_pat_[A-Za-z0-9_]{20,}",
        r"|glpat-[A-Za-z0-9_-]{20,}",
        r"|xox[abposre]-[A-Za-z0-9-]{10,}",
        r"|xapp-[0-9]-[A-Za-z0-9-]{10,}",
        r"|(?:AKIA|ASIA)[0-9A-Z]{16,}",
        r"|AIza[0-9A-Za-z_-]{35,}",
        r")",
    ))
    .expect("token pattern")
});

/// A PEM or PGP private key, or what is left of one when the text is cut short.
static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY[A-Z0-9 ]*-----(?s:.*?)",
        r"(?:-----END [A-Z0-9 ]*PRIVATE KEY[A-Z0-9 ]*-----|\z)",
    ))
    .expect("private key pattern")
});

/// The password in `scheme://user:password@host`, up to the last `@` before
/// the host; the user may be empty (`redis://:pw@host`). A `?` or `#` ends
/// the authority, so `host:8443/docs?u=bob@corp.com` is not a password.
static URL_PASSWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?-u:\b)([a-z][a-z0-9+.-]*://[^\s:/@]*:)[^\s@?#]*(?:@[^\s@/?#]*)*@")
        .expect("URL password pattern")
});

static BEARER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(?-u:\b)(bearer\s+)[A-Za-z0-9._~+/=-]{8,}").expect("bearer pattern")
});

/// Case-sensitive, as sent in headers, so the word "basic" in prose survives.
static BASIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?-u:\b)(Basic\s+)[A-Za-z0-9+/=]{8,}").expect("basic pattern"));

/// The local part may hold a `'` (`o'neil@…`) but never start with one, so
/// a quoted `'bob@x.io'` keeps its opening quote.
static EMAILS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"[\w.%+-][\w.%+'-]*@[\w-]+(?:\.[\w-]+)*\.\w{2,}").expect("email pattern")
});

/// Private keys, well-known tokens, `Bearer`/`Basic` credentials, values of
/// secret-named keys (`key=…`, `key: "…"`, `"key": …`, `--key …`) and URL
/// passwords all become `<redacted>`; the key, scheme or user stays.
pub(crate) fn redact_tokens(input: &str) -> String {
    let out = PRIVATE_KEY.replace_all(input, REDACTED);
    // A token glued onto another (`…AAsk-…`) only has a boundary once the
    // first is replaced, so repeat. Every token is longer than `<redacted>`,
    // so this ends.
    let mut out = out.into_owned();
    while TOKENS.is_match(&out) {
        out = TOKENS
            .replace_all(&out, format!("${{1}}{REDACTED}"))
            .into_owned();
    }
    let out = BEARER.replace_all(&out, format!("${{1}}{REDACTED}"));
    let out = BASIC.replace_all(&out, format!("${{1}}{REDACTED}"));
    let out = pairs::redact_secret_pairs(&out);
    URL_PASSWORD
        .replace_all(&out, format!("${{1}}{REDACTED}@"))
        .into_owned()
}

/// Email addresses become `<email>`.
pub(crate) fn redact_emails(input: &str) -> String {
    EMAILS.replace_all(input, EMAIL).into_owned()
}

/// A key whose value is a secret: one that mentions a password, secret, API,
/// private or access key, credential, authorization, cookie or signature, in
/// any case and spelling (`apiKey`, `API-KEY`), or has a `pass`, `pwd`, `pw`
/// or `sig` segment (`DB_PASS`, `dbPass`). Keys that mention a token are
/// secret too, unless they name a token counter (`max_tokens`,
/// `input_tokens`, `tokens_used`, `token_count`).
pub(crate) fn is_secret_key(key: &str) -> bool {
    let flat = flatten(key);
    is_strong_secret_key(key) || flat.contains("token") && !is_token_counter(&flat)
}

/// A secret key whose name alone says so (`password`, `api_key`, `cookie`),
/// as opposed to one that only mentions a token.
pub(crate) fn is_strong_secret_key(key: &str) -> bool {
    let flat = flatten(key);
    [
        "password",
        "passwd",
        "secret",
        "apikey",
        "privatekey",
        "accesskey",
        "credential",
        "authorization",
        "cookie",
        "signature",
    ]
    .iter()
    .any(|marker| flat.contains(marker))
        || segments(key).any(|segment| matches!(segment.as_str(), "pass" | "pwd" | "pw" | "sig"))
}

fn flatten(key: &str) -> String {
    key.chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase()
}

/// `max_tokens`, `input_tokens`, `cache_read_input_tokens`, `tokens_used`,
/// `token_count`: model usage numbers, not credentials. Anchored at the end,
/// so every suffix of a counter key is a counter too and the pair scanner
/// can skip a rejected key whole.
fn is_token_counter(flat: &str) -> bool {
    ["count", "used", "limit", "usage"]
        .iter()
        .any(|suffix| flat.ends_with(suffix))
        || flat == "tokens"
        || [
            "max",
            "min",
            "input",
            "output",
            "total",
            "prompt",
            "completion",
            "reasoning",
        ]
        .iter()
        .any(|word| flat.ends_with(&format!("{word}tokens")))
}

/// A key's words, split at punctuation and at camelCase humps, lower-cased.
fn segments(key: &str) -> impl Iterator<Item = String> + '_ {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut previous_lower = false;
    for c in key.chars() {
        if !c.is_ascii_alphanumeric() {
            words.push(std::mem::take(&mut word));
            previous_lower = false;
            continue;
        }
        if c.is_ascii_uppercase() && previous_lower {
            words.push(std::mem::take(&mut word));
        }
        previous_lower = c.is_ascii_lowercase();
        word.push(c.to_ascii_lowercase());
    }
    words.push(word);
    words.into_iter().filter(|word| !word.is_empty())
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
            "GET /cb?code=1&access_token=<redacted> next"
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
        assert_eq!(redact_tokens("token="), "token=");
        assert_eq!(redact_tokens("password=\n"), "password=\n");
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
            "secret_key",
            "AWS_SECRET_ACCESS_KEY",
            "private_key",
            "Authorization",
            "cookie",
            "credentials",
            "password_hint",
            "refresh_token_v2",
            "access_tokens",
            "github_tokens",
            "DB_PASS",
            "MYSQL_PWD",
            "dbPass",
            "X-Amz-Signature",
            "sig",
            "secret_count",
            "password_used",
            "maxtoken",
            "min_token_v2",
            "cache-github-tokens",
            "max_github_tokens",
        ] {
            assert!(is_secret_key(key), "{key}");
        }
        for key in [
            "tokens_used",
            "token_count",
            "max_tokens",
            "input_tokens",
            "cache_read_input_tokens",
            "id",
            "key",
            "sort_key",
            "session_id",
            "account",
            "passport",
            "compass",
            "design",
        ] {
            assert!(!is_secret_key(key), "{key}");
        }
    }

    #[test]
    fn secret_key_values_with_spaces_or_any_marker_position() {
        assert_eq!(
            redact_tokens("AWS_SECRET_ACCESS_KEY=wJalrXUtnFEMI/K7MDENG region=us"),
            "AWS_SECRET_ACCESS_KEY=<redacted> region=us"
        );
        assert_eq!(
            redact_tokens("password = hunter22 ok"),
            "password=<redacted> ok"
        );
        assert_eq!(
            redact_tokens("tokens_used=5 sort_key=name"),
            "tokens_used=5 sort_key=name"
        );
        assert_eq!(redact_tokens("msg=x token=abc"), "msg=x token=<redacted>");
    }

    #[test]
    fn unclosed_quote_still_redacts_the_value() {
        assert_eq!(redact_tokens("password=\"abc"), "password=<redacted>");
    }

    #[test]
    fn json_pairs_inside_text() {
        assert_eq!(
            redact_tokens(r#"resp {"access_token":"abc","n":1}"#),
            r#"resp {"access_token":"<redacted>","n":1}"#
        );
        assert_eq!(
            redact_tokens(r#"{"api_key": 42, "token" : "a\"b", "tokens_used": 5}"#),
            r#"{"api_key": "<redacted>", "token" : "<redacted>", "tokens_used": 5}"#
        );
        // A JSON log line embedded, escaped, in another JSON string.
        assert_eq!(
            redact_tokens(r#"{"msg":"{\"password\":\"p w\"}"}"#),
            r#"{"msg":"{\"password\":\"<redacted>\"}"}"#
        );
    }

    #[test]
    fn url_passwords() {
        assert_eq!(
            redact_tokens("fatal: https://alice:hunter2@github.com/acme/app.git"),
            "fatal: https://alice:<redacted>@github.com/acme/app.git"
        );
        assert_eq!(
            redact_tokens("ssh://git@github.com/x"),
            "ssh://git@github.com/x"
        );
    }

    #[test]
    fn basic_auth_and_private_keys() {
        assert_eq!(
            redact_tokens("Authorization: Basic dXNlcjpwYXNz"),
            "Authorization: Basic <redacted>"
        );
        assert_eq!(
            redact_tokens(
                "key:\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEA\nAAAA\n-----END OPENSSH PRIVATE KEY-----\ndone"
            ),
            "key:\n<redacted>\ndone"
        );
    }

    #[test]
    fn scheme_after_a_secret_key_goes_with_the_value() {
        assert_eq!(
            redact_tokens("INFO http: authorization=Bearer abcdefgh12345678 url=/x"),
            "INFO http: authorization=<redacted> url=/x"
        );
        assert_eq!(
            redact_tokens("authorization=Basic dXNlcjpwYXNzd29yZA=="),
            "authorization=<redacted>"
        );
    }

    #[test]
    fn colon_forms_with_quoted_or_token_shaped_values() {
        assert_eq!(
            redact_tokens(
                r#"Config { password: "hunter22", api_key: 'k3y-value-123', name: "x" }"#
            ),
            r#"Config { password: "<redacted>", api_key: '<redacted>', name: "x" }"#
        );
        assert_eq!(
            redact_tokens("X-Api-Key: abc123def456"),
            "X-Api-Key: <redacted>"
        );
        assert_eq!(
            redact_tokens("Authorization: token abcdefghijkl"),
            "Authorization: token <redacted>"
        );
        for prose in [
            "token: missing",
            "password: required",
            "Secret: the cake is a lie",
        ] {
            assert_eq!(redact_tokens(prose), prose);
        }
    }

    #[test]
    fn secret_values_run_to_whitespace_or_their_closing_quote() {
        assert_eq!(
            redact_tokens("password=hunter,22;more x"),
            "password=<redacted> x"
        );
        assert_eq!(
            redact_tokens(r#"password="a\"b SECRET" x"#),
            "password=<redacted> x"
        );
        assert_eq!(
            redact_tokens("password=\"abc def SECRET (truncated\nnext line"),
            "password=<redacted>\nnext line"
        );
        // Inside a JSON string the closing quote still ends the value.
        assert_eq!(
            redact_tokens(r#"{"m":"password=abc","n":1}"#),
            r#"{"m":"password=<redacted>","n":1}"#
        );
        assert_eq!(
            redact_tokens(r#"{\"m\":\"token=ab\\cd\"}"#),
            r#"{\"m\":\"token=<redacted>\"}"#
        );
    }

    #[test]
    fn more_token_shapes() {
        for token in [
            "sk_live_51HabcDEFghiJKLmnoPQR",
            "rk_test_51HabcDEFghiJKLmnoPQR",
            "AIzaSyA1b2C3d4E5f6G7h8I9j0K1l2M3n4O5p6Q",
            "glpat-AbCdEfGhIjKlMnOpQrSt",
            "npm_AbCdEfGhIjKlMnOpQrStUvWxYz0123456789",
            "hf_AbCdEfGhIjKlMnOpQrStUvWxYz01234567",
            "xapp-1-A0123456789-0123456789012-abcdef",
        ] {
            assert_eq!(gone(&format!("k {token} x"), token), "k <redacted> x");
        }
        assert_eq!(
            redact_tokens("DB_PASS=hunter22 MYSQL_PWD=hunter22 compass=north"),
            "DB_PASS=<redacted> MYSQL_PWD=<redacted> compass=north"
        );
    }

    #[test]
    fn distinctive_tokens_glued_after_underscore() {
        assert_eq!(
            redact_tokens(
                "/cache/token_ghp_16C7e42F292c6912E7710c838347Ae178B4aAKIAIOSFODNN7EXAMPLE"
            ),
            "/cache/token_<redacted>"
        );
        // The second token only has a boundary once the first is replaced.
        assert_eq!(
            redact_tokens("AKIAIOSFODNN7EXAMPLEsk-ant-api03-AbCdEfGhIjKlMnOpQrStUv"),
            "<redacted><redacted>"
        );
        assert_eq!(
            redact_tokens("aws_id_AKIAIOSFODNN7EXAMPLE"),
            "aws_id_<redacted>"
        );
    }

    #[test]
    fn url_passwords_with_empty_user_or_odd_characters() {
        assert_eq!(
            redact_tokens("redis://:hunter22pw@localhost:6379/0"),
            "redis://:<redacted>@localhost:6379/0"
        );
        assert_eq!(
            redact_tokens("https://x-access-token:ab/cd+ef@github.com/a"),
            "https://x-access-token:<redacted>@github.com/a"
        );
        assert_eq!(
            redact_tokens("https://u:p@ss@host/x?e=a@b.com"),
            "https://u:<redacted>@host/x?e=a@b.com"
        );
    }

    #[test]
    fn secret_arrays_inside_text() {
        assert_eq!(
            redact_tokens(r#"{"token": ["sekretA","sekretB"], "n": 1}"#),
            r#"{"token": "<redacted>", "n": 1}"#
        );
    }

    #[test]
    fn labelled_private_key_blocks() {
        assert_eq!(
            redact_tokens("-----BEGIN PGP PRIVATE KEY BLOCK-----\nabc\n-----END PGP PRIVATE KEY BLOCK-----\nok"),
            "<redacted>\nok"
        );
    }

    #[test]
    fn command_line_flags_and_signed_urls() {
        assert_eq!(
            redact_tokens("run --token abc123secretvalue --verbose"),
            "run --token <redacted> --verbose"
        );
        assert_eq!(
            redact_tokens("GET /blob?sv=2024&sig=AbCd%2Bef&se=1 and X-Amz-Signature=abc123"),
            "GET /blob?sv=2024&sig=<redacted>&se=1 and X-Amz-Signature=<redacted>"
        );
    }

    #[test]
    fn rejected_keys_do_not_make_scrubbing_quadratic() {
        // Each `-` starts a new candidate key; retrying after every one of
        // them took minutes on 40 KB.
        let input = format!("{}tokens_used=5 password=x", "a-".repeat(5_000));
        let started = std::time::Instant::now();
        let out = redact_tokens(&input);
        assert!(out.ends_with("tokens_used=5 password=<redacted>"));
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn a_counter_prefix_does_not_hide_a_secret_key() {
        assert_eq!(
            redact_tokens("cache-github-tokens=ghs_secret input_tokens=12"),
            "cache-github-tokens=<redacted> input_tokens=12"
        );
    }

    #[test]
    fn every_suffix_of_a_rejected_key_is_rejected_too() {
        // `replace_secrets` skips a rejected key whole, which is only safe
        // if no suffix of it would have been accepted on its own.
        for key in [
            "tokens_used",
            "token_count",
            "max_tokens",
            "max-input-tokens",
            "cache_read_input_tokens",
            "x-total-tokens",
            "id-tokens-limit",
        ] {
            assert!(!is_secret_key(key), "{key}");
            for (i, c) in key.char_indices() {
                if c == '-' || c == '_' {
                    let suffix = &key[i + 1..];
                    assert!(!is_secret_key(suffix), "{key} -> {suffix}");
                }
            }
        }
    }

    #[test]
    fn secret_keys_holding_objects_or_arrays_in_text() {
        assert_eq!(
            redact_tokens(r#"{"secret": {"value": "s3cr3tvalue"}, "n":1}"#),
            r#"{"secret": "<redacted>", "n":1}"#
        );
        assert_eq!(
            redact_tokens(r#"x "client_secret":{"v": "hunter hunter"} y"#),
            r#"x "client_secret":"<redacted>" y"#
        );
        assert_eq!(
            redact_tokens(r#"{"token": ["a b", "c, d"], "n": 1}"#),
            r#"{"token": "<redacted>", "n": 1}"#
        );
        assert_eq!(
            redact_tokens(r#"{\"secret\":{\"v\":\"x y\"},\"n\":1}"#),
            r#"{\"secret\":\"<redacted>\",\"n\":1}"#
        );
    }

    #[test]
    fn strong_header_names_need_no_digit() {
        assert_eq!(
            redact_tokens("X-Api-Key: abcdefghijklmnopqrstuv"),
            "X-Api-Key: <redacted>"
        );
        assert_eq!(
            redact_tokens("Cookie: session=abcdefghijklmnop; Path=/"),
            "Cookie: <redacted>; Path=/"
        );
        for prose in [
            "Password: incorrect",
            "token: missing",
            "Authorization: required",
        ] {
            assert_eq!(redact_tokens(prose), prose);
        }
    }

    #[test]
    fn a_port_and_an_at_sign_in_the_query_are_not_userinfo() {
        let url = "https://example.com:8443/docs?u=bob@corp.com#top";
        assert_eq!(redact_tokens(url), url);
    }

    #[test]
    fn distinctive_tokens_right_after_a_digit() {
        assert_eq!(
            redact_tokens("1ghp_16C7e42F292c6912E7710c838347Ae178B4a"),
            "1<redacted>"
        );
        assert_eq!(redact_tokens("id9AKIAIOSFODNN7EXAMPLE"), "id9<redacted>");
        // Prefixes that occur inside ordinary words still need a boundary.
        assert_eq!(
            redact_tokens("task_live_abcdefghijklmnopqrstu"),
            "task_live_abcdefghijklmnopqrstu"
        );
    }

    #[test]
    fn emails_do_not_take_the_opening_quote() {
        assert_eq!(
            redact_emails("{'email': 'bob@x.io'}"),
            "{'email': '<email>'}"
        );
        assert_eq!(redact_emails("o'neil@x.io"), "<email>");
    }
}
