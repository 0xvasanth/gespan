# Toasty in gespan-core

Gespan's own SQLite databases move from hand-written rusqlite to the
[Toasty](https://github.com/tokio-rs/toasty) ORM, pinned to `=0.11.0`. The code
is in `crates/gespan-core/src/orm/`, and the migrations are in
`crates/gespan-core/migrations/<kind>/`.

## Opening and querying

```rust
let database = gespan_core::orm::open(Kind::Gespan, &path)?; // or ":memory:"

// Typed queries. `run` blocks until the future finishes.
let task = database.run(|mut db| async move {
    MyTask::filter_by_key(key).first().exec(&mut db).await
})?;

// Raw SQL only for what Toasty cannot express (see "Toasty 0.11 limits to
// know"), with a comment saying which. Placeholders are ?1, ?2 and so on.
let n = database.run(|mut db| async move {
    toasty::sql::statement("UPDATE my_tasks SET status = 'withdrawn' WHERE asked_at < ?1")
        .bind(at)
        .exec(&mut db)
        .await
})?;

// One BEGIN IMMEDIATE transaction. It commits on Ok and rolls back on Err.
database.transaction(|tx| Box::pin(async move {
    TaskDecision::create().project(p).statement(s).why(w)
        .created_at(t).updated_at(t).exec(tx).await?;
    Ok(())
}))?;
```

The pool has one connection. Inside a transaction, run everything through
`tx`. Calling the database from inside the transaction deadlocks.
`orm::run(fut)` drives any Toasty future from synchronous code. It is safe to
call from inside another tokio runtime, but not from inside a future that
`run` is already driving.

`Store` holds the gespan database and no other connection to it. Store code
calls `self.orm()?.run(...)`. `Store::open` (tests and fixtures) opens the
same schema as `Store::open_gespan`. A read-only store (`Store::read_gespan`,
the search reader) opens the file with `orm::open_read_only`: the typed API
reads it as usual and SQLite refuses every write. A file Gespan has not
adopted yet is read through an adopted copy, so the file itself never
changes.

Queries use the typed API. `tests/toasty_only.rs` counts every file's raw
query lines in `QUERIES`: the count can only go down, so a change that moves a
file's queries to the typed API lowers its number there.

Store code that still needs raw SQL uses `Store::sql()` (`store/db.rs`): `query`,
`row`, `optional`, `value` and `execute` with `args![...]`, and `begin()`
for a `BEGIN IMMEDIATE` transaction that the store's other statements join
(the pool has one connection). Inside `begin()`, a typed statement must be a
single database operation (one select, insert, update, delete or upsert):
Toasty wraps a plan of several in its own transaction, which SQLite refuses
there.

Tests arrange rows the store's API cannot make with `store.conn()`
(`store/conn_tests.rs`), which runs raw SQL on the store's database with
rusqlite-shaped calls.

## Adding a model

1. Write a `#[derive(toasty::Model)]` struct with `#[table = "<existing name>"]`
   and every column the table has ever had, using the exact names (rename with
   `#[column("...")]`). A column left out of the model is dropped when a file
   is adopted.
2. Register the model in `Kind::models` in `src/orm/kind.rs`.
3. If the table had older hand-made shapes, move the store's
   `ALTER TABLE` / `pragma_table_info` upgrades into that kind's
   `Kind::legacy_upgrade`. Adoption runs them before it rebuilds the table.
4. Generate the migration:

   ```sh
   cargo run --manifest-path crates/gespan-core/Cargo.toml --bin toasty-migrate -- gespan add_notes
   ```

   This writes `migrations/gespan/migrations/NNNN_add_notes.sql`, the
   snapshot it leads to, and the `history.toml` entry. Commit all three.
   While a kind's migrations have not shipped, regenerate them as one first
   migration with `--reset` (`toasty-migrate gespan init --reset`). Adoption
   only rebuilds tables that the *first* migration creates.
5. `cargo test` runs `orm::tests::models_match_the_last_migration`. It fails
   when a model and the last snapshot differ.

## What happens on open

- **Backup:** before adoption or any pending migration changes an existing
  file, the file is copied to `<file>.bak-pre-toasty` with `VACUUM INTO`. The
  copy replaces any older backup.
- **Adoption:** this runs when a file has tables but no `__toasty_migrations`.
  It is one transaction with foreign keys off. It runs the legacy upgrade,
  then rebuilds each table the first migration creates into exactly that DDL:
  create the new table, copy the shared columns, drop the old one, rename.
  Then it creates the indexes and records the first migration as applied.
- **Migrate:** `MigrationSet::apply` applies everything else.

## Toasty 0.11 limits to know

- Generated DDL has no `DEFAULT`, no `REFERENCES` and no `DESC` index columns.
  The model must set every value. Foreign keys are not enforced by the schema.
- A new `NOT NULL` column on a table that already shipped is generated as
  `ALTER TABLE ... ADD COLUMN ... NOT NULL` with no default, which SQLite
  refuses. Edit the generated `.sql` by hand: add a `DEFAULT` to that
  `ADD COLUMN`, and end the migration with an `UPDATE` that gives existing
  rows their real values. `gespan/0001_decision_log.sql` does both. The
  snapshot stays as generated.
- An `#[auto]` integer key is stored as a 4-byte `INTEGER ... AUTOINCREMENT`
  column.
- Field-level `#[index]` cannot be named. Use a struct-level
  `#[index(name = "...", field)]` to keep an existing index name.
- Conditional upserts (`ON CONFLICT ... WHERE`), CTEs, `json_*`, `substr`,
  scalar subqueries in a projection, `INDEXED BY` and window functions go
  through `toasty::sql`. Binding a NULL needs `bind_typed`.
- `select` projects a field or a tuple of up to eight; nest tuples for more.
  `offset` needs a `limit` first.
- Queries against FTS5 and sqlite-vec virtual tables, which Toasty cannot
  model, go through `toasty::sql`.
- Plain upserts are typed: `Model::upsert_by_<key>(..)` replaces the row,
  `.on_create(..)` sets columns only a new row gets, `.on_update(..)` takes
  `toasty::stmt::increment()`, and `.or_ignore()` is `INSERT OR IGNORE`
  (it returns `None` on a conflict, which stands in for its count).
- There is no `rowid`. A query that breaks ties by insertion order
  (`ORDER BY position, rowid`) stays raw.
- There is no `GROUP BY` and no aggregate but `count()`. Per-group counts in
  one statement stay raw. A single `MAX` is an `order_by(..desc())` with
  `first()`.
- Joins exist only through relations, and the board models declare none.
  Use `in_query` with a `select` subquery, or two queries joined in Rust.
- `ORDER BY lower(name)` has no typed form. Sort in Rust with
  `to_ascii_lowercase`, which folds the same ASCII letters SQLite's `lower`
  does.
- Updates and deletes return no row count. Inside a write transaction, a
  read followed by a typed update is a compare-and-set, and counting the
  rows first with `count()` or `first()` gives an update's or delete's
  count.
- The SQLite driver panics when a stored value doesn't match the model's
  type, which ends that connection's task. A read the app must survive a
  corrupt row on (board settings, the saved-template list) stays raw, where a
  bad value is an error. A test that corrupts a row on purpose reads it on a
  connection of its own.
- Toasty brings `PartialEq` impls for tuples into scope. An
  `assert_eq!(x, [])` on a tuple vector may then need a type annotation.
