//! Gespan's own list of MCP servers: installed once here and used by every
//! agent through the hub. Imported from an agent's config once, never written
//! back to it. Env values live in the OS keychain, and nothing runs until the
//! user has approved the exact command and environment.
use crate::{mcp::Importable, mcp_registry::Listing};
use mcp_hub::{ServerSpec, Sharing, TokenStore};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

const FILE: &str = "mcp-catalog.json";
/// Prefix of a server's keychain name.
const SECRETS: &str = "mcp-server";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Class {
    Shared,
    #[default]
    PerWorktree,
    PerSession,
}

impl From<Class> for Sharing {
    fn from(class: Class) -> Self {
        match class {
            Class::Shared => Sharing::Shared,
            Class::PerWorktree => Sharing::PerWorkspace,
            Class::PerSession => Sharing::PerSession,
        }
    }
}

/// One server the user has in Gespan. Secrets are not here, only their names.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Entry {
    pub id: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    /// Environment names; the values are in the keychain.
    pub env: Vec<String>,
    /// A remote server's address, in place of a command.
    pub url: Option<String>,
    pub class: Class,
    pub enabled: bool,
    /// Projects this server is turned off in.
    pub disabled_projects: BTreeSet<String>,
    /// Fingerprint of the exact command and environment the user approved.
    pub approved: Option<String>,
    /// Settings it can't run without and doesn't have yet.
    pub needed: Vec<String>,
    /// Where it was imported from, for showing the user.
    pub source: Option<String>,
    pub tools: Option<Vec<Value>>,
}

impl Entry {
    fn spec(&self, env: BTreeMap<String, String>) -> ServerSpec {
        ServerSpec {
            id: self.id.clone(),
            // A remote server's process is a connection, so its address and
            // headers take the place of a command and its environment.
            command: self.url.clone().unwrap_or_else(|| self.command.clone()),
            args: self.args.clone(),
            env,
            // One sign-in means one set of credentials, so nothing is gained
            // by keeping connections apart.
            sharing: if self.url.is_some() {
                Sharing::Shared
            } else {
                self.class.into()
            },
            tools: self.tools.clone(),
        }
    }

