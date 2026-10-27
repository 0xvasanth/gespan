//! The app's own stdio MCP servers, in the shape each provider expects. A
//! server is mounted only when the turn's environment carries its capability.
use serde_json::{json, Map, Value};
use std::collections::HashMap;

struct ToolServer {
    name: &'static str,
    subcommand: &'static str,
    /// Endpoint, token, session, executable.
    keys: [&'static str; 4],
}

const SERVERS: [ToolServer; 2] = [
    ToolServer {
        name: "gespan_visual",
        subcommand: "visual-mcp",
        keys: crate::visual_bridge::ENV_KEYS,
    },
    ToolServer {
        name: "gespan_ticket",
        subcommand: "ticket-mcp",
        keys: crate::ticket_bridge::ENV_KEYS,
    },
];

fn mounted(
    environment: &HashMap<String, String>,
) -> impl Iterator<Item = &'static ToolServer> + '_ {
    SERVERS
        .iter()
        .filter(|server| server.keys.iter().all(|key| environment.contains_key(*key)))
}

fn executable<'a>(server: &ToolServer, environment: &'a HashMap<String, String>) -> &'a str {
    &environment[server.keys[3]]
}

/// Providers keep emitting while this holds so the runtime can answer tool
/// calls the provider is blocked on.
pub fn configured(environment: &HashMap<String, String>) -> bool {
    mounted(environment).next().is_some()
}

/// Claude takes every server in one `--mcp-config` document.
pub fn mcp_servers(environment: &HashMap<String, String>) -> Map<String, Value> {
    mounted(environment)
        .map(|server| {
            (
                server.name.to_owned(),
                json!({"type":"stdio","command":executable(server, environment),"args":[server.subcommand]}),
            )
        })
        .collect()
}

pub fn acp_servers(environment: &HashMap<String, String>) -> Value {
    mounted(environment)
        .map(|server| {
            // ACP's stdio server shape omits the transport discriminator.
            json!({
                "name": server.name,
                "command": executable(server, environment),
                "args": [server.subcommand],
                "env": server
                    .keys
                    .iter()
                    .map(|key| json!({"name":key,"value":environment[*key]}))
                    .collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>()
        .into()
}

/// Codex reads the capability from its own environment by name, so the token
/// never appears in its arguments.
pub fn configure_codex(command: &mut std::process::Command, environment: &HashMap<String, String>) {
    for server in mounted(environment) {
        for (key, value) in [
            ("command", json!(executable(server, environment))),
            ("args", json!([server.subcommand])),
            ("env_vars", json!(server.keys)),
        ] {
            // JSON's strings and arrays are valid TOML literals for these values.
            command.args(["-c", &format!("mcp_servers.{}.{key}={value}", server.name)]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capability(keys: [&str; 4], executable: &str) -> HashMap<String, String> {
        keys.iter()
            .zip(["127.0.0.1:9", "secret-token", "7", executable])
            .map(|(key, value)| (key.to_string(), value.to_string()))
            .collect()
    }

    fn both() -> HashMap<String, String> {
        let mut environment = capability(crate::visual_bridge::ENV_KEYS, "/test/app");
        let mut ticket = capability(crate::ticket_bridge::ENV_KEYS, "/test/app");
        ticket.insert(
            crate::ticket_bridge::ENV_KEYS[1].into(),
            "ticket-token".into(),
        );
        environment.extend(ticket);
        environment
    }

    #[test]
    fn claude_config_lists_both_servers_when_configured() {
        let servers = mcp_servers(&both());
        assert_eq!(
            Value::Object(servers),
            json!({
                "gespan_visual": {"type":"stdio","command":"/test/app","args":["visual-mcp"]},
                "gespan_ticket": {"type":"stdio","command":"/test/app","args":["ticket-mcp"]},
            })
        );
        let visual_only = capability(crate::visual_bridge::ENV_KEYS, "/test/app");
        assert_eq!(
            mcp_servers(&visual_only).keys().collect::<Vec<_>>(),
            ["gespan_visual"]
        );
        assert!(configured(&visual_only));
        assert!(mcp_servers(&HashMap::new()).is_empty());
        assert!(!configured(&HashMap::new()));
        // A partial capability mounts nothing rather than a broken server.
        let mut partial = capability(crate::ticket_bridge::ENV_KEYS, "/test/app");
        partial.remove(crate::ticket_bridge::ENV_KEYS[1]);
        assert!(!configured(&partial));
    }

    #[test]
    fn codex_args_name_both_servers() {
        let environment = both();
        let mut command = std::process::Command::new("codex");
        configure_codex(&mut command, &environment);
        let arguments = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert!(arguments.contains(&r#"mcp_servers.gespan_visual.args=["visual-mcp"]"#.into()));
        assert!(arguments.contains(&r#"mcp_servers.gespan_ticket.args=["ticket-mcp"]"#.into()));
        assert!(arguments.contains(&r#"mcp_servers.gespan_ticket.command="/test/app""#.into()));
        assert!(arguments.contains(
            &r#"mcp_servers.gespan_ticket.env_vars=["GESPAN_TICKET_ENDPOINT","GESPAN_TICKET_TOKEN","GESPAN_TICKET_SESSION","GESPAN_TICKET_EXECUTABLE"]"#.into()
        ));
        let joined = arguments.join(" ");
        assert!(!joined.contains("secret-token"));
        assert!(!joined.contains("ticket-token"));
        let mut bare = std::process::Command::new("codex");
        configure_codex(&mut bare, &HashMap::new());
        assert_eq!(bare.get_args().count(), 0);
    }

    #[test]
    fn acp_lists_both() {
        let servers = acp_servers(&both());
        let servers = servers.as_array().unwrap();
        assert_eq!(servers.len(), 2);
        assert_eq!(servers[0]["name"], "gespan_visual");
        assert_eq!(servers[1]["name"], "gespan_ticket");
        assert_eq!(servers[1]["args"], json!(["ticket-mcp"]));
        assert!(servers[1].get("type").is_none());
        let env = servers[1]["env"].as_array().unwrap();
        assert_eq!(env.len(), 4);
        assert!(env.contains(&json!({"name":"GESPAN_TICKET_TOKEN","value":"ticket-token"})));
        assert_eq!(acp_servers(&HashMap::new()), json!([]));
    }
}
