# `gespan-redact` Crate Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** A small shared crate, `crates/gespan-redact`, that scrubs home paths, project paths, emails, tokens and deny-listed fields from diagnostics text and JSON, plus fixture files both apps' tests can run against.

**Architecture:** One `Redactor` built from a `Config` (home directory, project roots, known secret values). `redact_text(&str) -> String` applies path, token and email rules in a fixed order. `redact_value(serde_json::Value) -> Value` drops deny-listed keys at any depth, blanks secret-named keys, and runs `redact_text` on every remaining key and string. Fixtures live in `fixtures/redaction.json`; an opt-in `fixtures` feature exposes a loader so `src-tauri` and `gespan-core` can run the same cases later.

**Tech Stack:** Rust 2021, `regex`, `serde`/`serde_json`, `proptest` (dev only).

**Spec:** GitHub issue #67 (approved design). Background: the "Scrubbing" section of epic #45. Working rules: #60.

## Global Constraints

- Crate path `crates/gespan-redact`, package name `gespan-redact`, `publish = false`, `edition = "2021"`, `license = "MIT"`. It follows the `crates/keep-awake` layout and is added to the root `Cargo.toml` workspace `members`.
- Placeholders, exactly: home → `~`; project root → `<project-N>` (1-based, the root's position in `Config::projects`), keeping everything after the root; email → `<email>`; token or secret → `<redacted>`.
- Token patterns: `sk-…`, `ghp_…` (and the other `gh?_` GitHub prefixes), `xox…`, `AKIA…`, JWTs, `Bearer …`, `password=` / `token=`.
- Deny-listed fields, dropped entirely: `prompt`, `content`, `diff`, `output`, `body`, `env`, `audio`, `transcript`.
- No dependency on the current machine: the home directory is always passed in, so tests are identical on macOS, Linux and Windows CI.
- Gates: `cargo test -p gespan-redact`, `bun run check`, `bun run native:check`. `Cargo.lock` is updated by cargo only.
- Gespan names only. Commits carry no tool or AI attribution.

## Design calls (mine, not in the issue)

- **Explicit home, no env lookup.** Callers already know the home directory (`dirs` in both apps). Passing it keeps the crate pure and testable.
- **Windows paths** (a drive letter or a backslash in the configured path) match ASCII-case-insensitively, and `\`, `/` and doubled `\\` (JSON-escaped text) all count as separators. POSIX paths match case-sensitively.
- **Path boundary.** A configured path only matches when the next character is not a letter, digit, `_` or `-`, so home `/Users/al` never eats `/Users/alice`. A home or project of `/`, `C:\` or empty is ignored, because it would redact every path.
- **Nested projects** match longest root first; numbering still follows list order.
- **Rule order:** project roots, home, known secrets, specific token patterns, `key=value` secrets, `Bearer`, emails. Paths go first so an `@` in a project folder name cannot break the path match.
- **Token boundaries.** Token patterns need a word boundary before them and a minimum length (`sk-` 20+, `gh?_` 20+, `xox?-` 10+, `AKIA`/`ASIA` + 16, JWT segments 5+, `Bearer` 8+). Without this, `risk-assessment-framework-v2` becomes `ri<redacted>`. The property test generates tokens on that contract.
- **`key=value` secrets** cover any key ending in `password`, `passwd`, `token`, `secret` or `api_key`/`apikey` (so `GITHUB_TOKEN=`, `access_token=`, `client_secret=` too), case-insensitive, `=` only (not `:`, which is too common in prose). Quoted values (which may span lines) and bare values up to whitespace, `&`, `;` or `,` are replaced; the key stays: `token=<redacted>`.
- **JSON keys named like secrets** (same key rule) keep the key and get the string `"<redacted>"` as their value, whatever its type. Text rules can't see JSON structure, so `{"token":"abc"}` would otherwise leak.
- **Known secrets** (`Config::secrets`, for keychain or account-store values, from the #45 scrubbing rules) are replaced literally. Values under 8 characters are ignored so a short value can't shred normal text.
- **Deny list** matches keys ASCII-case-insensitively and exactly (`prompt_id` survives).
- **Fixtures** are JSON (not TOML), so any later TypeScript test could read them too.

## Review Focus

1. A home that is a prefix of another user's home (`/Users/al` vs `/Users/alice`): must not match. Test in Task 2.
2. Windows text with mixed case, forward slashes, or JSON-escaped `\\` separators: still `~`. Fixture in Task 4.
3. Scrubbing already-scrubbed output (the bundle builder may run twice): output unchanged. Idempotence check in Task 4.
4. A token glued to a path or inside a URL query (`?access_token=…`, `/callback#token=…`): redacted. Fixture in Task 4.
5. Deny-listed fields nested inside arrays of objects: dropped. Test in Task 3.

---

### Task 1: Crate skeleton, paths and home

**Files:**
- Create: `crates/gespan-redact/Cargo.toml`, `crates/gespan-redact/src/lib.rs`, `crates/gespan-redact/src/paths.rs`
- Modify: `Cargo.toml` (root `members`), `Cargo.lock` (by cargo)

**Interfaces:**
- Produces:
  - `pub struct Config { pub home: Option<PathBuf>, pub projects: Vec<PathBuf>, pub secrets: Vec<String> }` (`Default`, `Clone`, `Debug`)
  - `pub struct Redactor`; `Redactor::new(config: Config) -> Redactor`; `Redactor::redact_text(&self, input: &str) -> String`
  - `pub const HOME: &str = "~"; pub const EMAIL: &str = "<email>"; pub const REDACTED: &str = "<redacted>";`
  - `paths::PathRule::new(path: &Path, placeholder: String) -> Option<PathRule>` (None for empty/root), `PathRule::apply(&self, input: &str) -> String`

- [ ] **Step 1: Write failing unit tests in `paths.rs` / `lib.rs`:** `home_becomes_tilde_on_macos` (`/Users/alice/Library/Logs/x.jsonl` → `~/Library/Logs/x.jsonl`), `home_on_linux`, `home_on_windows_case_and_separators` (`c:/users/ALICE\AppData` → `~/AppData`, and `C:\\Users\\Alice\\x` → `~\\x`), `home_prefix_of_other_user_is_left_alone` (home `/Users/al`, input `/Users/alice/x` unchanged), `project_keeps_relative_path` (`<project-1>/src/main.rs`), `nested_projects_longest_first` (projects `[/w/app, /w/app/sub]`: `/w/app/sub/a.rs` → `<project-2>/a.rs`), `project_under_home_wins_over_home`, `root_home_is_ignored`, `unicode_paths` (`/Users/zoë/项目/日本.rs`).
- [ ] **Step 2:** `cargo test -p gespan-redact` → fails to compile / FAIL.
- [ ] **Step 3: Implement.** Each `PathRule` is a `regex::Regex` built from the path's components (`regex::escape`), separators `/` for POSIX and `[\\/]+` plus `(?i)` for Windows; trailing separators trimmed. `apply` walks `find_iter` and skips matches whose next char is alphanumeric, `_` or `-`.
- [ ] **Step 4:** `cargo test -p gespan-redact` → PASS.
- [ ] **Step 5: Commit** `Add gespan-redact crate with home and project path scrubbing`.

### Task 2: Tokens, secrets and emails

**Files:** Create `crates/gespan-redact/src/patterns.rs`; modify `src/lib.rs`.

**Interfaces:** Produces `patterns::redact_tokens(input: &str) -> String`, `patterns::redact_emails(input: &str) -> String`, `patterns::is_secret_key(key: &str) -> bool`. Regexes are compiled once in `std::sync::LazyLock`.

- [ ] **Step 1: Failing tests**, one per pattern, each asserting the raw token is gone and the surrounding text is kept: `sk-ant-api03-…`, `sk-proj-…`, `ghp_` + 36, `github_pat_…`, `xoxb-…`, `AKIAIOSFODNN7EXAMPLE`, a three-part JWT, `Authorization: Bearer abc.def-123456` → `Authorization: Bearer <redacted>`, `password=hunter22&user=x` → `password=<redacted>&user=x`, `GITHUB_TOKEN="a b\nc"` (quoted, multi-line), `token=` (empty value left alone). Negative tests: `risk-assessment-framework-v2`, `task-runner`, `bearer of bad news`, `token: missing` are unchanged. Emails: `alice.o'neil+x@example.co.uk`, `zoë@exämple.de`, `git@github.com:org/repo` (redacted, accepted). Known secret `hunter2hunter2` replaced; a 4-char secret ignored.
- [ ] **Step 2:** run → FAIL.
- [ ] **Step 3: Implement** the patterns listed under Design calls and wire them into `redact_text` in the stated order.
- [ ] **Step 4:** run → PASS.
- [ ] **Step 5: Commit** `Scrub tokens, secrets and emails in gespan-redact`.

### Task 3: JSON values and the deny list

**Files:** Create `crates/gespan-redact/src/value.rs`; modify `src/lib.rs`.

**Interfaces:** Produces `pub const DENY_LIST: [&str; 8]` and `Redactor::redact_value(&self, value: serde_json::Value) -> serde_json::Value`.

- [ ] **Step 1: Failing tests:** `deny_listed_fields_are_dropped` (all eight names, plus `Prompt` mixed case), `similar_names_survive` (`prompt_id`, `contents`), `nested_in_arrays_are_dropped` (`{"turns":[{"prompt":"x","id":1}]}` → `{"turns":[{"id":1}]}`), `secret_named_keys_are_blanked` (`{"api_key": 42, "token":"t"}` → both `"<redacted>"`), `keys_and_strings_are_scrubbed` (a key that is a home path), `numbers_bools_null_untouched`.
- [ ] **Step 2:** run → FAIL. **Step 3:** implement recursively. **Step 4:** run → PASS.
- [ ] **Step 5: Commit** `Drop deny-listed fields from JSON in gespan-redact`.

### Task 4: Shared fixtures and the property test

**Files:** Create `crates/gespan-redact/fixtures/redaction.json`, `crates/gespan-redact/fixtures/README.md`, `crates/gespan-redact/src/fixtures.rs` (behind `feature = "fixtures"`), `crates/gespan-redact/tests/fixtures.rs`, `crates/gespan-redact/tests/property.rs`. Modify `Cargo.toml` (feature `fixtures = ["dep:serde"]`, dev-dep `proptest`, self dev-dep with the feature).

**Interfaces:** Produces (feature `fixtures`): `pub struct FixtureFile { pub contexts: BTreeMap<String, Config-like>, pub cases: Vec<Case> }`, `pub struct Case { pub name, pub context, pub kind: Kind /* Text | Json */, pub input: Value, pub expected: Value }`, `pub fn load() -> FixtureFile`, `FixtureFile::redactor(&self, context: &str) -> Redactor`, `Case::run(&self, &Redactor) -> Value`.

- [ ] **Step 1:** Write `redaction.json` with contexts `macos`, `linux`, `windows` and at least 25 cases: a log line per platform, a JSONL log record with `fields` holding `prompt`/`diff`/`env`, a multi-line panic message with paths and a token, a stack trace across several lines, Windows mixed/escaped separators, URL query tokens (`?access_token=`, `#token=`), Unicode home and project paths, an email next to a path, and negatives (`risk-assessment-framework-v2`, `/Users/alice2`).
- [ ] **Step 2:** `tests/fixtures.rs`: `every_fixture_matches` (collects every mismatch, reports names, asserts at least 25 cases ran) and `scrubbing_is_idempotent` (each expected output scrubbed again is unchanged). Run → FAIL before the loader exists.
- [ ] **Step 3:** `tests/property.rs` with proptest: for each token kind, a generated token on the documented contract, wrapped in arbitrary Unicode prefix/suffix separated by a non-word character, never appears in `redact_text` output and output contains `<redacted>`. Same for `Bearer` and `key=value`. Also `deny_listed_values_never_survive`: an arbitrary string under a deny-listed key never appears in the serialized output (when it's 6+ chars and not in the rest of the input).
- [ ] **Step 4:** implement the loader; `cargo test -p gespan-redact` → PASS. Fix any rule a fixture exposes, with its own unit test.
- [ ] **Step 5: Commit** `Add shared redaction fixtures and property tests`.

### Task 5: Gates

- [ ] `cargo test -p gespan-redact`, `cargo clippy -p gespan-redact --all-targets --all-features -- -D warnings`, `bun run check`, `env ZDOTDIR=$(mktemp -d) bun run native:check`. Quote result lines in the PR.

## Rulings during the build

An automated security review of the first commits, plus the shared fixtures, turned up gaps. Each fix below was test-driven:

- **Secret keys match anywhere in the name, not only at the end.** A key is secret if it mentions `password`, `passwd`, `secret`, `token`, `api key`, `private key`, `access key`, `credential`, `authorization` or `cookie`, ignoring case and punctuation. Before this, `AWS_SECRET_ACCESS_KEY=` leaked (a fixture caught it). Token counters (`tokens_used`, `token_count`) are excluded. Cost if wrong: harmless debug values like `password_hint` get redacted.
- **`key = value` allows spaces around `=`.** So `token= missing` now redacts `missing`; a bare `token=` with no value is still left alone. For a scrubber, over-redacting is the safer failure.
- **An unclosed quote still redacts** (`password="abc` from a truncated line).
- **JSON pairs inside text** (`"access_token":"…"`, also JSON-escaped) are redacted. Without this, a response body logged as a message leaks.
- **JSON strings that are themselves JSON objects or arrays** are parsed and scrubbed as JSON, so deny-listed fields inside them are dropped too.
- **Known secrets run first**, before the path rules, and also match their JSON-escaped and percent-encoded forms.
- **Added beyond #67's list:** URL passwords (`https://user:pw@host`), `Basic` auth (case-sensitive), and PEM private-key blocks, including a block cut off mid-way. They fall under the #45 rule "tokens and keys → `<redacted>`" and are cheap to add.

### After the whole-branch review

A fresh reviewer fed real inputs to the crate and found more ways for a secret to survive. Each fix below has a test that failed first:

- **`authorization=Bearer …` (the `tracing` field format):** the `key=` value now takes an optional `Bearer`/`Basic`/`token` scheme, and the `Bearer` and `Basic` rules run first. Before, only the scheme word was replaced and the token stayed.
- **Colon forms are now covered after all**, which reverses the earlier "declined" call. A quoted value after a colon (`password: "…"`, the form Rust's `Debug` writes) is always redacted. An unquoted value after a colon counts only when it has a scheme, or has 8+ characters including a digit, so `X-Api-Key: abc123…` goes and "token: missing" stays.
- **Where values end.** A bare value now runs to whitespace, `&` or a quote. Quoted values honour `\"` escapes. An unclosed quote runs to the end of the line.
- **Token counters are named explicitly:** plural `*tokens` keys with a usage prefix (`max`, `input`, `cache…`), or keys ending in `count`/`used`/`limit`/`usage`. `access_tokens` and `github_tokens` are secrets again. `pass`, `pwd`, `pw` and `sig` count as secret words, so `DB_PASS`, `MYSQL_PWD` and `dbPass` are caught, and so are signatures.
- **Deny-listed fields inside text** (cut-off or escaped JSON) get `"<redacted>"`, using a small scanner that understands JSON strings, nesting and escape levels (`src/fields.rs`).
- **More token shapes:** Stripe, Google, GitLab, npm, Hugging Face and Slack `xapp`. Tokens are now caught when glued after `_`; the left boundary is "not a letter or digit".
- **URL passwords** may contain `@` and `/`, and the user may be empty.
- **Other cheap fixes:** secret arrays in text, labelled PGP key blocks, `--token value` flags, lower-case and `+` URL encoding of known secrets, and dotted user names (`/Users/al.smith` is another user).
- **Repeat until stable.** The idempotence property found that one rule's placeholder can expose a match for another. `redact_text` now repeats its rules until the output stops changing (at most 5 passes). Because of this, the guarantee "scrubbing the output again changes nothing" holds by construction. The token rule also repeats, for tokens glued onto other tokens.
- **Rejected candidates no longer swallow text.** A candidate pair that isn't a secret (`compass: …`) only skips one character, so a real pair after it is still found.
- **Linear on crafted keys.** A rejected candidate resumes after its key. Restarting one character later took 110 s on a 10 KB key full of dashes (`rejected_keys_do_not_make_scrubbing_quadratic`).
- **Token counters are anchored at the end of the key** (`…inputtokens`, `maxtokens`, `…count`). That way every suffix of a counter key is a counter too, which is what makes skipping a rejected key safe; `cache-github-tokens=…` is a secret.
- **ASCII word boundaries** (`(?-u:\b)`) for speed. With Unicode `\b` the regex crate fell back to a slow engine on non-ASCII text: a mixed 1 MB log took 480 ms. It now takes 37 ms (release build).
- **Deferred minors:** boundary checks before a match (`/mnt/backup/Users/al` → `/mnt/backup~`, which over-redacts); Git Bash, WSL, `file://` and 8.3 forms of a Windows home; case-insensitive macOS paths; doubly escaped JSON; colliding scrubbed object keys; secrets in CLI argument arrays.

## Follow-ups (out of scope)

- Wiring into `src-tauri` and `gespan-core` (#72, #81, #124).
- `docs/diagnostics/scrubbing.md` (#66) should link to `crates/gespan-redact/fixtures/`.
- Cookie header values (`Cookie: a=b`) and Windows UNC paths (`\\server\share`) are not covered.
- The deferred minors listed under "After the whole-branch review".
