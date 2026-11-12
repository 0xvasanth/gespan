//! Synchronous SQL for the store's code that Toasty's typed API cannot
//! express: column projections that must stay on the sessions covering index
//! (a typed read loads every column, `blocks_json` included), json1, counted
//! conditional updates, `INSERT ... SELECT` and multi-statement transactions
//! shared by helpers.
//!
//! A writable store runs every statement on its Toasty database
//! ([`crate::orm::Database`]); a read-only store (`Store::read_gespan` and the
//! search reader) runs the same SQL on its read-only SQLite connection.
//! Placeholders are `?1`, `?2` and so on in both.
use rusqlite::Connection;
use toasty::schema::db::Type;
use toasty::stmt::Value;

/// One result row: its columns in select order.
#[derive(Clone, Debug)]
pub(crate) struct SqlRow(Vec<Value>);

impl SqlRow {
    /// The column at `index`, converted like `rusqlite::Row::get`.
    pub(crate) fn get<T: FromColumn>(&self, index: usize) -> Result<T, String> {
        let value = self
            .0
            .get(index)
            .ok_or_else(|| format!("Invalid column index: {index}"))?;
        T::from_column(value).map_err(|error| format!("Column {index}: {error}"))
    }
}

/// A Rust value read from one SQLite column.
pub(crate) trait FromColumn: Sized {
    fn from_column(value: &Value) -> Result<Self, String>;
}

fn mismatch(expected: &str, value: &Value) -> String {
    format!("expected {expected}, found {value:?}")
}

impl FromColumn for String {
    fn from_column(value: &Value) -> Result<Self, String> {
        match value {
            Value::String(text) => Ok(text.clone()),
            Value::Bytes(bytes) => String::from_utf8(bytes.clone()).map_err(|e| e.to_string()),
            other => Err(mismatch("text", other)),
        }
    }
}

impl FromColumn for i64 {
    fn from_column(value: &Value) -> Result<Self, String> {
        match value {
            Value::I64(number) => Ok(*number),
            Value::Bool(flag) => Ok(i64::from(*flag)),
            other => Err(mismatch("an integer", other)),
        }
    }
}

impl FromColumn for bool {
    fn from_column(value: &Value) -> Result<Self, String> {
        match value {
            Value::I64(number) => Ok(*number != 0),
            Value::Bool(flag) => Ok(*flag),
            other => Err(mismatch("a boolean", other)),
        }
    }
}

impl FromColumn for f64 {
    fn from_column(value: &Value) -> Result<Self, String> {
        match value {
            Value::F64(number) => Ok(*number),
            Value::I64(number) => Ok(*number as f64),
            other => Err(mismatch("a number", other)),
        }
    }
}

impl<T: FromColumn> FromColumn for Option<T> {
    fn from_column(value: &Value) -> Result<Self, String> {
        match value {
            Value::Null => Ok(None),
            value => T::from_column(value).map(Some),
        }
    }
}

/// A Rust value bound to a placeholder.
pub(crate) trait Arg {
    fn arg(self) -> Value;
}

impl Arg for Value {
    fn arg(self) -> Value {
        self
    }
}
impl Arg for &str {
    fn arg(self) -> Value {
        Value::String(self.to_owned())
    }
}
impl Arg for String {
    fn arg(self) -> Value {
        Value::String(self)
    }
}
impl Arg for &String {
    fn arg(self) -> Value {
        Value::String(self.clone())
    }
}
impl Arg for i64 {
    fn arg(self) -> Value {
        Value::I64(self)
    }
}
impl Arg for &i64 {
    fn arg(self) -> Value {
        Value::I64(*self)
    }
}
impl Arg for bool {
    fn arg(self) -> Value {
        Value::I64(i64::from(self))
    }
}
impl Arg for Vec<u8> {
    fn arg(self) -> Value {
        Value::Bytes(self)
    }
}
impl<T: Arg> Arg for Option<T> {
    fn arg(self) -> Value {
        self.map_or(Value::Null, Arg::arg)
    }
}

/// Placeholder values, in order: `args![id, title]`.
macro_rules! args {
    ($($value:expr),* $(,)?) => {
        vec![$($crate::store::db::Arg::arg($value)),*]
    };
}
pub(crate) use args;

/// The raw driver ignores the declared type of a bound value, but Toasty
/// needs one to bind a NULL.
fn bind_type(value: &Value) -> Type {
    match value {
        Value::I64(_) | Value::Bool(_) => Type::Integer(8),
        Value::F64(_) => Type::Float(8),
        Value::Bytes(_) => Type::Blob,
        _ => Type::Text,
    }
}

fn to_sqlite(value: Value) -> rusqlite::types::Value {
    use rusqlite::types::Value as Sqlite;
    match value {
        Value::Null => Sqlite::Null,
        Value::Bool(flag) => Sqlite::Integer(i64::from(flag)),
        Value::I64(number) => Sqlite::Integer(number),
        Value::F64(number) => Sqlite::Real(number),
        Value::String(text) => Sqlite::Text(text),
        Value::Bytes(bytes) => Sqlite::Blob(bytes),
        other => Sqlite::Text(format!("{other:?}")),
    }
}

fn from_sqlite(value: rusqlite::types::ValueRef<'_>) -> Value {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => Value::I64(number),
        ValueRef::Real(number) => Value::F64(number),
        ValueRef::Text(text) => Value::String(String::from_utf8_lossy(text).into_owned()),
        ValueRef::Blob(bytes) => Value::Bytes(bytes.to_vec()),
    }
}

