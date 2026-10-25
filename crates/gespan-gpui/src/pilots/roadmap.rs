//! The Pilot's SOP Roadmap tab: the Flows a Pilot owns, drawn as diagrams,
//! with a Flow picker, a Run picker and a details drawer for one state.
//!
//! The tab only reads the store, off the main thread; Flows change through
//! the Pilot's `flows.*` actions. With a run picked it draws that run's
//! snapshot (which may have been edited), otherwise the saved Flow.
use super::panels::{Route, empty_state};
use super::*;
use crate::flows::diagram::{DiagramEvent, DiagramView};
use crate::popover::{Anchor, MenuAction, MenuItem, menu_popover};
use gespan_core::flow::{
    FAILED, Flow, FlowRun, FlowState, FlowStore, FlowVisit, SUCCEEDED, diagram,
};
use gpui::FontWeight;

/// The empty state's copy, exactly.
pub(super) const EMPTY: &str = "This Pilot has no Flows yet. Ask it to create one below.";
const BLUEPRINT: &str = "Blueprint only";
/// Runs offered for the shown Flow, newest first.
const RUN_LIMIT: usize = 50;
const NOT_READY: &str = "Flows are not ready";
/// The ask input's placeholder, exactly.
pub(super) const ASK: &str = "Ask about this roadmap or ask for a change…";
/// Shown while a message sent from the tab waits for the Pilot, exactly.
pub(super) const QUEUED: &str = "Queued · the Pilot will pick this up when it's free";
/// The notice while one of the tab's queued messages failed to send.
pub(super) const SEND_FAILED: &str = "Couldn't send · retry it in the Pilot's chat";

/// "Open session" in the drawer: the Pilot opens this session's page.
pub(crate) struct OpenSession(pub i64);

/// The user asked the Pilot something from the tab: the chat message, its
/// last line naming what they are looking at when a Flow is shown.
pub(crate) struct AskPilot(pub String);

#[derive(Clone, Copy, PartialEq)]
enum Picker {
    Flow,
    Run,
}

/// One load from the store, for the selection it was asked for.
struct Load {
    flows: Vec<Flow>,
    flow: Option<String>,
    runs: Vec<FlowRun>,
    run: Option<(FlowRun, Vec<FlowVisit>)>,
}

/// Reads `pilot`'s Flows, newest `updatedAt` first, and resolves the asked
/// selection: a Flow that is gone falls back to the newest, a run that is
/// gone (or not of the shown Flow) to the blueprint.
fn load(
    store: &FlowStore,
    pilot: &str,
    flow: Option<&str>,
    run: Option<&str>,
) -> Result<Load, String> {
    let mut flows = store.owned(pilot)?;
    flows.sort_by(|a, b| {
        b.updated_at
            .cmp(&a.updated_at)
            .then_with(|| a.id.cmp(&b.id))
    });
    let kept = flow.filter(|id| flows.iter().any(|f| f.id == *id));
    let Some(shown) = kept.or_else(|| flows.first().map(|f| f.id.as_str())) else {
        return Ok(Load {
            flows,
            flow: None,
            runs: Vec::new(),
            run: None,
        });
    };
    let shown = shown.to_owned();
    // `shown` is one of `pilot`'s Flows; a run whose snapshot is not would
    // never load, so it is not offered.
    let mut runs = store.runs(&shown, RUN_LIMIT)?;
    runs.retain(|r| r.flow.owner.as_deref() == Some(pilot));
    let run = match run {
        Some(id) if kept.is_some() => store
            .owned_run(pilot, id)?
            .filter(|(run, _)| run.flow_id == shown),
        _ => None,
    };
    Ok(Load {
        flows,
        flow: Some(shown),
        runs,
        run,
    })
}

fn run_label(run: &FlowRun) -> String {
    match &run.tag {
        Some(tag) => format!("{} · {}", tag.task, tag.goal),
        None => run.id.clone(),
    }
}

