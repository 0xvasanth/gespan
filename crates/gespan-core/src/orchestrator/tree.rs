//! Which session started which, and the pure decisions made on that tree:
//! whether a start is allowed, what a cancel stops, whether a stop wakes the
//! parent. The app applies the [`Effect`]s; nothing here touches a session.
use super::{
    policy::{child_access, Code, Refusal, MAX_DEPTH, MAX_LIVE_CHILDREN},
    start::StartInput,
};
use crate::agent::AccessMode;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

/// What started a child.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    Chat,
    Pilot,
    Orchestration,
    Flow,
}

impl Origin {
    pub fn id(self) -> &'static str {
        match self {
            Self::Chat => "chat",
            Self::Pilot => "pilot",
            Self::Orchestration => "orchestration",
            Self::Flow => "flow",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Chat, Self::Pilot, Self::Orchestration, Self::Flow]
            .into_iter()
            .find(|origin| origin.id() == id)
    }
}

/// When a child's stop is reported to its parent.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Wake {
    /// Every stop, including waiting on the user and being cancelled.
    Always,
    /// When the child finished or failed.
    Settled,
    Never,
}

impl Wake {
    pub fn id(self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Settled => "settled",
            Self::Never => "never",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Always, Self::Settled, Self::Never]
            .into_iter()
            .find(|wake| wake.id() == id)
    }
}

/// One child and who started it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub child: i64,
    pub parent: i64,
    pub root: i64,
    pub depth: u8,
    pub origin: Origin,
    pub wake: Wake,
}

/// How a child's turn ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    Finished,
    Failed,
    Cancelled,
    WaitingOnUser,
}

impl Outcome {
    pub fn describe(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::Failed => "failed",
            Self::Cancelled => "was stopped",
            Self::WaitingOnUser => "is waiting on the user",
        }
    }
}

/// Every link, keyed by child.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tree {
    links: BTreeMap<i64, Link>,
}

/// What [`Tree::remove`] changed, for the store to write.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Removal {
    /// Children whose link is gone: the deleted session's own, and each of
    /// its children, now roots.
    pub removed: Vec<i64>,
    /// Links below those children, re-rooted.
    pub updated: Vec<Link>,
}

impl Tree {
    pub fn new(links: impl IntoIterator<Item = Link>) -> Self {
        Self {
            links: links.into_iter().map(|link| (link.child, link)).collect(),
        }
    }

    pub fn link(&self, child: i64) -> Option<&Link> {
        self.links.get(&child)
    }

    pub fn links(&self) -> impl Iterator<Item = &Link> {
        self.links.values()
    }

    pub fn insert(&mut self, link: Link) {
        self.links.insert(link.child, link);
    }

    pub fn parent(&self, id: i64) -> Option<i64> {
        self.link(id).map(|link| link.parent)
    }

    /// 0 for a session nobody started.
    pub fn depth(&self, id: i64) -> u8 {
        self.link(id).map_or(0, |link| link.depth)
    }

    pub fn root(&self, id: i64) -> i64 {
        self.link(id).map_or(id, |link| link.root)
    }

    /// Direct children, in id order.
    pub fn children(&self, id: i64) -> Vec<i64> {
        self.links
            .values()
            .filter(|link| link.parent == id)
            .map(|link| link.child)
            .collect()
    }

    /// Every session below `id`, each after all of its own descendants, so
    /// stopping in this order never leaves a child running without a parent.
    pub fn descendants(&self, id: i64) -> Vec<i64> {
        let mut order = Vec::new();
        let mut seen = HashSet::from([id]);
        self.post_order(id, &mut seen, &mut order);
        order
    }

    fn post_order(&self, id: i64, seen: &mut HashSet<i64>, order: &mut Vec<i64>) {
        for child in self.children(id) {
            // A corrupt cycle must not loop forever.
            if seen.insert(child) {
                self.post_order(child, seen, order);
                order.push(child);
            }
        }
    }

    pub fn is_descendant(&self, id: i64, of: i64) -> bool {
        let mut at = id;
        for _ in 0..=self.links.len() {
            match self.parent(at) {
                Some(parent) if parent == of => return true,
                Some(parent) => at = parent,
                None => return false,
            }
        }
        false
    }

