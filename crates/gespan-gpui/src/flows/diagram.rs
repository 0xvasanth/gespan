//! A Flow painted as a node-and-edge diagram, with pan, zoom, selection and
//! keyboard focus. `gespan_core::flow::diagram` owns the layout and the run
//! overlay; this view only draws them and turns input into a viewport and a
//! selection.
//!
//! Coordinates: the core lays out in flow space (pixels at zoom 1). A
//! [`Viewport`] maps flow space to screen space relative to the canvas'
//! top-left corner; GPUI has no canvas transform, so every point and size is
//! scaled by hand when painting.
use super::ui::{button, status_color};
use crate::theme::*;
use gespan_core::flow::FAILED;
use gespan_core::flow::diagram::{
    Diagram, DiagramEdge, DiagramNode, NodeKind, NodeStatus, Overlay, START_ID, label_size,
    with_implicit_fail,
};
use gpui::{
    Bounds, ContentMask, Context, CursorStyle, EventEmitter, FocusHandle, Focusable, Hsla,
    MouseButton, PathBuilder, Pixels, Point, Role, Size, Window, canvas, div, point, prelude::*,
    px, size,
};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::{Duration, Instant};

pub(crate) const MIN_ZOOM: f32 = 0.4;
pub(crate) const MAX_ZOOM: f32 = 2.0;
/// Space kept around the content when fitting it.
const FIT_PADDING: f32 = 32.;
/// How close (in screen pixels) a click must be to an edge to hit it.
const EDGE_TOLERANCE: f32 = 8.;
/// Points sampled along each bezier for hit testing.
const CURVE_SAMPLES: usize = 24;
/// Dash and gap length of dashed edges, in screen pixels.
const DASH: f32 = 6.;
/// How long the "changed" badge stays on a state.
const CHANGED_FOR: Duration = Duration::from_secs(10);
const KEY_ZOOM_STEP: f32 = 1.25;
/// Pointer travel (screen pixels) that turns a press into a drag.
const DRAG_SLOP: f32 = 3.;
/// Margin kept between an auto-followed node and the edge of the area.
const FOLLOW_MARGIN: f32 = 24.;
const PULSE_PERIOD: Duration = Duration::from_millis(1600);

const NODE_RADIUS: f32 = 8.;
const NODE_PAD: f32 = 12.;
const TITLE_SIZE: f32 = 13.;
const TITLE_LINE: f32 = 17.;
const TITLE_LINES: usize = 2;
const LABEL_SIZE: f32 = 11.;
const BADGE_SIZE: f32 = 10.;
const ARROW: f32 = 8.;

/// Maps flow space to screen space: `screen = flow * zoom + (x, y)`, with the
/// screen origin at the canvas' top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Viewport {
    pub x: f32,
    pub y: f32,
    pub zoom: f32,
}

impl Viewport {
    /// Shows all of `content` inside `area` less `padding`, never zooming in
    /// past 1x and never out past the minimum. An axis that fits is centred;
    /// one that still overflows at the minimum zoom is pinned to the padding,
    /// so the top (Start) and left stay in view.
    pub fn fit(content: Size<f32>, area: Size<f32>, padding: f32) -> Self {
        let room_w = (area.width - 2. * padding).max(1.);
        let room_h = (area.height - 2. * padding).max(1.);
        let zoom = (room_w / content.width.max(1.))
            .min(room_h / content.height.max(1.))
            .min(1.)
            .clamp(MIN_ZOOM, MAX_ZOOM);
        let place = |area: f32, content: f32| {
            let scaled = content * zoom;
            if scaled <= area - 2. * padding {
                (area - scaled) / 2.
            } else {
                padding
            }
        };
        Self {
            x: place(area.width, content.width),
            y: place(area.height, content.height),
            zoom,
        }
    }

    /// Multiplies the zoom by `factor` (clamped to 0.4..=2.0) keeping the flow
    /// point under `pointer` (screen space) where it is.
    pub fn zoom_at(&mut self, factor: f32, pointer: Point<f32>) {
        let zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let ratio = zoom / self.zoom;
        self.x = pointer.x - ratio * (pointer.x - self.x);
        self.y = pointer.y - ratio * (pointer.y - self.y);
        self.zoom = zoom;
    }

    pub fn to_screen(self, p: (f32, f32)) -> Point<f32> {
        point(p.0 * self.zoom + self.x, p.1 * self.zoom + self.y)
    }

    pub fn to_flow(self, p: Point<f32>) -> (f32, f32) {
        ((p.x - self.x) / self.zoom, (p.y - self.y) / self.zoom)
    }

