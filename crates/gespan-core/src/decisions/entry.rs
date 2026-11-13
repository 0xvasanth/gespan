//! The decision log's entry types, and the column mapping `task_decisions`
//! shares with My tasks' saved decisions.
use std::fmt;

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
    pub fn parse(s: &str) -> Result<Kind, Error> {
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
    pub fn parse(s: &str) -> Result<Status, Error> {
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
    pub fn parse(s: &str) -> Result<Scope, Error> {
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

#[derive(Debug, Clone, PartialEq)]
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

/// A fingerprint of what a review card showed of an entry: its title, text,
/// why and user answer. Approving or rejecting a proposal carries the
/// fingerprint the user saw, so a proposal an agent revised in the meantime
/// is refused instead of decided unseen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownHash([u8; 32]);

impl ShownHash {
    /// Each field is length-prefixed (and an absent answer marked apart from
    /// an empty one), so no two different wordings share a fingerprint by
    /// moving text from one field into the next.
    pub fn of(title: &str, text: &str, why: &str, user_answer: Option<&str>) -> ShownHash {
        use sha2::{Digest, Sha256};
        let mut hash = Sha256::new();
        for field in [Some(title), Some(text), Some(why), user_answer] {
            match field {
                Some(value) => {
                    hash.update([1]);
                    hash.update((value.len() as u64).to_le_bytes());
                    hash.update(value.as_bytes());
                }
                None => hash.update([0]),
            }
        }
        ShownHash(hash.finalize().into())
    }
}

#[derive(Debug, Clone, PartialEq)]
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

impl Config {
    /// The app's: Gespan's secret redactor.
    pub fn app() -> Config {
        Config {
            redact: crate::pilot::memory::redact,
        }
    }

    /// What every text field of `task_decisions` is stored as, whoever
    /// writes it (the decision log or My tasks): redacted, then capped at
    /// [`MAX_FIELD_CHARS`] characters (not bytes).
    pub fn clean(&self, text: &str) -> String {
        (self.redact)(text).chars().take(MAX_FIELD_CHARS).collect()
    }
}

/// Everything a decision-log operation can fail with.
#[derive(Debug)]
pub enum Error {
    /// The database refused or failed the operation.
    Db(String),
    /// An entry needs a reason (why).
    MissingWhy,
    NotFound(i64),
    NotPending(i64),
    NotActive(i64),
    SelfReplace(i64),
    FlagNotFound(i64),
    /// A stored kind, status or scope this build does not recognise.
    BadValue(String),
    /// A pending entry's wording is no longer what the user was shown.
    Changed,
    /// The embedder could not turn text into a vector, or its vectors do not
    /// fit the log's vector table.
    Embed(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Db(error) => write!(f, "decision database: {error}"),
            Error::MissingWhy => f.write_str("an entry needs a reason (why)"),
            Error::NotFound(id) => write!(f, "entry {id} does not exist"),
            Error::NotPending(id) => write!(f, "entry {id} is not pending"),
            Error::NotActive(id) => write!(f, "entry {id} is not active"),
            Error::SelfReplace(id) => write!(f, "entry {id} cannot replace itself"),
            Error::FlagNotFound(id) => write!(f, "flag {id} does not exist"),
            Error::BadValue(value) => write!(f, "stored value {value:?} is not recognised"),
            Error::Changed => f.write_str("This proposal changed. Review it again."),
            Error::Embed(error) => write!(f, "embedding: {error}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<String> for Error {
    fn from(error: String) -> Self {
        Error::Db(error)
    }
}

/// A title made from a statement: its first line, cut to this many
/// characters. My tasks' decisions have no title of their own.
pub const DERIVED_TITLE_CHARS: usize = 80;

/// The title My tasks gives a decision it saves or edits: the statement's
/// first line, trimmed of spaces, tabs and carriage returns, and cut to
/// [`DERIVED_TITLE_CHARS`] characters. Migration `0001_decision_log` titles
/// the rows older builds left with the same rule, in SQL.
pub fn derived_title(statement: &str) -> String {
    statement
        .split('\n')
        .next()
        .unwrap_or_default()
        .trim_matches([' ', '\t', '\r'])
        .chars()
        .take(DERIVED_TITLE_CHARS)
        .collect()
}

/// The `project` column of an entry in `scope`, which My tasks groups its
/// Decisions list by: the path of a project scope, otherwise the scope's own
/// text (`repo:<key>` or `global`).
pub fn project_column(scope: &Scope) -> String {
    match scope {
        Scope::Project(path) => path.clone(),
        other => other.to_string(),
    }
}
