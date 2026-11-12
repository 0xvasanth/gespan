use crate::db::{args, err, query, with_db, with_tx, Conn, Ex};
use crate::links::TICKET_KIND_ERROR;
use crate::models::{ActivityRow, BoardRow, CardMetaRow, CardRow, ColumnRow, LinkRow};
use crate::{now_millis, Activity, Board, BoardDetail, Card, Column};
use toasty::stmt::{Expr, List, Query};
use uuid::Uuid;

pub(crate) const MAX_NAME: usize = 200;
pub(crate) const MAX_TITLE: usize = 500;
const MAX_BODY: usize = 1_000_000;
const MAX_ACTIVITY: usize = 500;

const ROLES: [&str; 5] = ["todo", "doing", "review", "done", "cancelled"];
/// What a card can link to. Ticket kinds (`jira`, `linear`, `plane`) carry the
/// ticket's `TicketRef` JSON as their reference.
pub const LINK_KINDS: [&str; 8] = [
    "session", "pilot", "flow-run", "pr", "worktree", "jira", "linear", "plane",
];
/// The `board_card_links` role of the link `link_card` manages, so the single
/// link both apps show today stays one row among a card's links.
const PRIMARY: &str = "primary";
/// SQL true for a card that is not archived, for the raw queries left.
/// Archived cards leave their column's order; live cards keep positions
/// 0..n-1 without them. [`live`] is the same test for typed queries.
pub(crate) const LIVE: &str = "NOT EXISTS (SELECT 1 FROM board_card_meta m
   WHERE m.card_id = board_cards.id AND m.archived_at IS NOT NULL)";
const ACTORS: [&str; 2] = ["user", "system"];
/// Kinds `board_card_record` may append. Every other kind is written by the
/// mutation that causes it.
const RECORD_KINDS: [&str; 6] = [
    "started",
    "needs_input",
    "completed",
    "failed",
    "pr_opened",
    "pr_merged",
];

/// The ids of archived cards, as a subquery.
pub(crate) fn archived_ids() -> Query<List<String>> {
    CardMetaRow::filter(CardMetaRow::fields().archived_at().is_some())
        .select(CardMetaRow::fields().card_id())
}

/// A [`CardRow`] filter: the card is not archived. The typed [`LIVE`].
pub(crate) fn live() -> Expr<bool> {
    CardRow::fields().id().in_query(archived_ids()).not()
}

/// How many rows a count query found.
pub(crate) async fn count(ex: &mut Ex<'_>, query: Query<u64>) -> Result<i64, String> {
    let n = query.exec(ex).await.map_err(err)?;
    i64::try_from(n).map_err(|_| "Count out of range".to_string())
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn clean_text(value: &str, label: &str, max: usize) -> Result<String, String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(format!("{label} is required"));
    }
    if trimmed.chars().count() > max {
        return Err(format!("{label} is too long"));
    }
    Ok(trimmed.to_string())
}

pub(crate) fn check_body(body: &str) -> Result<(), String> {
    if body.len() > MAX_BODY {
        return Err("Card body is too long".into());
    }
    Ok(())
}

fn check_role(role: Option<&str>) -> Result<(), String> {
    match role {
        Some(role) if !ROLES.contains(&role) => Err(format!("Unknown column role: {role}")),
        _ => Ok(()),
    }
}