    /// Pans the least needed to bring the flow rect `(x, y, w, h)` inside
    /// `area` less `margin`; centres it when it is larger than that.
    fn reveal(&mut self, rect: (f32, f32, f32, f32), area: Size<f32>, margin: f32) {
        let (x, y, w, h) = rect;
        let axis = |offset: &mut f32, start: f32, len: f32, area: f32| {
            let lo = start * self.zoom + *offset;
            let hi = lo + len * self.zoom;
            if hi - lo > area - 2. * margin {
                *offset += area / 2. - (lo + hi) / 2.;
            } else if lo < margin {
                *offset += margin - lo;
            } else if hi > area - margin {
                *offset -= hi - (area - margin);
            }
        };
        let mut ox = self.x;
        let mut oy = self.y;
        axis(&mut ox, x, w, area.width);
        axis(&mut oy, y, h, area.height);
        self.x = ox;
        self.y = oy;
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Hit {
    Node(String),
    /// An index into `Diagram::edges`.
    Edge(usize),
}

/// The bezier `[p0, c1, c2, p1]` at `t`.
fn cubic_point(p: &[(f32, f32)], t: f32) -> (f32, f32) {
    let u = 1. - t;
    let (a, b, c, d) = (u * u * u, 3. * u * u * t, 3. * u * t * t, t * t * t);
    (
        a * p[0].0 + b * p[1].0 + c * p[2].0 + d * p[3].0,
        a * p[0].1 + b * p[1].1 + c * p[2].1 + d * p[3].1,
    )
}

/// The polyline an edge is drawn (or hit) along: a forward edge's bezier
/// sampled at 24 points, or a back-edge's vertices.
fn edge_polyline(edge: &DiagramEdge) -> Vec<(f32, f32)> {
    if is_curve(edge) {
        (0..=CURVE_SAMPLES)
            .map(|i| cubic_point(&edge.points, i as f32 / CURVE_SAMPLES as f32))
            .collect()
    } else {
        edge.points.clone()
    }
}

fn is_curve(edge: &DiagramEdge) -> bool {
    !edge.back && !edge.failure && edge.points.len() == 4
}

fn segment_distance(p: (f32, f32), a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx * dx + dy * dy;
    let t = if len == 0. {
        0.
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len).clamp(0., 1.)
    };
    let (cx, cy) = (a.0 + t * dx, a.1 + t * dy);
    ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt()
}

/// What is under `flow_point`. Nodes win, then edge labels (their estimated
/// box around `label_at`; the last painted on top), then the nearest edge
/// line within `tolerance` (flow units: pass 8px / zoom). On a tie between
/// lines a labelled edge wins, so a labelled edge that shares its route with
/// an unlabelled one (two failure exits from one state) stays selectable;
/// otherwise the first in edge order.
pub(crate) fn hit_test(d: &Diagram, flow_point: (f32, f32), tolerance: f32) -> Option<Hit> {
    let (x, y) = flow_point;
    if let Some(n) = d
        .nodes
        .iter()
        .rev()
        .find(|n| x >= n.x && x <= n.x + n.w && y >= n.y && y <= n.y + n.h)
    {
        return Some(Hit::Node(n.id.clone()));
    }
    if let Some(i) = d.edges.iter().rposition(|e| {
        e.label.as_deref().is_some_and(|label| {
            let (w, h) = label_size(label);
            let (cx, cy) = e.label_at;
            (x - cx).abs() <= w / 2. && (y - cy).abs() <= h / 2.
        })
    }) {
        return Some(Hit::Edge(i));
    }
    d.edges
        .iter()
        .enumerate()
        .filter_map(|(i, e)| {
            let line = edge_polyline(e);
            let near = line
                .windows(2)
                .map(|w| segment_distance(flow_point, w[0], w[1]))
                .fold(f32::INFINITY, f32::min);
            (near <= tolerance).then_some((near, e.label.is_none(), i))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)))
        .map(|(.., i)| Hit::Edge(i))
}

/// Back-edges and failure exits (declared or implicit) are dashed: they are
/// "maybe" paths. The Start arrow (also implicit) is solid.
fn dashed(edge: &DiagramEdge) -> bool {
    edge.back || edge.failure || (edge.implicit && edge.to == FAILED)
}

