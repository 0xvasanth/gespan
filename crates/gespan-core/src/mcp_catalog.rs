//! Gespan's own list of MCP servers: installed once here and used by every
//! agent through the hub. Imported from an agent's config once, never written
//! back to it. Env values live in the OS keychain, and nothing runs until the
//! user has approved the exact command and environment.
use crate::mcp::Importable;
use mcp_hub::{ServerSpec, Sharing};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Mutex,
};

const FILE: &str = "mcp-catalog.json";
/// Prefix of a server's keychain name.
const SECRETS: &str = "mcp-server";
/// Keychain name of the key that seals approvals of servers with secrets.
const APPROVAL_KEY: &str = "mcp-approval-key";

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
    /// Fingerprint of the exact command and environment the user approved,
    /// sealed when that environment holds secrets.
    pub approved: Option<String>,
    /// Where it was imported from, for showing the user.
    pub source: Option<String>,
    pub tools: Option<Vec<Value>>,
}

impl Entry {
    fn spec(&self, env: BTreeMap<String, String>) -> ServerSpec {
        ServerSpec {
            id: self.id.clone(),
            command: self.command.clone(),
            args: self.args.clone(),
            env,
            sharing: self.class.into(),
            tools: self.tools.clone(),
        }
    }

    /// The command line as the approval card shows it, quoted so each argument
    /// reads as the one word it will be.
    pub fn command_line(&self) -> String {
        std::iter::once(self.command.as_str())
            .chain(self.args.iter().map(String::as_str))
            .map(|word| match shlex::try_quote(word) {
                Ok(quoted) => quoted.into_owned(),
                Err(_) => format!("{word:?}"),
            })
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

/// What a change to the catalogue left.
pub struct Edited {
    pub catalog: Catalog,
    pub changed: bool,
    /// Rises with every edit, in the order they happened, so a reader can
    /// tell which of two results is the newer.
    pub revision: u64,
}

/// Every read-modify-write of the file goes through here, one at a time, so no
/// two can lose each other's change.
pub fn update(
    data_dir: &Path,
    edit: impl FnOnce(&mut Catalog) -> Result<(), String>,
) -> Result<Edited, String> {
    static REVISION: Mutex<u64> = Mutex::new(0);
    let mut revision = REVISION.lock().unwrap_or_else(|e| e.into_inner());
    let mut catalog = Catalog::load(data_dir)?;
    let before = catalog.clone();
    edit(&mut catalog)?;
    let changed = catalog != before;
    if changed {
        catalog.save(data_dir)?;
    }
    *revision += 1;
    Ok(Edited {
        catalog,
        changed,
        revision: *revision,
    })
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
    /// was. A server already here, or seen under another agent with the same
    /// environment, is skipped; with another environment it's a server of its
    /// own, since that may be a different account. Returns how many were added.
    pub fn import(&mut self, data_dir: &Path, found: Vec<Importable>) -> Result<usize, String> {
        let mut added = 0;
        for server in found {
            let same = |entry: &Entry| match &server.url {
                Some(url) => entry.url.as_deref() == Some(url),
                None => {
                    entry.command == server.command
                        && entry.args == server.args
                        && read_secrets(data_dir, entry).is_ok_and(|env| env == server.env)
                }
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
        let approval = approval(data_dir, &self.resolved(data_dir, entry)?)?;
        let entry = self.entry_mut(id).ok_or("Unknown server")?;
        entry.approved = Some(approval);
        entry.enabled = true;
        Ok(())
    }

    /// Approves what the user was shown, and only that: the entry may have
    /// changed while the question was open.
    pub fn approve_shown(&mut self, data_dir: &Path, shown: &Entry) -> Result<(), String> {
        let now = self
            .entries
            .iter()
            .find(|entry| entry.id == shown.id)
            .ok_or("Unknown server")?;
        if (&now.command, &now.args, &now.env) != (&shown.command, &shown.args, &shown.env) {
            return Err("This server changed. Review it again.".to_owned());
        }
        self.approve(data_dir, &shown.id)
    }

    /// The entry as it would run, if that's what the user approved.
    fn runnable(&self, data_dir: &Path, entry: &Entry) -> Option<ServerSpec> {
        let spec = self
            .resolved(data_dir, entry)
            .inspect_err(|error| tracing::warn!(%error, "an MCP server's secrets"))
            .ok()?;
        let approval = approval(data_dir, &spec).ok()?;
        (entry.approved.as_deref() == Some(approval.as_str())).then_some(spec)
    }

    /// Servers the hub may run. One edited since its approval, by hand or by
    /// anything else, is left out until it's approved again.
    pub fn specs(&self, data_dir: &Path) -> Vec<ServerSpec> {
        self.entries
            .iter()
            .filter(|entry| entry.enabled && entry.url.is_none())
            .filter_map(|entry| self.runnable(data_dir, entry))
            .collect()
    }

    /// Ids turned on that won't run, because their approval no longer matches.
    pub fn stale(&self, data_dir: &Path) -> BTreeSet<String> {
        self.entries
            .iter()
            .filter(|entry| entry.enabled && entry.url.is_none())
            .filter(|entry| self.runnable(data_dir, entry).is_none())
            .map(|entry| entry.id.clone())
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

/// The value a server's approval is stored as. With secrets in it, the plain
/// fingerprint would let someone holding the catalogue file guess them offline,
/// so it's sealed with a key the file doesn't have. Without any, there's nothing
/// to guess and no keychain to need.
fn approval(data_dir: &Path, spec: &ServerSpec) -> Result<String, String> {
    let fingerprint = spec.fingerprint();
    if spec.env.is_empty() {
        return Ok(fingerprint);
    }
    let sealed = Sha256::new()
        .chain_update(approval_key(data_dir)?)
        .chain_update(fingerprint)
        .finalize();
    Ok(sealed.iter().map(|byte| format!("{byte:02x}")).collect())
}

fn approval_key(data_dir: &Path) -> Result<String, String> {
    static CREATING: Mutex<()> = Mutex::new(());
    let _one_at_a_time = CREATING.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(key) = gespan_tracker_secrets::read_named(APPROVAL_KEY, data_dir)? {
        return Ok(key);
    }
    let bytes: [u8; 32] = rand::random();
    let key: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    gespan_tracker_secrets::write_named(APPROVAL_KEY, data_dir, &key)?;
    Ok(key)
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

/// Removes a server, then its secrets: a failed save must leave a server that
/// still has them, and a leftover secret harms nothing.
pub fn remove(data_dir: &Path, id: &str) -> Result<Edited, String> {
    let edited = update(data_dir, |catalog| {
        catalog.entries.retain(|entry| entry.id != id);
        Ok(())
    })?;
    gespan_tracker_secrets::delete_named(&secret_name(id), data_dir)?;
    Ok(edited)
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
        catalog.save(dir.path()).unwrap();
        assert!(remove(dir.path(), "docs").unwrap().changed);
        assert!(Catalog::load(dir.path()).unwrap().entries.is_empty());
        assert_eq!(
            gespan_tracker_secrets::read_named("mcp-server:docs", dir.path()).unwrap(),
            None
        );
    }

    #[test]
    fn a_server_keeps_its_secrets_if_its_removal_cant_be_saved() {
        let dir = setup();
        let mut catalog = Catalog::default();
        catalog
            .import(dir.path(), vec![found("docs", "npx", &[("TOKEN", "one")])])
            .unwrap();
        catalog.save(dir.path()).unwrap();
        // A corrupt file can't be loaded, so nothing can be saved either.
        std::fs::write(path(dir.path()), "not json").unwrap();
        assert!(remove(dir.path(), "docs").is_err());
        assert!(
            gespan_tracker_secrets::read_named("mcp-server:docs", dir.path())
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn the_same_command_with_another_account_is_a_server_of_its_own() {
        let dir = setup();
        let mut catalog = Catalog::default();
        let found_in = |token| vec![found("docs", "npx", &[("TOKEN", token)])];
        assert_eq!(catalog.import(dir.path(), found_in("one")).unwrap(), 1);
        assert_eq!(catalog.import(dir.path(), found_in("one")).unwrap(), 0);
        assert_eq!(catalog.import(dir.path(), found_in("two")).unwrap(), 1);
        let ids: Vec<_> = catalog.entries.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, ["docs", "docs-2"]);
        catalog.approve(dir.path(), "docs").unwrap();
        catalog.approve(dir.path(), "docs-2").unwrap();
        let tokens: Vec<_> = catalog
            .specs(dir.path())
            .into_iter()
            .map(|spec| spec.env["TOKEN"].clone())
            .collect();
        assert_eq!(tokens, ["one", "two"]);
    }

    #[test]
    fn concurrent_edits_all_land() {
        let dir = setup();
        std::thread::scope(|scope| {
            for n in 0..16 {
                let data_dir = dir.path();
                scope.spawn(move || {
                    update(data_dir, |catalog| {
                        catalog.entries.push(Entry {
                            id: format!("server-{n}"),
                            ..Entry::default()
                        });
                        Ok(())
                    })
                    .unwrap();
                });
            }
        });
        assert_eq!(Catalog::load(dir.path()).unwrap().entries.len(), 16);
    }

    #[test]
    fn revisions_rise_in_edit_order_and_only_a_change_is_saved() {
        let dir = setup();
        let first = update(dir.path(), |_| Ok(())).unwrap();
        assert!(!first.changed);
        assert!(!path(dir.path()).exists(), "nothing changed, nothing saved");
        let second = update(dir.path(), |catalog| {
            catalog.managed = true;
            Ok(())
        })
        .unwrap();
        assert!(second.changed && second.revision > first.revision);
        assert!(Catalog::load(dir.path()).unwrap().managed);
        assert!(update(dir.path(), |_| Err("no".into())).is_err());
    }

    #[test]
    fn an_approval_covers_only_what_was_shown() {
        let dir = setup();
        let mut catalog = Catalog::default();
        catalog
            .import(dir.path(), vec![found("docs", "npx", &[("TOKEN", "one")])])
            .unwrap();
        let shown = catalog.entries[0].clone();
        catalog
            .entry_mut("docs")
            .unwrap()
            .args
            .push("--evil".into());
        assert!(catalog
            .approve_shown(dir.path(), &shown)
            .unwrap_err()
            .contains("changed"));
        assert!(!catalog.entries[0].enabled);
        let shown = catalog.entries[0].clone();
        catalog.entry_mut("docs").unwrap().class = Class::Shared;
        catalog.approve_shown(dir.path(), &shown).unwrap();
        assert_eq!(catalog.specs(dir.path()).len(), 1, "sharing isn't run");
    }

    #[test]
    fn a_stored_approval_gives_nothing_to_guess_a_secret_from() {
        let dir = setup();
        let mut catalog = Catalog::default();
        catalog
            .import(dir.path(), vec![found("docs", "npx", &[("TOKEN", "weak")])])
            .unwrap();
        catalog.approve(dir.path(), "docs").unwrap();
        let spec = catalog.specs(dir.path()).remove(0);
        assert_ne!(
            catalog.entries[0].approved.as_deref(),
            Some(spec.fingerprint().as_str()),
            "the plain hash could be tested against guesses"
        );
    }

    #[test]
    fn a_server_with_no_secrets_needs_no_keychain_to_be_approved() {
        let dir = setup();
        let mut catalog = Catalog::default();
        catalog
            .import(dir.path(), vec![found("docs", "npx", &[])])
            .unwrap();
        catalog.approve(dir.path(), "docs").unwrap();
        let spec = catalog.specs(dir.path()).remove(0);
        assert_eq!(
            catalog.entries[0].approved.as_deref(),
            Some(spec.fingerprint().as_str())
        );
        assert_eq!(
            gespan_tracker_secrets::read_named(APPROVAL_KEY, dir.path()).unwrap(),
            None
        );
    }

    #[test]
    fn an_edited_server_reports_stale_until_approved_again() {
        let dir = setup();
        let mut catalog = Catalog::default();
        catalog
            .import(dir.path(), vec![found("docs", "npx", &[("TOKEN", "one")])])
            .unwrap();
        assert!(catalog.stale(dir.path()).is_empty(), "off isn't stale");
        catalog.approve(dir.path(), "docs").unwrap();
        assert!(catalog.stale(dir.path()).is_empty());
        catalog
            .entry_mut("docs")
            .unwrap()
            .args
            .push("--more".into());
        assert_eq!(catalog.stale(dir.path()).len(), 1);
        assert!(catalog.specs(dir.path()).is_empty());
        catalog.approve(dir.path(), "docs").unwrap();
        assert!(catalog.stale(dir.path()).is_empty());
    }

    #[test]
    fn the_command_line_shows_where_each_argument_ends() {
        let entry = Entry {
            command: "/opt/my tools/server".into(),
            args: vec!["--name".into(), "a b".into(), "plain".into()],
            ..Entry::default()
        };
        assert_eq!(
            entry.command_line(),
            r#"'/opt/my tools/server' --name 'a b' plain"#
        );
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
