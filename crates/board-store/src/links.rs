//! A card's links to sessions, pilots, runs, PRs, worktrees and tickets. The
//! single link both apps show today (`link_card`) is the row with role
//! `primary`; `board_cards.link_kind`/`link_id` mirror it for older readers.
use crate::db::{args, err, execute, query, query_opt, with_db, with_tx, Conn, Ex, Row};
use crate::models::{CardRow, LinkRow};
use crate::ops::{
    check_body, clean_text, create_card_in, get_card_in, log_activity, touch, NewActivity,
    LINK_KINDS, MAX_TITLE,
};
use crate::{now_millis, Card};
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

fn link_from_row(r: &Row) -> Result<CardLink, String> {
    Ok(CardLink {
        card_id: r.get(0)?,
        kind: r.get(1)?,
        reference: r.get(2)?,
        role: r.get(3)?,
        created_at: r.get(4)?,
    })
}

async fn log_link(
    ex: &mut Ex<'_>,
    board_id: &str,
    card_id: &str,
    kind: &str,
    detail: &str,
) -> Result<(), String> {
    log_activity(
        ex,
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
    .await
}

/// Adds an extra link; adding one the card already has changes nothing.
/// The primary link is set only through `link_card`, which keeps the legacy
/// link columns in step. Returns the owning board id.
pub fn add_link(
    conn: &Conn,
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
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        let added = execute(
            ex,
            "INSERT OR IGNORE INTO board_card_links (card_id, kind, ref, role, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            args![card_id, kind, reference, role, now_millis()],
        )
        .await?;
        if added == 0 {
            return Ok(card.board_id);
        }
        touch(ex, card_id).await?;
        log_link(
            ex,
            &card.board_id,
            card_id,
            "linked",
            &format!("{kind}:{reference}"),
        )
        .await?;
        Ok(card.board_id)
    })
}

/// Removes a link. The card's primary link (the legacy `link_kind`/`link_id`
/// columns) is cleared only when the removed row had role `primary`, or when
/// there was no row and only those columns matched. Removing an extra link
/// that is also the primary (added first, then set as primary through
/// `link_card`) leaves the primary in place, now as its `primary` row, the
/// same state as setting the primary first. Returns the owning board id.
pub fn remove_link(
    conn: &Conn,
    card_id: &str,
    kind: &str,
    reference: &str,
) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        // `None`: no row; `Some(role)`: the row's role.
        let row: Option<Option<String>> =
            LinkRow::filter_by_card_id_and_kind_and_reference(card_id, kind, reference)
                .first()
                .exec(ex)
                .await
                .map_err(err)?
                .map(|link| link.role);
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
                execute(
                    ex,
                    "UPDATE board_card_links SET role = ?4
                     WHERE card_id = ?1 AND kind = ?2 AND ref = ?3",
                    args![card_id, kind, reference, PRIMARY_ROLE],
                )
                .await?;
            }
            Some(role) => {
                LinkRow::filter_by_card_id_and_kind_and_reference(card_id, kind, reference)
                    .delete()
                    .exec(ex)
                    .await
                    .map_err(err)?;
                if is_primary && role.as_deref() == Some(PRIMARY_ROLE) {
                    clear_primary_columns(ex, card_id).await?;
                }
            }
            None => clear_primary_columns(ex, card_id).await?,
        }
        touch(ex, card_id).await?;
        log_link(
            ex,
            &card.board_id,
            card_id,
            "unlinked",
            &format!("{kind}:{reference}"),
        )
        .await?;
        Ok(card.board_id)
    })
}

async fn clear_primary_columns(ex: &mut Ex<'_>, card_id: &str) -> Result<(), String> {
    execute(
        ex,
        "UPDATE board_cards SET link_kind = NULL, link_id = NULL WHERE id = ?1",
        args![card_id],
    )
    .await
    .map(drop)
}

pub fn card_links(conn: &Conn, card_id: &str) -> Result<Vec<CardLink>, String> {
    with_db!(conn, |ex| {
        query(
            ex,
            "SELECT card_id, kind, ref, role, created_at FROM board_card_links
             WHERE card_id = ?1 ORDER BY kind, ref",
            args![card_id],
        )
        .await?
        .iter()
        .map(link_from_row)
        .collect()
    })
}

/// Every non-ticket link of the board's live cards: rows in
/// `board_card_links` of a kind other than a tracker, plus a link an older
/// build wrote only into the card's `link_kind`/`link_id` columns (as role
/// `primary`). For link status chips and the running set.
pub(crate) async fn board_links_in(
    ex: &mut Ex<'_>,
    board_id: &str,
) -> Result<Vec<CardLink>, String> {
    query(
        ex,
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
        args![board_id],
    )
    .await?
    .iter()
    .map(link_from_row)
    .collect()
}