pub(crate) struct Roadmap {
    pilot: String,
    owner: WeakEntity<Shell>,
    /// The Pilot's Flows, newest `updatedAt` first.
    pub(super) flows: Vec<Flow>,
    /// Runs of the shown Flow, newest first.
    pub(super) runs: Vec<FlowRun>,
    /// The shown Flow.
    pub(super) flow: Option<String>,
    /// The picked run; `None` is the blueprint.
    pub(super) run: Option<String>,
    /// The picked run's visits.
    visits: Vec<FlowVisit>,
    /// The definition on screen: the run's snapshot, else the saved Flow.
    pub(super) drawn: Option<Flow>,
    /// The last laid-out definition; status-only reloads reuse its layout.
    layout: Option<(Flow, diagram::Diagram)>,
    /// Per Flow id (blueprint) or run id (snapshot): the definition last
    /// loaded and how many of the run's edits were seen, so a reload badges
    /// only what changed since.
    last_loaded: HashMap<String, (Flow, usize)>,
    /// The state whose details are open.
    pub(super) drawer: Option<String>,
    /// The diagram's selected state.
    selected: Option<String>,
    pub(super) generation: u64,
    /// How many times a definition was laid out.
    #[cfg(test)]
    pub(super) layouts: usize,
    pub(super) loaded: bool,
    pub(super) error: Option<String>,
    pub(super) diagram: Entity<DiagramView>,
    picker: Option<(Picker, Entity<Menu>, Subscription)>,
    flow_anchor: AnchorSlot,
    run_anchor: AnchorSlot,
    /// The question or change request for the Pilot.
    pub(super) ask: Entity<Input>,
    /// Messages sent from the tab that the Pilot has not taken yet.
    pub(super) queued: Vec<String>,
    /// One of `queued` failed to send and waits for a retry in the chat.
    queue_failed: bool,
    _diagram_events: Subscription,
    _ask_events: Subscription,
}

impl EventEmitter<OpenSession> for Roadmap {}
impl EventEmitter<AskPilot> for Roadmap {}

impl Roadmap {
    pub fn new(pilot: String, owner: WeakEntity<Shell>, cx: &mut Context<Self>) -> Self {
        let diagram = cx.new(DiagramView::new);
        let events = cx.subscribe(&diagram, |this, _, event: &DiagramEvent, cx| {
            match event {
                DiagramEvent::Selected(id) => {
                    this.selected = id.clone();
                    // An open drawer follows the selection.
                    if let (Some(_), Some(id)) = (&this.drawer, id) {
                        this.drawer = Some(id.clone());
                    }
                }
                DiagramEvent::Open(id) => this.drawer = Some(id.clone()),
            }
            cx.notify();
        });
        let ask = cx.new(|cx| {
            let mut input = Input::new(cx);
            input.set_font_size(13.);
            input.set_label("Ask the Pilot about this roadmap");
            input.set_placeholder(ASK, cx);
            input
        });
        let ask_events = cx.subscribe(&ask, |this, _, _: &crate::input::Submitted, cx| {
            this.send_ask(cx)
        });
        Self {
            pilot,
            owner,
            flows: Vec::new(),
            runs: Vec::new(),
            flow: None,
            run: None,
            visits: Vec::new(),
            drawn: None,
            layout: None,
            last_loaded: HashMap::new(),
            drawer: None,
            selected: None,
            generation: 0,
            #[cfg(test)]
            layouts: 0,
            loaded: false,
            error: None,
            diagram,
            picker: None,
            flow_anchor: AnchorSlot::default(),
            run_anchor: AnchorSlot::default(),
            ask,
            queued: Vec::new(),
            queue_failed: false,
            _diagram_events: events,
            _ask_events: ask_events,
        }
    }

