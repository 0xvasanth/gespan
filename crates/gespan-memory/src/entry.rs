use std::fmt;

use crate::Error;

/// Each of title, text, why and user_answer is capped at this many characters.
pub const MAX_FIELD_CHARS: usize = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Preference,
    Decision,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Preference => "preference",
            Kind::Decision => "decision",
        }
    }
    pub(crate) fn parse(s: &str) -> Result<Kind, Error> {
        match s {
            "preference" => Ok(Kind::Preference),
            "decision" => Ok(Kind::Decision),
            other => Err(Error::BadValue(other.to_string())),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Pending,
    Active,
    Replaced,
    Rejected,
    Archived,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Pending => "pending",
            Status::Active => "active",
            Status::Replaced => "replaced",
            Status::Rejected => "rejected",
            Status::Archived => "archived",
        }
    }
    pub(crate) fn parse(s: &str) -> Result<Status, Error> {
        match s {
            "pending" => Ok(Status::Pending),
            "active" => Ok(Status::Active),
            "replaced" => Ok(Status::Replaced),
            "rejected" => Ok(Status::Rejected),
            "archived" => Ok(Status::Archived),
            other => Err(Error::BadValue(other.to_string())),
        }
    }
}

/// Where an entry applies. Stored as its `Display` text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    Global,
    Repo(String),
    Project(String),
}

impl fmt::Display for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Scope::Global => f.write_str("global"),
            Scope::Repo(r) => write!(f, "repo:{r}"),
            Scope::Project(p) => write!(f, "project:{p}"),
        }
    }
}

impl Scope {
    pub(crate) fn parse(s: &str) -> Result<Scope, Error> {
        if s == "global" {
            Ok(Scope::Global)
        } else if let Some(r) = s.strip_prefix("repo:") {
            Ok(Scope::Repo(r.to_string()))
        } else if let Some(p) = s.strip_prefix("project:") {
            Ok(Scope::Project(p.to_string()))
        } else {
            Err(Error::BadValue(s.to_string()))
        }
    }
}

/// Who or what an entry came from. An entry carries a "user source" when the
/// user said it, which is `question_ref` or `user_turn`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Source {
    pub session: Option<i64>,
    pub question_ref: Option<String>,
    pub user_turn: Option<String>,
    pub import: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewEntry {
    pub kind: Kind,
    pub scope: Scope,
    pub title: String,
    pub text: String,
    pub why: String,
    pub user_answer: Option<String>,
    pub source: Source,
}

#[derive(Debug, Clone)]
pub struct Edit {
    pub title: String,
    pub text: String,
    pub why: String,
}

#[derive(Debug, Clone)]
pub struct Entry {
    pub id: i64,
    pub kind: Kind,
    pub scope: Scope,
    pub status: Status,
    pub title: String,
    pub text: String,
    pub why: String,
    pub user_answer: Option<String>,
    pub source: Source,
    pub pinned: bool,
    pub supersedes: Option<i64>,
    pub replaced_by: Option<i64>,
    pub created_ms: i64,
    pub decided_ms: Option<i64>,
    pub open_flags: u32,
}

impl Entry {
    /// The id agents and users see: `D-14` for a decision, `P-3` for a
    /// preference.
    pub fn label(&self) -> String {
        match self.kind {
            Kind::Decision => format!("D-{}", self.id),
            Kind::Preference => format!("P-{}", self.id),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Flag {
    pub id: i64,
    pub entry: i64,
    pub session: i64,
    pub reason: String,
    pub created_ms: i64,
}

/// Store-wide behaviour the caller injects.
#[derive(Clone, Copy)]
pub struct Config {
    /// Scrubs secrets from text before it is stored.
    pub redact: fn(&str) -> String,
}
