//! Agents on herdr machines as peers of the chats here. They join the
//! roster of a repository of the same name, and the `sessions` tools reach
//! them by their (negative) ids: a start runs on a machine, a message waits
//! there until the agent has settled, and wait and read ask its herdr.
//! Every herdr call runs off the UI thread.
use super::Shell;
use gespan_core::machines::{Machine, Online};
use gespan_core::orchestrator::{
    bridge::UNAVAILABLE,
    mailbox::{self, Kind as MailKind, Mail, State as MailState},
    policy::{Code, Refusal},
    remote::{self, Remote},
    roster::Member,
    start::{StartInput, WorkspaceChoice},
};
use gpui::{App, Context};
use serde_json::{Value, json};
use std::{path::Path, sync::Arc, sync::mpsc, time::Duration};

type Reply = mpsc::Sender<Result<Value, String>>;

/// How a machine's herdr is reached: its ssh bridge, or a fake in tests.
pub(crate) type RemoteFor = Arc<dyn Fn(&Machine) -> Arc<dyn Remote> + Send + Sync>;

pub(super) fn default_remote() -> RemoteFor {
    Arc::new(|machine: &Machine| -> Arc<dyn Remote> {
        Arc::new(gespan_core::machines::client_for(machine))
    })
}

/// How long a message waits for a remote agent to settle.
const DELIVER_WAIT: Duration = Duration::from_secs(30 * 60);

/// A remote agent `sessions_*` can reach.
#[derive(Clone, Debug)]
pub(super) struct Peer {
    machine: Machine,
    /// Its pane, and the name it goes by there, checked before each call.
    target: remote::Target,
    title: String,
    /// The session that started it here; 0 for one found on the machine.
    parent: i64,
}

