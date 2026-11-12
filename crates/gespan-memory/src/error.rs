/// Everything a decision-log operation can fail with.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("decision database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("decision file: {0}")]
    Io(#[from] std::io::Error),
    #[error("decision schema version {found} is newer than this build ({supported})")]
    NewerSchema { found: i64, supported: i64 },
    #[error("an entry needs a reason (why)")]
    MissingWhy,
    #[error("entry {0} does not exist")]
    NotFound(i64),
    #[error("entry {0} is not pending")]
    NotPending(i64),
    #[error("entry {0} is not active")]
    NotActive(i64),
    #[error("entry {0} cannot replace itself")]
    SelfReplace(i64),
    #[error("flag {0} does not exist")]
    FlagNotFound(i64),
    #[error("stored value {0:?} is not recognised")]
    BadValue(String),
}