pub fn board_links(conn: &Conn, board_id: &str) -> Result<Vec<CardLink>, String> {
    with_db!(conn, |ex| board_links_in(ex, board_id).await)
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

async fn ticket_in(ex: &mut Ex<'_>, card_id: &str) -> Result<Option<String>, String> {
    match query_opt(
        ex,
        "SELECT ref FROM board_card_links
         WHERE card_id = ?1 AND role = 'ticket' AND kind IN ('jira', 'linear', 'plane')
         ORDER BY created_at DESC, kind LIMIT 1",
        args![card_id],
    )
    .await?
    {
        Some(row) => Ok(Some(row.get(0)?)),
        None => Ok(None),
    }
}

/// The card's ticket link as `TicketRef` JSON, if it has one.
pub fn ticket(conn: &Conn, card_id: &str) -> Result<Option<String>, String> {
    with_db!(conn, |ex| ticket_in(ex, card_id).await)
}

/// Every ticket link on the board as (card id, `TicketRef` JSON), for the
/// card faces.
pub fn board_tickets(conn: &Conn, board_id: &str) -> Result<Vec<(String, String)>, String> {
    with_db!(conn, |ex| {
        query(
            ex,
            "SELECT l.card_id, l.ref FROM board_card_links l
             JOIN board_cards c ON c.id = l.card_id
             WHERE c.board_id = ?1 AND l.role = 'ticket' AND l.kind IN ('jira', 'linear', 'plane')
             ORDER BY l.card_id",
            args![board_id],
        )
        .await?
        .iter()
        .map(|r| Ok((r.get(0)?, r.get(1)?)))
        .collect()
    })
}

/// Sets, replaces (`Some`) or removes (`None`) the card's one ticket link.
/// Re-saving the same tracker and key only refreshes the stored snapshot: no
/// activity row and no `updated_at` bump. Returns the owning board id.
pub fn set_ticket(conn: &Conn, card_id: &str, ticket_json: Option<&str>) -> Result<String, String> {
    let wanted = ticket_json.map(ticket_identity).transpose()?;
    with_tx!(conn, |ex| set_ticket_in(ex, card_id, ticket_json, wanted)
        .await)
}

/// [`set_ticket`] inside the caller's write transaction, which it leaves
/// uncommitted. `wanted` is `ticket_json`'s (tracker, key).
async fn set_ticket_in(
    ex: &mut Ex<'_>,
    card_id: &str,
    ticket_json: Option<&str>,
    wanted: Option<(String, String)>,
) -> Result<String, String> {
    let card = get_card_in(ex, card_id).await?;
    let existing: Vec<(String, String)> = query(
        ex,
        "SELECT kind, ref FROM board_card_links
         WHERE card_id = ?1 AND role = 'ticket' AND kind IN ('jira', 'linear', 'plane')",
        args![card_id],
    )
    .await?
    .iter()
    .map(|r| Ok((r.get(0)?, r.get(1)?)))
    .collect::<Result<_, String>>()?;
    let same = |(kind, reference): &(String, String)| {
        wanted.as_ref().is_some_and(|(tracker, key)| {
            kind == tracker
                && ticket_identity(reference).is_ok_and(|(_, existing_key)| &existing_key == key)
        })
    };
    let mut changed = false;
    for row in existing.iter().filter(|row| !same(row)) {
        execute(
            ex,
            "DELETE FROM board_card_links
             WHERE card_id = ?1 AND kind = ?2 AND ref = ?3 AND role = 'ticket'",
            args![card_id, &row.0, &row.1],
        )
        .await?;
        changed = true;
    }
    if let (Some(json), Some((tracker, key))) = (ticket_json, wanted.as_ref()) {
        match existing.iter().find(|row| same(row)) {
            Some((_, old)) if old == json => {}
            Some((kind, old)) => {
                execute(
                    ex,
                    "UPDATE board_card_links SET ref = ?1
                     WHERE card_id = ?2 AND kind = ?3 AND ref = ?4 AND role = 'ticket'",
                    args![json, card_id, kind, old],
                )
                .await?;
            }
            None => {
                LinkRow::create()
                    .card_id(card_id)
                    .kind(tracker)
                    .reference(json)
                    .role(Some(TICKET_ROLE.to_string()))
                    .created_at(now_millis())
                    .exec(ex)
                    .await
                    .map_err(err)?;
                log_link(
                    ex,
                    &card.board_id,
                    card_id,
                    "linked",
                    &format!("ticket: {key}"),
                )
                .await?;
                changed = true;
            }
        }
    } else if changed {
        log_link(ex, &card.board_id, card_id, "unlinked", "ticket removed").await?;
    }
    if changed {
        touch(ex, card_id).await?;
    }
    Ok(card.board_id)
}

/// "New card from ticket": a card in `column_id` titled with the ticket's
/// key, linked to the ticket and marked as titled from it, in one write.
pub fn create_card_from_ticket(
    conn: &Conn,
    column_id: &str,
    ticket_json: &str,
) -> Result<Card, String> {
    let (tracker, key) = ticket_identity(ticket_json)?;
    with_tx!(conn, |ex| {
        let card = create_card_in(ex, column_id, &key, "").await?;
        set_ticket_in(ex, &card.id, Some(ticket_json), Some((tracker, key))).await?;
        mark_title_from_ticket(ex, &card.id).await?;
        get_card_in(ex, &card.id).await
    })
}

/// Replaces a card's title with its ticket's fetched `title` while the card
/// is still titled with the ticket's `key` and still links that ticket, and
/// marks it as titled from the ticket. A title the user has changed, or a
/// card that no longer links the ticket, is left alone. Returns the owning
/// board id and whether the title changed.
pub fn retitle_card_from_ticket(
    conn: &Conn,
    card_id: &str,
    key: &str,
    title: &str,
) -> Result<(String, bool), String> {
    let title = clean_text(title, "Card title", MAX_TITLE)?;
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        let linked = ticket_in(ex, card_id)
            .await?
            .and_then(|json| ticket_identity(&json).ok())
            .is_some_and(|(_, linked_key)| linked_key == key);
        if !linked || card.title != key {
            return Ok((card.board_id, false));
        }
        if card.title != title {
            execute(
                ex,
                "UPDATE board_cards SET title = ?1, updated_at = ?2 WHERE id = ?3",
                args![&title, now_millis(), card_id],
            )
            .await?;
            log_activity(
                ex,
                NewActivity {
                    board_id: &card.board_id,
                    card_id: Some(card_id),
                    kind: "edited",
                    actor: "user",
                    from_column: None,
                    to_column: None,
                    detail: None,
                },
            )
            .await?;
        }
        mark_title_from_ticket(ex, card_id).await?;
        Ok((card.board_id, card.title != title))
    })
}

