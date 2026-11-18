//! The app's side of `sessions_start` and `sessions_cancel`: a chat starts a
//! child session that runs on its own, nested under it, and the child's stop
//! is reported back to the chat as a new turn once the chat is idle.
//!
//! Decisions come from the pure [`decide`]; this file only applies them.
//! What must survive a restart (the link, the request's receipt, a report
//! not yet delivered) is written to the store before it is acted on.
use super::Shell;
use crate::runtime::{Database, locked};
use gespan_core::{
    agent::{AccessMode, Settings},
    app_access as app,
    orchestrator::{
        bridge::UNAVAILABLE,
        policy::{Code, Refusal},
        start::{StartInput, WorkspaceChoice},
        tree::{Command, Effect, Launch, Link, Origin, Outcome, State, decide},
    },
    provider::Kind,
    store::Session,
};
use gpui::Context;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    path::Path,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
};

type Reply = mpsc::Sender<Result<Value, String>>;

/// A child the store now holds, for the window to show and run.
struct Started {
    summary: Session,
    settings: Settings,
    link: Link,
    value: Value,
}

enum Created {
    /// The same request ran before: its first answer.
    Again(Value),
    New(Box<Started>),
}

/// The mode a session's turns run in: a plan, orchestrator or read-only
/// turn is read-only whatever its composer says, and so is what it starts
/// or asks of others.
pub(crate) fn effective_access(view: &crate::conversation::Conversation) -> AccessMode {
    if view.app_read_only || view.settings.plan {
        AccessMode::ReadOnly
    } else {
        view.settings.access
    }
}

/// A request id for a start that brought none, unique in this process.
fn fresh_request() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "auto-{}-{}",
        gespan_core::time::now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// The app launch input for this request, in the shape the shared launch
/// helpers (`app_access::launch_settings`, `launch_checkout`) read.
fn launch_input(input: &StartInput, access: AccessMode) -> Value {
    let mut fields = json!({
        "prompt": input.prompt,
        "runtimeMode": access.id(),
        "workspaceMode": "current",
    });
    if let Some(model) = &input.model {
        fields["model"] = json!(model);
    }
    if input.workspace == WorkspaceChoice::Worktree {
        fields["workspaceMode"] = json!("worktree");
    }
    if let WorkspaceChoice::Existing(path) = &input.workspace {
        fields["worktreeCwd"] = json!(path);
    }
    fields
}

/// The sidebar title: the one asked for, or the prompt's first line.
fn title_of(input: &StartInput) -> String {
    input.title.clone().unwrap_or_else(|| {
        input
            .prompt
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or_default()
            .trim()
            .chars()
            .take(80)
            .collect()
    })
}

/// Runs on a worker: records the child, makes its checkout and leaves its
/// prompt queued, or returns the answer a repeated request already got. The
/// store's lock is never held while git or setup scripts run.
#[allow(clippy::too_many_arguments)]
fn create_child(
    store: &Database,
    caller: i64,
    project: &str,
    provider: &str,
    kind: Kind,
    settings: Settings,
    launch: &Launch,
    request: &str,
) -> Result<Created, String> {
    let command = format!("start:{caller}:{request}");
    if let Some(value) = locked(store)?.command_receipt(&command)? {
        return Ok(Created::Again(value));
    }
    let input = launch_input(&launch.input, launch.access);
    let models = if launch.input.model.is_some() {
        gespan_core::catalog::models_for(kind, &settings.account)?
    } else {
        gespan_core::catalog::cached_models_for(kind, &settings.account).unwrap_or_default()
    };
    let mut settings = app::launch_settings(&input, provider, settings, kind, &models)?;
    settings.revision = 0;
    settings.access = launch.access;
    let title = title_of(&launch.input);
    let fingerprint = app::fingerprint("sessions_start", &input);
    let (mut summary, fresh) =
        locked(store)?.create_app_session(caller, request, &fingerprint, kind.id(), project)?;
    if !fresh {
        // Created before a crash, before its receipt was saved.
        return Ok(Created::Again(
            json!({"id": summary.id, "title": summary.title}),
        ));
    }
    let checkout = match &launch.input.workspace {
        WorkspaceChoice::Main => {
            settings.worktree_cwd = None;
            Ok(())
        }
        WorkspaceChoice::Worktree => gespan_core::git::create_task_worktree(
            Path::new(project),
            launch.input.base.as_deref().unwrap_or("HEAD"),
            summary.id,
            &title,
        )
        .map(|path| settings.worktree_cwd = Some(path.to_string_lossy().into_owned())),
        WorkspaceChoice::Existing(_) => {
            app::launch_checkout(&input, project, summary.id, &mut settings)
        }
    };
    let mut store = locked(store)?;
    if let Err(error) = checkout {
        let _ = store.delete(summary.id);
        return Err(error);
    }
    store.title(summary.id, &title)?;
    summary.title = title.clone();
    store.configure_agent(summary.id, kind.id(), &settings)?;
    store.save_started_link(summary.id, caller)?;
    store.prepare_app_prompt(summary.id, request, &launch.input.prompt)?;
    summary.has_messages = true;
    let link = Link {
        child: summary.id,
        parent: caller,
        root: launch.root,
        depth: launch.depth,
        origin: Origin::Chat,
        wake: launch.wake,
    };
    store.save_session_link(&link)?;
    let value = json!({
        "id": summary.id,
        "title": title,
        "cwd": settings.worktree_cwd.clone().unwrap_or_else(|| project.to_owned()),
        "workspace": if settings.worktree_cwd.is_some() { "worktree" } else { "main" },
        "harness": kind.id(),
        "access": settings.access.id(),
        "wake": launch.wake.id(),
        "depth": launch.depth,
    });
    if let Some(earlier) =
        store.save_command_receipt(&command, caller, "start", summary.id, &value)?
    {
        return Ok(Created::Again(earlier));
    }
    Ok(Created::New(Box::new(Started {
        summary,
        settings,
        link,
        value,
    })))
}

