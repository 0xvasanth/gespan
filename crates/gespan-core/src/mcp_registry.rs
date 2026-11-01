//! The official MCP Registry. A listing only ever becomes a catalogue entry
//! that's off and unapproved: anyone can publish there, so what runs is what
//! the user read and approved, pinned to the listed version.
use crate::{mcp::Importable, mcp_catalog::env_name};
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

/// Text from a listing that means only itself. On Windows these commands go
/// through `cmd.exe`, where the rest would be read as syntax.
fn plain(word: &str) -> bool {
    !word.contains(['&', '|', '<', '>', '^', '%', '!', '"', '(', ')', '\n', '\r'])
}

fn name_part(part: &str) -> bool {
    part.starts_with(|c: char| c.is_ascii_alphanumeric())
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

/// An npm name, scoped or not.
fn npm_name(identifier: &str) -> bool {
    match identifier.strip_prefix('@') {
        Some(scoped) => scoped
            .split_once('/')
            .is_some_and(|(scope, name)| name_part(scope) && name_part(name)),
        None => name_part(identifier),
    }
}

/// `1` is a range to npm, so it takes all three numbers.
fn npm_version(version: &str) -> bool {
    let core = version.split(['-', '+']).next().unwrap_or(version);
    let numbers: Vec<_> = core.split('.').collect();
    numbers.len() == 3
        && numbers
            .iter()
            .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".+-".contains(c))
}

/// `==` makes any version exact here, as long as it's a version at all.
fn pypi_version(version: &str) -> bool {
    version.starts_with(|c: char| c.is_ascii_digit())
        && version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || ".+-_".contains(c))
        && !version
            .split('.')
            .any(|part| part.eq_ignore_ascii_case("x"))
}

/// An image reference with no tag or digest on it.
fn image_name(name: &str) -> bool {
    !name.starts_with('-')
        && !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "/._-:".contains(c))
}

/// A tag that names a release rather than whatever's newest, like `main`.
fn release_tag(tag: &str) -> bool {
    tag.strip_prefix('v')
        .unwrap_or(tag)
        .starts_with(|c: char| c.is_ascii_digit())
        && tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn digest(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(|hex| hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit()))
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
    words.iter().all(|word| plain(word)).then_some(words)
}

/// An image reference that says which image, by digest, a release tag or the
/// listed version.
fn pinned_image(identifier: &str, version: Option<&str>) -> Option<String> {
    if let Some((name, value)) = identifier.split_once('@') {
        return (image_name(name) && digest(value)).then(|| identifier.to_owned());
    }
    if !image_name(identifier) {
        return None;
    }
    let last = identifier.rsplit('/').next().unwrap_or(identifier);
    match last.split_once(':') {
        Some((_, tag)) => release_tag(tag).then(|| identifier.to_owned()),
        None => {
            let version = version.filter(|version| release_tag(version))?;
            Some(format!("{identifier}:{version}"))
        }
    }
}