    /// Sends the input's text to the Pilot with the selection's context
    /// line; blank input sends nothing. With no Flow shown the text goes
    /// alone, as the user may be asking for a first Flow.
    fn send_ask(&mut self, cx: &mut Context<Self>) {
        let Some(text) = self.ask.update(cx, |input, cx| input.take(cx)) else {
            return;
        };
        let text = text.trim();
        let message = match self.selection_context() {
            Some(context) => format!("{text}\n{context}"),
            None => text.to_owned(),
        };
        cx.emit(AskPilot(message));
    }

    /// Message `id`, sent from the tab, waits for the Pilot.
    pub(super) fn queue(&mut self, id: String, cx: &mut Context<Self>) {
        self.queued.push(id);
        cx.notify();
    }

    /// Keeps only the tab's messages still waiting. `outbox` says, per
    /// message, whether it is still queued (`None`: gone) and whether its
    /// send failed.
    pub(super) fn sync_queued(
        &mut self,
        outbox: impl Fn(&str) -> Option<bool>,
        cx: &mut Context<Self>,
    ) {
        let before = (self.queued.len(), self.queue_failed);
        self.queued.retain(|id| outbox(id).is_some());
        self.queue_failed = self.queued.iter().any(|id| outbox(id) == Some(true));
        if (self.queued.len(), self.queue_failed) != before {
            cx.notify();
        }
    }