impl From<CardRow> for Card {
    fn from(row: CardRow) -> Self {
        Card {
            id: row.id,
            board_id: row.board_id,
            column_id: row.column_id,
            number: row.number,
            title: row.title,
            body: row.body,
            position: row.position,
            link_kind: row.link_kind,
            link_id: row.link_id,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

impl From<ColumnRow> for Column {
    fn from(row: ColumnRow) -> Self {
        Column {
            id: row.id,
            board_id: row.board_id,
            name: row.name,
            position: row.position,
            role: row.role,
            color: row.color,
            wip_limit: row.wip_limit,
            collapsed: row.collapsed,
        }
    }
}

impl From<BoardRow> for Board {
    fn from(row: BoardRow) -> Self {
        Board {
            id: row.id,
            name: row.name,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}

/// The card, inside a job.
pub(crate) async fn get_card_in(ex: &mut Ex<'_>, card_id: &str) -> Result<Card, String> {
    CardRow::filter_by_id(card_id)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .map(Card::from)
        .ok_or_else(|| "Card not found".to_string())
}

pub fn get_card(conn: &Conn, card_id: &str) -> Result<Card, String> {
    with_db!(conn, |ex| get_card_in(ex, card_id).await)
}

pub(crate) async fn get_column(ex: &mut Ex<'_>, column_id: &str) -> Result<Column, String> {
    ColumnRow::filter_by_id(column_id)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .map(Column::from)
        .ok_or_else(|| "Column not found".to_string())
}

async fn board_exists(ex: &mut Ex<'_>, board_id: &str) -> Result<(), String> {
    let found = BoardRow::filter_by_id(board_id)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .is_some();
    if found {
        Ok(())
    } else {
        Err("Board not found".into())
    }
}

/// The column's live cards in order.
pub(crate) async fn column_card_ids(
    ex: &mut Ex<'_>,
    column_id: &str,
) -> Result<Vec<String>, String> {
    let card = CardRow::fields();
    CardRow::filter(card.column_id().eq(column_id).and(live()))
        .order_by((card.position().asc(), card.number().asc()))
        .select(card.id())
        .exec(ex)
        .await
        .map_err(err)
}

async fn board_column_ids(ex: &mut Ex<'_>, board_id: &str) -> Result<Vec<String>, String> {
    // Raw SQL: ties on position keep insertion order (`rowid`), which Toasty
    // 0.11 cannot order by.
    query(
        ex,
        "SELECT id FROM board_columns WHERE board_id = ?1 ORDER BY position, rowid",
        args![board_id],
    )
    .await?
    .iter()
    .map(|row| row.get(0))
    .collect()
}

/// Rewrite positions to 0..n-1 in the given order.
pub(crate) async fn write_card_positions(ex: &mut Ex<'_>, ids: &[String]) -> Result<(), String> {
    for (index, id) in ids.iter().enumerate() {
        let card = CardRow::fields();
        CardRow::filter(card.id().eq(id).and(card.position().ne(index as i64)))
            .update()
            .position(index as i64)
            .exec(ex)
            .await
            .map_err(err)?;
    }
    Ok(())
}

async fn write_column_positions(ex: &mut Ex<'_>, ids: &[String]) -> Result<(), String> {
    for (index, id) in ids.iter().enumerate() {
        let column = ColumnRow::fields();
        ColumnRow::filter(column.id().eq(id).and(column.position().ne(index as i64)))
            .update()
            .position(index as i64)
            .exec(ex)
            .await
            .map_err(err)?;
    }
    Ok(())
}

pub struct NewActivity<'a> {
    pub board_id: &'a str,
    pub card_id: Option<&'a str>,
    pub kind: &'a str,
    pub actor: &'a str,
    pub from_column: Option<&'a str>,
    pub to_column: Option<&'a str>,
    pub detail: Option<&'a str>,
}

pub(crate) async fn is_archived(ex: &mut Ex<'_>, card_id: &str) -> Result<bool, String> {
    let meta = CardMetaRow::fields();
    let archived = count(
        ex,
        CardMetaRow::filter(meta.card_id().eq(card_id).and(meta.archived_at().is_some())).count(),
    )
    .await?;
    Ok(archived > 0)
}

/// Bumps the card's `updated_at`, for edits that live in other tables.
pub(crate) async fn touch(ex: &mut Ex<'_>, card_id: &str) -> Result<(), String> {
    CardRow::filter_by_id(card_id)
        .update()
        .updated_at(now_millis())
        .exec(ex)
        .await
        .map_err(err)
}

/// One `edited` row by the user saying what changed.
pub(crate) async fn log_edit(
    ex: &mut Ex<'_>,
    board_id: &str,
    card_id: &str,
    detail: &str,
) -> Result<(), String> {
    log_activity(
        ex,
        NewActivity {
            board_id,
            card_id: Some(card_id),
            kind: "edited",
            actor: "user",
            from_column: None,
            to_column: None,
            detail: Some(detail),
        },
    )
    .await
}

pub async fn log_activity(ex: &mut Ex<'_>, entry: NewActivity<'_>) -> Result<(), String> {
    ActivityRow::create()
        .id(new_id())
        .board_id(entry.board_id)
        .card_id(entry.card_id.map(str::to_string))
        .kind(entry.kind)
        .actor(entry.actor)
        .moved_from(entry.from_column.map(str::to_string))
        .to_column(entry.to_column.map(str::to_string))
        .detail(entry.detail.map(str::to_string))
        .created_at(now_millis())
        .exec(ex)
        .await
        .map_err(err)?;
    Ok(())
}

async fn ensure_role_free(
    ex: &mut Ex<'_>,
    board_id: &str,
    role: Option<&str>,
    except_column: Option<&str>,
) -> Result<(), String> {
    check_role(role)?;
    let Some(role) = role else {
        return Ok(());
    };
    let column = ColumnRow::fields();
    let taken = count(
        ex,
        ColumnRow::filter(
            column
                .board_id()
                .eq(board_id)
                .and(column.role().eq(Some(role.to_string())))
                .and(column.id().ne(except_column.unwrap_or(""))),
        )
        .count(),
    )
    .await?;
    if taken > 0 {
        return Err(format!(
            "Another column on this board already has the {role} role"
        ));
    }
    Ok(())
}

pub fn list_boards(conn: &Conn) -> Result<Vec<Board>, String> {
    with_db!(conn, |ex| {
        // Raw SQL: boards made in the same millisecond keep insertion order
        // (`rowid`), which Toasty 0.11 cannot order by.
        query(
            ex,
            "SELECT id, name, created_at, updated_at FROM boards ORDER BY created_at, rowid",
            args![],
        )
        .await?
        .iter()
        .map(|row| {
            Ok(Board {
                id: row.get(0)?,
                name: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
            })
        })
        .collect()
    })
}

/// The board's columns in order, without reading its cards.
pub(crate) async fn board_columns(ex: &mut Ex<'_>, board_id: &str) -> Result<Vec<Column>, String> {
    // Raw SQL: ties on position keep insertion order (`rowid`), which Toasty
    // 0.11 cannot order by.
    query(
        ex,
        "SELECT id, board_id, name, position, role, color, wip_limit, collapsed
         FROM board_columns WHERE board_id = ?1 ORDER BY position, rowid",
        args![board_id],
    )
    .await?
    .iter()
    .map(|row| {
        Ok(Column {
            id: row.get(0)?,
            board_id: row.get(1)?,
            name: row.get(2)?,
            position: row.get(3)?,
            role: row.get(4)?,
            color: row.get(5)?,
            wip_limit: row.get(6)?,
            collapsed: row.get(7)?,
        })
    })
    .collect()
}

pub(crate) async fn get_board_in(ex: &mut Ex<'_>, board_id: &str) -> Result<BoardDetail, String> {
    let board = BoardRow::filter_by_id(board_id)
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .map(Board::from)
        .ok_or_else(|| "Board not found".to_string())?;
    let columns = board_columns(ex, board_id).await?;
    let card = CardRow::fields();
    let cards = CardRow::filter(card.board_id().eq(board_id).and(live()))
        .order_by((card.position().asc(), card.number().asc()))
        .exec(ex)
        .await
        .map_err(err)?
        .into_iter()
        .map(Card::from)
        .collect();
    Ok(BoardDetail {
        board,
        columns,
        cards,
    })
}

pub fn get_board(conn: &Conn, board_id: &str) -> Result<BoardDetail, String> {
    with_db!(conn, |ex| get_board_in(ex, board_id).await)
}

/// A board with the four default columns (the "blank" template).
pub fn create_board(conn: &Conn, name: &str) -> Result<Board, String> {
    let name = clean_text(name, "Board name", MAX_NAME)?;
    with_tx!(conn, |ex| crate::templates::create_blank(ex, &name).await)
}

pub fn rename_board(conn: &Conn, board_id: &str, name: &str) -> Result<(), String> {
    let name = clean_text(name, "Board name", MAX_NAME)?;
    with_tx!(conn, |ex| {
        board_exists(ex, board_id).await?;
        BoardRow::filter_by_id(board_id)
            .update()
            .name(name)
            .updated_at(now_millis())
            .exec(ex)
            .await
            .map_err(err)
    })
}

/// Deletes the board; everything on it goes with it (`boards_cascade`).
pub fn delete_board(conn: &Conn, board_id: &str) -> Result<(), String> {
    with_db!(conn, |ex| {
        BoardRow::filter_by_id(board_id)
            .delete()
            .exec(ex)
            .await
            .map_err(err)
    })
}

pub fn create_column(
    conn: &Conn,
    board_id: &str,
    name: &str,
    role: Option<&str>,
) -> Result<Column, String> {
    let name = clean_text(name, "Column name", MAX_NAME)?;
    with_tx!(conn, |ex| {
        board_exists(ex, board_id).await?;
        ensure_role_free(ex, board_id, role, None).await?;
        let position = count(
            ex,
            ColumnRow::filter(ColumnRow::fields().board_id().eq(board_id)).count(),
        )
        .await?;
        ColumnRow::create()
            .id(new_id())
            .board_id(board_id)
            .name(name)
            .position(position)
            .role(role.map(str::to_string))
            .color(None::<String>)
            .wip_limit(None::<i64>)
            .collapsed(false)
            .exec(ex)
            .await
            .map(Column::from)
            .map_err(err)
    })
}

/// Returns the owning board id.
pub fn update_column(
    conn: &Conn,
    column_id: &str,
    name: &str,
    role: Option<&str>,
) -> Result<String, String> {
    let name = clean_text(name, "Column name", MAX_NAME)?;
    with_tx!(conn, |ex| {
        let column = get_column(ex, column_id).await?;
        ensure_role_free(ex, &column.board_id, role, Some(column_id)).await?;
        ColumnRow::filter_by_id(column_id)
            .update()
            .name(name)
            .role(role.map(str::to_string))
            .exec(ex)
            .await
            .map_err(err)?;
        Ok(column.board_id)
    })
}

/// Returns the owning board id.
pub fn move_column(conn: &Conn, column_id: &str, index: usize) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let column = get_column(ex, column_id).await?;
        let mut ids = board_column_ids(ex, &column.board_id).await?;
        ids.retain(|id| id != column_id);
        ids.insert(index.min(ids.len()), column_id.to_string());
        write_column_positions(ex, &ids).await?;
        Ok(column.board_id)
    })
}

/// Returns the owning board id.
pub fn delete_column(conn: &Conn, column_id: &str) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let column = get_column(ex, column_id).await?;
        let cards = count(
            ex,
            CardRow::filter(CardRow::fields().column_id().eq(column_id)).count(),
        )
        .await?;
        if cards > 0 {
            let plural = if cards == 1 { "card" } else { "cards" };
            return Err(format!("Column has {cards} {plural}"));
        }
        ColumnRow::filter_by_id(column_id)
            .delete()
            .exec(ex)
            .await
            .map_err(err)?;
        let ids = board_column_ids(ex, &column.board_id).await?;
        write_column_positions(ex, &ids).await?;
        Ok(column.board_id)
    })
}

