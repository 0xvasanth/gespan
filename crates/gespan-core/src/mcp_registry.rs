//! The official MCP Registry. A listing only ever becomes a catalogue entry
//! that's off and unapproved: anyone can publish there, so what runs is what
//! the user read and approved, pinned to the listed version.
use crate::mcp::Importable;
use serde_json::Value;
use std::{collections::BTreeMap, io::Read};

const REGISTRY: &str = "https://registry.modelcontextprotocol.io/v0.1/servers";
const RESULTS: usize = 20;
/// Far more than a page of results. Anything bigger isn't one.
const MAX_ANSWER_BYTES: u64 = 4 * 1024 * 1024;

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

/// A listing's arguments as words. One it requires with no fixed value needs
/// something this can't supply; an optional one is left to its default.
fn arguments(list: &Value) -> Option<Vec<String>> {
    let mut words = Vec::new();
    for argument in list.as_array().into_iter().flatten() {
        let value = argument["value"]
            .as_str()
            .or(argument["default"].as_str())
            .filter(|value| !value.contains('{'));
        let Some(value) = value else {
            if argument["isRequired"] == true {
                return None;
            }
            continue;
        };
        if argument["type"] == "named" {
            words.push(argument["name"].as_str()?.to_owned());
        }
        words.push(value.to_owned());
    }
    Some(words)
}

/// An image name that says which image, by tag, digest or the listed version.
fn pinned_image(identifier: &str, version: Option<&str>) -> Option<String> {
    let last = identifier.rsplit('/').next().unwrap_or(identifier);
    if last.contains(':') || last.contains('@') {
        return Some(identifier.to_owned());
    }
    Some(format!("{identifier}:{}", version?))
}

