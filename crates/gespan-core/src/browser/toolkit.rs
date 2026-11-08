//! The browser tools in the app's MCP hub. The hub grant's session is the
//! browser's session ([`super::session_key`]); the host's policy decides
//! everything else on each call.

use std::sync::Arc;

use gespan_browser::agent::{mcp, tools};
use gespan_browser::protocol::ActionRequest;
use gespan_browser::BrowserHost;
use mcp_hub::Grant;
use serde_json::Value;

use super::session_key;

/// The `browser` toolkit. The app registers [`Toolkit::GLOBAL`], which
/// follows the process-wide host; tests give each toolkit a host of its own.
pub struct Toolkit {
    host: Source,
}

enum Source {
    /// The host [`super::start`] started, looked up at each call, so the
    /// hub may register the toolkit before the module starts.
    Global,
    Fixed(Option<Arc<BrowserHost>>),
}

impl Toolkit {
    pub const GLOBAL: Toolkit = Toolkit {
        host: Source::Global,
    };

    /// A toolkit on `host` rather than the process-wide one.
    pub fn with_host(host: Arc<BrowserHost>) -> Toolkit {
        Toolkit {
            host: Source::Fixed(Some(host)),
        }
    }

    fn host(&self) -> Option<Arc<BrowserHost>> {
        match &self.host {
            Source::Global => gespan_browser::host(),
            Source::Fixed(host) => host.clone(),
        }
    }
}

/// A request id for a call that brings none: retries bring their own.
fn fresh_request_id() -> String {
    format!("gespan-{}", uuid::Uuid::new_v4().simple())
}

