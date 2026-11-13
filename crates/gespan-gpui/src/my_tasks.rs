//! My tasks: every approval, question, Flow approval, Pilot choice and
//! decision to review waiting on the user, from every session, answered in
//! place.
use crate::input::{self, Input};
use crate::popover::AnchorSlot;
use crate::questions::QuestionCard;
use crate::theme::*;
use gespan_core::agent::{Approval, Decision};
use gespan_core::decisions::{Entry, Flag, Review};
use gespan_core::tasks::{Filter, History, Kind, SavedDecision, TaskRecord};
use gpui::{
    Context, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle, Subscription, Window, div,
    prelude::*,
};

mod card;
mod decision;
mod filter;
mod history;
mod list;
mod pane;
mod past;
pub(crate) use past::{Tab, Writing};
pub(crate) mod record;

/// Turns of history each "Load earlier" adds.
pub(crate) const HISTORY_PAGE: usize = 4;
/// The card's largest share of the pane's height.
const CARD_MAX_HEIGHT: f32 = 0.62;

/// The project key a decision-log task is listed (and filtered) under,
/// the same key the Decisions tab groups by: a project scope's path,
/// `repo:<key>` for a repository, or `global` for a preference.
pub(crate) fn decision_project(entry: &Entry) -> String {
    use gespan_core::decisions::{Kind as EntryKind, Scope};
    match &entry.scope {
        _ if entry.kind == EntryKind::Preference => past::PREFERENCES.into(),
        Scope::Global => past::PREFERENCES.into(),
        Scope::Project(path) => path.clone(),
        Scope::Repo(repo) => format!("repo:{repo}"),
    }
}

/// How a project key reads on the page: a folder's name, a repository's
/// name for `repo:<key>`, and "Your preferences" for `global`.
pub(crate) fn project_display(project: &str) -> String {
    if project == past::PREFERENCES {
        "Your preferences".into()
    } else if let Some(repo) = project.strip_prefix("repo:") {
        past::repo_name(repo)
    } else {
        crate::workspace::project_name(project)
    }
}

/// Where a task comes from, so the page can open it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Source {
    Session(i64),
    FlowRun(String),
    Pilot {
        pilot: String,
        session: i64,
    },
    /// A draft review on a pull or merge request, by the run that left it.
    DraftReview {
        run: String,
        url: String,
    },
    /// The decision log itself: an entry no session proposed (an import).
    Log,
}

/// What the card shows and answers.
#[derive(Clone)]
pub(crate) enum Body {
    Approval {
        approval: Box<Approval>,
        cwd: Option<String>,
    },
    Question(Entity<QuestionCard>),
    Flow {
        prompt: String,
    },
    Choice {
        options: Vec<String>,
    },
    /// The draft's text and its inline comments, read before deciding.
    DraftReview {
        body: String,
        comments: Vec<gespan_core::review_draft::ShownComment>,
        outside: usize,
    },
    /// A proposal to review, or (with `flag`) an agent's flag on an approved
    /// entry. `similar` are the active entries approving it might replace.
    Decision {
        entry: Box<Entry>,
        flag: Option<Flag>,
        similar: Vec<Entry>,
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
    /// The agent that asked ("Claude Code", "Pilot"), shown as a pill on its
    /// row; `None` for a Flow step, whose kind already says Flow.
    pub agent: Option<String>,
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
        key: String,
        pilot: String,
        text: String,
    },
    /// Submit the draft review left by run `run`, or keep it as a draft.
    DraftDecide {
        run: String,
        submit: bool,
    },
    Open(Source),
    LoadEarlier(String),
    /// The user's review of decision-log entry `id`, from the task `key`.
    DecisionReview {
        key: String,
        id: i64,
        review: Review,
    },
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
    /// A Flow approval's note.
    pub(crate) note: Entity<Input>,
    /// A decision's wording while the user edits it, or writes a replacement.
    pub(crate) decision_title: Entity<Input>,
    pub(crate) decision_text: Entity<Input>,
    pub(crate) decision_why: Entity<Input>,
    decision_editing: bool,
    /// What the card showed when the edit began: approving the edit is
    /// refused if the proposal changed since.
    edit_shown: Option<gespan_core::decisions::ShownHash>,
    /// "Always include" on the card's proposal.
    decision_pinned: bool,
    /// The similar active entries ticked under "Replaces:".
    decision_replaces: Vec<i64>,
    /// Why the last review of the card's decision failed, by task key.
    review_error: Option<(String, String)>,
    history_scroll: ScrollHandle,
    /// When scrolling last asked for earlier turns: one page per gesture.
    last_load: Option<std::time::Instant>,
    /// Waiting (the default), Answered or Decisions.
    pub(crate) tab: Tab,
    /// Where answers and decisions are kept; set by the shell.
    pub(crate) store: Option<crate::runtime::Database>,
    /// Answered and withdrawn requests, newest first.
    past: Vec<TaskRecord>,
    decisions: Vec<SavedDecision>,
    past_selected: Option<String>,
    decision_selected: Option<i64>,
    /// A decision being worded: one to save from an answer, or an edit.
    pub(crate) statement: Entity<Input>,
    writing: Option<Writing>,
    /// A question or choice just answered, offered as a decision.
    offer: Option<TaskRecord>,
    error: Option<String>,
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
        let _search_changed = cx.subscribe(&search, |this, _, _: &input::Changed, cx| {
            this.sync_selection(cx)
        });
        Self {
            tasks: Vec::new(),
            selected: None,
            now_ms: 0,
            focus: cx.focus_handle(),
            search,
            filter: Filter::default(),
            filters_open: false,
            filter_anchor: AnchorSlot::default(),
            note: cx
                .new(|cx| Input::field("Note for the run", "Note (optional)", content(0.4), cx)),
            decision_title: cx.new(|cx| Input::field("Title", "Title", content(0.4), cx)),
            decision_text: cx.new(|cx| Input::multiline_field("Decision", "What to do", cx)),
            decision_why: cx.new(|cx| Input::field("Why", "Why", content(0.4), cx)),
            decision_editing: false,
            edit_shown: None,
            decision_pinned: false,
            decision_replaces: Vec::new(),
            review_error: None,
            history_scroll: ScrollHandle::new(),
            last_load: None,
            tab: Tab::default(),
            store: None,
            past: Vec::new(),
            decisions: Vec::new(),
            past_selected: None,
            decision_selected: None,
            statement: cx.new(|cx| Input::field("Decision", "What you decided", content(0.4), cx)),
            writing: None,
            offer: None,
            error: None,
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
        let before = self.selected.clone();
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
        let moved = self.selected != before;
        let gone: Vec<String> = self
            .tasks
            .iter()
            .filter(|old| !tasks.iter().any(|t| t.key == old.key))
            .map(|t| t.key.clone())
            .collect();
        self.tasks = tasks;
        if !gone.is_empty() {
            // An answer just left the list: the history has a new entry.
            self.reload(cx);
            self.offer_from(&gone, cx);
        }
        self.now_ms = now_ms;
        if moved {
            self.selection_changed(cx);
        }
        self.sync_selection(cx);
    }

