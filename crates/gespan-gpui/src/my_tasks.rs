//! My tasks: every approval, question, Flow approval and Pilot choice waiting
//! on the user, from every session, answered in place.
use crate::questions::QuestionCard;
use crate::theme::*;
use gespan_core::agent::{Approval, Decision};
use gespan_core::tasks::{History, Kind};
use gpui::{Context, Entity, EventEmitter, FocusHandle, Focusable, Window, div, prelude::*};

/// Where a task comes from, so the page can open it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Session(i64),
    FlowRun(String),
    Pilot { pilot: String, session: i64 },
}

/// What the card shows and answers.
#[derive(Clone)]
pub(crate) enum Body {
    Approval {
        approval: Approval,
        cwd: Option<String>,
    },
    Question(Entity<QuestionCard>),
    Flow {
        prompt: String,
    },
    Choice {
        options: Vec<String>,
    },
}

/// One thing waiting on the user.
#[derive(Clone)]
pub(crate) struct TaskItem {
    /// Stable across refreshes: kind, source and request.
    pub key: String,
    pub kind: Kind,
    pub project: String,
    pub headline: String,
    pub who: String,
    pub source_label: String,
    pub source: Source,
    pub body: Body,
    /// When Gespan first saw it waiting, in milliseconds.
    pub since_ms: u64,
    pub history: History,
}

/// What the page asks the shell to do.
#[derive(Clone, Debug)]
pub(crate) enum Event {
    Decide {
        session: i64,
        request: String,
        decision: Decision,
    },
    FlowDecide {
        run: String,
        approved: bool,
        note: String,
    },
    Choose {
        pilot: String,
        text: String,
    },
    Open(Source),
    LoadEarlier(String),
}

pub(crate) struct MyTasks {
    tasks: Vec<TaskItem>,
    selected: Option<String>,
    now_ms: u64,
    focus: FocusHandle,
}

impl EventEmitter<Event> for MyTasks {}

impl Focusable for MyTasks {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl MyTasks {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        Self {
            tasks: Vec::new(),
            selected: None,
            now_ms: 0,
            focus: cx.focus_handle(),
        }
    }

    /// Replaces the list with a fresh snapshot, longest wait first. The
    /// selection stays on its task; when that task is gone, the one now in
    /// its place is selected.
    pub(crate) fn set_tasks(
        &mut self,
        mut tasks: Vec<TaskItem>,
        now_ms: u64,
        cx: &mut Context<Self>,
    ) {
        gespan_core::tasks::order(&mut tasks, |t| t.since_ms);
        let was = self
            .selected
            .as_ref()
            .and_then(|key| self.tasks.iter().position(|t| &t.key == key));
        self.selected = match &self.selected {
            Some(key) if tasks.iter().any(|t| &t.key == key) => Some(key.clone()),
            _ => was
                .or(Some(0))
                .and_then(|at| tasks.get(at.min(tasks.len().saturating_sub(1))))
                .map(|t| t.key.clone()),
        };
        self.tasks = tasks;
        self.now_ms = now_ms;
        cx.notify();
    }

    pub(crate) fn tasks(&self) -> &[TaskItem] {
        &self.tasks
    }

    pub(crate) fn open_count(&self) -> usize {
        self.tasks.len()
    }

    pub(crate) fn selected(&self) -> Option<&TaskItem> {
        let key = self.selected.as_ref()?;
        self.tasks.iter().find(|t| &t.key == key)
    }

    pub(crate) fn select(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.tasks.iter().any(|t| t.key == key) {
            self.selected = Some(key.into());
            cx.notify();
        }
    }
}

impl Render for MyTasks {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("my-tasks")
            .debug_selector(|| "my-tasks".into())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .text_color(content(1.))
    }
}
