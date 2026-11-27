//! The limits on sessions starting sessions, and the typed refusals every
//! `sessions` tool returns.
use crate::agent::AccessMode;
use std::fmt;

/// How deep the tree goes: a user's chat is depth 0, its children 1, their
/// children 2.
pub const MAX_DEPTH: u8 = 2;
/// The most children of one session that may be live at once.
pub const MAX_LIVE_CHILDREN: usize = 5;

/// Why a `sessions` call was refused. Agents read the code; people read the
/// message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Code {
    DepthLimit,
    FanoutLimit,
    PermissionEscalation,
    NotFound,
    OutOfScope,
    SelfTarget,
    HopLimit,
    InvalidInput,
}

impl Code {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DepthLimit => "depth_limit",
            Self::FanoutLimit => "fanout_limit",
            Self::PermissionEscalation => "permission_escalation",
            Self::NotFound => "not_found",
            Self::OutOfScope => "out_of_scope",
            Self::SelfTarget => "self_target",
            Self::HopLimit => "hop_limit",
            Self::InvalidInput => "invalid_input",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refusal {
    pub code: Code,
    pub message: String,
}

impl Refusal {
    pub fn new(code: Code, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn invalid(message: impl Into<String>) -> Self {
        Self::new(Code::InvalidInput, message)
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code.as_str(), self.message)
    }
}

impl From<Refusal> for String {
    fn from(refusal: Refusal) -> Self {
        refusal.to_string()
    }
}

/// Where a mode sits, narrowest first: read-only, supervised, auto-accept
/// edits, auto, full access.
pub fn rank(mode: AccessMode) -> u8 {
    match mode {
        AccessMode::ReadOnly => 0,
        AccessMode::Supervised => 1,
        AccessMode::AutoAcceptEdits => 2,
        AccessMode::Auto => 3,
        AccessMode::FullAccess => 4,
    }
}

/// The mode a child runs in: its parent's, or a narrower one it asked for.
/// A child never gets more power than the chat that started it.
pub fn child_access(
    parent: AccessMode,
    requested: Option<AccessMode>,
) -> Result<AccessMode, Refusal> {
    match requested {
        None => Ok(parent),
        Some(requested) if rank(requested) <= rank(parent) => Ok(requested),
        Some(requested) => Err(Refusal::new(
            Code::PermissionEscalation,
            format!(
                "this session runs {}, so it can't start one that runs {}",
                parent.id(),
                requested.id()
            ),
        )),
    }
}