    /// The command line as the approval card shows it.
    pub fn command_line(&self) -> String {
        if let Some(url) = &self.url {
            return url.clone();
        }
        std::iter::once(self.command.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Catalog {
    /// Set once the user moves their servers here. From then on agents
    /// launched by Gespan no longer load their own copies.
    pub managed: bool,
    pub entries: Vec<Entry>,
}

fn path(data_dir: &Path) -> PathBuf {
    data_dir.join(FILE)
}

/// Servers named after what they do, by how they must be shared. Anything else
/// gets the safe default of one process per worktree.
fn class_for(name: &str, command: &str, args: &[String]) -> Class {
    let text = format!("{name} {command} {}", args.join(" ")).to_lowercase();
    if ["playwright", "puppeteer", "browser", "selenium", "repl"]
        .iter()
        .any(|word| text.contains(word))
    {
        Class::PerSession
    } else if ["context7", "fetch", "exa", "brave-search", "tavily", "docs"]
        .iter()
        .any(|word| text.contains(word))
    {
        Class::Shared
    } else {
        Class::PerWorktree
    }
}

/// Lowercase letters, digits and single hyphens: it prefixes every tool name,
/// and agents cap those lengths.
fn id_for(name: &str, taken: &BTreeSet<String>) -> String {
    let mut base = String::new();
    for character in name.to_lowercase().chars() {
        if character.is_ascii_alphanumeric() {
            base.push(character);
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    let base: String = base.trim_end_matches('-').chars().take(20).collect();
    let base = if base.is_empty() {
        "server".into()
    } else {
        base
    };
    let mut id = base.clone();
    let mut next = 2;
    while taken.contains(&id) {
        id = format!("{base}-{next}");
        next += 1;
    }
    id
}

impl Catalog {
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        match std::fs::read(path(data_dir)) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| e.to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn save(&self, data_dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(data_dir).map_err(|e| e.to_string())?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let mut file = tempfile::NamedTempFile::new_in(data_dir).map_err(|e| e.to_string())?;
        std::io::Write::write_all(&mut file, &bytes).map_err(|e| e.to_string())?;
        file.persist(path(data_dir)).map_err(|e| e.to_string())?;
        Ok(())
    }

    fn ids(&self) -> BTreeSet<String> {
        self.entries.iter().map(|entry| entry.id.clone()).collect()
    }

    pub fn entry_mut(&mut self, id: &str) -> Option<&mut Entry> {
        self.entries.iter_mut().find(|entry| entry.id == id)
    }

    /// Takes in what an agent's config held. Each server lands disabled and
    /// unapproved, its env in the keychain, and the agent's file is left as it
    /// was. A server already here, or seen under another agent, is skipped.
    /// Returns how many were added.
    pub fn import(&mut self, data_dir: &Path, found: Vec<Importable>) -> Result<usize, String> {
        let mut added = 0;
        for server in found {
            let same = |entry: &Entry| match &server.url {
                Some(url) => entry.url.as_deref() == Some(url),
                None => entry.command == server.command && entry.args == server.args,
            };
            if self.entries.iter().any(same) {
                continue;
            }
            let id = id_for(&server.name, &self.ids());
            write_secrets(data_dir, &id, &server.env)?;
            self.entries.push(Entry {
                class: class_for(&server.name, &server.command, &server.args),
                id,
                name: server.name,
                command: server.command,
                args: server.args,
                env: server.env.into_keys().collect(),
                url: server.url,
                source: Some(format!("{} ({})", server.provider, server.scope)),
                ..Entry::default()
            });
            added += 1;
        }
        Ok(added)
    }

    /// Adds a registry listing, off and unapproved. Its required settings are
    /// named with no value, so it can't be approved until they're given.
    pub fn install(&mut self, data_dir: &Path, listing: &Listing) -> Result<String, String> {
        let before = self.entries.len();
        self.import(data_dir, vec![listing.importable()])?;
        let Some(entry) = self.entries.get_mut(before) else {
            return Err("That server is already here".into());
        };
        entry.source = Some(format!("MCP Registry {}", listing.version));
        entry.needed = listing
            .inputs
            .iter()
            .filter(|input| input.required)
            .map(|input| input.name.clone())
            .collect();
        Ok(entry.id.clone())
    }

    /// Sets a server's env values in the keychain.
    pub fn set_secrets(
        &mut self,
        data_dir: &Path,
        id: &str,
        values: BTreeMap<String, String>,
    ) -> Result<(), String> {
        let entry = self.entry_mut(id).ok_or("Unknown server")?;
        entry.env = values.keys().cloned().collect();
        entry.needed.retain(|name| !values.contains_key(name));
        // The env changed, so what was approved no longer describes it.
        entry.approved = None;
        entry.enabled = false;
        write_secrets(data_dir, id, &values)
    }

    /// What the user is approving: the entry with its secrets resolved.
    fn resolved(&self, data_dir: &Path, entry: &Entry) -> Result<ServerSpec, String> {
        Ok(entry.spec(read_secrets(data_dir, entry)?))
    }

    /// Approves exactly what would run now, and turns it on.
    pub fn approve(&mut self, data_dir: &Path, id: &str) -> Result<(), String> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.id == id)
            .ok_or("Unknown server")?;
        if !entry.needed.is_empty() {
            return Err(format!("Set {} first", entry.needed.join(", ")));
        }
        let fingerprint = self.resolved(data_dir, entry)?.fingerprint();
        let entry = self.entry_mut(id).ok_or("Unknown server")?;
        entry.approved = Some(fingerprint);
        entry.enabled = true;
        Ok(())
    }

    /// Servers the hub may run. One edited since its approval, by hand or by
    /// anything else, is left out until it's approved again.
    pub fn specs(&self, data_dir: &Path) -> Vec<ServerSpec> {
        self.entries
            .iter()
            .filter(|entry| entry.enabled)
            .filter_map(|entry| {
                let spec = self
                    .resolved(data_dir, entry)
                    .inspect_err(|error| tracing::warn!(%error, "an MCP server's secrets"))
                    .ok()?;
                (entry.approved.as_deref() == Some(spec.fingerprint().as_str())).then_some(spec)
            })
            .collect()
    }

    /// Ids a session in this project may reach.
    pub fn for_project(&self, project: &str) -> BTreeSet<String> {
        self.entries
            .iter()
            .filter(|entry| entry.enabled && !entry.disabled_projects.contains(project))
            .map(|entry| entry.id.clone())
            .collect()
    }
}

fn secret_name(id: &str) -> String {
    format!("{SECRETS}:{id}")
}

fn write_secrets(data_dir: &Path, id: &str, env: &BTreeMap<String, String>) -> Result<(), String> {
    if env.is_empty() {
        return Ok(());
    }
    let value = serde_json::to_string(env).map_err(|e| e.to_string())?;
    gespan_tracker_secrets::write_named(&secret_name(id), data_dir, &value)
}

fn read_secrets(data_dir: &Path, entry: &Entry) -> Result<BTreeMap<String, String>, String> {
    if entry.env.is_empty() {
        return Ok(BTreeMap::new());
    }
    let value = gespan_tracker_secrets::read_named(&secret_name(&entry.id), data_dir)?
        .ok_or("This server's secrets are missing from the keychain")?;
    serde_json::from_str(&value).map_err(|e| e.to_string())
}

/// Removes a server, its secrets and any sign-in.
pub fn remove(catalog: &mut Catalog, data_dir: &Path, id: &str) -> Result<(), String> {
    let url = catalog
        .entries
        .iter()
        .find(|entry| entry.id == id)
        .and_then(|entry| entry.url.clone());
    catalog.entries.retain(|entry| entry.id != id);
    if let Some(url) = url {
        SignIn::new(data_dir, id, &url).clear()?;
    }
    gespan_tracker_secrets::delete_named(&secret_name(id), data_dir)
}

/// A remote server's OAuth credentials, in the keychain. Named after the exact
/// address they were issued for, so a changed address finds nothing to send, and
/// no agent ever receives them.
pub struct SignIn {
    name: String,
    data_dir: PathBuf,
}

impl SignIn {
    pub fn new(data_dir: &Path, id: &str, url: &str) -> Self {
        let address: String = Sha256::digest(url.as_bytes())
            .iter()
            .take(16)
            .map(|byte| format!("{byte:02x}"))
            .collect();
        Self {
            name: format!("mcp-sign-in:{id}:{address}"),
            data_dir: data_dir.to_owned(),
        }
    }

    pub fn exists(&self) -> bool {
        self.load().is_ok_and(|value| value.is_some())
    }
}

impl mcp_hub::TokenStore for SignIn {
    fn load(&self) -> Result<Option<String>, String> {
        gespan_tracker_secrets::read_named(&self.name, &self.data_dir)
    }

    fn save(&self, value: &str) -> Result<(), String> {
        gespan_tracker_secrets::write_named(&self.name, &self.data_dir, value)
    }

    fn clear(&self) -> Result<(), String> {
        gespan_tracker_secrets::delete_named(&self.name, &self.data_dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn found(name: &str, command: &str, env: &[(&str, &str)]) -> Importable {
        Importable {
            provider: "claude".into(),
            scope: "user".into(),
            name: name.into(),
            command: command.into(),
            args: vec!["-y".into(), "server".into()],
            env: env
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
            url: None,
        }
    }

    fn setup() -> tempfile::TempDir {
        gespan_tracker_secrets::testing::use_memory_keychain();
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn import_lands_off_unapproved_and_keeps_secrets_out_of_the_file() {
        let dir = setup();
        let mut catalog = Catalog::default();
        let added = catalog
            .import(
                dir.path(),
                vec![found("GitHub Tools", "npx", &[("TOKEN", "s3cret-value")])],
            )
            .unwrap();
        assert_eq!(added, 1);
        catalog.save(dir.path()).unwrap();
        let saved = std::fs::read_to_string(path(dir.path())).unwrap();
        assert!(!saved.contains("s3cret-value"), "{saved}");
        let entry = &Catalog::load(dir.path()).unwrap().entries[0];
        assert_eq!(entry.id, "github-tools");
        assert!(!entry.enabled && entry.approved.is_none());
        assert_eq!(entry.env, ["TOKEN"]);
        assert_eq!(entry.source.as_deref(), Some("claude (user)"));
        assert!(catalog.specs(dir.path()).is_empty(), "nothing runs yet");
    }

    #[test]
    fn the_same_server_from_two_agents_is_imported_once() {
        let dir = setup();
        let mut catalog = Catalog::default();
        let added = catalog
            .import(
                dir.path(),
                vec![found("a", "npx", &[]), found("b", "npx", &[])],
            )
            .unwrap();
        assert_eq!(added, 1);
        assert_eq!(
            catalog
                .import(dir.path(), vec![found("a", "npx", &[])])
                .unwrap(),
            0
        );
    }

    #[test]
    fn an_approved_server_runs_with_its_secrets_and_a_changed_one_does_not() {
        let dir = setup();
        let mut catalog = Catalog::default();
        catalog
            .import(dir.path(), vec![found("docs", "npx", &[("TOKEN", "one")])])
            .unwrap();
        catalog.approve(dir.path(), "docs").unwrap();
        let specs = catalog.specs(dir.path());
        assert_eq!(specs[0].env["TOKEN"], "one");

        // A command changed behind the user's back no longer runs.
        catalog
            .entry_mut("docs")
            .unwrap()
            .args
            .push("--evil".into());
        assert!(catalog.specs(dir.path()).is_empty());
        catalog.approve(dir.path(), "docs").unwrap();
        assert_eq!(catalog.specs(dir.path()).len(), 1);

        // So does a changed secret.
        write_secrets(
            dir.path(),
            "docs",
            &BTreeMap::from([("TOKEN".to_owned(), "two".to_owned())]),
        )
        .unwrap();
        assert!(catalog.specs(dir.path()).is_empty());
    }

    #[test]
    fn a_disabled_server_is_not_offered_and_projects_can_opt_out() {
        let dir = setup();
        let mut catalog = Catalog::default();
        catalog
            .import(dir.path(), vec![found("docs", "npx", &[])])
            .unwrap();
        catalog.approve(dir.path(), "docs").unwrap();
        let entry = catalog.entry_mut("docs").unwrap();
        entry.disabled_projects.insert("/work/quiet".into());
        assert!(catalog.for_project("/work/quiet").is_empty());
        assert_eq!(catalog.for_project("/work/busy").len(), 1);
        catalog.entry_mut("docs").unwrap().enabled = false;
        assert!(catalog.for_project("/work/busy").is_empty());
        assert!(catalog.specs(dir.path()).is_empty());
    }

    #[test]
    fn ids_are_short_clean_and_unique() {
        let taken = BTreeSet::from(["docs".to_owned()]);
        assert_eq!(id_for("Docs", &taken), "docs-2");
        assert_eq!(id_for("a__b  c", &BTreeSet::new()), "a-b-c");
        assert_eq!(id_for("***", &BTreeSet::new()), "server");
        assert!(id_for(&"x".repeat(80), &BTreeSet::new()).len() <= 20);
    }

    #[test]
    fn known_servers_get_the_class_they_need_and_unknown_ones_the_safe_default() {
        let none: &[String] = &[];
        assert_eq!(class_for("playwright", "npx", none), Class::PerSession);
        assert_eq!(class_for("context7", "npx", none), Class::Shared);
        assert_eq!(class_for("filesystem", "npx", none), Class::PerWorktree);
    }

    #[test]
    fn removing_a_server_takes_its_secrets_too() {
        let dir = setup();
        let mut catalog = Catalog::default();
        catalog
            .import(dir.path(), vec![found("docs", "npx", &[("TOKEN", "one")])])
            .unwrap();
        remove(&mut catalog, dir.path(), "docs").unwrap();
        assert!(catalog.entries.is_empty());
        assert_eq!(
            gespan_tracker_secrets::read_named("mcp-server:docs", dir.path()).unwrap(),
            None
        );
    }

    #[test]
    fn a_remote_server_is_shared_approved_by_its_address_and_keeps_headers_secret() {
        let dir = setup();
        let mut catalog = Catalog::default();
        let mut server = found("Linear", "", &[("Authorization", "Bearer s3cret")]);
        server.url = Some("https://mcp.example/mcp".into());
        server.args.clear();
        catalog.import(dir.path(), vec![server]).unwrap();
        catalog.entries[0].class = Class::PerSession;
        catalog.approve(dir.path(), "linear").unwrap();
        catalog.save(dir.path()).unwrap();
        let saved = std::fs::read_to_string(path(dir.path())).unwrap();
        assert!(!saved.contains("s3cret"), "{saved}");
        let specs = catalog.specs(dir.path());
        assert_eq!(specs[0].command, "https://mcp.example/mcp");
        assert_eq!(specs[0].sharing, Sharing::Shared);
        assert_eq!(specs[0].env["Authorization"], "Bearer s3cret");
        assert_eq!(catalog.entries[0].command_line(), "https://mcp.example/mcp");
        catalog.entry_mut("linear").unwrap().url = Some("https://evil.example/mcp".into());
        assert!(
            catalog.specs(dir.path()).is_empty(),
            "a new address needs approval"
        );
    }

    #[test]
    fn a_sign_in_lives_in_the_keychain_and_goes_with_the_server() {
        let dir = setup();
        let mut catalog = Catalog::default();
        let url = "https://mcp.example/mcp";
        let sign_in = SignIn::new(dir.path(), "docs", url);
        assert!(!sign_in.exists());
        sign_in.save("{\"token\":1}").unwrap();
        assert!(sign_in.exists());
        assert!(
            !SignIn::new(dir.path(), "docs", "https://evil.example/mcp").exists(),
            "a sign-in belongs to one address"
        );
        assert!(!std::fs::read_dir(dir.path()).unwrap().any(|file| {
            std::fs::read_to_string(file.unwrap().path()).is_ok_and(|text| text.contains("token"))
        }));
        catalog.entries.push(Entry {
            id: "docs".into(),
            url: Some(url.into()),
            ..Entry::default()
        });
        remove(&mut catalog, dir.path(), "docs").unwrap();
        assert!(!sign_in.exists());
    }

    fn listing() -> Listing {
        Listing {
            name: "com.acme/search".into(),
            description: String::new(),
            version: "1.2.0".into(),
            install: crate::mcp_registry::Install::Command {
                command: "npx".into(),
                args: vec!["-y".into(), "acme@1.2.0".into()],
            },
            inputs: vec![crate::mcp_registry::Input {
                name: "API_KEY".into(),
                description: String::new(),
                required: true,
            }],
        }
    }

    #[test]
    fn a_registry_server_lands_off_and_needs_its_settings_before_approval() {
        let dir = setup();
        let mut catalog = Catalog::default();
        let id = catalog.install(dir.path(), &listing()).unwrap();
        assert_eq!(id, "search");
        let entry = &catalog.entries[0];
        assert!(!entry.enabled && entry.approved.is_none());
        assert_eq!(entry.source.as_deref(), Some("MCP Registry 1.2.0"));
        assert_eq!(
            catalog.approve(dir.path(), &id).unwrap_err(),
            "Set API_KEY first"
        );
        let values = BTreeMap::from([("API_KEY".to_owned(), "k".to_owned())]);
        catalog.set_secrets(dir.path(), &id, values).unwrap();
        catalog.approve(dir.path(), &id).unwrap();
        assert_eq!(catalog.specs(dir.path())[0].env["API_KEY"], "k");
        assert!(
            catalog.install(dir.path(), &listing()).is_err(),
            "only once"
        );
    }

    #[test]
    fn changing_a_servers_settings_takes_it_back_to_approval() {
        let dir = setup();
        let mut catalog = Catalog::default();
        catalog
            .import(dir.path(), vec![found("docs", "npx", &[("K", "1")])])
            .unwrap();
        catalog.approve(dir.path(), "docs").unwrap();
        let values = BTreeMap::from([("K".to_owned(), "2".to_owned())]);
        catalog.set_secrets(dir.path(), "docs", values).unwrap();
        assert!(!catalog.entries[0].enabled);
        assert!(catalog.specs(dir.path()).is_empty());
    }

    #[test]
    fn old_catalogs_load_with_defaults() {
        let dir = setup();
        std::fs::write(
            path(dir.path()),
            json!({"entries":[{"id":"docs","command":"npx"}]}).to_string(),
        )
        .unwrap();
        let catalog = Catalog::load(dir.path()).unwrap();
        assert!(!catalog.managed);
        assert_eq!(catalog.entries[0].class, Class::PerWorktree);
    }
}
