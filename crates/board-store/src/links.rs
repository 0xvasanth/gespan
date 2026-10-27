//! A card's links to sessions, pilots, runs, PRs, worktrees and tickets. The
//! single link both apps show today (`link_card`) is the row with role
//! `primary`; `board_cards.link_kind`/`link_id` mirror it for older readers.
use crate::ops::{
    clean_text, create_card_in, db, get_card, log_activity, touch, write_tx, NewActivity,
    LINK_KINDS, MAX_TITLE,
};
use crate::{now_millis, Card};
use rusqlite::{params, Connection, OptionalExtension, Transaction};
use serde::Serialize;

const MAX_REF: usize = 10_000;
/// The role of the card's single link, which only `link_card` writes.
const PRIMARY_ROLE: &str = "primary";

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CardLink {
    pub card_id: String,
    pub kind: String,
    /// The linked thing's id; for tickets, the `TicketRef` JSON.
    pub reference: String,
    pub role: Option<String>,
    pub created_at: i64,
}

fn log_link(
    tx: &rusqlite::Transaction<'_>,
    board_id: &str,
    card_id: &str,
    kind: &str,
    detail: &str,
) -> Result<(), String> {
    log_activity(
        tx,
        NewActivity {
            board_id,
            card_id: Some(card_id),
            kind,
            actor: "user",
            from_column: None,
            to_column: None,
            detail: Some(detail),
        },
    )
}

