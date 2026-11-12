//! The shell's side of My tasks: the page entity, its count for the sidebar,
//! opening it, and the snapshot of everything waiting, built from every chat,
//! the Pilots and the Flow store.
use super::*;
use crate::conversation::approval_card::Card;
use crate::my_tasks::{Body, MyTasks, Source, TaskItem};
use gespan_core::tasks::{self, History, Kind};

/// Turns of history a task opens with, and how many each "load more" adds.
pub(super) const HISTORY_TURNS: usize = 4;

impl Shell {
    /// The page, created the first time anything needs it.
    pub(super) fn ensure_my_tasks(&mut self, cx: &mut Context<Self>) -> Entity<MyTasks> {
        if let Some(view) = &self.my_tasks {
            return view.clone();
        }
        let view = cx.new(MyTasks::new);
        self.my_tasks = Some(view.clone());
        view
    }

    /// Open tasks, for the sidebar count.
    pub(super) fn my_task_count(&self, cx: &App) -> usize {
        self.my_tasks
            .as_ref()
            .map_or(0, |view| view.read(cx).open_count())
    }

    /// Hands the page a fresh snapshot and redraws the sidebar count when it
    /// changes.
    pub(super) fn set_my_tasks(&mut self, tasks: Vec<TaskItem>, cx: &mut Context<Self>) {
        let before = self.my_task_count(cx);
        let view = self.ensure_my_tasks(cx);
        let now = gespan_core::time::now_ms();
        view.update(cx, |view, cx| view.set_tasks(tasks, now, cx));
        if self.my_task_count(cx) != before {
            self.notify_navigation(cx);
        }
    }

    /// Rebuilds the list from what is waiting now. Each task keeps the time
    /// Gespan first saw it, so its wait grows across refreshes.
    pub(crate) fn refresh_my_tasks(&mut self, cx: &mut Context<Self>) {
        let mut tasks = self.collect_tasks(cx);
        let keys: HashSet<String> = tasks.iter().map(|t| t.key.clone()).collect();
        self.my_task_seen.retain(|key, _| keys.contains(key));
        self.my_task_turns.retain(|key, _| keys.contains(key));
        for task in &mut tasks {
            task.since_ms = *self
                .my_task_seen
                .entry(task.key.clone())
                .or_insert(task.since_ms);
        }
        self.set_my_tasks(tasks, cx);
    }

