//! Branch and worktree RPCs shared with the original remote project pickers.
use super::{Branch, Snapshot, Worktree};
use crate::remote::{default_connections, parse_remote_path, remote_path, Connections};
use crate::worktree_copy::Copied;
use crate::worktree_scripts::{self, Exit};
use serde::Deserialize;
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

const JOBS: &str = "worktree.jobs";
const POLL_EVERY: Duration = Duration::from_millis(300);
/// Polls in a row that can fail before the host counts as gone: about two
/// minutes at the request's own 30-second timeout, seconds when it refuses.
const MISSED_POLLS: u32 = 4;
const OLD_HOST: &str =
    "This host can’t run worktree scripts yet. Update it in Settings → Connections.";

#[derive(Deserialize)]
struct Project {
    id: String,
    cwd: String,
}
#[derive(Deserialize)]
struct Trees {
    worktrees: Vec<Worktree>,
}
#[derive(Deserialize)]
struct Branches {
    current: Option<String>,
    branches: Vec<String>,
    remotes: Vec<RemoteBranch>,
}
#[derive(Deserialize)]
struct RemoteBranch {
    name: String,
    remote: String,
}

struct Context {
    connections: Arc<Connections>,
    machine: String,
    project: String,
    environment: String,
    cwd: String,
}
impl Context {
    fn request(&self, method: &str, mut params: Value) -> Result<Value, String> {
        params["projectId"] = json!(self.project);
        params["cwd"] = json!(self.cwd);
        self.connections.request(&self.machine, method, params)
    }
    fn trees(&self) -> Result<Vec<Worktree>, String> {
        let listed: Trees = decode(self.request("git.worktrees", json!({}))?)?;
        Ok(listed
            .worktrees
            .into_iter()
            .map(|mut tree| {
                tree.path = remote_path(&self.environment, &tree.path);
                tree
            })
            .collect())
    }
}
/// One look at a job on the host.
#[derive(Deserialize)]
struct Polled {
    lines: Vec<String>,
    next: u64,
    done: bool,
    code: Option<i32>,
    copied: Option<Copied>,
    error: Option<String>,
}
/// Follows the job to its end. Requests go through `request`, so the protocol
/// can be tried without a host.
fn follow(
    request: &dyn Fn(&str, Value) -> Result<Value, String>,
    id: &str,
    line: &mut dyn FnMut(String),
    cancelled: &AtomicBool,
    every: Duration,
) -> Result<Option<Polled>, String> {
    let (mut after, mut missed, mut told) = (0, 0, false);
    loop {
        // A cancel that doesn't arrive is tried again, or the script runs on.
        if cancelled.load(Ordering::SeqCst) && !told {
            match request("worktree.jobCancel", json!({"jobId":id})) {
                Ok(_) => told = true,
                Err(error) => {
                    missed += 1;
                    if missed >= MISSED_POLLS {
                        return Err(format!("Lost touch with the host: {error}"));
                    }
                }
            }
        }
        let polled = match request("worktree.jobPoll", json!({"jobId":id,"after":after})) {
            Ok(value) => {
                if told || !cancelled.load(Ordering::SeqCst) {
                    missed = 0;
                }
                decode::<Polled>(value)?
            }
            Err(error) => {
                missed += 1;
                if missed >= MISSED_POLLS {
                    return Err(format!("Lost touch with the host: {error}"));
                }
                std::thread::sleep(every);
                continue;
            }
        };
        after = polled.next;
        let done = polled.done;
        let error = polled.error.clone();
        let mut polled = polled;
        for text in polled.lines.drain(..) {
            line(text);
        }
        if done {
            if let Some(error) = error {
                return Err(error);
            }
            return Ok((!told).then_some(polled));
        }
        std::thread::sleep(every);
    }
}
impl Context {
    fn start_job(&self, params: Value) -> Result<String, String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Started {
            job_id: String,
        }
        Ok(decode::<Started>(self.request("worktree.jobStart", params)?)?.job_id)
    }
    fn has_jobs(&self) -> Result<bool, String> {
        self.connections
            .request(&self.machine, "environment.describe", json!({}))
            .map(|host| offers_jobs(&host))
    }
    /// The host's copy of a project's `.worktreeinclude` files into this
    /// context's worktree. An older host has no way to.
    fn copy_included(&self) -> Result<Copied, String> {
        if !self.has_jobs()? {
            return Ok(Copied::default());
        }
        let id = self.start_job(json!({"kind":"copy"}))?;
        let request = |method: &str, params| self.request(method, params);
        let polled = follow(
            &request,
            &id,
            &mut |_| {},
            &AtomicBool::new(false),
            POLL_EVERY,
        )?;
        Ok(polled.and_then(|polled| polled.copied).unwrap_or_default())
    }
}
fn offers_jobs(host: &Value) -> bool {
    host["capabilities"]
        .as_array()
        .is_some_and(|names| names.iter().any(|name| name == JOBS))
}
/// Where the project's scripts run for a remote worktree.
pub(crate) fn script_runner<'a>(
    worktree: &'a Path,
    branch: Option<&'a str>,
) -> Box<worktree_scripts::Runner<'a>> {
    Box::new(move |script, line, cancelled| {
        let context = context(worktree)?;
        if !context.has_jobs()? {
            return Err(OLD_HOST.into());
        }
        let id = context.start_job(json!({
            "kind": "script",
            "script": script,
            "branch": branch.unwrap_or_default(),
        }))?;
        let request = |method: &str, params| context.request(method, params);
        Ok(follow(&request, &id, line, cancelled, POLL_EVERY)?
            .map_or(Exit::Cancelled, |polled| Exit::Code(polled.code)))
    })
}
/// The checkout a remote project's scripts are kept under.
pub(super) fn main_checkout(cwd: &Path) -> Result<String, String> {
    context(cwd)?
        .trees()?
        .into_iter()
        .next()
        .map(|tree| tree.path)
        .ok_or_else(|| "No working copies found".into())
}
/// A worktree the host just made: its included files, then Setup. Like a local
/// copy, a problem copying never fails the worktree.
pub(super) fn set_up(
    turn: worktree_scripts::Turn,
    main: &Path,
    tree: Worktree,
) -> Result<Worktree, String> {
    let worktree = Path::new(&tree.path);
    let copied = context(worktree)
        .and_then(|context| context.copy_included())
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "could not copy a project's included files on its host");
            Copied::default()
        });
    worktree_scripts::set_up(turn, main, worktree, tree.branch.as_deref(), copied)?;
    Ok(tree)
}
pub(crate) fn remove_worktree(cwd: &Path, path: &str) -> Result<(), String> {
    let context = context(cwd)?;
    let host_path = parse_remote_path(path)
        .ok_or("Invalid remote working copy")?
        .host_path;
    context.request("git.worktreeRemove", json!({"path":host_path,"force":true}))?;
    Ok(())
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("Invalid remote Git response: {error}"))
}
fn context(cwd: &Path) -> Result<Context, String> {
    let remote = parse_remote_path(&cwd.to_string_lossy()).ok_or("Invalid remote working copy")?;
    let connections = default_connections()?;
    let machine = connections
        .machine_for_environment(&remote.environment_id)?
        .id;
    let projects: Vec<Project> =
        decode(connections.request(&machine, "projects.list", json!({}))?)?;
    let mut project = projects
        .iter()
        .find(|project| key(&project.cwd) == key(&remote.host_path))
        .map(|project| project.id.clone());
    // Worktrees commonly live beside, rather than under, their main checkout.
    if project.is_none() {
        for candidate in &projects {
            let listed =
                connections.request(&machine, "git.worktrees", json!({"projectId":candidate.id}));
            if let Ok(listed) = listed.and_then(decode::<Trees>) {
                if listed
                    .worktrees
                    .iter()
                    .any(|tree| !tree.missing && key(&tree.path) == key(&remote.host_path))
                {
                    project = Some(candidate.id.clone());
                    break;
                }
            }
        }
    }
    Ok(Context {
        connections,
        machine,
        project: project.ok_or("Choose an available worktree of this project")?,
        environment: remote.environment_id,
        cwd: remote.host_path,
    })
}
fn key(path: &str) -> String {
    crate::files::path_key(path)
}