/// A raw statement with `args` bound, for code that runs it itself (on a
/// Toasty transaction, say: `statement(..).exec(&mut *tx)`).
pub(crate) fn statement(sql: &str, args: Vec<Value>) -> toasty::sql::Statement {
    args.into_iter()
        .fold(toasty::sql::statement(sql), |s, arg| {
            let ty = bind_type(&arg);
            s.bind_typed(arg, ty)
        })
}

/// A raw query with `args` bound; [`rows`] reads what it returns.
pub(crate) fn query(sql: &str, args: Vec<Value>) -> toasty::sql::Query {
    args.into_iter().fold(toasty::sql::query(sql), |q, arg| {
        let ty = bind_type(&arg);
        q.bind_typed(arg, ty)
    })
}

/// The rows a raw Toasty query returned.
pub(crate) fn rows(values: Vec<Value>) -> Result<Vec<SqlRow>, String> {
    values
        .into_iter()
        .map(|row| match row {
            Value::Record(record) => Ok(SqlRow(record.fields)),
            other => Err(format!("Unexpected row {other:?}")),
        })
        .collect()
}

/// Where a store's SQL runs.
#[derive(Clone, Copy)]
pub(crate) enum Sql<'a> {
    /// The store's Toasty database: every writable store.
    Orm(&'a crate::orm::Database),
    /// A plain SQLite connection: read-only stores, and the legacy native
    /// tables of `Store::open`.
    Conn(&'a Connection),
}

impl<'a> Sql<'a> {
    /// Every row `sql` returns.
    pub(crate) fn query(self, sql: &str, args: Vec<Value>) -> Result<Vec<SqlRow>, String> {
        match self {
            Sql::Orm(database) => {
                let query = query(sql, args);
                rows(database.run(|mut db| async move { query.exec(&mut db).await })?)
            }
            Sql::Conn(conn) => {
                let mut statement = conn.prepare_cached(sql).map_err(|e| e.to_string())?;
                let width = statement.column_count();
                let mut rows = statement
                    .query(rusqlite::params_from_iter(args.into_iter().map(to_sqlite)))
                    .map_err(|e| e.to_string())?;
                let mut out = Vec::new();
                while let Some(row) = rows.next().map_err(|e| e.to_string())? {
                    out.push(SqlRow(
                        (0..width)
                            .map(|ix| row.get_ref(ix).map(from_sqlite))
                            .collect::<rusqlite::Result<_>>()
                            .map_err(|e| e.to_string())?,
                    ));
                }
                Ok(out)
            }
        }
    }

    /// The Toasty database, for typed writes; a read-only store has none.
    pub(crate) fn orm(self) -> Result<&'a crate::orm::Database, String> {
        match self {
            Sql::Orm(database) => Ok(database),
            Sql::Conn(_) => Err("This Gespan store is read-only".into()),
        }
    }

    /// The first row, or `None`.
    pub(crate) fn optional(self, sql: &str, args: Vec<Value>) -> Result<Option<SqlRow>, String> {
        Ok(self.query(sql, args)?.into_iter().next())
    }

    /// The first row; no row is an error, as with `rusqlite::Connection::query_row`.
    pub(crate) fn row(self, sql: &str, args: Vec<Value>) -> Result<SqlRow, String> {
        self.optional(sql, args)?
            .ok_or_else(|| rusqlite::Error::QueryReturnedNoRows.to_string())
    }

    /// The first column of the first row.
    pub(crate) fn value<T: FromColumn>(self, sql: &str, args: Vec<Value>) -> Result<T, String> {
        self.row(sql, args)?.get(0)
    }

    /// Runs a statement that returns no rows; the number of rows it changed.
    pub(crate) fn execute(self, sql: &str, args: Vec<Value>) -> Result<usize, String> {
        match self {
            Sql::Orm(database) => {
                let statement = statement(sql, args);
                database
                    .run(|mut db| async move { statement.exec(&mut db).await })
                    .map(|count| count as usize)
            }
            Sql::Conn(conn) => conn
                .prepare_cached(sql)
                .and_then(|mut statement| {
                    statement.execute(rusqlite::params_from_iter(args.into_iter().map(to_sqlite)))
                })
                .map_err(|e| e.to_string()),
        }
    }

    /// Starts a transaction that takes the write lock up front, like
    /// `store::write_transaction`. Statements this store runs before
    /// [`SqlTx::commit`] are part of it: its database has one connection.
    /// Dropping the guard without committing rolls back.
    ///
    /// Inside, use raw SQL only. Toasty's typed API starts its own
    /// transaction for some statements, which SQLite refuses inside this one.
    pub(crate) fn begin(self) -> Result<SqlTx<'a>, String> {
        self.execute("BEGIN IMMEDIATE", Vec::new())?;
        Ok(SqlTx {
            sql: self,
            done: false,
        })
    }
}

/// An open transaction from [`Sql::begin`].
pub(crate) struct SqlTx<'a> {
    sql: Sql<'a>,
    done: bool,
}

impl SqlTx<'_> {
    pub(crate) fn commit(mut self) -> Result<(), String> {
        self.sql.execute("COMMIT", Vec::new())?;
        self.done = true;
        Ok(())
    }
}

impl Drop for SqlTx<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.sql.execute("ROLLBACK", Vec::new());
        }
    }
}