    /// Drops a deleted session: its own link goes, its children become roots
    /// and everything below them is re-rooted one family down.
    pub fn remove(&mut self, deleted: i64) -> Removal {
        let mut removal = Removal::default();
        if self.links.remove(&deleted).is_some() {
            removal.removed.push(deleted);
        }
        for child in self.children(deleted) {
            let Some(old) = self.links.remove(&child) else {
                continue;
            };
            removal.removed.push(child);
            for below in self.descendants(child) {
                if let Some(link) = self.links.get_mut(&below) {
                    link.root = child;
                    link.depth = link.depth.saturating_sub(old.depth);
                    removal.updated.push(*link);
                }
            }
        }
        removal.updated.sort_by_key(|link| link.child);
        removal
    }
}

/// What decisions are made from.
pub struct State<'a> {
    pub tree: &'a Tree,
    /// Sessions whose agent is working, waiting on the user or about to
    /// start. Only these count toward [`MAX_LIVE_CHILDREN`].
    pub live: &'a HashSet<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    Start {
        caller: i64,
        /// The mode the caller's turn runs in.
        caller_access: AccessMode,
        /// The caller's starts not yet answered, which will be live.
        starting: usize,
        input: Box<StartInput>,
    },
    Cancel {
        caller: i64,
        target: i64,
    },
    /// A child's turn ended, or it is waiting on the user.
    Settled {
        session: i64,
        outcome: Outcome,
    },
}

/// A child allowed to start, with where it sits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Launch {
    pub parent: i64,
    pub root: i64,
    pub depth: u8,
    pub access: AccessMode,
    pub wake: Wake,
    pub input: StartInput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    Launch(Box<Launch>),
    /// Stop these sessions, in this order.
    Stop(Vec<i64>),
    /// Tell `parent` how `child`'s turn went.
    Report {
        parent: i64,
        child: i64,
        outcome: Outcome,
    },
}

pub fn decide(state: &State, command: Command) -> Result<Vec<Effect>, Refusal> {
    match command {
        Command::Start {
            caller,
            caller_access,
            starting,
            input,
        } => {
            let depth = state.tree.depth(caller) + 1;
            if depth > MAX_DEPTH {
                return Err(Refusal::new(
                    Code::DepthLimit,
                    format!("sessions started {MAX_DEPTH} levels below a chat can't start more"),
                ));
            }
            let live = state
                .tree
                .children(caller)
                .into_iter()
                .filter(|child| state.live.contains(child))
                .count()
                + starting;
            if live >= MAX_LIVE_CHILDREN {
                return Err(Refusal::new(
                    Code::FanoutLimit,
                    format!(
                        "{MAX_LIVE_CHILDREN} sessions this one started are still running; wait for one to finish or cancel one"
                    ),
                ));
            }
            let access = child_access(caller_access, input.access)?;
            Ok(vec![Effect::Launch(Box::new(Launch {
                parent: caller,
                root: state.tree.root(caller),
                depth,
                access,
                wake: input.wake,
                input: *input,
            }))])
        }
        Command::Cancel { caller, target } => {
            if caller == target {
                return Err(Refusal::new(
                    Code::SelfTarget,
                    "a session can't cancel itself; finish the turn instead",
                ));
            }
            if !state.tree.is_descendant(target, caller) {
                return Err(Refusal::new(
                    Code::OutOfScope,
                    format!("session {target} wasn't started by this session or one it started"),
                ));
            }
            let mut order = state.tree.descendants(target);
            order.push(target);
            Ok(vec![Effect::Stop(order)])
        }
        Command::Settled { session, outcome } => {
            let Some(link) = state.tree.link(session) else {
                return Ok(Vec::new());
            };
            let wakes = match link.wake {
                Wake::Always => true,
                Wake::Settled => matches!(outcome, Outcome::Finished | Outcome::Failed),
                Wake::Never => false,
            };
            Ok(if wakes {
                vec![Effect::Report {
                    parent: link.parent,
                    child: session,
                    outcome,
                }]
            } else {
                Vec::new()
            })
        }
    }
}
