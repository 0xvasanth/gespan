//! What the UI and the agent tools call to turn a pasted key or URL into a
//! ticket, read it, comment on it and change its status. Nothing here polls:
//! every call is one request to the tracker, and `resolve` makes none.
use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use super::{parse_ticket, TicketRef, Tracker};
use crate::inbox;
use crate::provider_boards::jira::JiraProvider;
use crate::provider_boards::linear::LinearProvider;
use crate::provider_boards::plane::PlaneProvider;
use crate::provider_boards::{
    BoardProvider, MoveTarget, ProviderError, ProviderResult, RemoteIssue, RequiredField,
    StateGroup, Transition,
};

const ALL: [Tracker; 3] = [Tracker::Jira, Tracker::Linear, Tracker::Plane];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resolved {
    One(TicketRef),
    Ambiguous(Vec<Tracker>),
    NotConnected(Tracker),
    /// The pasted URL names another site (or Plane workspace) than the saved
    /// connection, so a same-keyed ticket there would be a different ticket.
    OtherSite {
        tracker: Tracker,
        connected: String,
        pasted: String,
    },
    Invalid,
}

impl Resolved {
    /// What to tell the user when the input did not resolve to one ticket.
    pub fn message(&self) -> Option<String> {
        match self {
            Self::NotConnected(tracker) => Some(TicketError::NotConnected(*tracker).to_string()),
            Self::OtherSite {
                tracker,
                connected,
                pasted,
            } => Some(format!(
                "{} is connected to {connected}, not {pasted}",
                tracker.label()
            )),
            Self::Invalid => Some("Not a ticket key or link".into()),
            Self::One(_) | Self::Ambiguous(_) => None,
        }
    }
}

/// Where a saved connection points.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Location {
    pub site: String,
    pub workspace: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StatusChoice {
    /// What to pass back to `set_status`.
    pub id: String,
    pub name: String,
    pub group: StateGroup,
    pub required_fields: Vec<RequiredField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TicketError {
    NotConnected(Tracker),
    NeedsFields(Vec<RequiredField>),
    Invalid(String),
    Remote(String),
}

impl fmt::Display for TicketError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConnected(tracker) => write!(f, "Connect {} in Settings", tracker.label()),
            Self::NeedsFields(fields) => {
                let names: Vec<&str> = fields.iter().map(|field| field.name.as_str()).collect();
                write!(f, "Moving this ticket needs: {}", names.join(", "))
            }
            Self::Invalid(message) | Self::Remote(message) => f.write_str(message),
        }
    }
}

impl std::error::Error for TicketError {}

impl From<ProviderError> for TicketError {
    fn from(error: ProviderError) -> Self {
        match error {
            ProviderError::InvalidInput(message) => Self::Invalid(message),
            other => Self::Remote(other.to_string()),
        }
    }
}

/// A tracker client plus the one thing only some trackers have: per-ticket
/// transitions that can demand fields.
pub(crate) trait Connection: Send + Sync {
    fn board(&self) -> &dyn BoardProvider;

    fn transitions(&self, _key: &str) -> ProviderResult<Option<Vec<Transition>>> {
        Ok(None)
    }

    /// `None` when URLs need no site check (Linear has one site).
    fn location(&self) -> Option<Location> {
        None
    }
}

impl Connection for JiraProvider {
    fn board(&self) -> &dyn BoardProvider {
        self
    }

    fn transitions(&self, key: &str) -> ProviderResult<Option<Vec<Transition>>> {
        JiraProvider::transitions(self, key).map(Some)
    }

    fn location(&self) -> Option<Location> {
        Some(Location {
            site: self.site().to_string(),
            workspace: None,
        })
    }
}

impl Connection for LinearProvider {
    fn board(&self) -> &dyn BoardProvider {
        self
    }
}

impl Connection for PlaneProvider {
    fn board(&self) -> &dyn BoardProvider {
        self
    }

    fn location(&self) -> Option<Location> {
        let config = self.config();
        Some(Location {
            site: config.base_url.clone(),
            workspace: Some(config.workspace.clone()),
        })
    }
}

