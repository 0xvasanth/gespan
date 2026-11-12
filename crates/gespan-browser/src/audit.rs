//! Audit: what the agent did and where, never what
//! it saw. Four tables in the app's browser database:
//!
//! - `browser_audit`: one row per agent action with redacted parameters.
//! - `browser_events`: connection and permission events (who caused them).
//! - `browser_leases`: tabs the module opened, for diagnostics after a crash.
//!   Tabs from an earlier run are never re-adopted.
//! - `browser_settings`: the saved settings, the run counter and the
//!   fingerprint secret.
//!
//! The tables are Toasty models ([`models`]). The app opens the database
//! with them, applies their migrations and runs [`CONSTRAINTS`]; this module
//! never creates or alters a table.
//!
//! Retention is 30 days, pruned at startup; the user can clear everything
//! from Settings.

use hmac::{Hmac, KeyInit, Mac};
use serde_json::{json, Map, Value};
use sha2::Sha256;

use super::embedder::BrowserDb;
use super::policy::{audit_url, now_ms};
use super::protocol::{ActionKind, AuditEntry, BrowserSettings, Epoch, Outcome, Reason, TabId};

/// Rows older than 30 days are pruned.
pub const RETENTION_MS: i64 = 30 * 24 * 3600 * 1000;
/// Settings shows the last 50 rows per session.
pub const DEFAULT_LIST_LIMIT: u32 = 50;
pub const MAX_LIST_LIMIT: u32 = 1000;

/// One agent action.
#[derive(Debug, toasty::Model)]
#[table = "browser_audit"]
#[index(name = "browser_audit_ts", ts)]
pub(crate) struct AuditRow {
    #[key]
    #[auto]
    id: i64,
    ts: i64,
    session_id: String,
    request_id: String,
    global_epoch: String,
    session_epoch: String,
    action: String,
    tab_id: Option<String>,
    url: Option<String>,
    params: String,
    outcome: String,
    reason: Option<String>,
    duration_ms: i64,
    commands_sent: i64,
}

/// A connection or permission event.
#[derive(Debug, toasty::Model)]
#[table = "browser_events"]
#[index(name = "browser_events_ts", ts)]
pub(crate) struct EventRow {
    #[key]
    #[auto]
    id: i64,
    ts: i64,
    kind: String,
    actor: String,
    session_id: Option<String>,
    detail: Option<String>,
}

/// A tab the module opened.
#[derive(Debug, toasty::Model)]
#[table = "browser_leases"]
pub(crate) struct LeaseRow {
    #[key]
    tab_id: String,
    run_id: String,
    target_id: String,
    session_id: String,
    opened_by: Option<String>,
    url: String,
    created_at: i64,
    closed_at: Option<i64>,
}

/// One saved value (settings, run counter, secret).
#[derive(Debug, toasty::Model)]
#[table = "browser_settings"]
pub(crate) struct SettingRow {
    #[key]
    key: String,
    value: String,
}

/// Every table of the browser database, for the app to open it with.
pub fn models() -> toasty::ModelSet {
    toasty::models!(AuditRow, EventRow, LeaseRow, SettingRow)
}

/// What Toasty's generated DDL cannot say, run after the migrations on every
/// open. Idempotent.
///
/// - `browser_audit_session_ts` lists a session's rows newest first; Toasty
///   indexes have no descending columns.
pub const CONSTRAINTS: &[&str] = &[
    "CREATE INDEX IF NOT EXISTS browser_audit_session_ts ON browser_audit (session_id, ts DESC)",
];

const SETTINGS_KEY: &str = "settings";
const RUN_COUNTER_KEY: &str = "run_counter";
const SECRET_KEY: &str = "audit_secret";

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

