use rusqlite::Connection;

pub fn ensure_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS boards (
           id TEXT PRIMARY KEY,
           name TEXT NOT NULL,
           card_seq INTEGER NOT NULL DEFAULT 0,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL
         );
         CREATE TABLE IF NOT EXISTS board_columns (
           id TEXT PRIMARY KEY,
           board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
           name TEXT NOT NULL,
           position INTEGER NOT NULL,
           role TEXT
         );
         CREATE UNIQUE INDEX IF NOT EXISTS board_columns_role_idx
           ON board_columns (board_id, role) WHERE role IS NOT NULL;
         CREATE TABLE IF NOT EXISTS board_cards (
           id TEXT PRIMARY KEY,
           board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
           column_id TEXT NOT NULL REFERENCES board_columns(id),
           number INTEGER NOT NULL,
           title TEXT NOT NULL,
           body TEXT NOT NULL DEFAULT '',
           position INTEGER NOT NULL,
           link_kind TEXT,
           link_id TEXT,
           created_at INTEGER NOT NULL,
           updated_at INTEGER NOT NULL,
           UNIQUE (board_id, number)
         );
         CREATE INDEX IF NOT EXISTS board_cards_column_idx
           ON board_cards (column_id, position);
         CREATE INDEX IF NOT EXISTS board_cards_link_idx
           ON board_cards (link_kind, link_id);
         CREATE TABLE IF NOT EXISTS board_activity (
           id TEXT PRIMARY KEY,
           board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
           card_id TEXT REFERENCES board_cards(id) ON DELETE CASCADE,
           kind TEXT NOT NULL,
           actor TEXT NOT NULL,
           from_column TEXT,
           to_column TEXT,
           detail TEXT,
           created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS board_activity_board_idx
           ON board_activity (board_id, created_at DESC);
         CREATE INDEX IF NOT EXISTS board_activity_card_idx
           ON board_activity (card_id, created_at DESC);",
    )
}
