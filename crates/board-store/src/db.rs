//! The board's handle on its database, and the small layer every store
//! function uses on top of Toasty.
//!
//! [`Conn`] is a Toasty [`toasty::Db`] plus a [`Runner`] that drives a future
//! to completion from synchronous code. gespan-core passes its ORM runtime's
//! `orm::run`, so the app has one runtime; the test fixture in
//! [`crate::testing`] passes its own. The public functions stay synchronous
//! and each one is one job on that runner: [`with_db!`] for a read,
//! [`with_tx!`] for a write, which takes SQLite's write lock up front
//! (`BEGIN IMMEDIATE`) as the hand-written store did.
//!
//! Inside a job every helper takes `&mut Ex<'_>`, the database or the open
//! transaction. Plain CRUD goes through the typed models in
//! [`crate::models`]; SQL Toasty cannot express goes through [`execute`],
//! [`query`] and friends, which run raw SQL on the same executor.
use std::future::Future;
use std::pin::Pin;
use toasty::stmt::Value;
use toasty_core::driver::operation::TransactionMode;

/// Runs `$body` with `$ex: &mut Ex` on the database, outside a transaction.
macro_rules! with_db {
    ($conn:expr, |$ex:ident| $body:expr) => {
        $conn.run(move |mut db| async move {
            let $ex: &mut $crate::db::Ex<'_> = &mut db;
            $body
        })
    };
}

/// Runs `$body` with `$ex: &mut Ex` inside one `BEGIN IMMEDIATE`
/// transaction, which commits when `$body` is `Ok` and rolls back otherwise.
macro_rules! with_tx {
    ($conn:expr, |$ex:ident| $body:expr) => {
        $conn.run(move |mut db| async move {
            let mut tx = $crate::db::begin(&mut db).await?;
            let result = async {
                let $ex: &mut $crate::db::Ex<'_> = &mut tx;
                $body
            }
            .await;
            $crate::db::finish(tx, result).await
        })
    };
}

pub(crate) use {with_db, with_tx};

/// What a store helper runs its statements on: the database, or the
/// transaction a write holds.
pub type Ex<'a> = dyn toasty::Executor + 'a;

/// One job for a [`Runner`]: a future it must drive to completion before it
/// returns.
pub type Job<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// Drives a [`Job`] to completion on the caller's thread. gespan-core passes
/// `|job| gespan_core::orm::run(job)`.
pub type Runner = fn(Job<'_>);

/// The board's database: a Toasty handle and the runner its jobs run on.
/// Clones share the one connection.
#[derive(Clone)]
pub struct Conn {
    db: toasty::Db,
    runner: Runner,
}

impl std::fmt::Debug for Conn {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Conn").finish_non_exhaustive()
    }
}

impl Conn {
    /// `db` must have been built with [`crate::register_models`]' models and
    /// had [`crate::CONSTRAINTS`] run on it.
    pub fn new(db: toasty::Db, runner: Runner) -> Self {
        Self { db, runner }
    }

    /// A handle for async code; clones share the one connection.
    pub fn db(&self) -> toasty::Db {
        self.db.clone()
    }

    /// Runs `work` with a database handle on the runner and waits for it.
    pub fn run<T, F, Fut>(&self, work: F) -> Result<T, String>
    where
        F: FnOnce(toasty::Db) -> Fut,
        Fut: Future<Output = Result<T, String>> + Send,
        T: Send,
    {
        let future = work(self.db.clone());
        let mut out = None;
        (self.runner)(Box::pin(async {
            out = Some(future.await);
        }));
        out.unwrap_or_else(|| Err("The board database job did not finish".into()))
    }

    /// One raw SQL statement outside any transaction, for tests and tools.
    #[doc(hidden)]
    pub fn execute_sql(&self, sql: &str, args: Vec<Arg>) -> Result<usize, String> {
        with_db!(self, |ex| execute(ex, sql, args).await)
    }

    /// One raw SQL query outside any transaction, for tests and tools.
    #[doc(hidden)]
    pub fn query_sql(&self, sql: &str, args: Vec<Arg>) -> Result<Vec<Row>, String> {
        with_db!(self, |ex| query(ex, sql, args).await)
    }
}