impl Shell {
    /// Sessions whose agent is working, waiting on the user, or about to
    /// start: what counts toward a parent's live children.
    fn live_sessions(&self, cx: &gpui::App) -> HashSet<i64> {
        self.session_tree
            .links()
            .map(|link| link.child)
            .filter(|id| {
                self.launching.contains(id)
                    || self.sessions.get(id).is_some_and(|view| {
                        let view = view.read(cx);
                        view.busy || view.needs_approval()
                    })
            })
            .collect()
    }

    pub(super) fn answer_start(
        &mut self,
        caller: i64,
        input: StartInput,
        reply: Reply,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.sessions.get(&caller) else {
            let _ = reply.send(Err(UNAVAILABLE.into()));
            return;
        };
        let view = view.read(cx);
        if view.is_remote() {
            let _ = reply.send(Err(Refusal::new(
                Code::OutOfScope,
                "a session on another machine can't start sessions here yet",
            )
            .into()));
            return;
        }
        let project = view.summary.cwd.clone();
        let provider = view.summary.provider.clone();
        let mut settings = view.settings.clone();
        let caller_access = effective_access(view);
        let live = self.live_sessions(cx);
        let starting = self.starting.get(&caller).copied().unwrap_or(0);
        let decided = decide(
            &State {
                tree: &self.session_tree,
                live: &live,
            },
            Command::Start {
                caller,
                caller_access,
                starting,
                input,
            },
        );
        let launch = match decided {
            Ok(effects) => match effects.into_iter().next() {
                Some(Effect::Launch(launch)) => launch,
                _ => return,
            },
            Err(refusal) => {
                let _ = reply.send(Err(refusal.into()));
                return;
            }
        };
        let kind = launch
            .input
            .harness
            .as_deref()
            .map_or_else(|| Kind::from_id(&provider), Kind::from_id);
        if kind.id() != provider {
            settings.account = crate::accounts::get(cx)
                .map(|center| center.read(cx).selected(kind, &project))
                .unwrap_or_default();
        }
        let request = launch
            .input
            .request_id
            .clone()
            .unwrap_or_else(fresh_request);
        // Counted as live until it is answered, so parallel calls can't
        // start more than the limit.
        *self.starting.entry(caller).or_default() += 1;
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            create_child(
                &store, caller, &project, &provider, kind, settings, &launch, &request,
            )
        });
        cx.spawn(async move |this, cx| {
            let created = work.await;
            let answer = this.update(cx, |this, cx| {
                if let Some(count) = this.starting.get_mut(&caller) {
                    *count = count.saturating_sub(1);
                }
                let started = match created? {
                    Created::Again(value) => return Ok(value),
                    Created::New(started) => *started,
                };
                let value = started.value.clone();
                this.install_child(started, cx);
                Ok(value)
            });
            let _ = reply.send(answer.unwrap_or_else(|_| Err(UNAVAILABLE.into())));
        })
        .detach();
    }

    /// Shows a new child under its parent and sends its first turn, without
    /// taking focus from what the user is doing.
    fn install_child(&mut self, started: Started, cx: &mut Context<Self>) {
        let Started {
            summary,
            settings,
            link,
            ..
        } = started;
        let id = summary.id;
        self.draft_sessions.insert(id);
        self.sidebar_hidden_sessions.remove(&id);
        self.session_worktrees
            .insert(id, settings.worktree_cwd.clone());
        self.session_tree.insert(link);
        self.launching.insert(id);
        // Its first turn is one hop from the turn that started it.
        let hops = self.turn_hops.get(&link.parent).copied().unwrap_or(0);
        self.turn_hops.insert(id, hops.saturating_add(1));
        self.add(summary, cx);
        self.started_by.insert(id, link.parent);
        if let Some(view) = self.sessions.get(&id) {
            view.update(cx, |view, cx| view.install_app_launch(settings, true, cx));
        }
        self.refresh_child_watches(link.parent, cx);
        self.persist(cx);
        self.invalidate_chrome(cx);
        cx.notify();
    }

    /// The parent's Watching chip lists the children it will hear back from:
    /// those still running whose wake isn't `never`. A Pilot chat keeps the
    /// list its Pilot sets.
    fn refresh_child_watches(&mut self, parent: i64, cx: &mut Context<Self>) {
        use gespan_core::{
            orchestrator::tree::Wake,
            watching::{Source, Watch},
        };
        let Some(view) = self.sessions.get(&parent).cloned() else {
            return;
        };
        if view.read(cx).settings.pilot.is_some() {
            return;
        }
        let live = self.live_sessions(cx);
        let watches: Vec<Watch> = self
            .session_tree
            .children(parent)
            .into_iter()
            .filter(|child| live.contains(child))
            .filter(|child| {
                self.session_tree
                    .link(*child)
                    .is_some_and(|link| link.wake != Wake::Never)
            })
            .map(|child| Watch {
                id: format!("session-{child}"),
                source: Source::Session(child),
                label: self.session_metadata_title(child, cx),
                detail: "Reports back here when it finishes".into(),
                started_at: None,
            })
            .collect();
        view.update(cx, |view, cx| view.set_delegated_watches(watches, cx));
    }

    /// The user stopped waiting on a child from the Watching chip: it keeps
    /// running, but no longer reports back.
    pub(super) fn stop_watching_child(&mut self, parent: i64, child: i64, cx: &mut Context<Self>) {
        let Some(mut link) = self.session_tree.link(child).copied() else {
            return;
        };
        if link.parent != parent {
            return;
        }
        link.wake = gespan_core::orchestrator::tree::Wake::Never;
        self.session_tree.insert(link);
        self.refresh_child_watches(parent, cx);
        let store = self.store.clone();
        cx.background_executor()
            .spawn(async move {
                if let Ok(store) = locked(&store) {
                    let _ = store.save_session_link(&link);
                }
            })
            .detach();
    }

    /// Everything below `id` that is still running, leaves first.
    pub(super) fn running_descendants(&self, id: i64, cx: &gpui::App) -> Vec<i64> {
        let live = self.live_sessions(cx);
        self.session_tree
            .descendants(id)
            .into_iter()
            .filter(|child| live.contains(child))
            .collect()
    }

    /// The user's "Stop the sessions it started": the same stop a parent's
    /// `sessions_cancel` makes, on every running descendant.
    pub(super) fn stop_started(&mut self, id: i64, cx: &mut Context<Self>) {
        for child in self.running_descendants(id, cx) {
            self.launching.remove(&child);
            if let Some(view) = self.sessions.get(&child).cloned() {
                view.update(cx, |view, cx| view.stop_app_session(cx));
            }
        }
    }

    /// A Pilot's `sessions.start` goes through the same limits as
    /// `sessions_start`: depth, running children and permission ordering.
    /// Returns the mode the child runs in.
    pub(crate) fn pilot_start_policy(
        &mut self,
        caller: i64,
        caller_access: AccessMode,
        requested: Option<AccessMode>,
        cx: &gpui::App,
    ) -> Result<AccessMode, String> {
        let starting = self.starting.get(&caller).copied().unwrap_or(0);
        let live = self.live_sessions(cx);
        let input = StartInput {
            prompt: "pilot".into(),
            harness: None,
            model: None,
            workspace: WorkspaceChoice::Main,
            base: None,
            wake: gespan_core::orchestrator::tree::Wake::Never,
            access: requested,
            title: None,
            request_id: None,
        };
        let effects = decide(
            &State {
                tree: &self.session_tree,
                live: &live,
            },
            Command::Start {
                caller,
                caller_access,
                starting,
                input,
            },
        )?;
        // Counted as live until it is made or fails, so parallel calls
        // can't start more than the limit.
        *self.starting.entry(caller).or_default() += 1;
        match effects.into_iter().next() {
            Some(Effect::Launch(launch)) => Ok(launch.access),
            _ => Ok(caller_access),
        }
    }

    /// Records that a Pilot's chat or habit run started `child`. The Pilot
    /// delivers its own reports (to its chat's outbox or the habit's
    /// mailbox), so the link never wakes the parent itself.
    pub(crate) fn pilot_link(&mut self, caller: i64, child: i64, cx: &mut Context<Self>) {
        if let Some(count) = self.starting.get_mut(&caller) {
            *count = count.saturating_sub(1);
        }
        self.child_link(caller, child, Origin::Pilot, cx);
    }

    /// A Pilot's start that failed before it made a session.
    pub(crate) fn pilot_start_failed(&mut self, caller: i64) {
        if let Some(count) = self.starting.get_mut(&caller) {
            *count = count.saturating_sub(1);
        }
    }

    /// Links `child` under `parent` for a caller that reports by itself (a
    /// Pilot, an orchestration lead), and saves the link.
    pub(crate) fn child_link(
        &mut self,
        parent: i64,
        child: i64,
        origin: Origin,
        cx: &mut Context<Self>,
    ) {
        let link = Link {
            child,
            parent,
            root: self.session_tree.root(parent),
            depth: self.session_tree.depth(parent) + 1,
            origin,
            wake: gespan_core::orchestrator::tree::Wake::Never,
        };
        self.session_tree.insert(link);
        self.started_by.insert(child, parent);
        let store = self.store.clone();
        cx.background_executor()
            .spawn(async move {
                if let Ok(store) = locked(&store) {
                    let _ = store.save_session_link(&link);
                }
            })
            .detach();
    }

    pub(super) fn answer_cancel(
        &mut self,
        caller: i64,
        target: i64,
        reply: Reply,
        cx: &mut Context<Self>,
    ) {
        let live = HashSet::new();
        let decided = decide(
            &State {
                tree: &self.session_tree,
                live: &live,
            },
            Command::Cancel { caller, target },
        );
        let order = match decided {
            Ok(effects) => effects
                .into_iter()
                .find_map(|effect| match effect {
                    Effect::Stop(order) => Some(order),
                    _ => None,
                })
                .unwrap_or_default(),
            Err(refusal) => {
                let _ = reply.send(Err(refusal.into()));
                return;
            }
        };
        for id in &order {
            self.launching.remove(id);
            if let Some(view) = self.ensure_session(*id, cx) {
                view.update(cx, |view, cx| view.stop_app_session(cx));
            }
        }
        let store = self.store.clone();
        let stopped = order.clone();
        cx.background_executor()
            .spawn(async move {
                if let Ok(store) = locked(&store) {
                    let _ = store.append_orch_event("cancel", caller, &json!({"stopped": stopped}));
                }
            })
            .detach();
        let _ = reply.send(Ok(json!({ "stopped": order })));
    }

    /// After any change to `id`: whether its turn ended or it started waiting
    /// on the user (which may wake its parent), and whether reports waiting
    /// for it can now be delivered.
    pub(super) fn orchestrator_session_changed(&mut self, id: i64, cx: &mut Context<Self>) {
        let Some(view) = self.sessions.get(&id) else {
            return;
        };
        let view = view.read(cx);
        let (ends, last, waiting) = (view.turn_ends, view.last_turn, view.needs_approval());
        let mut outcomes = Vec::new();
        if self.seen_turn_ends.get(&id).copied().unwrap_or(0) != ends {
            self.seen_turn_ends.insert(id, ends);
            self.launching.remove(&id);
            outcomes.extend(last);
        }
        if !waiting {
            self.waiting_reported.remove(&id);
        } else if self.session_tree.link(id).is_some() && self.waiting_reported.insert(id) {
            outcomes.push(Outcome::WaitingOnUser);
        }
        for outcome in outcomes {
            let live = HashSet::new();
            let effects = decide(
                &State {
                    tree: &self.session_tree,
                    live: &live,
                },
                Command::Settled {
                    session: id,
                    outcome,
                },
            )
            .unwrap_or_default();
            for effect in effects {
                if let Effect::Report {
                    parent,
                    child,
                    outcome,
                } = effect
                {
                    self.queue_report(parent, child, outcome, ends, cx);
                }
            }
        }
        if let Some(parent) = self.session_tree.parent(id) {
            self.refresh_child_watches(parent, cx);
        }
        self.orchestrator_typed(id, cx);
        self.deliver_mail(id, cx);
        self.resolve_waits(cx);
    }
}
