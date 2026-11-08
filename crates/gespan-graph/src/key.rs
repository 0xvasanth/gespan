//! Node identities, edge provenance and the props attached to both.
use std::collections::BTreeMap;

use crate::error::GraphError;
use crate::kind::NodeKind;

/// A node's natural key within its kind. Stored verbatim: 1a never parses
/// or normalises it (typed per-kind constructors arrive with #386).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeKey(String);

impl NodeKey {
    pub fn new(key: impl Into<String>) -> Result<NodeKey, GraphError> {
        let key = key.into();
        if key.is_empty() {
            return Err(GraphError::EmptyKey);
        }
        Ok(NodeKey(key))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A node's external identity: its kind and natural key.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeRef {
    pub kind: NodeKind,
    pub key: NodeKey,
}

impl NodeRef {
    pub fn new(kind: NodeKind, key: impl Into<String>) -> Result<NodeRef, GraphError> {
        Ok(NodeRef {
            kind,
            key: NodeKey::new(key)?,
        })
    }

    /// The node for a persisted session.
    pub fn session(id: &SessionId) -> Result<NodeRef, GraphError> {
        NodeRef::new(NodeKind::Session, id.0.clone())
    }
}

/// A persisted `sessions.id`, verbatim.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SessionId(pub String);

/// A node's rowid. Valid only within one `graph.db`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct NodeId(pub i64);

/// An edge's rowid. Valid only within one `graph.db`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct EdgeId(pub i64);

/// Who asserted an edge, which is what deleting by owner needs.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    /// Asserted by one anchor; deleting that anchor deletes the edge.
    Owner(NodeRef),
    /// Asserted by many owners (e.g. a worktree's project); the earliest
    /// `valid_from` wins.
    Structural,
}

impl Scope {
    /// `"<kind>:<key>"` or `"structural"`.
    pub fn as_str(&self) -> String {
        match self {
            Scope::Owner(node) => scope_of(node),
            Scope::Structural => "structural".to_string(),
        }
    }
}

pub(crate) fn scope_of(node: &NodeRef) -> String {
    format!("{}:{}", node.kind.as_str(), node.key.as_str())
}

/// Where an edge fact came from. Stored as text with no SQL `CHECK`, so a
/// new variant (such as #132's live events) is not a schema change.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    Backfill(BackfillSource),
}

impl Source {
    /// `"backfill:<source>"`.
    pub fn as_str(&self) -> String {
        match self {
            Source::Backfill(source) => format!("backfill:{}", source.as_str()),
        }
    }
}

/// The source readers #386's backfill will have.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BackfillSource {
    Sessions,
    SessionWorktree,
    SessionBranch,
    LinkedWorkItem,
    Blocks,
    BoardCards,
    BoardLinks,
    BoardRelations,
    BoardProjects,
    Notes,
    AutomationRuns,
    PilotRoster,
}

impl BackfillSource {
    pub fn as_str(self) -> &'static str {
        match self {
            BackfillSource::Sessions => "sessions",
            BackfillSource::SessionWorktree => "session_worktree",
            BackfillSource::SessionBranch => "session_branch",
            BackfillSource::LinkedWorkItem => "linked_work_item",
            BackfillSource::Blocks => "blocks",
            BackfillSource::BoardCards => "board_cards",
            BackfillSource::BoardLinks => "board_links",
            BackfillSource::BoardRelations => "board_relations",
            BackfillSource::BoardProjects => "board_projects",
            BackfillSource::Notes => "notes",
            BackfillSource::AutomationRuns => "automation_runs",
            BackfillSource::PilotRoster => "pilot_roster",
        }
    }
}

/// How an edge's `valid_from` was chosen. 1a stores what the caller says;
/// #386 owns the rules that pick it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TimeBasis {
    /// The source records when the fact began.
    Native,
    /// The owning row's creation time stands in.
    OwnerCreated,
    /// The time the fact was first observed.
    ObservedBy,
}

impl TimeBasis {
    pub fn as_str(self) -> &'static str {
        match self {
            TimeBasis::Native => "native",
            TimeBasis::OwnerCreated => "owner_created",
            TimeBasis::ObservedBy => "observed_by",
        }
    }
}

/// Free-form attributes on a node or edge, serialised once in key order.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Props(pub BTreeMap<&'static str, serde_json::Value>);

impl Props {
    /// The stored JSON; empty props are `{}`.
    pub fn to_json(&self) -> Result<String, GraphError> {
        Ok(serde_json::to_string(&self.0)?)
    }
}
