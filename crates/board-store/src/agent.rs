//! What agents read from and write to the board: write-back comments, the
//! "Run with…" prompt, the running set, and the card tools.
//!
//! Every scope here (which cards a session may touch) is read from the
//! database at call time from the session key the app supplies, never from
//! agent input.
use crate::comments::{add_comment, list_comments};
use crate::links::{board_links, title_from_ticket};
use crate::ops::{cards_for_link, db, get_card, write_tx};
use crate::settings::{board_settings, check_session_key};
use crate::subtasks::{get_subtask, list_subtasks, toggle_subtask_in};
use crate::Card;
use rusqlite::{params, Connection};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use std::collections::HashSet;

const MAX_WRITE_BACK: usize = 4_000;
const MAX_PROMPT: usize = 20_000;
const PROMPT_COMMENTS: usize = 10;

pub const READ_ONLY: &str = "This session is read-only";
pub const NOT_LINKED: &str = "This subtask isn't on a card linked to this session";
pub const WRITE_BACK_OFF: &str = "Write-back is turned off for this board";

/// `text` cut to at most `max` characters, the last one `…` when cut.
fn cap(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut cut: String = text.chars().take(max - 1).collect();
    cut.push('…');
    cut
}

/// Writes the turn's final reply as one comment, by `agent:<session_key>`, on
/// every live card linked to the session whose board has write-back on. The
/// reply is trimmed and cut to 4,000 characters; an empty reply writes
/// nothing. One failing card does not stop the others; the first error is
/// returned after every card was tried. Returns the ids of the cards written.
pub fn write_back(
    conn: &Connection,
    session_key: &str,
    summary: &str,
) -> Result<Vec<String>, String> {
    check_session_key(session_key)?;
    let author = format!("agent:{session_key}");
    let body = cap(summary.trim(), MAX_WRITE_BACK);
    if body.is_empty() {
        return Ok(Vec::new());
    }
    let mut written = Vec::new();
    let mut first_error = None;
    for card in cards_for_link(conn, "session", session_key)? {
        let result = board_settings(conn, &card.board_id).and_then(|settings| {
            if settings.write_back {
                add_comment(conn, &card.id, &body, &author).map(|_| true)
            } else {
                Ok(false)
            }
        });
        match result {
            Ok(true) => written.push(card.id),
            Ok(false) => {}
            Err(error) => {
                first_error.get_or_insert(error);
            }
        }
    }
    first_error.map_or(Ok(written), Err)
}

/// The board's live cards linked to a running session or Pilot. `running`
/// holds (`session` | `pilot`, id) pairs; other kinds never count.
pub fn running_card_ids(
    conn: &Connection,
    board_id: &str,
    running: &[(String, String)],
) -> Result<HashSet<String>, String> {
    if running.is_empty() {
        return Ok(HashSet::new());
    }
    let running: HashSet<(&str, &str)> = running
        .iter()
        .filter(|(kind, _)| kind == "session" || kind == "pilot")
        .map(|(kind, reference)| (kind.as_str(), reference.as_str()))
        .collect();
    Ok(board_links(conn, board_id)?
        .into_iter()
        .filter(|link| running.contains(&(link.kind.as_str(), link.reference.as_str())))
        .map(|link| link.card_id)
        .collect())
}

/// The card's title as agents may see it: `None` when the title was copied
/// from a ticket and not edited since (it stays marked through a ticket
/// rename, an unlink and a duplicate), or when any of the card's ticket links
/// has a snapshot title that is the same text (ignoring case and surrounding
/// space), since the title is then ticket content.
fn agent_title(conn: &Connection, card: &Card) -> Result<Option<String>, String> {
    if title_from_ticket(conn, &card.id)? {
        return Ok(None);
    }
    let title = card.title.trim();
    let wanted = title.to_lowercase();
    let mut statement = conn
        .prepare(
            "SELECT ref FROM board_card_links
             WHERE card_id = ?1 AND role = 'ticket' AND kind IN ('jira', 'linear', 'plane')",
        )
        .map_err(db)?;
    let tickets = statement
        .query_map([&card.id], |r| r.get::<_, String>(0))
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db)?;
    let copied = tickets.iter().any(|json| {
        serde_json::from_str::<Value>(json)
            .ok()
            .and_then(|value| value.get("title")?.as_str().map(str::to_string))
            .is_some_and(|ticket| ticket.trim().to_lowercase() == wanted)
    });
    Ok((!copied && !title.is_empty()).then(|| title.to_string()))
}

/// The first prompt of a session started from the card ("Run with…"): its
/// title, description, subtasks and the newest 10 user comments (oldest
/// first), empty sections left out, capped at 20,000 characters. Ticket
/// snapshot fields and agent-written comments are never included.
pub fn run_prompt(conn: &Connection, card_id: &str) -> Result<String, String> {
    let card = get_card(conn, card_id)?;
    let mut sections = vec![match agent_title(conn, &card)? {
        Some(title) => format!("Work on board card #{}: {title}", card.number),
        None => format!("Work on board card #{}", card.number),
    }];
    let description = card.body.trim();
    if !description.is_empty() {
        sections.push(description.to_string());
    }
    let subtasks = list_subtasks(conn, card_id)?;
    if !subtasks.is_empty() {
        let lines: Vec<String> = subtasks
            .iter()
            .map(|task| format!("- [{}] {}", if task.done { "x" } else { " " }, task.title))
            .collect();
        sections.push(format!("Subtasks:\n{}", lines.join("\n")));
    }
    let comments: Vec<String> = list_comments(conn, card_id)?
        .into_iter()
        .filter(|comment| comment.author == "user")
        .map(|comment| format!("- {}", comment.body))
        .collect();
    if !comments.is_empty() {
        let newest = &comments[comments.len().saturating_sub(PROMPT_COMMENTS)..];
        sections.push(format!("Comments:\n{}", newest.join("\n")));
    }
    Ok(cap(&sections.join("\n\n"), MAX_PROMPT))
}