/// The failed status colour, muted for the failure lane.
fn failure_tone() -> Hsla {
    let mut tone = node_color(NodeStatus::Failed);
    tone.a = 0.55;
    tone
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct EdgeStyle {
    color: Hsla,
    width: f32,
    dashed: bool,
}

/// How an edge is stroked. Taken or selected edges are highlighted in the
/// accent colour; otherwise failure exits use the muted failed tone and the
/// rest a neutral one.
fn edge_style(edge: &DiagramEdge, taken: bool, selected: bool) -> EdgeStyle {
    let lit = taken || selected;
    EdgeStyle {
        color: if lit {
            accent_color()
        } else if edge.failure {
            failure_tone()
        } else {
            content(0.28)
        },
        width: if lit { 2. } else { 1.5 },
        dashed: dashed(edge),
    }
}

/// The shared failure lane, from where the topmost failure exit reaches it
/// down to the Failed pill's top. Painted once so the dashes of the exits
/// that share it do not interleave. `None` without failure exits.
fn failure_lane(d: &Diagram) -> Option<((f32, f32), (f32, f32))> {
    let mut lane: Option<((f32, f32), (f32, f32))> = None;
    for e in d.edges.iter().filter(|e| e.failure) {
        let [.., join, end] = e.points.as_slice() else {
            continue;
        };
        lane = Some(match lane {
            Some((top, _)) if top.1 <= join.1 => (top, *end),
            _ => (*join, *end),
        });
    }
    lane
}

/// Whether each of `d.edges` was taken by the run. The overlay names edges
/// by `(from, to, index among from's edges to the same target)`. The Start
/// arrow counts as taken once the initial state was visited, and the
/// implicit failure edge whenever it is drawn.
fn taken_edges(d: &Diagram, overlay: Option<&Overlay>) -> Vec<bool> {
    let Some(overlay) = overlay else {
        return vec![false; d.edges.len()];
    };
    let mut seen: HashMap<(&str, &str), usize> = HashMap::new();
    d.edges
        .iter()
        .map(|e| {
            let index = seen.entry((&e.from, &e.to)).or_default();
            let i = *index;
            *index += 1;
            if e.from == START_ID {
                overlay.nodes.get(&e.to).is_some_and(|(_, n)| *n > 0)
            } else if e.implicit {
                true
            } else {
                overlay.taken.contains(&(e.from.clone(), e.to.clone(), i))
            }
        })
        .collect()
}

/// `title` wrapped to at most two lines of `max_w`; the last line ends in
/// `…` when text is left over, and any line too wide (one long word) is cut.
fn wrap_title(title: &str, max_w: f32, mut measure: impl FnMut(&str) -> f32) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut overflow = false;
    for word in title.split_whitespace() {
        let candidate = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if current.is_empty() || measure(&candidate) <= max_w {
            current = candidate;
        } else if lines.len() + 1 == TITLE_LINES {
            overflow = true;
            current = candidate;
            break;
        } else {
            lines.push(std::mem::replace(&mut current, word.to_owned()));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    let last = lines.len().saturating_sub(1);
    for (i, line) in lines.iter_mut().enumerate() {
        if measure(line) > max_w || (i == last && overflow) {
            *line = ellipsize(line, max_w, &mut measure);
        }
    }
    lines
}

fn ellipsize(text: &str, max_w: f32, measure: &mut impl FnMut(&str) -> f32) -> String {
    let mut chars: Vec<char> = text.chars().collect();
    loop {
        let cut: String = chars.iter().collect::<String>().trim_end().to_owned() + "…";
        if chars.is_empty() || measure(&cut) <= max_w {
            return cut;
        }
        chars.pop();
    }
}

fn selectable(n: &DiagramNode) -> bool {
    matches!(n.kind, NodeKind::Agent | NodeKind::Approval)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Nav {
    /// Along the first outgoing edge (Tab, Down).
    Next,
    /// Back along the first incoming edge (Shift-Tab, Up).
    Previous,
    Left,
    Right,
}

/// Where keyboard focus goes from `from`. With nothing focused it lands on
/// the initial state. `None` means there is nowhere to go. Only states are
/// focusable; Start and the terminal pills are skipped.
fn step_focus(d: &Diagram, from: Option<&str>, nav: Nav) -> Option<String> {
    let node = |id: &str| d.nodes.iter().find(|n| n.id == id);
    let ok = |id: &str| node(id).is_some_and(selectable);
    let Some(from) = from.filter(|id| ok(id)) else {
        return d
            .edges
            .iter()
            .find(|e| e.from == START_ID && ok(&e.to))
            .map(|e| e.to.clone())
            .or_else(|| d.nodes.iter().find(|n| selectable(n)).map(|n| n.id.clone()));
    };
    match nav {
        Nav::Next => d
            .edges
            .iter()
            .find(|e| e.from == from && e.to != from && ok(&e.to))
            .map(|e| e.to.clone()),
        Nav::Previous => d
            .edges
            .iter()
            .find(|e| e.to == from && e.from != from && ok(&e.from))
            .map(|e| e.from.clone()),
        Nav::Left | Nav::Right => {
            let here = node(from)?;
            let mut row: Vec<&DiagramNode> = d
                .nodes
                .iter()
                .filter(|n| n.rank == here.rank && selectable(n))
                .collect();
            row.sort_by_key(|n| n.order);
            let at = row.iter().position(|n| n.id == from)?;
            let to = if nav == Nav::Left {
                at.checked_sub(1)?
            } else {
                at + 1
            };
            row.get(to).map(|n| n.id.clone())
        }
    }
}

fn status_text(status: NodeStatus) -> &'static str {
    match status {
        NodeStatus::NotReached => "not reached",
        NodeStatus::Running => "running",
        NodeStatus::AwaitingApproval => "awaiting approval",
        NodeStatus::Completed => "completed",
        NodeStatus::Failed => "failed",
        NodeStatus::Cancelled => "cancelled",
    }
}

/// The Flows status colour for a node (the tokens `status_chip` uses).
fn node_color(status: NodeStatus) -> Hsla {
    status_color(match status {
        NodeStatus::Running => "running",
        NodeStatus::AwaitingApproval => "waiting-approval",
        NodeStatus::Completed => "completed",
        NodeStatus::Failed => "failed",
        NodeStatus::NotReached | NodeStatus::Cancelled => "",
    })
}

/// A state's border: tone, width and style. Cancelled is dashed in a muted
/// tone so it does not pass for Not reached.
fn node_border(status: NodeStatus) -> (Hsla, f32, gpui::BorderStyle) {
    match status {
        NodeStatus::NotReached => (content(0.2), 1., gpui::BorderStyle::Solid),
        NodeStatus::Cancelled => (content(0.45), 1.5, gpui::BorderStyle::Dashed),
        other => (node_color(other), 1.5, gpui::BorderStyle::Solid),
    }
}

fn node_status(overlay: Option<&Overlay>, id: &str) -> (NodeStatus, u32) {
    overlay
        .and_then(|o| o.nodes.get(id).copied())
        .unwrap_or((NodeStatus::NotReached, 0))
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DiagramEvent {
    /// The selected state changed (`None`: cleared).
    Selected(Option<String>),
    /// Enter or a double-click on a state.
    Open(String),
}

struct Drag {
    pointer: Point<Pixels>,
    start: Viewport,
    moved: bool,
}

pub(crate) struct DiagramView {
    diagram: Rc<Diagram>,
    overlay: Option<Rc<Overlay>>,
    /// What the viewport was last fitted for: the layout hash, and whether
    /// the drawing has a Failed pill (an implicit one adds a rank).
    structure: Option<(u64, bool)>,
    /// When each state was marked changed, on the executor's clock.
    changed: HashMap<String, Instant>,
    viewport: Viewport,
    /// The canvas' window position and size, once painted.
    origin: Point<f32>,
    area: Option<Size<f32>>,
    /// A refit is owed but the area is not known yet.
    needs_fit: bool,
    /// Pan the current state into view on updates; off once the user pans.
    following: bool,
    selected: Option<String>,
    /// A clicked edge, which shows its full guard label.
    selected_edge: Option<usize>,
    drag: Option<Drag>,
    focus: FocusHandle,
}

impl EventEmitter<DiagramEvent> for DiagramView {}

impl Focusable for DiagramView {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl DiagramView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            diagram: Rc::new(Diagram {
                nodes: Vec::new(),
                edges: Vec::new(),
                width: 0.,
                height: 0.,
            }),
            overlay: None,
            structure: None,
            changed: HashMap::new(),
            viewport: Viewport {
                x: 0.,
                y: 0.,
                zoom: 1.,
            },
            origin: point(0., 0.),
            area: None,
            needs_fit: true,
            following: true,
            selected: None,
            selected_edge: None,
            drag: None,
            // A tracked handle is a Tab stop only when the handle says so;
            // the element's own `tab_stop` applies to handles it makes.
            focus: cx.focus_handle().tab_stop(true),
        }
    }

    /// Shows a freshly built `diagram` with an optional run `overlay`.
    /// `changed` names states newly changed by this update; they wear a
    /// badge for 10 s. The viewport is refitted only when `structure` (the
    /// Flow's `structure_key`) differs from the last call, or a Failed pill
    /// appears or goes; otherwise a new current state is panned into view,
    /// unless the user has panned.
    pub fn set(
        &mut self,
        mut diagram: Diagram,
        overlay: Option<Overlay>,
        changed: HashSet<String>,
        structure: u64,
        cx: &mut Context<Self>,
    ) {
        if let Some(o) = &overlay
            && o.implicit_fail
            && let Some(from) = o.current.as_deref()
        {
            with_implicit_fail(&mut diagram, from);
        }
        let current = |o: &Option<Rc<Overlay>>| o.as_ref().and_then(|o| o.current.clone());
        let before = current(&self.overlay);
        let structure = (structure, diagram.nodes.iter().any(|n| n.id == FAILED));
        let restructured = self.structure != Some(structure);
        self.diagram = Rc::new(diagram);
        self.overlay = overlay.map(Rc::new);
        self.structure = Some(structure);
        let moved_on = current(&self.overlay) != before;

        if !changed.is_empty() {
            let now = cx.background_executor().now();
            for id in changed {
                self.changed.insert(id, now);
            }
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(CHANGED_FOR).await;
                let _ = this.update(cx, |this, cx| this.prune_changed(cx));
            })
            .detach();
        }

        if self
            .selected_edge
            .is_some_and(|i| i >= self.diagram.edges.len())
            || restructured
        {
            self.selected_edge = None;
        }
        let gone = self.selected.as_deref().is_some_and(|id| {
            !self
                .diagram
                .nodes
                .iter()
                .any(|n| n.id == id && selectable(n))
        });
        if gone {
            self.select(None, cx);
        }

        if restructured {
            self.following = true;
            self.needs_fit = true;
            self.refit();
            self.follow();
        } else if moved_on && self.following {
            self.follow();
        }
        cx.notify();
    }

    /// Owners follow `DiagramEvent::Selected` instead.
    #[cfg(test)]
    pub fn selected(&self) -> Option<&str> {
        self.selected.as_deref()
    }

    /// The ids of the states drawn now, in diagram order.
    #[cfg(test)]
    pub fn state_ids(&self) -> Vec<String> {
        self.diagram
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Agent | NodeKind::Approval))
            .map(|n| n.id.clone())
            .collect()
    }

    /// States wearing the "changed" badge now, sorted.
    #[cfg(test)]
    pub fn badges(&self, cx: &gpui::App) -> Vec<String> {
        self.badged(cx)
    }

    /// Drops every badge, so a test can see whether an update adds any.
    #[cfg(test)]
    pub fn clear_badges_for_test(&mut self, cx: &mut Context<Self>) {
        self.changed.clear();
        cx.notify();
    }

    #[cfg(test)]
    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    /// The run's current state, from the overlay.
    #[cfg(test)]
    pub fn current(&self) -> Option<String> {
        self.overlay.as_ref().and_then(|o| o.current.clone())
    }

    /// A manual pan, as a drag or wheel makes.
    #[cfg(test)]
    pub fn pan_for_test(&mut self, dx: f32, dy: f32, cx: &mut Context<Self>) {
        self.pan_by(dx, dy);
        cx.notify();
    }

    fn prune_changed(&mut self, cx: &mut Context<Self>) {
        let now = cx.background_executor().now();
        let before = self.changed.len();
        self.changed
            .retain(|_, at| now.saturating_duration_since(*at) < CHANGED_FOR);
        if self.changed.len() != before {
            cx.notify();
        }
    }

    /// States wearing the "changed" badge now, sorted.
    fn badged(&self, cx: &gpui::App) -> Vec<String> {
        let now = cx.background_executor().now();
        let mut ids: Vec<String> = self
            .changed
            .iter()
            .filter(|(_, at)| now.saturating_duration_since(**at) < CHANGED_FOR)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// Records the canvas' place in the window; fits if a fit is owed.
    fn resize(&mut self, origin: Point<f32>, area: Size<f32>) {
        self.origin = origin;
        if self.area != Some(area) {
            self.area = Some(area);
            if self.needs_fit {
                self.refit();
                self.follow();
            }
        }
    }

    fn refit(&mut self) {
        if let Some(area) = self.area {
            let content = size(self.diagram.width, self.diagram.height);
            self.viewport = Viewport::fit(content, area, FIT_PADDING);
            self.needs_fit = false;
        }
    }

    /// Brings the current state into view (not a user pan).
    fn follow(&mut self) {
        let Some(area) = self.area else { return };
        let Some(current) = self.overlay.as_ref().and_then(|o| o.current.as_deref()) else {
            return;
        };
        if let Some(n) = self.diagram.nodes.iter().find(|n| n.id == current) {
            self.viewport
                .reveal((n.x, n.y, n.w, n.h), area, FOLLOW_MARGIN);
        }
    }

    /// The "Re-centre" button and the F key: fit, and follow again.
    fn recentre(&mut self, cx: &mut Context<Self>) {
        self.following = true;
        self.needs_fit = true;
        self.refit();
        self.follow();
        cx.notify();
    }

    /// A manual pan: following stops until the next re-centre.
    fn pan_by(&mut self, dx: f32, dy: f32) {
        self.viewport.x += dx;
        self.viewport.y += dy;
        self.following = false;
    }

    /// A manual zoom at a screen point: following stops too.
    fn zoom_at(&mut self, factor: f32, pointer: Point<f32>) {
        self.viewport.zoom_at(factor, pointer);
        self.following = false;
    }

    fn local(&self, p: Point<Pixels>) -> Point<f32> {
        point(p.x.as_f32() - self.origin.x, p.y.as_f32() - self.origin.y)
    }

    /// Selects a state (Start and the pills are ignored) and tells listeners.
    fn select(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        if let Some(id) = &id
            && !self
                .diagram
                .nodes
                .iter()
                .any(|n| n.id == *id && selectable(n))
        {
            return;
        }
        if id != self.selected {
            self.selected = id.clone();
            cx.emit(DiagramEvent::Selected(id));
            cx.notify();
        }
    }

    fn open_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(id) = self.selected.clone() {
            cx.emit(DiagramEvent::Open(id));
        }
    }

    /// Moves keyboard focus; returns false when there is nowhere to go.
    fn navigate(&mut self, nav: Nav, cx: &mut Context<Self>) -> bool {
        let Some(next) = step_focus(&self.diagram, self.selected.as_deref(), nav) else {
            return false;
        };
        if let (Some(area), Some(n)) = (self.area, self.diagram.nodes.iter().find(|n| n.id == next))
        {
            self.viewport
                .reveal((n.x, n.y, n.w, n.h), area, FOLLOW_MARGIN);
        }
        self.select(Some(next), cx);
        cx.notify();
        true
    }

    /// The accessible name: the focused state as "<title>, <status>".
    fn aria_label(&self) -> String {
        let node = self
            .selected
            .as_deref()
            .and_then(|id| self.diagram.nodes.iter().find(|n| n.id == id));
        match node {
            Some(n) => {
                let (status, _) = node_status(self.overlay.as_deref(), &n.id);
                format!("{}, {}", n.title, status_text(status))
            }
            None => {
                let steps = self.diagram.nodes.iter().filter(|n| selectable(n)).count();
                let noun = if steps == 1 { "step" } else { "steps" };
                format!("Flow diagram, {steps} {noun}")
            }
        }
    }

    fn click(&mut self, position: Point<Pixels>, count: usize, cx: &mut Context<Self>) {
        let at = self.viewport.to_flow(self.local(position));
        match hit_test(&self.diagram, at, EDGE_TOLERANCE / self.viewport.zoom) {
            Some(Hit::Node(id)) => {
                self.selected_edge = None;
                let state = self
                    .diagram
                    .nodes
                    .iter()
                    .any(|n| n.id == id && selectable(n));
                if state {
                    self.select(Some(id.clone()), cx);
                    if count >= 2 {
                        cx.emit(DiagramEvent::Open(id));
                    }
                }
            }
            Some(Hit::Edge(i)) => {
                self.selected_edge = (self.selected_edge != Some(i)).then_some(i);
            }
            None => {
                self.selected_edge = None;
                self.select(None, cx);
            }
        }
        cx.notify();
    }

    fn key(&mut self, event: &gpui::KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let keys = &event.keystroke;
        if keys.modifiers.platform || keys.modifiers.control || keys.modifiers.alt {
            return;
        }
        let moved = match keys.key.as_str() {
            "tab" if keys.modifiers.shift => self.navigate(Nav::Previous, cx),
            "tab" | "down" => self.navigate(Nav::Next, cx),
            "up" => self.navigate(Nav::Previous, cx),
            "left" => self.navigate(Nav::Left, cx),
            "right" => self.navigate(Nav::Right, cx),
            "enter" => {
                self.open_selected(cx);
                self.selected.is_some()
            }
            "f" => {
                self.recentre(cx);
                true
            }
            "+" | "=" => self.zoom_key(KEY_ZOOM_STEP, cx),
            "-" => self.zoom_key(1. / KEY_ZOOM_STEP, cx),
            "escape" if self.selected.is_some() || self.selected_edge.is_some() => {
                self.selected_edge = None;
                self.select(None, cx);
                cx.notify();
                true
            }
            _ => false,
        };
        // Arrow keys at a dead end are still ours; Tab at one moves on.
        if moved || matches!(keys.key.as_str(), "up" | "down" | "left" | "right") {
            cx.stop_propagation();
        }
    }

    fn zoom_key(&mut self, factor: f32, cx: &mut Context<Self>) -> bool {
        let area = self.area.unwrap_or(size(0., 0.));
        self.zoom_at(factor, point(area.width / 2., area.height / 2.));
        cx.notify();
        true
    }

    /// What the canvas needs to paint one frame.
    fn scene(&self, pulse: f32, cx: &Context<Self>) -> Scene {
        Scene {
            taken: taken_edges(&self.diagram, self.overlay.as_deref()),
            diagram: self.diagram.clone(),
            overlay: self.overlay.clone(),
            changed: self.badged(cx).into_iter().collect(),
            selected: self.selected.clone(),
            selected_edge: self.selected_edge,
            pulse,
        }
    }
}

