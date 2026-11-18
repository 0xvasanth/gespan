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
    agent::Settings,
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

/// How much of a child's last reply its report quotes.
const REPORT_REPLY: usize = 2_000;
const REPORT: &str = "report";

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

/// A request id for a start that brought none, unique in this process.
fn fresh_request() -> String {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    format!(
        "auto-{}-{}",
        gespan_core::time::now_ms(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

/// The `/operator` start's input for the same request, so both share one
/// launch path until `/operator` is retired.
fn launch_input(input: &StartInput, access: gespan_core::agent::AccessMode) -> Value {
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
        let live = self.live_sessions(cx);
        let decided = decide(
            &State {
                tree: &self.session_tree,
                live: &live,
            },
            Command::Start {
                caller,
                caller_access: settings.access,
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
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            create_child(
                &store, caller, &project, &provider, kind, settings, &launch, &request,
            )
        });
        cx.spawn(async move |this, cx| {
            let created = work.await;
            let answer = this.update(cx, |this, cx| {
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
        if self.session_tree.children(id).is_empty() {
            return;
        }
        self.flush_reports(id, cx);
    }

    /// Saves a report for `parent`, then tries to deliver it.
    fn queue_report(
        &mut self,
        parent: i64,
        child: i64,
        outcome: Outcome,
        turn: u64,
        cx: &mut Context<Self>,
    ) {
        let Some(view) = self.sessions.get(&child) else {
            return;
        };
        let view = view.read(cx);
        let title = view.summary.title.clone();
        let reply = match outcome {
            Outcome::WaitingOnUser => view
                .pending_input(cx)
                .map(|input| input.label)
                .unwrap_or_default(),
            _ => view.last_reply(cx),
        };
        let reply: String = reply.chars().take(REPORT_REPLY).collect();
        let effect = match outcome {
            Outcome::WaitingOnUser => format!("report:{child}:{turn}:waiting"),
            _ => format!("report:{child}:{turn}"),
        };
        let payload = json!({
            "child": child,
            "title": title,
            "outcome": outcome,
            "reply": reply,
        });
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let store = locked(&store)?;
            store.queue_orch_effect(&effect, REPORT, parent, &payload)?;
            store.append_orch_event("report-queued", parent, &payload)
        });
        cx.spawn(async move |this, cx| {
            if work.await.is_ok() {
                let _ = this.update(cx, |this, cx| this.flush_reports(parent, cx));
            }
        })
        .detach();
    }

    /// Delivers every report waiting for `parent` as one turn, if `parent`
    /// is idle: not working, not held, not waiting on the user, with nothing
    /// typed or queued. A parent that isn't open is loaded off-screen; an
    /// archived one keeps its reports until it is unarchived.
    fn flush_reports(&mut self, parent: i64, cx: &mut Context<Self>) {
        if self.reports_flushing.contains(&parent) {
            return;
        }
        if self
            .session_summaries
            .get(&parent)
            .is_some_and(|summary| summary.archived)
        {
            return;
        }
        let Some(view) = self.ensure_session(parent, cx) else {
            return;
        };
        let idle = {
            let live = view.read(cx);
            !live.summary.archived
                && !live.needs_approval()
                && !self.session_held(parent, Some(live), cx)
                && live.available_for_managed_run(cx)
        };
        if !idle {
            return;
        }
        self.reports_flushing.insert(parent);
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let pending = locked(&store)?.pending_orch_effects(Some(REPORT))?;
            Ok::<_, String>(
                pending
                    .into_iter()
                    .filter(|effect| effect.session == parent)
                    .collect::<Vec<_>>(),
            )
        });
        cx.spawn(async move |this, cx| {
            let pending = work.await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.reports_flushing.remove(&parent);
                if pending.is_empty() {
                    return;
                }
                let Some(view) = this.sessions.get(&parent).cloned() else {
                    return;
                };
                // The parent may have started a turn meanwhile.
                if !view.read(cx).available_for_managed_run(cx) {
                    return;
                }
                let text = report_text(
                    &pending
                        .iter()
                        .map(|effect| &effect.payload)
                        .collect::<Vec<_>>(),
                );
                view.update(cx, |view, cx| view.send_worker_turn(text, cx));
                let ids: Vec<String> = pending.into_iter().map(|effect| effect.id).collect();
                let store = this.store.clone();
                cx.background_executor()
                    .spawn(async move {
                        if let Ok(store) = locked(&store) {
                            for id in ids {
                                let _ = store.finish_orch_effect(&id, None);
                            }
                        }
                    })
                    .detach();
            });
        })
        .detach();
    }

    /// At start: reports saved before the app last closed are delivered.
    pub(super) fn resume_wake_reports(&mut self, cx: &mut Context<Self>) {
        let store = self.store.clone();
        let work = cx.background_executor().spawn(async move {
            let pending = locked(&store)?.pending_orch_effects(Some(REPORT))?;
            Ok::<_, String>(
                pending
                    .into_iter()
                    .map(|effect| effect.session)
                    .collect::<HashSet<_>>(),
            )
        });
        cx.spawn(async move |this, cx| {
            let parents = work.await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                for parent in parents {
                    this.flush_reports(parent, cx);
                }
            });
        })
        .detach();
    }
}

/// One turn's text for every report waiting for a parent, each labelled with
/// the child that sent it.
fn report_text(reports: &[&Value]) -> String {
    reports
        .iter()
        .map(|report| {
            let title = report["title"].as_str().unwrap_or("A session");
            let outcome: Outcome =
                serde_json::from_value(report["outcome"].clone()).unwrap_or(Outcome::Finished);
            let mut text = format!(
                "[From session \"{}\" (id {})] It {}.",
                title.replace('"', "'"),
                report["child"],
                outcome.describe()
            );
            if let Some(reply) = report["reply"].as_str().filter(|r| !r.trim().is_empty()) {
                text.push_str("\n\n");
                text.push_str(reply.trim());
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n")
}