/// A Toasty error as the store reports it. A failure inside SQLite reads as
/// SQLite's own message (`RAISE` text, `UNIQUE constraint failed: …`), as it
/// did when the store ran on rusqlite, without the error codes Toasty adds.
pub fn err(error: toasty::Error) -> String {
    let mut source: Option<&(dyn std::error::Error + 'static)> = Some(&error);
    while let Some(current) = source {
        if let Some(sqlite) = current.downcast_ref::<rusqlite::Error>() {
            return sqlite.to_string();
        }
        source = current.source();
    }
    error.to_string()
}

/// Board writes take SQLite's write lock before reading what they change, so a
/// second app (or connection) writing the same board waits out `busy_timeout`
/// instead of failing at once or acting on rows read before the other write.
pub(crate) async fn begin(db: &mut toasty::Db) -> Result<toasty::Transaction<'_>, String> {
    db.transaction_builder()
        .mode(TransactionMode::Immediate)
        .begin()
        .await
        .map_err(err)
}

pub(crate) async fn finish<T>(
    tx: toasty::Transaction<'_>,
    result: Result<T, String>,
) -> Result<T, String> {
    match result {
        Ok(value) => {
            tx.commit().await.map_err(err)?;
            Ok(value)
        }
        Err(error) => {
            // The error that stopped the write is the one worth reporting.
            let _ = tx.rollback().await;
            Err(error)
        }
    }
}

/// A raw SQL parameter. `NULL` is bound with an explicit type, which Toasty
/// needs to bind it.
#[derive(Clone, Debug, PartialEq)]
pub enum Arg {
    Text(String),
    Int(i64),
    Null,
}

impl From<&str> for Arg {
    fn from(value: &str) -> Self {
        Arg::Text(value.to_string())
    }
}

impl From<String> for Arg {
    fn from(value: String) -> Self {
        Arg::Text(value)
    }
}

impl From<&String> for Arg {
    fn from(value: &String) -> Self {
        Arg::Text(value.clone())
    }
}

impl From<i64> for Arg {
    fn from(value: i64) -> Self {
        Arg::Int(value)
    }
}

impl From<&i64> for Arg {
    fn from(value: &i64) -> Self {
        Arg::Int(*value)
    }
}

impl From<bool> for Arg {
    fn from(value: bool) -> Self {
        Arg::Int(i64::from(value))
    }
}

impl<T: Into<Arg>> From<Option<T>> for Arg {
    fn from(value: Option<T>) -> Self {
        value.map_or(Arg::Null, Into::into)
    }
}

/// `args![a, b]`: raw SQL parameters for `?1`, `?2` and so on.
macro_rules! args {
    () => { Vec::<$crate::db::Arg>::new() };
    ($($value:expr),+ $(,)?) => { vec![$($crate::db::Arg::from($value)),+] };
}
pub(crate) use args;

/// One row of a raw query, its columns in selected order.
#[derive(Clone, Debug)]
pub struct Row(Vec<Value>);

impl Row {
    /// Column `index` as `T`, like `rusqlite::Row::get`.
    pub fn get<T: FromValue>(&self, index: usize) -> Result<T, String> {
        let value = self
            .0
            .get(index)
            .ok_or_else(|| format!("No column {index} in the row"))?;
        T::from_value(value).map_err(|kind| format!("Column {index}: {kind}"))
    }