/// A card tool call from an agent, tagged by `tool`. Unknown keys are
/// refused.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(from = "RawCardTool")]
pub enum CardTool {
    /// `card_subtasks`: the subtasks of every card linked to the session.
    Subtasks,
    /// `card_subtask_done {subtaskId, done}`.
    SubtaskDone { subtask_id: String, done: bool },
}

/// The wire shape. `card_subtasks` is an empty struct variant so serde
/// refuses extra keys on it too (it doesn't on a unit variant).
#[derive(Deserialize)]
#[serde(tag = "tool", deny_unknown_fields)]
enum RawCardTool {
    #[serde(rename = "card_subtasks")]
    Subtasks {},
    #[serde(rename = "card_subtask_done", rename_all = "camelCase")]
    SubtaskDone { subtask_id: String, done: bool },
}

impl From<RawCardTool> for CardTool {
    fn from(raw: RawCardTool) -> Self {
        match raw {
            RawCardTool::Subtasks {} => Self::Subtasks,
            RawCardTool::SubtaskDone { subtask_id, done } => Self::SubtaskDone { subtask_id, done },
        }
    }
}

impl CardTool {
    /// Parses a call by tool name and its arguments object.
    pub fn from_call(name: &str, args: &Value) -> Result<Self, String> {
        let mut object = match args {
            Value::Object(map) => map.clone(),
            Value::Null => Map::new(),
            _ => return Err("Tool arguments must be an object".into()),
        };
        if object.contains_key("tool") {
            return Err("unknown field `tool`".into());
        }
        object.insert("tool".into(), Value::String(name.to_string()));
        serde_json::from_value(Value::Object(object)).map_err(|error| error.to_string())
    }
}

/// Whether the live card is linked to the session, read now.
fn linked_to_session(conn: &Connection, card_id: &str, session_key: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM board_cards c
           WHERE c.id = ?1
             AND (c.id IN (SELECT card_id FROM board_card_links
                           WHERE kind = 'session' AND ref = ?2)
                  OR (c.link_kind = 'session' AND c.link_id = ?2))
             AND NOT EXISTS (SELECT 1 FROM board_card_meta m
               WHERE m.card_id = c.id AND m.archived_at IS NOT NULL))",
        params![card_id, session_key],
        |r| r.get(0),
    )
    .map_err(db)
}

/// Answers a card tool call for `session_key`. The cards in reach are the
/// live cards linked to the session right now. A read-only session may list
/// but not tick; ticking needs the card's board to have write-back on.
pub fn answer_card(
    conn: &Connection,
    session_key: &str,
    tool: &CardTool,
    read_only: bool,
) -> Result<Value, String> {
    check_session_key(session_key)?;
    match tool {
        CardTool::Subtasks => {
            // Only boards with write-back on share their cards with agents,
            // the same scope ticking a subtask has (ruling C6-k).
            let mut write_back_by_board = std::collections::HashMap::new();
            let mut cards = Vec::new();
            for card in cards_for_link(conn, "session", session_key)? {
                let write_back = match write_back_by_board.get(&card.board_id) {
                    Some(on) => *on,
                    None => {
                        let on = board_settings(conn, &card.board_id)?.write_back;
                        write_back_by_board.insert(card.board_id.clone(), on);
                        on
                    }
                };
                if !write_back {
                    continue;
                }
                let subtasks: Vec<Value> = list_subtasks(conn, &card.id)?
                    .into_iter()
                    .map(|task| json!({ "id": task.id, "title": task.title, "done": task.done }))
                    .collect();
                let mut entry = json!({ "number": card.number, "subtasks": subtasks });
                if let Some(title) = agent_title(conn, &card)? {
                    entry["title"] = Value::String(title);
                }
                cards.push(entry);
            }
            Ok(json!({ "cards": cards }))
        }
        CardTool::SubtaskDone { subtask_id, done } => {
            if read_only {
                return Err(READ_ONLY.into());
            }
            let tx = write_tx(conn)?;
            let Some(subtask) = get_subtask(&tx, subtask_id)? else {
                return Err(NOT_LINKED.into());
            };
            if !linked_to_session(&tx, &subtask.card_id, session_key)? {
                return Err(NOT_LINKED.into());
            }
            let card = get_card(&tx, &subtask.card_id)?;
            if !board_settings(&tx, &card.board_id)?.write_back {
                return Err(WRITE_BACK_OFF.into());
            }
            let actor = format!("agent:{session_key}");
            let board_id = toggle_subtask_in(&tx, &subtask, *done, &actor)?;
            tx.commit().map_err(db)?;
            Ok(json!({
                "boardId": board_id,
                "cardNumber": card.number,
                "subtask": { "id": subtask.id, "title": subtask.title, "done": *done },
            }))
        }
    }
}
