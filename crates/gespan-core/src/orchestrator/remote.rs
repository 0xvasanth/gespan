//! Agents on herdr machines as peers: they join the roster of a repository
//! of the same name, and a local chat can start one, message it, wait on it
//! and read it. Everything goes through a machine's herdr API; nothing types
//! into a terminal while its agent works.
use super::roster::{Member, Status, Workspace};
use herdr_link::{Agent, AgentStatus, Client, Transport};
use std::path::{Path, PathBuf};

/// The settled states a message waits for before it is typed.
const SETTLED: [AgentStatus; 3] = [AgentStatus::Idle, AgentStatus::Done, AgentStatus::Blocked];
/// How long herdr may take to see a started agent ready.
const START_TIMEOUT_MS: u64 = 60_000;
/// How much recent output a read returns.
const READ_LINES: u32 = 200;

/// One machine's herdr, as the core uses it.
pub trait Remote: Send + Sync {
    /// A new pane in `cwd`, beside the focused one. Its id.
    fn split_pane(&self, cwd: &str) -> Result<String, String>;
    /// A new worktree of the checkout at `cwd`: its path and first pane.
    fn create_worktree(&self, cwd: &str, base: Option<&str>) -> Result<(String, String), String>;
    fn start_agent(&self, name: &str, kind: &str, pane: &str) -> Result<(), String>;
    fn prompt(&self, target: &str, text: &str) -> Result<(), String>;
    /// Waits for idle, done or blocked; the state reached.
    fn wait_settled(&self, target: &str, timeout_ms: u64) -> Result<String, String>;
    fn read(&self, target: &str) -> Result<String, String>;
    /// Who runs in `pane` now: `Some(name)` (empty for an unnamed agent),
    /// or `None` when no agent is there.
    fn agent_in(&self, pane: &str) -> Result<Option<String>, String>;
}

impl<T: Transport + Send + Sync> Remote for Client<T> {
    fn split_pane(&self, cwd: &str) -> Result<String, String> {
        Client::split_pane(self, None, cwd).map_err(|e| e.to_string())
    }
    fn create_worktree(&self, cwd: &str, base: Option<&str>) -> Result<(String, String), String> {
        Client::create_worktree(self, cwd, base)
            .map(|made| (made.path, made.pane_id))
            .map_err(|e| e.to_string())
    }
    fn start_agent(&self, name: &str, kind: &str, pane: &str) -> Result<(), String> {
        Client::start_agent(self, name, kind, pane, Some(START_TIMEOUT_MS))
            .map(drop)
            .map_err(|e| e.to_string())
    }
    fn prompt(&self, target: &str, text: &str) -> Result<(), String> {
        Client::prompt_agent(self, target, text, None)
            .map(drop)
            .map_err(|e| e.to_string())
    }
    fn wait_settled(&self, target: &str, timeout_ms: u64) -> Result<String, String> {
        let event = Client::wait_agent(self, target, &SETTLED, Some(timeout_ms))
            .map_err(|e| e.to_string())?;
        Ok(event["data"]["agent_status"]
            .as_str()
            .unwrap_or("idle")
            .to_owned())
    }
    fn read(&self, target: &str) -> Result<String, String> {
        Client::read_agent(self, target, Some(READ_LINES))
            .map(|read| read.text)
            .map_err(|e| e.to_string())
    }
    fn agent_in(&self, pane: &str) -> Result<Option<String>, String> {
        Client::get_agent(self, pane)
            .map(|agent| agent.map(|agent| agent.name.unwrap_or_default()))
            .map_err(|e| e.to_string())
    }
}

/// A remote agent as Gespan knows it: the pane it runs in, and the name it
/// went by, when it had one. A pane can be reused, so every call checks the
/// pane still holds that agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub pane: String,
    pub name: Option<String>,
}

/// Fails unless `target`'s pane still holds the agent Gespan means.
pub fn check(remote: &dyn Remote, target: &Target) -> Result<(), String> {
    match (remote.agent_in(&target.pane)?, &target.name) {
        (None, _) => Err("that agent no longer runs on the machine".into()),
        (Some(now), Some(name)) if now != *name => Err(format!(
            "another agent (\"{now}\") now runs where \"{name}\" was; nothing was sent"
        )),
        _ => Ok(()),
    }
}

/// Whether a session running `sender` may start a remote agent or type into
/// one. Gespan can't see or limit what an agent on another machine may do,
/// so only a full-access session may tell one what to do; an unknown mode is
/// refused.
pub fn may_direct(sender: Option<crate::agent::AccessMode>) -> bool {
    sender == Some(crate::agent::AccessMode::FullAccess)
}

/// Where a started remote agent runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteStart {
    pub name: String,
    pub pane_id: String,
    pub cwd: String,
}

