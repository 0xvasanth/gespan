//! The Pilot's Pipeline tab: the Flows a Pilot owns, drawn top to bottom as
//! the board draws them, with pipeline and version pickers, the run shown
//! ("This run"), its earlier runs and a details drawer for one step.
//!
//! The tab only reads the store, off the main thread; Flows change through
//! the Pilot's `flows.*` actions. With a run shown it draws that run's
//! snapshot (which may have been edited), otherwise the saved Flow.
use super::graph::{GraphEvent, PipelineGraph, faint_ink};
use super::panels::{Route, empty_state};
use super::*;
use crate::popover::{Anchor, MenuAction, MenuItem, menu_popover};
use gespan_core::flow::graph::{self as layout, GraphRow};
use gespan_core::flow::{
    FAILED, Flow, FlowRun, FlowState, FlowStore, FlowVisit, RunStatus, SUCCEEDED, diagram,
};
use gpui::{FontWeight, Hsla, ScrollHandle};

/// The empty state's copy, exactly.
pub(super) const EMPTY: &str = "This Pilot has no Flows yet. Ask it to create one below.";
/// Runs offered for the shown Flow, newest first.
const RUN_LIMIT: usize = 50;
const NOT_READY: &str = "Flows are not ready";
/// The ask input's placeholder, exactly.
pub(super) const ASK: &str = "Ask to change this pipeline";
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
    Version,
}

/// One load from the store, for the selection it was asked for.
struct Load {
    flows: Vec<Flow>,
    flow: Option<String>,
    /// The shown Flow's versions, its draft first.
    versions: Vec<gespan_core::flow::FlowVersion>,
    runs: Vec<FlowRun>,
    run: Option<(FlowRun, Vec<FlowVisit>)>,
}

