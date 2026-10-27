//! The ticket tools coding agents call, shared by the native MCP bridge and
//! the Tauri app: what each tool is, when a session may not run it, and how
//! the app answers it through `Tickets`. Transport and scope (which ticket,
//! which session) belong to the caller; nothing here trusts agent input for
//! either.
use crate::provider_boards::RequiredField;
use crate::tickets::{TicketError, TicketRef, Tickets};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

pub const STATUS_OFF: &str = "Changing ticket status is turned off for this session";
pub const READ_ONLY: &str = "This session is read-only";
/// A ticket tool called by a session with no ticket linked right now.
pub const NO_TICKET: &str = "This session isn't linked to a ticket";

/// A ticket tool call from an agent, tagged by `tool`. Unknown keys are
/// refused.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "RawTicketTool", into = "RawTicketTool")]
pub enum TicketTool {
    Get,
    Comment {
        markdown: String,
    },
    Statuses,
    SetStatus {
        target: String,
        fields: BTreeMap<String, Value>,
    },
}

/// The wire shape. `get` and `statuses` are empty struct variants so serde
/// refuses extra keys on them too (it doesn't on a unit variant).
#[derive(Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "snake_case", deny_unknown_fields)]
enum RawTicketTool {
    Get {},
    Comment {
        markdown: String,
    },
    Statuses {},
    SetStatus {
        target: String,
        #[serde(default)]
        fields: BTreeMap<String, Value>,
    },
}

impl From<RawTicketTool> for TicketTool {
    fn from(raw: RawTicketTool) -> Self {
        match raw {
            RawTicketTool::Get {} => Self::Get,
            RawTicketTool::Comment { markdown } => Self::Comment { markdown },
            RawTicketTool::Statuses {} => Self::Statuses,
            RawTicketTool::SetStatus { target, fields } => Self::SetStatus { target, fields },
        }
    }
}

impl From<TicketTool> for RawTicketTool {
    fn from(tool: TicketTool) -> Self {
        match tool {
            TicketTool::Get => Self::Get {},
            TicketTool::Comment { markdown } => Self::Comment { markdown },
            TicketTool::Statuses => Self::Statuses {},
            TicketTool::SetStatus { target, fields } => Self::SetStatus { target, fields },
        }
    }
}

/// Why a tool may not run in this turn, before it reaches the app.
pub fn refusal(
    tool: &TicketTool,
    may_change_status: bool,
    read_only: bool,
) -> Option<&'static str> {
    let writes = matches!(
        tool,
        TicketTool::Comment { .. } | TicketTool::SetStatus { .. }
    );
    if writes && read_only {
        Some(READ_ONLY)
    } else if matches!(tool, TicketTool::SetStatus { .. }) && !may_change_status {
        Some(STATUS_OFF)
    } else {
        None
    }
}

/// What a successful call did to the ticket, so the app can update its chip
/// without fetching again, or know it needs nothing.
#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    None,
    Commented,
    Moved(TicketRef),
}

#[derive(Debug)]
pub struct Answered {
    pub value: Value,
    pub effect: Effect,
}