pub(super) fn snapshot(cwd: &Path) -> Result<Snapshot, String> {
    let context = context(cwd)?;
    let branches: Branches = decode(context.request("git.branches", json!({}))?)?;
    let trees = context.trees()?;
    Ok(Snapshot {
        detached: branches.current.is_none(),
        current: branches.current,
        linked: trees
            .iter()
            .any(|tree| !tree.main && key(&tree.path) == key(&cwd.to_string_lossy())),
        branches: branches
            .branches
            .into_iter()
            .map(|name| Branch { name, remote: None })
            .chain(branches.remotes.into_iter().map(|branch| Branch {
                name: branch.name,
                remote: Some(branch.remote),
            }))
            .collect(),
        worktrees: trees,
    })
}
pub(super) fn worktrees(cwd: &Path) -> Result<Vec<Worktree>, String> {
    context(cwd)?.trees()
}
pub(super) fn checkout(cwd: &Path, branch: &Branch) -> Result<(), String> {
    context(cwd)?.request(
        "git.switch",
        json!({"branch":branch.name,"remote":branch.remote}),
    )?;
    Ok(())
}
pub(super) fn create_branch(cwd: &Path, branch: &str) -> Result<(), String> {
    context(cwd)?.request("git.createBranch", json!({"branch":branch}))?;
    Ok(())
}
pub(super) fn create_worktree(
    cwd: &Path,
    branch: &str,
    base: Option<&str>,
    existing: bool,
) -> Result<Worktree, String> {
    let context = context(cwd)?;
    let mut tree: Worktree = decode(context.request(
        "git.worktreeCreate",
        json!({"branch":branch,"base":base.unwrap_or("HEAD"),"existing":existing}),
    )?)?;
    tree.path = remote_path(&context.environment, &tree.path);
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    fn polled(lines: &[&str], next: u64, done: bool, code: Option<i32>) -> Value {
        json!({"lines":lines,"next":next,"done":done,"code":code,"copied":null,"error":null})
    }
    type Asked = Arc<Mutex<Vec<(String, Value)>>>;
    /// Answers each request from a list, and keeps what it was asked.
    fn host(
        answers: Vec<Result<Value, String>>,
    ) -> (impl Fn(&str, Value) -> Result<Value, String>, Asked) {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let answers = Mutex::new(answers.into_iter());
        let seen = asked.clone();
        (
            move |method: &str, params: Value| {
                seen.lock().unwrap().push((method.to_owned(), params));
                if method == "worktree.jobCancel" {
                    return Ok(json!({}));
                }
                answers.lock().unwrap().next().expect("an answer")
            },
            asked,
        )
    }

    #[test]
    fn a_job_is_followed_from_where_the_last_poll_stopped() {
        let (request, asked) = host(vec![
            Ok(polled(&["one", "two"], 2, false, None)),
            Ok(polled(&["three"], 3, true, Some(4))),
        ]);
        let mut lines = Vec::new();
        let done = follow(
            &request,
            "job",
            &mut |line| lines.push(line),
            &AtomicBool::new(false),
            Duration::ZERO,
        )
        .unwrap()
        .unwrap();
        assert_eq!(lines, ["one", "two", "three"]);
        assert_eq!(done.code, Some(4));
        let asked = asked.lock().unwrap();
        assert_eq!(asked[0].1["after"], 0);
        assert_eq!(asked[1].1["after"], 2);
    }

    #[test]
    fn a_cancelled_job_is_told_once_and_reads_as_cancelled() {
        let (request, asked) = host(vec![
            Ok(polled(&[], 0, false, None)),
            Ok(polled(&[], 0, true, None)),
        ]);
        let done = follow(
            &request,
            "job",
            &mut |_| {},
            &AtomicBool::new(true),
            Duration::ZERO,
        )
        .unwrap();
        assert!(done.is_none());
        let cancels = asked
            .lock()
            .unwrap()
            .iter()
            .filter(|(method, _)| method == "worktree.jobCancel")
            .count();
        assert_eq!(cancels, 1);
    }

    #[test]
    fn a_poll_can_miss_but_a_silent_host_ends_the_job() {
        let (request, _) = host(vec![Err("blip".into()), Ok(polled(&[], 0, true, Some(0)))]);
        let done = follow(
            &request,
            "job",
            &mut |_| {},
            &AtomicBool::new(false),
            Duration::ZERO,
        );
        assert_eq!(done.unwrap().unwrap().code, Some(0));
        let (request, _) = host(vec![
            Err("gone".into()),
            Err("gone".into()),
            Err("gone".into()),
            Err("gone".into()),
        ]);
        let error = follow(
            &request,
            "job",
            &mut |_| {},
            &AtomicBool::new(false),
            Duration::ZERO,
        )
        .err()
        .unwrap();
        assert!(error.contains("gone"), "{error}");
    }

    #[test]
    fn a_cancel_that_fails_is_sent_again() {
        let answers = Mutex::new(vec![true, false]);
        let cancels = Mutex::new(0);
        let request = |method: &str, _: Value| -> Result<Value, String> {
            if method == "worktree.jobCancel" {
                *cancels.lock().unwrap() += 1;
                return if answers.lock().unwrap().pop() == Some(false) {
                    Err("blip".into())
                } else {
                    Ok(json!({}))
                };
            }
            let told = *cancels.lock().unwrap() >= 2;
            Ok(polled(&[], 0, told, None))
        };
        let done = follow(
            &request,
            "job",
            &mut |_| {},
            &AtomicBool::new(true),
            Duration::ZERO,
        )
        .unwrap();
        assert!(done.is_none());
        assert_eq!(*cancels.lock().unwrap(), 2);
    }

    #[test]
    fn only_a_host_that_lists_jobs_is_asked_to_run_them() {
        assert!(offers_jobs(&json!({"capabilities":["git.worktrees",JOBS]})));
        assert!(!offers_jobs(&json!({"capabilities":["git.worktrees"]})));
        assert!(!offers_jobs(&json!({})));
    }

    #[test]
    fn a_job_that_could_not_start_reports_why() {
        let (request, _) = host(vec![Ok(json!({
            "lines": [], "next": 0, "done": true, "code": null, "copied": null,
            "error": "Could not start the script: sh missing",
        }))]);
        let error = follow(
            &request,
            "job",
            &mut |_| {},
            &AtomicBool::new(false),
            Duration::ZERO,
        )
        .err()
        .unwrap();
        assert!(error.contains("sh missing"), "{error}");
    }
}
