//! `GraphStore`: one connection to `graph.db`.
use std::path::{Path, PathBuf};

use rusqlite::Connection;

use crate::error::GraphError;
use crate::{migrate, schema};

/// The graph database's file name. The caller chooses the directory.
pub const FILE_NAME: &str = "graph.db";

/// The knowledge graph store. Every call blocks: use it from a background
/// thread, never the UI thread or an async executor. `Send`, not `Sync`.
pub struct GraphStore {
    pub(crate) conn: Connection,
    path: PathBuf,
}

impl GraphStore {
    /// Opens (creating if needed) the graph database at `path` and brings it
    /// to the current schema. A file written by a newer build is refused
    /// with `NewerSchema` before anything is written to it.
    pub fn open(path: &Path) -> Result<GraphStore, GraphError> {
        #[cfg(unix)]
        create_private(path)?;
        let conn = Connection::open(path)?;
        let found = migrate::version(&conn)?;
        if found > schema::VERSION {
            return Err(GraphError::NewerSchema {
                found,
                supported: schema::VERSION,
            });
        }
        conn.execute_batch(
            "PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL; PRAGMA busy_timeout = 5000;",
        )?;
        migrate::run(&conn)?;
        Ok(GraphStore {
            conn,
            path: path.to_path_buf(),
        })
    }

    /// The file this store opened.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The raw connection, for tests and the seeded generator only.
    #[doc(hidden)]
    pub fn connection(&self) -> &Connection {
        &self.conn
    }
}

/// Creates the file owner-only before SQLite first opens it, as the
/// settings sidecar does.
#[cfg(unix)]
fn create_private(path: &Path) -> Result<(), GraphError> {
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(e) => Err(e.into()),
    }
}