/// The app's side of a call. Errors are the user-facing tracker messages,
/// never credentials or raw responses.
pub fn answer(
    tickets: &Tickets,
    ticket: &TicketRef,
    tool: &TicketTool,
) -> Result<Answered, String> {
    let unchanged = |value| Answered {
        value,
        effect: Effect::None,
    };
    let result = match tool {
        TicketTool::Get => tickets.read(ticket).map(|(fresh, description)| {
            json!({
                "key": fresh.key,
                "tracker": fresh.tracker.label(),
                "title": fresh.title,
                "status": fresh.status,
                "url": fresh.url,
                "description": description,
            })
        })
        .map(unchanged),
        TicketTool::Comment { markdown } => tickets
            .comment(ticket, markdown)
            .map(|url| Answered {
                value: json!({"commented":true,"url":url}),
                effect: Effect::Commented,
            }),
        TicketTool::Statuses => tickets.statuses(ticket).map(|choices| {
            json!({
                "key": ticket.key,
                "statuses": choices
                    .into_iter()
                    .map(|choice| json!({
                        "id": choice.id,
                        "name": choice.name,
                        "group": choice.group,
                        "requiredFields": choice
                            .required_fields
                            .iter()
                            .map(|field| json!({"key":field.key,"name":field.name,"allowed":field.allowed}))
                            .collect::<Vec<_>>(),
                    }))
                    .collect::<Vec<_>>(),
            })
        })
        .map(unchanged),
        TicketTool::SetStatus { target, fields } => tickets
            .set_status(ticket, target, fields.clone())
            .map(|moved| Answered {
                value: json!({"key":moved.key,"status":moved.status,"url":moved.url}),
                effect: Effect::Moved(moved),
            }),
    };
    result.map_err(|error| match &error {
        TicketError::NeedsFields(fields) => format!(
            "{error}. Pass them in fields by key: {}",
            fields
                .iter()
                .map(describe_field)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        TicketError::Remote(message) => short_remote_error(message),
        _ => error.to_string(),
    })
}

const MAX_REMOTE_ERROR: usize = 300;

/// Remote failures can carry a raw response or request dump. Agents get the
/// first non-blank line, cut to `MAX_REMOTE_ERROR` characters. A line that
/// names a credential header (`authorization`, `x-api-key`, `cookie`) is
/// replaced by a generic message. This is a header-name denylist on that one
/// line, not a scrubber for token values.
fn short_remote_error(message: &str) -> String {
    const GENERIC: &str = "The tracker request failed";
    let Some(line) = message.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return GENERIC.into();
    };
    let lower = line.to_ascii_lowercase();
    if ["authorization", "x-api-key", "cookie"]
        .iter()
        .any(|header| lower.contains(header))
    {
        return GENERIC.into();
    }
    line.chars().take(MAX_REMOTE_ERROR).collect()
}

