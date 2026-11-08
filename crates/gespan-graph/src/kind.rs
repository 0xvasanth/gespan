//! The fixed node and edge kinds, and which kinds an edge may join.
use std::str::FromStr;

use crate::error::GraphError;

/// What a graph node stands for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum NodeKind {
    Project,
    Repo,
    Worktree,
    Branch,
    Session,
    Pilot,
    AutomationRun,
    File,
    Pr,
    Ticket,
    Card,
    Person,
    Note,
    Memory,
}

impl NodeKind {
    pub const ALL: [NodeKind; 14] = [
        NodeKind::Project,
        NodeKind::Repo,
        NodeKind::Worktree,
        NodeKind::Branch,
        NodeKind::Session,
        NodeKind::Pilot,
        NodeKind::AutomationRun,
        NodeKind::File,
        NodeKind::Pr,
        NodeKind::Ticket,
        NodeKind::Card,
        NodeKind::Person,
        NodeKind::Note,
        NodeKind::Memory,
    ];

    /// The stored, snake_case name.
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Project => "project",
            NodeKind::Repo => "repo",
            NodeKind::Worktree => "worktree",
            NodeKind::Branch => "branch",
            NodeKind::Session => "session",
            NodeKind::Pilot => "pilot",
            NodeKind::AutomationRun => "automation_run",
            NodeKind::File => "file",
            NodeKind::Pr => "pr",
            NodeKind::Ticket => "ticket",
            NodeKind::Card => "card",
            NodeKind::Person => "person",
            NodeKind::Note => "note",
            NodeKind::Memory => "memory",
        }
    }

    /// Anchors mirror one source row each and are deleted with it. Every
    /// other kind is derived: it lives while an anchor still points at it.
    pub fn is_anchor(self) -> bool {
        matches!(
            self,
            NodeKind::Session
                | NodeKind::Pilot
                | NodeKind::AutomationRun
                | NodeKind::Card
                | NodeKind::Note
                | NodeKind::Memory
        )
    }
}

impl FromStr for NodeKind {
    type Err = GraphError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        NodeKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == s)
            .ok_or_else(|| GraphError::UnknownKind(s.to_string()))
    }
}

/// What a graph edge says about its two nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum EdgeKind {
    InProject,
    OfRepo,
    OnBranch,
    RanIn,
    Touched,
    Read,
    OpenedPr,
    LinksTicket,
    Tracks,
    Blocks,
    Authored,
    Assigned,
    Spawned,
    NotedFrom,
    About,
}

impl EdgeKind {
    pub const ALL: [EdgeKind; 15] = [
        EdgeKind::InProject,
        EdgeKind::OfRepo,
        EdgeKind::OnBranch,
        EdgeKind::RanIn,
        EdgeKind::Touched,
        EdgeKind::Read,
        EdgeKind::OpenedPr,
        EdgeKind::LinksTicket,
        EdgeKind::Tracks,
        EdgeKind::Blocks,
        EdgeKind::Authored,
        EdgeKind::Assigned,
        EdgeKind::Spawned,
        EdgeKind::NotedFrom,
        EdgeKind::About,
    ];

    /// The stored, snake_case name.
    pub fn as_str(self) -> &'static str {
        match self {
            EdgeKind::InProject => "in_project",
            EdgeKind::OfRepo => "of_repo",
            EdgeKind::OnBranch => "on_branch",
            EdgeKind::RanIn => "ran_in",
            EdgeKind::Touched => "touched",
            EdgeKind::Read => "read",
            EdgeKind::OpenedPr => "opened_pr",
            EdgeKind::LinksTicket => "links_ticket",
            EdgeKind::Tracks => "tracks",
            EdgeKind::Blocks => "blocks",
            EdgeKind::Authored => "authored",
            EdgeKind::Assigned => "assigned",
            EdgeKind::Spawned => "spawned",
            EdgeKind::NotedFrom => "noted_from",
            EdgeKind::About => "about",
        }
    }

    /// Whether this edge may run from a `src` node to a `dst` node.
    /// `authored`/`assigned` are reserved for people, and `about` for
    /// memories (#137).
    pub fn allows(self, src: NodeKind, dst: NodeKind) -> bool {
        use NodeKind as N;
        match self {
            EdgeKind::InProject => {
                dst == N::Project
                    && matches!(
                        src,
                        N::Session | N::Worktree | N::Branch | N::File | N::Card | N::Note
                    )
            }
            EdgeKind::OfRepo => dst == N::Repo && matches!(src, N::Pr | N::Ticket),
            EdgeKind::OnBranch => src == N::Session && dst == N::Branch,
            EdgeKind::RanIn => src == N::Session && dst == N::Worktree,
            EdgeKind::Touched | EdgeKind::Read => src == N::Session && dst == N::File,
            EdgeKind::OpenedPr => src == N::Session && dst == N::Pr,
            EdgeKind::LinksTicket => src == N::Session && dst == N::Ticket,
            EdgeKind::Tracks => {
                src == N::Card && matches!(dst, N::Session | N::Pilot | N::Pr | N::Ticket)
            }
            EdgeKind::Blocks => src == N::Card && dst == N::Card,
            EdgeKind::Authored | EdgeKind::Assigned => {
                src == N::Person && matches!(dst, N::Pr | N::Ticket)
            }
            EdgeKind::Spawned => {
                dst == N::Session && matches!(src, N::Pilot | N::Session | N::AutomationRun)
            }
            EdgeKind::NotedFrom => src == N::Note && dst == N::Session,
            EdgeKind::About => src == N::Memory,
        }
    }
}

impl FromStr for EdgeKind {
    type Err = GraphError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        EdgeKind::ALL
            .into_iter()
            .find(|kind| kind.as_str() == s)
            .ok_or_else(|| GraphError::UnknownKind(s.to_string()))
    }
}
