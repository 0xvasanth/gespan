use serde::Deserialize;
use serde_json::Value;

/// A machine saved in herdr, as `herdr machine list --json` prints it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Machine {
    pub id: String,
    pub label: String,
    pub target: String,
    pub session: String,
    pub enabled: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[serde(other)]
    Unknown,
}

/// One entry of `agent.list`. Fields herdr adds later are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Agent {
    pub terminal_id: String,
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    pub agent_status: AgentStatus,
    #[serde(default)]
    pub completion_seq: Option<u64>,
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Workspace {
    pub workspace_id: String,
    pub label: String,
}

/// The reply to `ping`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Pong {
    pub version: String,
    pub protocol: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    AgentStatusChanged {
        pane_id: String,
        workspace_id: String,
        agent_status: AgentStatus,
    },
    PaneCreated {
        pane_id: String,
    },
    PaneClosed {
        pane_id: String,
    },
    AgentDetected,
    /// An event this crate does not model, by its name.
    Other(String),
}

pub fn parse_machines(json: &str) -> Result<Vec<Machine>, serde_json::Error> {
    serde_json::from_str(json)
}

/// Parse one line of a subscription stream. Replies such as the subscription
/// ack carry no `event` name and are not events.
pub fn parse_event(line: &str) -> Option<Event> {
    let value: Value = serde_json::from_str(line).ok()?;
    let name = value.get("event")?.as_str()?;
    let data = value.get("data").unwrap_or(&Value::Null);
    let text = |v: &Value, key: &str| Some(v.get(key)?.as_str()?.to_owned());
    // Subscription events are dotted (`pane.agent_status_changed`); the other
    // events use snake_case (`pane_created`).
    Some(match name {
        "pane.agent_status_changed" => Event::AgentStatusChanged {
            pane_id: text(data, "pane_id")?,
            workspace_id: text(data, "workspace_id")?,
            agent_status: serde_json::from_value(data.get("agent_status")?.clone()).ok()?,
        },
        "pane_created" => Event::PaneCreated {
            pane_id: text(data.get("pane")?, "pane_id")?,
        },
        "pane_closed" => Event::PaneClosed {
            pane_id: text(data, "pane_id")?,
        },
        "pane_agent_detected" => Event::AgentDetected,
        other => Event::Other(other.to_owned()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event_lines() -> Vec<&'static str> {
        include_str!("../tests/fixtures/events.json")
            .lines()
            .collect()
    }

    #[test]
    fn parses_saved_machines() {
        let m = parse_machines(include_str!("../tests/fixtures/machine_list.json")).unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].session, "default");
        assert_eq!(m[0].target, "herdr-spike@orb");
        assert!(m[0].enabled);
    }

    #[test]
    fn status_event_parses() {
        assert_eq!(
            parse_event(event_lines()[0]),
            Some(Event::AgentStatusChanged {
                pane_id: "w1:p1".into(),
                workspace_id: "w1".into(),
                agent_status: AgentStatus::Working,
            })
        );
    }

    #[test]
    fn pane_created_uses_underscore_name() {
        assert_eq!(
            parse_event(event_lines()[1]),
            Some(Event::PaneCreated {
                pane_id: "w2:p1".into()
            })
        );
    }

    #[test]
    fn pane_closed_parses() {
        let line = r#"{"data":{"pane_id":"w1:p1","workspace_id":"w1","type":"pane_closed"},"event":"pane_closed"}"#;
        assert_eq!(
            parse_event(line),
            Some(Event::PaneClosed {
                pane_id: "w1:p1".into()
            })
        );
    }

    #[test]
    fn agent_detected_uses_snake_case_name() {
        let line = r#"{"data":{"pane_id":"w1:p1","workspace_id":"w1","agent":"claude"},"event":"pane_agent_detected"}"#;
        assert_eq!(parse_event(line), Some(Event::AgentDetected));
    }

    #[test]
    fn dotted_request_tags_are_not_pane_events() {
        for name in ["pane.created", "pane.closed", "pane.agent_detected"] {
            let line = format!(r#"{{"data":{{"pane_id":"w1:p1"}},"event":"{name}"}}"#);
            assert_eq!(parse_event(&line), Some(Event::Other(name.into())));
        }
    }

    #[test]
    fn subscription_ack_is_not_an_event() {
        assert_eq!(parse_event(event_lines()[2]), None);
    }

    #[test]
    fn garbage_line_is_not_an_event() {
        assert_eq!(parse_event("not json"), None);
        assert_eq!(parse_event(""), None);
    }

    #[test]
    fn unknown_event_is_other() {
        assert_eq!(
            parse_event(r#"{"data":{},"event":"layout.updated"}"#),
            Some(Event::Other("layout.updated".into()))
        );
    }

    #[test]
    fn modelled_event_missing_fields_is_dropped() {
        assert_eq!(
            parse_event(r#"{"data":{"pane_id":"w1:p1"},"event":"pane.agent_status_changed"}"#),
            None
        );
    }

    #[test]
    fn unknown_status_in_event_is_unknown() {
        let line = r#"{"data":{"pane_id":"w1:p1","workspace_id":"w1","agent_status":"sleeping"},"event":"pane.agent_status_changed"}"#;
        assert!(matches!(
            parse_event(line),
            Some(Event::AgentStatusChanged {
                agent_status: AgentStatus::Unknown,
                ..
            })
        ));
    }

    #[test]
    fn agent_list_fixture_parses() {
        let v: Value =
            serde_json::from_str(include_str!("../tests/fixtures/agent_list.json")).unwrap();
        let agents: Vec<Agent> = serde_json::from_value(v["result"]["agents"].clone()).unwrap();
        assert_eq!(agents[0].agent.as_deref(), Some("claude"));
        assert_eq!(agents[0].agent_status, AgentStatus::Blocked);
        assert_eq!(agents[0].completion_seq, None);
        assert_eq!(agents[0].tab_id, "w1:t1");
    }

    #[test]
    fn unknown_status_and_fields_are_tolerated() {
        let agent: Agent = serde_json::from_str(
            r#"{"terminal_id":"t","pane_id":"w1:p1","workspace_id":"w1","tab_id":"w1:t1","agent_status":"sleeping","completion_seq":4,"brand_new":{"x":1}}"#,
        )
        .unwrap();
        assert_eq!(agent.agent_status, AgentStatus::Unknown);
        assert_eq!(agent.completion_seq, Some(4));
    }

    #[test]
    fn workspace_and_pong_fixtures_parse() {
        let v: Value =
            serde_json::from_str(include_str!("../tests/fixtures/workspace_list.json")).unwrap();
        let ws: Vec<Workspace> = serde_json::from_value(v["result"]["workspaces"].clone()).unwrap();
        assert_eq!(
            (ws[0].workspace_id.as_str(), ws[0].label.as_str()),
            ("w1", "tmp")
        );
        let v: Value = serde_json::from_str(include_str!("../tests/fixtures/ping.json")).unwrap();
        let pong: Pong = serde_json::from_value(v["result"].clone()).unwrap();
        assert_eq!(
            pong,
            Pong {
                version: "0.9.5".into(),
                protocol: 22
            }
        );
    }
}
