use crate::key::NodeRef;
use crate::kind::{EdgeKind, NodeKind};

/// Everything a graph operation can fail with.
#[derive(Debug, thiserror::Error)]
pub enum GraphError {
    #[error("graph database: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("graph database: {0}")]
    Toasty(#[from] toasty::Error),
    #[error("graph file: {0}")]
    Io(#[from] std::io::Error),
    #[error("graph props: {0}")]
    Props(#[from] serde_json::Error),
    /// The file has migrations this build does not know: a newer build
    /// wrote it. Nothing was written to it.
    #[error("graph database has migrations this build does not know: {}", .0.join(", "))]
    UnknownMigrations(Vec<String>),
    #[error("{edge:?} cannot join {src:?} to {dst:?}")]
    InvalidEndpoints {
        edge: EdgeKind,
        src: NodeKind,
        dst: NodeKind,
    },
    #[error("an edge cannot join {0:?} to itself")]
    SelfLoop(NodeRef),
    #[error("graph node {0:?} does not exist")]
    MissingNode(NodeRef),
    #[error("{0:?} is not an anchor kind")]
    NotAnchor(NodeKind),
    #[error("hop count {0} is outside 1..=4")]
    InvalidHops(u8),
    #[error("unknown kind {0:?}")]
    UnknownKind(String),
    #[error("graph node key is empty")]
    EmptyKey,
}
