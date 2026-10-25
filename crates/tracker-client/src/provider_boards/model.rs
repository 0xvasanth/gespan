//! Provider-neutral shapes shared by every external board.
use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ProviderKind {
    Linear,
    Jira,
    Plane,
    #[cfg(any(test, feature = "testing"))]
    #[doc(hidden)]
    Fake,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Linear => "linear",
            Self::Jira => "jira",
            Self::Plane => "plane",
            #[cfg(any(test, feature = "testing"))]
            Self::Fake => "fake",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Jira => "Jira",
            Self::Plane => "Plane",
            #[cfg(any(test, feature = "testing"))]
            Self::Fake => "Fake",
        }
    }
}

/// The fixed bucket a tracker's own workflow state belongs to. Status mapping
/// between a Gespan board and a tracker is keyed on this, not on state names.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StateGroup {
    Triage,
    Backlog,
    Unstarted,
    Started,
    Completed,
    Cancelled,
    #[default]
    Unknown,
}

impl StateGroup {
    /// Linear `WorkflowState.type`.
    pub fn from_linear_type(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "triage" => Self::Triage,
            "backlog" => Self::Backlog,
            "unstarted" => Self::Unstarted,
            "started" => Self::Started,
            "completed" => Self::Completed,
            "canceled" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }

    /// Jira `statusCategory.key`. Jira has no cancelled category.
    pub fn from_jira_category(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "new" => Self::Unstarted,
            "indeterminate" => Self::Started,
            "done" => Self::Completed,
            _ => Self::Unknown,
        }
    }

    /// Plane `state.group`.
    pub fn from_plane_group(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "triage" => Self::Triage,
            "backlog" => Self::Backlog,
            "unstarted" => Self::Unstarted,
            "started" => Self::Started,
            "completed" => Self::Completed,
            "cancelled" => Self::Cancelled,
            _ => Self::Unknown,
        }
    }

    pub fn is_closed(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub name: String,
    pub email: String,
}