struct Scene {
    diagram: Rc<Diagram>,
    overlay: Option<Rc<Overlay>>,
    taken: Vec<bool>,
    changed: HashSet<String>,
    selected: Option<String>,
    selected_edge: Option<usize>,
    /// Border alpha for Running states, 0..1.
    pulse: f32,
}

impl Render for DiagramView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self
            .overlay
            .as_ref()
            .is_some_and(|o| o.nodes.values().any(|(s, _)| *s == NodeStatus::Running));
        let pulse = if running && crate::pulse::request(crate::pulse::Rate::Smooth, window, cx) {
            let phase = crate::pulse::phase(PULSE_PERIOD, cx);
            0.6 + 0.4 * (phase * std::f32::consts::TAU).cos()
        } else {
            1.
        };
        let scene = self.scene(pulse, cx);
        let this = cx.entity();
        let edge_note = self
            .selected_edge
            .and_then(|i| self.diagram.edges.get(i))
            .and_then(|e| e.full_label.clone());
        div()
            .id("flow-diagram")
            .debug_selector(|| "flow-diagram".into())
            .track_focus(&self.focus)
            .key_context("FlowDiagram")
            .role(Role::Figure)
            .aria_label(self.aria_label())
            .relative()
            .size_full()
            .overflow_hidden()
            .cursor(if self.drag.as_ref().is_some_and(|d| d.moved) {
                CursorStyle::ClosedHand
            } else {
                CursorStyle::Arrow
            })
            .on_key_down(cx.listener(Self::key))
            .on_action(cx.listener(|this, _: &crate::shell::FocusNext, _, cx| {
                if !this.navigate(Nav::Next, cx) {
                    cx.propagate();
                }
            }))
            .on_action(cx.listener(|this, _: &crate::shell::FocusPrevious, _, cx| {
                if !this.navigate(Nav::Previous, cx) {
                    cx.propagate();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                    window.focus(&this.focus, cx);
                    this.drag = Some(Drag {
                        pointer: event.position,
                        start: this.viewport,
                        moved: false,
                    });
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                let Some(drag) = &mut this.drag else { return };
                if event.pressed_button != Some(MouseButton::Left) {
                    this.drag = None;
                    return;
                }
                let dx = (event.position.x - drag.pointer.x).as_f32();
                let dy = (event.position.y - drag.pointer.y).as_f32();
                if !drag.moved && dx.hypot(dy) < DRAG_SLOP {
                    return;
                }
                drag.moved = true;
                let start = drag.start;
                this.viewport = start;
                this.pan_by(dx, dy);
                cx.notify();
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, event: &gpui::MouseUpEvent, _, cx| {
                    if let Some(drag) = this.drag.take()
                        && !drag.moved
                    {
                        this.click(event.position, event.click_count, cx);
                    }
                    cx.notify();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    this.drag = None;
                    cx.notify();
                }),
            )
            .on_scroll_wheel(cx.listener(|this, event: &gpui::ScrollWheelEvent, _, cx| {
                let delta = event.delta.pixel_delta(px(16.));
                if event.modifiers.control || event.modifiers.platform {
                    // GPUI's wheel deltas have the opposite sign to DOM ones.
                    let pointer = this.local(event.position);
                    this.zoom_at((delta.y.as_f32() * 0.01).exp(), pointer);
                } else {
                    this.pan_by(delta.x.as_f32(), delta.y.as_f32());
                }
                cx.notify();
                cx.stop_propagation();
            }))
            .on_pinch(cx.listener(|this, event: &gpui::PinchEvent, _, cx| {
                let pointer = this.local(event.position);
                this.zoom_at(1. + event.delta, pointer);
                cx.notify();
                cx.stop_propagation();
            }))
            .child(
                canvas(
                    move |bounds, _, cx| {
                        this.update(cx, |this, _| {
                            this.resize(
                                point(bounds.origin.x.as_f32(), bounds.origin.y.as_f32()),
                                size(bounds.size.width.as_f32(), bounds.size.height.as_f32()),
                            );
                            this.viewport
                        })
                    },
                    move |bounds, viewport, window, cx| {
                        window.with_content_mask(Some(ContentMask { bounds }), |window| {
                            paint(&scene, bounds, viewport, window, cx)
                        })
                    },
                )
                .absolute()
                .size_full(),
            )
            .when_some(edge_note, |d, note| {
                d.child(
                    div()
                        .absolute()
                        .left(px(8.))
                        .bottom(px(8.))
                        .px(px(8.))
                        .py(px(4.))
                        .rounded(px(6.))
                        .bg(floating_fill())
                        .border_1()
                        .border_color(content(0.12))
                        .text_size(px(11.))
                        .text_color(content(0.8))
                        .child(note),
                )
            })
            .when(!self.following, |d| {
                d.child(
                    div().absolute().top(px(8.)).right(px(8.)).child(
                        button("flow-diagram-recentre", "Re-centre")
                            .bg(floating_fill())
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(|this, _, _, cx| this.recentre(cx))),
                    ),
                )
            })
    }
}

