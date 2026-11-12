//! Test support: raw SQL on a store's own database, shaped like the rusqlite
//! calls tests used before the store dropped its rusqlite connection. Tests
//! arrange rows the store's API cannot make (older shapes, broken JSON,
//! hidden sessions) with it. Production code never uses it.
use super::db::{FromColumn, SqlRow};
use super::Store;
use rusqlite::types::{ToSqlOutput, ValueRef};
use rusqlite::ToSql;
use toasty::stmt::Value;

/// Placeholder values, as rusqlite takes them: `[a, b]`, `params![a, b]` or `[]`.
pub(crate) trait TestParams {
    fn values(self) -> Vec<Value>;
}

fn value(arg: &dyn ToSql) -> Value {
    let output = arg.to_sql().expect("a bindable test value");
    let owned;
    let value = match &output {
        ToSqlOutput::Borrowed(value) => *value,
        ToSqlOutput::Owned(value) => {
            owned = value.clone();
            ValueRef::from(&owned)
        }
        other => panic!("unsupported test value {other:?}"),
    };
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => Value::I64(number),
        ValueRef::Real(number) => Value::F64(number),
        ValueRef::Text(text) => Value::String(String::from_utf8_lossy(text).into_owned()),
        ValueRef::Blob(bytes) => Value::Bytes(bytes.to_vec()),
    }
}

impl TestParams for &[&dyn ToSql] {
    fn values(self) -> Vec<Value> {
        self.iter().map(|arg| value(*arg)).collect()
    }
}

impl TestParams for [&dyn ToSql; 0] {
    fn values(self) -> Vec<Value> {
        Vec::new()
    }
}

macro_rules! array_params {
    ($($n:literal)*) => {$(
        impl<T: ToSql> TestParams for [T; $n] {
            fn values(self) -> Vec<Value> {
                self.iter().map(|arg| value(arg)).collect()
            }
        }
    )*};
}
array_params!(1 2 3 4 5 6 7 8);

/// A row in a `query_row` closure.
pub(crate) struct TestRow(SqlRow);

/// A column index, as rusqlite's `Row::get` takes it.
pub(crate) trait TestIndex {
    fn index(self) -> usize;
}

impl TestIndex for usize {
    fn index(self) -> usize {
        self
    }
}

impl TestRow {
    pub(crate) fn get<I: TestIndex, T: FromColumn>(&self, index: I) -> Result<T, String> {
        self.0.get(index.index())
    }
}

pub(crate) struct TestConn<'a>(&'a Store);

impl TestConn<'_> {
    pub(crate) fn execute(&self, sql: &str, params: impl TestParams) -> Result<usize, String> {
        self.0.sql()?.execute(sql, params.values())
    }

    /// Runs each `;`-separated statement of `sql`. A trigger's
    /// `BEGIN ... END` body stays in its statement.
    pub(crate) fn execute_batch(&self, sql: &str) -> Result<(), String> {
        let mut statement = String::new();
        for part in sql.split_inclusive(';') {
            statement.push_str(part);
            let upper = statement.to_uppercase();
            let open =
                upper.contains("BEGIN") && !upper.trim_end_matches(';').trim_end().ends_with("END");
            if open {
                continue;
            }
            if !statement.trim().trim_end_matches(';').trim().is_empty() {
                self.0.sql()?.execute(&statement, Vec::new())?;
            }
            statement.clear();
        }
        if !statement.trim().is_empty() {
            self.0.sql()?.execute(&statement, Vec::new())?;
        }
        Ok(())
    }

    pub(crate) fn query_row<T>(
        &self,
        sql: &str,
        params: impl TestParams,
        read: impl FnOnce(&TestRow) -> Result<T, String>,
    ) -> Result<T, String> {
        read(&TestRow(self.0.sql()?.row(sql, params.values())?))
    }

    /// Every row `sql` returns, each read by `read`.
    pub(crate) fn query_map<T>(
        &self,
        sql: &str,
        params: impl TestParams,
        read: impl FnMut(&TestRow) -> Result<T, String>,
    ) -> Result<Vec<T>, String> {
        self.0
            .sql()?
            .query(sql, params.values())?
            .into_iter()
            .map(TestRow)
            .collect::<Vec<_>>()
            .iter()
            .map(read)
            .collect()
    }
}

impl Store {
    /// Raw SQL on this store's database, for tests only.
    pub(crate) fn conn(&self) -> TestConn<'_> {
        TestConn(self)
    }
}
