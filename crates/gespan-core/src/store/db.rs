//! Synchronous SQL for the store's code that Toasty's typed API cannot
//! express: the sidebar list that must read only the sessions covering index
//! (`INDEXED BY`), json1, bounded `substr` reads, counted conditional updates,
//! and the `BEGIN IMMEDIATE` transactions shared by helpers.
//!
//! Every statement runs on the store's Toasty database
//! ([`crate::orm::Database`]), which a read-only store (`Store::read_gespan`
//! and the search reader) opens read-only. Placeholders are `?1`, `?2` and so
//! on.
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

/// A raw statement with `args` bound, for code that runs it itself (on a
/// Toasty transaction, say: `statement(..).exec(&mut *tx)`).
pub(crate) fn statement(sql: &str, args: Vec<Value>) -> toasty::sql::Statement {
    args.into_iter()
        .fold(toasty::sql::statement(sql), |s, arg| {
            let ty = bind_type(&arg);
            s.bind_typed(arg, ty)
        })
}

/// Where a store's SQL runs: its Toasty database.
#[derive(Clone, Copy)]
pub(crate) struct Sql<'a>(pub(crate) &'a crate::orm::Database);

impl<'a> Sql<'a> {
    /// Every row `sql` returns.
    pub(crate) fn query(self, sql: &str, args: Vec<Value>) -> Result<Vec<SqlRow>, String> {
        let mut query = toasty::sql::query(sql);
        for arg in args {
            let ty = bind_type(&arg);
            query = query.bind_typed(arg, ty);
        }
        let rows = self
            .0
            .run(|mut db| async move { query.exec(&mut db).await })?;
        rows.into_iter()
            .map(|row| match row {
                Value::Record(record) => Ok(SqlRow(record.fields)),
                other => Err(format!("Unexpected row {other:?}")),
            })
            .collect()
    }

    /// The first row, or `None`.
    pub(crate) fn optional(self, sql: &str, args: Vec<Value>) -> Result<Option<SqlRow>, String> {
        Ok(self.query(sql, args)?.into_iter().next())
    }

    /// The first row; no row is an error, as with `rusqlite::Connection::query_row`.
    pub(crate) fn row(self, sql: &str, args: Vec<Value>) -> Result<SqlRow, String> {
        self.optional(sql, args)?
            .ok_or_else(|| "Query returned no rows".to_owned())
    }

    /// The first column of the first row.
    pub(crate) fn value<T: FromColumn>(self, sql: &str, args: Vec<Value>) -> Result<T, String> {
        self.row(sql, args)?.get(0)
    }

    /// Runs a statement that returns no rows; the number of rows it changed.
    pub(crate) fn execute(self, sql: &str, args: Vec<Value>) -> Result<usize, String> {
        let statement = statement(sql, args);
        self.0
            .run(|mut db| async move { statement.exec(&mut db).await })
            .map(|count| count as usize)
    }

    /// Starts a transaction that takes the write lock up front
    /// (`BEGIN IMMEDIATE`). Statements this store runs before
    /// [`SqlTx::commit`] are part of it: its database has one connection.
    /// Dropping the guard without committing rolls back.
    ///
    /// Inside, a typed statement must be one database operation: Toasty
    /// starts its own transaction for a plan of several, which SQLite
    /// refuses inside this one.
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
