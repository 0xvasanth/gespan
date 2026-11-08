//! Audit: what the agent did and where, never what
//! it saw. Three tables in the app's SQLite database:
//!
//! - `browser_audit`: one row per agent action with redacted parameters.
//! - `browser_events`: connection and permission events (who caused them).
//! - `browser_leases`: tabs the module opened, for diagnostics after a crash.
//!   Tabs from an earlier run are never re-adopted.
//!
//! Retention is 30 days, pruned at startup; the user can clear everything
//! from Settings.

use hmac::{Hmac, KeyInit, Mac};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Map, Value};
use sha2::Sha256;

use super::policy::{audit_url, now_ms};
use super::protocol::{ActionKind, AuditEntry, BrowserSettings, Epoch, Outcome, Reason, TabId};

/// Rows older than 30 days are pruned.
pub const RETENTION_MS: i64 = 30 * 24 * 3600 * 1000;
/// Settings shows the last 50 rows per session.
pub const DEFAULT_LIST_LIMIT: u32 = 50;
pub const MAX_LIST_LIMIT: u32 = 1000;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS browser_audit (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  session_id TEXT NOT NULL,
  request_id TEXT NOT NULL,
  global_epoch TEXT NOT NULL,
  session_epoch TEXT NOT NULL,
  action TEXT NOT NULL,
  tab_id TEXT,
  url TEXT,
  params TEXT NOT NULL,
  outcome TEXT NOT NULL,
  reason TEXT,
  duration_ms INTEGER NOT NULL,
  commands_sent INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS browser_audit_session_ts ON browser_audit (session_id, ts DESC);
CREATE INDEX IF NOT EXISTS browser_audit_ts ON browser_audit (ts);
CREATE TABLE IF NOT EXISTS browser_events (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  ts INTEGER NOT NULL,
  kind TEXT NOT NULL,
  actor TEXT NOT NULL,
  session_id TEXT,
  detail TEXT
);
CREATE INDEX IF NOT EXISTS browser_events_ts ON browser_events (ts);
CREATE TABLE IF NOT EXISTS browser_leases (
  tab_id TEXT PRIMARY KEY,
  run_id TEXT NOT NULL,
  target_id TEXT NOT NULL,
  session_id TEXT NOT NULL,
  opened_by TEXT,
  url TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  closed_at INTEGER
);
CREATE TABLE IF NOT EXISTS browser_settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

const SETTINGS_KEY: &str = "settings";
const RUN_COUNTER_KEY: &str = "run_counter";
const SECRET_KEY: &str = "audit_secret";

/// Create the tables. Idempotent; called from `browser::init`.
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(SCHEMA)
}

/// Who caused a `browser_events` row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Actor {
    User,
    Agent,
    System,
    Error,
}

impl Actor {
    pub fn as_str(self) -> &'static str {
        match self {
            Actor::User => "user",
            Actor::Agent => "agent",
            Actor::System => "system",
            Actor::Error => "error",
        }
    }
}

/// One `browser_audit` row before it is written.
#[derive(Clone, Debug)]
pub struct AuditRecord<'a> {
    pub session_id: &'a str,
    pub request_id: &'a str,
    pub global_epoch: Epoch,
    pub session_epoch: Epoch,
    pub action: &'a str,
    pub tab: Option<&'a TabId>,
    /// Already stripped of query and fragment (see [`audit_url`]).
    pub url: Option<&'a str>,
    /// Already redacted (see [`redact_params`]).
    pub params: &'a Value,
    pub outcome: Outcome,
    pub reason: Option<Reason>,
    pub duration_ms: i64,
    pub commands_sent: i64,
}