pub(crate) async fn title_from_ticket_in(ex: &mut Ex<'_>, card_id: &str) -> Result<bool, String> {
    CardRow::filter_by_id(card_id)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .map(|card| card.title_from_ticket)
        .ok_or_else(|| "Card not found".to_string())
}

/// Whether the card's title was copied from a ticket and not edited since.
pub fn title_from_ticket(conn: &Conn, card_id: &str) -> Result<bool, String> {
    with_db!(conn, |ex| title_from_ticket_in(ex, card_id).await)
}

/// Sets a card's description to text from a tracker and marks it, so it only
/// reaches an agent as untrusted text. Returns the owning board id.
pub fn set_body_from_ticket(conn: &Conn, card_id: &str, body: &str) -> Result<String, String> {
    check_body(body)?;
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        CardRow::filter_by_id(card_id)
            .update()
            .body(body)
            .body_from_ticket(!body.trim().is_empty())
            .updated_at(now_millis())
            .exec(ex)
            .await
            .map_err(err)?;
        Ok(card.board_id)
    })
}

pub(crate) async fn ticket_body_in(
    ex: &mut Ex<'_>,
    card_id: &str,
) -> Result<Option<String>, String> {
    let card = CardRow::filter_by_id(card_id)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .ok_or_else(|| "Card not found".to_string())?;
    let body = card.body.trim();
    Ok((card.body_from_ticket && !body.is_empty()).then(|| body.to_string()))
}

/// The card's trimmed description when it came from a tracker, for the caller
/// to wrap as untrusted text; `None` when it is empty or the user's own.
pub fn ticket_body(conn: &Conn, card_id: &str) -> Result<Option<String>, String> {
    with_db!(conn, |ex| ticket_body_in(ex, card_id).await)
}

async fn mark_title_from_ticket(ex: &mut Ex<'_>, card_id: &str) -> Result<(), String> {
    CardRow::filter_by_id(card_id)
        .update()
        .title_from_ticket(true)
        .exec(ex)
        .await
        .map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{fresh, Fixture};
    use crate::{create_board, create_card, list_activity};

    fn setup() -> (Fixture, String, String) {
        let conn = fresh();
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

    fn details(conn: &Fixture, board: &str, card: &str) -> Vec<String> {
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
        conn.raw()
            .execute(
                "UPDATE board_cards SET updated_at = 1 WHERE id = ?1",
                [&card],
            )
            .unwrap();
        set_ticket(&conn, &card, Some(&json("jira", "ABC-1", None))).unwrap();
        let after_link: i64 = conn
            .raw()
            .query_row(
                "SELECT updated_at FROM board_cards WHERE id = ?1",
                [&card],
                |r| r.get(0),
            )
            .unwrap();
        assert!(after_link > 1);
        conn.raw()
            .execute(
                "UPDATE board_cards SET updated_at = 1 WHERE id = ?1",
                [&card],
            )
            .unwrap();
        set_ticket(&conn, &card, None).unwrap();
        let after_remove: i64 = conn
            .raw()
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
        conn.raw()
            .execute(
                "INSERT INTO board_card_links (card_id, kind, ref, role, created_at)
             VALUES (?1, 'jira', 'ABC-1', 'primary', 1)",
                [&card],
            )
            .unwrap();
        conn.raw()
            .execute(
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