/// The folder a repository key (its git common directory) names: `app` for
/// `/home/dev/app/.git`.
fn repo_folder(key: &Path) -> Option<String> {
    let dir = if key.file_name().is_some_and(|name| name == ".git") {
        key.parent()?
    } else {
        key
    };
    dir.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

impl Shell {
    fn machines_online(&self, cx: &App) -> Vec<gespan_core::machines::MachineView> {
        crate::machines::get(cx)
            .map(|machines| {
                machines
                    .read(cx)
                    .views()
                    .iter()
                    .filter(|view| view.machine.enabled && view.snapshot.online == Online::Online)
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The remote agents working in a checkout named like `caller`'s
    /// repository, as roster members.
    pub(super) fn remote_siblings(&self, caller: &Member, cx: &App) -> Vec<Member> {
        let Some(key) = caller.repo.as_ref() else {
            return Vec::new();
        };
        let Some(folder) = repo_folder(key) else {
            return Vec::new();
        };
        let key = key.to_string_lossy();
        self.machines_online(cx)
            .iter()
            .flat_map(|view| {
                remote::remote_members(
                    &view.machine.id,
                    &view.machine.label,
                    &view.snapshot.agents,
                    &folder,
                    &key,
                )
            })
            .map(|(_, member, _)| member)
            .collect()
    }

    /// The caller as a roster member, when it may reach remote agents: a
    /// local, unarchived session.
    fn remote_caller(&self, caller: i64, cx: &App) -> Result<Member, Refusal> {
        let local = self.sessions.get(&caller).is_some_and(|view| {
            let view = view.read(cx);
            !view.is_remote() && !view.summary.archived
        });
        if !local {
            return Err(Refusal::new(
                Code::OutOfScope,
                "only a local session here can reach agents on herdr machines",
            ));
        }
        self.roster_member(caller, cx)
            .ok_or_else(|| Refusal::new(Code::NotFound, "this session isn't loaded"))
    }

    /// The remote agent `id` names, if `caller` may reach it: one it started,
    /// or one working in a checkout of the caller's repository.
    pub(super) fn remote_peer_for(&self, caller: i64, id: i64, cx: &App) -> Result<Peer, Refusal> {
        let member = self.remote_caller(caller, cx)?;
        if let Some(peer) = self.remote_started.get(&id) {
            return if peer.parent == caller {
                Ok(peer.clone())
            } else {
                Err(Refusal::new(
                    Code::OutOfScope,
                    format!("remote agent {id} was started by another session"),
                ))
            };
        }
        if !self.remote_siblings(&member, cx).iter().any(|m| m.id == id) {
            return Err(Refusal::new(
                Code::OutOfScope,
                format!("remote agent {id} doesn't work in this repository"),
            ));
        }
        self.machines_online(cx)
            .into_iter()
            .find_map(|view| {
                view.snapshot
                    .agents
                    .iter()
                    .find(|agent| remote::remote_id(&view.machine.id, &agent.pane_id) == id)
                    .map(|agent| Peer {
                        target: remote::Target {
                            pane: agent.pane_id.clone(),
                            name: agent.name.clone(),
                        },
                        title: agent.name.clone().unwrap_or_else(|| agent.pane_id.clone()),
                        machine: view.machine.clone(),
                        parent: 0,
                    })
            })
            .ok_or_else(|| Refusal::new(Code::NotFound, format!("no remote agent {id}")))
    }

    /// `sessions_start` with `machine`: the agent runs on that herdr.
    pub(super) fn answer_remote_start(
        &mut self,
        caller: i64,
        input: StartInput,
        reply: Reply,
        cx: &mut Context<Self>,
    ) {
        if let Err(refusal) = self.remote_start_policy(caller, &input, cx) {
            let _ = reply.send(Err(refusal.into()));
            return;
        }
        let label = input.machine.clone().unwrap_or_default();
        let Some(view) = self
            .machines_online(cx)
            .into_iter()
            .find(|view| view.machine.label == label || view.machine.id == label)
        else {
            let _ = reply.send(Err(Refusal::new(
                Code::NotFound,
                format!("no herdr machine \"{label}\" is online"),
            )
            .into()));
            return;
        };
        let machine = view.machine.clone();
        let remote = (self.remote_for)(&machine);
        let kind = input.harness.clone().unwrap_or_else(|| "claude".into());
        let title = input.title.clone().unwrap_or_else(|| {
            input
                .prompt
                .lines()
                .next()
                .unwrap_or_default()
                .chars()
                .take(80)
                .collect()
        });
        let name = format!(
            "{}-{}",
            remote::agent_name(&title)
                .chars()
                .take(26)
                .collect::<String>(),
            gespan_core::time::now_ms() % 10_000
        );
        let cwd = input.cwd.clone().unwrap_or_default();
        let worktree = input.workspace == WorkspaceChoice::Worktree;
        let base = input.base.clone();
        let prompt = input.prompt.clone();
        let work = cx.background_executor().spawn(async move {
            remote::start_on(
                remote.as_ref(),
                &kind,
                &name,
                &cwd,
                worktree,
                base.as_deref(),
                &prompt,
            )
        });
        *self.starting.entry(caller).or_default() += 1;
        cx.spawn(async move |this, cx| {
            let started = work.await;
            let answer = this.update(cx, |this, cx| {
                if let Some(count) = this.starting.get_mut(&caller) {
                    *count = count.saturating_sub(1);
                }
                let started = started?;
                let id = remote::remote_id(&machine.id, &started.pane_id);
                this.remote_started.insert(
                    id,
                    Peer {
                        machine: machine.clone(),
                        target: remote::Target {
                            pane: started.pane_id.clone(),
                            name: Some(started.name.clone()),
                        },
                        title: title.clone(),
                        parent: caller,
                    },
                );
                this.push_rosters(cx);
                Ok(json!({
                    "id": id,
                    "title": title,
                    "machine": machine.label,
                    "cwd": started.cwd,
                    "workspace": if worktree { "worktree" } else { "main" },
                    "name": started.name,
                }))
            });
            let _ = reply.send(answer.unwrap_or_else(|_| Err(UNAVAILABLE.into())));
        })
        .detach();
    }

    /// What a remote start must pass. A herdr agent runs interactively on
    /// its machine, asking there before it acts: as `supervised` at best. So
    /// a read-only or plan turn can't start one; the depth and running
    /// limits count remote children with local ones; and it runs in a
    /// checkout named like the caller's repository.
    fn remote_start_policy(
        &self,
        caller: i64,
        input: &StartInput,
        cx: &App,
    ) -> Result<(), Refusal> {
        use gespan_core::agent::AccessMode;
        use gespan_core::orchestrator::tree::{Command, State, decide};
        let member = self.remote_caller(caller, cx)?;
        let view = self.sessions.get(&caller).map(|view| view.read(cx));
        let caller_access = view
            .map(super::orchestrator_tree::effective_access)
            .unwrap_or(AccessMode::ReadOnly);
        let folder = member.repo.as_deref().and_then(repo_folder);
        let cwd = input.cwd.clone().unwrap_or_default();
        let same_repo = folder
            .as_ref()
            .is_some_and(|folder| remote::names_folder(&cwd, folder));
        if !same_repo {
            return Err(Refusal::new(
                Code::OutOfScope,
                "a remote agent runs in a checkout of this repository: cwd must name its folder",
            ));
        }
        let remote_children = self
            .remote_started
            .values()
            .filter(|peer| peer.parent == caller)
            .count();
        let starting = self.starting.get(&caller).copied().unwrap_or(0) + remote_children;
        let live = self.live_sessions(cx);
        let mut checked = input.clone();
        checked.access = Some(AccessMode::Supervised);
        decide(
            &State {
                tree: &self.session_tree,
                live: &live,
            },
            Command::Start {
                caller,
                caller_access,
                starting,
                input: Box::new(checked),
            },
        )
        .map(drop)
    }

    /// `sessions_message` to a remote agent: typed once it has settled,
    /// labelled and fenced like any mail. Answered once it is queued.
    pub(super) fn answer_remote_message(
        &mut self,
        caller: i64,
        peer: Peer,
        text: String,
        reply: Reply,
        cx: &mut Context<Self>,
    ) {
        // The remote agent acts as supervised; a narrower sender can't make
        // it act for it.
        let sender = self
            .sessions
            .get(&caller)
            .map(|view| super::orchestrator_tree::effective_access(view.read(cx)));
        if let Err(_held) =
            mailbox::admit_known_access(sender, Some(gespan_core::agent::AccessMode::Supervised))
        {
            let _ = reply.send(Err(Refusal::new(
                Code::PermissionEscalation,
                "this session can't message an agent that may do more than it",
            )
            .into()));
            return;
        }
        let mail = Mail {
            id: String::new(),
            to: 0,
            from: Some(caller),
            from_title: self.session_metadata_title(caller, cx),
            kind: MailKind::Message,
            text,
            hops: 1,
            created_ms: gespan_core::time::now_ms(),
            state: MailState::Queued,
            held: None,
            released: false,
            sender_access: sender,
        };
        let turn = mailbox::turn_text(&[&mail]);
        let remote = (self.remote_for)(&peer.machine);
        let target = peer.target.clone();
        let answer = json!({
            "to": peer.title,
            "machine": peer.machine.label,
            "state": "queued",
        });
        // The wait can be long; it runs on a thread of its own. The caller
        // hears at once if the agent is gone or was replaced.
        std::thread::spawn(move || {
            if let Err(error) = remote::check(remote.as_ref(), &target) {
                let _ = reply.send(Err(Refusal::new(Code::NotFound, error).into()));
                return;
            }
            let _ = reply.send(Ok(answer));
            if let Err(error) = remote::deliver_when_idle(
                remote.as_ref(),
                &target,
                &turn,
                DELIVER_WAIT.as_millis() as u64,
            ) {
                tracing::warn!(%error, "a message to a remote agent was not delivered");
            }
        });
    }

    pub(super) fn answer_remote_read(&mut self, peer: Peer, reply: Reply, cx: &mut Context<Self>) {
        let remote = (self.remote_for)(&peer.machine);
        cx.background_executor()
            .spawn(async move {
                if let Err(error) = remote::check(remote.as_ref(), &peer.target) {
                    let _ = reply.send(Err(Refusal::new(Code::NotFound, error).into()));
                    return;
                }
                let answer = remote::read_from(remote.as_ref(), &peer.target)
                    .map(|text| json!({"machine": peer.machine.label, "recent": text}));
                let _ = reply.send(answer);
            })
            .detach();
    }

    pub(super) fn answer_remote_wait(
        &mut self,
        peer: Peer,
        id: i64,
        timeout: Duration,
        reply: Reply,
    ) {
        let remote = (self.remote_for)(&peer.machine);
        std::thread::spawn(move || {
            if let Err(error) = remote::check(remote.as_ref(), &peer.target) {
                let _ = reply.send(Err(Refusal::new(Code::NotFound, error).into()));
                return;
            }
            let answer =
                match remote::wait_on(remote.as_ref(), &peer.target, timeout.as_millis() as u64) {
                    Ok(status) => {
                        Ok(json!({"ready": [{"id": id, "status": status}], "timedOut": false}))
                    }
                    Err(error) if error.contains("timeout") => {
                        Ok(json!({"ready": [], "timedOut": true}))
                    }
                    Err(error) => Err(error),
                };
            let _ = reply.send(answer);
        });
    }
}
