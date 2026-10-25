//! Links from sessions and board cards to tickets in an external tracker.
mod parse;
mod service;

use serde::{Deserialize, Serialize};

use crate::provider_boards::{ProviderKind, StateGroup};

pub use parse::{parse_ticket, ParsedTicket};
pub use service::{Resolved, StatusChoice, TicketError, Tickets};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tracker {
    Jira,
    Linear,
    Plane,
}

impl Tracker {
    pub fn as_str(self) -> &'static str {
        self.provider_kind().as_str()
    }

    pub fn label(self) -> &'static str {
        self.provider_kind().label()
    }

    pub fn provider_kind(self) -> ProviderKind {
        match self {
            Self::Jira => ProviderKind::Jira,
            Self::Linear => ProviderKind::Linear,
            Self::Plane => ProviderKind::Plane,
        }
    }
}

/// A linked ticket plus the last fetched snapshot of it. Everything but the
/// tracker and key is a cache that a refresh may overwrite.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketRef {
    pub tracker: Tracker,
    pub key: String,
    pub url: Option<String>,
    pub title: Option<String>,
    pub status: Option<String>,
    pub status_group: Option<StateGroup>,
    /// Unix milliseconds of the last successful fetch.
    pub fetched_at: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracker_maps_to_provider_kind() {
        for (tracker, name, label) in [
            (Tracker::Jira, "jira", "Jira"),
            (Tracker::Linear, "linear", "Linear"),
            (Tracker::Plane, "plane", "Plane"),
        ] {
            assert_eq!(tracker.as_str(), name);
            assert_eq!(tracker.label(), label);
            assert_eq!(tracker.provider_kind().as_str(), name);
        }
    }

    #[test]
    fn ticket_ref_serializes_camel_case_and_round_trips() {
        let ticket = TicketRef {
            tracker: Tracker::Plane,
            key: "WEB-3".into(),
            url: None,
            title: Some("Fix".into()),
            status: Some("In Progress".into()),
            status_group: Some(StateGroup::Started),
            fetched_at: Some(1_700_000_000_000),
        };
        let json = serde_json::to_value(&ticket).unwrap();
        assert_eq!(json["tracker"], "plane");
        assert_eq!(json["statusGroup"], "started");
        assert_eq!(json["fetchedAt"], 1_700_000_000_000_i64);
        assert_eq!(serde_json::from_value::<TicketRef>(json).unwrap(), ticket);
    }
}