/// `npx`, `uvx` or `docker`, pinned to the listed version. Anything else is
/// left out: a prebuilt binary means a download nothing here has checked, and
/// with no version what runs later may not be what the user approved.
fn package_install(package: &Value) -> Option<Install> {
    if package["transport"]["type"].as_str().unwrap_or("stdio") != "stdio" {
        return None;
    }
    let identifier = package["identifier"].as_str()?;
    let version = package["version"].as_str().filter(|v| !v.is_empty());
    let runtime = arguments(&package["runtimeArguments"])?;
    let (command, mut args) = match package["registryType"].as_str()? {
        "npm" => {
            let mut args = vec!["-y".to_owned()];
            args.extend(runtime);
            args.push(format!("{identifier}@{}", version?));
            ("npx", args)
        }
        "pypi" => {
            let mut args = runtime;
            args.push(format!("{identifier}=={}", version?));
            ("uvx", args)
        }
        "oci" => {
            let mut args = ["run", "-i", "--rm"].map(str::to_owned).to_vec();
            // A container sees only what it's handed by name.
            for input in inputs(package) {
                args.extend(["-e".to_owned(), input.name]);
            }
            args.extend(runtime);
            args.push(pinned_image(identifier, version)?);
            ("docker", args)
        }
        _ => return None,
    };
    args.extend(arguments(&package["packageArguments"])?);
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
        // A templated address needs values this can't supply, and anyone can
        // publish here, so a listing doesn't get to send traffic in the clear.
        let usable =
            !url.contains('{') && url::Url::parse(url).is_ok_and(|u| u.scheme() == "https");
        usable.then(|| Install::Remote {
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

fn read_answer(reader: impl Read) -> Result<Value, String> {
    serde_json::from_reader(reader.take(MAX_ANSWER_BYTES))
        .map_err(|_| "The MCP Registry sent an unreadable answer".to_owned())
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
    Ok(parse(&read_answer(response.into_reader())?))
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
            json!({"name":"a/b","version":"1","packages":[{"registryType":"npm","identifier":"x","version":"1","transport":{"type":"streamable-http"}}]}),
        ] {
            assert!(parse(&page(server.clone())).is_empty(), "{server}");
        }
    }

    fn package(fields: Value) -> Vec<Listing> {
        let mut package = json!({"registryType":"npm","identifier":"x","version":"1"});
        package
            .as_object_mut()
            .unwrap()
            .extend(fields.as_object().unwrap().clone());
        parse(&page(
            json!({"name":"a/b","version":"1","packages":[package]}),
        ))
    }

    fn command_of(listings: &[Listing]) -> Vec<String> {
        match &listings[0].install {
            Install::Command { command, args } => std::iter::once(command.clone())
                .chain(args.clone())
                .collect(),
            Install::Remote { .. } => panic!("not a command"),
        }
    }

    #[test]
    fn a_package_with_no_version_is_left_out_since_nothing_pins_it() {
        for registry in ["npm", "pypi"] {
            assert!(
                package(json!({"registryType":registry,"version":null})).is_empty(),
                "{registry}"
            );
        }
        let image = |identifier: &str, version: Value| {
            package(json!({"registryType":"oci","identifier":identifier,"version":version}))
        };
        assert!(image("ghcr.io/a/b", Value::Null).is_empty());
        assert_eq!(
            command_of(&image("ghcr.io/a/b", json!("2")))
                .last()
                .unwrap(),
            "ghcr.io/a/b:2"
        );
        assert_eq!(
            command_of(&image("ghcr.io/a/b:3", Value::Null))
                .last()
                .unwrap(),
            "ghcr.io/a/b:3"
        );
        assert_eq!(
            command_of(&image("host:5000/a/b@sha256:ab", Value::Null))
                .last()
                .unwrap(),
            "host:5000/a/b@sha256:ab"
        );
    }

    #[test]
    fn a_container_is_handed_its_settings_by_name() {
        let listings = package(json!({
            "registryType":"oci",
            "identifier":"ghcr.io/a/b:2",
            "environmentVariables":[{"name":"API_KEY","isRequired":true},{"name":"LOG_LEVEL"}],
        }));
        assert_eq!(
            command_of(&listings),
            [
                "docker",
                "run",
                "-i",
                "--rm",
                "-e",
                "API_KEY",
                "-e",
                "LOG_LEVEL",
                "ghcr.io/a/b:2"
            ]
        );
    }

    #[test]
    fn runtime_and_package_arguments_land_either_side_of_the_package() {
        let arguments = json!({
            "runtimeArguments":[{"type":"named","name":"--cache","value":"/tmp/c"}],
            "packageArguments":[
                {"type":"positional","value":"serve"},
                {"type":"named","name":"--mode","value":"stdio"},
                {"type":"named","name":"--level","default":"info"},
                {"type":"named","name":"--optional"},
            ],
        });
        assert_eq!(
            command_of(&package(arguments.clone())),
            [
                "npx", "-y", "--cache", "/tmp/c", "x@1", "serve", "--mode", "stdio", "--level",
                "info"
            ]
        );
        let mut arguments = arguments;
        arguments["registryType"] = json!("pypi");
        assert_eq!(
            command_of(&package(arguments)),
            ["uvx", "--cache", "/tmp/c", "x==1", "serve", "--mode", "stdio", "--level", "info"]
        );
    }

    #[test]
    fn a_required_argument_with_no_fixed_value_leaves_the_listing_out() {
        for argument in [
            json!({"type":"positional","valueHint":"path","isRequired":true}),
            json!({"type":"named","name":"--token","isRequired":true}),
            json!({"type":"named","name":"--token","value":"{token}","isRequired":true}),
        ] {
            assert!(
                package(json!({"packageArguments":[argument.clone()]})).is_empty(),
                "{argument}"
            );
        }
    }

    #[test]
    fn a_remote_listing_must_use_tls() {
        let listing = |url: &str| {
            parse(&page(json!({"name":"a/b","version":"1","remotes":[
                {"type":"streamable-http","url":url},
            ]})))
        };
        assert!(listing("http://mcp.example/mcp").is_empty());
        assert!(
            listing("http://127.0.0.1:9/mcp").is_empty(),
            "a listing is never local"
        );
        assert_eq!(listing("https://mcp.example/mcp").len(), 1);
    }

    #[test]
    fn an_answer_too_big_to_be_a_page_of_results_is_refused() {
        let padding = "x".repeat(MAX_ANSWER_BYTES as usize);
        let big = json!({"servers":[],"padding":padding}).to_string();
        assert!(read_answer(big.as_bytes()).is_err());
        assert!(read_answer(&br#"{"servers":[]}"#[..]).is_ok());
    }

    #[test]
    fn a_junk_answer_lists_nothing() {
        assert!(parse(&json!(5)).is_empty());
        assert!(parse(&json!({"servers":[5,{"server":7}]})).is_empty());
    }
}
