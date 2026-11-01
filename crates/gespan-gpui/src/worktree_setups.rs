//! What a new worktree's setup is doing, for any workflow that made one: the
//! files copied, the script's output, and what to do when it fails. Core blocks
//! the work on an answer, so a run started by a Flow or a Pilot waits here the
//! way it waits on an approval.
use crate::{modal, theme::*};
use gespan_core::{
    worktree_copy::Copied,
    worktree_scripts::{self, Decision, Event, Host, Outcome, Run, Stage, Tail},
};
use gpui::{
    AnyElement, App, Context, Entity, Global, Role, ScrollHandle, SharedString, div, prelude::*, px,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};

/// How long a step that finished stays on screen.
const LINGER: Duration = Duration::from_secs(6);
const NAMES_SHOWN: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
enum State {
    Running,
    Failed(String),
    Done(Outcome),
}

struct Step {
    run: Run,
    tail: Tail,
    state: State,
    scroll: ScrollHandle,
}

type Answers = Arc<Mutex<HashMap<u64, mpsc::Sender<Decision>>>>;

pub(crate) struct WorktreeSetups {
    steps: Vec<Step>,
    answers: Answers,
}

struct Bridge {
    events: async_channel::Sender<Event>,
    answers: Answers,
}
impl Host for Bridge {
    fn event(&self, event: Event) {
        let _ = self.events.try_send(event);
    }
    fn decide(&self, id: u64) -> Decision {
        let (answer, answered) = mpsc::channel();
        if let Ok(mut answers) = self.answers.lock() {
            answers.insert(id, answer);
        }
        // Nobody left to answer means the app is going away.
        answered.recv().unwrap_or(Decision::Cancel)
    }
}

struct Shared(Entity<WorktreeSetups>);
impl Global for Shared {}

pub(crate) fn get(cx: &App) -> Option<Entity<WorktreeSetups>> {
    cx.try_global::<Shared>().map(|shared| shared.0.clone())
}

/// Starts showing every workflow's setups. Call once.
pub(crate) fn install(cx: &mut App) {
    let (sender, events) = async_channel::unbounded();
    let answers = Answers::default();
    worktree_scripts::set_host(Arc::new(Bridge {
        events: sender,
        answers: answers.clone(),
    }));
    let view = cx.new(|_| WorktreeSetups {
        steps: Vec::new(),
        answers,
    });
    cx.set_global(Shared(view.clone()));
    cx.spawn(async move |cx| {
        while let Ok(event) = events.recv().await {
            view.update(cx, |view, cx| view.apply(event, cx));
        }
    })
    .detach();
}

/// "Copied .env, .env.local and 2 more", with any left out.
fn copied_summary(copied: &Copied) -> Option<String> {
    let mut summary = String::new();
    if copied.files > 0 {
        let shown = copied.names.iter().take(NAMES_SHOWN);
        let names = shown.map(String::as_str).collect::<Vec<_>>().join(", ");
        let more = copied
            .files
            .saturating_sub(NAMES_SHOWN.min(copied.names.len()));
        summary = match (names.is_empty(), more) {
            (true, _) => format!("Copied {} files", copied.files),
            (false, 0) => format!("Copied {names}"),
            (false, more) => format!("Copied {names} and {more} more"),
        };
    }
    if copied.skipped > 0 {
        if !summary.is_empty() {
            summary.push_str(" · ");
        }
        summary.push_str(&format!("Skipped {}", plural(copied.skipped, "file")));
    }
    (!summary.is_empty()).then_some(summary)
}
fn plural(count: usize, noun: &str) -> String {
    if count == 1 {
        format!("1 {noun}")
    } else {
        format!("{count} {noun}s")
    }
}

/// The heading for a step in `state`.
fn heading(run: &Run, state: &State) -> String {
    let name = run
        .branch
        .clone()
        .unwrap_or_else(|| run.worktree.display().to_string());
    match (run.stage, state) {
        (Stage::Setup, State::Running) => format!("Setting up {name}"),
        (Stage::Teardown, State::Running) => format!("Tearing down {name}"),
        (Stage::Setup, State::Failed(_)) => format!("Setup failed for {name}"),
        (Stage::Teardown, State::Failed(_)) => format!("Teardown failed for {name}"),
        (Stage::Setup, State::Done(Outcome::Passed)) if !run.scripted => format!("{name} is ready"),
        (Stage::Setup, State::Done(Outcome::Passed)) => format!("Setup passed for {name}"),
        (Stage::Teardown, State::Done(Outcome::Passed)) => format!("Teardown passed for {name}"),
        (Stage::Setup, State::Done(Outcome::Overridden(_))) => {
            format!("Started {name} without setup")
        }
        (Stage::Teardown, State::Done(Outcome::Overridden(_))) => {
            format!("Removing {name} after a failed teardown")
        }
        (_, State::Done(_)) => format!("Cancelled for {name}"),
    }
}

