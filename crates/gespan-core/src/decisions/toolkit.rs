//! The `decisions` tools: how an agent reads, searches, proposes and flags
//! entries in the decision log. Agents can never approve, replace or reject.
use super::entry::{Edit, Entry, Error, Kind, NewEntry, Scope, Source, Status};
use super::render::{render_block, START_BUDGET};
use super::scope::scope_for;
use super::service;
use crate::tool_servers::{DECISIONS, DECISIONS_READ};
use mcp_hub::Grant;
use serde_json::{json, Map, Value};
use std::collections::BTreeSet;

const OFF: &str = "The decision log is off. The user can turn it on in Settings.";
const UNVERIFIED: &str = "Saved for review; not shown to other sessions because the answer wasn't found in your conversation with the user.";
const READ_ONLY: &str = "This session is read-only for the decision log: it can't propose entries. Use decisions_flag to raise a concern.";
const SEARCH_LIMIT: usize = 5;
const SEARCH_MAX: usize = 20;
/// Two titles, and two texts, this alike (share of their words) are the same
/// decision. A reversal ("tabs" to "spaces") changes the text and so differs.
const SAME_WORDS: f64 = 0.75;

/// The decision tools. Workers (`may_propose` false) get read tools and
/// `decisions_flag` only.
pub(crate) struct Toolkit {
    pub may_propose: bool,
}

impl mcp_hub::Toolkit for Toolkit {
    fn name(&self) -> &'static str {
        if self.may_propose {
            DECISIONS
        } else {
            DECISIONS_READ
        }
    }

    fn tools(&self, _: &Grant) -> Vec<Value> {
        let mut tools = vec![list_tool(), search_tool(), get_tool()];
        if self.may_propose {
            tools.push(propose_tool());
        }
        tools.push(flag_tool());
        tools
    }

    fn call(&self, grant: &Grant, tool: &str, input: Value) -> Result<Value, String> {
        if tool == "decisions_propose" && !self.may_propose {
            return Err(READ_ONLY.into());
        }
        if !service::enabled() {
            return Ok(plain(OFF));
        }
        let fields = fields(&input)?;
        let text = match tool {
            "decisions_list" => {
                only(&fields, &[])?;
                list(grant)?
            }
            "decisions_search" => {
                only(&fields, &["query", "limit"])?;
                let limit = match fields.get("limit") {
                    None | Some(Value::Null) => SEARCH_LIMIT,
                    Some(value) => value
                        .as_u64()
                        .ok_or("limit must be a whole number")?
                        .clamp(1, SEARCH_MAX as u64) as usize,
                };
                search(grant, required(&fields, "query")?, limit)?
            }
            "decisions_get" => {
                only(&fields, &["id"])?;
                get(grant, required(&fields, "id")?)?
            }
            "decisions_propose" => {
                only(
                    &fields,
                    &["title", "text", "why", "question", "user_answer"],
                )?;
                let (text, wrote) = propose(grant, &fields)?;
                if wrote {
                    service::changed();
                }
                text
            }
            "decisions_flag" => {
                only(&fields, &["id", "reason"])?;
                flag(
                    grant,
                    required(&fields, "id")?,
                    required(&fields, "reason")?,
                )?
            }
            _ => return Err("Unknown decisions tool".into()),
        };
        Ok(plain(&text))
    }

    /// A worker that saw `decisions_propose` elsewhere is told why it can't.
    fn routes(&self, _: &Grant, tool: &str) -> bool {
        !self.may_propose && tool == "decisions_propose"
    }
}

fn plain(text: &str) -> Value {
    json!({"content": [{"type": "text", "text": text}], "isError": false})
}

fn list_tool() -> Value {
    json!({
        "name": "decisions_list",
        "description": "List the user's preferences and this project's decisions from Gespan's decision log. Follow every approved one.",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false},
        "annotations": {"readOnlyHint": true, "openWorldHint": false},
    })
}

fn search_tool() -> Value {
    json!({
        "name": "decisions_search",
        "description": "Search the decision log for entries about a topic, best match first. Use it before making a design choice the log may already cover.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Words to look for"},
                "limit": {"type": "integer", "description": "How many entries at most (default 5, up to 20)"},
            },
            "required": ["query"],
            "additionalProperties": false,
        },
        "annotations": {"readOnlyHint": true, "openWorldHint": false},
    })
}

