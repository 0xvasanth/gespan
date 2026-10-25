//! A Flow drawn as a node-and-edge diagram, laid out top to bottom.
//!
//! Pure and deterministic: the same `Flow` always gives the same coordinates,
//! in flow space (pixels at zoom 1, origin top-left). The widget that paints
//! it lives in `gespan-gpui`.
//!
//! The layout is layered:
//! 1. back-edges are found by a depth-first search from `initial`;
//! 2. ranks are the longest path from Start over the remaining edges;
//! 3. the order within a rank is refined by barycenter sweeps;
//! 4. each rank row is centred on the widest one;
//! 5. edges are routed so none crosses a node: adjacent-rank edges are
//!    curves, failure exits run down a shared lane on the left into the
//!    Failed pill at the bottom, and back-edges, self-loops and edges that
//!    skip ranks run in lanes on the right (shorter spans nearer the nodes);
//! 6. each label is nudged along its own path until it clears the nodes and
//!    the labels placed before it.
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

use super::contract::select_transition;
use super::runner::INTERRUPTED;
use super::{
    Flow, FlowRun, FlowState, FlowVisit, Guard, GuardOp, RunEdit, RunStatus, Transition,
    VisitResult, VisitStatus, FAILED, SUCCEEDED,
};

pub const NODE_W: f32 = 200.0;
pub const NODE_H: f32 = 56.0;
pub const RANK_GAP: f32 = 72.0;
pub const NODE_GAP: f32 = 32.0;
/// Width of the Start marker and the terminal pills; their height is `NODE_H / 2`.
pub const PILL_W: f32 = 120.0;
/// The Start marker's node id. The terminal pills reuse `$succeeded` / `$failed`.
pub const START_ID: &str = "$start";