/// Adds an extra link; adding one the card already has changes nothing.
/// The primary link is set only through `link_card`, which keeps the legacy
/// link columns in step. Returns the owning board id.
pub fn add_link(
    conn: &Connection,
    card_id: &str,
    kind: &str,
    reference: &str,
    role: Option<&str>,
) -> Result<String, String> {
    if !LINK_KINDS.contains(&kind) {
        return Err(format!("Unknown link kind: {kind}"));
    }
    if is_ticket_kind(kind) {
        return Err(TICKET_KIND_ERROR.into());
    }
    if reference.is_empty() {
        return Err("Link id is required".into());
    }
    if reference.len() > MAX_REF {
        return Err("Link id is too long".into());
    }
    if role == Some(PRIMARY_ROLE) {
        return Err("Use link_card for the primary link".into());
    }
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let added = tx
        .execute(
            "INSERT OR IGNORE INTO board_card_links (card_id, kind, ref, role, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![card_id, kind, reference, role, now_millis()],
        )
        .map_err(db)?;
    if added == 0 {
        return Ok(card.board_id);
    }
    touch(&tx, card_id)?;
    log_link(
        &tx,
        &card.board_id,
        card_id,
        "linked",
        &format!("{kind}:{reference}"),
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

/// Removes a link. The card's primary link (the legacy `link_kind`/`link_id`
/// columns) is cleared only when the removed row had role `primary`, or when
/// there was no row and only those columns matched. Removing an extra link
/// that is also the primary (added first, then set as primary through
/// `link_card`) leaves the primary in place, now as its `primary` row, the
/// same state as setting the primary first. Returns the owning board id.
pub fn remove_link(
    conn: &Connection,
    card_id: &str,
    kind: &str,
    reference: &str,
) -> Result<String, String> {
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    // `None`: no row; `Some(role)`: the row's role.
    let row: Option<Option<String>> = tx
        .query_row(
            "SELECT role FROM board_card_links WHERE card_id = ?1 AND kind = ?2 AND ref = ?3",
            params![card_id, kind, reference],
            |r| r.get(0),
        )
        .optional()
        .map_err(db)?;
    let is_primary =
        card.link_kind.as_deref() == Some(kind) && card.link_id.as_deref() == Some(reference);
    match &row {
        None if !is_primary => return Ok(card.board_id),
        // A ticket row is never turned into the primary link.
        Some(role)
            if is_primary
                && role.as_deref() != Some(PRIMARY_ROLE)
                && role.as_deref() != Some(TICKET_ROLE) =>
        {
            tx.execute(
                "UPDATE board_card_links SET role = ?4
                 WHERE card_id = ?1 AND kind = ?2 AND ref = ?3",
                params![card_id, kind, reference, PRIMARY_ROLE],
            )
            .map_err(db)?;
        }
        Some(role) => {
            tx.execute(
                "DELETE FROM board_card_links WHERE card_id = ?1 AND kind = ?2 AND ref = ?3",
                params![card_id, kind, reference],
            )
            .map_err(db)?;
            if is_primary && role.as_deref() == Some(PRIMARY_ROLE) {
                clear_primary_columns(&tx, card_id)?;
            }
        }
        None => clear_primary_columns(&tx, card_id)?,
    }
    touch(&tx, card_id)?;
    log_link(
        &tx,
        &card.board_id,
        card_id,
        "unlinked",
        &format!("{kind}:{reference}"),
    )?;
    tx.commit().map_err(db)?;
    Ok(card.board_id)
}

fn clear_primary_columns(tx: &rusqlite::Transaction<'_>, card_id: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE board_cards SET link_kind = NULL, link_id = NULL WHERE id = ?1",
        [card_id],
    )
    .map(drop)
    .map_err(db)
}

pub fn card_links(conn: &Connection, card_id: &str) -> Result<Vec<CardLink>, String> {
    let mut statement = conn
        .prepare(
            "SELECT card_id, kind, ref, role, created_at FROM board_card_links
             WHERE card_id = ?1 ORDER BY kind, ref",
        )
        .map_err(db)?;
    let rows = statement
        .query_map([card_id], |r| {
            Ok(CardLink {
                card_id: r.get(0)?,
                kind: r.get(1)?,
                reference: r.get(2)?,
                role: r.get(3)?,
                created_at: r.get(4)?,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    rows
}

/// Every non-ticket link of the board's live cards: rows in
/// `board_card_links` of a kind other than a tracker, plus a link an older
/// build wrote only into the card's `link_kind`/`link_id` columns (as role
/// `primary`). For link status chips and the running set.
pub fn board_links(conn: &Connection, board_id: &str) -> Result<Vec<CardLink>, String> {
    let mut statement = conn
        .prepare(
            "SELECT l.card_id, l.kind, l.ref, l.role, l.created_at
             FROM board_card_links l JOIN board_cards c ON c.id = l.card_id
             WHERE c.board_id = ?1
               AND l.kind NOT IN ('jira', 'linear', 'plane')
               AND NOT EXISTS (SELECT 1 FROM board_card_meta m
                 WHERE m.card_id = c.id AND m.archived_at IS NOT NULL)
             UNION ALL
             SELECT c.id, c.link_kind, c.link_id, 'primary', c.updated_at
             FROM board_cards c
             WHERE c.board_id = ?1
               AND c.link_kind IS NOT NULL AND c.link_id IS NOT NULL
               AND c.link_kind NOT IN ('jira', 'linear', 'plane')
               AND NOT EXISTS (SELECT 1 FROM board_card_links l
                 WHERE l.card_id = c.id AND l.kind = c.link_kind AND l.ref = c.link_id)
               AND NOT EXISTS (SELECT 1 FROM board_card_meta m
                 WHERE m.card_id = c.id AND m.archived_at IS NOT NULL)
             ORDER BY 1, 2, 3",
        )
        .map_err(db)?;
    let rows = statement
        .query_map([board_id], |r| {
            Ok(CardLink {
                card_id: r.get(0)?,
                kind: r.get(1)?,
                reference: r.get(2)?,
                role: r.get(3)?,
                created_at: r.get(4)?,
            })
        })
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    rows
}

const TICKET_KINDS: [&str; 3] = ["jira", "linear", "plane"];
/// The `board_card_links` role of a ticket link. The queries below match on
/// it, so a plain link of the same kind is never read as a ticket.
const TICKET_ROLE: &str = "ticket";
pub(crate) const TICKET_KIND_ERROR: &str = "Use set_ticket for tracker tickets";

pub(crate) fn is_ticket_kind(kind: &str) -> bool {
    TICKET_KINDS.contains(&kind)
}

/// The link kind and key inside a `TicketRef` JSON; the store keeps the JSON
/// opaque otherwise, since its snapshot fields are the caller's to shape.
fn ticket_identity(json: &str) -> Result<(String, String), String> {
    if json.len() > MAX_REF {
        return Err("Ticket is too long".into());
    }
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("Invalid ticket: {e}"))?;
    let field = |name: &str| value.get(name).and_then(|v| v.as_str()).unwrap_or("");
    let (tracker, key) = (field("tracker"), field("key"));
    if !TICKET_KINDS.contains(&tracker) {
        return Err(format!("Unknown ticket tracker: {tracker}"));
    }
    if key.is_empty() {
        return Err("Ticket key is required".into());
    }
    Ok((tracker.to_string(), key.to_string()))
}

/// The card's ticket link as `TicketRef` JSON, if it has one.
pub fn ticket(conn: &Connection, card_id: &str) -> Result<Option<String>, String> {
    let mut statement = conn
        .prepare(
            "SELECT ref FROM board_card_links
             WHERE card_id = ?1 AND role = 'ticket' AND kind IN ('jira', 'linear', 'plane')
             ORDER BY created_at DESC, kind LIMIT 1",
        )
        .map_err(db)?;
    let mut rows = statement.query([card_id]).map_err(db)?;
    match rows.next().map_err(db)? {
        Some(row) => Ok(Some(row.get(0).map_err(db)?)),
        None => Ok(None),
    }
}

/// Every ticket link on the board as (card id, `TicketRef` JSON), for the
/// card faces.
pub fn board_tickets(conn: &Connection, board_id: &str) -> Result<Vec<(String, String)>, String> {
    let mut statement = conn
        .prepare(
            "SELECT l.card_id, l.ref FROM board_card_links l
             JOIN board_cards c ON c.id = l.card_id
             WHERE c.board_id = ?1 AND l.role = 'ticket' AND l.kind IN ('jira', 'linear', 'plane')
             ORDER BY l.card_id",
        )
        .map_err(db)?;
    let rows = statement
        .query_map([board_id], |r| Ok((r.get(0)?, r.get(1)?)))
        .map_err(db)?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(db);
    rows
}

/// Sets, replaces (`Some`) or removes (`None`) the card's one ticket link.
/// Re-saving the same tracker and key only refreshes the stored snapshot: no
/// activity row and no `updated_at` bump. Returns the owning board id.
pub fn set_ticket(
    conn: &Connection,
    card_id: &str,
    ticket_json: Option<&str>,
) -> Result<String, String> {
    let wanted = ticket_json.map(ticket_identity).transpose()?;
    let tx = write_tx(conn)?;
    let board_id = set_ticket_in(&tx, card_id, ticket_json, wanted)?;
    tx.commit().map_err(db)?;
    Ok(board_id)
}

/// [`set_ticket`] inside the caller's write transaction, which it leaves
/// uncommitted. `wanted` is `ticket_json`'s (tracker, key).
fn set_ticket_in(
    tx: &Transaction<'_>,
    card_id: &str,
    ticket_json: Option<&str>,
    wanted: Option<(String, String)>,
) -> Result<String, String> {
    let card = get_card(tx, card_id)?;
    let existing: Vec<(String, String)> = {
        let mut statement = tx
            .prepare(
                "SELECT kind, ref FROM board_card_links
                 WHERE card_id = ?1 AND role = 'ticket' AND kind IN ('jira', 'linear', 'plane')",
            )
            .map_err(db)?;
        let rows = statement
            .query_map([card_id], |r| Ok((r.get(0)?, r.get(1)?)))
            .map_err(db)?
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(db)?;
        rows
    };
    let same = |(kind, reference): &(String, String)| {
        wanted.as_ref().is_some_and(|(tracker, key)| {
            kind == tracker
                && ticket_identity(reference).is_ok_and(|(_, existing_key)| &existing_key == key)
        })
    };
    let mut changed = false;
    for row in existing.iter().filter(|row| !same(row)) {
        tx.execute(
            "DELETE FROM board_card_links
             WHERE card_id = ?1 AND kind = ?2 AND ref = ?3 AND role = 'ticket'",
            params![card_id, row.0, row.1],
        )
        .map_err(db)?;
        changed = true;
    }
    if let (Some(json), Some((tracker, key))) = (ticket_json, wanted.as_ref()) {
        match existing.iter().find(|row| same(row)) {
            Some((_, old)) if old == json => {}
            Some((kind, old)) => {
                tx.execute(
                    "UPDATE board_card_links SET ref = ?1
                     WHERE card_id = ?2 AND kind = ?3 AND ref = ?4 AND role = 'ticket'",
                    params![json, card_id, kind, old],
                )
                .map_err(db)?;
            }
            None => {
                tx.execute(
                    "INSERT INTO board_card_links (card_id, kind, ref, role, created_at)
                     VALUES (?1, ?2, ?3, ?5, ?4)",
                    params![card_id, tracker, json, now_millis(), TICKET_ROLE],
                )
                .map_err(db)?;
                log_link(
                    tx,
                    &card.board_id,
                    card_id,
                    "linked",
                    &format!("ticket: {key}"),
                )?;
                changed = true;
            }
        }
    } else if changed {
        log_link(tx, &card.board_id, card_id, "unlinked", "ticket removed")?;
    }
    if changed {
        touch(tx, card_id)?;
    }
    Ok(card.board_id)
}

/// "New card from ticket": a card in `column_id` titled with the ticket's
/// key, linked to the ticket and marked as titled from it, in one write.
pub fn create_card_from_ticket(
    conn: &Connection,
    column_id: &str,
    ticket_json: &str,
) -> Result<Card, String> {
    let (tracker, key) = ticket_identity(ticket_json)?;
    let tx = write_tx(conn)?;
    let card = create_card_in(&tx, column_id, &key, "")?;
    set_ticket_in(&tx, &card.id, Some(ticket_json), Some((tracker, key)))?;
    mark_title_from_ticket(&tx, &card.id)?;
    let card = get_card(&tx, &card.id)?;
    tx.commit().map_err(db)?;
    Ok(card)
}

/// Replaces a card's title with its ticket's fetched `title` while the card
/// is still titled with the ticket's `key` and still links that ticket, and
/// marks it as titled from the ticket. A title the user has changed, or a
/// card that no longer links the ticket, is left alone. Returns the owning
/// board id and whether the title changed.
pub fn retitle_card_from_ticket(
    conn: &Connection,
    card_id: &str,
    key: &str,
    title: &str,
) -> Result<(String, bool), String> {
    let title = clean_text(title, "Card title", MAX_TITLE)?;
    let tx = write_tx(conn)?;
    let card = get_card(&tx, card_id)?;
    let linked = ticket(&tx, card_id)?
        .and_then(|json| ticket_identity(&json).ok())
        .is_some_and(|(_, linked_key)| linked_key == key);
    if !linked || card.title != key {
        return Ok((card.board_id, false));
    }
    if card.title != title {
        tx.execute(
            "UPDATE board_cards SET title = ?1, updated_at = ?2 WHERE id = ?3",
            params![title, now_millis(), card_id],
        )
        .map_err(db)?;
        log_activity(
            &tx,
            NewActivity {
                board_id: &card.board_id,
                card_id: Some(card_id),
                kind: "edited",
                actor: "user",
                from_column: None,
                to_column: None,
                detail: None,
            },
        )?;
    }
    mark_title_from_ticket(&tx, card_id)?;
    tx.commit().map_err(db)?;
    Ok((card.board_id, card.title != title))
}

/// Whether the card's title was copied from a ticket and not edited since.
pub fn title_from_ticket(conn: &Connection, card_id: &str) -> Result<bool, String> {
    conn.query_row(
        "SELECT title_from_ticket FROM board_cards WHERE id = ?1",
        [card_id],
        |r| r.get::<_, i64>(0),
    )
    .optional()
    .map_err(db)?
    .map(|flag| flag != 0)
    .ok_or_else(|| "Card not found".to_string())
}

fn mark_title_from_ticket(tx: &Transaction<'_>, card_id: &str) -> Result<(), String> {
    tx.execute(
        "UPDATE board_cards SET title_from_ticket = 1 WHERE id = ?1",
        [card_id],
    )
    .map_err(db)
    .map(drop)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{create_board, create_card, ensure_tables, list_activity};

    fn setup() -> (Connection, String, String) {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        ensure_tables(&conn).unwrap();
        let board = create_board(&conn, "Work").unwrap();
        let column = crate::get_board(&conn, &board.id).unwrap().columns[0]
            .id
            .clone();
        let card = create_card(&conn, &column, "Card", "").unwrap();
        (conn, board.id, card.id)
    }

    fn json(tracker: &str, key: &str, title: Option<&str>) -> String {
        match title {
            Some(t) => format!(r#"{{"tracker":"{tracker}","key":"{key}","title":"{t}"}}"#),
            None => format!(r#"{{"tracker":"{tracker}","key":"{key}"}}"#),
        }
    }

    fn details(conn: &Connection, board: &str, card: &str) -> Vec<String> {
        let mut rows: Vec<String> = list_activity(conn, board, Some(card), 50)
            .unwrap()
            .into_iter()
            .filter_map(|a| a.detail)
            .filter(|d| d.starts_with("ticket"))
            .collect();
        rows.reverse();
        rows
    }

    #[test]
    fn card_ticket_link_round_trip() {
        let (conn, board, card) = setup();
        assert_eq!(ticket(&conn, &card).unwrap(), None);
        let first = json("jira", "ABC-1", None);
        assert_eq!(set_ticket(&conn, &card, Some(&first)).unwrap(), board);
        assert_eq!(
            ticket(&conn, &card).unwrap().as_deref(),
            Some(first.as_str())
        );
        assert_eq!(card_links(&conn, &card).unwrap()[0].kind, "jira");
        assert_eq!(details(&conn, &board, &card), ["ticket: ABC-1"]);
        set_ticket(&conn, &card, None).unwrap();
        assert_eq!(ticket(&conn, &card).unwrap(), None);
        assert!(card_links(&conn, &card).unwrap().is_empty());
        assert_eq!(
            details(&conn, &board, &card),
            ["ticket: ABC-1", "ticket removed"]
        );
    }

    #[test]
    fn setting_replaces_the_ticket_even_across_trackers() {
        let (conn, board, card) = setup();
        set_ticket(&conn, &card, Some(&json("jira", "ABC-1", None))).unwrap();
        let linear = json("linear", "ENG-2", None);
        set_ticket(&conn, &card, Some(&linear)).unwrap();
        assert_eq!(card_links(&conn, &card).unwrap().len(), 1);
        assert_eq!(
            ticket(&conn, &card).unwrap().as_deref(),
            Some(linear.as_str())
        );
        assert_eq!(
            details(&conn, &board, &card),
            ["ticket: ABC-1", "ticket: ENG-2"]
        );
    }

    #[test]
    fn noop_writes_no_activity_and_snapshot_refresh_is_silent() {
        let (conn, board, card) = setup();
        set_ticket(&conn, &card, None).unwrap();
        assert!(details(&conn, &board, &card).is_empty());
        let plain = json("plane", "WEB-3", None);
        set_ticket(&conn, &card, Some(&plain)).unwrap();
        set_ticket(&conn, &card, Some(&plain)).unwrap();
        let fetched = json("plane", "WEB-3", Some("Fetched title"));
        set_ticket(&conn, &card, Some(&fetched)).unwrap();
        assert_eq!(
            ticket(&conn, &card).unwrap().as_deref(),
            Some(fetched.as_str())
        );
        assert_eq!(card_links(&conn, &card).unwrap().len(), 1);
        assert_eq!(details(&conn, &board, &card), ["ticket: WEB-3"]);
    }

    #[test]
    fn linking_and_unlinking_touch_updated_at() {
        let (conn, _, card) = setup();
        conn.execute(
            "UPDATE board_cards SET updated_at = 1 WHERE id = ?1",
            [&card],
        )
        .unwrap();
        set_ticket(&conn, &card, Some(&json("jira", "ABC-1", None))).unwrap();
        let after_link: i64 = conn
            .query_row(
                "SELECT updated_at FROM board_cards WHERE id = ?1",
                [&card],
                |r| r.get(0),
            )
            .unwrap();
        assert!(after_link > 1);
        conn.execute(
            "UPDATE board_cards SET updated_at = 1 WHERE id = ?1",
            [&card],
        )
        .unwrap();
        set_ticket(&conn, &card, None).unwrap();
        let after_remove: i64 = conn
            .query_row(
                "SELECT updated_at FROM board_cards WHERE id = ?1",
                [&card],
                |r| r.get(0),
            )
            .unwrap();
        assert!(after_remove > 1);
    }

    #[test]
    fn rejects_bad_tickets_and_unknown_cards_without_writing() {
        let (conn, _, card) = setup();
        for bad in [
            "not json",
            r#"{"tracker":"github","key":"A-1"}"#,
            r#"{"tracker":"jira","key":""}"#,
        ] {
            assert!(set_ticket(&conn, &card, Some(bad)).is_err(), "{bad}");
        }
        assert!(set_ticket(&conn, "missing", Some(&json("jira", "A-1", None))).is_err());
        assert_eq!(ticket(&conn, &card).unwrap(), None);
    }

    #[test]
    fn board_tickets_lists_only_this_boards_ticket_links() {
        let (conn, board, card) = setup();
        let column = crate::get_board(&conn, &board).unwrap().columns[0]
            .id
            .clone();
        let plain = create_card(&conn, &column, "No ticket", "").unwrap();
        add_link(&conn, &plain.id, "session", "s-1", None).unwrap();
        let other = create_board(&conn, "Other").unwrap();
        let other_column = crate::get_board(&conn, &other.id).unwrap().columns[0]
            .id
            .clone();
        let elsewhere = create_card(&conn, &other_column, "Elsewhere", "").unwrap();
        let mine = json("jira", "ABC-1", None);
        set_ticket(&conn, &card, Some(&mine)).unwrap();
        set_ticket(&conn, &elsewhere.id, Some(&json("plane", "WEB-1", None))).unwrap();
        assert_eq!(board_tickets(&conn, &board).unwrap(), [(card, mine)]);
    }

    #[test]
    fn tracker_kinds_only_link_through_set_ticket() {
        let (conn, _, card) = setup();
        for kind in ["jira", "linear", "plane"] {
            assert_eq!(
                add_link(&conn, &card, kind, "ABC-1", Some("primary")).unwrap_err(),
                TICKET_KIND_ERROR
            );
            assert_eq!(
                crate::link_card(&conn, &card, Some(kind), Some("ABC-1")).unwrap_err(),
                TICKET_KIND_ERROR
            );
        }
        assert!(card_links(&conn, &card).unwrap().is_empty());
        assert_eq!(crate::get_card(&conn, &card).unwrap().link_kind, None);
    }

    #[test]
    fn ticket_queries_ignore_rows_that_are_not_ticket_links() {
        let (conn, board, card) = setup();
        // A row an older build or another writer left: tracker kind, plain key.
        conn.execute(
            "INSERT INTO board_card_links (card_id, kind, ref, role, created_at)
             VALUES (?1, 'jira', 'ABC-1', 'primary', 1)",
            [&card],
        )
        .unwrap();
        conn.execute(
            "UPDATE board_cards SET link_kind = 'jira', link_id = 'ABC-1' WHERE id = ?1",
            [&card],
        )
        .unwrap();
        assert_eq!(ticket(&conn, &card).unwrap(), None);
        assert!(board_tickets(&conn, &board).unwrap().is_empty());
        let mine = json("jira", "ABC-2", None);
        set_ticket(&conn, &card, Some(&mine)).unwrap();
        assert_eq!(
            ticket(&conn, &card).unwrap().as_deref(),
            Some(mine.as_str())
        );
        set_ticket(&conn, &card, None).unwrap();
        let rows: Vec<_> = card_links(&conn, &card).unwrap();
        assert_eq!(rows.len(), 1, "the other role's row survives");
        assert_eq!(rows[0].role.as_deref(), Some("primary"));
        let mirror = crate::get_card(&conn, &card).unwrap();
        assert_eq!(mirror.link_kind.as_deref(), Some("jira"));
    }

    #[test]
    fn other_links_are_left_alone() {
        let (conn, _, card) = setup();
        add_link(&conn, &card, "session", "s-1", None).unwrap();
        set_ticket(&conn, &card, Some(&json("jira", "ABC-1", None))).unwrap();
        set_ticket(&conn, &card, None).unwrap();
        let links = card_links(&conn, &card).unwrap();
        assert_eq!(links.len(), 1);
        assert_eq!(links[0].kind, "session");
    }
}
