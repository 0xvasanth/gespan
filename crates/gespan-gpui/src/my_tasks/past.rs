//! The Answered and Decisions tabs: every request answered or withdrawn,
//! kept in the database with the answer, and the decisions saved from them.
use super::card::answer as button;
use super::list::{group_label, kind_status, row_line, row_title, task_row, tile, waited};
use super::pane::{open_button, top_bar};
use super::project_display;
use super::*;
use crate::glass::{self, Button};
use crate::workspace::project_name;
use gespan_core::tasks::{SavedDecision, Status, TaskRecord};
use gpui::{AnyElement, Div, FontWeight, SharedString, px};
use serde_json::Value;

/// Which list the page shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Tab {
    /// What waits on the user now: the default.
    #[default]
    Waiting,
    /// Requests answered, or withdrawn with no answer, newest first.
    Answered,
    Decisions,
}

/// What the statement field is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Writing {
    /// A decision to save from the answer under this key.
    Remember(String),
    /// A saved decision being edited.
    Edit(i64),
}

/// "just now", "5m ago", "2d ago".
pub(super) fn ago(now: u64, at: u64) -> String {
    match waited(now.saturating_sub(at)).as_str() {
        "now" => "just now".into(),
        span => format!("{span} ago"),
    }
}

fn caption(text: impl Into<SharedString>) -> Div {
    glass::eyebrow(text)
}

/// The Decisions list's group key for preferences: their scope's text.
pub(super) const PREFERENCES: &str = "global";

/// Where the Decisions list groups a decision, and the name a group of its
/// own would show: preferences under "Your preferences"; a project decision
/// under its project's folder name; a repository decision, which can come
/// from any clone, under its repository's scope, named after the checkout it
/// was saved in or, for an agent's proposal, after the repository.
pub(super) fn decision_group(decision: &SavedDecision) -> (String, String) {
    use gespan_core::decisions::{Kind as EntryKind, Scope};
    match &decision.scope {
        _ if decision.kind == EntryKind::Preference => {
            (PREFERENCES.into(), "Your preferences".into())
        }
        Scope::Global => (PREFERENCES.into(), "Your preferences".into()),
        Scope::Project(path) => (path.clone(), project_name(path)),
        Scope::Repo(repo) => {
            let saved_in = decision.project.as_str();
            let label = if saved_in.starts_with("repo:") || saved_in == PREFERENCES {
                repo_name(repo)
            } else {
                project_name(saved_in)
            };
            (format!("repo:{repo}"), label)
        }
    }
}

/// `github.com/o/gespan@0123456789ab` is the repository `gespan`.
pub(super) fn repo_name(repo: &str) -> String {
    let path = repo.rsplit_once('@').map_or(repo, |(path, _)| path);
    path.rsplit('/').next().unwrap_or(path).to_owned()
}

/// Decisions grouped as [`decision_group`] says, in the order their first
/// one comes. A group is named after its first member saved in a checkout,
/// so a repository's decisions all show under one name.
pub(super) fn decision_groups<'a>(
    decisions: impl IntoIterator<Item = &'a SavedDecision>,
) -> Vec<(String, String, Vec<&'a SavedDecision>)> {
    let mut groups: Vec<(String, String, Vec<&SavedDecision>)> = Vec::new();
    for decision in decisions {
        let (key, label) = decision_group(decision);
        match groups.iter_mut().find(|(k, _, _)| *k == key) {
            Some((_, name, members)) => {
                if !decision.project.starts_with("repo:")
                    && members.iter().all(|m| m.project.starts_with("repo:"))
                {
                    *name = label;
                }
                members.push(decision);
            }
            None => groups.push((key, label, vec![decision])),
        }
    }
    groups
}