    /// Column `index` as `T`, moved out of the row (the column reads as
    /// `NULL` afterwards). Large reads use it to skip copying text.
    pub fn take<T: FromValue>(&mut self, index: usize) -> Result<T, String> {
        let value = self
            .0
            .get_mut(index)
            .ok_or_else(|| format!("No column {index} in the row"))?;
        T::from_owned(std::mem::replace(value, Value::Null))
            .map_err(|kind| format!("Column {index}: {kind}"))
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// A Rust value a raw query column reads as.
pub trait FromValue: Sized {
    fn from_value(value: &Value) -> Result<Self, String>;

    /// [`FromValue::from_value`] on a value the caller gives up.
    fn from_owned(value: Value) -> Result<Self, String> {
        Self::from_value(&value)
    }
}

fn integer(value: &Value) -> Option<i64> {
    match value {
        Value::I64(n) => Some(*n),
        Value::I32(n) => Some(i64::from(*n)),
        Value::I16(n) => Some(i64::from(*n)),
        Value::I8(n) => Some(i64::from(*n)),
        Value::U8(n) => Some(i64::from(*n)),
        Value::U16(n) => Some(i64::from(*n)),
        Value::U32(n) => Some(i64::from(*n)),
        Value::U64(n) => i64::try_from(*n).ok(),
        Value::Bool(b) => Some(i64::from(*b)),
        _ => None,
    }
}

impl FromValue for i64 {
    fn from_value(value: &Value) -> Result<Self, String> {
        integer(value).ok_or_else(|| format!("expected an integer, found {value:?}"))
    }
}

impl FromValue for bool {
    fn from_value(value: &Value) -> Result<Self, String> {
        i64::from_value(value).map(|n| n != 0)
    }
}

impl FromValue for f64 {
    fn from_value(value: &Value) -> Result<Self, String> {
        match value {
            Value::F64(n) => Ok(*n),
            Value::F32(n) => Ok(f64::from(*n)),
            other => i64::from_value(other).map(|n| n as f64),
        }
    }
}

impl FromValue for String {
    fn from_value(value: &Value) -> Result<Self, String> {
        match value {
            Value::String(text) => Ok(text.clone()),
            other => Err(format!("expected text, found {other:?}")),
        }
    }

    fn from_owned(value: Value) -> Result<Self, String> {
        match value {
            Value::String(text) => Ok(text),
            other => Err(format!("expected text, found {other:?}")),
        }
    }
}

impl<T: FromValue> FromValue for Option<T> {
    fn from_value(value: &Value) -> Result<Self, String> {
        match value {
            Value::Null => Ok(None),
            other => T::from_value(other).map(Some),
        }
    }

    fn from_owned(value: Value) -> Result<Self, String> {
        match value {
            Value::Null => Ok(None),
            other => T::from_owned(other).map(Some),
        }
    }
}

fn bind_statement(mut statement: toasty::sql::Statement, args: Vec<Arg>) -> toasty::sql::Statement {
    for arg in args {
        statement = match arg {
            Arg::Text(text) => statement.bind(text),
            Arg::Int(n) => statement.bind(n),
            Arg::Null => statement.bind_typed(Value::Null, toasty::schema::db::Type::Text),
        };
    }
    statement
}

fn bind_query(mut query: toasty::sql::Query, args: Vec<Arg>) -> toasty::sql::Query {
    for arg in args {
        query = match arg {
            Arg::Text(text) => query.bind(text),
            Arg::Int(n) => query.bind(n),
            Arg::Null => query.bind_typed(Value::Null, toasty::schema::db::Type::Text),
        };
    }
    query
}

/// Runs one raw SQL statement that returns no rows. Returns the rows changed.
pub(crate) async fn execute(ex: &mut Ex<'_>, sql: &str, args: Vec<Arg>) -> Result<usize, String> {
    bind_statement(toasty::sql::statement(sql), args)
        .exec(ex)
        .await
        .map(|count| count as usize)
        .map_err(err)
}

/// Runs one raw SQL query. Returns every row.
pub(crate) async fn query(ex: &mut Ex<'_>, sql: &str, args: Vec<Arg>) -> Result<Vec<Row>, String> {
    let rows = bind_query(toasty::sql::query(sql), args)
        .exec(ex)
        .await
        .map_err(err)?;
    Ok(rows
        .into_iter()
        .map(|row| match row {
            Value::Record(record) => Row(record.fields),
            other => Row(vec![other]),
        })
        .collect())
}

/// The first row of a raw query, if any.
pub(crate) async fn query_opt(
    ex: &mut Ex<'_>,
    sql: &str,
    args: Vec<Arg>,
) -> Result<Option<Row>, String> {
    Ok(query(ex, sql, args).await?.into_iter().next())
}

/// The first column of the first row of a raw query, which must have a row
/// (an aggregate, `EXISTS` and the like).
pub(crate) async fn query_value<T: FromValue>(
    ex: &mut Ex<'_>,
    sql: &str,
    args: Vec<Arg>,
) -> Result<T, String> {
    query_opt(ex, sql, args)
        .await?
        .ok_or_else(|| "Query returned no rows".to_string())?
        .get(0)
}
