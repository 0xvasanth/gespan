//! My tasks: every approval, question, Flow approval and Pilot choice waiting
//! on the user, from every session, answered in place.
use crate::input::{self, Input};
use crate::popover::AnchorSlot;
use crate::questions::QuestionCard;
use crate::theme::*;
use gespan_core::agent::{Approval, Decision};
use gespan_core::tasks::{Filter, History, Kind};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, Subscription, Window, div, prelude::*,
};

mod filter;
mod list;

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
    pub(crate) search: Entity<Input>,
    /// The filter menu's choices; the query comes from `search`.
    filter: Filter,
    filters_open: bool,
    filter_anchor: AnchorSlot,
    _search_changed: Subscription,
}

impl EventEmitter<Event> for MyTasks {}

impl Focusable for MyTasks {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl MyTasks {
    pub(crate) fn new(cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| Input::field("Search tasks", "Search tasks", content(0.4), cx));
        let _search_changed = cx.subscribe(&search, |_, _, _: &input::Changed, cx| cx.notify());
        Self {
            tasks: Vec::new(),
            selected: None,
            now_ms: 0,
            focus: cx.focus_handle(),
            search,
            filter: Filter::default(),
            filters_open: false,
            filter_anchor: AnchorSlot::default(),
            _search_changed,
        }
    }

    /// The tasks the filter and search leave, longest wait first.
    pub(crate) fn visible(&self, cx: &gpui::App) -> Vec<&TaskItem> {
        let mut filter = self.filter.clone();
        filter.query = self.search.read(cx).content.to_string();
        self.tasks
            .iter()
            .filter(|t| {
                let text = format!("{} {} {}", t.headline, t.source_label, t.who);
                filter.keeps(
                    &t.project,
                    t.kind,
                    self.now_ms.saturating_sub(t.since_ms),
                    &text,
                )
            })
            .collect()
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

    /// The selected task, or the first one shown when the selection is
    /// filtered out.
    pub(crate) fn selected(&self) -> Option<&TaskItem> {
        self.selected_in(&self.filter_only())
    }

    fn selected_in<'a>(&'a self, shown: &[&'a TaskItem]) -> Option<&'a TaskItem> {
        let key = self.selected.as_ref();
        shown
            .iter()
            .find(|t| Some(&t.key) == key)
            .or(shown.first())
            .copied()
    }

    /// `visible` without the search text, for callers with no `App`.
    fn filter_only(&self) -> Vec<&TaskItem> {
        self.tasks
            .iter()
            .filter(|t| {
                self.filter.keeps(
                    &t.project,
                    t.kind,
                    self.now_ms.saturating_sub(t.since_ms),
                    "",
                )
            })
            .collect()
    }

    pub(crate) fn select(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.tasks.iter().any(|t| t.key == key) {
            self.selected = Some(key.into());
            cx.notify();
        }
    }
}

impl Render for MyTasks {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("my-tasks")
            .debug_selector(|| "my-tasks".into())
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .text_color(content(1.))
            .child(self.list_column(cx))
            .when(self.filters_open, |d| d.child(self.filter_menu(cx)))
    }
}

#[cfg(test)]
mod tests;