pub struct Tickets {
    data_dir: PathBuf,
    #[cfg(test)]
    injected: Option<HashMap<Tracker, Arc<dyn Connection>>>,
}

impl Tickets {
    pub fn new(data_dir: PathBuf) -> Self {
        Self {
            data_dir,
            #[cfg(test)]
            injected: None,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_providers(
        data_dir: PathBuf,
        providers: Vec<(Tracker, Arc<dyn Connection>)>,
    ) -> Self {
        Self {
            data_dir,
            injected: Some(providers.into_iter().collect()),
        }
    }

    /// Trackers with saved credentials, in a fixed order.
    pub fn connected(&self) -> Vec<Tracker> {
        ALL.into_iter()
            .filter(|tracker| match self.connection(*tracker) {
                Ok(_) => true,
                Err(error) => {
                    // A credential that exists but cannot be read still counts as set up,
                    // so the user sees the real error on first use, not a prompt to reconnect.
                    !matches!(error, TicketError::NotConnected(_))
                }
            })
            .collect()
    }

    /// Never touches the network. A URL decides the tracker; a bare key belongs
    /// to whichever trackers are connected.
    pub fn resolve(&self, input: &str) -> Resolved {
        let Some(parsed) = parse_ticket(input) else {
            return Resolved::Invalid;
        };
        let ticket = |tracker, url| {
            Resolved::One(TicketRef {
                tracker,
                key: parsed.key.clone(),
                url,
                title: None,
                status: None,
                status_group: None,
                fetched_at: None,
            })
        };
        if let Some(tracker) = parsed.tracker {
            let Ok(connection) = self.connection(tracker) else {
                return Resolved::NotConnected(tracker);
            };
            if let Some(saved) = connection.location() {
                if let Some((connected, pasted)) = site_mismatch(
                    &saved,
                    parsed.site.as_deref().unwrap_or_default(),
                    parsed.workspace.as_deref(),
                ) {
                    return Resolved::OtherSite {
                        tracker,
                        connected,
                        pasted,
                    };
                }
            }
            return ticket(tracker, Some(input.trim().to_string()));
        }
        let mut connected = self.connected();
        match connected.len() {
            0 => Resolved::Invalid,
            1 => ticket(connected.remove(0), None),
            _ => Resolved::Ambiguous(connected),
        }
    }

    pub fn fetch(&self, r: &TicketRef) -> Result<TicketRef, TicketError> {
        self.read(r).map(|(ticket, _)| ticket)
    }

    /// The refreshed ticket and its description as Markdown.
    pub fn read(&self, r: &TicketRef) -> Result<(TicketRef, String), TicketError> {
        let connection = self.connection(r.tracker)?;
        let issue = connection.board().get_issue(&r.key)?;
        let description = issue.description.clone().unwrap_or_default();
        Ok((refreshed(r, &issue), description))
    }

    /// Returns the new comment's URL.
    pub fn comment(&self, r: &TicketRef, markdown: &str) -> Result<String, TicketError> {
        let connection = self.connection(r.tracker)?;
        Ok(connection.board().comment(&r.key, markdown, None)?.url)
    }

    pub fn statuses(&self, r: &TicketRef) -> Result<Vec<StatusChoice>, TicketError> {
        let connection = self.connection(r.tracker)?;
        if let Some(transitions) = connection.transitions(&r.key)? {
            return Ok(transitions
                .into_iter()
                .map(|t| StatusChoice {
                    id: t.id,
                    name: t.to.name,
                    group: t.to.group,
                    required_fields: t.required_fields,
                })
                .collect());
        }
        let issue = connection.board().get_issue(&r.key)?;
        Ok(connection
            .board()
            .list_states(&issue.container_id)?
            .into_iter()
            .map(|state| StatusChoice {
                id: state.id,
                name: state.name,
                group: state.group,
                required_fields: Vec::new(),
            })
            .collect())
    }

    /// `target` is a `StatusChoice` id, or a status name. A move that needs
    /// fields comes back as `NeedsFields` for the caller to ask for.
    pub fn set_status(
        &self,
        r: &TicketRef,
        target: &str,
        fields: BTreeMap<String, Value>,
    ) -> Result<TicketRef, TicketError> {
        let connection = self.connection(r.tracker)?;
        let board = connection.board();
        let state_id = match connection.transitions(&r.key)? {
            Some(transitions) => {
                ensure_fields(&transitions, target, &fields)?;
                target.to_string()
            }
            None => {
                supplied_fields_fit(&[], &fields)?;
                let issue = board.get_issue(&r.key)?;
                let target = target.trim();
                board
                    .list_states(&issue.container_id)?
                    .into_iter()
                    .find(|s| s.id == target || s.name.eq_ignore_ascii_case(target))
                    .map(|s| s.id)
                    .ok_or_else(|| TicketError::Invalid(format!("No status named {target}")))?
            }
        };
        let moved = board.move_issue(
            &r.key,
            &MoveTarget {
                state_id,
                required_fields: fields,
            },
        )?;
        Ok(refreshed(r, &moved))
    }

    fn connection(&self, tracker: Tracker) -> Result<Arc<dyn Connection>, TicketError> {
        #[cfg(test)]
        if let Some(injected) = &self.injected {
            return injected
                .get(&tracker)
                .cloned()
                .ok_or(TicketError::NotConnected(tracker));
        }
        let remote = |error: String| TicketError::Remote(error);
        let connection: Option<Arc<dyn Connection>> = match tracker {
            Tracker::Jira => inbox::jira::provider(&self.data_dir)
                .map_err(remote)?
                .map(|p| Arc::new(p) as _),
            Tracker::Linear => inbox::linear::provider(&self.data_dir)
                .map_err(remote)?
                .map(|p| Arc::new(p) as _),
            Tracker::Plane => inbox::plane::provider(&self.data_dir)
                .map_err(remote)?
                .map(|p| Arc::new(p) as _),
        };
        connection.ok_or(TicketError::NotConnected(tracker))
    }
}

fn plain_host(origin: &str) -> String {
    origin
        .split_once("://")
        .map_or(origin, |(_, rest)| rest)
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

/// Plane Cloud's web host and API host are the same site.
fn same_host(a: &str, b: &str) -> bool {
    let canonical = |origin: &str| match plain_host(origin).as_str() {
        "api.plane.so" => "app.plane.so".to_string(),
        host => host.to_string(),
    };
    canonical(a) == canonical(b)
}

/// `(saved, pasted)` labels when the pasted URL is for another site or workspace.
fn site_mismatch(
    saved: &Location,
    pasted_site: &str,
    pasted_workspace: Option<&str>,
) -> Option<(String, String)> {
    let same_site = same_host(&saved.site, pasted_site);
    let same_workspace = match (saved.workspace.as_deref(), pasted_workspace) {
        (Some(saved), Some(pasted)) => saved.eq_ignore_ascii_case(pasted),
        _ => true,
    };
    if same_site && same_workspace {
        return None;
    }
    let label = |site: &str, workspace: Option<&str>| match workspace {
        Some(workspace) => format!("{}/{workspace}", plain_host(site)),
        None => plain_host(site),
    };
    Some((
        label(&saved.site, saved.workspace.as_deref()),
        label(pasted_site, pasted_workspace),
    ))
}

/// Fails with the fields still missing when every transition to `target`
/// demands something the caller has not supplied. Supplied fields must be
/// ones the chosen transition asks for, with allowed values: they come from
/// the agent as well as the UI. A target no transition matches is left for
/// the tracker to reject, but carries no fields.
fn ensure_fields(
    transitions: &[Transition],
    target: &str,
    fields: &BTreeMap<String, Value>,
) -> Result<(), TicketError> {
    let target = target.trim();
    let missing_for = |t: &Transition| -> Vec<RequiredField> {
        t.required_fields
            .iter()
            .filter(|field| !fields.contains_key(&field.key))
            .cloned()
            .collect()
    };
    let mut fewest: Option<Vec<RequiredField>> = None;
    let mut refused = None;
    for t in transitions
        .iter()
        .filter(|t| t.id == target || t.to.id == target || t.to.name.eq_ignore_ascii_case(target))
    {
        let missing = missing_for(t);
        if missing.is_empty() {
            match supplied_fields_fit(&t.required_fields, fields) {
                Ok(()) => return Ok(()),
                Err(error) => {
                    refused.get_or_insert(error);
                    continue;
                }
            }
        }
        if fewest
            .as_ref()
            .is_none_or(|best| missing.len() < best.len())
        {
            fewest = Some(missing);
        }
    }
    if let Some(error) = refused {
        return Err(error);
    }
    match fewest {
        Some(missing) => Err(TicketError::NeedsFields(missing)),
        None => supplied_fields_fit(&[], fields),
    }
}

fn supplied_fields_fit(
    required: &[RequiredField],
    fields: &BTreeMap<String, Value>,
) -> Result<(), TicketError> {
    for (key, value) in fields {
        let Some(field) = required.iter().find(|field| &field.key == key) else {
            return Err(TicketError::Invalid(format!(
                "Field {key} isn't part of this status change"
            )));
        };
        if !field.allowed.is_empty() && !allowed_value(value, &field.allowed) {
            return Err(TicketError::Invalid(format!(
                "{} must be one of: {}",
                field.name,
                field.allowed.join(", ")
            )));
        }
    }
    Ok(())
}

/// A label, a `{name|value|id: label}` object, or a list of those.
fn allowed_value(value: &Value, allowed: &[String]) -> bool {
    let label = |text: &str| allowed.iter().any(|choice| choice == text);
    match value {
        Value::String(text) => label(text),
        Value::Array(items) => {
            !items.is_empty() && items.iter().all(|item| allowed_value(item, allowed))
        }
        Value::Object(map) => {
            !map.is_empty()
                && map.iter().all(|(key, value)| {
                    matches!(key.as_str(), "name" | "value" | "id")
                        && value.as_str().is_some_and(label)
                })
        }
        _ => false,
    }
}

fn refreshed(r: &TicketRef, issue: &RemoteIssue) -> TicketRef {
    TicketRef {
        tracker: r.tracker,
        key: r.key.clone(),
        url: if issue.url.is_empty() {
            r.url.clone()
        } else {
            Some(issue.url.clone())
        },
        title: Some(issue.title.clone()),
        status: Some(issue.state.name.clone()),
        status_group: Some(issue.state.group),
        fetched_at: Some(chrono::Utc::now().timestamp_millis()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_boards::fake::FakeProvider;
    use crate::provider_boards::WorkflowState;
    use gespan_tracker_secrets::testing;
    use serde_json::json;

    struct Conn {
        fake: FakeProvider,
        transitions: Option<Vec<Transition>>,
        location: Option<Location>,
    }

    impl Connection for Conn {
        fn board(&self) -> &dyn BoardProvider {
            &self.fake
        }

        fn transitions(&self, _: &str) -> ProviderResult<Option<Vec<Transition>>> {
            Ok(self.transitions.clone())
        }

        fn location(&self) -> Option<Location> {
            self.location.clone()
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
                group: StateGroup::Unstarted,
            },
            description: Some("Login fails on **Safari**".into()),
            ..Default::default()
        }
    }

    fn conn(fake: FakeProvider, transitions: Option<Vec<Transition>>) -> Arc<dyn Connection> {
        Arc::new(Conn {
            fake,
            transitions,
            location: None,
        })
    }

    fn tickets(providers: Vec<(Tracker, Arc<dyn Connection>)>) -> Tickets {
        testing::use_memory_keychain();
        Tickets::with_providers(PathBuf::from("/unused"), providers)
    }

    fn plain(fake: FakeProvider) -> Arc<dyn Connection> {
        conn(fake, None)
    }

    fn link(tracker: Tracker, key: &str) -> TicketRef {
        TicketRef {
            tracker,
            key: key.into(),
            url: None,
            title: None,
            status: None,
            status_group: None,
            fetched_at: None,
        }
    }

    fn at(site: &str, workspace: Option<&str>) -> Arc<dyn Connection> {
        Arc::new(Conn {
            fake: FakeProvider::new(),
            transitions: None,
            location: Some(Location {
                site: site.into(),
                workspace: workspace.map(String::from),
            }),
        })
    }

    fn other_site(tracker: Tracker, connected: &str, pasted: &str) -> Resolved {
        Resolved::OtherSite {
            tracker,
            connected: connected.into(),
            pasted: pasted.into(),
        }
    }

    #[test]
    fn jira_url_for_another_site_is_not_resolved_against_the_saved_one() {
        let t = tickets(vec![(
            Tracker::Jira,
            at("https://acme.atlassian.net", None),
        )]);
        let resolved = t.resolve("https://other.atlassian.net/browse/ENG-1");
        assert_eq!(
            resolved,
            other_site(Tracker::Jira, "acme.atlassian.net", "other.atlassian.net")
        );
        assert_eq!(
            resolved.message().unwrap(),
            "Jira is connected to acme.atlassian.net, not other.atlassian.net"
        );
    }

    #[test]
    fn jira_site_match_ignores_case() {
        let t = tickets(vec![(
            Tracker::Jira,
            at("https://acme.atlassian.net/", None),
        )]);
        assert!(matches!(
            t.resolve("https://ACME.atlassian.net/browse/ENG-1"),
            Resolved::One(_)
        ));
    }

    #[test]
    fn plane_web_and_api_hosts_are_the_same_site() {
        let t = tickets(vec![(
            Tracker::Plane,
            at("https://api.plane.so", Some("acme")),
        )]);
        assert!(matches!(
            t.resolve("https://app.plane.so/acme/browse/WEB-3"),
            Resolved::One(_)
        ));
    }

    #[test]
    fn plane_other_workspace_or_host_is_another_site() {
        let t = tickets(vec![(
            Tracker::Plane,
            at("https://plane.example.com", Some("acme")),
        )]);
        assert_eq!(
            t.resolve("https://plane.example.com/beta/browse/WEB-3"),
            other_site(
                Tracker::Plane,
                "plane.example.com/acme",
                "plane.example.com/beta"
            )
        );
        assert_eq!(
            t.resolve("https://plane.other.org/acme/browse/WEB-3"),
            other_site(
                Tracker::Plane,
                "plane.example.com/acme",
                "plane.other.org/acme"
            )
        );
        assert!(matches!(
            t.resolve("https://plane.example.com/ACME/browse/WEB-3"),
            Resolved::One(_)
        ));
    }

    #[test]
    fn bare_keys_and_linear_urls_skip_the_site_check() {
        let t = tickets(vec![
            (Tracker::Jira, at("https://acme.atlassian.net", None)),
            (Tracker::Linear, plain(FakeProvider::new())),
        ]);
        assert!(matches!(t.resolve("ENG-1"), Resolved::Ambiguous(_)));
        assert!(matches!(
            t.resolve("https://linear.app/any/issue/ENG-2/x"),
            Resolved::One(_)
        ));
    }

    fn done_transition(required: Vec<RequiredField>) -> Transition {
        Transition {
            id: "31".into(),
            name: "Finish".into(),
            to: WorkflowState {
                id: "done".into(),
                name: "Done".into(),
                group: StateGroup::Completed,
            },
            required_fields: required,
        }
    }

    fn resolution_field() -> RequiredField {
        RequiredField {
            key: "resolution".into(),
            name: "Resolution".into(),
            schema_type: "option".into(),
            allowed: vec!["Fixed".into()],
        }
    }

    #[test]
    fn resolve_url_picks_tracker() {
        let t = tickets(vec![
            (Tracker::Jira, plain(FakeProvider::new())),
            (Tracker::Linear, plain(FakeProvider::new())),
        ]);
        let url = "https://linear.app/acme/issue/ENG-42/fix-login";
        let Resolved::One(ticket) = t.resolve(url) else {
            panic!("expected One");
        };
        assert_eq!(ticket.tracker, Tracker::Linear);
        assert_eq!(ticket.key, "ENG-42");
        assert_eq!(ticket.url.as_deref(), Some(url));
        assert_eq!(ticket.fetched_at, None);
    }

    #[test]
    fn resolve_bare_key_with_one_tracker() {
        let t = tickets(vec![(Tracker::Plane, plain(FakeProvider::new()))]);
        let Resolved::One(ticket) = t.resolve(" web-7 ") else {
            panic!("expected One");
        };
        assert_eq!(
            (ticket.tracker, ticket.key.as_str()),
            (Tracker::Plane, "WEB-7")
        );
        assert_eq!(ticket.url, None);
    }

    #[test]
    fn resolve_bare_key_with_nothing_connected_or_garbage_is_invalid() {
        let t = tickets(vec![]);
        assert_eq!(t.resolve("ENG-1"), Resolved::Invalid);
        let t = tickets(vec![(Tracker::Jira, plain(FakeProvider::new()))]);
        assert_eq!(t.resolve("not a ticket"), Resolved::Invalid);
        assert_eq!(t.resolve(""), Resolved::Invalid);
    }

    #[test]
    fn resolve_reports_ambiguity() {
        let t = tickets(vec![
            (Tracker::Plane, plain(FakeProvider::new())),
            (Tracker::Jira, plain(FakeProvider::new())),
        ]);
        assert_eq!(
            t.resolve("ENG-1"),
            Resolved::Ambiguous(vec![Tracker::Jira, Tracker::Plane])
        );
    }

    #[test]
    fn resolve_disconnected_tracker() {
        let t = tickets(vec![(Tracker::Jira, plain(FakeProvider::new()))]);
        assert_eq!(
            t.resolve("https://linear.app/acme/issue/ENG-42/x"),
            Resolved::NotConnected(Tracker::Linear)
        );
    }

    #[test]
    fn connected_reads_saved_credentials_without_a_network_call() {
        testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        let t = Tickets::new(dir.path().to_path_buf());
        assert!(t.connected().is_empty());
        gespan_tracker_secrets::write(
            gespan_tracker_secrets::Tracker::Linear,
            dir.path(),
            "lin_api_x",
        )
        .unwrap();
        assert_eq!(t.connected(), vec![Tracker::Linear]);
        assert!(matches!(t.resolve("ENG-1"), Resolved::One(_)));
    }

    #[test]
    fn fetch_fills_cache_fields() {
        let t = tickets(vec![(
            Tracker::Linear,
            plain(FakeProvider::new().with_issue(issue("ENG-1"))),
        )]);
        let before = chrono::Utc::now().timestamp_millis();
        let fetched = t.fetch(&link(Tracker::Linear, "ENG-1")).unwrap();
        assert_eq!(fetched.title.as_deref(), Some("Fix login"));
        assert_eq!(fetched.status.as_deref(), Some("Todo"));
        assert_eq!(fetched.status_group, Some(StateGroup::Unstarted));
        assert_eq!(fetched.url.as_deref(), Some("https://tracker.test/ENG-1"));
        assert!(fetched.fetched_at.unwrap() >= before);
    }

    #[test]
    fn read_returns_the_description_and_remote_failures_are_remote_errors() {
        let fake = FakeProvider::new().with_issue(issue("ENG-1"));
        fake.fail_next("get_issue", ProviderError::Remote("HTTP 500".into()));
        let t = tickets(vec![(Tracker::Linear, plain(fake))]);
        let r = link(Tracker::Linear, "ENG-1");
        assert_eq!(
            t.read(&r).unwrap_err(),
            TicketError::Remote("HTTP 500".into())
        );
        let (_, description) = t.read(&r).unwrap();
        assert_eq!(description, "Login fails on **Safari**");
        assert!(matches!(
            t.fetch(&link(Tracker::Linear, "ENG-404")),
            Err(TicketError::Invalid(_))
        ));
    }

    #[test]
    fn comment_sends_markdown_and_returns_the_url() {
        let fake = Arc::new(FakeProvider::new().with_issue(issue("ENG-1")));
        struct Shared(Arc<FakeProvider>);
        impl Connection for Shared {
            fn board(&self) -> &dyn BoardProvider {
                &*self.0
            }
        }
        let t = tickets(vec![(Tracker::Linear, Arc::new(Shared(fake.clone())))]);
        let url = t
            .comment(&link(Tracker::Linear, "ENG-1"), "Shipped in `abc`")
            .unwrap();
        assert!(url.starts_with("fake://ENG-1#"));
        assert_eq!(fake.comments("ENG-1")[0].1, "Shipped in `abc`");
    }

    #[test]
    fn comment_on_disconnected_tracker_is_a_reconnect_error() {
        let t = tickets(vec![]);
        let error = t.comment(&link(Tracker::Jira, "ENG-1"), "hi").unwrap_err();
        assert_eq!(error, TicketError::NotConnected(Tracker::Jira));
        assert!(error.to_string().contains("Connect Jira in Settings"));
    }

    #[test]
    fn statuses_for_jira_are_the_transitions() {
        let fake = FakeProvider::new().with_issue(issue("ENG-1"));
        let t = tickets(vec![(
            Tracker::Jira,
            conn(fake, Some(vec![done_transition(vec![resolution_field()])])),
        )]);
        let choices = t.statuses(&link(Tracker::Jira, "ENG-1")).unwrap();
        assert_eq!(
            choices,
            vec![StatusChoice {
                id: "31".into(),
                name: "Done".into(),
                group: StateGroup::Completed,
                required_fields: vec![resolution_field()],
            }]
        );
    }

    #[test]
    fn statuses_for_other_trackers_are_the_container_states() {
        let t = tickets(vec![(
            Tracker::Plane,
            plain(FakeProvider::new().with_issue(issue("WEB-1"))),
        )]);
        let choices = t.statuses(&link(Tracker::Plane, "WEB-1")).unwrap();
        let names: Vec<&str> = choices.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Todo", "Doing", "Done"]);
        assert!(choices.iter().all(|c| c.required_fields.is_empty()));
    }

    #[test]
    fn set_status_resolves_a_state_name_and_returns_the_new_snapshot() {
        let t = tickets(vec![(
            Tracker::Linear,
            plain(FakeProvider::new().with_issue(issue("ENG-1"))),
        )]);
        let r = link(Tracker::Linear, "ENG-1");
        let moved = t.set_status(&r, "doing", BTreeMap::new()).unwrap();
        assert_eq!(moved.status.as_deref(), Some("Doing"));
        assert_eq!(moved.status_group, Some(StateGroup::Started));
        let moved = t.set_status(&r, "done", BTreeMap::new()).unwrap();
        assert_eq!(moved.status.as_deref(), Some("Done"));
        let error = t.set_status(&r, "Nope", BTreeMap::new()).unwrap_err();
        assert_eq!(error, TicketError::Invalid("No status named Nope".into()));
    }

    #[test]
    fn set_status_needing_fields_returns_needs_fields() {
        let fake = FakeProvider::new()
            .with_issue(issue("ENG-1"))
            .require_fields("done", &["resolution"]);
        let t = tickets(vec![(
            Tracker::Jira,
            conn(fake, Some(vec![done_transition(vec![resolution_field()])])),
        )]);
        let r = link(Tracker::Jira, "ENG-1");
        let error = t.set_status(&r, "31", BTreeMap::new()).unwrap_err();
        assert_eq!(error, TicketError::NeedsFields(vec![resolution_field()]));
        assert_eq!(error.to_string(), "Moving this ticket needs: Resolution");

        let mut fields = BTreeMap::new();
        fields.insert("resolution".to_string(), json!({"name": "Fixed"}));
        // The fake only knows state ids, so the retry passes the target status id.
        let moved = t.set_status(&r, "done", fields).unwrap();
        assert_eq!(moved.status.as_deref(), Some("Done"));
    }

    #[test]
    fn set_status_accepts_a_transition_that_needs_nothing_over_one_that_does() {
        let plain_done = Transition {
            id: "41".into(),
            ..done_transition(vec![])
        };
        let t = tickets(vec![(
            Tracker::Jira,
            conn(
                FakeProvider::new().with_issue(issue("ENG-1")),
                Some(vec![done_transition(vec![resolution_field()]), plain_done]),
            ),
        )]);
        let moved = t
            .set_status(&link(Tracker::Jira, "ENG-1"), "done", BTreeMap::new())
            .unwrap();
        assert_eq!(moved.status.as_deref(), Some("Done"));
    }

    fn jira_needing_resolution() -> (Arc<FakeProvider>, Tickets) {
        let fake = Arc::new(FakeProvider::new().with_issue(issue("ENG-1")));
        struct Shared(Arc<FakeProvider>, Vec<Transition>);
        impl Connection for Shared {
            fn board(&self) -> &dyn BoardProvider {
                &*self.0
            }
            fn transitions(&self, _: &str) -> ProviderResult<Option<Vec<Transition>>> {
                Ok(Some(self.1.clone()))
            }
        }
        // The fake moves by state id, so the transition shares it.
        let transition = Transition {
            id: "done".into(),
            ..done_transition(vec![resolution_field()])
        };
        let t = tickets(vec![(
            Tracker::Jira,
            Arc::new(Shared(fake.clone(), vec![transition])),
        )]);
        (fake, t)
    }

    fn fields(entries: &[(&str, Value)]) -> BTreeMap<String, Value> {
        entries
            .iter()
            .map(|(key, value)| (key.to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn set_status_refuses_fields_the_transition_does_not_ask_for() {
        let (fake, t) = jira_needing_resolution();
        let r = link(Tracker::Jira, "ENG-1");
        let error = t
            .set_status(
                &r,
                "done",
                fields(&[
                    ("resolution", json!("Fixed")),
                    ("assignee", json!({"id": "x"})),
                ]),
            )
            .unwrap_err();
        assert_eq!(
            error,
            TicketError::Invalid("Field assignee isn't part of this status change".into())
        );
        // Fields for a target no transition matches are refused too.
        let error = t
            .set_status(&r, "Elsewhere", fields(&[("resolution", json!("Fixed"))]))
            .unwrap_err();
        assert_eq!(
            error,
            TicketError::Invalid("Field resolution isn't part of this status change".into())
        );
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Todo");
    }

    #[test]
    fn set_status_refuses_a_value_outside_the_allowed_list() {
        let (fake, t) = jira_needing_resolution();
        let r = link(Tracker::Jira, "ENG-1");
        for value in [
            json!({"name": "Duplicate"}),
            json!("Duplicate"),
            json!(7),
            json!({"self": "Fixed"}),
            json!(["Fixed", "Duplicate"]),
        ] {
            let error = t
                .set_status(&r, "done", fields(&[("resolution", value.clone())]))
                .unwrap_err();
            assert_eq!(
                error,
                TicketError::Invalid("Resolution must be one of: Fixed".into()),
                "{value}"
            );
        }
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Todo");
        for value in [
            json!("Fixed"),
            json!({"value": "Fixed"}),
            json!({"id": "Fixed"}),
        ] {
            let (fake, t) = jira_needing_resolution();
            let moved = t
                .set_status(&r, "done", fields(&[("resolution", value)]))
                .unwrap();
            assert_eq!(moved.status.as_deref(), Some("Done"));
            assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Done");
        }
    }

    #[test]
    fn set_status_refuses_fields_where_moves_take_none() {
        for tracker in [Tracker::Linear, Tracker::Plane] {
            let t = tickets(vec![(
                tracker,
                plain(FakeProvider::new().with_issue(issue("ENG-1"))),
            )]);
            let r = link(tracker, "ENG-1");
            let error = t
                .set_status(&r, "done", fields(&[("priority", json!("High"))]))
                .unwrap_err();
            assert_eq!(
                error,
                TicketError::Invalid("Field priority isn't part of this status change".into())
            );
            assert_eq!(t.fetch(&r).unwrap().status.as_deref(), Some("Todo"));
            let moved = t.set_status(&r, "done", BTreeMap::new()).unwrap();
            assert_eq!(moved.status.as_deref(), Some("Done"));
        }
    }
}