fn describe_field(field: &RequiredField) -> String {
    if field.allowed.is_empty() {
        format!("{} ({})", field.key, field.name)
    } else {
        format!(
            "{} ({}; one of: {})",
            field.key,
            field.name,
            field.allowed.join(", ")
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider_boards::fake::FakeProvider;
    use crate::provider_boards::{
        BoardProvider, ProviderResult, RemoteIssue, StateGroup, Transition, WorkflowState,
    };
    use crate::tickets::{Connection, Tracker};
    use std::path::PathBuf;
    use std::sync::Arc;

    #[test]
    fn ticket_tool_parses_the_real_payloads_and_refuses_unknown_keys() {
        let parse = |value: Value| serde_json::from_value::<TicketTool>(value);
        assert_eq!(parse(json!({"tool": "get"})).unwrap(), TicketTool::Get);
        assert_eq!(
            parse(json!({"tool": "statuses"})).unwrap(),
            TicketTool::Statuses
        );
        assert_eq!(
            parse(json!({"tool": "comment", "markdown": "Done"})).unwrap(),
            TicketTool::Comment {
                markdown: "Done".into()
            }
        );
        let fields: BTreeMap<String, Value> = [("resolution".to_string(), json!("Fixed"))].into();
        assert_eq!(
            parse(
                json!({"tool": "set_status", "target": "Done", "fields": {"resolution": "Fixed"}})
            )
            .unwrap(),
            TicketTool::SetStatus {
                target: "Done".into(),
                fields: fields.clone()
            }
        );
        // `fields` may be left out.
        assert_eq!(
            parse(json!({"tool": "set_status", "target": "Done"})).unwrap(),
            TicketTool::SetStatus {
                target: "Done".into(),
                fields: BTreeMap::new()
            }
        );
        // An extra key on any tool is refused, by name.
        for bad in [
            json!({"tool": "get", "key": "ENG-2"}),
            json!({"tool": "statuses", "sessionId": "other"}),
            json!({"tool": "comment", "markdown": "x", "ticket": "ENG-2"}),
            json!({"tool": "set_status", "target": "Done", "fields": {}, "key": "ENG-2"}),
        ] {
            let error = parse(bad.clone()).expect_err(&bad.to_string());
            assert!(
                error.to_string().contains("unknown field"),
                "{bad}: {error}"
            );
        }
        for bad in [
            json!({"tool": "comment"}),
            json!({"tool": "delete"}),
            json!({"markdown": "x"}),
        ] {
            assert!(parse(bad.clone()).is_err(), "{bad}");
        }
        // The wire shape round-trips: what the native bridge sends parses back.
        for tool in [
            TicketTool::Get,
            TicketTool::Statuses,
            TicketTool::Comment {
                markdown: "m".into(),
            },
            TicketTool::SetStatus {
                target: "Done".into(),
                fields,
            },
        ] {
            let wire = serde_json::to_value(&tool).unwrap();
            assert_eq!(parse(wire).unwrap(), tool);
        }
        assert_eq!(
            serde_json::to_value(TicketTool::Get).unwrap(),
            json!({"tool": "get"})
        );
    }

    struct Conn {
        fake: Arc<FakeProvider>,
        transitions: Option<Vec<Transition>>,
    }

    impl Connection for Conn {
        fn board(&self) -> &dyn BoardProvider {
            &*self.fake
        }

        fn transitions(&self, _: &str) -> ProviderResult<Option<Vec<Transition>>> {
            Ok(self.transitions.clone())
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

    fn linked(key: &str) -> TicketRef {
        TicketRef {
            tracker: Tracker::Linear,
            key: key.into(),
            url: None,
            title: None,
            status: None,
            status_group: None,
            fetched_at: None,
        }
    }

    fn tickets(fake: Arc<FakeProvider>, transitions: Option<Vec<Transition>>) -> Tickets {
        gespan_tracker_secrets::testing::use_memory_keychain();
        Tickets::with_providers(
            PathBuf::from("/unused"),
            vec![(Tracker::Linear, Arc::new(Conn { fake, transitions }))],
        )
    }

    fn set_status(target: &str) -> TicketTool {
        TicketTool::SetStatus {
            target: target.into(),
            fields: BTreeMap::new(),
        }
    }

    #[test]
    fn refusal_rules_for_read_only_and_status_off() {
        let comment = TicketTool::Comment {
            markdown: "Note".into(),
        };
        let reads = [TicketTool::Get, TicketTool::Statuses];
        // Reads and status listings are always allowed.
        for read_only in [false, true] {
            for may_change_status in [false, true] {
                for tool in &reads {
                    assert_eq!(
                        refusal(tool, may_change_status, read_only),
                        None,
                        "{tool:?}"
                    );
                }
            }
        }
        // A writable session with status changes on may do everything.
        assert_eq!(refusal(&comment, true, false), None);
        assert_eq!(refusal(&set_status("Done"), true, false), None);
        // Status changes off stops only the move; comments still go through.
        assert_eq!(refusal(&set_status("Done"), false, false), Some(STATUS_OFF));
        assert_eq!(refusal(&comment, false, false), None);
        // Read-only stops every write, and wins over status-off.
        for may_change_status in [false, true] {
            assert_eq!(refusal(&comment, may_change_status, true), Some(READ_ONLY));
            assert_eq!(
                refusal(&set_status("Done"), may_change_status, true),
                Some(READ_ONLY)
            );
        }
        assert_eq!(
            STATUS_OFF,
            "Changing ticket status is turned off for this session"
        );
        assert_eq!(READ_ONLY, "This session is read-only");
        assert_eq!(NO_TICKET, "This session isn't linked to a ticket");
    }

    #[test]
    fn remote_errors_reach_agents_as_one_short_line() {
        use crate::provider_boards::ProviderError;
        let remote = |message: String| {
            let fake = Arc::new(FakeProvider::new().with_issue(issue("ENG-1")));
            fake.fail_next("get_issue", ProviderError::Remote(message));
            let t = tickets(fake, None);
            answer(&t, &linked("ENG-1"), &TicketTool::Get).unwrap_err()
        };
        // Only the first line survives.
        let error = remote("HTTP 500\nauthorization: Bearer abc\nx-api-key: def".into());
        assert_eq!(error, "HTTP 500");
        // Leading blank lines are skipped; nothing left falls back.
        assert_eq!(remote("\n  \nHTTP 502\nbody".into()), "HTTP 502");
        assert_eq!(remote(" \n\n".into()), "The tracker request failed");
        // Length is capped.
        let error = remote("a".repeat(1000));
        assert_eq!(error.chars().count(), 300);
        // A first line naming a credential header is not passed on.
        for line in ["Authorization: Bearer abc", "sent X-Api-Key: def"] {
            let error = remote(format!("{line}\nmore"));
            let lower = error.to_ascii_lowercase();
            assert!(!lower.contains("authorization") && !lower.contains("x-api-key"));
            assert!(!error.contains("abc") && !error.contains("def"));
            assert!(!error.is_empty());
        }
    }

    #[test]
    fn answer_reports_needs_fields_with_field_list() {
        let fake = Arc::new(FakeProvider::new().with_issue(issue("ENG-1")));
        let transition = Transition {
            // The fake moves by state id, so the transition shares it.
            id: "done".into(),
            name: "Finish".into(),
            to: WorkflowState {
                id: "done".into(),
                name: "Done".into(),
                group: StateGroup::Completed,
            },
            required_fields: vec![
                RequiredField {
                    key: "resolution".into(),
                    name: "Resolution".into(),
                    schema_type: "option".into(),
                    allowed: vec!["Fixed".into(), "Won't Do".into()],
                },
                RequiredField {
                    key: "customfield_1".into(),
                    name: "Root cause".into(),
                    schema_type: "string".into(),
                    allowed: Vec::new(),
                },
            ],
        };
        let tickets = tickets(fake.clone(), Some(vec![transition]));
        let ticket = linked("ENG-1");
        let listing = answer(&tickets, &ticket, &TicketTool::Statuses).unwrap();
        assert_eq!(listing.effect, Effect::None);
        assert_eq!(listing.value["key"], "ENG-1");
        assert_eq!(listing.value["statuses"][0]["id"], "done");
        assert_eq!(
            listing.value["statuses"][0]["requiredFields"],
            json!([
                {"key":"resolution","name":"Resolution","allowed":["Fixed","Won't Do"]},
                {"key":"customfield_1","name":"Root cause","allowed":[]},
            ])
        );
        let error = answer(&tickets, &ticket, &set_status("done")).unwrap_err();
        assert_eq!(
            error,
            "Moving this ticket needs: Resolution, Root cause. Pass them in fields by key: \
             resolution (Resolution; one of: Fixed, Won't Do), customfield_1 (Root cause)"
        );
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Todo");
        let moved = answer(
            &tickets,
            &ticket,
            &TicketTool::SetStatus {
                target: "done".into(),
                fields: BTreeMap::from([
                    ("resolution".into(), json!({"name":"Fixed"})),
                    ("customfield_1".into(), json!("Race")),
                ]),
            },
        )
        .unwrap();
        assert_eq!(moved.value["status"], "Done");
        assert_eq!(fake.get_issue("ENG-1").unwrap().state.name, "Done");
    }

    #[test]
    fn answers_report_what_changed_on_the_ticket() {
        let fake = Arc::new(FakeProvider::new().with_issue(issue("ENG-1")));
        let tickets = tickets(fake.clone(), None);
        let ticket = linked("ENG-1");
        let effect = |tool: TicketTool| answer(&tickets, &ticket, &tool).unwrap().effect;
        assert_eq!(effect(TicketTool::Get), Effect::None);
        assert_eq!(effect(TicketTool::Statuses), Effect::None);
        assert_eq!(
            effect(TicketTool::Comment {
                markdown: "Note".into()
            }),
            Effect::Commented
        );
        assert_eq!(fake.comments("ENG-1")[0].1, "Note");
        let Effect::Moved(moved) = effect(set_status("Doing")) else {
            panic!("a status move returns the moved ticket");
        };
        assert_eq!(moved.key, "ENG-1");
        assert_eq!(moved.status.as_deref(), Some("Doing"));
        assert_eq!(moved.status_group, Some(StateGroup::Started));
        assert!(answer(&tickets, &linked("ENG-404"), &TicketTool::Get).is_err());
    }

    #[test]
    fn ticket_get_answers_key_title_status_and_description() {
        let fake = Arc::new(FakeProvider::new().with_issue(issue("ENG-1")));
        let tickets = tickets(fake, None);
        let read = answer(&tickets, &linked("ENG-1"), &TicketTool::Get).unwrap();
        assert_eq!(
            read.value,
            json!({
                "key": "ENG-1",
                "tracker": "Linear",
                "title": "Fix login",
                "status": "Todo",
                "url": "https://tracker.test/ENG-1",
                "description": "Login fails on **Safari**",
            })
        );
    }

    #[test]
    fn tools_round_trip_on_the_tool_tag() {
        for (tool, wire) in [
            (TicketTool::Get, json!({"tool":"get"})),
            (TicketTool::Statuses, json!({"tool":"statuses"})),
            (
                TicketTool::Comment {
                    markdown: "Hi".into(),
                },
                json!({"tool":"comment","markdown":"Hi"}),
            ),
            (
                set_status("Done"),
                json!({"tool":"set_status","target":"Done","fields":{}}),
            ),
        ] {
            assert_eq!(serde_json::to_value(&tool).unwrap(), wire);
            assert_eq!(serde_json::from_value::<TicketTool>(wire).unwrap(), tool);
        }
    }
}