/// Starts agent `kind` named `name` on a machine, in its checkout at `cwd`
/// or a new worktree of it, and gives it `prompt`.
pub fn start_on(
    remote: &dyn Remote,
    kind: &str,
    name: &str,
    cwd: &str,
    worktree: bool,
    base: Option<&str>,
    prompt: &str,
) -> Result<RemoteStart, String> {
    let (cwd, pane_id) = if worktree {
        remote.create_worktree(cwd, base)?
    } else {
        (cwd.to_owned(), remote.split_pane(cwd)?)
    };
    remote.start_agent(name, kind, &pane_id)?;
    remote.prompt(name, prompt)?;
    Ok(RemoteStart {
        name: name.to_owned(),
        pane_id,
        cwd,
    })
}

/// Types `text` into a remote agent once it has settled, never while it
/// works. An agent waiting on its own user's approval gets nothing, and the
/// pane is checked again after the wait: its agent may have changed.
pub fn deliver_when_idle(
    remote: &dyn Remote,
    target: &Target,
    text: &str,
    timeout_ms: u64,
) -> Result<(), String> {
    check(remote, target)?;
    let status = remote.wait_settled(&target.pane, timeout_ms)?;
    if status == AgentStatus::Blocked.as_str() {
        return Err("the agent is waiting on its user's answer; try again later".into());
    }
    check(remote, target)?;
    remote.prompt(&target.pane, text)
}

pub fn read_from(remote: &dyn Remote, target: &Target) -> Result<String, String> {
    check(remote, target)?;
    remote.read(&target.pane)
}

/// Waits for the agent to settle; the state it reached.
pub fn wait_on(remote: &dyn Remote, target: &Target, timeout_ms: u64) -> Result<String, String> {
    check(remote, target)?;
    remote.wait_settled(&target.pane, timeout_ms)
}

/// A name herdr accepts (`[a-z][a-z0-9_-]{0,31}`) from a title.
pub fn agent_name(title: &str) -> String {
    let mut name = String::new();
    for c in title.trim().chars() {
        let c = c.to_ascii_lowercase();
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            name.push(c);
        } else if !name.ends_with('-') && !name.is_empty() {
            name.push('-');
        }
    }
    let mut name = name.trim_matches('-').to_owned();
    if name.is_empty() {
        return "gespan".into();
    }
    if !name.starts_with(|c: char| c.is_ascii_lowercase()) {
        name.insert_str(0, "a-");
    }
    name.truncate(32);
    name.trim_end_matches('-').to_owned()
}

/// The id a remote agent goes by in `sessions_*` calls: negative, so never
/// a local session's, and the same for the same machine and pane.
pub fn remote_id(machine_id: &str, pane_id: &str) -> i64 {
    let hash = super::roster::fingerprint(&format!("{machine_id}\u{0}{pane_id}"));
    -((hash >> 1) as i64) - 1
}

/// The agents on a machine that work in a folder named like the caller's
/// repository (or one inside it), as roster members: `(id, member, pane)`.
/// They share the caller's repository key, so they list as siblings.
pub fn remote_members(
    machine_id: &str,
    machine_label: &str,
    agents: &[Agent],
    repo_folder: &str,
    repo_key: &str,
) -> Vec<(i64, Member, String)> {
    agents
        .iter()
        .filter(|agent| {
            agent
                .cwd
                .as_deref()
                .is_some_and(|cwd| names_folder(cwd, repo_folder))
        })
        .map(|agent| {
            let status = match agent.agent_status {
                AgentStatus::Working => Status::Working,
                AgentStatus::Blocked => Status::WaitingOnUser,
                _ => Status::Idle,
            };
            let kind = agent.agent.clone().unwrap_or_else(|| "agent".into());
            let member = Member {
                id: remote_id(machine_id, &agent.pane_id),
                title: agent.name.clone().unwrap_or_else(|| agent.pane_id.clone()),
                harness: format!("{kind} on {machine_label}"),
                status,
                workspace: Workspace::Main,
                goal: agent.title.clone().unwrap_or_default(),
                files: Vec::new(),
                repo: Some(PathBuf::from(repo_key)),
                cwd: agent.cwd.clone().unwrap_or_default(),
                last_active_ms: 0,
                archived: false,
                remote: false,
                parent: None,
            };
            (member.id, member, agent.pane_id.clone())
        })
        .collect()
}

/// Whether `path` is a plain path (absolute or under `~`, no `.` or `..`)
/// with a folder named `folder` in it: a checkout of that repository, or a
/// folder inside one.
pub fn names_folder(path: &str, folder: &str) -> bool {
    use std::path::Component;
    let plain = path.starts_with('/') || path == "~" || path.starts_with("~/");
    plain
        && Path::new(path)
            .components()
            .all(|part| !matches!(part, Component::ParentDir | Component::CurDir))
        && Path::new(path)
            .components()
            .any(|part| part.as_os_str() == folder)
}