/// The options a question item offered, with the ones picked marked.
fn picked_options(item: &Value, picked: Option<&Value>) -> Vec<(String, bool)> {
    let chosen: Vec<&str> = picked
        .and_then(Value::as_array)
        .map(|labels| labels.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    item["options"]
        .as_array()
        .map(|options| {
            options
                .iter()
                .filter_map(|o| o["label"].as_str().or(o.as_str()))
                .map(|label| (label.to_owned(), chosen.contains(&label)))
                .collect()
        })
        .unwrap_or_default()
}

impl MyTasks {
    /// Reads the Answered and Decisions lists again from the database.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(store) = &self.store else {
            return;
        };
        let loaded = crate::runtime::locked(store)
            .and_then(|store| Ok((store.past_tasks()?, store.decisions()?)));
        match loaded {
            Ok((past, decisions)) => {
                self.past = past;
                self.decisions = decisions;
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        cx.notify();
    }

    pub(crate) fn set_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            self.filters_open = false;
            self.writing = None;
            self.reload(cx);
        }
    }

    /// Answered and withdrawn requests the filter and search leave.
    pub(crate) fn visible_past(&self, cx: &gpui::App) -> Vec<&TaskRecord> {
        let query = self.search.read(cx).content.to_string();
        self.past
            .iter()
            .filter(|t| {
                let text = format!(
                    "{} {} {}",
                    t.headline,
                    t.source_label,
                    t.answer.as_deref().unwrap_or_default()
                );
                let mut filter = self.filter.clone();
                filter.query = query.clone();
                // A closed request has no wait left to filter on.
                filter.min_wait = Default::default();
                filter.keeps(&t.project, t.kind, 0, &text)
            })
            .collect()
    }

    /// A project's name in the filter menu: on the Decisions tab, the name
    /// its group shows.
    pub(super) fn project_label(&self, project: &str) -> String {
        if self.tab == Tab::Decisions
            && let Some((_, label, _)) = decision_groups(&self.decisions)
                .into_iter()
                .find(|(key, _, _)| key == project)
        {
            return label;
        }
        project_display(project)
    }

    /// Every decision's group: its key, its name and its decisions' ids.
    #[cfg(test)]
    pub(crate) fn decision_groups_for_test(&self) -> Vec<(String, String, Vec<i64>)> {
        decision_groups(&self.decisions)
            .into_iter()
            .map(|(key, name, members)| (key, name, members.iter().map(|d| d.id).collect()))
            .collect()
    }

    pub(crate) fn visible_decisions(&self, cx: &gpui::App) -> Vec<&SavedDecision> {
        let query = self.search.read(cx).content.to_lowercase();
        self.decisions
            .iter()
            .filter(|d| {
                !self.filter.hidden_projects.contains(&decision_group(d).0)
                    && (query.is_empty()
                        || format!("{} {}", d.statement, d.why)
                            .to_lowercase()
                            .contains(query.trim()))
            })
            .collect()
    }

    fn selected_past<'a>(&'a self, shown: &[&'a TaskRecord]) -> Option<&'a TaskRecord> {
        shown
            .iter()
            .find(|t| Some(&t.key) == self.past_selected.as_ref())
            .or(shown.first())
            .copied()
    }

    fn selected_decision<'a>(&'a self, shown: &[&'a SavedDecision]) -> Option<&'a SavedDecision> {
        shown
            .iter()
            .find(|d| Some(d.id) == self.decision_selected)
            .or(shown.first())
            .copied()
    }

    fn select_past(&mut self, key: String, cx: &mut Context<Self>) {
        if self.past_selected.as_ref() != Some(&key) {
            self.past_selected = Some(key);
            self.writing = None;
            cx.notify();
        }
    }

    fn select_decision(&mut self, id: i64, cx: &mut Context<Self>) {
        if self.decision_selected != Some(id) {
            self.decision_selected = Some(id);
            self.writing = None;
            cx.notify();
        }
    }

    /// Opens the statement field for `writing`, filled with `text`.
    fn start_writing(&mut self, writing: Writing, text: String, cx: &mut Context<Self>) {
        self.statement
            .update(cx, |field, cx| field.set_text(text, cx));
        self.writing = Some(writing);
        cx.notify();
    }

    /// Saves the answer under `key` as a decision for its project, worded as
    /// the statement field says.
    pub(crate) fn save_decision(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(record) = self
            .past
            .iter()
            .chain(self.offer.iter())
            .find(|t| t.key == key)
            .cloned()
        else {
            return;
        };
        let statement = self.statement.read(cx).content.to_string();
        let Some(store) = self.store.clone() else {
            return;
        };
        let key = key.to_owned();
        // The scope runs git: work it out off the UI thread, before taking
        // the store's lock.
        let work = cx.background_spawn(async move {
            let scope = gespan_core::decisions::scope_for(std::path::Path::new(&record.project));
            crate::runtime::locked(&store).and_then(|store| {
                store.save_decision_in(
                    &scope,
                    &record.project,
                    &statement,
                    "",
                    std::slice::from_ref(&record.key),
                    gespan_core::time::now_ms(),
                )
            })
        });
        cx.spawn(async move |this, cx| {
            let saved = work.await;
            let _ = this.update(cx, |this, cx| match saved {
                Ok(_) => {
                    // The decision is the decision log's too: its harness
                    // files follow (nothing happens while the log is off).
                    gespan_core::decisions::service::regenerate();
                    this.writing = None;
                    if this.offer.as_ref().is_some_and(|o| o.key == key) {
                        this.offer = None;
                    }
                    this.reload(cx);
                }
                Err(error) => {
                    this.error = Some(error);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// Edits or archives a saved decision off the UI thread (the store's
    /// lock is taken there, never here), then shows the result.
    fn change_decision(
        &mut self,
        change: impl FnOnce(&gespan_core::store::Store, u64) -> Result<(), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = self.store.clone() else {
            return;
        };
        let work = cx.background_spawn(async move {
            crate::runtime::locked(&store)
                .and_then(|store| change(&store, gespan_core::time::now_ms()))
        });
        cx.spawn(async move |this, cx| {
            let changed = work.await;
            let _ = this.update(cx, |this, cx| match changed {
                Ok(()) => {
                    gespan_core::decisions::service::regenerate();
                    this.writing = None;
                    this.reload(cx);
                }
                Err(error) => {
                    this.error = Some(error);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// An answer that just left the Waiting list: offered as a decision when
    /// it can be one.
    pub(super) fn offer_from(&mut self, gone: &[String], cx: &mut Context<Self>) {
        let offer = self
            .past
            .iter()
            .find(|t| gone.contains(&t.key) && t.can_remember())
            .cloned();
        if let Some(offer) = offer {
            let text = offer.suggested_statement();
            self.start_writing(Writing::Remember(offer.key.clone()), text, cx);
            self.offer = Some(offer);
        }
    }

    /// Rows grouped by project, in the order their first row comes.
    fn grouped<T>(items: Vec<T>, project: impl Fn(&T) -> &str) -> Vec<(String, Vec<T>)> {
        let mut groups: Vec<(String, Vec<T>)> = Vec::new();
        for item in items {
            let name = project(&item).to_owned();
            match groups.iter_mut().find(|(p, _)| *p == name) {
                Some((_, members)) => members.push(item),
                None => groups.push((name, vec![item])),
            }
        }
        groups
    }

    /// A project's label over its rows, with how many there are.
    fn group_header(project: &str, count: usize, first: bool) -> AnyElement {
        group_label(
            format!("my-tasks-group-{project}"),
            format!("{} \u{b7} {count}", project_display(project)),
            first,
        )
    }

    pub(super) fn past_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let shown = self.visible_past(cx);
        let selected = self.selected_past(&shown).map(|t| t.key.clone());
        let now = self.now_ms;
        let mut rows = Vec::new();
        for (ix, (project, tasks)) in Self::grouped(shown, |t| &t.project).into_iter().enumerate() {
            rows.push(Self::group_header(&project, tasks.len(), ix == 0));
            for task in tasks {
                let key = task.key.clone();
                let (answer, tone) = match task.status {
                    Status::Answered => (
                        format!("You: {}", task.answer.as_deref().unwrap_or_default()),
                        glass::Tone::Ok,
                    ),
                    _ => ("Not answered".into(), glass::Tone::Muted),
                };
                let (word, _) = kind_status(task.kind);
                let ago = waited(now.saturating_sub(task.closed_at.unwrap_or(now)));
                rows.push(
                    task_row(
                        format!("my-tasks-past-{}", task.key),
                        selected.as_ref() == Some(&task.key),
                    )
                    .aria_label(SharedString::from(task.headline.clone()))
                    .child(tile(task.kind, 32.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(3.))
                            .child(row_title(task.headline.clone(), Some((ago, content(0.45)))))
                            .child(row_line(answer))
                            .child(
                                div()
                                    .mt(px(4.))
                                    .flex()
                                    .gap(px(6.))
                                    .child(glass::status(word, tone)),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select_past(key.clone(), cx)))
                    .into_any_element(),
                );
            }
        }
        rows
    }

    pub(super) fn decision_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let shown = self.visible_decisions(cx);
        let selected = self.selected_decision(&shown).map(|d| d.id);
        let mut rows = Vec::new();
        for (ix, (key, name, decisions)) in decision_groups(shown).into_iter().enumerate() {
            rows.push(group_label(
                format!("my-tasks-group-{key}"),
                format!("{name} \u{b7} {}", decisions.len()),
                ix == 0,
            ));
            for decision in decisions {
                let id = decision.id;
                let meta = match (decision.sources.len(), decision.session) {
                    (0, Some(session)) => format!("Proposed in session {session}"),
                    (0, None) => "Written by hand".to_owned(),
                    (1, _) => "From 1 answer".to_owned(),
                    (n, _) => format!("From {n} answers"),
                };
                rows.push(
                    task_row(format!("my-tasks-decision-{id}"), selected == Some(id))
                        .aria_label(SharedString::from(decision.statement.clone()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .gap(px(3.))
                                .child(row_title(decision.statement.clone(), None))
                                .child(row_line(meta)),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.select_decision(id, cx)))
                        .into_any_element(),
                );
            }
        }
        rows
    }

    /// "Remember this for <project>?": the statement field with Save as
    /// decision and Just this once.
    fn remember(&self, record: &TaskRecord, cx: &mut Context<Self>) -> AnyElement {
        let writing = self.writing == Some(Writing::Remember(record.key.clone()));
        let key = record.key.clone();
        let text = record.suggested_statement();
        glass::card()
            .debug_selector(|| "my-tasks-remember".into())
            .flex()
            .flex_col()
            .gap(px(10.))
            .p(px(14.))
            .child(div().text_size(px(13.)).child(format!(
                "Remember this for {}?",
                project_name(&record.project)
            )))
            .when(writing, |d| {
                d.child(
                    glass::field()
                        .debug_selector(|| "my-tasks-statement".into())
                        .h(px(36.))
                        .text_size(px(14.))
                        .child(div().flex_1().min_w_0().child(self.statement.clone())),
                )
            })
            .child(
                div()
                    .flex()
                    .gap(px(8.))
                    .child(if writing {
                        let key = key.clone();
                        button(
                            "my-tasks-save-decision",
                            "my-tasks-save-decision".into(),
                            "Save as decision",
                            Button::Primary,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.save_decision(&key, cx)))
                    } else {
                        let key = key.clone();
                        button(
                            "my-tasks-remember-start",
                            "my-tasks-remember-start".into(),
                            "Save as decision",
                            Button::Primary,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.start_writing(Writing::Remember(key.clone()), text.clone(), cx)
                        }))
                    })
                    .when(writing, |d| {
                        d.child(
                            button(
                                "my-tasks-just-once",
                                "my-tasks-just-once".into(),
                                "Just this once",
                                Button::Ghost,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    this.writing = None;
                                    if this.offer.as_ref().is_some_and(|o| o.key == key) {
                                        this.offer = None;
                                    }
                                    cx.notify();
                                },
                            )),
                        )
                    }),
            )
            .into_any_element()
    }

    /// The banner over the Waiting tab after a question or choice is
    /// answered: the moment to keep it as a decision.
    pub(super) fn offer_banner(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let offer = self.offer.as_ref()?;
        Some(
            div()
                .flex_shrink_0()
                .px(px(24.))
                .pt(px(14.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(content(0.55))
                        .child(format!(
                            "Answered \u{201c}{}\u{201d}: {}",
                            offer.headline,
                            offer.answer.as_deref().unwrap_or_default()
                        )),
                )
                .child(self.remember(offer, cx))
                .into_any_element(),
        )
    }

    /// What was asked, as the database kept it.
    fn request_body(record: &TaskRecord) -> Vec<AnyElement> {
        let request = &record.request;
        let mono = |text: String| {
            div()
                .px(px(12.))
                .py(px(10.))
                .rounded(px(10.))
                .bg(black(if is_light() { 0.05 } else { 0.35 }))
                .border_1()
                .border_color(glass::edge())
                .font_family(".SystemUIFont")
                .text_size(px(12.))
                .line_height(px(19.))
                .child(text)
                .into_any_element()
        };
        let option = |label: String, picked: bool| {
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(13.))
                .text_color(content(if picked { 1. } else { 0.55 }))
                .child(
                    div()
                        .size(px(14.))
                        .when(picked, |d| d.child(icon("check", 14.))),
                )
                .child(label)
                .into_any_element()
        };
        match record.kind {
            Kind::Approval => {
                let input = &request["tool"]["input"];
                let shown = ["command", "file_path", "path", "url"]
                    .iter()
                    .find_map(|key| input[*key].as_str())
                    .map(str::to_owned)
                    .or_else(|| {
                        request["detail"]
                            .as_str()
                            .filter(|d| !d.is_empty())
                            .map(str::to_owned)
                    });
                shown.map(mono).into_iter().collect()
            }
            Kind::Question => {
                let items = request["items"].as_array().cloned().unwrap_or_default();
                let picked = record.answer_value.as_ref().and_then(Value::as_array);
                items
                    .iter()
                    .enumerate()
                    .map(|(ix, item)| {
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .when(items.len() > 1, |d| {
                                d.child(
                                    div()
                                        .text_size(px(13.))
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(
                                            item["question"]
                                                .as_str()
                                                .unwrap_or_default()
                                                .to_owned(),
                                        ),
                                )
                            })
                            .children(
                                picked_options(item, picked.and_then(|p| p.get(ix)))
                                    .into_iter()
                                    .map(|(label, on)| option(label, on)),
                            )
                            .into_any_element()
                    })
                    .collect()
            }
            Kind::FlowApproval => request["prompt"]
                .as_str()
                .filter(|p| !p.is_empty())
                .map(|p| mono(p.to_owned()))
                .into_iter()
                .collect(),
            Kind::DraftReview => {
                let mut shown = Vec::new();
                if let Some(body) = request["body"].as_str().filter(|b| !b.is_empty()) {
                    shown.push(mono(body.to_owned()));
                }
                for comment in request["comments"].as_array().into_iter().flatten() {
                    let side = if comment["side"] == "old" {
                        " (old)"
                    } else {
                        ""
                    };
                    shown.push(mono(format!(
                        "{}:{}{side} \u{2014} {}",
                        comment["path"].as_str().unwrap_or_default(),
                        comment["line"],
                        comment["body"].as_str().unwrap_or_default()
                    )));
                }
                shown
            }
            Kind::Decision => ["text", "why", "user_answer", "flag"]
                .iter()
                .filter_map(|field| request[*field].as_str().filter(|t| !t.is_empty()))
                .map(|text| mono(text.to_owned()))
                .collect(),
            Kind::PilotChoice => {
                let answer = record.answer.as_deref().unwrap_or_default();
                request["options"]
                    .as_array()
                    .map(|options| {
                        options
                            .iter()
                            .filter_map(Value::as_str)
                            .map(|o| option(o.to_owned(), o == answer))
                            .collect()
                    })
                    .unwrap_or_default()
            }
        }
    }

    /// A detail pane: the top bar over one glass card, centred.
    fn detail(id: &'static str, bar: gpui::Div, card: gpui::Div) -> AnyElement {
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(bar)
            .child(
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(24.))
                    .py(px(28.))
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        card.w_full()
                            .max_w(px(720.))
                            .p(px(18.))
                            .flex()
                            .flex_col()
                            .gap(px(12.)),
                    ),
            )
            .into_any_element()
    }

    pub(super) fn past_pane(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.visible_past(cx);
        let Some(record) = self.selected_past(&shown).cloned() else {
            return self.empty_pane(None);
        };
        let now = self.now_ms;
        let closed = record.closed_at.unwrap_or(record.asked_at);
        let outcome = match record.status {
            Status::Answered => format!("You: {}", record.answer.clone().unwrap_or_default()),
            _ => "Not answered: the agent stopped waiting".into(),
        };
        let saved = record.decision.and_then(|id| {
            self.decisions
                .iter()
                .find(|d| d.id == id)
                .map(|d| (id, d.statement.clone()))
        });
        let body = Self::request_body(&record);
        let remember = record.can_remember().then(|| self.remember(&record, cx));
        let open = Source::from_origin(&record.origin);
        let bar =
            top_bar(&record.project, record.source_label.clone())
                .child(div().flex_1())
                .when_some(open, |d, source| {
                    d.child(open_button("Open").on_click(
                        cx.listener(move |_, _, _, cx| cx.emit(Event::Open(source.clone()))),
                    ))
                });
        let card = glass::card_strong()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .child(tile(record.kind, 28.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(px(17.))
                            .line_height(px(24.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(record.headline.clone()),
                    ),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(content(0.55))
                    .child(format!(
                        "{} \u{b7} asked {} \u{b7} closed {}",
                        record.kind.label().trim_end_matches('s'),
                        ago(now, record.asked_at),
                        ago(now, closed)
                    )),
            )
            .children(body)
            .child(
                div()
                    .debug_selector(|| "my-tasks-past-answer".into())
                    .text_size(px(14.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(if record.status == Status::Answered {
                        1.
                    } else {
                        0.5
                    }))
                    .child(outcome),
            )
            .when_some(saved, |d, (id, statement)| {
                d.child(
                    div()
                        .id("my-tasks-view-decision")
                        .debug_selector(|| "my-tasks-view-decision".into())
                        .cursor_pointer()
                        .text_size(px(13.))
                        .text_color(content(0.7))
                        .hover(|s| s.text_color(content(1.)))
                        .child(format!("Saved as a decision: {statement}"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.decision_selected = Some(id);
                            this.set_tab(Tab::Decisions, cx);
                        })),
                )
            })
            .children(remember)
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(gpui::rgb(0xff6b6b))
                    .child(error)
            }));
        Self::detail("my-tasks-past-detail", bar, card)
    }

    pub(super) fn decision_pane(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.visible_decisions(cx);
        let Some(decision) = self.selected_decision(&shown).cloned() else {
            return self.empty_pane(None);
        };
        let id = decision.id;
        let now = self.now_ms;
        let editing = self.writing == Some(Writing::Edit(id));
        let changed = if decision.updated_at > decision.created_at {
            format!("Edited {}", ago(now, decision.updated_at))
        } else {
            format!("Saved {}", ago(now, decision.created_at))
        };
        let statement = decision.statement.clone();
        let sources: Vec<AnyElement> = decision
            .sources
            .iter()
            .map(|source| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .p(px(10.))
                    .rounded(px(10.))
                    .bg(glass::fill())
                    .border_1()
                    .border_color(glass::edge())
                    .child(div().text_size(px(13.)).child(source.headline.clone()))
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(content(0.75))
                            .child(format!(
                                "You: {}",
                                source.answer.as_deref().unwrap_or_default()
                            )),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child(format!(
                                "{} \u{b7} {}",
                                source.source_label,
                                ago(now, source.closed_at.unwrap_or(source.asked_at))
                            )),
                    )
                    .into_any_element()
            })
            .collect();
        // The name its list header shows.
        let group = self.project_label(&decision_group(&decision).0);
        let noun = match decision.kind {
            gespan_core::decisions::Kind::Preference => "Preference",
            gespan_core::decisions::Kind::Decision => "Decision",
        };
        let bar = top_bar(&group, noun.into());
        let actions = if editing {
            div()
                .flex()
                .gap(px(8.))
                .child(
                    button(
                        "my-tasks-edit-save",
                        "my-tasks-edit-save".into(),
                        "Save",
                        Button::Primary,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let text = this.statement.read(cx).content.to_string();
                        this.change_decision(
                            move |store, now| store.edit_decision(id, &text, now),
                            cx,
                        );
                    })),
                )
                .child(
                    button(
                        "my-tasks-edit-cancel",
                        "my-tasks-edit-cancel".into(),
                        "Cancel",
                        Button::Ghost,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.writing = None;
                        cx.notify();
                    })),
                )
        } else {
            div()
                .flex()
                .gap(px(8.))
                .child(
                    button(
                        "my-tasks-edit-decision",
                        "my-tasks-edit-decision".into(),
                        "Edit",
                        Button::Glass,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.start_writing(Writing::Edit(id), statement.clone(), cx)
                    })),
                )
                .child(
                    button(
                        "my-tasks-archive-decision",
                        "my-tasks-archive-decision".into(),
                        "Archive",
                        Button::Ghost,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_decision(move |store, now| store.archive_decision(id, now), cx);
                    })),
                )
        };
        let card = glass::card_strong()
            .child({
                let selector = format!("my-tasks-decision-group-{group}");
                caption(group).debug_selector(move || selector.clone())
            })
            .child(if editing {
                glass::field()
                    .debug_selector(|| "my-tasks-statement".into())
                    .h(px(40.))
                    .text_size(px(16.))
                    .child(div().flex_1().min_w_0().child(self.statement.clone()))
            } else {
                div()
                    .debug_selector(|| "my-tasks-decision-statement".into())
                    .text_size(px(18.))
                    .line_height(px(26.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(decision.statement.clone())
            })
            .when(!decision.why.is_empty(), |d| {
                d.child(
                    div()
                        .text_size(px(13.))
                        .text_color(content(0.7))
                        .child(decision.why.clone()),
                )
            })
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child(changed),
            )
            .child(actions)
            .when(!sources.is_empty(), |d| {
                d.child(caption("How you got here").mt(px(8.)))
                    .children(sources)
            })
            .children(self.error.clone().map(|error| {
                div()
                    .text_size(px(12.))
                    .text_color(gpui::rgb(0xff6b6b))
                    .child(error)
            }));
        Self::detail("my-tasks-decision-detail", bar, card)
    }
}
