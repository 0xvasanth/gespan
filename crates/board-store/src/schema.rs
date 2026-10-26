use crate::migrate;
use rusqlite::Connection;

pub const VERSION: i64 = 5;

/// The tables as both apps created them before `board_schema` existed.
pub(crate) const V2_TABLES: &str = "
CREATE TABLE IF NOT EXISTS boards (
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
  ON board_activity (card_id, created_at DESC);
";

pub(crate) const V3_TABLES: &str = "
CREATE TABLE IF NOT EXISTS board_schema (version INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS board_card_meta (
  card_id TEXT PRIMARY KEY REFERENCES board_cards(id) ON DELETE CASCADE,
  priority INTEGER NOT NULL DEFAULT 0,
  due_at INTEGER,
  start_at INTEGER,
  estimate_minutes INTEGER,
  archived_at INTEGER,
  completed_at INTEGER
);
CREATE TABLE IF NOT EXISTS board_labels (
  id TEXT PRIMARY KEY,
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  color TEXT NOT NULL,
  position INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX IF NOT EXISTS board_labels_name_idx
  ON board_labels (board_id, lower(name));
CREATE TABLE IF NOT EXISTS board_card_labels (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  label_id TEXT NOT NULL REFERENCES board_labels(id) ON DELETE CASCADE,
  PRIMARY KEY (card_id, label_id)
);
CREATE TABLE IF NOT EXISTS board_subtasks (
  id TEXT PRIMARY KEY,
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  title TEXT NOT NULL,
  done INTEGER NOT NULL DEFAULT 0,
  position INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS board_subtasks_card_idx
  ON board_subtasks (card_id, position);
CREATE TABLE IF NOT EXISTS board_comments (
  id TEXT PRIMARY KEY,
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  body TEXT NOT NULL,
  author TEXT NOT NULL DEFAULT 'user',
  created_at INTEGER NOT NULL,
  edited_at INTEGER
);
CREATE INDEX IF NOT EXISTS board_comments_card_idx
  ON board_comments (card_id, created_at);
CREATE TABLE IF NOT EXISTS board_card_projects (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  project TEXT NOT NULL,
  PRIMARY KEY (card_id, project)
);
CREATE TABLE IF NOT EXISTS board_card_relations (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  related_card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  kind TEXT NOT NULL CHECK (kind IN ('blocks', 'relates', 'duplicates')),
  PRIMARY KEY (card_id, related_card_id, kind)
);
CREATE INDEX IF NOT EXISTS board_card_relations_related_idx
  ON board_card_relations (related_card_id);
CREATE TABLE IF NOT EXISTS board_fields (
  id TEXT PRIMARY KEY,
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  kind TEXT NOT NULL,
  options_json TEXT,
  position INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS board_card_field_values (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  field_id TEXT NOT NULL REFERENCES board_fields(id) ON DELETE CASCADE,
  value_json TEXT NOT NULL,
  PRIMARY KEY (card_id, field_id)
);
CREATE TABLE IF NOT EXISTS board_card_links (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  kind TEXT NOT NULL,
  ref TEXT NOT NULL,
  role TEXT,
  created_at INTEGER NOT NULL,
  PRIMARY KEY (card_id, kind, ref)
);
CREATE INDEX IF NOT EXISTS board_card_links_ref_idx
  ON board_card_links (kind, ref);
CREATE TABLE IF NOT EXISTS board_settings (
  board_id TEXT PRIMARY KEY REFERENCES boards(id) ON DELETE CASCADE,
  run_with INTEGER NOT NULL DEFAULT 1,
  write_back INTEGER NOT NULL DEFAULT 1,
  ticket_status_changes INTEGER NOT NULL DEFAULT 1,
  auto_move_json TEXT
);
";

/// Columns v3 adds to tables that already existed, as (table, column, declaration).
pub(crate) const V3_COLUMNS: [(&str, &str, &str); 3] = [
    ("board_columns", "color", "TEXT"),
    ("board_columns", "wip_limit", "INTEGER"),
    ("board_columns", "collapsed", "INTEGER NOT NULL DEFAULT 0"),
];

/// v4: saved views, one row per tab after the built-in "All cards".
pub(crate) const V4_TABLES: &str = "
CREATE TABLE IF NOT EXISTS board_views (
  id TEXT PRIMARY KEY,
  board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL,
  position INTEGER NOT NULL,
  spec_json TEXT NOT NULL,
  layout_json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS board_views_board_idx
  ON board_views (board_id, position);
";

/// v5: board appearance and display settings, templates, and per-session
/// ticket status overrides.
pub(crate) const V5_TABLES: &str = "
CREATE TABLE IF NOT EXISTS board_templates (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  spec_json TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS board_session_settings (
  session_key TEXT PRIMARY KEY,
  may_change_status INTEGER
);
";

/// Columns v5 adds to `board_settings`. NULL means the default.
pub(crate) const V5_COLUMNS: [(&str, &str, &str); 5] = [
    ("board_settings", "icon", "TEXT"),
    ("board_settings", "color", "TEXT"),
    ("board_settings", "default_view_id", "TEXT"),
    ("board_settings", "display_json", "TEXT"),
    ("board_settings", "priority_json", "TEXT"),
];

/// Brings the board tables to the current version. Both apps call this on
/// every open, so it is cheap once the database is current.
pub fn ensure_tables(conn: &Connection) -> rusqlite::Result<()> {
    if migrate::version(conn)? >= VERSION {
        return Ok(());
    }
    migrate::run(conn)
}