    /// Reloads off the main thread; a result older than the latest reload
    /// is dropped.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let (owner, pilot) = (self.owner.clone(), self.pilot.clone());
        let (flow, run) = (self.flow.clone(), self.run.clone());
        cx.spawn(async move |this, cx| {
            // Read here, not in `reload`, which may run inside a Shell update.
            let store = owner
                .read_with(cx, |shell, _| shell.pilot_flows_store())
                .unwrap_or_else(|_| Err(NOT_READY.into()));
            let result = match store {
                Ok(store) => {
                    cx.background_executor()
                        .spawn(async move {
                            let store = store
                                .lock()
                                .map_err(|_| "Flows are unavailable".to_string())?;
                            load(&store, &pilot, flow.as_deref(), run.as_deref())
                        })
                        .await
                }
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                this.loaded = true;
                match result {
                    Ok(load) => {
                        this.error = None;
                        this.apply(load, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply(&mut self, load: Load, cx: &mut Context<Self>) {
        self.flows = load.flows;
        self.runs = load.runs;
        self.flow = load.flow;
        self.run = load.run.as_ref().map(|(run, _)| run.id.clone());
        let (drawn, overlay, visits, edits) = match load.run {
            Some((run, visits)) => {
                let overlay = diagram::overlay(&run, &visits);
                let key = run.id.clone();
                (
                    Some(run.flow),
                    Some(overlay),
                    visits,
                    Some((key, run.edits)),
                )
            }
            None => (self.saved_flow().cloned(), None, Vec::new(), None),
        };
        self.visits = visits;
        if self.drawer.as_ref().is_some_and(|id| {
            !drawn
                .as_ref()
                .is_some_and(|f| f.states.iter().any(|s| s.id() == id))
        }) {
            self.drawer = None;
        }
        match &drawn {
            Some(flow) => {
                let laid_out = match &self.layout {
                    Some((before, laid_out)) if before == flow => laid_out.clone(),
                    _ => {
                        let laid_out = diagram::build(flow);
                        #[cfg(test)]
                        {
                            self.layouts += 1;
                        }
                        self.layout = Some((flow.clone(), laid_out.clone()));
                        laid_out
                    }
                };
                let (key, edits) = edits.unwrap_or_else(|| (flow.id.clone(), Vec::new()));
                let changed = self.changed_since_last_load(key, flow, &edits);
                let structure = diagram::structure_key(flow);
                self.diagram.update(cx, |view, cx| {
                    view.set(laid_out, overlay, changed, structure, cx)
                });
            }
            // `selected` stays the widget's: it keeps its selection too.
            None => self.layout = None,
        }
        self.drawn = drawn;
        // Forget Flows and runs that are gone; the picked run is kept even
        // when it has fallen out of the newest runs.
        let (flows, runs, run) = (&self.flows, &self.runs, &self.run);
        self.last_loaded.retain(|key, _| {
            flows.iter().any(|f| &f.id == key)
                || runs.iter().any(|r| &r.id == key)
                || run.as_ref() == Some(key)
        });
    }

    /// The states of `flow` (shown under `key`) changed since `key` last
    /// loaded: a diff against that copy plus the run's newer edits. The
    /// first load of a key badges nothing, and a status-only reload names
    /// nothing, so the badges it already wears are not restarted.
    fn changed_since_last_load(
        &mut self,
        key: String,
        flow: &Flow,
        edits: &[gespan_core::flow::RunEdit],
    ) -> HashSet<String> {
        let changed = match self.last_loaded.get(&key) {
            Some((before, seen)) if before != flow || *seen < edits.len() => {
                diagram::changed_states(Some(before), flow, &edits[(*seen).min(edits.len())..])
            }
            _ => HashSet::new(),
        };
        self.last_loaded.insert(key, (flow.clone(), edits.len()));
        changed
    }

    fn saved_flow(&self) -> Option<&Flow> {
        let id = self.flow.as_deref()?;
        self.flows.iter().find(|f| f.id == id)
    }

    /// Shows `flow`'s blueprint.
    pub(super) fn select_flow(&mut self, flow: String, cx: &mut Context<Self>) {
        if self.flow.as_deref() != Some(flow.as_str()) {
            self.flow = Some(flow);
            self.run = None;
            self.drawer = None;
        }
        self.reload(cx);
    }

    /// Shows `run` of the shown Flow; `None` is its blueprint.
    pub(super) fn select_run(&mut self, run: Option<String>, cx: &mut Context<Self>) {
        if self.run != run {
            self.run = run;
            self.drawer = None;
        }
        self.reload(cx);
    }

    /// "Blueprint only", then the shown Flow's runs as "<task> · <goal>".
    pub(super) fn run_choices(&self) -> Vec<(Option<String>, String)> {
        std::iter::once((None, BLUEPRINT.to_string()))
            .chain(
                self.runs
                    .iter()
                    .map(|run| (Some(run.id.clone()), run_label(run))),
            )
            .collect()
    }

    /// The line naming what the user is looking at, for a message to the Pilot:
    /// `[SOP Roadmap · Flow "<name>" (<flowId>) · run <runId|blueprint> · step "<stateId>"]`,
    /// the step only while a state is selected.
    pub fn selection_context(&self) -> Option<String> {
        // Runs of whitespace (newlines too) become one space, so the line
        // stays a single line. Inside the quotes, `\` and `"` are escaped and
        // `]` becomes `)`, so a name can neither end its quote nor close the
        // bracket early.
        let quoted = |text: &str| {
            text.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .replace('\\', "\\\\")
                .replace('"', "\\\"")
                .replace(']', ")")
        };
        let drawn = self.drawn.as_ref()?;
        let name = quoted(self.saved_flow().map_or(&drawn.name, |f| &f.name));
        let run = self.run.as_deref().unwrap_or("blueprint");
        let mut line = format!("[SOP Roadmap · Flow \"{name}\" ({}) · run {run}", drawn.id);
        if let Some(step) = &self.selected {
            line.push_str(&format!(" · step \"{}\"", quoted(step)));
        }
        line.push(']');
        Some(line)
    }

    fn open_picker(&mut self, which: Picker, window: &mut Window, cx: &mut Context<Self>) {
        let items: Vec<MenuItem> = match which {
            Picker::Flow => self
                .flows
                .iter()
                .map(|f| {
                    MenuAction::new(format!("flow:{}", f.id), f.name.clone())
                        .checked(self.flow.as_deref() == Some(f.id.as_str()))
                        .into()
                })
                .collect(),
            Picker::Run => self
                .run_choices()
                .into_iter()
                .map(|(id, label)| {
                    let checked = self.run == id;
                    MenuAction::new(format!("run:{}", id.unwrap_or_default()), label)
                        .checked(checked)
                        .into()
                })
                .collect(),
        };
        let label = match which {
            Picker::Flow => "Flow",
            Picker::Run => "Run",
        };
        let menu = cx.new(|cx| Menu::new(label, items, window, cx).width(280.));
        let events = cx.subscribe(&menu, |this, _, event: &MenuEvent, cx| {
            this.picker = None;
            if let MenuEvent::Picked(id, _) = event {
                if let Some(flow) = id.strip_prefix("flow:") {
                    this.select_flow(flow.to_owned(), cx);
                } else if let Some(run) = id.strip_prefix("run:") {
                    this.select_run((!run.is_empty()).then(|| run.to_owned()), cx);
                }
            }
            cx.notify();
        });
        self.picker = Some((which, menu, events));
        cx.notify();
    }

    fn picker_button(
        &self,
        which: Picker,
        caption: &'static str,
        value: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (id, anchor) = match which {
            Picker::Flow => ("pilot-roadmap-flow-picker", &self.flow_anchor),
            Picker::Run => ("pilot-roadmap-run-picker", &self.run_anchor),
        };
        let open = self.picker.as_ref().is_some_and(|(p, _, _)| *p == which);
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Button)
            .aria_label(SharedString::from(format!("{caption}: {value}")))
            .relative()
            .flex_1()
            .min_w_0()
            .h(px(28.))
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(content(0.1))
            .when(open, |d| d.bg(content(0.08)))
            .hover(|s| s.bg(content(0.06)))
            .cursor_pointer()
            .flex()
            .items_center()
            .gap_1p5()
            .text_size(px(12.))
            .child(anchor.probe())
            .child(
                div()
                    .flex_shrink_0()
                    .text_color(content(0.45))
                    .child(caption),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_color(content(0.9))
                    .child(value),
            )
            .child(icon("chevron-down", 12.))
            .on_click(cx.listener(move |this, _, window, cx| {
                if this.picker.as_ref().is_some_and(|(p, _, _)| *p == which) {
                    this.picker = None;
                    cx.notify();
                } else {
                    this.open_picker(which, window, cx);
                }
            }))
            .into_any_element()
    }

    fn drawer_view(&self, state_id: &str, cx: &mut Context<Self>) -> Option<AnyElement> {
        let flow = self.drawn.as_ref()?;
        let state = flow.states.iter().find(|s| s.id() == state_id)?;
        let (title, text, outputs) = match state {
            FlowState::Agent(s) => (s.title.clone(), s.brief.clone(), s.outputs.clone()),
            FlowState::Approval(s) => (
                s.title.clone(),
                s.message.clone(),
                gespan_core::flow::definition::approval_outputs(),
            ),
        };
        let target = |to: &str| match to {
            SUCCEEDED => "Succeeded".to_string(),
            FAILED => "Failed".to_string(),
            id => flow
                .states
                .iter()
                .find(|s| s.id() == id)
                .map_or_else(|| id.to_string(), |s| state_title(s).to_string()),
        };
        let transitions: Vec<(String, Option<String>)> = state
            .transitions()
            .iter()
            .zip(diagram::transition_labels(state.transitions()))
            .map(|(t, label)| (target(&t.to), label))
            .collect();
        let now = gespan_core::time::now_ms() as i64;
        let visits: Vec<&FlowVisit> = self
            .visits
            .iter()
            .filter(|v| v.state_id == state_id)
            .collect();

        let section = |heading: &'static str| {
            div().flex().flex_col().gap_1().child(
                div()
                    .text_size(px(11.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(0.5))
                    .child(heading),
            )
        };
        let line = |text: String| {
            div()
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(content(0.85))
                .child(text)
        };
        let mut body = div()
            .id("pilot-roadmap-drawer-body")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .p_3()
            .flex()
            .flex_col()
            .gap_4()
            .child(section("Brief").child(line(text)));
        if !outputs.is_empty() {
            body = body.child(section("Outputs").children(outputs.into_iter().map(|o| {
                line(format!(
                    "{} ({}) · {}",
                    o.name,
                    serde_json::to_value(o.r#type)
                        .ok()
                        .and_then(|v| v.as_str().map(str::to_owned))
                        .unwrap_or_default(),
                    o.description
                ))
            })));
        }
        body = body.child(section("Transitions").children(transitions.into_iter().map(
            |(to, label)| match label {
                Some(label) => line(format!("→ {to} · when {label}")),
                None => line(format!("→ {to}")),
            },
        )));
        let mut visit_rows = section("Visits");
        if visits.is_empty() {
            visit_rows = visit_rows.child(line(if self.run.is_some() {
                "Not reached in this run.".into()
            } else {
                "Pick a run to see its visits.".into()
            }));
        }
        for visit in visits {
            let seq = visit.seq;
            let session = visit
                .session_id
                .as_deref()
                .and_then(|s| s.parse::<i64>().ok());
            visit_rows = visit_rows.child(
                div()
                    .id(SharedString::from(format!("pilot-roadmap-visit-{seq}")))
                    .debug_selector(move || format!("pilot-roadmap-visit-{seq}"))
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .size(px(8.))
                            .rounded_full()
                            .flex_shrink_0()
                            .bg(crate::flows::status_color(visit.status.as_str())),
                    )
                    .child(
                        line(format!(
                            "#{seq} · {} · {}",
                            crate::flows::visit_status_label(visit.status),
                            crate::flows::duration(visit, now)
                        ))
                        .flex_1()
                        .min_w_0(),
                    )
                    .when_some(session, |d, session| {
                        d.child(
                            super::panels::button(
                                SharedString::from(format!("pilot-roadmap-open-session-{seq}")),
                                "Open session",
                            )
                            .on_click(
                                cx.listener(move |_, _, _, cx| cx.emit(OpenSession(session))),
                            ),
                        )
                    }),
            );
        }
        body = body.child(visit_rows);
        Some(
            div()
                .id("pilot-roadmap-drawer")
                .debug_selector(|| "pilot-roadmap-drawer".into())
                .role(Role::Region)
                .aria_label(SharedString::from(format!("{title} details")))
                .absolute()
                .top_0()
                .right_0()
                .bottom_0()
                .w(gpui::relative(0.9))
                .max_w(px(340.))
                .occlude()
                .bg(floating_fill())
                .border_l_1()
                .border_color(content(0.1))
                .flex()
                .flex_col()
                .child(
                    div()
                        .h(px(36.))
                        .flex_shrink_0()
                        .pl_3()
                        .pr_1()
                        .border_b_1()
                        .border_color(content(0.07))
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(content(1.))
                                .child(title),
                        )
                        .child(
                            icon_button("pilot-roadmap-drawer-close", "Close details", "close")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.drawer = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(body)
                .into_any_element(),
        )
    }
}

fn state_title(state: &FlowState) -> &str {
    match state {
        FlowState::Agent(s) => &s.title,
        FlowState::Approval(s) => &s.title,
    }
}

impl Roadmap {
    /// The notice over the input while a message from the tab waits.
    pub(super) fn queued_notice(&self) -> Option<&'static str> {
        (!self.queued.is_empty()).then_some(if self.queue_failed {
            SEND_FAILED
        } else {
            QUEUED
        })
    }

    /// The ask input, under the queued notice while a message waits.
    fn ask_view(&self) -> AnyElement {
        let notice = self.queued_notice();
        div()
            .id("pilot-roadmap-ask")
            .debug_selector(|| "pilot-roadmap-ask".into())
            .flex_shrink_0()
            .px_2()
            .py(px(6.))
            .border_t_1()
            .border_color(crate::right_panel::stroke())
            .flex()
            .flex_col()
            .gap_1()
            .when_some(notice, |d, notice| {
                d.child(
                    div()
                        .id("pilot-roadmap-queued")
                        .debug_selector(|| "pilot-roadmap-queued".into())
                        .role(Role::Status)
                        .aria_label(notice)
                        .px_1()
                        .text_size(px(11.))
                        .text_color(content(0.55))
                        .child(notice),
                )
            })
            .child(
                div()
                    .rounded_md()
                    .border_1()
                    .border_color(content(0.1))
                    .px_2()
                    .py_1()
                    .child(self.ask.clone()),
            )
            .into_any_element()
    }

    /// The tab above the ask input: the empty state or the pickers and
    /// diagram; nothing until the first load.
    fn content(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.loaded {
            return None;
        }
        if self.flows.is_empty() {
            let message = self.error.clone().unwrap_or_else(|| EMPTY.to_string());
            return Some(
                div()
                    .id("pilot-roadmap-empty")
                    .debug_selector(|| "pilot-roadmap-empty".into())
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(empty_state("git-merge", &message))
                    .into_any_element(),
            );
        }
        let flow_name = self
            .saved_flow()
            .map(|f| f.name.clone())
            .unwrap_or_default();
        let run_name = self
            .run_choices()
            .into_iter()
            .find(|(id, _)| *id == self.run)
            .map_or_else(|| BLUEPRINT.to_string(), |(_, label)| label);
        let drawer = self.drawer.clone().and_then(|id| self.drawer_view(&id, cx));
        let popover = self.picker.as_ref().map(|(which, menu, _)| {
            let anchor = match which {
                Picker::Flow => self.flow_anchor.clone(),
                Picker::Run => self.run_anchor.clone(),
            };
            menu_popover(menu, Anchor::Slot(anchor), cx).priority(60)
        });
        Some(
            div()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .flex()
                .flex_col()
                // Pickers stay on top; the diagram takes the rest, so plain
                // wheel scrolls (which pan it) never need the tab to scroll.
                .child(
                    div()
                        .flex_shrink_0()
                        .px_2()
                        .py(px(6.))
                        .border_b_1()
                        .border_color(crate::right_panel::stroke())
                        .flex()
                        .items_center()
                        .gap_1p5()
                        .child(self.picker_button(Picker::Flow, "Flow", flow_name, cx))
                        .child(self.picker_button(Picker::Run, "Run", run_name, cx)),
                )
                .when_some(self.error.clone(), |d, error| {
                    d.child(
                        div()
                            .flex_shrink_0()
                            .px_3()
                            .py_1()
                            .text_size(px(11.))
                            .text_color(rgb(0xf87171))
                            .child(error),
                    )
                })
                .child(
                    div()
                        .id("pilot-roadmap-canvas")
                        .relative()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_hidden()
                        .when(self.drawn.is_some(), |d| d.child(self.diagram.clone()))
                        .children(drawer),
                )
                .children(popover)
                .into_any_element(),
        )
    }
}

impl Render for Roadmap {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = self.content(cx);
        div()
            .id("pilot-roadmap")
            .debug_selector(|| "pilot-roadmap".into())
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(content.unwrap_or_else(|| div().flex_1().into_any_element()))
            .child(self.ask_view())
    }
}

impl Pilots {
    /// `pilot`'s roadmap, made (and loaded) on first use.
    pub(super) fn roadmap(&mut self, pilot: &str, cx: &mut Context<Self>) -> Entity<Roadmap> {
        if let Some((roadmap, _)) = self.roadmaps.get(pilot) {
            return roadmap.clone();
        }
        let owner = self.owner.clone();
        let roadmap = cx.new(|cx| {
            let mut roadmap = Roadmap::new(pilot.to_owned(), owner, cx);
            roadmap.reload(cx);
            roadmap
        });
        // "Open session" reuses the Sessions tab's session page.
        let open = cx.subscribe(&roadmap, |this, _, event: &OpenSession, cx| {
            this.route = Some(Route::Session(event.0));
            cx.notify();
        });
        let source = pilot.to_owned();
        let ask = cx.subscribe(&roadmap, move |this, roadmap, event: &AskPilot, cx| {
            this.ask_from_roadmap(&source, event.0.clone(), roadmap, cx)
        });
        self.roadmaps
            .insert(pilot.to_owned(), (roadmap.clone(), [open, ask]));
        // It just loaded.
        self.stale_roadmaps.remove(pilot);
        roadmap
    }

    /// A question or change request from `pilot`'s roadmap goes to its chat
    /// as a visible message, queued like one typed there; the tab shows it
    /// waiting until the chat takes it.
    fn ask_from_roadmap(
        &mut self,
        pilot: &str,
        message: String,
        roadmap: Entity<Roadmap>,
        cx: &mut Context<Self>,
    ) {
        self.ensure_chat(pilot, cx);
        if let Some(id) = self.enqueue(pilot, message, Vec::new(), false, cx) {
            roadmap.update(cx, |r, cx| r.queue(id, cx));
            // Delivery may already have dropped it from the outbox.
            self.sync_roadmap_queues(cx);
        }
    }

    /// Clears each roadmap's queued notice for messages that left the
    /// Pilot's outbox (delivered, discarded, or the Pilot is gone), and
    /// tells it which of the rest failed to send.
    pub(super) fn sync_roadmap_queues(&mut self, cx: &mut Context<Self>) {
        for (pilot, (roadmap, _)) in &self.roadmaps {
            if roadmap.read(cx).queued.is_empty() {
                continue;
            }
            let outbox = self.roster.get(pilot).map(|p| &p.outbox);
            roadmap.update(cx, |r, cx| {
                r.sync_queued(
                    |id| {
                        outbox
                            .and_then(|o| o.iter().find(|m| m.id == id))
                            .map(|m| m.error.is_some())
                    },
                    cx,
                )
            });
        }
    }

    /// `pilot`'s roadmap as drawn in its Overview: reloaded first if it went
    /// stale. Only a drawn roadmap reloads, so one behind a closed panel,
    /// another tab or another Pilot waits until it is on screen again.
    pub(super) fn shown_roadmap(&mut self, pilot: &str, cx: &mut Context<Self>) -> Entity<Roadmap> {
        let roadmap = self.roadmap(pilot, cx);
        if self.stale_roadmaps.remove(pilot) {
            roadmap.update(cx, |r, cx| r.reload(cx));
        }
        roadmap
    }

    /// Marks `pilot`'s roadmap for a reload the next time it is drawn.
    fn stale_roadmap(&mut self, pilot: &str, cx: &mut Context<Self>) {
        if self.roadmaps.contains_key(pilot) {
            self.stale_roadmaps.insert(pilot.to_owned());
            cx.notify();
        }
    }

    /// Reloads the shown Pilot's roadmap, now if it is drawn, else when it
    /// next is: on the clock, on picking its tab and on showing the Pilot.
    pub(super) fn reload_roadmap(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.active.clone() {
            self.stale_roadmap(&id, cx);
        }
    }

    /// A successful mutating `flows.*` call changed `pilot`'s Flows or runs.
    pub(super) fn flows_changed(&mut self, pilot: &str, cx: &mut Context<Self>) {
        #[cfg(test)]
        self.flows_changes.push(pilot.to_owned());
        self.stale_roadmap(pilot, cx);
    }

    /// Run `run_id` of `pilot` moved on: its roadmap repaints the status if
    /// it shows that run.
    pub(super) fn roadmap_run_moved(&mut self, pilot: &str, run_id: &str, cx: &mut Context<Self>) {
        let shows = self
            .roadmaps
            .get(pilot)
            .is_some_and(|(r, _)| r.read(cx).run.as_deref() == Some(run_id));
        if shows {
            self.stale_roadmap(pilot, cx);
        }
    }
}