impl WorktreeSetups {
    fn apply(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Started(run) => self.steps.push(Step {
                run,
                tail: Tail::default(),
                state: State::Running,
                scroll: ScrollHandle::new(),
            }),
            Event::Output { id, line } => {
                if let Some(step) = self.step(id) {
                    step.tail.push(line);
                    step.scroll.scroll_to_bottom();
                }
            }
            Event::Retrying { id } => {
                if let Some(step) = self.step(id) {
                    step.state = State::Running;
                    step.tail.clear();
                }
            }
            Event::Failed { id, reason } => {
                if let Some(step) = self.step(id) {
                    step.state = State::Failed(reason);
                }
            }
            Event::Finished { id, outcome } => {
                if let Some(step) = self.step(id) {
                    step.state = State::Done(outcome);
                }
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(LINGER).await;
                    let _ = this.update(cx, |this, cx| {
                        this.steps.retain(|step| step.run.id != id);
                        cx.notify();
                    });
                })
                .detach();
            }
        }
        cx.notify();
    }
    fn step(&mut self, id: u64) -> Option<&mut Step> {
        self.steps.iter_mut().find(|step| step.run.id == id)
    }
    fn answer(&mut self, id: u64, decision: Decision, cx: &mut Context<Self>) {
        let answer = self.answers.lock().ok().and_then(|mut a| a.remove(&id));
        if let Some(answer) = answer {
            let _ = answer.send(decision);
        }
        cx.notify();
    }
    pub(crate) fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    fn render_step(&self, step: &Step, cx: &mut Context<Self>) -> AnyElement {
        let id = step.run.id;
        let selector = |name: &str| format!("worktree-setup-{name}-{id}");
        let lines: Vec<String> = step.tail.lines().map(str::to_owned).collect();
        let failed = matches!(step.state, State::Failed(_));
        let summary = copied_summary(&step.run.copied);
        let reason = match &step.state {
            State::Failed(reason) | State::Done(Outcome::Overridden(reason)) => {
                Some(reason.clone())
            }
            _ => None,
        };
        let go_on = match step.run.stage {
            Stage::Setup => "Start anyway",
            Stage::Teardown => "Remove anyway",
        };
        let button_id = |name: &str| SharedString::from(selector(name));
        div()
            .id(SharedString::from(selector("step")))
            .debug_selector({
                let selector = selector("step");
                move || selector.clone()
            })
            .role(if failed { Role::Alert } else { Role::Status })
            .aria_label(heading(&step.run, &step.state))
            .p(px(12.))
            .rounded(px(10.))
            .border_1()
            .border_color(content(0.12))
            .bg(floating_fill())
            .shadow_md()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(13.))
                    .font_weight(gpui::FontWeight::MEDIUM)
                    .child(heading(&step.run, &step.state)),
            )
            .children(summary.map(|summary| {
                div()
                    .text_size(px(12.))
                    .text_color(content(0.6))
                    .child(summary)
            }))
            .children(reason.map(|reason| {
                div()
                    .text_size(px(12.))
                    .text_color(diff_foreground(false))
                    .child(reason)
            }))
            .when(!lines.is_empty(), |card| {
                card.child(
                    div()
                        .id(SharedString::from(selector("output")))
                        .debug_selector({
                            let selector = selector("output");
                            move || selector.clone()
                        })
                        .track_scroll(&step.scroll)
                        .overflow_y_scroll()
                        .max_h(px(160.))
                        .p(px(8.))
                        .rounded(px(6.))
                        .bg(content(0.05))
                        .text_size(px(11.))
                        .text_color(content(0.75))
                        .children(lines.into_iter().map(|line| div().child(line))),
                )
            })
            .when(failed, |card| {
                card.child(
                    modal::actions()
                        .child(
                            modal::ghost_button(button_id("cancel"), "Cancel")
                                .debug_selector({
                                    let selector = selector("cancel");
                                    move || selector.clone()
                                })
                                .cursor_pointer()
                                .tab_index(0)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.answer(id, Decision::Cancel, cx);
                                })),
                        )
                        .child(
                            modal::ghost_button(button_id("continue"), go_on)
                                .debug_selector({
                                    let selector = selector("continue");
                                    move || selector.clone()
                                })
                                .cursor_pointer()
                                .tab_index(0)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.answer(id, Decision::Continue, cx);
                                })),
                        )
                        .child(
                            modal::primary_button(button_id("retry"), "Retry")
                                .debug_selector({
                                    let selector = selector("retry");
                                    move || selector.clone()
                                })
                                .cursor_pointer()
                                .tab_index(0)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.answer(id, Decision::Retry, cx);
                                })),
                        ),
                )
            })
            .when(step.state == State::Running && step.run.scripted, |card| {
                card.child(
                    modal::actions().child(
                        modal::ghost_button(button_id("stop"), "Cancel")
                            .debug_selector({
                                let selector = selector("stop");
                                move || selector.clone()
                            })
                            .cursor_pointer()
                            .tab_index(0)
                            .on_click(move |_, _, _| worktree_scripts::cancel(id)),
                    ),
                )
            })
            .into_any_element()
    }
}

impl Render for WorktreeSetups {
    fn render(&mut self, _: &mut gpui::Window, cx: &mut Context<Self>) -> impl IntoElement {
        let steps: Vec<AnyElement> = self
            .steps
            .iter()
            .map(|step| self.render_step(step, cx))
            .collect();
        div()
            .id("worktree-setups")
            .debug_selector(|| "worktree-setups".into())
            // A click here is not for the card or row underneath.
            .occlude()
            .absolute()
            .top(px(48.))
            .right(px(12.))
            .w(px(420.))
            .max_h(px(520.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(8.))
            .children(steps)
    }
}

#[cfg(test)]
#[path = "worktree_setups_tests.rs"]
mod tests;
