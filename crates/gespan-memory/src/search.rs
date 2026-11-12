use crate::entry::{Entry, Scope};
use crate::store::{row_to_entry, visible_filter, Store, COLS};
use crate::Error;

impl Store {
    /// Keyword search over title, text and why, best match first. Sees exactly
    /// what `visible` sees (same filter), so a rejected, replaced or
    /// archived entry, or another project's entry, is never returned.
    ///
    /// The query is arbitrary text. Every word becomes a quoted phrase, so
    /// FTS5 operators and syntax in it are plain words and cannot make the
    /// search fail. Words are OR-ed, and bm25 ranks entries matching more of
    /// them (or matching them more often) higher. A query with no usable
    /// words returns nothing.
    pub fn search(&self, scope: &Scope, query: &str, limit: usize) -> Result<Vec<Entry>, Error> {
        let Some(m) = match_expr(query) else {
            return Ok(Vec::new());
        };
        let sql = format!(
            "SELECT {COLS} FROM entries_fts \
             JOIN entries e ON e.id = entries_fts.rowid \
             WHERE entries_fts MATCH ?2 AND {} \
             ORDER BY bm25(entries_fts), e.id DESC LIMIT ?3",
            visible_filter()
        );
        let conn = self.conn();
        let mut stmt = conn.prepare(&sql)?;
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let rows = stmt.query_map(rusqlite::params![scope.to_string(), m, limit], row_to_entry)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r??);
        }
        Ok(out)
    }
}

/// An FTS5 MATCH expression for free text: each whitespace-separated word as
/// a quoted phrase, OR-ed. Words with no letter or digit (`*`, `-`, `"`) have
/// no tokens to match, so they are dropped. `None` when nothing is left.
fn match_expr(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split_whitespace()
        .filter(|w| w.chars().any(char::is_alphanumeric))
        .map(|w| format!("\"{}\"", w.replace('"', "\"\"")))
        .collect();
    if words.is_empty() {
        None
    } else {
        Some(words.join(" OR "))
    }
}