fn get_tool() -> Value {
    json!({
        "name": "decisions_get",
        "description": "Show one decision-log entry in full, with the entries it replaced.",
        "inputSchema": {
            "type": "object",
            "properties": {"id": {"type": "string", "description": "The entry's id, such as D-14 or P-3"}},
            "required": ["id"],
            "additionalProperties": false,
        },
        "annotations": {"readOnlyHint": true, "openWorldHint": false},
    })
}

fn propose_tool() -> Value {
    json!({
        "name": "decisions_propose",
        "description": "After the user answers a design question, propose the decision for their review. Pass the question, the user's exact words as user_answer, the decision (title and text) and why. Proposals are never active until the user approves them. Gespan checks that user_answer appears in the user's own messages in this session; a proposal that doesn't check out is saved for review but stays hidden from other sessions.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "title": {"type": "string", "description": "A short name for the decision"},
                "text": {"type": "string", "description": "The decision itself"},
                "why": {"type": "string", "description": "Why the user decided this (required)"},
                "question": {"type": "string", "description": "The question you asked the user"},
                "user_answer": {"type": "string", "description": "The user's answer, in their exact words"},
            },
            "required": ["title", "text", "why"],
            "additionalProperties": false,
        },
        "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false},
    })
}

fn flag_tool() -> Value {
    json!({
        "name": "decisions_flag",
        "description": "Flag an approved entry you think is wrong for the case in front of you. Keep following it; the user reviews the flag. Never stop your turn to ask about it.",
        "inputSchema": {
            "type": "object",
            "properties": {
                "id": {"type": "string", "description": "The entry's id, such as D-14"},
                "reason": {"type": "string", "description": "Why it seems wrong here"},
            },
            "required": ["id", "reason"],
            "additionalProperties": false,
        },
        "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false},
    })
}

fn fields(input: &Value) -> Result<Map<String, Value>, String> {
    match input {
        Value::Null => Ok(Map::new()),
        Value::Object(fields) => Ok(fields.clone()),
        _ => Err("Arguments must be an object".into()),
    }
}

fn only(fields: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    match fields.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(extra) => Err(format!("Unknown argument: {extra}")),
        None => Ok(()),
    }
}

fn required<'a>(fields: &'a Map<String, Value>, field: &str) -> Result<&'a str, String> {
    match optional(fields, field)? {
        Some(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(format!("{field} is required")),
    }
}

fn optional<'a>(fields: &'a Map<String, Value>, field: &str) -> Result<Option<&'a str>, String> {
    match fields.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value)),
        Some(_) => Err(format!("{field} must be a string")),
    }
}

fn now_ms() -> i64 {
    i64::try_from(crate::time::now_ms()).unwrap_or(i64::MAX)
}

/// The decision scope of the session's checkout. Every worktree of one
/// repository resolves to the same scope.
fn scope(grant: &Grant) -> Scope {
    scope_for(&grant.workspace)
}

fn failed(error: impl std::fmt::Display) -> String {
    format!("The decision log failed: {error}")
}

/// An entry this session may see: its project's or global, and, if pending,
/// rejected or archived, either carrying a user source or proposed by this
/// very session.
fn visible_to(entry: &Entry, scope: &Scope, session: i64) -> bool {
    let in_scope = entry.scope == Scope::Global || entry.scope == *scope;
    // Without a user source, only the proposer sees an entry that never became
    // (or no longer is) approved.
    let unapproved = matches!(
        entry.status,
        Status::Pending | Status::Rejected | Status::Archived
    );
    let hidden = unapproved && !has_user_source(entry) && entry.source.session != Some(session);
    in_scope && !hidden
}

fn has_user_source(entry: &Entry) -> bool {
    entry.source.question_ref.is_some() || entry.source.user_turn.is_some()
}

