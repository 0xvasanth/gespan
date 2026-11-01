//! A catalogue change reaching a session that's already running. Tool lists
//! come from the catalogue, so this needs no real agent and starts no server.
use gespan_core::{
    mcp_catalog::{Catalog, Entry},
    tool_servers::{self, Access, Mount},
};
use serde_json::{json, Value};
use std::collections::BTreeSet;

struct Agent {
    url: String,
    bearer: String,
    session: Option<String>,
}

impl Agent {
    fn connect(access: &Access) -> Self {
        let request = access.claude_control("test");
        let server = &request["request"]["servers"]["gespan"];
        let mut agent = Self {
            url: server["url"].as_str().unwrap().to_owned(),
            bearer: server["headers"]["Authorization"]
                .as_str()
                .unwrap()
                .to_owned(),
            session: None,
        };
        agent.send(
            json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{
                "protocolVersion":"2025-11-25",
                "capabilities":{},
                "clientInfo":{"name":"test","version":"1"},
            }}),
        );
        agent.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
        agent
    }

    fn send(&mut self, message: Value) -> Option<Value> {
        let mut request = ureq::post(&self.url)
            .set("Authorization", &self.bearer)
            .set("Accept", "application/json, text/event-stream")
            .set("Content-Type", "application/json");
        if let Some(session) = &self.session {
            request = request.set("Mcp-Session-Id", session);
        }
        let response = request.send_string(&message.to_string()).unwrap();
        if let Some(session) = response.header("Mcp-Session-Id") {
            self.session = Some(session.to_owned());
        }
        let body = response.into_string().unwrap();
        body.lines()
            .rev()
            .filter_map(|line| line.strip_prefix("data:"))
            .find(|payload| !payload.trim().is_empty())
            .map(|payload| serde_json::from_str(payload.trim()).unwrap())
    }

    fn tools(&mut self) -> Vec<String> {
        let reply = self
            .send(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .unwrap();
        reply["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tool| tool["name"].as_str().unwrap().to_owned())
            .collect()
    }
}

#[test]
fn a_catalogue_change_reaches_a_running_session_until_it_is_revoked() {
    let data = tempfile::tempdir().unwrap();
    let session = 910_001;
    let project = "/work/live";
    let mut catalog = Catalog::default();
    catalog.entries.push(Entry {
        id: "probe".into(),
        name: "Probe".into(),
        command: "true".into(),
        tools: Some(vec![json!({"name":"ping","inputSchema":{"type":"object"}})]),
        ..Entry::default()
    });
    catalog.save(data.path()).unwrap();
    tool_servers::configure(data.path());

    let mount = Mount {
        session,
        worktree: data.path().to_owned(),
        toolkits: BTreeSet::new(),
        project: project.to_owned(),
        ..Mount::default()
    };
    let access = Access::open(&mount).unwrap();
    tool_servers::refresh(&mount);
    let mut agent = Agent::connect(&access);
    let probe = "probe__ping".to_owned();
    assert!(!agent.tools().contains(&probe), "not approved yet");

    catalog.approve(data.path(), "probe").unwrap();
    catalog.save(data.path()).unwrap();
    tool_servers::sync_servers();
    assert!(agent.tools().contains(&probe), "turned on mid-session");

    let entry = catalog.entry_mut("probe").unwrap();
    entry.disabled_projects.insert(project.to_owned());
    catalog.save(data.path()).unwrap();
    tool_servers::sync_servers();
    assert!(
        !agent.tools().contains(&probe),
        "turned off in this project"
    );

    catalog
        .entry_mut("probe")
        .unwrap()
        .disabled_projects
        .clear();
    catalog.save(data.path()).unwrap();
    tool_servers::revoke(session);
    tool_servers::sync_servers();
    assert!(!access.is_live(session), "a revoked session stays revoked");
    let refused = ureq::post(&agent.url)
        .set("Authorization", &agent.bearer)
        .set("Accept", "application/json, text/event-stream")
        .set("Content-Type", "application/json")
        .send_string(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}).to_string());
    assert!(
        matches!(refused, Err(ureq::Error::Status(401 | 403, _))),
        "the old token still works"
    );
}