    /// Keeps the card on a task the list shows: when the filter or search
    /// hides the selected one, the first shown task takes its place, with a
    /// fresh note.
    fn sync_selection(&mut self, cx: &mut Context<Self>) {
        let first = {
            let shown = self.visible(cx);
            let kept = shown.iter().any(|t| Some(&t.key) == self.selected.as_ref());
            (!kept).then(|| shown.first().map(|t| t.key.clone()))
        };
        if let Some(first) = first
            && first != self.selected
        {
            self.selected = first;
            self.selection_changed(cx);
        }
        cx.notify();
    }

    /// Every task waiting now, for Home and tests.
    pub(crate) fn tasks(&self) -> &[TaskItem] {
        &self.tasks
    }

    pub(crate) fn open_count(&self) -> usize {
        self.tasks.len()
    }

    /// The selected task, or the first one shown when the selection is
    /// filtered out.
    #[cfg(test)]
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
    #[cfg(test)]
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
        if self.tasks.iter().any(|t| t.key == key) && self.selected.as_deref() != Some(key) {
            self.selected = Some(key.into());
            self.selection_changed(cx);
            cx.notify();
        }
    }

    /// A new task is on the card: its note starts empty, a decision starts
    /// unedited, unpinned and replacing nothing, and its history opens on the
    /// latest turn.
    fn selection_changed(&mut self, cx: &mut Context<Self>) {
        self.note
            .update(cx, |note, cx| note.set_text(String::new(), cx));
        self.decision_editing = false;
        self.edit_shown = None;
        self.decision_pinned = false;
        self.decision_replaces.clear();
        self.history_scroll.scroll_to_bottom();
    }

    /// "2 of 5": the card's place among the tasks shown.
    pub(crate) fn position(&self, cx: &gpui::App) -> String {
        let shown = self.visible(cx);
        let at = self
            .selected_in(&shown)
            .and_then(|t| shown.iter().position(|s| s.key == t.key))
            .map_or(0, |at| at + 1);
        format!("{at} of {}", shown.len())
    }

    /// Moves the card `by` tasks through those shown, wrapping at either end.
    fn step(&mut self, by: isize, cx: &mut Context<Self>) {
        let shown = self.visible(cx);
        if shown.is_empty() {
            return;
        }
        let at = self
            .selected_in(&shown)
            .and_then(|t| shown.iter().position(|s| s.key == t.key))
            .unwrap_or(0) as isize;
        let len = shown.len() as isize;
        let key = shown[(at + by).rem_euclid(len) as usize].key.clone();
        self.select(&key, cx);
    }
}

impl Render for MyTasks {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("my-tasks")
            .debug_selector(|| "my-tasks".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::enter))
            .size_full()
            .flex()
            .text_color(content(1.))
            .child(self.list_column(cx))
            .child(match self.tab {
                Tab::Waiting => self.task_pane(cx),
                Tab::Answered => self.past_pane(cx),
                Tab::Decisions => self.decision_pane(cx),
            })
            .when(self.filters_open, |d| d.child(self.filter_menu(cx)))
    }
}

#[cfg(test)]
mod tests;