    fn collect_tasks(&self, cx: &App) -> Vec<TaskItem> {
        let now = gespan_core::time::now_ms();
        let turns = |key: &str| {
            self.my_task_turns
                .get(key)
                .copied()
                .unwrap_or(HISTORY_TURNS)
        };
        let mut found = Vec::new();
        for view in self.notification_chats(cx) {
            let chat = view.read(cx);
            let id = chat.summary.id;
            let project = chat.summary.cwd.clone();
            let cwd = (!project.starts_with("remote://")).then_some(project.as_str());
            let harness = harness_label(&chat.summary.provider);
            let title = chat.display_title().to_owned();
            let rows = chat.history_rows(cx);
            for approval in chat.pending_approvals() {
                let key = format!("approval:{id}:{}", approval.id);
                found.push(TaskItem {
                    history: tasks::history(rows, cwd, turns(&key)),
                    headline: Card::new(approval, cwd).title(approval),
                    key,
                    kind: Kind::Approval,
                    project: project.clone(),
                    who: format!("{harness} wants approval"),
                    source_label: title.clone(),
                    source: Source::Session(id),
                    body: Body::Approval {
                        approval: approval.clone(),
                        cwd: cwd.map(str::to_owned),
                    },
                    since_ms: now,
                });
            }
            if let Some(card) = chat.front_question() {
                let question = card.read(cx);
                let key = format!("question:{id}:{}", question.id());
                found.push(TaskItem {
                    history: tasks::history(rows, cwd, turns(&key)),
                    headline: question.headline().to_owned(),
                    key,
                    kind: Kind::Question,
                    project: project.clone(),
                    who: format!("{harness} asks"),
                    source_label: title.clone(),
                    source: Source::Session(id),
                    body: Body::Question(card.clone()),
                    since_ms: now,
                });
            }
        }
        if let Some(pilots) = &self.pilots {
            for (pilot, view, open) in pilots.read(cx).open_choices(cx) {
                let chat = view.read(cx);
                let session = chat.summary.id;
                let key = format!("choice:{session}:{}", open.row);
                let rows = &chat.history_rows(cx)[..open.row];
                let history = tasks::history(rows, None, turns(&key));
                let name = chat.display_title().to_owned();
                found.push(TaskItem {
                    headline: last_agent_line(&history)
                        .unwrap_or_else(|| "Choose an option".into()),
                    history,
                    key,
                    kind: Kind::PilotChoice,
                    project: chat.summary.cwd.clone(),
                    who: format!("{name} asks"),
                    source_label: name,
                    source: Source::Pilot { pilot, session },
                    body: Body::Choice {
                        options: open.options,
                    },
                    since_ms: open.at.unwrap_or(now),
                });
            }
        }
        if let Some(store) = &self.flows_state.store
            && let Ok(store) = store.lock()
            && let Ok(runs) = store.active_runs()
        {
            for run in runs {
                let Ok(visits) = store.visits(&run.id) else {
                    continue;
                };
                let Some(visit) = tasks::waiting_visit(&visits) else {
                    continue;
                };
                let short: String = run.id.chars().take(8).collect();
                found.push(TaskItem {
                    key: format!("flow:{}:{}", run.id, visit.id),
                    kind: Kind::FlowApproval,
                    project: run.flow.cwd.clone(),
                    headline: format!("Approve step \u{201c}{}\u{201d}?", visit.state_id),
                    who: format!("{} Flow", run.flow.name),
                    source_label: format!("{} · run {short}", run.flow.name),
                    source: Source::FlowRun(run.id.clone()),
                    body: Body::Flow {
                        prompt: visit.prompt.clone().unwrap_or_default(),
                    },
                    since_ms: u64::try_from(visit.started_at).unwrap_or(now),
                    history: tasks::flow_history(&visits),
                });
            }
        }
        found
    }

    pub(super) fn open_my_tasks(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.prepare_app_view(window, cx);
        self.ensure_my_tasks(cx);
        self.show_app_view(app_views::AppView::MyTasks, window, cx);
    }

    #[cfg(test)]
    pub(super) fn show_my_tasks_for_test(&mut self, tasks: Vec<TaskItem>, cx: &mut Context<Self>) {
        self.set_my_tasks(tasks, cx);
    }
}

/// The harness's name, as the toasts show it.
fn harness_label(provider: &str) -> &'static str {
    gespan_core::provider::Kind::from_id(provider).label()
}

/// The first line of the agent's last message, for a card that has no
/// question text of its own.
fn last_agent_line(history: &History) -> Option<String> {
    history.turns.iter().rev().find_map(|turn| {
        turn.entries.iter().rev().find_map(|entry| match entry {
            tasks::Entry::Agent { text, .. } => {
                let line = text.lines().find(|l| !l.trim().is_empty())?.trim();
                let clipped: String = line.chars().take(140).collect();
                Some(if clipped.len() < line.len() {
                    format!("{clipped}…")
                } else {
                    clipped
                })
            }
            _ => None,
        })
    })
}

#[cfg(test)]
impl Shell {
    /// (kind, headline) of each task, oldest first.
    pub(crate) fn my_task_snapshot_for_test(&self, cx: &App) -> Vec<(Kind, String)> {
        self.my_tasks.as_ref().map_or_else(Vec::new, |view| {
            view.read(cx)
                .tasks()
                .iter()
                .map(|t| (t.kind, t.headline.clone()))
                .collect()
        })
    }
}