pub fn record(conn: &Connection, row: &AuditRecord<'_>) -> rusqlite::Result<i64> {
    conn.execute(
        "INSERT INTO browser_audit (ts, session_id, request_id, global_epoch, session_epoch, action, tab_id, url, params, outcome, reason, duration_ms, commands_sent)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
        params![
            now_ms(),
            row.session_id,
            row.request_id,
            row.global_epoch.to_string(),
            row.session_epoch.to_string(),
            row.action,
            row.tab.map(|t| t.0.as_str()),
            row.url,
            row.params.to_string(),
            outcome_str(row.outcome),
            row.reason.map(Reason::as_str),
            row.duration_ms,
            row.commands_sent,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn outcome_str(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Done => "done",
        Outcome::Blocked => "blocked",
        Outcome::Failed => "failed",
        Outcome::Unknown => "unknown",
    }
}

fn parse_outcome(text: &str) -> Outcome {
    match text {
        "done" => Outcome::Done,
        "blocked" => Outcome::Blocked,
        "failed" => Outcome::Failed,
        _ => Outcome::Unknown,
    }
}

fn parse_reason(text: &str) -> Option<Reason> {
    Reason::ALL.iter().copied().find(|r| r.as_str() == text)
}

/// Newest first; `session` narrows to one session.
pub fn list(
    conn: &Connection,
    session: Option<&str>,
    limit: u32,
) -> rusqlite::Result<Vec<AuditEntry>> {
    let limit = limit.clamp(1, MAX_LIST_LIMIT) as i64;
    let mut statement = conn.prepare(
        "SELECT id, ts, session_id, request_id, global_epoch, session_epoch, action, tab_id, url, params, outcome, reason, duration_ms, commands_sent
         FROM browser_audit
         WHERE (?1 IS NULL OR session_id = ?1)
         ORDER BY id DESC
         LIMIT ?2",
    )?;
    let rows = statement.query_map(params![session, limit], |row| {
        let params_text: String = row.get(9)?;
        let outcome: String = row.get(10)?;
        let reason: Option<String> = row.get(11)?;
        Ok(AuditEntry {
            id: row.get(0)?,
            ts_ms: row.get(1)?,
            session_id: row.get(2)?,
            request_id: row.get(3)?,
            global_epoch: row.get(4)?,
            session_epoch: row.get(5)?,
            action: row.get(6)?,
            tab: row.get(7)?,
            url: row.get(8)?,
            params: serde_json::from_str(&params_text).unwrap_or(Value::Null),
            outcome: parse_outcome(&outcome),
            reason: reason.as_deref().and_then(parse_reason),
            duration_ms: row.get(12)?,
            commands_sent: row.get(13)?,
        })
    })?;
    rows.collect()
}

/// Settings "clear audit": everything, including events and lease history.
pub fn clear(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "DELETE FROM browser_audit; DELETE FROM browser_events; DELETE FROM browser_leases;",
    )
}

/// Drop rows older than the retention window. Returns rows removed.
pub fn prune(conn: &Connection, now_ms: i64) -> rusqlite::Result<usize> {
    let cutoff = now_ms - RETENTION_MS;
    let mut removed = conn.execute("DELETE FROM browser_audit WHERE ts < ?1", params![cutoff])?;
    removed += conn.execute("DELETE FROM browser_events WHERE ts < ?1", params![cutoff])?;
    removed += conn.execute(
        "DELETE FROM browser_leases WHERE COALESCE(closed_at, created_at) < ?1",
        params![cutoff],
    )?;
    Ok(removed)
}

/// A connection or permission event (`browser_events`).
pub fn event(
    conn: &Connection,
    kind: &str,
    actor: Actor,
    session: Option<&str>,
    detail: Option<&str>,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO browser_events (ts, kind, actor, session_id, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
        params![now_ms(), kind, actor.as_str(), session, detail],
    )?;
    Ok(())
}

/// One `browser_events` row as Settings lists it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EventEntry {
    pub id: i64,
    pub ts_ms: i64,
    pub kind: String,
    pub actor: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

pub fn events(conn: &Connection, limit: u32) -> rusqlite::Result<Vec<EventEntry>> {
    let limit = limit.clamp(1, MAX_LIST_LIMIT) as i64;
    let mut statement = conn.prepare(
        "SELECT id, ts, kind, actor, session_id, detail FROM browser_events ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = statement.query_map(params![limit], |row| {
        Ok(EventEntry {
            id: row.get(0)?,
            ts_ms: row.get(1)?,
            kind: row.get(2)?,
            actor: row.get(3)?,
            session_id: row.get(4)?,
            detail: row.get(5)?,
        })
    })?;
    rows.collect()
}