/// Reads `pilot`'s Flows, newest `updatedAt` first, and resolves the asked
/// selection: a Flow that is gone falls back to the newest, a run that is
/// gone (or not of the shown Flow) to the blueprint. With no run asked for
/// and `follow` set, the newest run still going is shown.
fn load(
    store: &FlowStore,
    pilot: &str,
    flow: Option<&str>,
    run: Option<&str>,
    follow: bool,
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
            versions: Vec::new(),
            runs: Vec::new(),
            run: None,
        });
    };
    let shown = shown.to_owned();
    // `shown` is one of `pilot`'s Flows; a run whose snapshot is not would
    // never load, so it is not offered.
    let mut runs = store.runs(&shown, RUN_LIMIT)?;
    runs.retain(|r| r.flow.owner.as_deref() == Some(pilot));
    let going = runs
        .iter()
        .find(|r| r.status.is_active())
        .map(|r| r.id.clone())
        .filter(|_| follow);
    let run = match run.or(going.as_deref()) {
        Some(id) if kept.is_some() || going.as_deref() == Some(id) => store
            .owned_run(pilot, id)?
            .filter(|(run, _)| run.flow_id == shown),
        _ => None,
    };
    let versions = store.versions(&shown)?;
    Ok(Load {
        flows,
        flow: Some(shown),
        versions,
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
    /// The person picked the shown Flow or a run. Until then the Pipeline
    /// tab, open from the start, follows the Pilot's newest Flow.
    picked: bool,
    /// The picked run; `None` is the blueprint.
    pub(super) run: Option<String>,
    /// The shown Flow's versions, its draft first, then newest first.
    pub(super) versions: Vec<gespan_core::flow::FlowVersion>,
    /// The version drawn instead of the live one: an earlier one or the
    /// draft. `None` is the live version.
    pub(super) version: Option<i64>,
    /// The Flow shown as YAML instead of as a graph.
    pub(super) yaml: bool,
    /// The YAML view shows the draft's lines against the live version's.
    pub(super) diff: bool,
    /// The picked run's visits.
    visits: Vec<FlowVisit>,
    /// The definition on screen: the run's snapshot, else the saved Flow.
    pub(super) drawn: Option<Flow>,
    /// The last laid-out definition; status-only reloads reuse its rows.
    layout: Option<(Flow, Vec<GraphRow>)>,
    /// Per Flow id (blueprint) or run id (snapshot): the definition last
    /// loaded and how many of the run's edits were seen, so a reload badges
    /// only what changed since.
    last_loaded: HashMap<String, (Flow, usize)>,
    /// The state whose details are open.
    pub(super) drawer: Option<String>,
    /// The graph's selected step.
    selected: Option<String>,
    pub(super) generation: u64,
    /// How many times a definition was laid out.
    #[cfg(test)]
    pub(super) layouts: usize,
    pub(super) loaded: bool,
    pub(super) error: Option<String>,
    pub(super) graph: Entity<PipelineGraph>,
    /// Scrolls the run card, the graph and the earlier runs together.
    pub(super) scroll: ScrollHandle,
    /// Watches the shown run's running sessions, to mark a step waiting on
    /// the user the moment its session asks.
    watching: Vec<(String, i64, Subscription)>,
    picker: Option<(Picker, Entity<Menu>, Subscription)>,
    flow_anchor: AnchorSlot,
    version_anchor: AnchorSlot,
    /// The question or change request for the Pilot.
    pub(super) ask: Entity<Input>,
    /// Messages sent from the tab that the Pilot has not taken yet.
    pub(super) queued: Vec<String>,
    /// One of `queued` failed to send and waits for a retry in the chat.
    queue_failed: bool,
    _graph_events: Subscription,
    _ask_events: Subscription,
}

impl EventEmitter<OpenSession> for Roadmap {}
impl EventEmitter<AskPilot> for Roadmap {}

impl Roadmap {
    pub fn new(pilot: String, owner: WeakEntity<Shell>, cx: &mut Context<Self>) -> Self {
        let graph = cx.new(PipelineGraph::new);
        let events = cx.subscribe(&graph, |this, _, event: &GraphEvent, cx| {
            match event {
                GraphEvent::Selected(id) => {
                    this.selected = id.clone();
                    // An open drawer follows the selection.
                    if let (Some(_), Some(id)) = (&this.drawer, id) {
                        this.drawer = Some(id.clone());
                    }
                }
                GraphEvent::Open(id) => this.drawer = Some(id.clone()),
                GraphEvent::Answer(session) => cx.emit(OpenSession(*session)),
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
            picked: false,
            run: None,
            versions: Vec::new(),
            version: None,
            yaml: false,
            diff: false,
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
            graph,
            scroll: ScrollHandle::new(),
            watching: Vec::new(),
            picker: None,
            flow_anchor: AnchorSlot::default(),
            version_anchor: AnchorSlot::default(),
            ask,
            queued: Vec::new(),
            queue_failed: false,
            _graph_events: events,
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
        let flow = self.flow.clone().filter(|_| self.picked);
        let run = self.run.clone();
        // Until the person picks, the tab shows the run still going; a
        // drawn earlier version or draft has no runs here.
        let follow = !self.picked && self.version.is_none();
        cx.spawn(async move |this, cx| {
            // Read here, not in `reload`, which may run inside a Shell update.
            // A change that just reported success may still be writing.
            let store = owner
                .update(cx, |shell, cx| {
                    shell
                        .pilot_flows_store()
                        .map(|store| (store, shell.flow_writes_saved(cx)))
                })
                .unwrap_or_else(|_| Err(NOT_READY.into()));
            let result = match store {
                Ok((store, saved)) => {
                    saved.await;
                    cx.background_executor()
                        .spawn(async move {
                            let store = store
                                .lock()
                                .map_err(|_| "Flows are unavailable".to_string())?;
                            load(&store, &pilot, flow.as_deref(), run.as_deref(), follow)
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
        if self.flow != load.flow {
            self.version = None;
        }
        self.flow = load.flow;
        self.versions = load.versions;
        // A version that is gone (a saved or discarded draft) shows live.
        if self
            .version
            .is_some_and(|v| !self.versions.iter().any(|ver| ver.version == v))
        {
            self.version = None;
        }
        self.run = load.run.as_ref().map(|(run, _)| run.id.clone());
        let (drawn, current, visits, edits) = match load.run {
            Some((run, visits)) => {
                let key = run.id.clone();
                (
                    Some(run.flow),
                    run.current_state,
                    visits,
                    Some((key, run.edits)),
                )
            }
            None => (
                self.picked_version()
                    .map(|v| v.flow.clone())
                    .or_else(|| self.saved_flow().cloned()),
                None,
                Vec::new(),
                None,
            ),
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
                let rows = match &self.layout {
                    Some((before, rows)) if before == flow => rows.clone(),
                    _ => {
                        let rows = layout::layout(flow);
                        #[cfg(test)]
                        {
                            self.layouts += 1;
                        }
                        self.layout = Some((flow.clone(), rows.clone()));
                        rows
                    }
                };
                let (key, edits) = edits.unwrap_or_else(|| (flow.id.clone(), Vec::new()));
                let changed = self.changed_since_last_load(key, flow, &edits);
                let cards = layout::cards(flow, &rows, &self.visits);
                let flow = flow.clone();
                self.graph.update(cx, |graph, cx| {
                    graph.set(flow, rows, cards, current, changed, cx)
                });
            }
            // `selected` stays the graph's: it keeps its selection too.
            None => self.layout = None,
        }
        self.drawn = drawn;
        self.watch_waiting(cx);
        // Forget Flows and runs that are gone; the picked run is kept even
        // when it has fallen out of the newest runs.
        let (flows, runs, run) = (&self.flows, &self.runs, &self.run);
        self.last_loaded.retain(|key, _| {
            flows.iter().any(|f| &f.id == key)
                || runs.iter().any(|r| &r.id == key)
                || run.as_ref() == Some(key)
        });
    }

    /// Watches the session of each running step of the shown run, and tells
    /// the graph which of them wait for the user.
    fn watch_waiting(&mut self, cx: &mut Context<Self>) {
        let running: Vec<(String, i64)> = self
            .visits
            .iter()
            .filter(|v| v.status == gespan_core::flow::VisitStatus::Running)
            .filter_map(|v| Some((v.state_id.clone(), v.session_id.as_deref()?.parse().ok()?)))
            .collect();
        let same = running.len() == self.watching.len()
            && running
                .iter()
                .zip(&self.watching)
                .all(|((s, id), (ws, wid, _))| s == ws && id == wid);
        if !same {
            let owner = self.owner.clone();
            self.watching = running
                .into_iter()
                .filter_map(|(state, session)| {
                    let view = owner.upgrade()?.read(cx).session_view(session)?;
                    let watch = cx.observe(&view, |this, _, cx| this.mark_waiting(cx));
                    Some((state, session, watch))
                })
                .collect();
        }
        self.mark_waiting(cx);
    }

    fn mark_waiting(&mut self, cx: &mut Context<Self>) {
        let Some(shell) = self.owner.upgrade() else {
            return;
        };
        let waiting: HashSet<String> = self
            .watching
            .iter()
            .filter(|(_, session, _)| {
                shell
                    .read(cx)
                    .session_view(*session)
                    .is_some_and(|view| view.read(cx).needs_approval())
            })
            .map(|(state, _, _)| state.clone())
            .collect();
        self.graph
            .update(cx, |graph, cx| graph.set_waiting(waiting, cx));
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
        self.picked = true;
        self.reload(cx);
    }

    /// Shows `run` of the shown Flow; `None` is its blueprint.
    pub(super) fn select_run(&mut self, run: Option<String>, cx: &mut Context<Self>) {
        if self.run != run {
            self.run = run;
            self.drawer = None;
        }
        self.picked = true;
        self.reload(cx);
    }

    /// The live version's number: the newest saved one.
    pub(super) fn live_version(&self) -> Option<i64> {
        self.versions
            .iter()
            .filter(|v| !v.draft)
            .map(|v| v.version)
            .max()
    }

    /// The shown Flow's draft, if the Pilot drafted a change.
    pub(super) fn draft(&self) -> Option<&gespan_core::flow::FlowVersion> {
        self.versions.iter().find(|v| v.draft)
    }

    /// The version drawn instead of the live one, if one is picked.
    fn picked_version(&self) -> Option<&gespan_core::flow::FlowVersion> {
        let picked = self.version?;
        self.versions.iter().find(|v| v.version == picked)
    }

    /// The version shown: the picked one, else the live one.
    pub(super) fn shown_version(&self) -> Option<i64> {
        self.version.or_else(|| self.live_version())
    }

    /// Draws `version` of the shown Flow (`None`: the live one) as a
    /// blueprint: picking a version leaves the run shown.
    pub(super) fn select_version(&mut self, version: Option<i64>, cx: &mut Context<Self>) {
        let version = version.filter(|v| Some(*v) != self.live_version());
        self.diff = false;
        if self.version != version || self.run.is_some() {
            self.version = version;
            self.run = None;
            self.drawer = None;
        }
        self.picked = true;
        self.reload(cx);
    }

    /// Graph or YAML.
    pub(super) fn show_yaml(&mut self, yaml: bool, cx: &mut Context<Self>) {
        self.yaml = yaml;
        self.diff = false;
        cx.notify();
    }

    /// "See the change": the draft's YAML against the live version's, or
    /// back to the graph.
    pub(super) fn show_change(&mut self, show: bool, cx: &mut Context<Self>) {
        self.version = None;
        self.yaml = show;
        self.diff = show;
        cx.notify();
    }

    /// Saves the draft as the next version, live from now on.
    pub(super) fn save_draft(&mut self, cx: &mut Context<Self>) {
        self.draft_action(true, cx);
    }

    /// Drops the draft; the live version stays.
    pub(super) fn discard_draft(&mut self, cx: &mut Context<Self>) {
        self.draft_action(false, cx);
    }

    fn draft_action(&mut self, save: bool, cx: &mut Context<Self>) {
        let Some(flow) = self.flow.clone() else {
            return;
        };
        let owner = self.owner.clone();
        cx.spawn(async move |this, cx| {
            // Read here, as `reload` does: this may run inside a Shell update.
            let store = owner
                .update(cx, |shell, _| shell.pilot_flows_store())
                .unwrap_or_else(|_| Err(NOT_READY.into()));
            let result = match store {
                Ok(store) => {
                    cx.background_executor()
                        .spawn(async move {
                            let store = store
                                .lock()
                                .map_err(|_| "Flows are unavailable".to_string())?;
                            if save {
                                store.publish_draft(&flow).map(|_| ())
                            } else {
                                store.discard_draft(&flow)
                            }
                        })
                        .await
                }
                Err(error) => Err(error),
            };
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.version = None;
                        this.diff = false;
                        this.error = None;
                    }
                    Err(error) => this.error = Some(error),
                }
                this.reload(cx);
            });
        })
        .detach();
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
            Picker::Version => self
                .versions
                .iter()
                .map(|v| {
                    let live = Some(v.version) == self.live_version();
                    let label = match (v.draft, live) {
                        (true, _) => format!("v{} · draft", v.version),
                        (_, true) => format!("v{} · live", v.version),
                        _ => format!("v{}", v.version),
                    };
                    MenuAction::new(format!("version:{}", v.version), label)
                        .checked(self.shown_version() == Some(v.version))
                        .into()
                })
                .collect(),
        };
        let label = match which {
            Picker::Flow => "Pipeline",
            Picker::Version => "Version",
        };
        let menu = cx.new(|cx| Menu::new(label, items, window, cx).width(280.));
        let events = cx.subscribe(&menu, |this, _, event: &MenuEvent, cx| {
            this.picker = None;
            if let MenuEvent::Picked(id, _) = event {
                if let Some(flow) = id.strip_prefix("flow:") {
                    this.select_flow(flow.to_owned(), cx);
                } else if let Some(version) = id.strip_prefix("version:") {
                    this.select_version(version.parse().ok(), cx);
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
            Picker::Version => ("pilot-roadmap-version-picker", &self.version_anchor),
        };
        // The board's `.select` chips: their value and a caret.
        let chip = true;
        let open = self.picker.as_ref().is_some_and(|(p, _, _)| *p == which);
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Button)
            .aria_label(SharedString::from(format!("{caption}: {value}")))
            .relative()
            .when(chip, |d| d.flex_shrink(1.).max_w(px(180.)))
            .when(!chip, |d| d.flex_1())
            .min_w_0()
            .h(px(28.))
            .px(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(crate::glass::edge())
            .bg(crate::glass::fill_strong())
            .when(open, |d| d.bg(content(0.1)))
            .hover(|s| s.bg(content(0.08)))
            .cursor_pointer()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.5))
            .child(anchor.probe())
            .when(!chip, |d| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .text_color(content(0.45))
                        .child(caption),
                )
            })
            .child(
                div()
                    .when(!chip, |d| d.flex_1())
                    .min_w_0()
                    .truncate()
                    .text_color(content(0.92))
                    .when(which == Picker::Flow, |d| {
                        d.font_weight(gpui::FontWeight::SEMIBOLD)
                    })
                    // The board's caret is part of the label.
                    .child(format!("{value} ▾")),
            )
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
            FlowState::Parallel(s) => (
                s.title.clone(),
                format!("Runs side by side: {}.", s.branches.join(", ")),
                Vec::new(),
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
    state.title()
}

/// When a run started or ended, as the board writes it: "Nov 12".
fn day(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map_or_else(String::new, |t| t.format("%b %-d").to_string())
}

impl Roadmap {
    /// The run shown is `run`; its version is the saved one whose
    /// definition its snapshot started from, if any.
    fn run_version(&self, run: &FlowRun) -> Option<i64> {
        let same = |a: &Flow, b: &Flow| {
            let mut a = a.clone();
            a.created_at = b.created_at;
            a.updated_at = b.updated_at;
            a == *b
        };
        self.versions
            .iter()
            .filter(|v| !v.draft)
            .find(|v| same(&v.flow, &run.flow))
            .map(|v| v.version)
    }

    /// "This run · <task> · v<n>", how far it is and how long it has taken.
    fn run_card(&self, cx: &App) -> Option<AnyElement> {
        let run_id = self.run.as_deref()?;
        let run = self.runs.iter().find(|r| r.id == run_id)?;
        let rows = self.layout.as_ref().map(|(_, rows)| rows.as_slice())?;
        let newest = self.runs.first().is_some_and(|r| r.id == run.id);
        let name = run
            .tag
            .as_ref()
            .map_or(run.flow.name.as_str(), |t| t.task.as_str());
        let mut label = format!("{} · {name}", if newest { "This run" } else { "Run" });
        if let Some(version) = self.run_version(run) {
            label.push_str(&format!(" · v{version}"));
        }
        let step = layout::current_row(rows, &self.visits).map_or(0, |i| i + 1);
        let waiting = self.graph.read(cx).any_waiting();
        let (status, tone) = match run.status {
            RunStatus::Succeeded => ("Done".to_string(), StatusTone::Ok),
            RunStatus::Failed => ("Failed".to_string(), StatusTone::Bad),
            RunStatus::Cancelled => ("Stopped".to_string(), StatusTone::Muted),
            _ if waiting => (format!("Step {step} of {}", rows.len()), StatusTone::Wait),
            _ => (format!("Step {step} of {}", rows.len()), StatusTone::Run),
        };
        let end = run
            .finished_at
            .unwrap_or_else(|| gespan_core::time::now_ms() as i64);
        let took = crate::shell::running_elapsed((end - run.created_at).max(0) as u64);
        Some(
            crate::glass::card()
                .debug_selector(|| "pilot-roadmap-run-card".into())
                .flex_shrink_0()
                .mb(px(12.))
                .px(px(10.))
                .py(px(8.))
                .rounded(px(10.))
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(12.))
                        .line_height(px(17.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(content(0.92))
                        .child(label),
                )
                .child(status_chip(status, tone))
                .child(
                    div()
                        .flex_shrink_0()
                        .text_size(px(11.5))
                        .line_height(px(16.))
                        .text_color(faint_ink())
                        .child(took),
                )
                .into_any_element(),
        )
    }

    /// "Earlier runs": every other run of the shown Flow, newest first;
    /// picking one shows it.
    fn earlier_runs(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let others: Vec<&FlowRun> = self
            .runs
            .iter()
            .filter(|r| Some(r.id.as_str()) != self.run.as_deref())
            .collect();
        if others.is_empty() {
            return None;
        }
        Some(
            div()
                .debug_selector(|| "pilot-roadmap-earlier".into())
                .flex()
                .flex_col()
                .child(
                    div()
                        .px(px(4.))
                        .pt(px(14.))
                        .pb(px(6.))
                        .text_size(px(11.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(faint_ink())
                        .child("EARLIER RUNS"),
                )
                .child(div().px(px(4.)).flex().flex_col().gap(px(2.)).children(
                    others.into_iter().map(|run| {
                        let id = run.id.clone();
                        let name = run
                            .tag
                            .as_ref()
                            .map_or_else(|| run_label(run), |t| t.task.clone());
                        let end = run
                            .finished_at
                            .unwrap_or_else(|| gespan_core::time::now_ms() as i64);
                        let when = format!(
                            "{} · {}",
                            day(run.created_at),
                            crate::shell::running_elapsed((end - run.created_at).max(0) as u64)
                        );
                        let dot = match run.status {
                            RunStatus::Succeeded => rgb(0x6ee7b7).into(),
                            RunStatus::Failed => rgb(0xfda4af).into(),
                            RunStatus::Running | RunStatus::WaitingApproval => rgb(0x459bf7).into(),
                            _ => content(0.22),
                        };
                        let selector = format!("pilot-roadmap-earlier-{id}");
                        div()
                            .id(SharedString::from(selector.clone()))
                            .debug_selector(move || selector.clone())
                            .role(Role::Button)
                            .aria_label(SharedString::from(format!("{name}, {when}")))
                            .cursor_pointer()
                            .rounded(px(6.))
                            .hover(|s| s.bg(content(0.05)))
                            .flex()
                            .gap(px(8.))
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .text_color(content(0.64))
                            .child(
                                div()
                                    .mt(px(5.))
                                    .size(px(7.))
                                    .rounded_full()
                                    .flex_shrink_0()
                                    .bg(dot),
                            )
                            .child(div().min_w_0().truncate().child(name))
                            .child(
                                div()
                                    .ml_auto()
                                    .flex_shrink_0()
                                    .text_size(px(11.5))
                                    .line_height(px(16.))
                                    .text_color(faint_ink())
                                    .child(when),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.version = None;
                                this.select_run(Some(id.clone()), cx);
                            }))
                    }),
                ))
                .into_any_element(),
        )
    }
}

#[derive(Clone, Copy)]
enum StatusTone {
    Ok,
    Bad,
    Run,
    Wait,
    Muted,
}

/// The board's `.status` chip: a dot and a word on a tint of its colour.
fn status_chip(label: String, tone: StatusTone) -> AnyElement {
    let (ink, base): (Hsla, Hsla) = match tone {
        StatusTone::Ok => (rgb(0x6ee7b7).into(), rgb(0x6ee7b7).into()),
        StatusTone::Bad => (rgb(0xfda4af).into(), rgb(0xfda4af).into()),
        StatusTone::Run => (rgb(0x8cc4ff).into(), rgb(0x459bf7).into()),
        StatusTone::Wait => (rgb(0xf5dc84).into(), rgb(0xe8c547).into()),
        StatusTone::Muted => (content(0.62), content(0.4)),
    };
    div()
        .debug_selector(|| "pilot-roadmap-run-status".into())
        .flex_none()
        .h(px(22.))
        .pl(px(8.))
        .pr(px(9.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded_full()
        .bg(base.opacity(0.10))
        .border_1()
        .border_color(base.opacity(0.22))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(ink)
        .child(div().size(px(6.)).rounded_full().bg(ink))
        .child(label)
        .into_any_element()
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
        let version_name = match (self.shown_version(), self.picked_version()) {
            (_, Some(v)) if v.draft => format!("v{} draft", v.version),
            (Some(v), _) => format!("v{v}"),
            (None, _) => "v1".into(),
        };
        let drawer = self.drawer.clone().and_then(|id| self.drawer_view(&id, cx));
        let popover = self.picker.as_ref().map(|(which, menu, _)| {
            let anchor = match which {
                Picker::Flow => self.flow_anchor.clone(),
                Picker::Version => self.version_anchor.clone(),
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
                // The board's row: pipeline, version, then Graph / YAML.
                .child(
                    div()
                        .debug_selector(|| "pilot-roadmap-header".into())
                        .flex_shrink_0()
                        .px(px(16.))
                        .pt(px(14.))
                        .pb(px(6.))
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(self.picker_button(Picker::Flow, "Pipeline", flow_name, cx))
                        .child(self.picker_button(Picker::Version, "Version", version_name, cx))
                        .child(div().flex_1())
                        .child(self.view_switch(cx)),
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
                .map(|d| {
                    if self.yaml {
                        d.child(self.yaml_view(cx))
                    } else {
                        // The run card, the graph and the earlier runs
                        // scroll together under the pickers.
                        let run_card = self.run_card(cx);
                        let earlier = self.earlier_runs(cx);
                        d.child(
                            div()
                                .id("pilot-roadmap-canvas")
                                .relative()
                                .flex_1()
                                .min_h_0()
                                .min_w_0()
                                .child(
                                    div()
                                        .id("pilot-roadmap-scroll")
                                        .size_full()
                                        .overflow_y_scroll()
                                        .track_scroll(&self.scroll)
                                        .px(px(12.))
                                        .pb(px(12.))
                                        .flex()
                                        .flex_col()
                                        .children(run_card)
                                        .when(self.drawn.is_some(), |d| d.child(self.graph.clone()))
                                        .children(earlier),
                                )
                                .children(drawer),
                        )
                    }
                })
                .children(popover)
                .into_any_element(),
        )
    }
}

impl Roadmap {
    /// Graph / YAML, as a small segmented control.
    fn view_switch(&self, cx: &mut Context<Self>) -> AnyElement {
        crate::glass::segmented()
            .id("pilot-roadmap-view")
            .debug_selector(|| "pilot-roadmap-view".into())
            .role(Role::TabList)
            .aria_label("Pipeline view")
            .flex_shrink_0()
            .children([("Graph", false), ("YAML", true)].map(|(label, yaml)| {
                crate::glass::segment(
                    SharedString::from(format!("pilot-roadmap-{label}")),
                    label,
                    self.yaml == yaml,
                )
                .debug_selector(move || format!("pilot-roadmap-{label}"))
                .role(Role::Tab)
                .aria_label(label)
                .aria_selected(self.yaml == yaml)
                .h(px(24.))
                .px(px(9.))
                .text_size(px(12.))
                .on_click(cx.listener(move |this, _, _, cx| this.show_yaml(yaml, cx)))
            }))
            .into_any_element()
    }

    /// The draft's YAML against the live version's: removed lines red,
    /// added lines green, the rest as they are.
    fn diff_view(&self) -> Option<AnyElement> {
        use gespan_core::flow::DiffLine;
        let draft = self.draft()?;
        let live = self.saved_flow()?;
        let from = self.live_version().unwrap_or(1);
        let lines = gespan_core::flow::yaml_diff(
            &gespan_core::flow::flow_yaml(live, from),
            &gespan_core::flow::flow_yaml(&draft.flow, draft.version),
        );
        let changed = lines
            .iter()
            .filter(|l| !matches!(l, DiffLine::Same(_)))
            .count();
        Some(
            div()
                .id("pilot-roadmap-diff")
                .debug_selector(|| "pilot-roadmap-diff".into())
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(16.))
                .pb(px(12.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .debug_selector(|| "pilot-roadmap-diff-caption".into())
                        .text_size(px(11.5))
                        .text_color(content(0.54))
                        .child(format!(
                            "v{from} → v{} draft · {changed} {} changed",
                            draft.version,
                            if changed == 1 { "line" } else { "lines" }
                        )),
                )
                .child(
                    div()
                        .py(px(10.))
                        .rounded(px(10.))
                        .border_1()
                        .border_color(crate::glass::edge())
                        .bg(if crate::theme::is_light() {
                            content(0.04)
                        } else {
                            crate::theme::black(0.35)
                        })
                        .font_family(crate::fonts::monospace())
                        .text_size(px(12.))
                        .line_height(px(19.))
                        .children(lines.into_iter().enumerate().map(|(ix, line)| {
                            let (sign, text, added) = match line {
                                DiffLine::Same(t) => (" ", t, None),
                                DiffLine::Removed(t) => ("-", t, Some(false)),
                                DiffLine::Added(t) => ("+", t, Some(true)),
                            };
                            div()
                                .debug_selector(move || match added {
                                    Some(true) => format!("pilot-roadmap-diff-added-{ix}"),
                                    Some(false) => format!("pilot-roadmap-diff-removed-{ix}"),
                                    None => format!("pilot-roadmap-diff-line-{ix}"),
                                })
                                .px(px(12.))
                                .flex()
                                .text_color(match added {
                                    Some(add) => crate::theme::diff_foreground(add),
                                    None => content(0.7),
                                })
                                .when_some(added, |d, add| {
                                    d.bg(crate::theme::diff_background(add, false))
                                })
                                .child(div().w(px(14.)).flex_shrink_0().child(sign))
                                // Long lines wrap, so a change at the end of
                                // one is never cut off.
                                .child(div().flex_1().min_w_0().child(if text.is_empty() {
                                    " ".to_owned()
                                } else {
                                    text
                                }))
                        })),
                )
                .into_any_element(),
        )
    }

    /// The shown version as YAML, read-only, with Copy.
    fn yaml_view(&self, cx: &mut Context<Self>) -> AnyElement {
        if self.diff
            && let Some(diff) = self.diff_view()
        {
            return diff;
        }
        let Some(flow) = self.drawn.as_ref() else {
            return div().flex_1().into_any_element();
        };
        let version = self.shown_version().unwrap_or(1);
        let text = gespan_core::flow::flow_yaml(flow, version);
        let copy = text.clone();
        let slug: String = flow
            .name
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let draft = self.picked_version().is_some_and(|v| v.draft);
        div()
            .id("pilot-roadmap-yaml")
            .debug_selector(|| "pilot-roadmap-yaml".into())
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(16.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.5))
                            .text_color(content(0.54))
                            .child(format!(
                                "flows/{slug}.yaml · v{version}{}",
                                if draft { " draft" } else { " · read-only" }
                            )),
                    )
                    .child(
                        crate::glass::button(
                            "pilot-roadmap-copy",
                            "Copy",
                            crate::glass::Button::Ghost,
                        )
                        .debug_selector(|| "pilot-roadmap-copy".into())
                        .role(Role::Button)
                        .aria_label("Copy YAML")
                        .h(px(24.))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.write_to_clipboard(gpui::ClipboardItem::new_string(copy.clone()))
                        })),
                    ),
            )
            .child(
                div()
                    .debug_selector(|| "pilot-roadmap-yaml-text".into())
                    .px(px(12.))
                    .py(px(10.))
                    .rounded(px(10.))
                    .border_1()
                    .border_color(crate::glass::edge())
                    .bg(if crate::theme::is_light() {
                        content(0.04)
                    } else {
                        crate::theme::black(0.35)
                    })
                    .font_family(crate::fonts::monospace())
                    .text_size(px(12.))
                    .line_height(px(19.))
                    .text_color(content(0.86))
                    .children(text.lines().map(|line| {
                        div().child(if line.is_empty() {
                            " ".to_owned()
                        } else {
                            line.to_owned()
                        })
                    })),
            )
            .into_any_element()
    }

    /// The Pilot drafted a change: what it is, and saving or dropping it.
    fn draft_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let draft = self.draft()?;
        let live = self.live_version().unwrap_or(1);
        let next = draft.version;
        let showing = self.diff;
        Some(
            crate::glass::card()
                .debug_selector(|| "pilot-roadmap-draft".into())
                .flex_shrink_0()
                .mx(px(12.))
                .mb(px(8.))
                .px(px(10.))
                .py(px(8.))
                .rounded(px(10.))
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(content(0.86))
                        .child(format!(
                            "Drafted v{next}. v{live} stays live until you save."
                        )),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(6.))
                        .child(
                            crate::glass::button(
                                "pilot-roadmap-draft-see",
                                if showing {
                                    "Hide the change"
                                } else {
                                    "See the change"
                                },
                                crate::glass::Button::Glass,
                            )
                            .debug_selector(|| "pilot-roadmap-draft-see".into())
                            .role(Role::Button)
                            .aria_label(if showing {
                                "Hide the change"
                            } else {
                                "See the change"
                            })
                            .h(px(24.))
                            .text_size(px(11.5))
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.show_change(!showing, cx)),
                            ),
                        )
                        .child(
                            crate::glass::button(
                                "pilot-roadmap-draft-save",
                                format!("Save as v{next}"),
                                crate::glass::Button::Primary,
                            )
                            .debug_selector(|| "pilot-roadmap-draft-save".into())
                            .role(Role::Button)
                            .aria_label(format!("Save as v{next}"))
                            .h(px(24.))
                            .text_size(px(11.5))
                            .on_click(cx.listener(|this, _, _, cx| this.save_draft(cx))),
                        )
                        .child(
                            crate::glass::button(
                                "pilot-roadmap-draft-discard",
                                "Discard",
                                crate::glass::Button::Ghost,
                            )
                            .debug_selector(|| "pilot-roadmap-draft-discard".into())
                            .role(Role::Button)
                            .aria_label("Discard draft")
                            .h(px(24.))
                            .text_size(px(11.5))
                            .on_click(cx.listener(|this, _, _, cx| this.discard_draft(cx))),
                        ),
                )
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
            .children(self.draft_card(cx))
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