pub fn record(db: &BrowserDb, row: &AuditRecord<'_>) -> toasty::Result<i64> {
    let create = AuditRow::create()
        .ts(now_ms())
        .session_id(row.session_id)
        .request_id(row.request_id)
        .global_epoch(row.global_epoch.to_string())
        .session_epoch(row.session_epoch.to_string())
        .action(row.action)
        .tab_id(row.tab.map(|t| t.0.clone()))
        .url(row.url.map(str::to_owned))
        .params(row.params.to_string())
        .outcome(outcome_str(row.outcome))
        .reason(row.reason.map(|r| r.as_str().to_owned()))
        .duration_ms(row.duration_ms)
        .commands_sent(row.commands_sent);
    let created = db.run(|mut db| async move { create.exec(&mut db).await })?;
    Ok(created.id)
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

impl AuditRow {
    fn entry(self) -> AuditEntry {
        AuditEntry {
            id: self.id,
            ts_ms: self.ts,
            session_id: self.session_id,
            request_id: self.request_id,
            global_epoch: self.global_epoch,
            session_epoch: self.session_epoch,
            action: self.action,
            tab: self.tab_id,
            url: self.url,
            params: serde_json::from_str(&self.params).unwrap_or(Value::Null),
            outcome: parse_outcome(&self.outcome),
            reason: self.reason.as_deref().and_then(parse_reason),
            duration_ms: self.duration_ms,
            commands_sent: self.commands_sent,
        }
    }
}

/// Newest first; `session` narrows to one session.
pub fn list(db: &BrowserDb, session: Option<&str>, limit: u32) -> toasty::Result<Vec<AuditEntry>> {
    let limit = limit.clamp(1, MAX_LIST_LIMIT) as usize;
    let row = AuditRow::fields();
    let query = match session {
        Some(session) => AuditRow::filter(row.session_id().eq(session)),
        None => AuditRow::all(),
    }
    .order_by(row.id().desc())
    .limit(limit);
    let rows = db.run(|mut db| async move { query.exec(&mut db).await })?;
    Ok(rows.into_iter().map(AuditRow::entry).collect())
}

/// Settings "clear audit": everything, including events and lease history.
pub fn clear(db: &BrowserDb) -> toasty::Result<()> {
    db.run(|mut db| async move {
        AuditRow::all().delete().exec(&mut db).await?;
        EventRow::all().delete().exec(&mut db).await?;
        LeaseRow::all().delete().exec(&mut db).await
    })
}

/// Drop rows older than the retention window. Returns rows removed.
pub fn prune(db: &BrowserDb, now_ms: i64) -> toasty::Result<usize> {
    let cutoff = now_ms - RETENTION_MS;
    // A delete returns no count, so each table counts its old rows first, in
    // the same transaction.
    let removed = db.run(|mut db| async move {
        let mut tx = db.transaction().await?;
        let audit = AuditRow::fields();
        let old_audit = AuditRow::filter(audit.ts().lt(cutoff));
        let event = EventRow::fields();
        let old_events = EventRow::filter(event.ts().lt(cutoff));
        // A lease ages from when it closed, or from when it opened while it
        // is still open.
        let lease = LeaseRow::fields();
        let old_leases = LeaseRow::filter(
            lease.closed_at().lt(Some(cutoff)).or(lease
                .closed_at()
                .is_none()
                .and(lease.created_at().lt(cutoff))),
        );
        let removed = old_audit.clone().count().exec(&mut tx).await?
            + old_events.clone().count().exec(&mut tx).await?
            + old_leases.clone().count().exec(&mut tx).await?;
        old_audit.delete().exec(&mut tx).await?;
        old_events.delete().exec(&mut tx).await?;
        old_leases.delete().exec(&mut tx).await?;
        tx.commit().await?;
        Ok(removed)
    })?;
    Ok(removed as usize)
}

/// A connection or permission event (`browser_events`).
pub fn event(
    db: &BrowserDb,
    kind: &str,
    actor: Actor,
    session: Option<&str>,
    detail: Option<&str>,
) -> toasty::Result<()> {
    let create = EventRow::create()
        .ts(now_ms())
        .kind(kind)
        .actor(actor.as_str())
        .session_id(session.map(str::to_owned))
        .detail(detail.map(str::to_owned));
    db.run(|mut db| async move { create.exec(&mut db).await })?;
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

pub fn events(db: &BrowserDb, limit: u32) -> toasty::Result<Vec<EventEntry>> {
    let limit = limit.clamp(1, MAX_LIST_LIMIT) as usize;
    let row = EventRow::fields();
    let query = EventRow::all().order_by(row.id().desc()).limit(limit);
    let rows = db.run(|mut db| async move { query.exec(&mut db).await })?;
    Ok(rows
        .into_iter()
        .map(|row| EventEntry {
            id: row.id,
            ts_ms: row.ts,
            kind: row.kind,
            actor: row.actor,
            session_id: row.session_id,
            detail: row.detail,
        })
        .collect())
}

pub fn lease_opened(
    db: &BrowserDb,
    run_id: u64,
    tab: &TabId,
    target_id: &str,
    session: &str,
    opened_by: Option<&TabId>,
    url: &str,
) -> toasty::Result<()> {
    // A tab id seen again replaces its row.
    let upsert = LeaseRow::upsert_by_tab_id(tab.0.clone())
        .run_id(format!("{run_id:x}"))
        .target_id(target_id)
        .session_id(session)
        .opened_by(opened_by.map(|t| t.0.clone()))
        .url(audit_url(url))
        .created_at(now_ms())
        .closed_at(None::<i64>);
    db.run(|mut db| async move { upsert.exec(&mut db).await })?;
    Ok(())
}

pub fn lease_closed(db: &BrowserDb, tab: &TabId) -> toasty::Result<()> {
    let row = LeaseRow::fields();
    let update = LeaseRow::filter(row.tab_id().eq(&tab.0).and(row.closed_at().is_none()))
        .update()
        .closed_at(Some(now_ms()));
    db.run(|mut db| async move { update.exec(&mut db).await })
}

/// Close every open row of `run_id` whose tab is not in `live`.
pub fn close_leases_except(
    db: &BrowserDb,
    run_id: u64,
    live: &std::collections::HashSet<String>,
) -> toasty::Result<usize> {
    let row = LeaseRow::fields();
    let open = LeaseRow::filter(
        row.closed_at()
            .is_none()
            .and(row.run_id().eq(format!("{run_id:x}"))),
    );
    let now = now_ms();
    // An update returns no count; in one transaction, the open rows read are
    // the rows closed.
    db.run(|mut db| async move {
        let mut tx = db.transaction().await?;
        let gone: Vec<String> = open
            .exec(&mut tx)
            .await?
            .into_iter()
            .map(|lease| lease.tab_id)
            .filter(|tab| !live.contains(tab))
            .collect();
        for tab in &gone {
            LeaseRow::filter_by_tab_id(tab)
                .update()
                .closed_at(Some(now))
                .exec(&mut tx)
                .await?;
        }
        tx.commit().await?;
        Ok(gone.len())
    })
}

pub fn lease_transferred(db: &BrowserDb, tab: &TabId, to: &str) -> toasty::Result<()> {
    let update = LeaseRow::filter_by_tab_id(&tab.0).update().session_id(to);
    db.run(|mut db| async move { update.exec(&mut db).await })
}

/// Leases of earlier runs still marked open: closed on startup because the
/// module never re-adopts tabs. Returns how many were closed out.
pub fn close_stale_leases(db: &BrowserDb, run_id: u64) -> toasty::Result<usize> {
    let row = LeaseRow::fields();
    let stale = LeaseRow::filter(
        row.closed_at()
            .is_none()
            .and(row.run_id().ne(format!("{run_id:x}"))),
    );
    let now = now_ms();
    // An update returns no count, so it counts the rows first, in the same
    // transaction.
    let closed = db.run(|mut db| async move {
        let mut tx = db.transaction().await?;
        let closed = stale.clone().count().exec(&mut tx).await?;
        stale.update().closed_at(Some(now)).exec(&mut tx).await?;
        tx.commit().await?;
        Ok(closed)
    })?;
    Ok(closed as usize)
}

#[cfg(test)]
pub fn open_lease_count(db: &BrowserDb) -> toasty::Result<i64> {
    let row = LeaseRow::fields();
    let count = LeaseRow::filter(row.closed_at().is_none()).count();
    let count = db.run(|mut db| async move { count.exec(&mut db).await })?;
    Ok(count as i64)
}

/// The value saved under `key`, if any.
fn setting(db: &BrowserDb, key: &'static str) -> toasty::Result<Option<String>> {
    let row =
        db.run(|mut db| async move { SettingRow::filter_by_key(key).first().exec(&mut db).await })?;
    Ok(row.map(|row| row.value))
}

/// Saves `value` under `key`, replacing what was there.
fn put_setting(db: &BrowserDb, key: &'static str, value: String) -> toasty::Result<()> {
    let upsert = SettingRow::upsert_by_key(key).value(value);
    db.run(|mut db| async move { upsert.exec(&mut db).await })?;
    Ok(())
}

/// Persisted module settings, the main switch included (`enabled`): a
/// remembered ON comes back at launch without connecting or starting
/// anything until an agent needs the browser, so attaching to the user's
/// own browser still asks for its consent every time.
pub fn load_settings(db: &BrowserDb) -> toasty::Result<Option<BrowserSettings>> {
    Ok(setting(db, SETTINGS_KEY)?.and_then(|t| serde_json::from_str(&t).ok()))
}

/// The browser settings row as a start finds it.
#[derive(Debug, PartialEq)]
pub enum StoredSettings {
    Saved(BrowserSettings),
    /// Nothing was ever saved: a fresh install.
    Missing,
    /// A row is there but cannot be read or parsed.
    Unreadable,
}

impl StoredSettings {
    /// What a start begins from. Only a fresh install gets
    /// [`BrowserSettings::first_run`]; a row that cannot be read starts from
    /// the restrictive defaults, so a damaged database never turns the
    /// browser on.
    pub fn start_from(&self) -> BrowserSettings {
        match self {
            StoredSettings::Saved(settings) => settings.clone(),
            StoredSettings::Missing => BrowserSettings::first_run(),
            StoredSettings::Unreadable => BrowserSettings::default(),
        }
    }
}

pub fn stored_settings(db: &BrowserDb) -> StoredSettings {
    match setting(db, SETTINGS_KEY) {
        Ok(None) => StoredSettings::Missing,
        Ok(Some(text)) => serde_json::from_str(&text)
            .map(StoredSettings::Saved)
            .unwrap_or(StoredSettings::Unreadable),
        Err(_) => StoredSettings::Unreadable,
    }
}

pub fn save_settings(db: &BrowserDb, settings: &BrowserSettings) -> toasty::Result<()> {
    put_setting(
        db,
        SETTINGS_KEY,
        serde_json::to_string(settings).unwrap_or_default(),
    )
}

/// Monotonic app-run counter so an epoch from an earlier run never equals a
/// current one. Returns the id for this run.
pub fn next_run_id(db: &BrowserDb) -> toasty::Result<u64> {
    let next = setting(db, RUN_COUNTER_KEY)?
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0)
        .saturating_add(1);
    put_setting(db, RUN_COUNTER_KEY, next.to_string())?;
    Ok(next)
}

/// Key of the text fingerprints. Random per install and kept in the
/// database (`audit::clear` leaves it), so a stored fingerprint cannot be
/// matched against a table of common secrets computed elsewhere. Created on
/// first use.
pub fn install_secret(db: &BrowserDb) -> [u8; 32] {
    let stored = || -> Option<[u8; 32]> { decode_secret(&setting(db, SECRET_KEY).ok()??) };
    if let Some(secret) = stored() {
        return secret;
    }
    let fresh = random_secret();
    let encoded: String = fresh.iter().map(|b| format!("{b:02x}")).collect();
    // Replaces a damaged value; a concurrent first start keeps whichever
    // was written first, which `stored` then returns. Raw SQL: a conditional
    // upsert (`ON CONFLICT ... WHERE`), which Toasty 0.11 cannot express.
    let upsert = toasty::sql::statement(
        "INSERT INTO browser_settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value
         WHERE length(browser_settings.value) <> 64",
    )
    .bind(SECRET_KEY)
    .bind(encoded);
    let _ = db.run(|mut db| async move { upsert.exec(&mut db).await });
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

/// Roles a `select` is known to be harmless on: pickers of fixed options.
/// Anything else, including a role that is unknown because the ref is stale
/// or gone, has its values redacted. Chrome's accessibility tree reports
/// `<input type=password>` as a plain `textbox`, so that cannot be on this list.
pub fn role_is_benign_for_select(role: &str) -> bool {
    matches!(
        role.to_ascii_lowercase().as_str(),
        "combobox" | "listbox" | "menu" | "menuitem" | "option"
    )
}

/// Parameters as the audit stores them: `text` becomes `{len, fp}` (see
/// [`text_fingerprint`]), `url` loses credentials, query and fragment, and
/// the `value` of `wait {for: text}` is fingerprinted the same way. `press`
/// keeps a named key (`Enter`, `Control+A`) but only the length of a typed
/// character or of any key aimed at a password field; `select` keeps only
/// the lengths of its values unless the target is positively known to be a
/// picker of fixed options (`target_role`, the role of its `ref`). `ref`, `paths`, `tab` and
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
    let select_is_benign = target_role.is_some_and(role_is_benign_for_select);
    let password_target = target_role.is_some_and(|r| r.to_ascii_lowercase().contains("password"));
    let mut out = Map::with_capacity(object.len());
    for (key, value) in object {
        let redacted = match (action, key.as_str()) {
            // `type` text, `dialog` prompt text: length and a keyed fingerprint only.
            (_, "text") => hash_text(value, secret),
            (ActionKind::Wait, "value") if waits_for_text => hash_text(value, secret),
            (ActionKind::Press, "key") => match value {
                Value::String(keys)
                    if password_target || super::semantic::press_types_text(keys) =>
                {
                    json!({ "len": keys.chars().count() })
                }
                other => other.clone(),
            },
            (ActionKind::Select, "values") if !select_is_benign => match value {
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

    fn conn() -> BrowserDb {
        BrowserDb::memory()
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
    fn leases_age_from_closing_and_reopen_and_close_by_run() {
        let conn = conn();
        let open = |run: u64, tab: &str| {
            lease_opened(
                &conn,
                run,
                &TabId(tab.into()),
                "T",
                "s1",
                None,
                "about:blank",
            )
            .unwrap();
        };
        let now = now_ms();
        let old = now - RETENTION_MS - 1000;
        // Opened long ago and still open: pruned by when it opened.
        open(0x1, "t_old_open");
        // Opened long ago, closed recently: kept, it ages from closing.
        open(0x1, "t_closed_recently");
        // Opened and closed long ago: pruned.
        open(0x1, "t_closed_long_ago");
        conn.execute(&format!("UPDATE browser_leases SET created_at = {old}"));
        conn.execute(&format!(
            "UPDATE browser_leases SET closed_at = {now} WHERE tab_id = 't_closed_recently'"
        ));
        conn.execute(&format!(
            "UPDATE browser_leases SET closed_at = {old} WHERE tab_id = 't_closed_long_ago'"
        ));
        assert_eq!(prune(&conn, now).unwrap(), 2);
        let left = conn.text("SELECT group_concat(tab_id) FROM browser_leases");
        assert_eq!(left, "t_closed_recently");

        // A tab id seen again replaces its row and is open again.
        open(0x2, "t_closed_recently");
        assert_eq!(open_lease_count(&conn).unwrap(), 1);
        let run = conn.text("SELECT run_id FROM browser_leases WHERE tab_id = 't_closed_recently'");
        assert_eq!(run, "2");

        // Only this run's tabs that are gone close, and only once.
        open(0x2, "t_live");
        open(0x2, "t_gone");
        open(0x3, "t_other_run");
        let live = ["t_live".to_string()].into_iter().collect();
        assert_eq!(close_leases_except(&conn, 0x2, &live).unwrap(), 2);
        assert_eq!(close_leases_except(&conn, 0x2, &live).unwrap(), 0);
        assert_eq!(open_lease_count(&conn).unwrap(), 2);
        assert_eq!(close_stale_leases(&conn, 0x2).unwrap(), 1);
        assert_eq!(close_stale_leases(&conn, 0x2).unwrap(), 0);
        assert_eq!(open_lease_count(&conn).unwrap(), 1);
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
        let url = conn.text("SELECT url FROM browser_leases WHERE tab_id = 't_00000001'");
        assert_eq!(url, "https://a.b/c");
        assert_eq!(open_lease_count(&conn).unwrap(), 1);
        lease_transferred(&conn, &tab, "s2").unwrap();
        let owner = conn.text("SELECT session_id FROM browser_leases WHERE tab_id = 't_00000001'");
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
        let old = now - RETENTION_MS - 1000;
        conn.execute(&format!("UPDATE browser_audit SET ts = {old}"));
        conn.execute(&format!(
            "UPDATE browser_events SET ts = {old} WHERE kind = 'connected'"
        ));
        let removed = prune(&conn, now).unwrap();
        assert_eq!(removed, 2);
        assert!(list(&conn, None, 10).unwrap().is_empty());
        assert_eq!(events(&conn, 10).unwrap().len(), 1);
        clear(&conn).unwrap();
        assert!(events(&conn, 10).unwrap().is_empty());
        assert_eq!(open_lease_count(&conn).unwrap(), 0);
    }

    #[test]
    fn only_a_missing_settings_row_starts_with_the_browser_on() {
        let conn = conn();
        assert_eq!(stored_settings(&conn), StoredSettings::Missing);
        let fresh = stored_settings(&conn).start_from();
        assert!(fresh.enabled);
        assert_eq!(fresh.access, crate::protocol::AccessMode::Always);

        // A damaged row falls back to off and Ask, never to the fresh-install
        // default.
        conn.execute("INSERT INTO browser_settings (key, value) VALUES ('settings', 'not json')");
        assert_eq!(stored_settings(&conn), StoredSettings::Unreadable);
        let damaged = stored_settings(&conn).start_from();
        assert!(!damaged.enabled);
        assert_eq!(damaged.access, crate::protocol::AccessMode::Ask);

        // A saved choice is kept as it is.
        let off = BrowserSettings {
            enabled: false,
            ..BrowserSettings::first_run()
        };
        save_settings(&conn, &off).unwrap();
        assert_eq!(stored_settings(&conn).start_from(), off);
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

    fn test_db() -> BrowserDb {
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
        conn.execute("INSERT INTO browser_settings (key, value) VALUES ('audit_secret', 'nothex')");
        let secret = install_secret(&conn);
        assert_ne!(secret, [0u8; 32]);
        assert_eq!(install_secret(&conn), secret);
    }

    /// The descending session index Toasty cannot declare comes from
    /// [`CONSTRAINTS`], and a second run of them changes nothing.
    #[test]
    fn the_session_index_lists_newest_first() {
        let conn = test_db();
        let sql = conn.text(
            "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = 'browser_audit_session_ts'",
        );
        assert!(sql.contains("(session_id, ts DESC)"), "{sql}");
        for constraint in CONSTRAINTS {
            conn.execute(constraint);
        }
        let count = conn.text(
            "SELECT CAST(count(*) AS TEXT) FROM sqlite_master WHERE name = 'browser_audit_session_ts'",
        );
        assert_eq!(count, "1");
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
        for key in [
            "shift+x",
            " x",
            "Space",
            "Shift+Space",
            "nonsense-key",
            "Alt+Shift+q",
        ] {
            let press = redact_params(
                ActionKind::Press,
                &json!({ "tab": "t_1", "key": key }),
                &secret,
                None,
            );
            let expected = if key == "Alt+Shift+q" {
                json!(key)
            } else {
                json!({ "len": key.chars().count() })
            };
            assert_eq!(press["key"], expected, "{key}");
        }
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
    fn only_known_pickers_keep_their_select_values() {
        for role in ["combobox", "ListBox", "menu", "option"] {
            assert!(role_is_benign_for_select(role), "{role}");
        }
        for role in ["password", "textbox", "searchbox", "button", ""] {
            assert!(!role_is_benign_for_select(role), "{role}");
        }
    }

    #[test]
    fn select_with_an_unknown_role_is_redacted() {
        let select = redact_params(
            ActionKind::Select,
            &json!({ "tab": "t_1", "ref": "e9", "values": ["hunter2"] }),
            &[3u8; 32],
            None,
        );
        assert_eq!(select["values"], json!([{ "len": 7 }]));
    }
}