pub fn create_card(conn: &Conn, column_id: &str, title: &str, body: &str) -> Result<Card, String> {
    with_tx!(conn, |ex| create_card_in(ex, column_id, title, body).await)
}

/// [`create_card`] inside the caller's write transaction, which it leaves
/// uncommitted.
pub(crate) async fn create_card_in(
    ex: &mut Ex<'_>,
    column_id: &str,
    title: &str,
    body: &str,
) -> Result<Card, String> {
    let title = clean_text(title, "Card title", MAX_TITLE)?;
    check_body(body)?;
    let column = get_column(ex, column_id).await?;
    let number = next_card_number(ex, &column.board_id).await?;
    let position = live_card_count(ex, column_id).await?;
    let now = now_millis();
    let card: Card = CardRow::create()
        .id(new_id())
        .board_id(&column.board_id)
        .column_id(&column.id)
        .number(number)
        .title(title)
        .body(body)
        .position(position)
        .link_kind(None::<String>)
        .link_id(None::<String>)
        .created_at(now)
        .updated_at(now)
        .title_from_ticket(false)
        .body_from_ticket(false)
        .exec(ex)
        .await
        .map_err(err)?
        .into();
    log_activity(
        ex,
        NewActivity {
            board_id: &card.board_id,
            card_id: Some(&card.id),
            kind: "created",
            actor: "user",
            from_column: None,
            to_column: Some(&column.name),
            detail: None,
        },
    )
    .await?;
    Ok(card)
}

/// Takes the board's next card number.
pub(crate) async fn next_card_number(ex: &mut Ex<'_>, board_id: &str) -> Result<i64, String> {
    BoardRow::filter_by_id(board_id)
        .update()
        .card_seq(toasty::stmt::increment())
        .exec(ex)
        .await
        .map_err(err)?;
    BoardRow::filter_by_id(board_id)
        .select(BoardRow::fields().card_seq())
        .first()
        .exec(ex)
        .await
        .map_err(err)?
        .ok_or_else(|| "Board not found".to_string())
}

/// How many live cards the column has: where a card added at its end goes.
pub(crate) async fn live_card_count(ex: &mut Ex<'_>, column_id: &str) -> Result<i64, String> {
    count(
        ex,
        CardRow::filter(CardRow::fields().column_id().eq(column_id).and(live())).count(),
    )
    .await
}

/// Returns the owning board id.
pub fn update_card(conn: &Conn, card_id: &str, title: &str, body: &str) -> Result<String, String> {
    let title = clean_text(title, "Card title", MAX_TITLE)?;
    check_body(body)?;
    with_tx!(conn, |ex| {
        let Some(card) = CardRow::filter_by_id(card_id)
            .first()
            .exec(ex)
            .await
            .map_err(err)?
        else {
            return Err("Card not found".into());
        };
        if card.title == title && card.body == body {
            return Ok(card.board_id);
        }
        // A title the user edits is theirs, so it no longer counts as ticket
        // text; a body-only edit leaves the mark as it was. A description stays
        // marked as ticket text until it is emptied, since an edit can keep it.
        // Emptied means blank after trimming spaces, as SQL's `trim` did.
        let title_from_ticket = card.title_from_ticket && card.title == title;
        let body_from_ticket = card.body_from_ticket && !body.trim_matches(' ').is_empty();
        CardRow::filter_by_id(card_id)
            .update()
            .title(title)
            .body(body)
            .updated_at(now_millis())
            .title_from_ticket(title_from_ticket)
            .body_from_ticket(body_from_ticket)
            .exec(ex)
            .await
            .map_err(err)?;
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
        Ok(card.board_id)
    })
}