fn parse_id(id: &str) -> Result<(Option<Kind>, i64), String> {
    let bad = || format!("{id:?} isn't an entry id. Ids look like D-14 or P-3.");
    let id = id.trim();
    let (kind, digits) = match id.split_at_checked(2) {
        Some((prefix, rest)) if prefix.eq_ignore_ascii_case("d-") => (Some(Kind::Decision), rest),
        Some((prefix, rest)) if prefix.eq_ignore_ascii_case("p-") => (Some(Kind::Preference), rest),
        _ => (None, id),
    };
    digits.parse().map(|n| (kind, n)).map_err(|_| bad())
}

/// One entry as a line of text.
fn line(entry: &Entry) -> String {
    let state = match entry.status {
        Status::Pending => match entry.source.session {
            Some(session) if has_user_source(entry) => {
                format!("not yet approved — the user said this in session {session}")
            }
            _ => "not yet approved".to_owned(),
        },
        other => other.as_str().to_owned(),
    };
    format!(
        "{} ({state}): {} — {} (why: {})",
        entry.label(),
        entry.title,
        entry.text,
        entry.why
    )
}

fn list(grant: &Grant) -> Result<String, String> {
    let scope = scope(grant);
    let entries = service::with(|store| store.visible(&scope))?.map_err(failed)?;
    let (prefs, decisions): (Vec<Entry>, Vec<Entry>) = entries
        .into_iter()
        .partition(|entry| entry.kind == Kind::Preference);
    Ok(render_block(&prefs, &decisions, START_BUDGET).text)
}

fn search(grant: &Grant, query: &str, limit: usize) -> Result<String, String> {
    let scope = scope(grant);
    let hits = service::with(|store| store.search(&scope, query, limit))?.map_err(failed)?;
    if hits.is_empty() {
        return Ok("No matching decisions or preferences.".into());
    }
    Ok(hits.iter().map(line).collect::<Vec<_>>().join("\n"))
}

fn get(grant: &Grant, id: &str) -> Result<String, String> {
    let (kind, number) = parse_id(id)?;
    let scope = scope(grant);
    let found = service::with(|store| store.get(number))?.map_err(failed)?;
    let missing = || format!("No visible entry {id}. Call decisions_list or decisions_search.");
    let Some((entry, history)) = found else {
        return Ok(missing());
    };
    if kind.is_some_and(|kind| kind != entry.kind) || !visible_to(&entry, &scope, grant.session) {
        return Ok(missing());
    }
    let mut out = line(&entry);
    if let Some(answer) = &entry.user_answer {
        out.push_str(&format!("\nThe user's answer: {answer}"));
    }
    if entry.open_flags > 0 {
        out.push_str(&format!("\nOpen flags: {}", entry.open_flags));
    }
    for old in history
        .iter()
        .filter(|old| visible_to(old, &scope, grant.session))
    {
        out.push_str(&format!("\nReplaced: {}", line(old)));
    }
    Ok(out)
}

fn flag(grant: &Grant, id: &str, reason: &str) -> Result<String, String> {
    let (kind, number) = parse_id(id)?;
    let scope = scope(grant);
    let found = service::with(|store| store.get(number))?.map_err(failed)?;
    let visible = found.as_ref().is_some_and(|(entry, _)| {
        kind.is_none_or(|kind| kind == entry.kind) && visible_to(entry, &scope, grant.session)
    });
    let Some((entry, _)) = found.filter(|_| visible) else {
        return Ok(format!("No visible entry {id}. Nothing was flagged."));
    };
    if entry.status != Status::Active {
        return Err(format!(
            "{} is {}; only active entries can be flagged.",
            entry.label(),
            entry.status.as_str()
        ));
    }
    service::with(|store| store.flag(number, grant.session, reason, now_ms()))?.map_err(failed)?;
    service::changed();
    Ok(format!(
        "Flagged {}. The user will review it. Keep following the entry for now.",
        entry.label()
    ))
}