impl mcp_hub::Toolkit for Toolkit {
    fn name(&self) -> &'static str {
        crate::tool_servers::BROWSER
    }

    /// The tools, while the switch is on and the session has access.
    fn tools(&self, grant: &Grant) -> Vec<Value> {
        let Some(host) = self.host() else {
            return Vec::new();
        };
        if !gespan_browser::turn::granted(&host.state(), &session_key(grant.session)) {
            return Vec::new();
        }
        tools::TOOLS.into_iter().map(tools::definition).collect()
    }

    /// One action: always an MCP result, an error result naming the reason
    /// when the host refused or failed it.
    fn call(&self, grant: &Grant, tool: &str, arguments: Value) -> Result<Value, String> {
        let host = self
            .host()
            .ok_or("The browser module is not running in Gespan.")?;
        let mapped = tools::map_call(tool, Some(&arguments))?;
        let result = host.handle_request(
            &session_key(grant.session),
            ActionRequest {
                request_id: mapped.request_id.unwrap_or_else(fresh_request_id),
                action: mapped.kind.name().to_string(),
                input: mapped.input,
            },
        );
        Ok(mcp::tool_result(Some(mapped.kind), &result))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser::tests::{facts, host_on, unreachable_host, wait_for};
    use crate::browser::{
        begin_turn_on, session_deleted_on, AccessMode, ConnectionState, TurnFacts,
    };
    use gespan_browser::testing::FakeCdp;
    use mcp_hub::Toolkit as _;
    use serde_json::json;
    use std::collections::BTreeSet;
    use std::time::{Duration, Instant};

    fn grant(session: i64) -> Grant {
        Grant {
            session,
            workspace: "/repo".into(),
            toolkits: BTreeSet::from([crate::tool_servers::BROWSER.to_owned()]),
            servers: BTreeSet::new(),
        }
    }

    fn typed(session: i64) -> TurnFacts {
        TurnFacts {
            session,
            user_typed: true,
            ..facts()
        }
    }

    fn text(result: &Value) -> String {
        result["content"]
            .as_array()
            .expect("content array")
            .iter()
            .filter_map(|block| block["text"].as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn tool_names(tools: &[Value]) -> Vec<String> {
        tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    }

    /// A host attached to a fake browser, its switch on and "Always on".
    fn connected() -> (FakeCdp, Arc<BrowserHost>) {
        let server = FakeCdp::start();
        let host = host_on(&server.profile_dir(), AccessMode::Always);
        host.set_toggle(true);
        wait_for(&host, ConnectionState::Connected);
        (server, host)
    }

    fn open_tab(kit: &Toolkit, session: i64) -> String {
        let opened = kit
            .call(&grant(session), "browser_tabs", json!({"action": "open"}))
            .unwrap();
        assert_eq!(opened["isError"], json!(false), "{opened}");
        let tab = text(&opened)
            .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .find(|word| word.starts_with("t_") && word.len() == 10)
            .map(str::to_owned);
        tab.unwrap_or_else(|| panic!("no tab id in {opened}"))
    }

    fn listed_tabs(kit: &Toolkit, session: i64) -> String {
        let listed = kit
            .call(&grant(session), "browser_tabs", json!({"action": "list"}))
            .unwrap();
        assert_eq!(listed["isError"], json!(false), "{listed}");
        text(&listed)
    }

    #[test]
    fn the_toolkit_is_named_like_its_mount() {
        assert_eq!(Toolkit::GLOBAL.name(), crate::tool_servers::BROWSER);
        assert_eq!(crate::tool_servers::BROWSER, "browser");
    }

    #[test]
    fn tools_listed_only_for_a_granted_session() {
        let (host, _profile) = unreachable_host(AccessMode::Always);
        host.set_toggle(true);
        let kit = Toolkit::with_host(host.clone());
        let _turn = begin_turn_on(Some(host.clone()), &typed(1));
        let tools = kit.tools(&grant(1));
        assert_eq!(tools.len(), 18);
        let expected: Vec<String> = gespan_browser::agent::tools::TOOLS
            .iter()
            .map(|tool| tool.name.to_owned())
            .collect();
        assert_eq!(tool_names(&tools), expected);
        assert!(tools.iter().all(|tool| tool["inputSchema"].is_object()));
        assert!(kit.tools(&grant(2)).is_empty());
    }

    #[test]
    fn no_tools_and_a_plain_error_before_the_module_starts() {
        let kit = Toolkit {
            host: Source::Fixed(None),
        };
        assert!(kit.tools(&grant(1)).is_empty());
        let error = kit
            .call(&grant(1), "browser_tabs", json!({"action": "list"}))
            .unwrap_err();
        assert!(error.contains("not running"), "{error}");
    }

    #[test]
    fn bad_arguments_never_reach_the_browser() {
        let (server, host) = connected();
        let _turn = begin_turn_on(Some(host.clone()), &typed(1));
        let kit = Toolkit::with_host(host);
        let before = server.received().len();
        for (tool, arguments) in [
            ("browser_tabs", json!({"action": "explode"})),
            ("browser_click", json!("not an object")),
            ("browser_nope", json!({})),
            (
                "browser_tabs",
                json!({"action": "list", "request_id": "bad id!"}),
            ),
        ] {
            assert!(kit.call(&grant(1), tool, arguments).is_err(), "{tool}");
        }
        assert_eq!(server.received().len(), before);
    }

    #[test]
    fn calls_get_a_fresh_request_id_unless_retrying() {
        let (_server, host) = connected();
        let _turn = begin_turn_on(Some(host.clone()), &typed(1));
        let kit = Toolkit::with_host(host);
        let first = open_tab(&kit, 1);
        let second = open_tab(&kit, 1);
        assert_ne!(first, second, "two calls without an id are two actions");
        // A retry with the agent's own id is the same action, answered again.
        let retry = |kit: &Toolkit| {
            kit.call(
                &grant(1),
                "browser_tabs",
                json!({"action": "open", "request_id": "retry-1"}),
            )
            .unwrap()
        };
        let once = retry(&kit);
        let twice = retry(&kit);
        assert_eq!(once["isError"], json!(false), "{once}");
        assert_eq!(text(&once), text(&twice));
    }

    #[test]
    fn switch_off_mid_turn_fails_fast_and_unlists() {
        let (_server, host) = connected();
        let _turn = begin_turn_on(Some(host.clone()), &typed(1));
        let kit = Toolkit::with_host(host.clone());
        let tab = open_tab(&kit, 1);
        assert_eq!(kit.tools(&grant(1)).len(), 18);

        host.set_toggle(false);
        assert!(kit.tools(&grant(1)).is_empty());
        let started = Instant::now();
        let off = kit
            .call(&grant(1), "browser_snapshot", json!({"tab": tab}))
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(1));
        assert_eq!(off["isError"], json!(true), "{off}");
        assert!(text(&off).contains("off"), "{off}");
    }

    #[test]
    fn sessions_cannot_reach_each_others_tabs() {
        let (server, host) = connected();
        let _one = begin_turn_on(Some(host.clone()), &typed(1));
        let _two = begin_turn_on(Some(host.clone()), &typed(2));
        let kit = Toolkit::with_host(host);
        let tab = open_tab(&kit, 1);

        let closed = kit
            .call(
                &grant(2),
                "browser_tabs",
                json!({"action": "close", "tab": tab}),
            )
            .unwrap();
        assert_eq!(closed["isError"], json!(true), "{closed}");
        assert!(!server
            .received()
            .iter()
            .any(|r| r.method == "Target.closeTarget"));
        assert!(listed_tabs(&kit, 1).contains(&tab));
        assert!(!listed_tabs(&kit, 2).contains(&tab));
    }

    #[test]
    fn attach_failure_names_the_fix() {
        let (host, _profile) = unreachable_host(AccessMode::Always);
        host.set_toggle(true);
        wait_for(&host, ConnectionState::NeedsToggle);
        let _turn = begin_turn_on(Some(host.clone()), &typed(1));
        let kit = Toolkit::with_host(host);
        let started = Instant::now();
        let opened = kit
            .call(&grant(1), "browser_tabs", json!({"action": "open"}))
            .unwrap();
        assert!(started.elapsed() < Duration::from_secs(10));
        assert_eq!(opened["isError"], json!(true), "{opened}");
        assert!(text(&opened).contains("chrome://inspect"), "{opened}");
    }

    #[test]
    fn deleting_a_session_unlists_its_tools() {
        let (host, _profile) = unreachable_host(AccessMode::Always);
        host.set_toggle(true);
        let kit = Toolkit::with_host(host.clone());
        let turn = begin_turn_on(Some(host.clone()), &typed(1));
        assert_eq!(kit.tools(&grant(1)).len(), 18);
        drop(turn);
        session_deleted_on(&host, 1);
        assert!(kit.tools(&grant(1)).is_empty());
    }
}