/// Returns the owning board id.
pub fn move_card(
    conn: &Conn,
    card_id: &str,
    column_id: &str,
    index: usize,
    actor: &str,
) -> Result<String, String> {
    if !ACTORS.contains(&actor) {
        return Err(format!("Unknown actor: {actor}"));
    }
    with_tx!(conn, |ex| move_card_in(
        ex, card_id, column_id, index, actor
    )
    .await)
}

/// [`move_card`] inside the caller's write transaction, which it leaves
/// uncommitted. `actor` is not checked here.
pub(crate) async fn move_card_in(
    ex: &mut Ex<'_>,
    card_id: &str,
    column_id: &str,
    index: usize,
    actor: &str,
) -> Result<String, String> {
    let card = get_card_in(ex, card_id).await?;
    if is_archived(ex, card_id).await? {
        return Err("Card is archived".into());
    }
    let target = get_column(ex, column_id).await?;
    if target.board_id != card.board_id {
        return Err("Column not on this board".into());
    }
    let same_column = card.column_id == column_id;
    let mut target_ids = column_card_ids(ex, column_id).await?;
    let old_index = target_ids.iter().position(|id| id == card_id);
    target_ids.retain(|id| id != card_id);
    let new_index = index.min(target_ids.len());
    if same_column && old_index == Some(new_index) {
        return Ok(card.board_id);
    }
    target_ids.insert(new_index, card_id.to_string());

    let source = get_column(ex, &card.column_id).await?;
    if !same_column {
        CardRow::filter_by_id(card_id)
            .update()
            .column_id(column_id)
            .exec(ex)
            .await
            .map_err(err)?;
        let mut source_ids = column_card_ids(ex, &card.column_id).await?;
        source_ids.retain(|id| id != card_id);
        write_card_positions(ex, &source_ids).await?;
    }
    write_card_positions(ex, &target_ids).await?;
    touch(ex, card_id).await?;
    log_activity(
        ex,
        NewActivity {
            board_id: &card.board_id,
            card_id: Some(card_id),
            kind: "moved",
            actor,
            from_column: Some(&source.name),
            to_column: Some(&target.name),
            detail: None,
        },
    )
    .await?;
    Ok(card.board_id)
}

/// Passing `None` for both unlinks. Returns the owning board id.
pub fn link_card(
    conn: &Conn,
    card_id: &str,
    link_kind: Option<&str>,
    link_id: Option<&str>,
) -> Result<String, String> {
    let link = match (link_kind, link_id) {
        (Some(kind), Some(id)) => {
            if !LINK_KINDS.contains(&kind) {
                return Err(format!("Unknown link kind: {kind}"));
            }
            if crate::links::is_ticket_kind(kind) {
                return Err(TICKET_KIND_ERROR.into());
            }
            if id.is_empty() {
                return Err("Link id is required".into());
            }
            Some((kind, id))
        }
        (None, None) => None,
        _ => return Err("A link needs both a kind and an id".into()),
    };
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        let previous = card.link_kind.as_deref().zip(card.link_id.as_deref());
        if previous == link {
            return Ok(card.board_id);
        }
        let (kind, detail) = match (link, previous) {
            (Some((kind, id)), _) => ("linked", format!("{kind}:{id}")),
            (None, Some((kind, id))) => ("unlinked", format!("{kind}:{id}")),
            // Unlinking an unlinked card equals `previous == link`, handled above.
            (None, None) => return Ok(card.board_id),
        };
        let now = now_millis();
        CardRow::filter_by_id(card_id)
            .update()
            .link_kind(link.map(|l| l.0.to_string()))
            .link_id(link.map(|l| l.1.to_string()))
            .updated_at(now)
            .exec(ex)
            .await
            .map_err(err)?;
        let links = LinkRow::fields();
        LinkRow::filter(
            links
                .card_id()
                .eq(card_id)
                .and(links.role().eq(Some(PRIMARY.to_string()))),
        )
        .delete()
        .exec(ex)
        .await
        .map_err(err)?;
        // A link the card already has as an extra link stays an extra link: if
        // it were promoted, unlinking would delete the row the user added.
        if let Some((kind, id)) = link {
            LinkRow::upsert_by_card_id_and_kind_and_reference(card_id, kind, id)
                .role(Some(PRIMARY.to_string()))
                .created_at(now)
                .or_ignore()
                .exec(ex)
                .await
                .map_err(err)?;
        }
        log_activity(
            ex,
            NewActivity {
                board_id: &card.board_id,
                card_id: Some(card_id),
                kind,
                actor: "user",
                from_column: None,
                to_column: None,
                detail: Some(&detail),
            },
        )
        .await?;
        Ok(card.board_id)
    })
}

/// Returns the owning board id.
pub fn delete_card(conn: &Conn, card_id: &str) -> Result<String, String> {
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        delete_card_in(ex, &card).await?;
        Ok(card.board_id)
    })
}

/// Deletes the card only while it is still archived, all in one write
/// transaction, so a card another app restored meanwhile is never deleted.
/// Returns the owning board id when it deleted the card, `None` when the card
/// is live again or already gone.
pub fn delete_archived_card(conn: &Conn, card_id: &str) -> Result<Option<String>, String> {
    with_tx!(conn, |ex| {
        let card = match get_card_in(ex, card_id).await {
            Ok(card) => card,
            Err(error) if error == "Card not found" => return Ok(None),
            Err(error) => return Err(error),
        };
        if !is_archived(ex, card_id).await? {
            return Ok(None);
        }
        delete_card_in(ex, &card).await?;
        Ok(Some(card.board_id))
    })
}