/// Words of a title, lower-cased.
fn words(title: &str) -> BTreeSet<String> {
    title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Whether two texts are the same but for a word or two.
fn same_words(a: &str, b: &str) -> bool {
    let (a, b) = (words(a), words(b));
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let shared = a.intersection(&b).count() as f64;
    shared / a.union(&b).count() as f64 >= SAME_WORDS
}

/// Trimmed, with every run of whitespace as one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether `answer` appears, as whole words, in something the user wrote in
/// `session`. Whitespace is collapsed; case matters. Agent text never counts: only the
/// registered source's user-authored texts are searched.
fn user_wrote(session: i64, answer: &str) -> bool {
    let answer = collapse(answer);
    !answer.is_empty()
        && service::user_texts(session)
            .iter()
            .any(|text| contains_words(&collapse(text), &answer))
}

/// Whether `needle` occurs in `haystack` with the start or end of the text, or
/// a character that is not a letter or digit, on both sides.
fn contains_words(haystack: &str, needle: &str) -> bool {
    haystack.match_indices(needle).any(|(start, found)| {
        let before = haystack[..start].chars().next_back();
        let after = haystack[start + found.len()..].chars().next();
        !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
    })
}

/// The proposal's reply, and whether it wrote to the log (a new proposal or
/// a revision) rather than leaving it as it was.
fn propose(grant: &Grant, fields: &Map<String, Value>) -> Result<(String, bool), String> {
    let title = required(fields, "title")?;
    let text = required(fields, "text")?;
    let why = required(fields, "why")?;
    let question = optional(fields, "question")?
        .map(collapse)
        .filter(|q| !q.is_empty());
    let answer = optional(fields, "user_answer")?
        .map(collapse)
        .filter(|a| !a.is_empty());
    let session = grant.session;
    let scope = scope(grant);
    let verified = answer.as_deref().is_some_and(|a| user_wrote(session, a));
    let note = if verified { "" } else { UNVERIFIED };

    let user_source = Source {
        session: Some(session),
        question_ref: verified.then(|| question.clone()).flatten(),
        user_turn: verified.then(|| answer.clone()).flatten(),
        import: None,
    };

    // The same decision, already in the log or already proposed? Every entry
    // the session can see is compared, not just the best search hits, so a
    // crowded log can't hide a duplicate; so are the caller's own drafts,
    // which nobody else sees.
    let candidates = service::with(|store| -> Result<Vec<Entry>, Error> {
        let mut found = store.visible(&scope)?;
        for draft in store.pending()? {
            if draft.scope == scope
                && draft.source.session == Some(session)
                && !found.iter().any(|entry| entry.id == draft.id)
            {
                found.push(draft);
            }
        }
        Ok(found)
    })?
    .map_err(failed)?;
    for existing in candidates
        .iter()
        .filter(|e| same_words(&e.title, title) && same_words(&e.text, text))
    {
        match existing.status {
            Status::Active => {
                return Ok((format!("already decided: {}", line(existing)), false));
            }
            Status::Pending => {
                let mine = existing.source.session == Some(session);
                let sourced = has_user_source(existing);
                // Only the proposing session rewrites a proposal, and one that
                // carries a user source needs the same proof a new one would.
                if !mine || (sourced && !verified) {
                    return Ok((
                        format!(
                            "Already proposed as {} and waiting for the user's review; left as it was.",
                            existing.label()
                        ),
                        false,
                    ));
                }
                if verified && !sourced {
                    // A draft can't take a user source later: file a fresh one.
                    continue;
                }
                let edit = Edit {
                    title: title.into(),
                    text: text.into(),
                    why: why.into(),
                };
                let revised = service::with(|store| {
                    store.revise_pending(
                        existing.id,
                        edit,
                        answer.as_deref().filter(|_| verified),
                        &user_source,
                    )
                })?
                .map_err(failed)?;
                return Ok((
                    with_note(
                        format!("Updated your pending proposal {}.", revised.label()),
                        note,
                    ),
                    true,
                ));
            }
            _ => {}
        }
    }

    let source = user_source;
    let new = NewEntry {
        kind: Kind::Decision,
        scope,
        title: title.into(),
        text: text.into(),
        why: why.into(),
        user_answer: answer.filter(|_| verified),
        source,
    };
    let entry =
        service::with(|store| store.propose(new, now_ms()))?.map_err(|error| match error {
            Error::MissingWhy => "why is required".to_owned(),
            other => failed(other),
        })?;
    Ok((
        with_note(
            format!("Proposed {} for the user to review.", entry.label()),
            note,
        ),
        true,
    ))
}

fn with_note(message: String, note: &str) -> String {
    if note.is_empty() {
        message
    } else {
        format!("{message} {note}")
    }
}
