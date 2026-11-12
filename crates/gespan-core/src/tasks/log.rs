//! What My tasks keeps: every request that waited on the user, with the
//! user's answer, and the decisions saved from those answers.
use super::Kind;
use serde_json::Value;

impl Kind {
    /// The kind's name in the database.
    pub fn id(self) -> &'static str {
        match self {
            Self::Question => "question",
            Self::Approval => "approval",
            Self::FlowApproval => "flow-approval",
            Self::PilotChoice => "pilot-choice",
            Self::DraftReview => "draft-review",
            Self::Decision => "decision",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.id() == id)
    }
}

/// Where a request stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Still waiting on the user.
    Open,
    Answered,
    /// The agent stopped waiting before anyone answered: its turn was
    /// stopped, or Gespan quit.
    Withdrawn,
}

impl Status {
    pub fn id(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Answered => "answered",
            Self::Withdrawn => "withdrawn",
        }
    }
    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Open, Self::Answered, Self::Withdrawn]
            .into_iter()
            .find(|status| status.id() == id)
    }
}

/// One request and, once given, the user's answer.
#[derive(Clone, Debug, PartialEq)]
pub struct TaskRecord {
    /// The same key the live list uses, such as `approval:<session>:<id>`.
    pub key: String,
    pub kind: Kind,
    pub project: String,
    /// What opens it again: `session:<id>`, `pilot:<pilot>:<session>` or
    /// `flow:<run>`.
    pub origin: String,
    /// The session, chat or run, as the list names it.
    pub source_label: String,
    pub headline: String,
    /// The whole request: the tool and its input, the questions with every
    /// option, the step's prompt, or the choices offered.
    pub request: Value,
    pub asked_at: u64,
    pub status: Status,
    /// The answer as the user reads it, such as "Allowed once".
    pub answer: Option<String>,
    /// The answer as sent: the decision, the options picked per question,
    /// the note.
    pub answer_value: Option<Value>,
    /// When it was answered or withdrawn.
    pub closed_at: Option<u64>,
    /// The decision saved from this answer.
    pub decision: Option<i64>,
}

impl TaskRecord {
    /// A request still waiting. Each field is a different part of the
    /// request, so they are named at the call rather than bundled.
    #[allow(clippy::too_many_arguments)]
    pub fn open(
        key: impl Into<String>,
        kind: Kind,
        project: impl Into<String>,
        origin: impl Into<String>,
        source_label: impl Into<String>,
        headline: impl Into<String>,
        request: Value,
        asked_at: u64,
    ) -> Self {
        Self {
            key: key.into(),
            kind,
            project: project.into(),
            origin: origin.into(),
            source_label: source_label.into(),
            headline: headline.into(),
            request,
            asked_at,
            status: Status::Open,
            answer: None,
            answer_value: None,
            closed_at: None,
            decision: None,
        }
    }

    /// This request, answered with `answer` (readable) and `value` (as sent).
    pub fn answered(mut self, answer: impl Into<String>, value: Value, at: u64) -> Self {
        self.status = Status::Answered;
        self.answer = Some(answer.into());
        self.answer_value = Some(value);
        self.closed_at = Some(at);
        self
    }

    /// Whether this answer can become a project decision: an answer to a
    /// question or a choice, not yet saved as one.
    pub fn can_remember(&self) -> bool {
        self.status == Status::Answered
            && self.decision.is_none()
            && matches!(self.kind, Kind::Question | Kind::PilotChoice)
    }

    /// The decision this answer suggests, for the user to edit before saving.
    pub fn suggested_statement(&self) -> String {
        let answer = self.answer.as_deref().unwrap_or_default().trim();
        let asked = self.headline.trim().trim_end_matches(['?', ':']).trim();
        match (asked.is_empty(), answer.is_empty()) {
            (_, true) => asked.to_owned(),
            (true, false) => answer.to_owned(),
            (false, false) => format!("{asked}: {answer}"),
        }
    }
}

/// A project decision the user saved from one or more answers, or wrote.
#[derive(Clone, Debug, PartialEq)]
pub struct SavedDecision {
    pub id: i64,
    pub project: String,
    pub statement: String,
    pub why: String,
    pub created_at: u64,
    pub updated_at: u64,
    /// The answers it came from, newest first: "How you got here".
    pub sources: Vec<TaskRecord>,
    /// A preference or a project decision.
    pub kind: crate::decisions::Kind,
    /// Where it applies: a preference is global; a decision is keyed by its
    /// repository or project.
    pub scope: crate::decisions::Scope,
    /// The session whose agent proposed it, for one approved from the
    /// decision log.
    pub session: Option<i64>,
}