// ---- painting ------------------------------------------------------------

fn paint(
    scene: &Scene,
    bounds: Bounds<Pixels>,
    v: Viewport,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let origin = bounds.origin;
    let at = |p: (f32, f32)| {
        let s = v.to_screen(p);
        origin + point(px(s.x), px(s.y))
    };
    let d = &scene.diagram;
    let overlay = scene.overlay.as_deref();
    let stroke = |points: &[Point<Pixels>], curve: bool, style: EdgeStyle, window: &mut Window| {
        let Some((&first, rest)) = points.split_first() else {
            return;
        };
        let mut path = PathBuilder::stroke(px(style.width));
        if style.dashed {
            path = path.dash_array(&[px(DASH), px(DASH)]);
        }
        path.move_to(first);
        if curve {
            path.cubic_bezier_to(points[3], points[1], points[2]);
        } else {
            for &p in rest {
                path.line_to(p);
            }
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, style.color);
        }
        if let [.., before, tip] = points {
            arrowhead(*before, *tip, ARROW * v.zoom, style.color, window);
        }
    };

    // The shared failure lane, once, in the plain failure style.
    if let (Some((top, bottom)), Some(edge)) = (failure_lane(d), d.edges.iter().find(|e| e.failure))
    {
        stroke(
            &[at(top), at(bottom)],
            false,
            edge_style(edge, false, false),
            window,
        );
    }
    for (i, edge) in d.edges.iter().enumerate() {
        let taken = scene.taken.get(i).copied().unwrap_or(false);
        let selected = scene.selected_edge == Some(i);
        let style = edge_style(edge, taken, selected);
        let points: Vec<Point<Pixels>> = edge.points.iter().map(|&p| at(p)).collect();
        if edge.failure && !(taken || selected) {
            // Only the run up to the lane: the lane itself is drawn above.
            let mut stub = points;
            stub.pop();
            let Some((&first, rest)) = stub.split_first() else {
                continue;
            };
            let mut path = PathBuilder::stroke(px(style.width)).dash_array(&[px(DASH), px(DASH)]);
            path.move_to(first);
            for &p in rest {
                path.line_to(p);
            }
            if let Ok(path) = path.build() {
                window.paint_path(path, style.color);
            }
        } else {
            stroke(&points, is_curve(edge), style, window);
        }
    }

    for (i, edge) in d.edges.iter().enumerate() {
        let Some(label) = &edge.label else { continue };
        let taken = scene.taken.get(i).copied().unwrap_or(false);
        let selected = scene.selected_edge == Some(i);
        let font = LABEL_SIZE * v.zoom;
        let color = if taken { accent_color() } else { content(0.6) };
        let line = shape(label, font, color, window);
        let pad = 4. * v.zoom;
        let w = line.width().as_f32() + 2. * pad;
        let h = font * 1.4 + pad;
        let centre = at(edge.label_at);
        let rect = Bounds::new(centre - point(px(w / 2.), px(h / 2.)), size(px(w), px(h)));
        window.paint_quad(gpui::quad(
            rect,
            px(4. * v.zoom),
            floating_fill(),
            px(1.),
            if selected {
                accent_color()
            } else {
                content(0.1)
            },
            gpui::BorderStyle::Solid,
        ));
        let _ = line.paint(
            rect.origin + point(px(pad), px(pad / 2.)),
            px(font * 1.4),
            gpui::TextAlign::Left,
            None,
            window,
            cx,
        );
    }

    for n in &d.nodes {
        paint_node(scene, overlay, n, &at, v.zoom, window, cx);
    }
}

