//! The official MCP Registry. A listing only ever becomes a catalogue entry
//! that's off and unapproved: anyone can publish there, so what runs is what
//! the user read and approved, pinned to the listed version.
use crate::mcp::Importable;
use serde_json::Value;
use std::collections::BTreeMap;

const REGISTRY: &str = "https://registry.modelcontextprotocol.io/v0.1/servers";
const RESULTS: usize = 20;

/// A setting a listing asks for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Input {
    pub name: String,
    pub description: String,
    pub required: bool,
}

/// How one listed version would run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    Command { command: String, args: Vec<String> },
    Remote { url: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub name: String,
    pub description: String,
    pub version: String,
    pub install: Install,
    pub inputs: Vec<Input>,
}

impl Listing {
    /// The short name: `io.github.acme/search` is `search`.
    fn short_name(&self) -> &str {
        self.name.rsplit('/').next().unwrap_or(&self.name)
    }

    /// A catalogue entry-to-be. Required inputs are declared with no value, so
    /// it can't be approved until the user gives them.
    pub fn importable(&self) -> Importable {
        let (command, args, url) = match &self.install {
            Install::Command { command, args } => (command.clone(), args.clone(), None),
            Install::Remote { url } => (String::new(), Vec::new(), Some(url.clone())),
        };
        Importable {
            provider: "MCP Registry".into(),
            scope: self.version.clone(),
            name: self.short_name().to_owned(),
            command,
            args,
            env: BTreeMap::new(),
            url,
        }
    }
}

/// `npx`, `uvx` or `docker`, pinned to the listed version. Anything else is
/// left out: a prebuilt binary means a download nothing here has checked.
fn package_install(package: &Value) -> Option<Install> {
    if package["transport"]["type"].as_str().unwrap_or("stdio") != "stdio" {
        return None;
    }
    let identifier = package["identifier"].as_str()?;
    let version = package["version"].as_str();
    let (command, mut args) = match package["registryType"].as_str()? {
        "npm" => {
            let spec = version.map_or(identifier.to_owned(), |v| format!("{identifier}@{v}"));
            ("npx", vec!["-y".to_owned(), spec])
        }
        "pypi" => {
            let spec = version.map_or(identifier.to_owned(), |v| format!("{identifier}=={v}"));
            ("uvx", vec![spec])
        }
        "oci" => (
            "docker",
            vec![
                "run".to_owned(),
                "-i".to_owned(),
                "--rm".to_owned(),
                identifier.to_owned(),
            ],
        ),
        _ => return None,
    };
    args.extend(
        package["packageArguments"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|argument| argument["type"] == "positional")
            .filter_map(|argument| argument["value"].as_str().map(str::to_owned)),
    );
    Some(Install::Command {
        command: command.to_owned(),
        args,
    })
}

fn inputs(package: &Value) -> Vec<Input> {
    package["environmentVariables"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|variable| {
            Some(Input {
                name: variable["name"].as_str()?.to_owned(),
                description: variable["description"].as_str().unwrap_or("").to_owned(),
                required: variable["isRequired"] == true,
            })
        })
        .collect()
}

fn listing(server: &Value) -> Option<Listing> {
    let packaged = server["packages"]
        .as_array()
        .into_iter()
        .flatten()
        .find_map(|package| Some((package_install(package)?, inputs(package))));
    let remote = || {
        let url = server["remotes"]
            .as_array()?
            .iter()
            .find(|remote| remote["type"] == "streamable-http")?["url"]
            .as_str()?;
        // A templated address needs values this can't supply.
        (!url.contains('{')).then(|| Install::Remote {
            url: url.to_owned(),
        })
    };
    let (install, inputs) = packaged.or_else(|| Some((remote()?, Vec::new())))?;
    Some(Listing {
        name: server["name"].as_str()?.to_owned(),
        description: server["description"].as_str().unwrap_or("").to_owned(),
        version: server["version"].as_str().unwrap_or("").to_owned(),
        install,
        inputs,
    })
}

