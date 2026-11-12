//! What the hand-made board tables enforced that Toasty's generated DDL
//! cannot: foreign keys with their `ON DELETE CASCADE`, the `CHECK` on
//! relation kinds, the partial unique index on column roles and the
//! expression unique index on label names.
//!
//! gespan-core runs [`CONSTRAINTS`] on every open of the `gespan` database,
//! after its migrations (`Kind::constraints`). Each statement is idempotent.
//! To change one, add it under a new name and drop the old one here.
//!
//! A reference check raises the error SQLite's own foreign keys raised
//! (`FOREIGN KEY constraint failed`), and the relation-kind check the one its
//! `CHECK` raised, so callers see the same failures as before.

/// A column referencing another table's `id`: a row may only be inserted, or
/// the column changed, while the row it names exists. `NULL` names nothing.
macro_rules! reference {
    ($child:literal, $column:literal, $parent:literal) => {
        [
            concat!(
                "CREATE TRIGGER IF NOT EXISTS ",
                $child,
                "_",
                $column,
                "_fk_insert
                 BEFORE INSERT ON ",
                $child,
                "
                 WHEN NEW.",
                $column,
                " IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM ",
                $parent,
                " WHERE id = NEW.",
                $column,
                ")
                 BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END"
            ),
            concat!(
                "CREATE TRIGGER IF NOT EXISTS ",
                $child,
                "_",
                $column,
                "_fk_update
                 BEFORE UPDATE OF ",
                $column,
                " ON ",
                $child,
                "
                 WHEN NEW.",
                $column,
                " IS NOT NULL
                   AND NOT EXISTS (SELECT 1 FROM ",
                $parent,
                " WHERE id = NEW.",
                $column,
                ")
                 BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END"
            ),
        ]
    };
}

const REFERENCES: [[&str; 2]; 20] = [
    reference!("board_columns", "board_id", "boards"),
    reference!("board_cards", "board_id", "boards"),
    reference!("board_cards", "column_id", "board_columns"),
    reference!("board_activity", "board_id", "boards"),
    reference!("board_activity", "card_id", "board_cards"),
    reference!("board_card_meta", "card_id", "board_cards"),
    reference!("board_labels", "board_id", "boards"),
    reference!("board_card_labels", "card_id", "board_cards"),
    reference!("board_card_labels", "label_id", "board_labels"),
    reference!("board_subtasks", "card_id", "board_cards"),
    reference!("board_comments", "card_id", "board_cards"),
    reference!("board_card_projects", "card_id", "board_cards"),
    reference!("board_card_relations", "card_id", "board_cards"),
    reference!("board_card_relations", "related_card_id", "board_cards"),
    reference!("board_fields", "board_id", "boards"),
    reference!("board_card_field_values", "card_id", "board_cards"),
    reference!("board_card_field_values", "field_id", "board_fields"),
    reference!("board_card_links", "card_id", "board_cards"),
    reference!("board_settings", "board_id", "boards"),
    reference!("board_views", "board_id", "boards"),
];

/// The other statements: cascades, the column the cards still use, the
/// relation-kind check and the unique indexes.
const OTHERS: [&str; 10] = [
    // Deleting a board deletes everything on it. Cards go first, so their
    // own rows cascade through `board_cards_cascade` and no column is
    // deleted while a card still sits in it.
    "CREATE TRIGGER IF NOT EXISTS boards_cascade AFTER DELETE ON boards
     BEGIN
       DELETE FROM board_cards WHERE board_id = OLD.id;
       DELETE FROM board_activity WHERE board_id = OLD.id;
       DELETE FROM board_columns WHERE board_id = OLD.id;
       DELETE FROM board_labels WHERE board_id = OLD.id;
       DELETE FROM board_fields WHERE board_id = OLD.id;
       DELETE FROM board_settings WHERE board_id = OLD.id;
       DELETE FROM board_views WHERE board_id = OLD.id;
     END",
    // Deleting a card deletes its rows, its history included; a relation
    // goes when either of its cards does.
    "CREATE TRIGGER IF NOT EXISTS board_cards_cascade AFTER DELETE ON board_cards
     BEGIN
       DELETE FROM board_card_meta WHERE card_id = OLD.id;
       DELETE FROM board_card_labels WHERE card_id = OLD.id;
       DELETE FROM board_subtasks WHERE card_id = OLD.id;
       DELETE FROM board_comments WHERE card_id = OLD.id;
       DELETE FROM board_card_projects WHERE card_id = OLD.id;
       DELETE FROM board_card_relations
         WHERE card_id = OLD.id OR related_card_id = OLD.id;
       DELETE FROM board_card_field_values WHERE card_id = OLD.id;
       DELETE FROM board_card_links WHERE card_id = OLD.id;
       DELETE FROM board_activity WHERE card_id = OLD.id;
     END",
    "CREATE TRIGGER IF NOT EXISTS board_labels_cascade AFTER DELETE ON board_labels
     BEGIN
       DELETE FROM board_card_labels WHERE label_id = OLD.id;
     END",
    "CREATE TRIGGER IF NOT EXISTS board_fields_cascade AFTER DELETE ON board_fields
     BEGIN
       DELETE FROM board_card_field_values WHERE field_id = OLD.id;
     END",
    // `board_cards.column_id` had no cascade: a column that still holds a
    // card cannot be deleted.
    "CREATE TRIGGER IF NOT EXISTS board_columns_in_use BEFORE DELETE ON board_columns
     WHEN EXISTS (SELECT 1 FROM board_cards WHERE column_id = OLD.id)
     BEGIN SELECT RAISE(ABORT, 'FOREIGN KEY constraint failed'); END",
    // CHECK (kind IN ('blocks', 'relates', 'duplicates')).
    "CREATE TRIGGER IF NOT EXISTS board_card_relations_kind_insert
     BEFORE INSERT ON board_card_relations
     WHEN NEW.kind NOT IN ('blocks', 'relates', 'duplicates')
     BEGIN SELECT RAISE(ABORT, 'CHECK constraint failed: kind'); END",
    "CREATE TRIGGER IF NOT EXISTS board_card_relations_kind_update
     BEFORE UPDATE OF kind ON board_card_relations
     WHEN NEW.kind NOT IN ('blocks', 'relates', 'duplicates')
     BEGIN SELECT RAISE(ABORT, 'CHECK constraint failed: kind'); END",
    // One column per role on a board; columns without a role don't count.
    "CREATE UNIQUE INDEX IF NOT EXISTS board_columns_role_idx
     ON board_columns (board_id, role) WHERE role IS NOT NULL",
    // A card's number is unique on its board (`UNIQUE (board_id, number)`).
    "CREATE UNIQUE INDEX IF NOT EXISTS board_cards_number_idx
     ON board_cards (board_id, number)",
    // Label names are unique on a board, ignoring case.
    "CREATE UNIQUE INDEX IF NOT EXISTS board_labels_name_idx
     ON board_labels (board_id, lower(name))",
];

const fn all() -> [&'static str; 50] {
    let mut out = [""; 50];
    let mut i = 0;
    while i < REFERENCES.len() {
        out[2 * i] = REFERENCES[i][0];
        out[2 * i + 1] = REFERENCES[i][1];
        i += 1;
    }
    let mut j = 0;
    while j < OTHERS.len() {
        out[2 * REFERENCES.len() + j] = OTHERS[j];
        j += 1;
    }
    out
}

const ALL: [&str; 50] = all();

/// Every board constraint, in the order to run them.
pub const CONSTRAINTS: &[&str] = &ALL;