/// A Linear team, a Jira project, or (later) a Plane project.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Container {
    pub id: String,
    pub key: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct WorkflowState {
    pub id: String,
    pub name: String,
    pub group: StateGroup,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct BoardLabel {
    pub id: String,
    pub name: String,
    pub color: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Member {
    pub id: String,
    pub name: String,
    pub avatar_url: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IssueFilter {
    /// Empty means every container the credentials can see.
    pub container_ids: Vec<String>,
    pub assigned_to_me: bool,
    pub include_closed: bool,
    pub limit: u32,
}

impl Default for IssueFilter {
    fn default() -> Self {
        Self {
            container_ids: Vec::new(),
            assigned_to_me: false,
            include_closed: false,
            limit: 50,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Opaque; pass back to get the next page. `None` on the last page.
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RemoteIssue {
    pub id: String,
    /// Human key such as `ENG-42`.
    pub key: String,
    pub title: String,
    pub url: String,
    pub container_id: String,
    pub updated_at: String,
    pub state: WorkflowState,
    pub labels: Vec<BoardLabel>,
    pub assignees: Vec<Member>,
    /// Markdown. `None` when the call that produced this issue doesn't fetch it.
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IssueDraft {
    pub title: String,
    pub description: String,
    pub state_id: Option<String>,
    pub label_ids: Vec<String>,
    pub assignee_ids: Vec<String>,
}

/// Fields left `None` are not changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct IssuePatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub label_ids: Option<Vec<String>>,
    pub assignee_ids: Option<Vec<String>>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MoveTarget {
    pub state_id: String,
    /// Fields some trackers demand for a move (Jira transition screens).
    pub required_fields: BTreeMap<String, serde_json::Value>,
}

/// A Jira workflow move: the status it leads to and the fields its screen demands.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    pub id: String,
    pub name: String,
    pub to: WorkflowState,
    pub required_fields: Vec<RequiredField>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RequiredField {
    pub key: String,
    pub name: String,
    pub schema_type: String,
    pub allowed: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommentRef {
    pub id: String,
    pub url: String,
}

/// Opaque position in a provider's change feed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCursor(pub String);

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncBatch {
    pub changes: Vec<RemoteIssue>,
    pub next_cursor: SyncCursor,
    pub has_more: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeEvent {
    Created,
    Updated,
    Deleted,
}

/// What a webhook says changed. The receiver refetches the entity itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeHint {
    pub entity: String,
    pub id: String,
    pub event: ChangeEvent,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProviderError {
    /// The provider has no implementation of this operation yet.
    NotSupported {
        provider: ProviderKind,
        operation: &'static str,
    },
    /// Rejected before any request was sent.
    InvalidInput(String),
    /// The tracker, the network or the credentials failed.
    Remote(String),
}

impl fmt::Display for ProviderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSupported {
                provider,
                operation,
            } => write!(f, "{} does not support {operation} yet", provider.label()),
            Self::InvalidInput(message) | Self::Remote(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for ProviderError {}

impl From<String> for ProviderError {
    fn from(message: String) -> Self {
        Self::Remote(message)
    }
}

pub type ProviderResult<T> = Result<T, ProviderError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_state_types_map_to_groups() {
        let cases = [
            ("triage", StateGroup::Triage),
            ("backlog", StateGroup::Backlog),
            ("unstarted", StateGroup::Unstarted),
            ("started", StateGroup::Started),
            ("completed", StateGroup::Completed),
            ("canceled", StateGroup::Cancelled),
            ("Started", StateGroup::Started),
            ("bogus", StateGroup::Unknown),
            ("", StateGroup::Unknown),
        ];
        for (raw, group) in cases {
            assert_eq!(StateGroup::from_linear_type(raw), group, "{raw}");
        }
    }

    #[test]
    fn jira_categories_map_to_groups() {
        let cases = [
            ("new", StateGroup::Unstarted),
            ("indeterminate", StateGroup::Started),
            ("done", StateGroup::Completed),
            ("DONE", StateGroup::Completed),
            ("undefined", StateGroup::Unknown),
        ];
        for (raw, group) in cases {
            assert_eq!(StateGroup::from_jira_category(raw), group, "{raw}");
        }
    }

    #[test]
    fn plane_groups_map_to_groups() {
        let cases = [
            ("triage", StateGroup::Triage),
            ("backlog", StateGroup::Backlog),
            ("unstarted", StateGroup::Unstarted),
            ("started", StateGroup::Started),
            ("completed", StateGroup::Completed),
            ("cancelled", StateGroup::Cancelled),
            ("Started", StateGroup::Started),
            ("canceled", StateGroup::Unknown),
            ("", StateGroup::Unknown),
        ];
        for (raw, group) in cases {
            assert_eq!(StateGroup::from_plane_group(raw), group, "{raw}");
        }
    }

    #[test]
    fn closed_groups_are_completed_and_cancelled_only() {
        assert!(StateGroup::Completed.is_closed());
        assert!(StateGroup::Cancelled.is_closed());
        assert!(!StateGroup::Started.is_closed());
        assert!(!StateGroup::Unknown.is_closed());
    }

    #[test]
    fn not_supported_error_names_provider_and_operation() {
        let error = ProviderError::NotSupported {
            provider: ProviderKind::Linear,
            operation: "create_issue",
        };
        assert_eq!(
            error.to_string(),
            "Linear does not support create_issue yet"
        );
        let error = ProviderError::NotSupported {
            provider: ProviderKind::Jira,
            operation: "sync_since",
        };
        assert_eq!(error.to_string(), "Jira does not support sync_since yet");
    }

    #[test]
    fn string_errors_become_remote() {
        let error = ProviderError::from("Linear API key is invalid".to_string());
        assert_eq!(
            error,
            ProviderError::Remote("Linear API key is invalid".into())
        );
        assert_eq!(error.to_string(), "Linear API key is invalid");
        assert_eq!(ProviderError::InvalidInput("bad".into()).to_string(), "bad");
    }

    #[test]
    fn remote_issue_serializes_in_camel_case() {
        let issue = RemoteIssue {
            id: "1".into(),
            container_id: "c".into(),
            updated_at: "2026-01-01T00:00:00Z".into(),
            ..Default::default()
        };
        let value = serde_json::to_value(&issue).unwrap();
        assert_eq!(value["containerId"], "c");
        assert_eq!(value["updatedAt"], "2026-01-01T00:00:00Z");
        assert_eq!(value["state"]["group"], "unknown");
    }
}