/// Logs the tombstone, deletes the card and closes its column's gap.
async fn delete_card_in(ex: &mut Ex<'_>, card: &Card) -> Result<(), String> {
    let column = get_column(ex, &card.column_id).await?;
    // The tombstone has no card_id: the card's own rows cascade away with it.
    let detail = format!("#{} {}", card.number, card.title);
    log_activity(
        ex,
        NewActivity {
            board_id: &card.board_id,
            card_id: None,
            kind: "deleted",
            actor: "user",
            from_column: Some(&column.name),
            to_column: None,
            detail: Some(&detail),
        },
    )
    .await?;
    CardRow::filter_by_id(&card.id)
        .delete()
        .exec(ex)
        .await
        .map_err(err)?;
    let ids = column_card_ids(ex, &card.column_id).await?;
    write_card_positions(ex, &ids).await
}

pub(crate) async fn cards_for_link_in(
    ex: &mut Ex<'_>,
    link_kind: &str,
    link_id: &str,
) -> Result<Vec<Card>, String> {
    // The link columns are read too, for links an older build wrote after the
    // v3 migration copied them.
    let links = LinkRow::fields();
    let linked = LinkRow::filter(
        links
            .kind()
            .eq(link_kind)
            .and(links.reference().eq(link_id)),
    )
    .select(links.card_id());
    let card = CardRow::fields();
    let on_card = card
        .link_kind()
        .eq(Some(link_kind.to_string()))
        .and(card.link_id().eq(Some(link_id.to_string())));
    Ok(
        CardRow::filter(card.id().in_query(linked).or(on_card).and(live()))
            .order_by((card.board_id().asc(), card.number().asc()))
            .exec(ex)
            .await
            .map_err(err)?
            .into_iter()
            .map(Card::from)
            .collect(),
    )
}

pub fn cards_for_link(conn: &Conn, link_kind: &str, link_id: &str) -> Result<Vec<Card>, String> {
    with_db!(conn, |ex| cards_for_link_in(ex, link_kind, link_id).await)
}