pub fn lease_opened(
    conn: &Connection,
    run_id: u64,
    tab: &TabId,
    target_id: &str,
    session: &str,
    opened_by: Option<&TabId>,
    url: &str,
) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO browser_leases (tab_id, run_id, target_id, session_id, opened_by, url, created_at, closed_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL)",
        params![
            tab.0,
            format!("{run_id:x}"),
            target_id,
            session,
            opened_by.map(|t| t.0.as_str()),
            audit_url(url),
            now_ms(),
        ],
    )?;
    Ok(())
}

pub fn lease_closed(conn: &Connection, tab: &TabId) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE browser_leases SET closed_at = ?2 WHERE tab_id = ?1 AND closed_at IS NULL",
        params![tab.0, now_ms()],
    )?;
    Ok(())
}

/// Close every open row of `run_id` whose tab is not in `live`.
pub fn close_leases_except(
    conn: &Connection,
    run_id: u64,
    live: &std::collections::HashSet<String>,
) -> rusqlite::Result<usize> {
    let open: Vec<String> = {
        let mut statement = conn
            .prepare("SELECT tab_id FROM browser_leases WHERE closed_at IS NULL AND run_id = ?1")?;
        let rows = statement.query_map(params![format!("{run_id:x}")], |row| row.get(0))?;
        rows.collect::<rusqlite::Result<_>>()?
    };
    let now = now_ms();
    let mut closed = 0;
    for tab in open.iter().filter(|tab| !live.contains(*tab)) {
        closed += conn.execute(
            "UPDATE browser_leases SET closed_at = ?2 WHERE tab_id = ?1 AND closed_at IS NULL",
            params![tab, now],
        )?;
    }
    Ok(closed)
}

pub fn lease_transferred(conn: &Connection, tab: &TabId, to: &str) -> rusqlite::Result<()> {
    conn.execute(
        "UPDATE browser_leases SET session_id = ?2 WHERE tab_id = ?1",
        params![tab.0, to],
    )?;
    Ok(())
}

/// Leases of earlier runs still marked open: closed on startup because the
/// module never re-adopts tabs. Returns how many were closed out.
pub fn close_stale_leases(conn: &Connection, run_id: u64) -> rusqlite::Result<usize> {
    conn.execute(
        "UPDATE browser_leases SET closed_at = ?2 WHERE closed_at IS NULL AND run_id <> ?1",
        params![format!("{run_id:x}"), now_ms()],
    )
}

#[cfg(test)]
pub fn open_lease_count(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row(
        "SELECT COUNT(*) FROM browser_leases WHERE closed_at IS NULL",
        [],
        |row| row.get(0),
    )
    .optional()
    .map(|count| count.unwrap_or(0))
}

/// Persisted module settings, the main switch included (`enabled`): a
/// remembered ON comes back at launch without connecting or starting
/// anything until an agent needs the browser, so attaching to the user's
/// own browser still asks for its consent every time.
pub fn load_settings(conn: &Connection) -> rusqlite::Result<Option<BrowserSettings>> {
    let text: Option<String> = conn
        .query_row(
            "SELECT value FROM browser_settings WHERE key = ?1",
            params![SETTINGS_KEY],
            |row| row.get(0),
        )
        .optional()?;
    Ok(text.and_then(|t| serde_json::from_str(&t).ok()))
}

pub fn save_settings(conn: &Connection, settings: &BrowserSettings) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO browser_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![
            SETTINGS_KEY,
            serde_json::to_string(settings).unwrap_or_default()
        ],
    )?;
    Ok(())
}

/// Monotonic app-run counter so an epoch from an earlier run never equals a
/// current one. Returns the id for this run.
pub fn next_run_id(conn: &Connection) -> rusqlite::Result<u64> {
    let previous: Option<String> = conn
        .query_row(
            "SELECT value FROM browser_settings WHERE key = ?1",
            params![RUN_COUNTER_KEY],
            |row| row.get(0),
        )
        .optional()?;
    let next = previous
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_add(1);
    conn.execute(
        "INSERT INTO browser_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![RUN_COUNTER_KEY, next.to_string()],
    )?;
    Ok(next)
}