/// Guard labels longer than this many characters are cut and end in `…`.
const LABEL_MAX: usize = 24;
/// Horizontal spacing between parallel forward edges.
const PARALLEL_STEP: f32 = 24.0;
/// Vertical stagger between the label anchors of parallel forward edges.
const PARALLEL_LABEL_STEP: f32 = 16.0;
/// Spacing between right-hand lanes (back-edges, self-loops and forward
/// edges that skip ranks), and from the nodes beside a lane to the first one.
const LANE_STEP: f32 = 32.0;
/// How far left of every node the failure lane runs, at least.
const FAIL_MARGIN: f32 = 24.0;
/// The failure lane's x. The Failed pill is centred under it, so this keeps
/// the pill's left edge at 0.
const FAIL_LANE_X: f32 = PILL_W / 2.0;
/// Room kept between a failure stub's label and the lane or the node.
const STUB_CLEAR: f32 = 8.0;
/// How near (vertically) a node must be to a sideways run to block it.
const EDGE_CLEAR: f32 = 2.0;
/// Fractions along an edge's path its label tries, after any preferred
/// anchor: the middle, then 0.35/0.65, 0.25/0.75, then outwards in 0.05s.
const LABEL_STOPS: [f32; 19] = [
    0.5, 0.35, 0.65, 0.25, 0.75, 0.45, 0.55, 0.4, 0.6, 0.3, 0.7, 0.2, 0.8, 0.15, 0.85, 0.1, 0.9,
    0.05, 0.95,
];
const SWEEPS: usize = 4;
/// Label box estimate: width per character, horizontal padding, height.
const LABEL_CHAR_W: f32 = 6.5;
const LABEL_PAD: f32 = 8.0;
const LABEL_H: f32 = 20.0;
/// Space a label keeps from nodes and other labels; touching is not clear.
const LABEL_CLEAR: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum NodeKind {
    Start,
    Agent,
    Approval,
    Succeeded,
    Failed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DiagramNode {
    /// The state id, `START_ID`, `SUCCEEDED` or `FAILED`.
    pub id: String,
    pub kind: NodeKind,
    pub title: String,
    /// Row, from 0 (Start) downwards.
    pub rank: usize,
    /// Position within the row, from 0 at the left.
    pub order: usize,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// One arrow. `Diagram::edges` lists the Start edge first, then each state's
/// edges in declared state order and, within a state, in declared transition
/// order (transitions to unknown states are dropped). The run overlay names a
/// taken edge by `(from, to, index)`, where `index` is the edge's position
/// among that state's transitions with the same target; this order is what
/// makes that index line up.
#[derive(Clone, Debug, PartialEq)]
pub struct DiagramEdge {
    pub from: String,
    pub to: String,
    /// The guard label, cut to 24 characters plus `…`. `None` for a plain
    /// unguarded edge.
    pub label: Option<String>,
    /// The uncut label.
    pub full_label: Option<String>,
    /// Drawn up the right-hand side: a DFS back-edge (including a self-loop),
    /// or any edge whose target is not on a lower rank than its source.
    pub back: bool,
    /// Not a declared transition: the Start marker's edge into `initial`.
    pub implicit: bool,
    /// A failure exit (the target is `$failed`, declared or implicit): out
    /// of the source's left side and down the shared failure lane.
    pub failure: bool,
    /// Cubic bezier control points `[p0, c1, c2, p1]` for a forward edge
    /// between adjacent ranks; polyline vertices for everything else
    /// (back-edges, forward edges that skip ranks, failure exits).
    pub points: Vec<(f32, f32)>,
    /// Where the label (and the click target) is anchored: the centre of a
    /// label box estimated at 6.5 px per character plus 8, by 16 px.
    pub label_at: (f32, f32),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Diagram {
    /// Start, then the states in declared order, then the terminal pills that
    /// some transition targets (Succeeded before Failed).
    pub nodes: Vec<DiagramNode>,
    pub edges: Vec<DiagramEdge>,
    /// Bounds of every node, edge point and label box, from the origin.
    pub width: f32,
    pub height: f32,
}

/// The full (uncut) label of each of a state's transitions, in order. An
/// unguarded transition reads "otherwise" only after an alternative success
/// branch: an earlier Output-guarded sibling that does not go to `$failed`.
/// Status guards and guarded failure exits (`blocked is set`) do not count.
/// The diagram cuts these for display and drops `status = failed` on
/// failure exits; the Roadmap drawer shows them as they are.
pub fn transition_labels(transitions: &[Transition]) -> Vec<Option<String>> {
    let mut output_before = false;
    transitions
        .iter()
        .map(|t| {
            let label = guard_label(&t.when, output_before);
            output_before |= matches!(t.when, Some(Guard::Output { .. })) && t.to != FAILED;
            label
        })
        .collect()
}

fn value_text(value: &Option<serde_json::Value>) -> String {
    match value {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(v) => v.to_string(),
        None => "?".into(),
    }
}

/// The full (uncut) label for a transition's guard. `is_fallback` marks an
/// unguarded transition that follows an alternative success branch (an
/// Output-guarded sibling not going to `$failed`), which reads "otherwise".
/// `None` for a plain unguarded edge.
pub fn guard_label(guard: &Option<Guard>, is_fallback: bool) -> Option<String> {
    let text = match guard {
        None if is_fallback => "otherwise".to_string(),
        None => return None,
        Some(Guard::Status { is }) => match is {
            VisitResult::Completed => "status = completed".to_string(),
            VisitResult::Failed => "status = failed".to_string(),
        },
        Some(Guard::Output { field, op, value }) => match op {
            GuardOp::Eq => format!("{field} = {}", value_text(value)),
            GuardOp::Neq => format!("{field} ≠ {}", value_text(value)),
            GuardOp::Truthy => format!("{field} is set"),
            GuardOp::Falsy => format!("{field} is not set"),
            GuardOp::Contains => format!("{field} contains {}", value_text(value)),
        },
    };
    Some(text)
}

/// Estimated size of a label's box, matching how the widget paints it: the
/// text (estimated at 6.5 px per character) plus 4 px padding each side, and
/// 20 px tall, which covers the painted `11 * 1.4 + 4` px.
pub fn label_size(label: &str) -> (f32, f32) {
    (
        label.chars().count() as f32 * LABEL_CHAR_W + LABEL_PAD,
        LABEL_H,
    )
}

/// The bezier `[p0, c1, c2, p1]` at `t`.
fn cubic_point(p: &[(f32, f32)], t: f32) -> (f32, f32) {
    let u = 1.0 - t;
    let (a, b, c, d) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
    (
        a * p[0].0 + b * p[1].0 + c * p[2].0 + d * p[3].0,
        a * p[0].1 + b * p[1].1 + c * p[2].1 + d * p[3].1,
    )
}

fn truncate(label: &str) -> String {
    if label.chars().count() <= LABEL_MAX {
        return label.to_string();
    }
    label.chars().take(LABEL_MAX).chain(['…']).collect()
}

/// A hash of what decides the layout: `initial`, the state ids and kinds,
/// and every transition's `(from, to)`. Briefs, titles, guards and limits are
/// left out, so editing them does not re-lay the diagram out.
pub fn structure_key(flow: &Flow) -> u64 {
    let mut h = DefaultHasher::new();
    flow.initial.hash(&mut h);
    flow.states.len().hash(&mut h);
    for state in &flow.states {
        state.id().hash(&mut h);
        matches!(state, FlowState::Approval(_)).hash(&mut h);
        let transitions = state.transitions();
        transitions.len().hash(&mut h);
        for t in transitions {
            t.to.hash(&mut h);
        }
    }
    h.finish()
}

struct Edge {
    from: usize,
    to: usize,
    label: Option<String>,
    implicit: bool,
    back: bool,
    failure: bool,
}

/// Lays `flow` out. Never panics on an invalid Flow: duplicate state ids keep
/// the first, transitions to unknown states are dropped, and a missing
/// `initial` leaves every state in the unreachable rank.
pub fn build(flow: &Flow) -> Diagram {
    // Node indices: 0 is Start, then the states, then the terminal pills.
    let mut states: Vec<&FlowState> = Vec::new();
    let mut index: HashMap<&str, usize> = HashMap::new();
    for state in &flow.states {
        if !index.contains_key(state.id()) {
            index.insert(state.id(), states.len() + 1);
            states.push(state);
        }
    }
    let targets = |id: &str| {
        states
            .iter()
            .any(|s| s.transitions().iter().any(|t| t.to == id))
    };
    let mut kinds: Vec<NodeKind> = vec![NodeKind::Start];
    kinds.extend(states.iter().map(|s| match s {
        FlowState::Agent(_) => NodeKind::Agent,
        FlowState::Approval(_) => NodeKind::Approval,
    }));
    for (id, kind) in [(SUCCEEDED, NodeKind::Succeeded), (FAILED, NodeKind::Failed)] {
        if targets(id) {
            index.insert(id, kinds.len());
            kinds.push(kind);
        }
    }
    let count = kinds.len();
    let is_state = |n: usize| n >= 1 && n <= states.len();

    let mut edges: Vec<Edge> = Vec::new();
    let initial = index
        .get(flow.initial.as_str())
        .copied()
        .filter(|&n| is_state(n));
    if let Some(initial) = initial {
        edges.push(Edge {
            from: 0,
            to: initial,
            label: None,
            implicit: true,
            back: false,
            failure: false,
        });
    }
    for (i, state) in states.iter().enumerate() {
        let labels = transition_labels(state.transitions());
        for (t, label) in state.transitions().iter().zip(labels) {
            let Some(&to) = index.get(t.to.as_str()) else {
                continue;
            };
            let failure = kinds[to] == NodeKind::Failed;
            // The failure lane already says "failed".
            let label = match &t.when {
                Some(Guard::Status {
                    is: VisitResult::Failed,
                }) if failure => None,
                _ => label,
            };
            edges.push(Edge {
                from: i + 1,
                to,
                label,
                implicit: false,
                back: false,
                failure,
            });
        }
    }
    let mut out: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (e, edge) in edges.iter().enumerate() {
        out[edge.from].push(e);
    }

    // Back-edges: an iterative DFS from `initial`, transitions in declared
    // order; an edge to a node still on the stack (a self-loop included) is a
    // back-edge.
    let mut visited = vec![false; count];
    let mut on_stack = vec![false; count];
    let mut postorder: Vec<usize> = Vec::new();
    if let Some(initial) = initial {
        let mut stack: Vec<(usize, usize)> = vec![(initial, 0)];
        visited[initial] = true;
        on_stack[initial] = true;
        while let Some(&mut (node, ref mut next)) = stack.last_mut() {
            if let Some(&e) = out[node].get(*next) {
                *next += 1;
                let to = edges[e].to;
                if on_stack[to] {
                    edges[e].back = true;
                } else if !visited[to] {
                    visited[to] = true;
                    on_stack[to] = true;
                    stack.push((to, 0));
                }
            } else {
                on_stack[node] = false;
                postorder.push(node);
                stack.pop();
            }
        }
    }

    // Ranks: the longest path from Start over forward edges. Reverse
    // postorder is a topological order of the DFS's forward edges.
    let mut rank = vec![0usize; count];
    if let Some(initial) = initial {
        rank[initial] = 1;
    }
    for &node in postorder.iter().rev() {
        for &e in &out[node] {
            let edge = &edges[e];
            if !edge.back && is_state(edge.to) {
                rank[edge.to] = rank[edge.to].max(rank[node] + 1);
            }
        }
    }
    let deepest = (1..=states.len())
        .filter(|&n| visited[n])
        .map(|n| rank[n])
        .max()
        .unwrap_or(0);
    let unreachable: Vec<usize> = (1..=states.len()).filter(|&n| !visited[n]).collect();
    for &n in &unreachable {
        rank[n] = deepest + 1;
    }
    // Succeeded sits one below the deepest rank that targets it; Failed
    // sits alone at the very bottom, under the failure lane.
    let failed = index.get(FAILED).copied();
    for n in states.len() + 1..count {
        rank[n] = edges
            .iter()
            .filter(|e| e.to == n)
            .map(|e| rank[e.from] + 1)
            .max()
            .unwrap_or(1);
    }
    if let Some(f) = failed {
        rank[f] = (0..count)
            .filter(|&n| n != f)
            .map(|n| rank[n] + 1)
            .max()
            .unwrap_or(1);
    }
    for edge in &mut edges {
        if rank[edge.to] <= rank[edge.from] {
            edge.back = true;
        }
    }

    // Order within a rank: declared order (node index), refined by
    // barycenter sweeps over forward edges, ties broken by declared order.
    let ranks = rank.iter().copied().max().unwrap_or(0) + 1;
    let mut rows: Vec<Vec<usize>> = vec![Vec::new(); ranks];
    for n in 0..count {
        rows[rank[n]].push(n);
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut succs: Vec<Vec<usize>> = vec![Vec::new(); count];
    // Failure exits are left out: the Failed pill is not placed by order.
    for edge in edges.iter().filter(|e| !e.back && !e.failure) {
        preds[edge.to].push(edge.from);
        succs[edge.from].push(edge.to);
    }
    // A node's position, centred on its row so rows of different lengths
    // line up the way they are drawn.
    let mut pos = vec![0f32; count];
    let place = |row: &[usize], pos: &mut [f32]| {
        let mid = (row.len() as f32 - 1.0) / 2.0;
        for (i, &n) in row.iter().enumerate() {
            pos[n] = i as f32 - mid;
        }
    };
    for row in &rows {
        place(row, &mut pos);
    }
    for sweep in 0..SWEEPS {
        let down = sweep % 2 == 0;
        let order: Vec<usize> = if down {
            (1..ranks).collect()
        } else {
            (0..ranks.saturating_sub(1)).rev().collect()
        };
        for r in order {
            let neighbours = if down { &preds } else { &succs };
            let bary: Vec<(f32, usize)> = rows[r]
                .iter()
                .map(|&n| {
                    let around = &neighbours[n];
                    let b = if around.is_empty() {
                        pos[n]
                    } else {
                        around.iter().map(|&m| pos[m]).sum::<f32>() / around.len() as f32
                    };
                    (b, n)
                })
                .collect();
            let mut sorted = bary;
            sorted.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            rows[r] = sorted.into_iter().map(|(_, n)| n).collect();
            place(&rows[r], &mut pos);
        }
    }

    // Coordinates.
    let slot = NODE_W + NODE_GAP;
    let widest = rows.iter().map(Vec::len).max().unwrap_or(1).max(1);
    let total_w = widest as f32 * NODE_W + (widest - 1) as f32 * NODE_GAP;
    let mut nodes: Vec<DiagramNode> = Vec::with_capacity(count);
    let mut order = vec![0usize; count];
    for row in &rows {
        for (i, &n) in row.iter().enumerate() {
            order[n] = i;
        }
    }
    for n in 0..count {
        let kind = kinds[n];
        let (id, title) = match kind {
            NodeKind::Start => (START_ID.to_string(), "Start".to_string()),
            NodeKind::Succeeded => (SUCCEEDED.to_string(), "Done".to_string()),
            NodeKind::Failed => (FAILED.to_string(), "Failed".to_string()),
            NodeKind::Agent | NodeKind::Approval => {
                let s = states[n - 1];
                let title = match s {
                    FlowState::Agent(a) => a.title.clone(),
                    FlowState::Approval(a) => a.title.clone(),
                };
                (s.id().to_string(), title)
            }
        };
        let (w, h) = match kind {
            NodeKind::Agent | NodeKind::Approval => (NODE_W, NODE_H),
            _ => (PILL_W, NODE_H / 2.0),
        };
        let row_len = rows[rank[n]].len();
        let row_w = row_len as f32 * NODE_W + (row_len - 1) as f32 * NODE_GAP;
        let x = (total_w - row_w) / 2.0 + order[n] as f32 * slot + (NODE_W - w) / 2.0;
        nodes.push(DiagramNode {
            id,
            kind,
            title,
            rank: rank[n],
            order: order[n],
            x,
            y: rank[n] as f32 * (NODE_H + RANK_GAP),
            w,
            h,
        });
    }

    // The failure lane runs down the left, `FAIL_MARGIN` left of every
    // node, and the Failed pill is centred under it. A failure exit with a
    // label carries it on its horizontal stub, so the margin widens until
    // the widest such label fits there with `STUB_CLEAR` on both sides.
    if let Some(f) = failed {
        let mut margin = FAIL_MARGIN;
        for edge in edges.iter().filter(|e| e.failure) {
            let Some(label) = &edge.label else {
                continue;
            };
            let src = &nodes[edge.from];
            if side_route(&nodes, edge.from, false, src.y + src.h / 2.0, true).len() == 1 {
                let (w, _) = label_size(&truncate(label));
                margin = margin.max(w + 2.0 * STUB_CLEAR - src.x);
            }
        }
        let shift = FAIL_LANE_X + margin;
        for (n, node) in nodes.iter_mut().enumerate() {
            if n != f {
                node.x += shift;
            }
        }
        nodes[f].x = FAIL_LANE_X - PILL_W / 2.0;
    }

    // Edge geometry.
    let mut pair_count: HashMap<(usize, usize), usize> = HashMap::new();
    for edge in &edges {
        *pair_count.entry((edge.from, edge.to)).or_default() += 1;
    }
    let mut pair_seen: HashMap<(usize, usize), usize> = HashMap::new();
    let mut drawn: Vec<DiagramEdge> = Vec::with_capacity(edges.len());
    // A label anchor tried before the points along the path.
    let mut preferred: Vec<Option<(f32, f32)>> = Vec::with_capacity(edges.len());
    // Routes that run in a right-hand lane, placed once all are known:
    // (edge, rank span, points up to the lane, points from the lane).
    type Laned = (usize, usize, Vec<(f32, f32)>, Vec<(f32, f32)>);
    let mut laned: Vec<Laned> = Vec::new();
    for (e, edge) in edges.iter().enumerate() {
        let (src, dst) = (&nodes[edge.from], &nodes[edge.to]);
        let span = rank[edge.from].abs_diff(rank[edge.to]);
        let (points, pref) = if edge.failure {
            let points = failure_route(&nodes, edge.from, FAIL_LANE_X, dst.y);
            let pref = failure_anchor(&points);
            (points, Some(pref))
        } else if edge.back {
            // Out of the source's right side at 2/3 height, into the
            // target's right side at 1/3, so an exit and an entry on one
            // node never share a point.
            let exit = side_route(&nodes, edge.from, true, src.y + src.h * 2.0 / 3.0, true);
            let mut entry = side_route(&nodes, edge.to, true, dst.y + dst.h / 3.0, false);
            entry.reverse();
            laned.push((e, span, exit, entry));
            (Vec::new(), None)
        } else {
            let k = pair_count[&(edge.from, edge.to)];
            let seen = pair_seen.entry((edge.from, edge.to)).or_default();
            let step = *seen as f32 - (k as f32 - 1.0) / 2.0;
            *seen += 1;
            let dx = step * PARALLEL_STEP;
            let p0 = (src.x + src.w / 2.0 + dx, src.y + src.h);
            let p1 = (dst.x + dst.w / 2.0 + dx, dst.y);
            if span > 1 {
                // Skips ranks: down into the rank gap, across to a lane,
                // down it, and across into the target's top.
                let exit = vec![p0, (p0.0, src.y + NODE_H + RANK_GAP / 2.0)];
                let entry = vec![(p1.0, p1.1 - RANK_GAP / 2.0), p1];
                laned.push((e, span, exit, entry));
                (Vec::new(), None)
            } else {
                let c1 = (p0.0, p0.1 + RANK_GAP / 2.0);
                let c2 = (p1.0, p1.1 - RANK_GAP / 2.0);
                let mid = cubic_point(&[p0, c1, c2, p1], 0.5);
                let mid = (mid.0, mid.1 + step * PARALLEL_LABEL_STEP);
                (vec![p0, c1, c2, p1], Some(mid))
            }
        };
        drawn.push(DiagramEdge {
            from: src.id.clone(),
            to: dst.id.clone(),
            label: edge.label.as_deref().map(truncate),
            full_label: edge.label.clone(),
            back: edge.back,
            implicit: edge.implicit,
            failure: edge.failure,
            points,
            label_at: (0.0, 0.0),
        });
        preferred.push(pref);
    }

    // Right-hand lanes, shortest span first (declared order on a tie), so a
    // loop inside another takes the inner lane and they do not cross. Each
    // lane is the first `LANE_STEP` step right of the nodes beside it that
    // keeps `LANE_STEP` from every lane already placed over an overlapping
    // height.
    laned.sort_by_key(|&(e, span, ..)| (span, e));
    let mut lanes: Vec<(f32, f32, f32)> = Vec::new();
    for (e, _, exit, entry) in laned {
        let (Some(&(_, y0)), Some(&(_, y1))) = (exit.last(), entry.first()) else {
            continue;
        };
        let (top, bottom) = (y0.min(y1), y0.max(y1));
        let base = nodes
            .iter()
            .filter(|n| n.y - EDGE_CLEAR < bottom && n.y + n.h + EDGE_CLEAR > top)
            .map(|n| n.x + n.w)
            .chain(exit.iter().chain(&entry).map(|p| p.0))
            .fold(0f32, f32::max);
        // A labelled self-loop runs entirely beside its node, so its label
        // (anchored on the lane) only clears the node, and anything else
        // beside it, when the lane is pushed out by half the label's width.
        let mut first = base + LANE_STEP;
        if let Some(label) = drawn[e]
            .label
            .as_deref()
            .filter(|_| edges[e].from == edges[e].to)
        {
            let (w, _) = label_size(label);
            first = first.max(base + w / 2.0 + LABEL_CLEAR + 1.0);
        }
        let x = (0..)
            .map(|lane| first + LANE_STEP * lane as f32)
            .find(|&x| {
                lanes
                    .iter()
                    .all(|&(t, b, other)| t > bottom || top > b || (other - x).abs() >= LANE_STEP)
            })
            .unwrap_or(first);
        lanes.push((top, bottom, x));
        let mut points = exit;
        points.extend([(x, y0), (x, y1)]);
        points.extend(entry);
        drawn[e].points = points;
    }

    place_labels(&nodes, &mut drawn, &preferred);

    let mut width = nodes.iter().map(|n| n.x + n.w).fold(0f32, f32::max);
    let mut height = nodes.iter().map(|n| n.y + n.h).fold(0f32, f32::max);
    for e in &drawn {
        let (x, y) = extent(e);
        width = width.max(x);
        height = height.max(y);
    }
    Diagram {
        nodes,
        edges: drawn,
        width,
        height,
    }
}

type Rect = (f32, f32, f32, f32);

/// The furthest right and down an edge reaches: its points, its anchor and
/// its label box.
fn extent(e: &DiagramEdge) -> (f32, f32) {
    let (mut x, mut y) = e.label_at;
    for &(px, py) in &e.points {
        x = x.max(px);
        y = y.max(py);
    }
    if let Some(label) = &e.label {
        let (w, h) = label_size(label);
        x = x.max(e.label_at.0 + w / 2.0);
        y = y.max(e.label_at.1 + h / 2.0);
    }
    (x, y)
}

/// The start of a route leaving node `n` sideways at height `y`: to the
/// right-hand lanes when `right`, else to the failure lane on the left.
/// Direct (one point) when no other node lies on that side at that height;
/// otherwise it steps into the gap beside `n` and down (`below`) or up to
/// the middle of the rank gap, which holds no nodes, and the run to the lane
/// starts there.
fn side_route(
    nodes: &[DiagramNode],
    n: usize,
    right: bool,
    y: f32,
    below: bool,
) -> Vec<(f32, f32)> {
    let node = &nodes[n];
    let side = if right { node.x + node.w } else { node.x };
    let blocked = nodes.iter().enumerate().any(|(i, m)| {
        i != n
            && m.y - EDGE_CLEAR < y
            && y < m.y + m.h + EDGE_CLEAR
            && if right { m.x + m.w > side } else { m.x < side }
    });
    let mut points = vec![(side, y)];
    if blocked {
        let gx = if right {
            side + NODE_GAP / 2.0
        } else {
            side - NODE_GAP / 2.0
        };
        let gy = if below {
            node.y + NODE_H + RANK_GAP / 2.0
        } else {
            node.y - RANK_GAP / 2.0
        };
        points.extend([(gx, y), (gx, gy)]);
    }
    points
}

/// A failure exit: out of the source's left side at half height, across to
/// the failure lane at `lane_x` and down it into the top of the Failed pill
/// at `pill_y`. The pill is always below every state, so the lane only goes
/// down; a source at or below the pill (only a hand-made diagram) still
/// gets the same shape, running up.
fn failure_route(nodes: &[DiagramNode], n: usize, lane_x: f32, pill_y: f32) -> Vec<(f32, f32)> {
    let src = &nodes[n];
    let mut points = side_route(nodes, n, false, src.y + src.h / 2.0, true);
    let run_y = points.last().map_or(src.y, |p| p.1);
    points.extend([(lane_x, run_y), (lane_x, pill_y)]);
    points
}

/// Where a failure exit's label goes first: the middle of its stub when it
/// leaves straight for the lane, else halfway along.
fn failure_anchor(points: &[(f32, f32)]) -> (f32, f32) {
    if points.len() == 3 {
        (
            (points[0].0 + points[1].0) / 2.0,
            (points[0].1 + points[1].1) / 2.0,
        )
    } else {
        along(points, 0.5)
    }
}

fn is_bezier(e: &DiagramEdge) -> bool {
    !e.back && !e.failure && e.points.len() == 4
}

/// The line an edge is drawn along, as a polyline.
fn path_of(e: &DiagramEdge) -> Vec<(f32, f32)> {
    if is_bezier(e) {
        (0..=24)
            .map(|i| cubic_point(&e.points, i as f32 / 24.0))
            .collect()
    } else {
        e.points.clone()
    }
}

/// The point a fraction `t` of the way along `path`, by length.
fn along(path: &[(f32, f32)], t: f32) -> (f32, f32) {
    let seg = |w: &[(f32, f32)]| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
    let total: f32 = path.windows(2).map(seg).sum();
    let mut left = total * t;
    for w in path.windows(2) {
        let len = seg(w);
        if len > 0.0 && left <= len {
            let f = left / len;
            return (
                w[0].0 + (w[1].0 - w[0].0) * f,
                w[0].1 + (w[1].1 - w[0].1) * f,
            );
        }
        left -= len;
    }
    path.last().copied().unwrap_or((0.0, 0.0))
}

fn label_rect(at: (f32, f32), (w, h): (f32, f32)) -> Rect {
    (
        at.0 - w / 2.0,
        at.1 - h / 2.0,
        at.0 + w / 2.0,
        at.1 + h / 2.0,
    )
}

fn overlap_area(a: Rect, b: Rect) -> f32 {
    let w = (a.2.min(b.2) - a.0.max(b.0)).max(0.0);
    let h = (a.3.min(b.3) - a.1.max(b.1)).max(0.0);
    w * h
}

/// Anchors every edge's label, shortest path first. A label tries its
/// preferred anchor, then
/// points along its own path from the middle outwards (`LABEL_STOPS`), and
/// takes the first whose box keeps `LABEL_CLEAR` from every node and every
/// label placed before it, and stays inside the top and left of the diagram. If none does it keeps the candidate
/// with the least overlap (the first on a tie). Unlabelled edges anchor at
/// their first candidate.
fn place_labels(
    nodes: &[DiagramNode],
    edges: &mut [DiagramEdge],
    preferred: &[Option<(f32, f32)>],
) {
    let rects: Vec<Rect> = nodes
        .iter()
        .map(|n| (n.x, n.y, n.x + n.w, n.y + n.h))
        .collect();
    let mut placed: Vec<Rect> = Vec::new();
    // Shortest path first (declared order on a tie): a short path has few
    // free spots, a long one can move further along itself.
    let length = |e: &DiagramEdge| {
        path_of(e)
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .sum::<f32>()
    };
    let mut order: Vec<usize> = (0..edges.len()).collect();
    order.sort_by(|&a, &b| {
        length(&edges[a])
            .total_cmp(&length(&edges[b]))
            .then(a.cmp(&b))
    });
    for i in order {
        let (edge, pref) = (&mut edges[i], &preferred[i]);
        let path = path_of(edge);
        if path.is_empty() {
            continue;
        }
        let mut candidates = pref
            .iter()
            .copied()
            .chain(LABEL_STOPS.iter().map(|&t| along(&path, t)));
        let Some(label) = &edge.label else {
            edge.label_at = candidates.next().unwrap_or(path[0]);
            continue;
        };
        let size = label_size(label);
        let mut best: Option<(f32, (f32, f32))> = None;
        for at in candidates {
            let r = label_rect(at, size);
            let outside = (-r.0).max(0.0) * size.1 + (-r.1).max(0.0) * size.0;
            let padded = (
                r.0 - LABEL_CLEAR,
                r.1 - LABEL_CLEAR,
                r.2 + LABEL_CLEAR,
                r.3 + LABEL_CLEAR,
            );
            let cost = rects
                .iter()
                .chain(&placed)
                .map(|&o| overlap_area(padded, o))
                .sum::<f32>()
                + outside;
            if best.is_none_or(|(b, _)| cost < b) {
                best = Some((cost, at));
            }
            if cost == 0.0 {
                break;
            }
        }
        let Some((_, mut at)) = best else { continue };
        // Degenerate: when no candidate keeps the box inside the top-left
        // corner of the diagram, push it in so it stays in bounds.
        at.0 = at.0.max(size.0 / 2.0);
        at.1 = at.1.max(size.1 / 2.0);
        placed.push(label_rect(at, size));
        edge.label_at = at;
    }
}

/// A node's state in a run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeStatus {
    NotReached,
    Running,
    AwaitingApproval,
    Completed,
    Failed,
    Cancelled,
}

impl From<VisitStatus> for NodeStatus {
    fn from(status: VisitStatus) -> Self {
        match status {
            VisitStatus::Running => Self::Running,
            VisitStatus::WaitingApproval => Self::AwaitingApproval,
            VisitStatus::Completed => Self::Completed,
            VisitStatus::Failed => Self::Failed,
            VisitStatus::Cancelled => Self::Cancelled,
        }
    }
}

/// What a run adds to the diagram: per-node status, the edges it took and how
/// it ended. Computed from the run's visits; the layout is never redone.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Overlay {
    /// Latest status and visit count per state id. States never visited are
    /// absent (read as `NotReached`, 0).
    pub nodes: HashMap<String, (NodeStatus, u32)>,
    /// The state the run is on now.
    pub current: Option<String>,
    /// Taken edges as `(from, to, index)`, the index being the edge's position
    /// among `from`'s transitions with the same target.
    pub taken: HashSet<(String, String, usize)>,
    /// `$succeeded` or `$failed` once the run has finished that way.
    pub ended_at: Option<String>,
    /// The run failed but not through a `$failed` transition (nothing
    /// routed the last visit, a restart cut it off, or a visit or step cap
    /// stopped the run): the widget draws a dashed implicit edge from the
    /// current state to the Failed pill.
    pub implicit_fail: bool,
}

/// The overlay for `run` given its `visits` (in any order; `seq` decides).
/// Failed visits are routed with empty outputs; Completed visits without
/// outputs, visits a restart interrupted, and Running/Waiting/Cancelled ones
/// route nothing.
///
/// A routed visit's transition counts as taken only when the runner followed
/// it: the next visit is to its target, or it is the last visit and the run
/// ended on that terminal. A run can fail without either (a restart, a visit
/// or step cap); then the failure is implicit.
pub fn overlay(run: &FlowRun, visits: &[FlowVisit]) -> Overlay {
    let mut ordered: Vec<&FlowVisit> = visits.iter().collect();
    ordered.sort_by_key(|v| v.seq);

    let mut out = Overlay {
        current: run.current_state.clone(),
        ended_at: match run.status {
            RunStatus::Succeeded => Some(SUCCEEDED.to_string()),
            RunStatus::Failed => Some(FAILED.to_string()),
            _ => None,
        },
        ..Overlay::default()
    };
    let no_values = HashMap::new();
    // The transition each visit chose, as `(from, to, index)`.
    let mut chosen: Vec<Option<(String, String, usize)>> = Vec::with_capacity(ordered.len());
    for visit in &ordered {
        let entry = out
            .nodes
            .entry(visit.state_id.clone())
            .or_insert((NodeStatus::NotReached, 0));
        entry.0 = visit.status.into();
        entry.1 += 1;

        // A Failed visit normally has no outputs, which `status = failed` guards
        // do not need. A Completed one without outputs was interrupted before
        // it reported, and a Failed one `recover` cut off never settled: both
        // route nowhere.
        let result = match (visit.status, &visit.outputs) {
            (VisitStatus::Completed, Some(_)) => Some(VisitResult::Completed),
            (VisitStatus::Failed, _) if visit.error.as_deref() != Some(INTERRUPTED) => {
                Some(VisitResult::Failed)
            }
            _ => None,
        };
        let state = run.flow.states.iter().find(|s| s.id() == visit.state_id);
        chosen.push(result.and_then(|result| {
            let state = state?;
            let values = visit.outputs.as_ref().unwrap_or(&no_values);
            let t = select_transition(state, result, values)?;
            let index = state
                .transitions()
                .iter()
                .take_while(|other| !std::ptr::eq(*other, t))
                .filter(|other| other.to == t.to)
                .count();
            Some((state.id().to_string(), t.to.clone(), index))
        }));
    }
    out.implicit_fail = run.status == RunStatus::Failed
        && chosen
            .last()
            .is_some_and(|c| c.as_ref().is_none_or(|(_, to, _)| to != FAILED));
    let ended = out.ended_at.as_deref();
    for (i, edge) in chosen.into_iter().enumerate() {
        let Some(edge) = edge else {
            continue;
        };
        let followed = match ordered.get(i + 1) {
            Some(next) => next.state_id == edge.1,
            None => ended == Some(edge.1.as_str()),
        };
        if followed {
            out.taken.insert(edge);
        }
    }
    out
}

/// State ids whose definition differs between `before` and `after` (or that
/// are new), plus every id named by a run edit. With no `before`, only the
/// edits count.
pub fn changed_states(before: Option<&Flow>, after: &Flow, edits: &[RunEdit]) -> HashSet<String> {
    let mut changed: HashSet<String> = edits.iter().flat_map(|e| e.states.clone()).collect();
    if let Some(before) = before {
        for state in &after.states {
            if before.states.iter().find(|s| s.id() == state.id()) != Some(state) {
                changed.insert(state.id().to_string());
            }
        }
    }
    changed
}

/// Adds the implicit failure edge `from -> $failed`, drawn like any failure
/// exit: out of `from`'s left side and down the failure lane into the Failed
/// pill. When the diagram has no pill it gets one, one rank below the
/// deepest node and centred under a new failure lane; everything else moves
/// right to make room for the lane when it has to. Calling it again for the
/// same `from` changes nothing.
pub fn with_implicit_fail(diagram: &mut Diagram, from: &str) {
    if diagram
        .edges
        .iter()
        .any(|e| e.from == from && e.to == FAILED && e.implicit)
    {
        return;
    }
    let Some(src) = diagram.nodes.iter().position(|n| n.id == from) else {
        return;
    };
    if !diagram.nodes.iter().any(|n| n.id == FAILED) {
        let left = diagram
            .nodes
            .iter()
            .map(|n| n.x)
            .fold(f32::INFINITY, f32::min);
        let dx = (FAIL_LANE_X + FAIL_MARGIN - left).max(0.0);
        if dx > 0.0 {
            for n in &mut diagram.nodes {
                n.x += dx;
            }
            for e in &mut diagram.edges {
                for p in &mut e.points {
                    p.0 += dx;
                }
                e.label_at.0 += dx;
            }
            diagram.width += dx;
        }
        let rank = diagram.nodes.iter().map(|n| n.rank).max().unwrap_or(0) + 1;
        let pill = DiagramNode {
            id: FAILED.to_string(),
            kind: NodeKind::Failed,
            title: "Failed".to_string(),
            rank,
            order: 0,
            x: FAIL_LANE_X - PILL_W / 2.0,
            y: rank as f32 * (NODE_H + RANK_GAP),
            w: PILL_W,
            h: NODE_H / 2.0,
        };
        diagram.width = diagram.width.max(pill.x + pill.w);
        diagram.height = diagram.height.max(pill.y + pill.h);
        diagram.nodes.push(pill);
    }
    let pill = diagram.nodes.iter().find(|n| n.id == FAILED).unwrap();
    let points = failure_route(&diagram.nodes, src, pill.x + pill.w / 2.0, pill.y);
    let edge = DiagramEdge {
        from: from.to_string(),
        to: FAILED.to_string(),
        label: None,
        full_label: None,
        back: false,
        implicit: true,
        failure: true,
        label_at: failure_anchor(&points),
        points,
    };
    let (x, y) = extent(&edge);
    diagram.width = diagram.width.max(x);
    diagram.height = diagram.height.max(y);
    diagram.edges.push(edge);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flow::{AgentState, FlowState, GuardOp, OutputValue, Transition, VisitResult};
    use serde_json::json;

    fn to(target: &str) -> Transition {
        Transition {
            to: target.into(),
            when: None,
        }
    }

    fn when(target: &str, guard: Guard) -> Transition {
        Transition {
            to: target.into(),
            when: Some(guard),
        }
    }

    fn status(is: VisitResult) -> Guard {
        Guard::Status { is }
    }

    fn output(field: &str, op: GuardOp, value: Option<serde_json::Value>) -> Guard {
        Guard::Output {
            field: field.into(),
            op,
            value,
        }
    }

    fn agent(id: &str, transitions: Vec<Transition>) -> FlowState {
        FlowState::Agent(AgentState {
            id: id.into(),
            title: format!("Title {id}"),
            harness: "claude".into(),
            model: "claude:sonnet-5".into(),
            brief: format!("brief for {id}"),
            outputs: vec![],
            transitions,
            max_visits: 3,
        })
    }

    fn flow(initial: &str, states: Vec<FlowState>) -> Flow {
        Flow {
            id: "f1".into(),
            name: "Flow".into(),
            description: String::new(),
            cwd: "/repo".into(),
            inputs: vec![],
            initial: initial.into(),
            states,
            access: "auto".into(),
            owner: Some("pilot-1".into()),
            created_at: 0,
            updated_at: 0,
        }
    }

    fn node<'a>(d: &'a Diagram, id: &str) -> &'a DiagramNode {
        d.nodes
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("no node {id}"))
    }

    fn edges<'a>(d: &'a Diagram, from: &str, to: &str) -> Vec<&'a DiagramEdge> {
        d.edges
            .iter()
            .filter(|e| e.from == from && e.to == to)
            .collect()
    }

    /// Every coordinate is finite and inside the diagram's bounds.
    fn assert_in_bounds(d: &Diagram) {
        for n in &d.nodes {
            assert!(n.x >= 0.0 && n.y >= 0.0, "{} at {},{}", n.id, n.x, n.y);
            assert!(n.x + n.w <= d.width + 0.01, "{} past width", n.id);
            assert!(n.y + n.h <= d.height + 0.01, "{} past height", n.id);
        }
        for e in &d.edges {
            for &(x, y) in e.points.iter().chain([&e.label_at]) {
                assert!(x.is_finite() && y.is_finite());
                assert!(x <= d.width + 0.01 && y <= d.height + 0.01);
            }
        }
    }

    #[test]
    fn linear_flow_ranks_top_to_bottom() {
        let f = flow(
            "a",
            vec![agent("a", vec![to("b")]), agent("b", vec![to(SUCCEEDED)])],
        );
        let d = build(&f);

        let ids: Vec<&str> = d.nodes.iter().map(|n| n.id.as_str()).collect();
        assert_eq!(
            ids,
            [START_ID, "a", "b", SUCCEEDED],
            "no Failed pill when untargeted"
        );
        let ranks: Vec<usize> = d.nodes.iter().map(|n| n.rank).collect();
        assert_eq!(ranks, [0, 1, 2, 3]);
        for pair in d.nodes.windows(2) {
            assert!(
                pair[1].y > pair[0].y,
                "{} not below {}",
                pair[1].id,
                pair[0].id
            );
        }
        assert_eq!(node(&d, START_ID).kind, NodeKind::Start);
        assert_eq!(node(&d, "a").kind, NodeKind::Agent);
        assert_eq!(node(&d, "a").title, "Title a");
        assert_eq!(node(&d, SUCCEEDED).kind, NodeKind::Succeeded);
        assert_eq!(node(&d, "a").y, NODE_H + RANK_GAP);
        assert_eq!((node(&d, "a").w, node(&d, "a").h), (NODE_W, NODE_H));
        assert_eq!(node(&d, START_ID).h, NODE_H / 2.0);
        assert_eq!(node(&d, SUCCEEDED).h, NODE_H / 2.0);

        // Start -> a is the implicit entry edge; the rest are declared, unlabelled.
        let entry = edges(&d, START_ID, "a");
        assert_eq!(entry.len(), 1);
        assert!(entry[0].implicit && !entry[0].back);
        assert_eq!(d.edges.len(), 3);
        for e in &d.edges {
            assert!(!e.back);
            assert_eq!(e.label, None);
            assert_eq!(e.points.len(), 4, "forward edges are cubic beziers");
            let (from, to) = (node(&d, &e.from), node(&d, &e.to));
            // Bottom-centre of the source to top-centre of the target.
            assert_eq!(e.points[0], (from.x + from.w / 2.0, from.y + from.h));
            assert_eq!(e.points[3], (to.x + to.w / 2.0, to.y));
            assert_eq!(e.points[1].1, e.points[0].1 + RANK_GAP / 2.0);
            assert_eq!(e.points[2].1, e.points[3].1 - RANK_GAP / 2.0);
        }
        // A single column: every node's centre lines up.
        let cx = node(&d, "a").x + NODE_W / 2.0;
        for n in &d.nodes {
            assert_eq!(n.x + n.w / 2.0, cx, "{} off-centre", n.id);
        }
        assert_in_bounds(&d);
    }

    #[test]
    fn retry_loop_is_a_back_edge_on_the_side() {
        let f = flow(
            "implement",
            vec![
                agent("implement", vec![to("test")]),
                agent(
                    "test",
                    vec![
                        when("implement", status(VisitResult::Failed)),
                        to(SUCCEEDED),
                    ],
                ),
            ],
        );
        let d = build(&f);

        assert_eq!(node(&d, "implement").rank, 1);
        assert_eq!(node(&d, "test").rank, 2);
        assert_eq!(node(&d, SUCCEEDED).rank, 3);

        let retry = edges(&d, "test", "implement");
        assert_eq!(retry.len(), 1);
        let retry = retry[0];
        assert!(retry.back && !retry.implicit);
        assert_eq!(retry.label.as_deref(), Some("status = failed"));
        // Its only sibling guard is a status guard: no "otherwise".
        assert_eq!(edges(&d, "test", SUCCEEDED)[0].label, None);

        let max_x = retry.points.iter().map(|p| p.0).fold(f32::MIN, f32::max);
        for n in &d.nodes {
            assert!(max_x > n.x + n.w, "lane at {max_x} not right of {}", n.id);
        }
        // Leaves the source's right edge, enters the target's right edge.
        let (src, dst) = (node(&d, "test"), node(&d, "implement"));
        let first = retry.points[0];
        let last = *retry.points.last().unwrap();
        assert_eq!(first.0, src.x + src.w);
        assert!(first.1 > src.y && first.1 < src.y + src.h);
        assert_eq!(last.0, dst.x + dst.w);
        assert!(last.1 > dst.y && last.1 < dst.y + dst.h);
        // The label sits on the lane, between the two rows.
        assert_eq!(retry.label_at.0, max_x);
        assert!(retry.label_at.1 < first.1 && retry.label_at.1 > last.1);
        // The forward edges are not back-edges.
        assert!(d.edges.iter().filter(|e| e.back).count() == 1);
        assert_in_bounds(&d);
    }

    #[test]
    fn self_loop_and_parallel_edges_are_distinct() {
        let f = flow(
            "a",
            vec![
                agent(
                    "a",
                    vec![
                        when("a", output("again", GuardOp::Truthy, None)),
                        when("b", output("kind", GuardOp::Eq, Some(json!("x")))),
                        when("b", output("kind", GuardOp::Eq, Some(json!("y")))),
                    ],
                ),
                agent("b", vec![to(SUCCEEDED)]),
            ],
        );
        let d = build(&f);

        let from_a: Vec<&DiagramEdge> = d.edges.iter().filter(|e| e.from == "a").collect();
        assert_eq!(from_a.len(), 3);
        // Declared transition order is kept: the overlay keys taken edges by
        // (from, to, index among same-target siblings).
        let targets: Vec<&str> = from_a.iter().map(|e| e.to.as_str()).collect();
        assert_eq!(targets, ["a", "b", "b"]);
        let labels: Vec<Option<&str>> = from_a.iter().map(|e| e.label.as_deref()).collect();
        assert_eq!(
            labels,
            [Some("again is set"), Some("kind = x"), Some("kind = y")]
        );

        let self_loop = from_a[0];
        assert!(self_loop.back, "a self-loop is a back-edge");
        let max_x = self_loop
            .points
            .iter()
            .map(|p| p.0)
            .fold(f32::MIN, f32::max);
        let a = node(&d, "a");
        assert!(max_x > a.x + a.w);
        assert_ne!(self_loop.points[0], *self_loop.points.last().unwrap());

        let parallel = edges(&d, "a", "b");
        assert!(parallel.iter().all(|e| !e.back));
        assert_ne!(parallel[0].points, parallel[1].points);
        // Offsets of +-12 at both ends.
        assert_eq!(parallel[1].points[0].0 - parallel[0].points[0].0, 24.0);
        assert_eq!(parallel[1].points[3].0 - parallel[0].points[3].0, 24.0);

        let anchors: Vec<(f32, f32)> = from_a.iter().map(|e| e.label_at).collect();
        for i in 0..anchors.len() {
            for j in i + 1..anchors.len() {
                assert_ne!(
                    anchors[i], anchors[j],
                    "edges {i} and {j} share a label anchor"
                );
            }
        }
        assert_in_bounds(&d);
    }

    #[test]
    fn unreachable_and_dead_end_states_still_draw() {
        let f = flow(
            "a",
            vec![
                agent(
                    "a",
                    vec![to("dead"), when(FAILED, status(VisitResult::Failed))],
                ),
                // No transitions at all.
                agent("dead", vec![]),
                // Nothing reaches these two; one points back into the graph,
                // one at a state that does not exist.
                agent("orphan", vec![to("a"), to("ghost")]),
                agent("orphan2", vec![to(SUCCEEDED)]),
            ],
        );
        let d = build(&f);

        assert_eq!(d.nodes.len(), 7, "start, four states, two terminals");
        assert_eq!(node(&d, "a").rank, 1);
        assert_eq!(node(&d, "dead").rank, 2);
        // Unreachable states share one trailing rank, in declared order.
        let trailing = node(&d, "orphan").rank;
        assert_eq!(trailing, 3);
        assert_eq!(node(&d, "orphan2").rank, trailing);
        assert!(node(&d, "orphan").order < node(&d, "orphan2").order);
        // Succeeded sits one below the deepest rank that targets it; Failed
        // sits alone at the bottom.
        assert_eq!(node(&d, SUCCEEDED).rank, trailing + 1);
        assert_eq!(node(&d, FAILED).rank, trailing + 2);

        assert!(
            d.edges.iter().all(|e| e.to != "ghost"),
            "unknown targets are dropped"
        );
        let up = edges(&d, "orphan", "a");
        assert_eq!(up.len(), 1);
        assert!(up[0].back, "an upward edge is routed on the side");
        assert!(edges(&d, "dead", SUCCEEDED).is_empty());

        // No two nodes overlap.
        for (i, m) in d.nodes.iter().enumerate() {
            for n in &d.nodes[i + 1..] {
                let apart =
                    m.x + m.w <= n.x || n.x + n.w <= m.x || m.y + m.h <= n.y || n.y + n.h <= m.y;
                assert!(apart, "{} overlaps {}", m.id, n.id);
            }
        }
        assert_in_bounds(&d);

        // A Flow whose initial state is missing, and one with no states.
        let broken = build(&flow("missing", vec![agent("x", vec![to("x")])]));
        assert_eq!(node(&broken, "x").rank, 1);
        assert!(edges(&broken, START_ID, "missing").is_empty());
        assert_in_bounds(&broken);
        let empty = build(&flow("a", vec![]));
        assert_eq!(empty.nodes.len(), 1);
        assert!(empty.edges.is_empty());
        assert_in_bounds(&empty);
    }

    #[test]
    fn layout_is_deterministic() {
        let f = flow(
            "plan",
            vec![
                agent(
                    "plan",
                    vec![
                        when("fe", output("area", GuardOp::Eq, Some(json!("frontend")))),
                        when("be", output("area", GuardOp::Eq, Some(json!("backend")))),
                        to("docs"),
                    ],
                ),
                agent("docs", vec![to("review")]),
                agent("be", vec![to("review")]),
                agent("fe", vec![to("review")]),
                agent(
                    "review",
                    vec![
                        when("plan", output("approved", GuardOp::Falsy, None)),
                        when(
                            FAILED,
                            output("notes", GuardOp::Contains, Some(json!("api"))),
                        ),
                        when(FAILED, status(VisitResult::Failed)),
                        to(SUCCEEDED),
                    ],
                ),
            ],
        );
        let first = build(&f);
        assert_eq!(first, build(&f));
        let json = serde_json::to_string(&f).unwrap();
        let round_tripped: Flow = serde_json::from_str(&json).unwrap();
        assert_eq!(first, build(&round_tripped));
        assert_eq!(structure_key(&f), structure_key(&round_tripped));

        // The three branches share a rank, and barycenter sweeps keep the
        // fan-out in a stable order.
        let branch = node(&first, "docs").rank;
        assert_eq!(node(&first, "be").rank, branch);
        assert_eq!(node(&first, "fe").rank, branch);
        let mut orders: Vec<usize> = ["docs", "be", "fe"]
            .iter()
            .map(|id| node(&first, id).order)
            .collect();
        orders.sort();
        assert_eq!(orders, [0, 1, 2]);
        assert_in_bounds(&first);
    }

    #[test]
    fn guard_labels_follow_the_wording() {
        let cases = [
            (
                Some(output("kind", GuardOp::Eq, Some(json!("bug")))),
                "kind = bug",
            ),
            (
                Some(output("count", GuardOp::Neq, Some(json!(3)))),
                "count ≠ 3",
            ),
            (
                Some(output("ok", GuardOp::Eq, Some(json!(true)))),
                "ok = true",
            ),
            (Some(output("pr", GuardOp::Truthy, None)), "pr is set"),
            (Some(output("pr", GuardOp::Falsy, None)), "pr is not set"),
            (
                Some(output("tags", GuardOp::Contains, Some(json!("ui")))),
                "tags contains ui",
            ),
            (Some(status(VisitResult::Completed)), "status = completed"),
            (Some(status(VisitResult::Failed)), "status = failed"),
        ];
        for (guard, want) in &cases {
            assert_eq!(guard_label(guard, false).as_deref(), Some(*want));
            // A guard always wins over the fallback flag.
            assert_eq!(guard_label(guard, true).as_deref(), Some(*want));
        }
        assert_eq!(guard_label(&None, true).as_deref(), Some("otherwise"));
        assert_eq!(guard_label(&None, false), None);

        // On edges: "otherwise" only after a guarded sibling; long labels cut.
        let long = "x".repeat(40);
        let f = flow(
            "a",
            vec![
                agent(
                    "a",
                    vec![
                        when("b", output("summary", GuardOp::Eq, Some(json!(long)))),
                        to(SUCCEEDED),
                    ],
                ),
                agent(
                    "b",
                    vec![to(SUCCEEDED), when(FAILED, status(VisitResult::Failed))],
                ),
            ],
        );
        let d = build(&f);
        let cut = &edges(&d, "a", "b")[0];
        let full = format!("summary = {long}");
        let want: String = full.chars().take(24).chain(['…']).collect();
        assert_eq!(cut.label.as_deref(), Some(want.as_str()));
        assert_eq!(cut.label.as_ref().unwrap().chars().count(), 25);
        assert_eq!(cut.full_label.as_deref(), Some(full.as_str()));
        assert_eq!(
            edges(&d, "a", SUCCEEDED)[0].label.as_deref(),
            Some("otherwise")
        );
        // Unguarded before a guarded sibling is not a fallback.
        let plain = edges(&d, "b", SUCCEEDED)[0];
        assert_eq!(
            (plain.label.as_deref(), plain.full_label.as_deref()),
            (None, None)
        );
        // A status = failed exit runs down the failure lane, unlabelled.
        let lane = edges(&d, "b", FAILED)[0];
        assert!(lane.failure);
        assert_eq!(
            (lane.label.as_deref(), lane.full_label.as_deref()),
            (None, None)
        );
    }

    #[test]
    fn structure_key_ignores_briefs_but_not_transitions() {
        let base = flow(
            "a",
            vec![agent("a", vec![to("b")]), agent("b", vec![to(SUCCEEDED)])],
        );
        let key = structure_key(&base);

        let mut cosmetic = base.clone();
        if let FlowState::Agent(s) = &mut cosmetic.states[0] {
            s.brief = "a completely different brief".into();
            s.max_visits = 9;
        }
        cosmetic.name = "Renamed".into();
        cosmetic.updated_at = 99;
        assert_eq!(structure_key(&cosmetic), key);

        let mut retargeted = base.clone();
        if let FlowState::Agent(s) = &mut retargeted.states[1] {
            s.transitions = vec![to(FAILED)];
        }
        assert_ne!(structure_key(&retargeted), key);

        let mut added = base.clone();
        if let FlowState::Agent(s) = &mut added.states[1] {
            s.transitions.push(to("a"));
        }
        assert_ne!(structure_key(&added), key);

        let mut renamed = base.clone();
        if let FlowState::Agent(s) = &mut renamed.states[1] {
            s.id = "c".into();
        }
        if let FlowState::Agent(s) = &mut renamed.states[0] {
            s.transitions = vec![to("c")];
        }
        assert_ne!(structure_key(&renamed), key);

        let mut approval = base.clone();
        approval.states[1] = FlowState::Approval(crate::flow::ApprovalState {
            id: "b".into(),
            title: "Title b".into(),
            message: "ok?".into(),
            transitions: vec![to(SUCCEEDED)],
            max_visits: 3,
        });
        assert_ne!(structure_key(&approval), key);
        assert_eq!(build(&approval).nodes[2].kind, NodeKind::Approval);
    }

    fn run_of(f: &Flow, status: RunStatus, current: Option<&str>) -> FlowRun {
        FlowRun {
            id: "r1".into(),
            flow_id: f.id.clone(),
            flow: f.clone(),
            status,
            inputs: HashMap::new(),
            worktree_cwd: None,
            current_state: current.map(String::from),
            error: None,
            created_at: 0,
            finished_at: None,
            tag: None,
            edits: vec![],
        }
    }

    fn visit(
        seq: u32,
        state: &str,
        status: VisitStatus,
        outputs: Option<Vec<(&str, OutputValue)>>,
    ) -> FlowVisit {
        FlowVisit {
            id: format!("v{seq}"),
            run_id: "r1".into(),
            seq,
            state_id: state.into(),
            status,
            prompt: None,
            session_id: None,
            outputs: outputs.map(|o| o.into_iter().map(|(k, v)| (k.to_string(), v)).collect()),
            error: None,
            retried: false,
            note: None,
            started_at: 0,
            finished_at: None,
        }
    }

    fn key(from: &str, to: &str, i: usize) -> (String, String, usize) {
        (from.into(), to.into(), i)
    }

    #[test]
    fn overlay_marks_latest_status_and_visit_counts() {
        let f = flow(
            "a",
            vec![
                agent("a", vec![to("b")]),
                agent(
                    "b",
                    vec![when("a", status(VisitResult::Failed)), to(SUCCEEDED)],
                ),
            ],
        );
        let run = run_of(&f, RunStatus::Running, Some("a"));
        let visits = [
            // Out of order on purpose: seq decides which is latest.
            visit(3, "a", VisitStatus::Running, None),
            visit(1, "a", VisitStatus::Completed, Some(vec![])),
            visit(2, "b", VisitStatus::Failed, Some(vec![])),
        ];
        let o = overlay(&run, &visits);
        assert_eq!(o.nodes["a"], (NodeStatus::Running, 2));
        assert_eq!(o.nodes["b"], (NodeStatus::Failed, 1));
        assert!(!o.nodes.contains_key("zzz"));
        assert_eq!(o.current.as_deref(), Some("a"));
        assert_eq!(o.ended_at, None);
        assert!(!o.implicit_fail);
        assert_eq!(o.taken, HashSet::from([key("a", "b", 0), key("b", "a", 0)]));

        let waiting = overlay(&run, &[visit(1, "a", VisitStatus::WaitingApproval, None)]);
        assert_eq!(waiting.nodes["a"], (NodeStatus::AwaitingApproval, 1));
        let cancelled = overlay(&run, &[visit(1, "a", VisitStatus::Cancelled, None)]);
        assert_eq!(cancelled.nodes["a"], (NodeStatus::Cancelled, 1));

        let done = overlay(
            &run_of(&f, RunStatus::Succeeded, None),
            &[visit(1, "b", VisitStatus::Completed, Some(vec![]))],
        );
        assert_eq!(done.ended_at.as_deref(), Some(SUCCEEDED));
        assert!(done.taken.contains(&key("b", SUCCEEDED, 0)));
        assert!(overlay(&run, &[]).nodes.is_empty());
    }

    #[test]
    fn taken_edge_matches_the_runners_choice_between_parallel_edges() {
        let f = flow(
            "a",
            vec![
                agent(
                    "a",
                    vec![
                        when("b", output("kind", GuardOp::Eq, Some(json!("x")))),
                        when("c", output("kind", GuardOp::Eq, Some(json!("z")))),
                        when("b", output("kind", GuardOp::Eq, Some(json!("y")))),
                        to("b"),
                    ],
                ),
                agent("b", vec![]),
                agent("c", vec![]),
            ],
        );
        let run = run_of(&f, RunStatus::Running, Some("b"));
        // `next` is the state the runner entered after a.
        let taken = |kind: Option<&str>, next: &str| {
            let outputs = Some(match kind {
                Some(k) => vec![("kind", OutputValue::String(k.into()))],
                None => vec![],
            });
            let visits = [
                visit(1, "a", VisitStatus::Completed, outputs),
                visit(2, next, VisitStatus::Running, None),
            ];
            overlay(&run, &visits).taken
        };
        // The second edge to b (declared third) is index 1.
        assert_eq!(taken(Some("y"), "b"), HashSet::from([key("a", "b", 1)]));
        assert_eq!(taken(Some("x"), "b"), HashSet::from([key("a", "b", 0)]));
        assert_eq!(taken(Some("z"), "c"), HashSet::from([key("a", "c", 0)]));
        // The unguarded fallback is the third edge to b.
        assert_eq!(taken(None, "b"), HashSet::from([key("a", "b", 2)]));
        // Until the next visit exists the arrow is not taken yet.
        let pending = overlay(&run, &[visit(1, "a", VisitStatus::Completed, Some(vec![]))]);
        assert!(pending.taken.is_empty());

        // The index lines up with the diagram's edge order.
        let d = build(&f);
        let to_b: Vec<&DiagramEdge> = edges(&d, "a", "b");
        assert_eq!(to_b[1].full_label.as_deref(), Some("kind = y"));
        assert_eq!(to_b[2].full_label.as_deref(), Some("otherwise"));
    }

    #[test]
    fn failed_run_without_failed_transition_sets_implicit_fail() {
        let f = flow(
            "a",
            vec![agent("a", vec![to("b")]), agent("b", vec![to(SUCCEEDED)])],
        );
        let failed = run_of(&f, RunStatus::Failed, None);
        let visits = [
            visit(1, "a", VisitStatus::Completed, Some(vec![])),
            visit(2, "b", VisitStatus::Failed, Some(vec![])),
        ];
        let o = overlay(&failed, &visits);
        assert!(o.implicit_fail);
        assert_eq!(o.ended_at.as_deref(), Some(FAILED));
        assert_eq!(o.taken, HashSet::from([key("a", "b", 0)]));

        // A status = failed transition routes it: no implicit edge.
        let g = flow(
            "a",
            vec![agent("a", vec![when(FAILED, status(VisitResult::Failed))])],
        );
        let routed = overlay(
            &run_of(&g, RunStatus::Failed, None),
            &[visit(1, "a", VisitStatus::Failed, Some(vec![]))],
        );
        assert!(!routed.implicit_fail);
        assert!(routed.taken.contains(&key("a", FAILED, 0)));

        // The same visits on a run that is not failed never set it.
        let running = overlay(&run_of(&f, RunStatus::Running, Some("b")), &visits);
        assert!(!running.implicit_fail);
        // A failed run with no visits at all has nothing to draw from.
        assert!(!overlay(&failed, &[]).implicit_fail);
    }

    #[test]
    fn interrupted_visit_without_outputs_highlights_nothing() {
        let f = flow(
            "a",
            vec![agent("a", vec![to("b")]), agent("b", vec![to(SUCCEEDED)])],
        );
        let run = run_of(&f, RunStatus::Cancelled, None);
        let o = overlay(
            &run,
            &[
                visit(1, "a", VisitStatus::Cancelled, None),
                visit(2, "a", VisitStatus::Running, None),
                visit(3, "a", VisitStatus::Completed, None),
            ],
        );
        assert!(o.taken.is_empty());
        assert!(!o.implicit_fail);
        assert_eq!(o.nodes["a"], (NodeStatus::Completed, 3));
        // A visit to a state the snapshot no longer has is counted, not routed.
        let ghost = overlay(
            &run,
            &[visit(1, "gone", VisitStatus::Completed, Some(vec![]))],
        );
        assert!(ghost.taken.is_empty());
        assert_eq!(ghost.nodes["gone"], (NodeStatus::Completed, 1));
    }

    #[test]
    fn changed_states_from_diff_and_run_edits() {
        let before = flow(
            "a",
            vec![agent("a", vec![to("b")]), agent("b", vec![to(SUCCEEDED)])],
        );
        let mut after = before.clone();
        if let FlowState::Agent(s) = &mut after.states[1] {
            s.brief = "new brief".into();
        }
        after.states.push(agent("c", vec![]));
        after.name = "Renamed".into();

        let edits = [RunEdit {
            at: 1,
            note: "n".into(),
            states: vec!["a".into(), "removed".into()],
        }];
        let ids = |set: HashSet<String>| {
            let mut v: Vec<String> = set.into_iter().collect();
            v.sort();
            v
        };
        assert_eq!(ids(changed_states(Some(&before), &after, &[])), ["b", "c"]);
        assert_eq!(
            ids(changed_states(Some(&before), &after, &edits)),
            ["a", "b", "c", "removed"]
        );
        // No earlier version: only the edits count, nothing is "new".
        assert_eq!(ids(changed_states(None, &after, &edits)), ["a", "removed"]);
        assert!(changed_states(Some(&before), &before, &[]).is_empty());
    }

    #[test]
    fn implicit_fail_adds_the_failed_pill_once() {
        let f = flow(
            "a",
            vec![agent("a", vec![to("b")]), agent("b", vec![to(SUCCEEDED)])],
        );
        let mut d = build(&f);
        assert!(d.nodes.iter().all(|n| n.id != FAILED));
        let deepest = d.nodes.iter().map(|n| n.rank).max().unwrap();
        let (nodes, edge_count) = (d.nodes.len(), d.edges.len());

        with_implicit_fail(&mut d, "b");
        with_implicit_fail(&mut d, "b");
        assert_eq!(d.nodes.len(), nodes + 1);
        assert_eq!(d.edges.len(), edge_count + 1);
        let pill = node(&d, FAILED);
        assert_eq!(pill.kind, NodeKind::Failed);
        assert_eq!(pill.rank, deepest + 1);
        assert_eq!(pill.y, pill.rank as f32 * (NODE_H + RANK_GAP));
        assert_eq!((pill.w, pill.h), (PILL_W, NODE_H / 2.0));
        // The nodes moved right to make room for the failure lane; the pill
        // is centred under it.
        let lane = pill.x + pill.w / 2.0;
        let left = d
            .nodes
            .iter()
            .filter(|n| n.id != FAILED)
            .map(|n| n.x)
            .fold(f32::INFINITY, f32::min);
        assert!(pill.x >= 0.0);
        assert_eq!(left - lane, FAIL_MARGIN);

        let e = &edges(&d, "b", FAILED)[0];
        assert!(e.implicit && e.failure && !e.back);
        assert_eq!((e.label.as_deref(), e.full_label.as_deref()), (None, None));
        let b = node(&d, "b");
        assert_eq!(
            e.points,
            vec![
                (b.x, b.y + b.h / 2.0),
                (lane, b.y + b.h / 2.0),
                (lane, pill.y)
            ]
        );
        // Existing edges moved with their nodes.
        let ab = edges(&d, "a", "b")[0];
        let a = node(&d, "a");
        assert_eq!(ab.points[0], (a.x + a.w / 2.0, a.y + a.h));
        assert_no_edge_crosses_a_node(&d);
        assert_in_bounds(&d);

        // A second source reuses the pill and the lane, and moves nothing.
        let before = d.nodes.clone();
        with_implicit_fail(&mut d, "a");
        assert_eq!(d.nodes, before);
        assert_eq!(d.edges.len(), edge_count + 2);
        assert_eq!(edges(&d, "a", FAILED)[0].points.last().unwrap().0, lane);

        // A Flow that already draws Failed keeps its pill.
        let g = flow(
            "a",
            vec![agent("a", vec![when(FAILED, status(VisitResult::Failed))])],
        );
        let mut d = build(&g);
        let before = d.nodes.len();
        with_implicit_fail(&mut d, "a");
        assert_eq!(d.nodes.len(), before);
        assert_eq!(d.nodes.iter().filter(|n| n.id == FAILED).count(), 1);
    }

    #[test]
    fn failed_visit_without_outputs_highlights_its_status_guard() {
        let f = flow(
            "build",
            vec![
                agent(
                    "build",
                    vec![when("retry", status(VisitResult::Failed)), to(SUCCEEDED)],
                ),
                agent("retry", vec![to("build")]),
            ],
        );
        let run = run_of(&f, RunStatus::Running, Some("retry"));
        let o = overlay(
            &run,
            &[
                visit(1, "build", VisitStatus::Failed, None),
                visit(2, "retry", VisitStatus::Running, None),
            ],
        );
        assert_eq!(o.taken, HashSet::from([key("build", "retry", 0)]));
        assert_eq!(o.nodes["build"], (NodeStatus::Failed, 1));
        assert!(!o.implicit_fail);

        // With no status = failed transition, a failed run falls to the
        // implicit edge instead.
        let g = flow("build", vec![agent("build", vec![to(SUCCEEDED)])]);
        let o = overlay(
            &run_of(&g, RunStatus::Failed, None),
            &[visit(1, "build", VisitStatus::Failed, None)],
        );
        assert!(o.taken.is_empty());
        assert!(o.implicit_fail);
    }

    #[test]
    fn interrupted_completed_visit_routes_nothing_but_its_failure_shows() {
        let f = flow(
            "a",
            vec![agent(
                "a",
                vec![when("b", output("ok", GuardOp::Truthy, None))],
            )],
        );
        let o = overlay(
            &run_of(&f, RunStatus::Failed, None),
            &[visit(1, "a", VisitStatus::Completed, None)],
        );
        assert!(o.taken.is_empty());
        // The run failed and no `$failed` arrow got it there.
        assert!(o.implicit_fail);
        // The same visit with real (empty) outputs did run and routed nowhere.
        let o = overlay(
            &run_of(&f, RunStatus::Failed, None),
            &[visit(1, "a", VisitStatus::Completed, Some(vec![]))],
        );
        assert!(o.taken.is_empty());
        assert!(o.implicit_fail);
    }

    fn interrupted(seq: u32, state: &str) -> FlowVisit {
        let mut v = visit(seq, state, VisitStatus::Failed, None);
        v.error = Some(INTERRUPTED.into());
        v
    }

    /// `build` retries itself on failure, then hands over to `ship`.
    fn retrying() -> Flow {
        flow(
            "build",
            vec![
                agent(
                    "build",
                    vec![when("build", status(VisitResult::Failed)), to("ship")],
                ),
                agent("ship", vec![to(SUCCEEDED)]),
            ],
        )
    }

    #[test]
    fn interrupted_visit_takes_no_retry_arrow_and_fails_implicitly() {
        // `recover` fails a Running visit and its run without advancing.
        let f = retrying();
        let mut run = run_of(&f, RunStatus::Failed, Some("build"));
        run.error = Some(INTERRUPTED.into());
        let o = overlay(&run, &[interrupted(1, "build")]);
        assert!(o.taken.is_empty(), "the retry arrow was never followed");
        assert!(o.implicit_fail);
        assert_eq!(o.nodes["build"], (NodeStatus::Failed, 1));

        // A real failure followed by the retry visit does take it.
        let o = overlay(
            &run_of(&f, RunStatus::Running, Some("build")),
            &[
                visit(1, "build", VisitStatus::Failed, None),
                visit(2, "build", VisitStatus::Running, None),
            ],
        );
        assert_eq!(o.taken, HashSet::from([key("build", "build", 0)]));
    }

    #[test]
    fn resumed_run_does_not_take_the_interrupted_visits_retry_arrow() {
        // Interrupted at build by a restart, then resumed at build: the
        // fresh visit is the resume, not the status = failed retry.
        let f = retrying();
        let o = overlay(
            &run_of(&f, RunStatus::Running, Some("build")),
            &[
                interrupted(1, "build"),
                visit(2, "build", VisitStatus::Running, None),
            ],
        );
        assert!(o.taken.is_empty());
        assert!(!o.implicit_fail);
        assert_eq!(o.nodes["build"], (NodeStatus::Running, 2));
    }

    #[test]
    fn over_limit_failure_takes_no_arrow_into_the_capped_state() {
        // a <-> b until `enter_state` refuses a fourth visit of a: the run
        // fails with no new visit and current_state still on b.
        let f = flow(
            "a",
            vec![
                agent("a", vec![to("b")]),
                agent("b", vec![to("a"), to(SUCCEEDED)]),
            ],
        );
        let mut run = run_of(&f, RunStatus::Failed, Some("b"));
        run.error = Some("State a exceeded 3 visits".into());
        let visits: Vec<FlowVisit> = (1..=6)
            .map(|seq| {
                let state = if seq % 2 == 1 { "a" } else { "b" };
                visit(seq, state, VisitStatus::Completed, Some(vec![]))
            })
            .collect();
        let o = overlay(&run, &visits);
        assert_eq!(
            o.taken,
            HashSet::from([key("a", "b", 0), key("b", "a", 0)]),
            "b -> a was followed twice, but not the third time"
        );
        assert!(o.implicit_fail, "the failure shows from b");
        assert_eq!(o.current.as_deref(), Some("b"));

        // Same cap, but on the very first hop: nothing taken at all.
        let o = overlay(&run, &visits[..1]);
        assert!(o.taken.is_empty());
        assert!(o.implicit_fail);
    }

    #[test]
    fn terminal_arrow_counts_only_when_the_run_ended_there() {
        let f = flow("a", vec![agent("a", vec![to(SUCCEEDED)])]);
        let visits = [visit(1, "a", VisitStatus::Completed, Some(vec![]))];
        let done = overlay(&run_of(&f, RunStatus::Succeeded, None), &visits);
        assert_eq!(done.taken, HashSet::from([key("a", SUCCEEDED, 0)]));
        // Persisted before the run's own status caught up: not yet taken.
        let racing = overlay(&run_of(&f, RunStatus::Running, Some("a")), &visits);
        assert!(racing.taken.is_empty());
        // A run that failed anyway never reached Succeeded.
        let failed = overlay(&run_of(&f, RunStatus::Failed, Some("a")), &visits);
        assert!(failed.taken.is_empty());
        assert!(failed.implicit_fail);
    }

    #[test]
    fn implicit_fail_joins_the_failure_lane() {
        // a already fails to $failed; c fails implicitly and b -> c's retry
        // c -> b holds a right-hand lane.
        let f = flow(
            "a",
            vec![
                agent(
                    "a",
                    vec![when(FAILED, status(VisitResult::Failed)), to("b")],
                ),
                agent("b", vec![to("c")]),
                agent(
                    "c",
                    vec![when("b", status(VisitResult::Failed)), to(SUCCEEDED)],
                ),
            ],
        );
        let mut d = build(&f);
        let before = d.nodes.clone();
        let declared = edges(&d, "a", FAILED)[0].clone();
        let lane = declared.points.last().unwrap().0;
        with_implicit_fail(&mut d, "c");
        assert_eq!(d.nodes, before, "the pill and lane already exist");
        let e = edges(&d, "c", FAILED)
            .into_iter()
            .find(|e| e.implicit)
            .expect("implicit edge");
        assert!(e.failure && !e.back);
        let (c, pill) = (node(&d, "c"), node(&d, FAILED));
        assert!(pill.rank > c.rank, "the pill is below every state");
        assert_eq!(
            e.points,
            vec![
                (c.x, c.y + c.h / 2.0),
                (lane, c.y + c.h / 2.0),
                (lane, pill.y)
            ]
        );
        assert_no_edge_crosses_a_node(&d);
        assert_in_bounds(&d);
    }

    // ---- layout quality ----------------------------------------------------

    /// `(x0, y0, x1, y1)`.
    type Rect = (f32, f32, f32, f32);

    fn label_box(e: &DiagramEdge) -> Option<Rect> {
        let (w, h) = label_size(e.label.as_deref()?);
        let (x, y) = e.label_at;
        Some((x - w / 2.0, y - h / 2.0, x + w / 2.0, y + h / 2.0))
    }

    fn node_rect(n: &DiagramNode) -> Rect {
        (n.x, n.y, n.x + n.w, n.y + n.h)
    }

    fn overlaps(a: Rect, b: Rect) -> bool {
        a.0 < b.2 && b.0 < a.2 && a.1 < b.3 && b.1 < a.3
    }

    /// The line an edge is drawn along: a forward bezier sampled finely, or
    /// the polyline itself.
    fn drawn_line(e: &DiagramEdge) -> Vec<(f32, f32)> {
        if !e.back && !e.failure && e.points.len() == 4 {
            (0..=64)
                .map(|i| cubic_point(&e.points, i as f32 / 64.0))
                .collect()
        } else {
            e.points.clone()
        }
    }

    /// Whether the segment `a`-`b` enters the interior of `r` (shrunk by
    /// half a pixel so a line along or ending on a border does not count).
    fn segment_enters(a: (f32, f32), b: (f32, f32), r: Rect) -> bool {
        let r = (r.0 + 0.5, r.1 + 0.5, r.2 - 0.5, r.3 - 0.5);
        let len = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        let steps = (len.ceil() as usize).max(1) * 2;
        (0..=steps).any(|i| {
            let t = i as f32 / steps as f32;
            let (x, y) = (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
            x > r.0 && x < r.2 && y > r.1 && y < r.3
        })
    }

    /// No edge runs through a node other than its own source and target.
    fn assert_no_edge_crosses_a_node(d: &Diagram) {
        for e in &d.edges {
            let line = drawn_line(e);
            for n in d.nodes.iter().filter(|n| n.id != e.from && n.id != e.to) {
                for w in line.windows(2) {
                    assert!(
                        !segment_enters(w[0], w[1], node_rect(n)),
                        "{} -> {} runs through {}: {:?}",
                        e.from,
                        e.to,
                        n.id,
                        e.points
                    );
                }
            }
        }
    }

    /// No label box overlaps a node or another label, and every box is
    /// inside the bounds.
    fn assert_labels_clear(d: &Diagram) {
        let boxes: Vec<(&DiagramEdge, Rect)> = d
            .edges
            .iter()
            .filter_map(|e| label_box(e).map(|b| (e, b)))
            .collect();
        for (i, (e, b)) in boxes.iter().enumerate() {
            assert!(
                b.0 >= -0.01 && b.1 >= -0.01 && b.2 <= d.width + 0.01 && b.3 <= d.height + 0.01,
                "label of {} -> {} out of bounds: {b:?}",
                e.from,
                e.to
            );
            // At least 2 px apart: touching does not count as clear.
            let b = &(b.0 - 1.99, b.1 - 1.99, b.2 + 1.99, b.3 + 1.99);
            for n in &d.nodes {
                assert!(
                    !overlaps(*b, node_rect(n)),
                    "label {:?} of {} -> {} covers {}",
                    e.label,
                    e.from,
                    e.to,
                    n.id
                );
            }
            for (other, ob) in &boxes[i + 1..] {
                assert!(
                    !overlaps(*b, *ob),
                    "label {:?} of {} -> {} overlaps {:?} of {} -> {}",
                    e.label,
                    e.from,
                    e.to,
                    other.label,
                    other.from,
                    other.to
                );
            }
        }
    }

    fn deliver_a_ticket() -> Flow {
        serde_json::from_str(include_str!("fixtures/deliver_a_ticket.json")).unwrap()
    }

    #[test]
    fn deliver_a_ticket_draws_cleanly() {
        let d = build(&deliver_a_ticket());

        assert_no_edge_crosses_a_node(&d);
        assert_labels_clear(&d);
        assert_in_bounds(&d);

        // The happy path reads plainly; the retry is the fallback.
        for (from, to) in [("plan", "build"), ("build", "verify")] {
            let e = edges(&d, from, to);
            assert_eq!(e.len(), 1);
            assert_eq!(e[0].label, None, "{from} -> {to} is unlabelled");
        }
        let retry = edges(&d, "verify", "build");
        assert_eq!(retry.len(), 1);
        assert_eq!(retry[0].label.as_deref(), Some("otherwise"));

        // Every exit to Failed runs down the failure lane, left of every node.
        let failures: Vec<&DiagramEdge> = d.edges.iter().filter(|e| e.to == FAILED).collect();
        assert_eq!(
            failures.len(),
            7,
            "six status exits and Plan's blocked exit"
        );
        let left = d
            .nodes
            .iter()
            .filter(|n| n.id != FAILED)
            .map(|n| n.x)
            .fold(f32::INFINITY, f32::min);
        let pill = node(&d, FAILED);
        let lane = failures[0].points.last().unwrap().0;
        assert_eq!(pill.x + pill.w / 2.0, lane, "the pill sits under the lane");
        assert!(
            d.nodes.iter().all(|n| n.id == FAILED || n.rank < pill.rank),
            "the pill is at the bottom"
        );
        for e in &failures {
            assert!(e.failure && !e.back, "{} -> $failed", e.from);
            let n = e.points.len();
            assert!(n >= 3);
            let src = node(&d, &e.from);
            assert_eq!(
                e.points[0],
                (src.x, src.y + src.h / 2.0),
                "leaves the left side"
            );
            for &(x, _) in &e.points[n - 2..] {
                assert_eq!(x, lane);
                assert!(x < left, "lane point {x} not left of every node");
            }
            assert_eq!(e.points[n - 1].1, pill.y, "drops into the pill's top");
        }
        // status = failed exits carry no label; the blocked exit keeps its own,
        // on the stub between the lane and Plan.
        let plan = node(&d, "plan");
        let plan_fail = edges(&d, "plan", FAILED);
        assert_eq!(plan_fail[0].label, None);
        assert_eq!(plan_fail[1].label.as_deref(), Some("blocked is set"));
        let b = label_box(plan_fail[1]).unwrap();
        assert!(b.0 > lane && b.2 < plan.x, "{b:?} on the stub");
        assert_eq!(plan_fail[1].label_at.1, plan.y + plan.h / 2.0);
        for e in failures.iter().filter(|e| e.from != "plan") {
            assert_eq!((e.label.as_deref(), e.full_label.as_deref()), (None, None));
        }
        // Nothing else is a failure edge.
        assert!(d
            .edges
            .iter()
            .filter(|e| e.to != FAILED)
            .all(|e| !e.failure));
    }

    #[test]
    fn long_forward_edges_route_around_nodes() {
        // a -> c skips b, and b -> $succeeded skips c.
        let f = flow(
            "a",
            vec![
                agent(
                    "a",
                    vec![when("b", output("slow", GuardOp::Truthy, None)), to("c")],
                ),
                agent(
                    "b",
                    vec![
                        to("c"),
                        when(SUCCEEDED, output("done", GuardOp::Truthy, None)),
                    ],
                ),
                agent("c", vec![to(SUCCEEDED)]),
            ],
        );
        let d = build(&f);
        assert_eq!(node(&d, "a").rank, 1);
        assert_eq!(node(&d, "b").rank, 2);
        assert_eq!(node(&d, "c").rank, 3);
        assert_eq!(node(&d, SUCCEEDED).rank, 4);

        let right = d.nodes.iter().map(|n| n.x + n.w).fold(0f32, f32::max);
        for (from, to) in [("a", "c"), ("b", SUCCEEDED)] {
            let e = edges(&d, from, to)[0];
            assert!(!e.back && !e.failure);
            assert!(e.points.len() > 4, "{from} -> {to} is a polyline");
            let (src, dst) = (node(&d, from), node(&d, to));
            // Down out of the source's bottom, into the target's top.
            assert_eq!(e.points[0], (src.x + src.w / 2.0, src.y + src.h));
            assert_eq!(*e.points.last().unwrap(), (dst.x + dst.w / 2.0, dst.y));
            // Every segment is horizontal or vertical, and the lane is right
            // of every node.
            for w in e.points.windows(2) {
                assert!(w[0].0 == w[1].0 || w[0].1 == w[1].1, "{w:?}");
            }
            let lane = e.points.iter().map(|p| p.0).fold(f32::MIN, f32::max);
            assert!(
                lane > right,
                "{from} -> {to} lane {lane} not right of {right}"
            );
        }
        // The two long edges overlap in ranks, so their lanes differ.
        let lane = |from: &str, to: &str| {
            edges(&d, from, to)[0]
                .points
                .iter()
                .map(|p| p.0)
                .fold(f32::MIN, f32::max)
        };
        assert!((lane("a", "c") - lane("b", SUCCEEDED)).abs() >= LANE_STEP - 0.01);
        // Adjacent-rank edges stay curves.
        assert_eq!(edges(&d, "a", "b")[0].points.len(), 4);
        assert_no_edge_crosses_a_node(&d);
        assert_labels_clear(&d);
        assert_in_bounds(&d);
    }

    #[test]
    fn otherwise_needs_an_output_guarded_sibling() {
        let f = flow(
            "a",
            vec![
                // Only a status sibling: the plain edge is the happy path.
                agent(
                    "a",
                    vec![when(FAILED, status(VisitResult::Failed)), to("b")],
                ),
                // An earlier output guard: the plain edge is the fallback.
                agent(
                    "b",
                    vec![
                        when(FAILED, status(VisitResult::Failed)),
                        when("c", output("ok", GuardOp::Truthy, None)),
                        to("a"),
                    ],
                ),
                // An output guard after the plain edge does not count.
                agent(
                    "c",
                    vec![to("d"), when("a", output("redo", GuardOp::Truthy, None))],
                ),
                // A status = completed sibling is still status-only.
                agent(
                    "d",
                    vec![when(SUCCEEDED, status(VisitResult::Completed)), to(FAILED)],
                ),
            ],
        );
        let d = build(&f);
        let label = |from: &str, to: &str| edges(&d, from, to)[0].label.clone();
        assert_eq!(label("a", "b"), None);
        assert_eq!(label("b", "a").as_deref(), Some("otherwise"));
        assert_eq!(
            edges(&d, "b", "a")[0].full_label.as_deref(),
            Some("otherwise")
        );
        assert_eq!(label("c", "d"), None);
        assert_eq!(label("c", "a").as_deref(), Some("redo is set"));
        assert_eq!(label("d", SUCCEEDED).as_deref(), Some("status = completed"));
        // d's unguarded exit to Failed: a failure exit, but no fallback label.
        let df = edges(&d, "d", FAILED)[0];
        assert!(df.failure);
        assert_eq!(df.label, None);
        // An output-guarded failure exit is not a success branch either.
        let h = flow(
            "a",
            vec![agent(
                "a",
                vec![
                    when(FAILED, output("blocked", GuardOp::Truthy, None)),
                    to(SUCCEEDED),
                ],
            )],
        );
        let d = build(&h);
        assert_eq!(edges(&d, "a", SUCCEEDED)[0].label, None);
        assert_eq!(
            edges(&d, "a", FAILED)[0].label.as_deref(),
            Some("blocked is set")
        );
        // A failure exit that is a real fallback keeps "otherwise".
        let g = flow(
            "a",
            vec![agent(
                "a",
                vec![
                    when(SUCCEEDED, output("ok", GuardOp::Truthy, None)),
                    to(FAILED),
                ],
            )],
        );
        let d = build(&g);
        assert_eq!(
            edges(&d, "a", FAILED)[0].label.as_deref(),
            Some("otherwise")
        );
        assert_labels_clear(&d);
    }

    /// Whether two segments cross (touching at a shared end does not count).
    fn segments_cross(a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)) -> bool {
        let cross = |o: (f32, f32), p: (f32, f32), q: (f32, f32)| {
            (p.0 - o.0) * (q.1 - o.1) - (p.1 - o.1) * (q.0 - o.0)
        };
        let (d1, d2) = (cross(c, d, a), cross(c, d, b));
        let (d3, d4) = (cross(a, b, c), cross(a, b, d));
        d1 * d2 < 0.0 && d3 * d4 < 0.0
    }

    #[test]
    fn nested_loops_use_ordered_lanes() {
        // a -> b -> c -> d, with an outer loop d -> a declared before the
        // inner loop c -> b.
        let f = flow(
            "a",
            vec![
                agent("a", vec![to("b")]),
                agent("b", vec![to("c")]),
                agent(
                    "d",
                    vec![
                        when("a", output("again", GuardOp::Truthy, None)),
                        to(SUCCEEDED),
                    ],
                ),
                agent(
                    "c",
                    vec![when("b", output("retry", GuardOp::Truthy, None)), to("d")],
                ),
            ],
        );
        let d = build(&f);
        let outer = edges(&d, "d", "a")[0];
        let inner = edges(&d, "c", "b")[0];
        assert!(outer.back && inner.back);
        let outer_pos = d.edges.iter().position(|e| e.from == "d" && e.to == "a");
        let inner_pos = d.edges.iter().position(|e| e.from == "c" && e.to == "b");
        assert!(outer_pos < inner_pos, "edge order stays declared order");

        let lane = |e: &DiagramEdge| e.points[1].0;
        assert!(
            lane(inner) < lane(outer),
            "the shorter loop takes the inner lane: {} vs {}",
            lane(inner),
            lane(outer)
        );
        assert!(lane(outer) - lane(inner) >= LANE_STEP - 0.01);
        let right = d.nodes.iter().map(|n| n.x + n.w).fold(0f32, f32::max);
        assert_eq!(
            lane(inner),
            right + LANE_STEP,
            "the first lane is 32 px out"
        );
        for i in outer.points.windows(2) {
            for j in inner.points.windows(2) {
                assert!(
                    !segments_cross(i[0], i[1], j[0], j[1]),
                    "{i:?} crosses {j:?}"
                );
            }
        }
        // Exits at 2/3 height, entries at 1/3: never the same point.
        for e in [outer, inner] {
            let (src, dst) = (node(&d, &e.from), node(&d, &e.to));
            assert_eq!(e.points[0], (src.x + src.w, src.y + src.h * 2.0 / 3.0));
            assert_eq!(
                *e.points.last().unwrap(),
                (dst.x + dst.w, dst.y + dst.h / 3.0)
            );
        }
        assert_no_edge_crosses_a_node(&d);
        assert_labels_clear(&d);
        assert_in_bounds(&d);
    }

    #[test]
    fn edges_from_inner_nodes_step_around_their_neighbours() {
        // b and c share a row; c (right) fails to the lane on the left and
        // b (left) loops back to a past c on the right.
        let f = flow(
            "a",
            vec![
                agent(
                    "a",
                    vec![when("b", output("x", GuardOp::Truthy, None)), to("c")],
                ),
                agent(
                    "b",
                    vec![
                        when("a", output("again", GuardOp::Truthy, None)),
                        to(SUCCEEDED),
                    ],
                ),
                agent(
                    "c",
                    vec![
                        when(FAILED, output("broken", GuardOp::Truthy, None)),
                        to(SUCCEEDED),
                    ],
                ),
            ],
        );
        let d = build(&f);
        assert_eq!(node(&d, "b").rank, node(&d, "c").rank);
        assert!(node(&d, "b").x < node(&d, "c").x);
        assert!(edges(&d, "c", FAILED)[0].failure);
        assert!(edges(&d, "b", "a")[0].back);
        assert_no_edge_crosses_a_node(&d);
        assert_labels_clear(&d);
        assert_in_bounds(&d);
    }

    #[test]
    fn label_size_matches_the_painted_box() {
        // Text at 6.5 px per character plus 4 px padding each side; 20 px
        // covers the painted 11 * 1.4 + 4 = 19.4 px height.
        assert_eq!(label_size("abcd"), (4.0 * 6.5 + 8.0, 20.0));
        assert_eq!(label_size(""), (8.0, 20.0));
        assert!(label_size("x").1 >= 11.0 * 1.4 + 4.0);
    }

    #[test]
    fn labelled_self_loop_label_clears_its_node() {
        let f = flow(
            "a",
            vec![
                agent("a", vec![when("a", status(VisitResult::Failed)), to("b")]),
                // b's own self-loop is unlabelled and keeps the first lane.
                agent("b", vec![to("b"), to(SUCCEEDED)]),
            ],
        );
        let d = build(&f);
        let a = node(&d, "a");
        let lp = edges(&d, "a", "a")[0];
        assert!(lp.back);
        assert_eq!(lp.label.as_deref(), Some("status = failed"));
        let b = label_box(lp).unwrap();
        assert!(b.0 >= a.x + a.w + 2.0, "label {b:?} clears a's right edge");
        // The label sits on the lane, between the exit and the entry.
        assert_eq!(lp.label_at.0, lp.points[1].0);
        assert!(lp.label_at.1 > lp.points[2].1 && lp.label_at.1 < lp.points[1].1);
        // An unlabelled self-loop stays on the first lane.
        let bl = edges(&d, "b", "b")[0];
        let bn = node(&d, "b");
        assert_eq!(bl.points[1].0, bn.x + bn.w + LANE_STEP);
        assert_no_edge_crosses_a_node(&d);
        assert_labels_clear(&d);
        assert_in_bounds(&d);
    }

    #[test]
    fn transition_labels_share_the_fallback_rule() {
        let long = "y".repeat(40);
        let ts = vec![
            when(FAILED, status(VisitResult::Failed)),
            to("first"),
            when(FAILED, output("blocked", GuardOp::Truthy, None)),
            to("second"),
            when("c", output("summary", GuardOp::Eq, Some(json!(long)))),
            to("third"),
            when("d", status(VisitResult::Completed)),
        ];
        let labels = transition_labels(&ts);
        assert_eq!(
            labels,
            vec![
                Some("status = failed".to_string()),
                // Only a status sibling before it.
                None,
                Some("blocked is set".to_string()),
                // An output guard, but on a failure exit.
                None,
                // Full, uncut text.
                Some(format!("summary = {long}")),
                Some("otherwise".to_string()),
                Some("status = completed".to_string()),
            ]
        );
        assert!(transition_labels(&[]).is_empty());
        // The diagram uses the same labels (cut for display, and without
        // the status = failed text on a failure exit).
        let d = build(&flow("a", vec![agent("a", ts.clone())]));
        let from_a: Vec<&DiagramEdge> = d.edges.iter().filter(|e| e.from == "a").collect();
        assert_eq!(from_a[0].full_label, None);
        for (e, want) in from_a.iter().zip(&labels).skip(1) {
            if e.to != FAILED || want.is_some() {
                assert_eq!(&e.full_label, want, "{} -> {}", e.from, e.to);
            }
        }
    }
}
