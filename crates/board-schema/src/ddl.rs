//! Board DDL. `BASE_DDL` is the original four tables, unchanged since v1 so a
//! build from before v3 can still open the file. `V3_DDL` only adds tables;
//! the new `board_columns` columns are added in `migrate`.

/// The v1 tables, exactly as both apps created them before v3.
pub const BASE_DDL: &str = "CREATE TABLE IF NOT EXISTS boards (
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
  ON board_activity (card_id, created_at DESC);";

/// The v3 tables. `IF NOT EXISTS` throughout: this runs on every open.
pub(crate) const V3_DDL: &str = "CREATE TABLE IF NOT EXISTS board_schema_migrations (version INTEGER PRIMARY KEY, applied_at INTEGER NOT NULL);
CREATE TABLE IF NOT EXISTS board_card_meta (
  card_id TEXT PRIMARY KEY REFERENCES board_cards(id) ON DELETE CASCADE,
  priority INTEGER NOT NULL DEFAULT 0,          -- index into PRIORITIES, 0 = None
  due_at INTEGER, start_at INTEGER, estimate_minutes INTEGER,
  archived_at INTEGER, completed_at INTEGER);
CREATE TABLE IF NOT EXISTS board_labels (
  id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL, color TEXT NOT NULL, position INTEGER NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS board_labels_name_idx ON board_labels (board_id, lower(name));
CREATE TABLE IF NOT EXISTS board_card_labels (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  label_id TEXT NOT NULL REFERENCES board_labels(id) ON DELETE CASCADE,
  PRIMARY KEY (card_id, label_id));
CREATE INDEX IF NOT EXISTS board_card_labels_label_idx ON board_card_labels (label_id);
CREATE TABLE IF NOT EXISTS board_subtasks (
  id TEXT PRIMARY KEY, card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0, position INTEGER NOT NULL,
  converted_card_id TEXT REFERENCES board_cards(id) ON DELETE SET NULL);  -- room for Convert to card
CREATE INDEX IF NOT EXISTS board_subtasks_card_idx ON board_subtasks (card_id, position);
CREATE TABLE IF NOT EXISTS board_comments (
  id TEXT PRIMARY KEY, card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  body TEXT NOT NULL, author TEXT NOT NULL DEFAULT 'user',  -- user | agent:<session> | external:<provider>
  created_at INTEGER NOT NULL, edited_at INTEGER);
CREATE INDEX IF NOT EXISTS board_comments_card_idx ON board_comments (card_id, created_at);
CREATE TABLE IF NOT EXISTS board_card_projects (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  project TEXT NOT NULL, PRIMARY KEY (card_id, project));
CREATE TABLE IF NOT EXISTS board_card_relations (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  related_card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  kind TEXT NOT NULL, created_at INTEGER NOT NULL,
  PRIMARY KEY (card_id, related_card_id, kind));
CREATE INDEX IF NOT EXISTS board_card_relations_related_idx ON board_card_relations (related_card_id);
CREATE TABLE IF NOT EXISTS board_fields (
  id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE,
  name TEXT NOT NULL, kind TEXT NOT NULL, options_json TEXT NOT NULL DEFAULT '{}', position INTEGER NOT NULL);
CREATE UNIQUE INDEX IF NOT EXISTS board_fields_name_idx ON board_fields (board_id, lower(name));
CREATE TABLE IF NOT EXISTS board_card_field_values (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  field_id TEXT NOT NULL REFERENCES board_fields(id) ON DELETE CASCADE,
  value_json TEXT NOT NULL, PRIMARY KEY (card_id, field_id));
CREATE INDEX IF NOT EXISTS board_card_field_values_field_idx ON board_card_field_values (field_id);
CREATE TABLE IF NOT EXISTS board_card_links (
  card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE,
  kind TEXT NOT NULL, ref TEXT NOT NULL, role TEXT NOT NULL DEFAULT 'primary', created_at INTEGER NOT NULL,
  PRIMARY KEY (card_id, kind, ref));
CREATE INDEX IF NOT EXISTS board_card_links_ref_idx ON board_card_links (kind, ref);";
