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
    /// The goal and task a Pilot started this run for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<RunTag>,
    /// Edits made to the running definition, oldest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<RunEdit>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunTag {
    pub goal: String,
    pub task: String,
}

/// One edit to a run's Flow snapshot.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEdit {
    pub at: i64,
    pub note: String,
    /// Ids of the states changed or added.
    pub states: Vec<String>,
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

    #[test]
    fn run_tag_and_edits_round_trip_and_default_when_absent() {
        let flow = crate::flow::starter::starter_flows().remove(0);
        let mut run = FlowRun {
            id: "r".into(),
            flow_id: flow.id.clone(),
            flow,
            status: RunStatus::Running,
            inputs: HashMap::new(),
            worktree_cwd: None,
            current_state: None,
            error: None,
            created_at: 1,
            finished_at: None,
            tag: None,
            edits: vec![],
        };
        let plain = serde_json::to_value(&run).unwrap();
        assert!(plain.get("tag").is_none() && plain.get("edits").is_none());
        let back: FlowRun = serde_json::from_value(plain).unwrap();
        assert_eq!(back, run);

        run.tag = Some(RunTag {
            goal: "g".into(),
            task: "t".into(),
        });
        run.edits = vec![RunEdit {
            at: 5,
            note: "n".into(),
            states: vec!["a".into()],
        }];
        let json = serde_json::to_value(&run).unwrap();
        assert_eq!(json["tag"]["goal"], "g");
        assert_eq!(json["edits"][0]["states"][0], "a");
        let back: FlowRun = serde_json::from_value(json).unwrap();
        assert_eq!(back, run);
    }
}