/// Reads one page of the registry's answer.
pub fn parse(body: &Value) -> Vec<Listing> {
    body["servers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| listing(&entry["server"]))
        .take(RESULTS)
        .collect()
}

/// Searches the registry. Blocking.
pub fn search(query: &str) -> Result<Vec<Listing>, String> {
    let limit = RESULTS.to_string();
    let response = ureq::get(REGISTRY)
        .query("search", query)
        .query("limit", &limit)
        .query("version", "latest")
        .timeout(std::time::Duration::from_secs(15))
        .call()
        .map_err(|_| "Could not reach the MCP Registry".to_owned())?;
    let body: Value = response
        .into_json()
        .map_err(|_| "The MCP Registry sent an unreadable answer".to_owned())?;
    Ok(parse(&body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn page(server: Value) -> Value {
        json!({"servers":[{"server":server,"_meta":{}}],"metadata":{"count":1}})
    }

    #[test]
    fn an_npm_package_runs_through_npx_pinned_to_its_version() {
        let listings = parse(&page(json!({
            "name":"com.pulsemcp/remote-filesystem",
            "description":"Files in a bucket",
            "version":"0.1.5",
            "packages":[{
                "registryType":"npm",
                "identifier":"remote-filesystem-mcp-server",
                "version":"0.1.5",
                "transport":{"type":"stdio"},
                "environmentVariables":[
                    {"name":"GCS_BUCKET","description":"Bucket","isRequired":true},
                    {"name":"LOG_LEVEL"},
                ],
            }],
        })));
        assert_eq!(
            listings[0].install,
            Install::Command {
                command: "npx".into(),
                args: vec!["-y".into(), "remote-filesystem-mcp-server@0.1.5".into()],
            }
        );
        assert_eq!(listings[0].inputs[0].name, "GCS_BUCKET");
        assert!(listings[0].inputs[0].required && !listings[0].inputs[1].required);
        let importable = listings[0].importable();
        assert_eq!(importable.name, "remote-filesystem");
        assert_eq!(importable.scope, "0.1.5");
    }

    #[test]
    fn pypi_and_oci_packages_have_their_own_runners() {
        let run = |registry: &str, identifier: &str| {
            parse(&page(json!({"name":"a/b","version":"2","packages":[
                {"registryType":registry,"identifier":identifier,"version":"2"}
            ]})))[0]
                .install
                .clone()
        };
        assert_eq!(
            run("pypi", "mcp-x"),
            Install::Command {
                command: "uvx".into(),
                args: vec!["mcp-x==2".into()]
            }
        );
        assert!(matches!(
            run("oci", "ghcr.io/a/b:2"),
            Install::Command { command, .. } if command == "docker"
        ));
    }

    #[test]
    fn a_remote_listing_keeps_its_address_and_a_templated_one_is_left_out() {
        let listing = |url: &str| {
            parse(&page(json!({"name":"a/b","version":"1","remotes":[
                {"type":"sse","url":"https://old.example/sse"},
                {"type":"streamable-http","url":url},
            ]})))
        };
        assert_eq!(
            listing("https://mcp.example/mcp")[0].install,
            Install::Remote {
                url: "https://mcp.example/mcp".into()
            }
        );
        assert!(listing("https://{tenant}.example/mcp").is_empty());
    }

    #[test]
    fn a_listing_with_no_way_to_run_it_safely_is_left_out() {
        for server in [
            json!({"name":"a/b","version":"1"}),
            json!({"name":"a/b","version":"1","packages":[{"registryType":"mcpb","identifier":"https://x/y.mcpb"}]}),
            json!({"name":"a/b","version":"1","packages":[{"registryType":"npm","identifier":"x","transport":{"type":"streamable-http"}}]}),
        ] {
            assert!(parse(&page(server.clone())).is_empty(), "{server}");
        }
    }

    #[test]
    fn a_junk_answer_lists_nothing() {
        assert!(parse(&json!(5)).is_empty());
        assert!(parse(&json!({"servers":[5,{"server":7}]})).is_empty());
    }
}
