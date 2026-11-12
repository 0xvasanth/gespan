//! The Answered and Decisions tabs: every request answered or withdrawn,
//! kept in the database with the answer, and the decisions saved from them.
use super::card::answer as button;
use super::list::{kind_icon, waited};
use super::*;
use crate::workspace::project_name;
use gespan_core::tasks::{SavedDecision, Status, TaskRecord};
use gpui::{AnyElement, Div, FontWeight, Role, SharedString, px};
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
    div()
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(content(0.4))
        .child(text.into())
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

    pub(crate) fn visible_decisions(&self, cx: &gpui::App) -> Vec<&SavedDecision> {
        let query = self.search.read(cx).content.to_lowercase();
        self.decisions
            .iter()
            .filter(|d| {
                !self.filter.hidden_projects.contains(&d.project)
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
        let Some(store) = &self.store else {
            return;
        };
        let saved = crate::runtime::locked(store).and_then(|store| {
            store.save_decision(
                &record.project,
                &statement,
                "",
                std::slice::from_ref(&record.key),
                gespan_core::time::now_ms(),
            )
        });
        match saved {
            Ok(_) => {
                self.writing = None;
                if self.offer.as_ref().is_some_and(|o| o.key == key) {
                    self.offer = None;
                }
                self.reload(cx);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    fn change_decision(
        &mut self,
        change: impl FnOnce(&gespan_core::store::Store, u64) -> Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        let Some(store) = &self.store else {
            return;
        };
        match crate::runtime::locked(store)
            .and_then(|store| change(&store, gespan_core::time::now_ms()))
        {
            Ok(()) => {
                self.writing = None;
                self.reload(cx);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
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

    pub(super) fn tab_pills(&self, cx: &mut Context<Self>) -> AnyElement {
        let pill = |tab: Tab, id: &'static str, label: &'static str, count: Option<usize>| {
            let on = self.tab == tab;
            div()
                .id(id)
                .debug_selector(move || id.into())
                .role(Role::Tab)
                .aria_selected(on)
                .cursor_pointer()
                .h(px(28.))
                .px(px(10.))
                .rounded(px(6.))
                .when(on, |d| d.bg(content(0.1)))
                .hover(|s| s.bg(content(0.06)))
                .flex()
                .items_center()
                .gap(px(6.))
                .text_size(px(12.))
                .child(label)
                .when_some(count, |d, count| {
                    d.child(div().text_color(content(0.5)).child(count.to_string()))
                })
                .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
        };
        div()
            .h(px(40.))
            .flex_shrink_0()
            .px(px(8.))
            .flex()
            .items_center()
            .gap(px(2.))
            .border_b_1()
            .border_color(content(0.07))
            .child(pill(
                Tab::Waiting,
                "my-tasks-tab-waiting",
                "Waiting",
                Some(self.tasks.len()),
            ))
            .child(pill(
                Tab::Answered,
                "my-tasks-tab-answered",
                "Answered",
                None,
            ))
            .child(pill(
                Tab::Decisions,
                "my-tasks-tab-decisions",
                "Decisions",
                Some(self.decisions.len()),
            ))
            .into_any_element()
    }

    fn group_header(project: &str, count: usize) -> AnyElement {
        div()
            .px(px(8.))
            .pt(px(10.))
            .pb(px(4.))
            .text_size(px(10.))
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(content(0.4))
            .child(format!("{} \u{b7} {count}", project_name(project)).to_uppercase())
            .into_any_element()
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

    fn list_row(id: String, on: bool) -> gpui::Stateful<Div> {
        let selector = id.clone();
        div()
            .id(SharedString::from(id))
            .debug_selector(move || selector.clone())
            .role(Role::ListBoxOption)
            .aria_selected(on)
            .cursor_pointer()
            .p(px(8.))
            .rounded(px(8.))
            .flex()
            .items_center()
            .gap(px(10.))
            .when(on, |d| d.bg(content(0.1)))
            .hover(|s| s.bg(content(0.06)))
    }

    pub(super) fn past_rows(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let shown = self.visible_past(cx);
        let selected = self.selected_past(&shown).map(|t| t.key.clone());
        let now = self.now_ms;
        let mut rows = Vec::new();
        for (project, tasks) in Self::grouped(shown, |t| &t.project) {
            rows.push(Self::group_header(&project, tasks.len()));
            for task in tasks {
                let key = task.key.clone();
                let answer = match task.status {
                    Status::Answered => {
                        format!("You: {}", task.answer.as_deref().unwrap_or_default())
                    }
                    _ => "Not answered".into(),
                };
                rows.push(
                    Self::list_row(
                        format!("my-tasks-past-{}", task.key),
                        selected.as_ref() == Some(&task.key),
                    )
                    .aria_label(SharedString::from(task.headline.clone()))
                    .child(icon(kind_icon(task.kind), 13.).text_color(content(0.6)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .truncate()
                                    .child(task.headline.clone()),
                            )
                            .child(
                                div()
                                    .text_size(px(12.))
                                    .text_color(content(0.55))
                                    .truncate()
                                    .child(answer),
                            ),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child(waited(now.saturating_sub(task.closed_at.unwrap_or(now)))),
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
        for (project, decisions) in Self::grouped(shown, |d| &d.project) {
            rows.push(Self::group_header(&project, decisions.len()));
            for decision in decisions {
                let id = decision.id;
                let meta = match decision.sources.len() {
                    0 => "Written by hand".to_owned(),
                    1 => "From 1 answer".to_owned(),
                    n => format!("From {n} answers"),
                };
                rows.push(
                    Self::list_row(format!("my-tasks-decision-{id}"), selected == Some(id))
                        .aria_label(SharedString::from(decision.statement.clone()))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .text_size(px(13.))
                                        .truncate()
                                        .child(decision.statement.clone()),
                                )
                                .child(
                                    div()
                                        .text_size(px(12.))
                                        .text_color(content(0.55))
                                        .child(meta),
                                ),
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
        div()
            .debug_selector(|| "my-tasks-remember".into())
            .flex()
            .flex_col()
            .gap(px(8.))
            .p(px(12.))
            .rounded(px(10.))
            .bg(content(0.05))
            .border_1()
            .border_color(content(0.08))
            .child(div().text_size(px(13.)).child(format!(
                "Remember this for {}?",
                project_name(&record.project)
            )))
            .when(writing, |d| {
                d.child(
                    div()
                        .debug_selector(|| "my-tasks-statement".into())
                        .px(px(10.))
                        .py(px(8.))
                        .rounded(px(8.))
                        .bg(black(0.3))
                        .text_size(px(14.))
                        .child(self.statement.clone()),
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
                            true,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| this.save_decision(&key, cx)))
                    } else {
                        let key = key.clone();
                        button(
                            "my-tasks-remember-start",
                            "my-tasks-remember-start".into(),
                            "Save as decision",
                            true,
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
                                false,
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
                .px(px(16.))
                .pt(px(12.))
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
                .p(px(10.))
                .rounded(px(8.))
                .bg(black(0.3))
                .font_family(".SystemUIFont")
                .text_size(px(12.))
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

    fn pane_bar(&self, crumb: String, open: Option<Source>, cx: &mut Context<Self>) -> AnyElement {
        div()
            .h(px(40.))
            .flex_shrink_0()
            .pl(px(20.))
            .pr(px(12.))
            .flex()
            .items_center()
            .gap(px(8.))
            .border_b_1()
            .border_color(content(0.07))
            .text_size(px(12.))
            .text_color(content(0.55))
            .child(div().flex_1().min_w_0().truncate().child(crumb))
            .when_some(open, |d, source| {
                d.child(
                    div()
                        .id("my-tasks-open-source")
                        .debug_selector(|| "my-tasks-open-source".into())
                        .cursor_pointer()
                        .h(px(26.))
                        .px(px(10.))
                        .rounded(px(6.))
                        .bg(content(0.06))
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .text_color(content(0.9))
                        .hover(|s| s.bg(content(0.1)))
                        .child("Open")
                        .child(icon("external-link", 11.))
                        .on_click(
                            cx.listener(move |_, _, _, cx| cx.emit(Event::Open(source.clone()))),
                        ),
                )
            })
            .into_any_element()
    }

    fn empty_pane(text: &'static str) -> AnyElement {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(14.))
            .text_color(content(0.5))
            .child(text)
            .into_any_element()
    }

    pub(super) fn past_pane(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.visible_past(cx);
        let Some(record) = self.selected_past(&shown).cloned() else {
            return Self::empty_pane(if self.past.is_empty() {
                "Nothing answered yet."
            } else {
                "No answers match the search or filters."
            });
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
        let crumb = format!(
            "{}  \u{203a}  {}",
            project_name(&record.project),
            record.source_label
        );
        let bar = self.pane_bar(crumb, Source::from_origin(&record.origin), cx);
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(bar)
            .child(
                div()
                    .id("my-tasks-past-detail")
                    .debug_selector(|| "my-tasks-past-detail".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(icon(kind_icon(record.kind), 16.))
                            .child(
                                div()
                                    .text_size(px(16.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(record.headline.clone()),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(content(0.5))
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
                    })),
            )
            .into_any_element()
    }

    pub(super) fn decision_pane(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let shown = self.visible_decisions(cx);
        let Some(decision) = self.selected_decision(&shown).cloned() else {
            return Self::empty_pane(if self.decisions.is_empty() {
                "No decisions yet. Answer a question, then save it as a decision."
            } else {
                "No decisions match the search or filters."
            });
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
                    .rounded(px(8.))
                    .bg(content(0.04))
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
        let bar = self.pane_bar(
            format!("{}  \u{203a}  Decision", project_name(&decision.project)),
            None,
            cx,
        );
        let actions = if editing {
            div()
                .flex()
                .gap(px(8.))
                .child(
                    button(
                        "my-tasks-edit-save",
                        "my-tasks-edit-save".into(),
                        "Save",
                        true,
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
                        false,
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
                        false,
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
                        false,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_decision(move |store, now| store.archive_decision(id, now), cx);
                    })),
                )
        };
        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(bar)
            .child(
                div()
                    .id("my-tasks-decision-detail")
                    .debug_selector(|| "my-tasks-decision-detail".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(20.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(caption(project_name(&decision.project).to_uppercase()))
                    .child(if editing {
                        div()
                            .debug_selector(|| "my-tasks-statement".into())
                            .px(px(10.))
                            .py(px(8.))
                            .rounded(px(8.))
                            .bg(black(0.3))
                            .text_size(px(16.))
                            .child(self.statement.clone())
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
                        d.child(caption("HOW YOU GOT HERE").mt(px(8.)))
                            .children(sources)
                    })
                    .children(self.error.clone().map(|error| {
                        div()
                            .text_size(px(12.))
                            .text_color(gpui::rgb(0xff6b6b))
                            .child(error)
                    })),
            )
            .into_any_element()
    }
}