/// `npx`, `uvx` or `docker`, pinned to the listed version. Anything else is
/// left out: a prebuilt binary means a download nothing here has checked, and
/// with no exact version what runs later may not be what the user approved.
/// So is a listing with flags for the runner itself, which could fetch another
/// package than the one named, or take a container out of its box.
fn package_install(package: &Value) -> Option<Install> {
    if package["transport"]["type"].as_str().unwrap_or("stdio") != "stdio" {
        return None;
    }
    let identifier = package["identifier"].as_str()?;
    let version = package["version"].as_str();
    if !arguments(&package["runtimeArguments"])?.is_empty() {
        return None;
    }
    if !inputs(package).iter().all(|input| env_name(&input.name)) {
        return None;
    }
    let (command, mut args) = match package["registryType"].as_str()? {
        "npm" => {
            let version = version.filter(|version| npm_version(version))?;
            npm_name(identifier).then_some(())?;
            (
                "npx",
                vec!["-y".to_owned(), format!("{identifier}@{version}")],
            )
        }
        "pypi" => {
            let version = version.filter(|version| pypi_version(version))?;
            name_part(identifier).then_some(())?;
            ("uvx", vec![format!("{identifier}=={version}")])
        }
        "oci" => {
            let mut args = crate::mcp_catalog::CONTAINER_RUN
                .map(str::to_owned)
                .to_vec();
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

/// Whether an address is https and doesn't name a local or private host. A name
/// can still resolve to one; the user sees and approves the address itself.
fn public_https(url: &str) -> bool {
    let Ok(url) = url::Url::parse(url) else {
        return false;
    };
    let private_v4 = |ip: std::net::Ipv4Addr| {
        ip.is_loopback() || ip.is_private() || ip.is_link_local() || ip.is_unspecified()
    };
    url.scheme() == "https"
        && match url.host() {
            Some(url::Host::Domain(name)) => {
                let name = name.trim_end_matches('.').to_lowercase();
                !(name == "localhost" || name.ends_with(".localhost") || name.ends_with(".local"))
            }
            Some(url::Host::Ipv4(ip)) => !private_v4(ip),
            Some(url::Host::Ipv6(ip)) => {
                let first = ip.segments()[0];
                !(ip.is_loopback()
                    || ip.is_unspecified()
                    || first & 0xfe00 == 0xfc00
                    || first & 0xffc0 == 0xfe80
                    || ip.to_ipv4_mapped().is_some_and(private_v4))
            }
            None => false,
        }
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
        // publish here, so a listing doesn't get to send traffic in the clear
        // or point at this machine or its network.
        let usable = !url.contains('{') && public_https(url);
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
    let unreadable = || "The MCP Registry sent an unreadable answer".to_owned();
    let mut bytes = Vec::new();
    reader
        .take(MAX_ANSWER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| unreadable())?;
    if bytes.len() as u64 > MAX_ANSWER_BYTES {
        return Err(unreadable());
    }
    serde_json::from_slice(&bytes).map_err(|_| unreadable())
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
        let mut package = json!({"registryType":"npm","identifier":"x","version":"1.0.0"});
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
            command_of(&image("ghcr.io/a/b:v3.1", Value::Null))
                .last()
                .unwrap(),
            "ghcr.io/a/b:v3.1"
        );
        let digest = format!("host:5000/a/b@sha256:{}", "ab".repeat(32));
        assert_eq!(
            command_of(&image(&digest, Value::Null)).last().unwrap(),
            &digest
        );
    }

    #[test]
    fn package_arguments_follow_the_package() {
        let arguments = json!({
            "packageArguments":[
                {"type":"positional","value":"serve"},
                {"type":"named","name":"--mode","value":"stdio"},
                {"type":"named","name":"--level","default":"info"},
                {"type":"named","name":"--optional"},
            ],
        });
        assert_eq!(
            command_of(&package(arguments.clone())),
            ["npx", "-y", "x@1.0.0", "serve", "--mode", "stdio", "--level", "info"]
        );
        let mut arguments = arguments;
        arguments["registryType"] = json!("pypi");
        assert_eq!(
            command_of(&package(arguments)),
            ["uvx", "x==1.0.0", "serve", "--mode", "stdio", "--level", "info"]
        );
    }

    #[test]
    fn a_listing_cant_give_the_runner_flags_of_its_own() {
        for (registry, identifier, name, value) in [
            ("pypi", "x", "--index-url", "https://evil.example/simple"),
            ("npm", "x", "--registry", "https://evil.example"),
            ("oci", "ghcr.io/a/b:2", "--privileged", "true"),
            ("oci", "ghcr.io/a/b:2", "-v", "/:/host"),
        ] {
            let listings = package(json!({
                "registryType":registry,
                "identifier":identifier,
                "runtimeArguments":[{"type":"named","name":name,"value":value}],
            }));
            assert!(listings.is_empty(), "{name}");
        }
        let unused = json!({"runtimeArguments":[{"type":"named","name":"--cache"}]});
        assert_eq!(package(unused).len(), 1, "nothing would reach the runner");
    }

    #[test]
    fn each_registry_has_its_own_names() {
        for identifier in [
            "--from=evil",
            "-x",
            "",
            "a b",
            "a;b",
            "a&calc",
            "git+https://x",
            "foo@npm:evil",
            "file:../x",
            "a/b",
        ] {
            assert!(
                package(json!({"identifier":identifier})).is_empty(),
                "npm {identifier}"
            );
        }
        assert_eq!(package(json!({"identifier":"@scope/pkg.name_1"})).len(), 1);
        for identifier in ["--with=evil", "a/b", "a@b", "a:b", "@a/b", "https://x"] {
            assert!(
                package(json!({"registryType":"pypi","identifier":identifier})).is_empty(),
                "pypi {identifier}"
            );
        }
        for identifier in ["--net=host", "a b:1", "a;b:1"] {
            assert!(
                package(json!({"registryType":"oci","identifier":identifier})).is_empty(),
                "oci {identifier}"
            );
        }
    }

    #[test]
    fn a_setting_name_a_shell_could_read_leaves_the_listing_out() {
        let listing = |name: &str| package(json!({"environmentVariables":[{"name":name}]}));
        assert_eq!(listing("API_KEY").len(), 1);
        for name in ["A&calc", "A B", "1A", ""] {
            assert!(listing(name).is_empty(), "{name}");
        }
    }

    #[test]
    fn text_cmd_would_read_as_syntax_is_left_out() {
        for value in [
            "a&calc", "a|b", "%PATH%", "a\"b", "!x!", "(a)", "a>b", "a\nb",
        ] {
            let positional = json!({"packageArguments":[{"type":"positional","value":value}]});
            assert!(package(positional).is_empty(), "{value}");
        }
        let spaced = json!({"packageArguments":[{"type":"positional","value":"a b=c"}]});
        assert_eq!(package(spaced).len(), 1);
    }

    #[test]
    fn only_an_exact_version_counts_as_pinned() {
        for version in [
            "latest", "^1.2", "~1", ">=1", "1", "1.2", "1.x", "1.2.x", "*", "", "1 || 2",
        ] {
            assert!(
                package(json!({"version":version})).is_empty(),
                "npm {version}"
            );
        }
        for version in ["1.2.3", "1.2.3-beta.1", "1.0.0+build"] {
            assert_eq!(
                package(json!({"version":version})).len(),
                1,
                "npm {version}"
            );
        }
        let pypi = |version: &str| package(json!({"registryType":"pypi","version":version}));
        for version in ["latest", "1.*", "1.x", "", ">=1"] {
            assert!(pypi(version).is_empty(), "pypi {version}");
        }
        for version in ["1", "2024.1", "1.2.3rc1"] {
            assert_eq!(pypi(version).len(), 1, "pypi {version}");
        }
        let image = |identifier: &str| {
            package(json!({"registryType":"oci","identifier":identifier,"version":null}))
        };
        for identifier in [
            "ghcr.io/a/b:latest",
            "ghcr.io/a/b:",
            "ghcr.io/a/b:main",
            "ghcr.io/a/b@sha256:ab",
            "ghcr.io/a/b@md5:ab",
        ] {
            assert!(image(identifier).is_empty(), "{identifier}");
        }
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
        for local in [
            "http://127.0.0.1:9/mcp",
            "https://localhost/mcp",
            "https://app.localhost/mcp",
            "https://127.0.0.1/mcp",
            "https://10.0.0.5/mcp",
            "https://192.168.1.1/mcp",
            "https://169.254.169.254/latest",
            "https://[::1]/mcp",
            "https://[fd00::1]/mcp",
            "https://[::ffff:10.0.0.5]/mcp",
        ] {
            assert!(listing(local).is_empty(), "{local}");
        }
        assert_eq!(listing("https://mcp.example/mcp").len(), 1);
    }

    #[test]
    fn an_answer_too_big_to_be_a_page_of_results_is_refused() {
        let padding = "x".repeat(MAX_ANSWER_BYTES as usize);
        let big = json!({"servers":[],"padding":padding}).to_string();
        assert!(read_answer(big.as_bytes()).is_err());
        assert!(read_answer(&br#"{"servers":[]}"#[..]).is_ok());
        let mut trailing = br#"{"servers":[]}"#.to_vec();
        trailing.resize(MAX_ANSWER_BYTES as usize + 1, b' ');
        assert!(
            read_answer(&trailing[..]).is_err(),
            "it ends just past the limit"
        );
    }

    #[test]
    fn a_junk_answer_lists_nothing() {
        assert!(parse(&json!(5)).is_empty());
        assert!(parse(&json!({"servers":[5,{"server":7}]})).is_empty());
    }
}
