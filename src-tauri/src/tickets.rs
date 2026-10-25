//! Ticket links for sessions: resolve what the user typed, refresh a linked
//! ticket, and the Plane connection. The logic lives in `gespan-tracker-client`;
//! these commands only run it off the main thread (credentials come from the
//! keychain, and fetches go over the network).
use std::path::PathBuf;

use gespan_tracker_client::credentials::plane::{self, PlaneStatus};
use gespan_tracker_client::tickets::{parse_ticket, Resolved, TicketRef, Tickets, Tracker};
use serde::Serialize;
use tauri::{AppHandle, Manager};

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ResolveResult {
    One {
        ticket: TicketRef,
    },
    /// A bare key that more than one connected tracker could own; the caller
    /// picks one and builds the ref from `key`.
    Ambiguous {
        key: String,
        trackers: Vec<Tracker>,
    },
    Error {
        message: String,
    },
}

impl ResolveResult {
    fn new(input: &str, resolved: Resolved) -> Self {
        match resolved {
            Resolved::One(ticket) => Self::One { ticket },
            Resolved::Ambiguous(trackers) => Self::Ambiguous {
                key: parse_ticket(input).map(|p| p.key).unwrap_or_default(),
                trackers,
            },
            other => Self::Error {
                message: other
                    .message()
                    .unwrap_or_else(|| "Not a ticket key or link".into()),
            },
        }
    }
}

fn resolve_with(tickets: &Tickets, input: &str) -> ResolveResult {
    ResolveResult::new(input, tickets.resolve(input))
}

fn fetch_with(tickets: &Tickets, ticket: &TicketRef) -> Result<TicketRef, String> {
    tickets.fetch(ticket).map_err(|error| error.to_string())
}

fn data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    app.path().app_data_dir().map_err(|error| error.to_string())
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
}

#[tauri::command]
pub async fn ticket_resolve(app: AppHandle, input: String) -> Result<ResolveResult, String> {
    let dir = data_dir(&app)?;
    blocking(move || Ok(resolve_with(&Tickets::new(dir), &input))).await
}

#[tauri::command]
pub async fn ticket_fetch(app: AppHandle, ticket: TicketRef) -> Result<TicketRef, String> {
    let dir = data_dir(&app)?;
    blocking(move || fetch_with(&Tickets::new(dir), &ticket)).await
}

/// Trackers with saved credentials, for the stale marker on ticket chips.
#[tauri::command]
pub async fn ticket_connected(app: AppHandle) -> Result<Vec<Tracker>, String> {
    let dir = data_dir(&app)?;
    blocking(move || Ok(Tickets::new(dir).connected())).await
}

#[tauri::command]
pub async fn plane_status(app: AppHandle) -> Result<PlaneStatus, String> {
    let dir = data_dir(&app)?;
    blocking(move || plane::plane_status(dir)).await
}

/// An empty key disconnects.
#[tauri::command]
pub async fn plane_set_config(
    app: AppHandle,
    base_url: String,
    workspace: String,
    api_key: String,
) -> Result<PlaneStatus, String> {
    let dir = data_dir(&app)?;
    blocking(move || plane::plane_set_config(dir, base_url, workspace, api_key)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_tracker_client::provider_boards::fake::FakeProvider;
    use gespan_tracker_client::provider_boards::{
        BoardProvider, RemoteIssue, StateGroup, WorkflowState,
    };
    use gespan_tracker_client::tickets::Connection;
    use serde_json::json;
    use std::sync::Arc;

    struct Conn(FakeProvider);

    impl Connection for Conn {
        fn board(&self) -> &dyn BoardProvider {
            &self.0
        }
    }

    fn issue(key: &str) -> RemoteIssue {
        RemoteIssue {
            id: key.into(),
            key: key.into(),
            title: "Fix login".into(),
            url: format!("https://tracker.test/{key}"),
            container_id: "C1".into(),
            updated_at: "2026-03-01T10:00:00Z".into(),
            state: WorkflowState {
                id: "todo".into(),
                name: "Todo".into(),
                group: StateGroup::Started,
            },
            ..Default::default()
        }
    }

    fn tickets(trackers: &[Tracker], fake: impl Fn() -> FakeProvider) -> Tickets {
        gespan_tracker_secrets::testing::use_memory_keychain();
        let providers = trackers
            .iter()
            .map(|tracker| (*tracker, Arc::new(Conn(fake())) as Arc<dyn Connection>))
            .collect();
        Tickets::with_providers(PathBuf::from("/unused"), providers)
    }

    #[test]
    fn a_bare_key_with_two_trackers_connected_asks_which_one() {
        let t = tickets(&[Tracker::Jira, Tracker::Linear], FakeProvider::new);
        let result = resolve_with(&t, "ENG-12");
        assert_eq!(
            result,
            ResolveResult::Ambiguous {
                key: "ENG-12".into(),
                trackers: vec![Tracker::Jira, Tracker::Linear]
            }
        );
        assert_eq!(
            serde_json::to_value(&result).unwrap(),
            json!({ "kind": "ambiguous", "key": "ENG-12", "trackers": ["jira", "linear"] })
        );
    }

    #[test]
    fn a_linear_url_resolves_to_one_ticket_without_a_fetch() {
        let t = tickets(&[Tracker::Linear], FakeProvider::new);
        let result = resolve_with(&t, "https://linear.app/acme/issue/ENG-7/fix-login");
        let ResolveResult::One { ticket } = &result else {
            panic!("expected one ticket, got {result:?}");
        };
        assert_eq!(
            (ticket.tracker, ticket.key.as_str()),
            (Tracker::Linear, "ENG-7")
        );
        assert_eq!(ticket.title, None);
        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(value["kind"], "one");
        assert_eq!(value["ticket"]["key"], "ENG-7");
    }

    #[test]
    fn a_url_for_a_tracker_that_is_not_connected_is_an_error_message() {
        let t = tickets(&[Tracker::Jira], FakeProvider::new);
        let result = resolve_with(&t, "https://linear.app/acme/issue/ENG-7/fix-login");
        assert_eq!(
            result,
            ResolveResult::Error {
                message: "Connect Linear in Settings".into()
            }
        );
    }

    #[test]
    fn garbage_and_a_bare_key_with_nothing_connected_are_errors() {
        let t = tickets(&[], FakeProvider::new);
        for input in ["hello there", "ENG-12", ""] {
            assert_eq!(
                resolve_with(&t, input),
                ResolveResult::Error {
                    message: "Not a ticket key or link".into()
                },
                "{input:?}"
            );
        }
    }

    #[test]
    fn fetch_fills_the_snapshot_and_errors_are_user_facing_text() {
        let t = tickets(&[Tracker::Linear], || {
            FakeProvider::new().with_issue(issue("ENG-7"))
        });
        let linked = TicketRef {
            tracker: Tracker::Linear,
            key: "ENG-7".into(),
            url: None,
            title: None,
            status: None,
            status_group: None,
            fetched_at: None,
        };
        let filled = fetch_with(&t, &linked).unwrap();
        assert_eq!(filled.title.as_deref(), Some("Fix login"));
        assert_eq!(filled.status.as_deref(), Some("Todo"));
        assert_eq!(filled.status_group, Some(StateGroup::Started));
        assert!(filled.fetched_at.is_some());

        let jira = TicketRef {
            tracker: Tracker::Jira,
            ..linked
        };
        assert_eq!(
            fetch_with(&t, &jira).unwrap_err(),
            "Connect Jira in Settings"
        );
    }
}
