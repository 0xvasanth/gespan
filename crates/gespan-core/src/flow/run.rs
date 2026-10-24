//! Flow run and visit records, shared by storage and the runner.
use super::definition::{Flow, OutputValue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunStatus {
    Running,
    WaitingApproval,
    Succeeded,
    Failed,
    Cancelled,
}

impl RunStatus {
    /// The string stored in the `status` column (same as the serde form).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingApproval => "waiting-approval",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
    /// A run that still holds its Flow: running or waiting for approval.
    pub fn is_active(self) -> bool {
        matches!(self, Self::Running | Self::WaitingApproval)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VisitStatus {
    Running,
    WaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

impl VisitStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::WaitingApproval => "waiting-approval",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowRun {
    pub id: String,
    pub flow_id: String,
    /// Definition snapshot taken when the run started.
    pub flow: Flow,
    pub status: RunStatus,
    pub inputs: HashMap<String, String>,
    pub worktree_cwd: Option<String>,
    pub current_state: Option<String>,
    pub error: Option<String>,
    pub created_at: i64,
    pub finished_at: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowVisit {
    pub id: String,
    pub run_id: String,
    pub seq: u32,
    pub state_id: String,
    pub status: VisitStatus,
    pub prompt: Option<String>,
    pub session_id: Option<String>,
    pub outputs: Option<HashMap<String, OutputValue>>,
    pub error: Option<String>,
    pub retried: bool,
    pub note: Option<String>,
    pub started_at: i64,
    pub finished_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_strings_match_serde_and_column() {
        for s in [
            RunStatus::Running,
            RunStatus::WaitingApproval,
            RunStatus::Succeeded,
            RunStatus::Failed,
            RunStatus::Cancelled,
        ] {
            assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
        }
        for s in [
            VisitStatus::Running,
            VisitStatus::WaitingApproval,
            VisitStatus::Completed,
            VisitStatus::Failed,
            VisitStatus::Cancelled,
        ] {
            assert_eq!(serde_json::to_value(s).unwrap(), s.as_str());
        }
        assert!(RunStatus::WaitingApproval.is_active() && !RunStatus::Failed.is_active());
    }
}
