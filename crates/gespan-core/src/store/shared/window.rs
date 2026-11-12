//! An indexed Pilot is read like the desktop app pages it: the newest turns
//! first, older turns only on request. Row indices count from a sticky window
//! start, so saves, turn receipts and edits never read or rewrite older blocks.
use super::*;
use crate::store::schema::{PilotBlock, PilotTranscript, PilotTurn};

/// Turns in the first window and in each older page, as desktop pages them.
pub const PAGE_TURNS: usize = 10;

/// Rows strictly before a window or an earlier page, oldest first.
#[derive(Debug, Default, Clone)]
pub struct HistoryPage {
    pub rows: Vec<Row>,
    /// Turn receipts keyed by row offset within `rows`.
    pub turns: HashMap<usize, crate::agent::TurnInfo>,
    /// The cursor for the page before this one, if any older blocks remain.
    pub before: Option<i64>,
}

impl Store {
    pub(crate) fn shared_indexed(&self, original: &str) -> Result<bool, String> {
        let original = original.to_owned();
        self.orm()?.run(|mut db| async move {
            PilotTranscript::filter_by_session_id(original)
                .first()
                .exec(&mut db)
                .await
                .map(|transcript| transcript.is_some())
        })
    }
    /// The first block sequence of an indexed Pilot's window, chosen once per
    /// store so that every row index handed out for it shares one origin.
    /// None for a history stored as one blob, which is always read whole.
    pub(super) fn shared_window(&self, id: i64, original: &str) -> Result<Option<i64>, String> {
        if !self.shared_indexed(original)? {
            return Ok(None);
        }
        let windows = &self.shared.windows;
        if let Some(from) = windows.borrow().get(&id) {
            return Ok(Some(*from));
        }
        let original = original.to_owned();
        let from = self
            .orm()?
            .run(|mut db| async move {
                let turn = PilotTurn::fields();
                PilotTurn::filter(turn.session_id().eq(original))
                    .order_by(turn.seq().desc())
                    .limit(1)
                    .offset(PAGE_TURNS - 1)
                    .select(turn.seq())
                    .exec(&mut db)
                    .await
            })?
            .into_iter()
            .next()
            .unwrap_or(0);
        windows.borrow_mut().insert(id, from);
        Ok(Some(from))
    }
    pub(super) fn shared_indexed_blocks(
        &self,
        original: &str,
        from: i64,
        before: i64,
    ) -> Result<Vec<Value>, String> {
        let original = original.to_owned();
        let records = self.orm()?.run(|mut db| async move {
            let block = PilotBlock::fields();
            PilotBlock::filter(
                block
                    .session_id()
                    .eq(original)
                    .and(block.seq().ge(from))
                    .and(block.seq().lt(before)),
            )
            .order_by(block.seq().asc())
            .select(block.block_json())
            .exec(&mut db)
            .await
        })?;
        records.iter().map(|raw| parse(raw)).collect()
    }
    /// Every block, numbered from the first. Only whole-history readers such as
    /// search use this; row indices it implies are not the window's.
    pub(super) fn shared_all_blocks(&self, id: i64) -> Result<Vec<Value>, String> {
        let original = self.persisted_session_id(id)?;
        if self.shared_indexed(&original)? {
            self.shared_indexed_blocks(&original, 0, i64::MAX)
        } else {
            self.shared_blocks(id)
        }
    }
    /// The cursor for the page before the window, if anything precedes it.
    pub(crate) fn shared_history_before(&self, id: i64) -> Result<Option<i64>, String> {
        let original = self.persisted_session_id(id)?;
        let Some(from) = self.shared_window(id, &original)? else {
            return Ok(None);
        };
        let older = self.orm()?.run(|mut db| async move {
            let block = PilotBlock::fields();
            PilotBlock::filter(block.session_id().eq(original).and(block.seq().lt(from)))
                .limit(1)
                .select(block.seq())
                .exec(&mut db)
                .await
        })?;
        Ok((!older.is_empty()).then_some(from))
    }
    /// The turns before `before`: one page, or with `all` everything older.
    /// Pages are read-only. Their rows precede the window and have no index in
    /// it, so nothing here records a revision or moves the window.
    pub(crate) fn shared_load_before(
        &self,
        id: i64,
        before: i64,
        all: bool,
    ) -> Result<HistoryPage, String> {
        let original = self.persisted_session_id(id)?;
        if !self.shared_indexed(&original)? {
            return Ok(HistoryPage::default());
        }
        let mut start = 0;
        let mut more = false;
        if !all {
            let session = original.clone();
            let starts = self.orm()?.run(|mut db| async move {
                let turn = PilotTurn::fields();
                PilotTurn::filter(turn.session_id().eq(session).and(turn.seq().lt(before)))
                    .order_by(turn.seq().desc())
                    .limit(PAGE_TURNS + 1)
                    .select(turn.seq())
                    .exec(&mut db)
                    .await
            })?;
            if starts.len() > PAGE_TURNS {
                start = starts[PAGE_TURNS - 1];
                more = true;
            }
        }
        let blocks = self.shared_indexed_blocks(&original, start, before)?;
        Ok(HistoryPage {
            rows: blocks::project(&blocks)?,
            turns: project_turns(&blocks)?,
            before: more.then_some(start),
        })
    }
    /// Whether any block of the whole history, not only the window, records
    /// this accepted message.
    pub(super) fn shared_delivered(&self, id: i64, message: &str) -> Result<bool, String> {
        let original = self.persisted_session_id(id)?;
        if !self.shared_indexed(&original)? {
            return Ok(self
                .shared_blocks(id)?
                .iter()
                .any(|b| b["appRequestId"].as_str() == Some(message)));
        }
        // json1: Toasty 0.11 cannot filter on a field inside a JSON column.
        self.db()?.value(
            "SELECT EXISTS(SELECT 1 FROM pilot_blocks WHERE session_id=?1 AND is_user=1
             AND json_extract(block_json,'$.appRequestId')=?2)",
            args![original, message],
        )
    }
}