/// Key of the text fingerprints. Random per install and kept in the
/// database (`audit::clear` leaves it), so a stored fingerprint cannot be
/// matched against a table of common secrets computed elsewhere. Created on
/// first use.
pub fn install_secret(conn: &Connection) -> [u8; 32] {
    let stored = || -> Option<[u8; 32]> {
        let text: String = conn
            .query_row(
                "SELECT value FROM browser_settings WHERE key = ?1",
                params![SECRET_KEY],
                |row| row.get(0),
            )
            .optional()
            .ok()??;
        decode_secret(&text)
    };
    if let Some(secret) = stored() {
        return secret;
    }
    let fresh = random_secret();
    let encoded: String = fresh.iter().map(|b| format!("{b:02x}")).collect();
    // Replaces a damaged value; a concurrent first start keeps whichever
    // was written first, which `stored` then returns.
    let _ = conn.execute(
        "INSERT INTO browser_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value
         WHERE length(browser_settings.value) <> 64",
        params![SECRET_KEY, encoded],
    );
    stored().unwrap_or(fresh)
}

/// 32 random bytes from the OS (two v4 UUIDs, which the crate already has).
pub fn random_secret() -> [u8; 32] {
    let mut out = [0u8; 32];
    out[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    out
}

fn decode_secret(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 || !text.is_ascii() {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&text[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// HMAC-SHA256 of `text` under the install secret, first 8 bytes as 16 hex
/// characters. Lets the user see that two typed values were the same
/// without the log being a way to recover a short secret.
pub fn text_fingerprint(secret: &[u8; 32], text: &str) -> String {
    let mut mac =
        <Hmac<Sha256> as KeyInit>::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(text.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .take(8)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Roles whose content may be a secret. Chrome's accessibility tree reports
/// `<input type=password>` as a plain `textbox`, so a text box counts.
pub fn role_may_hold_secret(role: &str) -> bool {
    let role = role.to_ascii_lowercase();
    role.contains("password") || role == "textbox" || role == "searchbox"
}

/// Parameters as the audit stores them: `text` becomes `{len, fp}` (see
/// [`text_fingerprint`]), `url` loses credentials, query and fragment, and
/// the `value` of `wait {for: text}` is fingerprinted the same way. `press`
/// keeps a named key (`Enter`, `Control+A`) but only the length of a typed
/// character or of any key aimed at a password field; `select` keeps only
/// the lengths of its values when the target may hold a secret
/// (`target_role`, the role of its `ref`). `ref`, `paths`, `tab` and
/// everything else stay.
pub fn redact_params(
    action: ActionKind,
    input: &Value,
    secret: &[u8; 32],
    target_role: Option<&str>,
) -> Value {
    let Some(object) = input.as_object() else {
        return Value::Object(Map::new());
    };
    let waits_for_text = object.get("for").and_then(Value::as_str) == Some("text");
    let secret_target = target_role.is_some_and(role_may_hold_secret);
    let password_target = target_role.is_some_and(|r| r.to_ascii_lowercase().contains("password"));
    let mut out = Map::with_capacity(object.len());
    for (key, value) in object {
        let redacted = match (action, key.as_str()) {
            // `type` text, `dialog` prompt text: length and a keyed fingerprint only.
            (_, "text") => hash_text(value, secret),
            (ActionKind::Wait, "value") if waits_for_text => hash_text(value, secret),
            (ActionKind::Press, "key") => match value {
                Value::String(keys) if password_target || types_a_character(keys) => {
                    json!({ "len": keys.chars().count() })
                }
                other => other.clone(),
            },
            (ActionKind::Select, "values") if secret_target => match value {
                Value::Array(values) => Value::Array(
                    values
                        .iter()
                        .map(|v| match v {
                            Value::String(s) => json!({ "len": s.chars().count() }),
                            _ => json!({}),
                        })
                        .collect(),
                ),
                _ => json!({}),
            },
            // `fill`: the text of each field, like `type`.
            (ActionKind::Fill, "fields") => match value {
                Value::Array(fields) => Value::Array(
                    fields
                        .iter()
                        .map(|field| redact_params(ActionKind::Type, field, secret, None))
                        .collect(),
                ),
                other => other.clone(),
            },
            (_, "url") => match value {
                Value::String(url) => Value::String(audit_url(url)),
                other => other.clone(),
            },
            _ => value.clone(),
        };
        out.insert(key.clone(), redacted);
    }
    Value::Object(out)
}

/// `Modifier+...+Key` whose key is one printable character and whose
/// modifiers are at most Shift: the text of a keystroke, not a command
/// (`Control+A` is a command).
fn types_a_character(keys: &str) -> bool {
    // `Control++` presses "+": the last part is empty after the split.
    let (modifiers, key) = if keys == "+" {
        ("", "+")
    } else if let Some(modifiers) = keys.strip_suffix("++") {
        (modifiers, "+")
    } else {
        keys.rsplit_once('+').unwrap_or(("", keys))
    };
    key.chars().count() == 1 && modifiers.split('+').all(|m| m.is_empty() || m == "Shift")
}

fn hash_text(value: &Value, secret: &[u8; 32]) -> Value {
    let text = match value {
        Value::String(text) => text.as_str(),
        other => return other.clone(),
    };
    json!({ "len": text.chars().count(), "fp": text_fingerprint(secret, text) })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redact_params_for_tests(action: ActionKind, input: &Value) -> Value {
        redact_params(action, input, &[7u8; 32], None)
    }

    fn conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        migrate(&conn).unwrap();
        migrate(&conn).unwrap();
        conn
    }

    #[test]
    fn records_list_newest_first_and_filter_by_session() {
        let conn = conn();
        let tab = TabId("t_0000abcd".into());
        let params = json!({ "tab": "t_0000abcd", "ref": "e3", "text": { "len": 5, "fp": "deadbeefdeadbeef" } });
        for (i, session) in ["s1", "s1", "s2"].iter().enumerate() {
            let id = record(
                &conn,
                &AuditRecord {
                    session_id: session,
                    request_id: &format!("r{i}"),
                    global_epoch: Epoch { run: 0xab, seq: 1 },
                    session_epoch: Epoch {
                        run: 0xab,
                        seq: i as u64,
                    },
                    action: "type",
                    tab: Some(&tab),
                    url: Some("https://example.com/path"),
                    params: &params,
                    outcome: if i == 2 {
                        Outcome::Blocked
                    } else {
                        Outcome::Done
                    },
                    reason: (i == 2).then_some(Reason::NoLease),
                    duration_ms: 12 + i as i64,
                    commands_sent: 3,
                },
            )
            .unwrap();
            assert_eq!(id, i as i64 + 1);
        }
        let all = list(&conn, None, 50).unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].request_id, "r2");
        assert_eq!(all[0].outcome, Outcome::Blocked);
        assert_eq!(all[0].reason, Some(Reason::NoLease));
        assert_eq!(all[0].global_epoch, "ab:1");
        assert_eq!(all[2].session_epoch, "ab:0");
        assert_eq!(all[2].params["text"]["fp"], "deadbeefdeadbeef");
        assert_eq!(all[2].tab.as_deref(), Some("t_0000abcd"));
        assert_eq!(all[2].url.as_deref(), Some("https://example.com/path"));
        assert_eq!(all[2].commands_sent, 3);
        let s1 = list(&conn, Some("s1"), 1).unwrap();
        assert_eq!(s1.len(), 1);
        assert_eq!(s1[0].request_id, "r1");
        assert!(list(&conn, Some("nobody"), 50).unwrap().is_empty());
    }

    #[test]
    fn events_leases_prune_and_clear() {
        let conn = conn();
        event(&conn, "connected", Actor::User, None, Some("Chrome/144")).unwrap();
        event(&conn, "grant_on", Actor::User, Some("s1"), None).unwrap();
        let rows = events(&conn, 10).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].kind, "grant_on");
        assert_eq!(rows[0].actor, "user");
        assert_eq!(rows[0].session_id.as_deref(), Some("s1"));
        assert_eq!(rows[1].detail.as_deref(), Some("Chrome/144"));

        let tab = TabId("t_00000001".into());
        lease_opened(
            &conn,
            0x77,
            &tab,
            "T1",
            "s1",
            None,
            "https://a.b/c?secret=1#frag",
        )
        .unwrap();
        let url: String = conn
            .query_row(
                "SELECT url FROM browser_leases WHERE tab_id = 't_00000001'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(url, "https://a.b/c");
        assert_eq!(open_lease_count(&conn).unwrap(), 1);
        lease_transferred(&conn, &tab, "s2").unwrap();
        let owner: String = conn
            .query_row(
                "SELECT session_id FROM browser_leases WHERE tab_id = 't_00000001'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(owner, "s2");
        // A previous run's open lease is closed out at startup, ours is kept.
        lease_opened(
            &conn,
            0x11,
            &TabId("t_00000002".into()),
            "T2",
            "s1",
            Some(&tab),
            "about:blank",
        )
        .unwrap();
        assert_eq!(close_stale_leases(&conn, 0x77).unwrap(), 1);
        assert_eq!(open_lease_count(&conn).unwrap(), 1);
        lease_closed(&conn, &tab).unwrap();
        assert_eq!(open_lease_count(&conn).unwrap(), 0);

        // Prune: age the first audit/event rows past retention.
        record(
            &conn,
            &AuditRecord {
                session_id: "s1",
                request_id: "old",
                global_epoch: Epoch { run: 1, seq: 1 },
                session_epoch: Epoch { run: 1, seq: 1 },
                action: "click",
                tab: None,
                url: None,
                params: &json!({}),
                outcome: Outcome::Done,
                reason: None,
                duration_ms: 1,
                commands_sent: 1,
            },
        )
        .unwrap();
        let now = now_ms();
        conn.execute(
            "UPDATE browser_audit SET ts = ?1",
            params![now - RETENTION_MS - 1000],
        )
        .unwrap();
        conn.execute(
            "UPDATE browser_events SET ts = ?1 WHERE kind = 'connected'",
            params![now - RETENTION_MS - 1000],
        )
        .unwrap();
        let removed = prune(&conn, now).unwrap();
        assert_eq!(removed, 2);
        assert!(list(&conn, None, 10).unwrap().is_empty());
        assert_eq!(events(&conn, 10).unwrap().len(), 1);
        clear(&conn).unwrap();
        assert!(events(&conn, 10).unwrap().is_empty());
        assert_eq!(open_lease_count(&conn).unwrap(), 0);
    }

    #[test]
    fn settings_and_run_counter_persist() {
        let conn = conn();
        assert!(load_settings(&conn).unwrap().is_none());
        let mut settings = BrowserSettings {
            separate_window: true,
            upload_roots: vec!["/data".into()],
            ..BrowserSettings::default()
        };
        save_settings(&conn, &settings).unwrap();
        assert_eq!(load_settings(&conn).unwrap().unwrap(), settings);
        settings.limits.tabs_total = 3;
        save_settings(&conn, &settings).unwrap();
        assert_eq!(load_settings(&conn).unwrap().unwrap().limits.tabs_total, 3);
        assert_eq!(next_run_id(&conn).unwrap(), 1);
        assert_eq!(next_run_id(&conn).unwrap(), 2);
        clear(&conn).unwrap();
        // Clearing the audit keeps settings and the run counter.
        assert_eq!(next_run_id(&conn).unwrap(), 3);
        assert!(load_settings(&conn).unwrap().is_some());
    }

    #[test]
    fn redaction_hides_typed_text_and_query_strings_but_keeps_refs_and_paths() {
        let typed = redact_params_for_tests(
            ActionKind::Type,
            &json!({ "tab": "t_1", "ref": "e7", "text": "hunter2 pass", "submit": true }),
        );
        assert_eq!(typed["tab"], "t_1");
        assert_eq!(typed["ref"], "e7");
        assert_eq!(typed["submit"], true);
        assert_eq!(typed["text"]["len"], 12);
        let digest = typed["text"]["fp"].as_str().unwrap();
        assert_eq!(digest.len(), 16);
        assert!(digest.chars().all(|c| c.is_ascii_hexdigit()));
        assert!(typed.to_string().find("hunter2").is_none());
        let nav = redact_params_for_tests(
            ActionKind::Navigate,
            &json!({ "tab": "t_1", "url": "https://x.y/login?token=abc#frag" }),
        );
        assert_eq!(nav["url"], "https://x.y/login");
        let upload = redact_params_for_tests(
            ActionKind::Upload,
            &json!({ "tab": "t_1", "ref": "e1", "paths": ["/repo/a.png"] }),
        );
        assert_eq!(upload["paths"][0], "/repo/a.png");
        let wait = redact_params_for_tests(
            ActionKind::Wait,
            &json!({ "tab": "t_1", "for": "text", "value": "Welcome" }),
        );
        assert_eq!(wait["value"]["len"], 7);
        let wait_time = redact_params_for_tests(
            ActionKind::Wait,
            &json!({ "tab": "t_1", "for": "time", "value": 500 }),
        );
        assert_eq!(wait_time["value"], 500);
        let wait_url = redact_params_for_tests(
            ActionKind::Wait,
            &json!({ "tab": "t_1", "for": "url", "value": "https://x.y/done*" }),
        );
        assert_eq!(wait_url["value"], "https://x.y/done*");
        let dialog = redact_params_for_tests(
            ActionKind::Dialog,
            &json!({ "tab": "t_1", "action": "accept", "text": "secret" }),
        );
        assert_eq!(dialog["text"]["len"], 6);
        let fill = redact_params_for_tests(
            ActionKind::Fill,
            &json!({ "tab": "t_1", "fields": [
                { "ref": "e3", "text": "hunter2" },
                { "ref": "e4", "checked": true },
            ], "then_click": "e9" }),
        );
        assert_eq!(fill["fields"][0]["ref"], "e3");
        assert_eq!(fill["fields"][0]["text"]["len"], 7);
        assert_eq!(fill["fields"][1]["checked"], true);
        assert_eq!(fill["then_click"], "e9");
        assert!(fill.to_string().find("hunter2").is_none());
        assert_eq!(
            redact_params_for_tests(ActionKind::Click, &Value::Null),
            json!({})
        );
    }

    fn test_db() -> Connection {
        conn()
    }

    fn unsalted_sha256_prefix(text: &str) -> String {
        use sha2::{Digest, Sha256};
        Sha256::digest(text.as_bytes())
            .iter()
            .take(4)
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn typed_text_is_not_an_unsalted_hash() {
        let conn = test_db();
        let secret = install_secret(&conn);
        let stored = text_fingerprint(&secret, "1234");
        assert_eq!(stored.len(), 16);
        assert!(stored.chars().all(|c| c.is_ascii_hexdigit()));
        // Neither the old 8-hex prefix nor a prefix of the plain SHA-256
        // (which a rainbow table of short PINs inverts instantly).
        assert_ne!(stored[..8], unsalted_sha256_prefix("1234"));
        // Stable per install, and the same secret comes back on the next start.
        assert_eq!(stored, text_fingerprint(&secret, "1234"));
        assert_eq!(install_secret(&conn), secret);
        assert_ne!(stored, text_fingerprint(&secret, "1235"));
    }

    #[test]
    fn two_installs_fingerprint_differently() {
        let (a, b) = (test_db(), test_db());
        let (sa, sb) = (install_secret(&a), install_secret(&b));
        assert_ne!(sa, sb);
        assert_ne!(text_fingerprint(&sa, "1234"), text_fingerprint(&sb, "1234"));
    }

    #[test]
    fn the_secret_survives_clearing_the_audit() {
        let conn = test_db();
        let secret = install_secret(&conn);
        clear(&conn).unwrap();
        assert_eq!(install_secret(&conn), secret);
    }

    #[test]
    fn a_damaged_stored_secret_is_replaced_not_trusted() {
        let conn = test_db();
        conn.execute(
            "INSERT INTO browser_settings (key, value) VALUES ('audit_secret', 'nothex')",
            [],
        )
        .unwrap();
        let secret = install_secret(&conn);
        assert_ne!(secret, [0u8; 32]);
        assert_eq!(install_secret(&conn), secret);
    }

    #[test]
    fn redacted_text_carries_the_keyed_fingerprint() {
        let secret = [9u8; 32];
        let typed = redact_params(
            ActionKind::Type,
            &json!({ "ref": "e1", "text": "1234" }),
            &secret,
            None,
        );
        assert_eq!(typed["text"]["fp"], text_fingerprint(&secret, "1234"));
        assert_eq!(typed["text"]["len"], 4);
        assert!(typed["text"].get("sha256_8").is_none());
    }

    #[test]
    fn url_credentials_are_stripped() {
        assert_eq!(
            audit_url("https://alice:pw@example.com/a?q=1#f"),
            "https://example.com/a"
        );
        assert_eq!(
            audit_url("https://token@example.com:8443/a"),
            "https://example.com:8443/a"
        );
        // Unparseable input still loses its userinfo.
        assert_eq!(audit_url("alice:pw@host/x?y=1"), "host/x");
        let nav = redact_params(
            ActionKind::Navigate,
            &json!({ "url": "https://bob:hunter2@x.y/login?t=1" }),
            &[0u8; 32],
            None,
        );
        assert_eq!(nav["url"], "https://x.y/login");
    }

    #[test]
    fn press_and_select_on_a_password_field_store_only_lengths() {
        let secret = [3u8; 32];
        let press = redact_params(
            ActionKind::Press,
            &json!({ "tab": "t_1", "ref": "e3", "key": "Enter" }),
            &secret,
            Some("password"),
        );
        assert_eq!(press["key"], json!({ "len": 5 }));
        assert_eq!(press["ref"], "e3");
        let select = redact_params(
            ActionKind::Select,
            &json!({ "tab": "t_1", "ref": "e3", "values": ["hunter2", "x"] }),
            &secret,
            Some("Password"),
        );
        assert_eq!(select["values"], json!([{ "len": 7 }, { "len": 1 }]));
        assert!(!select.to_string().contains("hunter2"));
    }

    #[test]
    fn a_typed_character_is_never_stored_whatever_the_target() {
        let secret = [3u8; 32];
        // Chrome reports a password input as a plain textbox, so a printable
        // keystroke is redacted on any target, even with no ref at all.
        for role in [None, Some("textbox"), Some("button")] {
            let press = redact_params(
                ActionKind::Press,
                &json!({ "tab": "t_1", "key": "x" }),
                &secret,
                role,
            );
            assert_eq!(press["key"], json!({ "len": 1 }), "{role:?}");
        }
        let shifted = redact_params(
            ActionKind::Press,
            &json!({ "tab": "t_1", "key": "Shift+q" }),
            &secret,
            None,
        );
        assert_eq!(shifted["key"], json!({ "len": 7 }));
        let plus = redact_params(
            ActionKind::Press,
            &json!({ "tab": "t_1", "key": "+" }),
            &secret,
            None,
        );
        assert_eq!(plus["key"], json!({ "len": 1 }));
    }

    #[test]
    fn ordinary_keys_and_selects_stay_readable() {
        let secret = [3u8; 32];
        for key in [
            "Enter",
            "Tab",
            "Control+A",
            "Control++",
            "Meta+q",
            "ArrowDown",
            "Shift+Tab",
        ] {
            let press = redact_params(
                ActionKind::Press,
                &json!({ "tab": "t_1", "ref": "e2", "key": key }),
                &secret,
                Some("textbox"),
            );
            assert_eq!(press["key"], key);
        }
        let select = redact_params(
            ActionKind::Select,
            &json!({ "tab": "t_1", "ref": "e2", "values": ["fr"] }),
            &secret,
            Some("combobox"),
        );
        assert_eq!(select["values"], json!(["fr"]));
    }

    #[test]
    fn target_roles_that_can_hold_a_secret() {
        for role in ["password", "Password", "textbox", "searchbox"] {
            assert!(role_may_hold_secret(role), "{role}");
        }
        for role in ["button", "combobox", "listbox", "link", ""] {
            assert!(!role_may_hold_secret(role), "{role}");
        }
    }
}