fn arrowhead(from: Point<Pixels>, tip: Point<Pixels>, len: f32, color: Hsla, window: &mut Window) {
    let (dx, dy) = ((tip.x - from.x).as_f32(), (tip.y - from.y).as_f32());
    let norm = dx.hypot(dy);
    let (ux, uy) = if norm > f32::EPSILON {
        (dx / norm, dy / norm)
    } else {
        (0., 1.)
    };
    let half = len / 2.;
    let base = (tip.x.as_f32() - ux * len, tip.y.as_f32() - uy * len);
    let mut path = PathBuilder::fill();
    path.move_to(tip);
    path.line_to(point(px(base.0 - uy * half), px(base.1 + ux * half)));
    path.line_to(point(px(base.0 + uy * half), px(base.1 - ux * half)));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

fn shape(text: &str, font_size: f32, color: Hsla, window: &mut Window) -> gpui::ShapedLine {
    let style = window.text_style();
    let run = gpui::TextRun {
        len: text.len(),
        font: style.font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    window
        .text_system()
        .shape_line(text.to_owned().into(), px(font_size), &[run], None)
}

/// A filled pill with `text` centred on `centre`.
fn badge(
    text: &str,
    centre: Point<Pixels>,
    zoom: f32,
    fill: Hsla,
    ink: Hsla,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let font = BADGE_SIZE * zoom;
    let line = shape(text, font, ink, window);
    let h = font * 1.6;
    let w = (line.width().as_f32() + font).max(h);
    let rect = Bounds::new(centre - point(px(w / 2.), px(h / 2.)), size(px(w), px(h)));
    window.paint_quad(gpui::fill(rect, fill).corner_radii(px(h / 2.)));
    let _ = line.paint(
        point(centre.x - line.width() / 2., centre.y - px(font * 1.4 / 2.)),
        px(font * 1.4),
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

fn paint_node(
    scene: &Scene,
    overlay: Option<&Overlay>,
    n: &DiagramNode,
    at: &impl Fn((f32, f32)) -> Point<Pixels>,
    zoom: f32,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let rect = Bounds::new(at((n.x, n.y)), size(px(n.w * zoom), px(n.h * zoom)));
    let (status, visits) = node_status(overlay, &n.id);
    match n.kind {
        NodeKind::Start => {
            window.paint_quad(gpui::quad(
                rect,
                rect.size.height / 2.,
                content(0.06),
                px(1.),
                content(0.25),
                gpui::BorderStyle::Solid,
            ));
            centred_text(&n.title, rect, zoom, content(0.7), window, cx);
        }
        NodeKind::Succeeded | NodeKind::Failed => {
            let lit = overlay.and_then(|o| o.ended_at.as_deref()) == Some(n.id.as_str());
            let tone = if n.kind == NodeKind::Succeeded {
                node_color(NodeStatus::Completed)
            } else {
                node_color(NodeStatus::Failed)
            };
            let (fill, border, ink) = if lit {
                let mut wash = tone;
                wash.a = 0.18;
                (wash, tone, tone)
            } else {
                (content(0.04), content(0.2), content(0.55))
            };
            window.paint_quad(gpui::quad(
                rect,
                rect.size.height / 2.,
                fill,
                px(if lit { 1.5 } else { 1. }),
                border,
                gpui::BorderStyle::Solid,
            ));
            centred_text(&n.title, rect, zoom, ink, window, cx);
        }
        NodeKind::Agent | NodeKind::Approval => {
            let selected = scene.selected.as_deref() == Some(n.id.as_str());
            if selected {
                let ring = rect.dilate(px(3.));
                window.paint_quad(
                    gpui::outline(ring, accent_color(), gpui::BorderStyle::Solid)
                        .corner_radii(px(NODE_RADIUS * zoom + 3.))
                        .border_widths(px(2.)),
                );
            }
            let (mut border, border_w, border_style) = node_border(status);
            if status == NodeStatus::Running {
                border.a *= scene.pulse;
            }
            let reached = status != NodeStatus::NotReached;
            window.paint_quad(gpui::quad(
                rect,
                px(NODE_RADIUS * zoom),
                floating_fill(),
                px(border_w),
                border,
                border_style,
            ));
            if n.kind == NodeKind::Approval {
                // Approvals carry a stripe in the "needs approval" colour.
                let stripe = Bounds::new(
                    rect.origin + point(px(0.), px(NODE_RADIUS * zoom)),
                    size(
                        px(3. * zoom),
                        rect.size.height - px(2. * NODE_RADIUS * zoom),
                    ),
                );
                window.paint_quad(gpui::fill(stripe, node_color(NodeStatus::AwaitingApproval)));
            }
            let ink = if status == NodeStatus::NotReached && overlay.is_some() {
                content(0.6)
            } else {
                content(0.9)
            };
            let max_w = n.w - 2. * NODE_PAD;
            let lines = wrap_title(&n.title, max_w, |s| {
                shape(s, TITLE_SIZE, ink, window).width().as_f32()
            });
            let block = lines.len() as f32 * TITLE_LINE;
            let top = n.y + (n.h - block) / 2.;
            for (i, text) in lines.iter().enumerate() {
                let line = shape(text, TITLE_SIZE * zoom, ink, window);
                let _ = line.paint(
                    at((n.x + NODE_PAD, top + i as f32 * TITLE_LINE)),
                    px(TITLE_LINE * zoom),
                    gpui::TextAlign::Left,
                    None,
                    window,
                    cx,
                );
            }
            if visits > 1 {
                let tone = if reached {
                    node_color(status)
                } else {
                    content(0.4)
                };
                badge(
                    &format!("×{visits}"),
                    at((n.x + n.w - 4., n.y)),
                    zoom,
                    tone,
                    white(1.),
                    window,
                    cx,
                );
            }
            if scene.changed.contains(&n.id) {
                badge(
                    "changed",
                    at((n.x + 32., n.y)),
                    zoom,
                    accent_color(),
                    white(1.),
                    window,
                    cx,
                );
            }
        }
    }
}

fn centred_text(
    text: &str,
    rect: Bounds<Pixels>,
    zoom: f32,
    ink: Hsla,
    window: &mut Window,
    cx: &mut gpui::App,
) {
    let font = LABEL_SIZE * zoom;
    let line = shape(text, font, ink, window);
    let _ = line.paint(
        point(
            rect.center().x - line.width() / 2.,
            rect.center().y - px(font * 1.4 / 2.),
        ),
        px(font * 1.4),
        gpui::TextAlign::Left,
        None,
        window,
        cx,
    );
}

#[cfg(test)]
mod tests;