pub fn list_activity(
    conn: &Conn,
    board_id: &str,
    card_id: Option<&str>,
    limit: usize,
) -> Result<Vec<Activity>, String> {
    let limit = limit.clamp(1, MAX_ACTIVITY) as i64;
    with_db!(conn, |ex| {
        // Raw SQL: rows written in the same millisecond keep insertion order
        // (`rowid`), which Toasty 0.11 cannot order by.
        query(
            ex,
            "SELECT id, board_id, card_id, kind, actor, from_column, to_column, detail, created_at
             FROM board_activity
             WHERE board_id = ?1 AND (?2 IS NULL OR card_id = ?2)
             ORDER BY created_at DESC, rowid DESC
             LIMIT ?3",
            args![board_id, card_id, limit],
        )
        .await?
        .iter()
        .map(|row| {
            Ok(Activity {
                id: row.get(0)?,
                board_id: row.get(1)?,
                card_id: row.get(2)?,
                kind: row.get(3)?,
                actor: row.get(4)?,
                from_column: row.get(5)?,
                to_column: row.get(6)?,
                detail: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .collect()
    })
}

/// Appends a `system` row without touching the card. Returns the owning board id.
pub fn record_card(
    conn: &Conn,
    card_id: &str,
    kind: &str,
    detail: Option<&str>,
) -> Result<String, String> {
    if !RECORD_KINDS.contains(&kind) {
        return Err(format!("Cannot record activity kind: {kind}"));
    }
    with_tx!(conn, |ex| {
        let card = get_card_in(ex, card_id).await?;
        log_activity(
            ex,
            NewActivity {
                board_id: &card.board_id,
                card_id: Some(card_id),
                kind,
                actor: "system",
                from_column: None,
                to_column: None,
                detail,
            },
        )
        .await?;
        Ok(card.board_id)
    })
}

/// Moves several cards into one column at `index`, keeping the order given.
/// Writes one `moved` row per card whose column or position changed. Returns
/// the owning board id.
pub fn move_cards(
    conn: &Conn,
    card_ids: &[String],
    column_id: &str,
    index: usize,
    actor: &str,
) -> Result<String, String> {
    if !ACTORS.contains(&actor) {
        return Err(format!("Unknown actor: {actor}"));
    }
    with_tx!(conn, |ex| {
        let target = get_column(ex, column_id).await?;
        let mut moving: Vec<Card> = Vec::new();
        for card_id in card_ids {
            if moving.iter().any(|card| &card.id == card_id) {
                continue;
            }
            let card = get_card_in(ex, card_id).await?;
            if card.board_id != target.board_id {
                return Err("Column not on this board".into());
            }
            if is_archived(ex, card_id).await? {
                return Err("Card is archived".into());
            }
            moving.push(card);
        }
        if moving.is_empty() {
            return Ok(target.board_id);
        }
        let mut target_ids = column_card_ids(ex, column_id).await?;
        target_ids.retain(|id| moving.iter().all(|card| &card.id != id));
        let at = index.min(target_ids.len());
        target_ids.splice(at..at, moving.iter().map(|card| card.id.clone()));

        let mut sources: Vec<String> = Vec::new();
        for card in &moving {
            if card.column_id != column_id && !sources.contains(&card.column_id) {
                sources.push(card.column_id.clone());
            }
        }
        let ids: Vec<String> = moving.iter().map(|card| card.id.clone()).collect();
        CardRow::filter(CardRow::fields().id().in_list(ids))
            .update()
            .column_id(column_id)
            .exec(ex)
            .await
            .map_err(err)?;
        for source in &sources {
            let ids = column_card_ids(ex, source).await?;
            write_card_positions(ex, &ids).await?;
        }
        write_card_positions(ex, &target_ids).await?;

        let now = now_millis();
        for card in &moving {
            let position = target_ids.iter().position(|id| id == &card.id).unwrap_or(0) as i64;
            if card.column_id == column_id && card.position == position {
                continue;
            }
            let source = get_column(ex, &card.column_id).await?;
            CardRow::filter_by_id(&card.id)
                .update()
                .updated_at(now)
                .exec(ex)
                .await
                .map_err(err)?;
            log_activity(
                ex,
                NewActivity {
                    board_id: &card.board_id,
                    card_id: Some(&card.id),
                    kind: "moved",
                    actor,
                    from_column: Some(&source.name),
                    to_column: Some(&target.name),
                    detail: None,
                },
            )
            .await?;
        }
        Ok(target.board_id)
    })
}

/// Sets how a column looks: its colour (six hex digits), WIP limit and
/// collapsed state, all three together. Returns the owning board id. Kept
/// for templates; an edit from the UI uses [`update_column_display`].
pub fn set_column_display(
    conn: &Conn,
    column_id: &str,
    color: Option<&str>,
    wip_limit: Option<i64>,
    collapsed: bool,
) -> Result<String, String> {
    if let Some(color) = color {
        check_color(color)?;
    }
    if wip_limit.is_some_and(|limit| limit < 1) {
        return Err("A WIP limit must be at least 1".into());
    }
    with_tx!(conn, |ex| {
        let column = get_column(ex, column_id).await?;
        ColumnRow::filter_by_id(column_id)
            .update()
            .color(color.map(str::to_string))
            .wip_limit(wip_limit)
            .collapsed(collapsed)
            .exec(ex)
            .await
            .map_err(err)?;
        Ok(column.board_id)
    })
}

/// The column display fields to change. `None` leaves a field as it is;
/// for colour and WIP limit `Some(None)` clears it. From the front end it
/// reads camelCase, where an absent key is `None` and an explicit `null` is
/// `Some(None)` (a `null` for `collapsed` leaves it alone). Unknown keys are
/// refused.
#[derive(Clone, Debug, Default, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ColumnDisplayPatch {
    #[serde(default, deserialize_with = "present")]
    pub color: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    pub wip_limit: Option<Option<i64>>,
    #[serde(default)]
    pub collapsed: Option<bool>,
}

/// A key that is present, `null` included, is `Some`; `default` makes an
/// absent key `None`.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    <Option<T> as serde::Deserialize>::deserialize(deserializer).map(Some)
}

/// Changes only the display fields `patch` names, so collapsing a column in
/// one app never resets a colour or WIP limit the other app just set. The
/// checks are those of [`set_column_display`]. Returns the column after.
pub fn update_column_display(
    conn: &Conn,
    column_id: &str,
    patch: &ColumnDisplayPatch,
) -> Result<Column, String> {
    if let Some(Some(color)) = &patch.color {
        check_color(color)?;
    }
    if let Some(Some(limit)) = patch.wip_limit {
        if limit < 1 {
            return Err("A WIP limit must be at least 1".into());
        }
    }
    with_tx!(conn, |ex| {
        get_column(ex, column_id).await?;
        if let Some(color) = &patch.color {
            ColumnRow::filter_by_id(column_id)
                .update()
                .color(color.clone())
                .exec(ex)
                .await
                .map_err(err)?;
        }
        if let Some(limit) = patch.wip_limit {
            ColumnRow::filter_by_id(column_id)
                .update()
                .wip_limit(limit)
                .exec(ex)
                .await
                .map_err(err)?;
        }
        if let Some(collapsed) = patch.collapsed {
            ColumnRow::filter_by_id(column_id)
                .update()
                .collapsed(collapsed)
                .exec(ex)
                .await
                .map_err(err)?;
        }
        get_column(ex, column_id).await
    })
}

pub(crate) fn check_color(color: &str) -> Result<(), String> {
    if color.len() != 6 || !color.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("Colour must be six hex digits".into());
    }
    Ok(())
}

/// The local calendar day of a timestamp, as the user picked it, for the log.
pub(crate) fn day_label(at: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(at)
        .single()
        .map(|time| time.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| at.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{fresh, Fixture};

    fn setup() -> Fixture {
        fresh()
    }

    fn col_positions(conn: &Fixture, column_id: &str) -> Vec<(String, i64)> {
        let mut statement = conn
            .raw()
            .prepare("SELECT id, position FROM board_cards WHERE column_id = ?1 ORDER BY position")
            .unwrap();
        let rows = statement
            .query_map([column_id], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap();
        rows.map(|row| row.unwrap()).collect()
    }

    fn assert_contiguous(conn: &Fixture, column_id: &str) {
        let positions: Vec<i64> = col_positions(conn, column_id)
            .into_iter()
            .map(|(_, position)| position)
            .collect();
        let expected: Vec<i64> = (0..positions.len() as i64).collect();
        assert_eq!(positions, expected, "column {column_id} not contiguous");
    }

    fn titles(conn: &Fixture, column_id: &str) -> Vec<String> {
        col_positions(conn, column_id)
            .into_iter()
            .map(|(id, _)| get_card(conn, &id).unwrap().title)
            .collect()
    }

    /// A board plus its default columns (todo, doing, review, done).
    fn board_with_columns(conn: &Fixture) -> (Board, Vec<Column>) {
        let board = create_board(conn, "Work").unwrap();
        let columns = get_board(conn, &board.id).unwrap().columns;
        (board, columns)
    }

    fn add_cards(conn: &Fixture, column_id: &str, titles: &[&str]) -> Vec<Card> {
        titles
            .iter()
            .map(|title| create_card(conn, column_id, title, "").unwrap())
            .collect()
    }

    #[test]
    fn create_board_makes_four_default_columns_with_roles() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        assert_eq!(board.name, "Work");
        let summary: Vec<(&str, i64, Option<&str>)> = columns
            .iter()
            .map(|column| {
                (
                    column.name.as_str(),
                    column.position,
                    column.role.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("Backlog", 0, Some("todo")),
                ("In Progress", 1, Some("doing")),
                ("Review", 2, Some("review")),
                ("Done", 3, Some("done")),
            ]
        );
        assert_eq!(list_boards(&conn).unwrap().len(), 1);
    }

    #[test]
    fn board_serializes_without_card_seq() {
        let conn = setup();
        let board = create_board(&conn, "Work").unwrap();
        let value = serde_json::to_value(&board).unwrap();
        let mut keys: Vec<&str> = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, vec!["createdAt", "id", "name", "updatedAt"]);
    }

    #[test]
    fn card_create_appends_at_end_and_logs_created() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        assert_eq!(
            cards.iter().map(|card| card.position).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "b", "c"]);

        let log = list_activity(&conn, &board.id, Some(&cards[2].id), 50).unwrap();
        assert_eq!(log.len(), 1);
        assert_eq!(log[0].kind, "created");
        assert_eq!(log[0].actor, "user");
        assert_eq!(log[0].card_id.as_deref(), Some(cards[2].id.as_str()));
        assert_eq!(log[0].to_column.as_deref(), Some("Backlog"));
    }

    #[test]
    fn card_numbers_increase_per_board_and_are_never_reused() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        assert_eq!(
            cards.iter().map(|card| card.number).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        delete_card(&conn, &cards[2].id).unwrap();
        let next = create_card(&conn, &columns[0].id, "d", "").unwrap();
        assert_eq!(next.number, 4);

        let (_, other_columns) = board_with_columns(&conn);
        let first = create_card(&conn, &other_columns[0].id, "x", "").unwrap();
        assert_eq!(first.number, 1);
    }

    #[test]
    fn move_within_column_renumbers_contiguously() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c", "d"]);
        move_card(&conn, &cards[0].id, &columns[0].id, 2, "user").unwrap();
        assert_eq!(titles(&conn, &columns[0].id), vec!["b", "c", "a", "d"]);
        assert_contiguous(&conn, &columns[0].id);
    }

    #[test]
    fn move_across_columns_renumbers_both_and_logs_from_to() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        add_cards(&conn, &columns[1].id, &["x", "y"]);
        move_card(&conn, &cards[1].id, &columns[1].id, 1, "system").unwrap();

        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "c"]);
        assert_eq!(titles(&conn, &columns[1].id), vec!["x", "b", "y"]);
        assert_contiguous(&conn, &columns[0].id);
        assert_contiguous(&conn, &columns[1].id);
        assert_eq!(
            get_card(&conn, &cards[1].id).unwrap().column_id,
            columns[1].id
        );

        let log = list_activity(&conn, &board.id, Some(&cards[1].id), 50).unwrap();
        assert_eq!(log[0].kind, "moved");
        assert_eq!(log[0].actor, "system");
        assert_eq!(log[0].from_column.as_deref(), Some("Backlog"));
        assert_eq!(log[0].to_column.as_deref(), Some("In Progress"));
    }

    #[test]
    fn move_index_past_end_clamps_to_end() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b"]);
        add_cards(&conn, &columns[1].id, &["x"]);
        move_card(&conn, &cards[0].id, &columns[1].id, 99, "user").unwrap();
        assert_eq!(titles(&conn, &columns[1].id), vec!["x", "a"]);
        move_card(&conn, &cards[1].id, &columns[1].id, 1_000, "user").unwrap();
        assert_eq!(titles(&conn, &columns[1].id), vec!["x", "a", "b"]);
        assert_contiguous(&conn, &columns[1].id);
        assert_contiguous(&conn, &columns[0].id);
    }

    #[test]
    fn move_to_same_position_writes_no_activity() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b", "c"]);
        let before = list_activity(&conn, &board.id, None, 500).unwrap().len();
        move_card(&conn, &cards[1].id, &columns[0].id, 1, "user").unwrap();
        // Dropping onto the end of its own column when it is already last.
        move_card(&conn, &cards[2].id, &columns[0].id, 50, "user").unwrap();
        assert_eq!(
            list_activity(&conn, &board.id, None, 500).unwrap().len(),
            before
        );
        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "b", "c"]);
    }

    #[test]
    fn move_to_column_of_other_board_rejected() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let (_, other_columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a"]);
        let error = move_card(&conn, &cards[0].id, &other_columns[0].id, 0, "user").unwrap_err();
        assert_eq!(error, "Column not on this board");
        assert_eq!(titles(&conn, &columns[0].id), vec!["a"]);
        assert!(list_activity(&conn, &board.id, Some(&cards[0].id), 50)
            .unwrap()
            .iter()
            .all(|row| row.kind != "moved"));
    }

    #[test]
    fn move_rejects_unknown_actor() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a"]);
        assert!(move_card(&conn, &cards[0].id, &columns[1].id, 0, "robot").is_err());
    }

    #[test]
    fn interleaved_moves_keep_positions_contiguous() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let mut cards = add_cards(&conn, &columns[0].id, &["a", "b", "c", "d", "e", "f"]);
        cards.extend(add_cards(&conn, &columns[2].id, &["g", "h"]));
        // Deterministic xorshift so a failure is reproducible.
        let mut state: u64 = 0x2545_F491_4F6C_DD1D;
        let mut next = move |bound: usize| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state % bound as u64) as usize
        };
        for _ in 0..20 {
            let card = &cards[next(cards.len())];
            let column = &columns[next(columns.len())];
            move_card(&conn, &card.id, &column.id, next(6), "user").unwrap();
            for column in &columns {
                assert_contiguous(&conn, &column.id);
            }
        }
        let total: usize = columns
            .iter()
            .map(|column| col_positions(&conn, &column.id).len())
            .sum();
        assert_eq!(total, cards.len());
    }

    #[test]
    fn column_move_renumbers_columns_contiguously() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        move_column(&conn, &columns[3].id, 0).unwrap();
        let detail = get_board(&conn, &board.id).unwrap();
        let order: Vec<(&str, i64)> = detail
            .columns
            .iter()
            .map(|column| (column.name.as_str(), column.position))
            .collect();
        assert_eq!(
            order,
            vec![
                ("Done", 0),
                ("Backlog", 1),
                ("In Progress", 2),
                ("Review", 3)
            ]
        );
        move_column(&conn, &columns[3].id, 99).unwrap();
        let detail = get_board(&conn, &board.id).unwrap();
        assert_eq!(detail.columns.last().unwrap().name, "Done");
        let positions: Vec<i64> = detail
            .columns
            .iter()
            .map(|column| column.position)
            .collect();
        assert_eq!(positions, vec![0, 1, 2, 3]);
    }

    #[test]
    fn column_create_appends_and_delete_closes_the_gap() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let created = create_column(&conn, &board.id, "QA", None).unwrap();
        assert_eq!((created.position, created.role.as_deref()), (4, None));
        delete_column(&conn, &columns[1].id).unwrap();
        let detail = get_board(&conn, &board.id).unwrap();
        let positions: Vec<i64> = detail
            .columns
            .iter()
            .map(|column| column.position)
            .collect();
        assert_eq!(positions, vec![0, 1, 2, 3]);
        assert_eq!(detail.columns.last().unwrap().name, "QA");
    }

    #[test]
    fn column_delete_refused_when_not_empty() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        add_cards(&conn, &columns[0].id, &["a", "b"]);
        let error = delete_column(&conn, &columns[0].id).unwrap_err();
        assert_eq!(error, "Column has 2 cards");
    }

    #[test]
    fn duplicate_role_rejected() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let error = create_column(&conn, &board.id, "Also done", Some("done")).unwrap_err();
        assert!(error.contains("role"), "{error}");
        let error = update_column(&conn, &columns[0].id, "Backlog", Some("review")).unwrap_err();
        assert!(error.contains("role"), "{error}");
        // Re-saving a column with its own role is fine, and clearing frees it.
        update_column(&conn, &columns[0].id, "Todo", Some("todo")).unwrap();
        update_column(&conn, &columns[3].id, "Done", None).unwrap();
        create_column(&conn, &board.id, "Shipped", Some("done")).unwrap();
        assert!(create_column(&conn, &board.id, "Bad", Some("nonsense")).is_err());
    }

    #[test]
    fn board_delete_cascades_columns_cards_activity() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b"]);
        move_card(&conn, &cards[0].id, &columns[1].id, 0, "user").unwrap();
        delete_card(&conn, &cards[1].id).unwrap();
        let (_, other_columns) = board_with_columns(&conn);
        add_cards(&conn, &other_columns[0].id, &["keep"]);

        delete_board(&conn, &board.id).unwrap();
        let count = |sql: &str| -> i64 {
            conn.raw()
                .query_row(sql, [board.id.as_str()], |row| row.get(0))
                .unwrap()
        };
        assert_eq!(count("SELECT COUNT(*) FROM boards WHERE id = ?1"), 0);
        assert_eq!(
            count("SELECT COUNT(*) FROM board_columns WHERE board_id = ?1"),
            0
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM board_cards WHERE board_id = ?1"),
            0
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM board_activity WHERE board_id = ?1"),
            0
        );
        assert_eq!(list_boards(&conn).unwrap().len(), 1);
        assert_eq!(col_positions(&conn, &other_columns[0].id).len(), 1);
    }

    #[test]
    fn card_delete_logs_deleted_with_number_title_and_null_card_id() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "Fix login", "c"]);
        delete_card(&conn, &cards[1].id).unwrap();

        assert_eq!(titles(&conn, &columns[0].id), vec!["a", "c"]);
        assert_contiguous(&conn, &columns[0].id);
        let log = list_activity(&conn, &board.id, None, 50).unwrap();
        assert_eq!(log[0].kind, "deleted");
        assert_eq!(log[0].card_id, None);
        assert_eq!(log[0].detail.as_deref(), Some("#2 Fix login"));
        // The deleted card's own history goes with it; only the tombstone stays.
        assert!(log
            .iter()
            .all(|row| row.card_id.as_deref() != Some(cards[1].id.as_str())));
    }

    #[test]
    fn cards_for_link_finds_linked_cards() {
        let conn = setup();
        let (_, columns) = board_with_columns(&conn);
        let (_, other_columns) = board_with_columns(&conn);
        let cards = add_cards(&conn, &columns[0].id, &["a", "b"]);
        let other = add_cards(&conn, &other_columns[0].id, &["c"]);
        link_card(&conn, &cards[0].id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &other[0].id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &cards[1].id, Some("pilot"), Some("s-1")).unwrap();

        let found = cards_for_link(&conn, "session", "s-1").unwrap();
        let mut ids: Vec<&str> = found.iter().map(|card| card.id.as_str()).collect();
        ids.sort_unstable();
        let mut expected = vec![cards[0].id.as_str(), other[0].id.as_str()];
        expected.sort_unstable();
        assert_eq!(ids, expected);
        assert!(cards_for_link(&conn, "session", "nope").unwrap().is_empty());
    }

    #[test]
    fn link_and_unlink_log_and_validate() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        assert!(link_card(&conn, &card.id, Some("session"), None).is_err());
        assert!(link_card(&conn, &card.id, Some("bogus"), Some("x")).is_err());

        link_card(&conn, &card.id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &card.id, Some("session"), Some("s-1")).unwrap();
        link_card(&conn, &card.id, None, None).unwrap();
        link_card(&conn, &card.id, None, None).unwrap();
        let kinds: Vec<String> = list_activity(&conn, &board.id, Some(&card.id), 50)
            .unwrap()
            .into_iter()
            .map(|row| row.kind)
            .collect();
        assert_eq!(kinds, vec!["unlinked", "linked", "created"]);
        let card = get_card(&conn, &card.id).unwrap();
        assert_eq!((card.link_kind, card.link_id), (None, None));
    }

    #[test]
    fn update_logs_edited_only_when_changed() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        update_card(&conn, &card.id, "a", "").unwrap();
        update_card(&conn, &card.id, "  b  ", "notes").unwrap();
        let saved = get_card(&conn, &card.id).unwrap();
        assert_eq!((saved.title.as_str(), saved.body.as_str()), ("b", "notes"));
        assert!(update_card(&conn, &card.id, "   ", "x").is_err());
        let edited = list_activity(&conn, &board.id, Some(&card.id), 50)
            .unwrap()
            .iter()
            .filter(|row| row.kind == "edited")
            .count();
        assert_eq!(edited, 1);
    }

    #[test]
    fn record_appends_system_row_without_moving() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        record_card(&conn, &card.id, "failed", Some("exit 1")).unwrap();
        let log = list_activity(&conn, &board.id, Some(&card.id), 50).unwrap();
        assert_eq!(log[0].kind, "failed");
        assert_eq!(log[0].actor, "system");
        assert_eq!(log[0].detail.as_deref(), Some("exit 1"));
        assert_eq!(get_card(&conn, &card.id).unwrap().column_id, columns[0].id);
        assert!(record_card(&conn, &card.id, "deleted", None).is_err());
    }

    #[test]
    fn activity_is_newest_first_and_limited() {
        let conn = setup();
        let (board, columns) = board_with_columns(&conn);
        let card = add_cards(&conn, &columns[0].id, &["a"]).remove(0);
        record_card(&conn, &card.id, "started", None).unwrap();
        record_card(&conn, &card.id, "completed", None).unwrap();
        let log = list_activity(&conn, &board.id, None, 2).unwrap();
        let kinds: Vec<&str> = log.iter().map(|row| row.kind.as_str()).collect();
        assert_eq!(kinds, vec!["completed", "started"]);
    }
}
