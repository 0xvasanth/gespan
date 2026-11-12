-- The board tables as both apps created them before v3, with the native
-- app's board_native_* extras, and a few rows in each.
CREATE TABLE boards (id TEXT PRIMARY KEY, name TEXT NOT NULL, card_seq INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL);
CREATE TABLE board_columns (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE, name TEXT NOT NULL, position INTEGER NOT NULL, role TEXT);
CREATE UNIQUE INDEX board_columns_role_idx ON board_columns (board_id, role) WHERE role IS NOT NULL;
CREATE TABLE board_cards (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE, column_id TEXT NOT NULL REFERENCES board_columns(id), number INTEGER NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL DEFAULT '', position INTEGER NOT NULL, link_kind TEXT, link_id TEXT, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, UNIQUE (board_id, number));
CREATE INDEX board_cards_column_idx ON board_cards (column_id, position);
CREATE INDEX board_cards_link_idx ON board_cards (link_kind, link_id);
CREATE TABLE board_activity (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE, card_id TEXT REFERENCES board_cards(id) ON DELETE CASCADE, kind TEXT NOT NULL, actor TEXT NOT NULL, from_column TEXT, to_column TEXT, detail TEXT, created_at INTEGER NOT NULL);
CREATE TABLE board_native_meta (card_id TEXT PRIMARY KEY REFERENCES board_cards(id) ON DELETE CASCADE, priority INTEGER NOT NULL DEFAULT 0, due_at INTEGER);
CREATE TABLE board_native_projects (card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE, project TEXT NOT NULL, PRIMARY KEY (card_id, project));
CREATE TABLE board_native_labels (id TEXT PRIMARY KEY, board_id TEXT NOT NULL REFERENCES boards(id) ON DELETE CASCADE, name TEXT NOT NULL, color TEXT NOT NULL);
CREATE TABLE board_native_card_labels (card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE, label_id TEXT NOT NULL REFERENCES board_native_labels(id) ON DELETE CASCADE, PRIMARY KEY (card_id, label_id));
CREATE TABLE board_native_subtasks (id TEXT PRIMARY KEY, card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE, title TEXT NOT NULL, done INTEGER NOT NULL DEFAULT 0, position INTEGER NOT NULL);
CREATE TABLE board_native_comments (id TEXT PRIMARY KEY, card_id TEXT NOT NULL REFERENCES board_cards(id) ON DELETE CASCADE, body TEXT NOT NULL, created_at INTEGER NOT NULL);
INSERT INTO boards VALUES ('b1', 'Work', 2, 10, 10);
INSERT INTO board_columns VALUES ('c-todo', 'b1', 'Backlog', 0, 'todo');
INSERT INTO board_columns VALUES ('c-doing', 'b1', 'In Progress', 1, 'doing');
INSERT INTO board_cards VALUES ('k1', 'b1', 'c-todo', 1, 'Fix login', 'body', 0, 'session', 's-1', 20, 21);
INSERT INTO board_cards VALUES ('k2', 'b1', 'c-todo', 2, 'Write docs', '', 1, NULL, NULL, 22, 23);
INSERT INTO board_activity VALUES ('a1', 'b1', 'k1', 'created', 'user', NULL, 'Backlog', NULL, 20);
INSERT INTO board_native_meta VALUES ('k1', 3, 1793404800000);
INSERT INTO board_native_projects VALUES ('k1', '/a'), ('k1', '/b');
INSERT INTO board_native_labels VALUES ('l-docs', 'b1', 'Docs', '4da3f5'), ('l-bug', 'b1', 'bug', 'e5534b');
INSERT INTO board_native_card_labels VALUES ('k1', 'l-docs'), ('k1', 'l-bug'), ('k2', 'l-docs');
INSERT INTO board_native_subtasks VALUES ('s1', 'k1', 'One', 1, 0), ('s2', 'k1', 'Two', 0, 1);
INSERT INTO board_native_comments VALUES ('m1', 'k1', 'first', 30), ('m2', 'k1', 'second', 31);
