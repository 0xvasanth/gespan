use super::*;
use gespan_core::flow::diagram::{build, label_size};
use gespan_core::flow::{AgentState, Flow, FlowState, Guard, GuardOp, Transition, VisitResult};
use gpui::{Entity, TestAppContext, point, size};
use std::cell::RefCell;
use std::rc::Rc;

fn to(target: &str) -> Transition {
    Transition {
        to: target.into(),
        when: None,
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

/// a -> b -> $succeeded, with b retrying a.
fn small() -> Diagram {
    build(&flow(
        "a",
        vec![
            agent("a", vec![to("b")]),
            agent("b", vec![to("$succeeded"), to("a")]),
        ],
    ))
}

/// A straight chain s0 -> s1 -> … -> s{n-1} -> $succeeded.
fn chain(n: usize) -> Diagram {
    let states = (0..n)
        .map(|i| {
            let next = if i + 1 == n {
                "$succeeded".to_string()
            } else {
                format!("s{}", i + 1)
            };
            agent(&format!("s{i}"), vec![to(&next)])
        })
        .collect();
    build(&flow("s0", states))
}

fn node<'a>(d: &'a Diagram, id: &str) -> &'a DiagramNode {
    d.nodes.iter().find(|n| n.id == id).expect("node")
}

fn running_at(id: &str) -> Overlay {
    Overlay {
        nodes: [(id.to_string(), (NodeStatus::Running, 1))].into(),
        current: Some(id.to_string()),
        ..Overlay::default()
    }
}

const AREA: Size<f32> = Size {
    width: 800.,
    height: 600.,
};

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

// ---- viewport ----------------------------------------------------------

#[test]
fn fit_centres_content_and_respects_zoom_limits() {
    // Content that fits at zoom 1 is not blown up and sits in the middle.
    let v = Viewport::fit(size(200., 100.), AREA, 32.);
    assert!(
        close(v.zoom, 1.),
        "small content stays at 1x, got {}",
        v.zoom
    );
    assert!(close(v.x, 300.) && close(v.y, 250.), "centred: {v:?}");

    // Wider than the area: scaled down to fit inside the padding, centred.
    let v = Viewport::fit(size(1472., 200.), AREA, 32.);
    assert!(close(v.zoom, 0.5), "fits the padded width, got {}", v.zoom);
    assert!(close(v.x, 32.), "left padding: {v:?}");
    assert!(close(v.y, (600. - 100.) / 2.), "vertically centred: {v:?}");

    // Far too tall: the zoom floors at 0.4 and the top (Start) stays in
    // view instead of being centred off-screen.
    let v = Viewport::fit(size(200., 10_000.), AREA, 32.);
    assert!(
        close(v.zoom, MIN_ZOOM),
        "clamped to the minimum: {}",
        v.zoom
    );
    assert!(close(v.y, 32.), "pinned to the top padding: {v:?}");
    assert!(
        close(v.x, (800. - 200. * MIN_ZOOM) / 2.),
        "still centred across"
    );

    // A degenerate area or empty content never yields NaN or a zero zoom.
    let v = Viewport::fit(size(0., 0.), size(0., 0.), 32.);
    assert!(v.zoom.is_finite() && v.x.is_finite() && v.y.is_finite());
    assert!((MIN_ZOOM..=MAX_ZOOM).contains(&v.zoom));
}

#[test]
fn zoom_at_keeps_the_pointer_fixed() {
    let mut v = Viewport {
        x: 40.,
        y: -20.,
        zoom: 1.,
    };
    let pointer = point(300., 200.);
    let under = v.to_flow(pointer);
    v.zoom_at(1.5, pointer);
    assert!(close(v.zoom, 1.5));
    let after = v.to_screen(under);
    assert!(close(after.x, pointer.x) && close(after.y, pointer.y));

    // Clamped at both ends, and the pointer still stays put.
    v.zoom_at(100., pointer);
    assert!(close(v.zoom, MAX_ZOOM));
    let after = v.to_screen(under);
    assert!(close(after.x, pointer.x) && close(after.y, pointer.y));
    v.zoom_at(0.001, pointer);
    assert!(close(v.zoom, MIN_ZOOM));
    let after = v.to_screen(under);
    assert!(close(after.x, pointer.x) && close(after.y, pointer.y));

    // to_flow is the inverse of to_screen.
    let p = v.to_screen((12.5, 99.));
    let back = v.to_flow(p);
    assert!(close(back.0, 12.5) && close(back.1, 99.));
}

// ---- hit testing -------------------------------------------------------

#[test]
fn hit_test_prefers_nodes_and_finds_edges_within_tolerance() {
    let d = small();
    let a = node(&d, "a");
    let inside = (a.x + 5., a.y + 5.);
    assert_eq!(hit_test(&d, inside, 8.), Some(Hit::Node("a".into())));

    // The a -> b edge starts on a's bottom border: a point there is both on
    // the node and on the edge, and the node wins.
    let ab = d
        .edges
        .iter()
        .position(|e| e.from == "a" && e.to == "b")
        .unwrap();
    let p0 = d.edges[ab].points[0];
    assert_eq!(
        hit_test(&d, (p0.0, p0.1 - 0.5), 8.),
        Some(Hit::Node("a".into()))
    );

    // The middle of the curve, nudged sideways within tolerance, is the edge.
    let mid = cubic_point(&d.edges[ab].points, 0.5);
    assert_eq!(hit_test(&d, (mid.0 + 5., mid.1), 8.), Some(Hit::Edge(ab)));
    // Outside the tolerance it is nothing.
    assert_eq!(hit_test(&d, (mid.0 + 20., mid.1), 8.), None);
    // The same offset is a hit when the tolerance is wider (zoomed out).
    assert_eq!(hit_test(&d, (mid.0 + 20., mid.1), 24.), Some(Hit::Edge(ab)));

    // Back-edges are polylines: the vertical lane of b -> a is hittable.
    let ba = d
        .edges
        .iter()
        .position(|e| e.from == "b" && e.to == "a")
        .unwrap();
    assert!(d.edges[ba].back);
    let lane = d.edges[ba].points[1];
    let lane_end = d.edges[ba].points[2];
    let on_lane = (lane.0 + 3., (lane.1 + lane_end.1) / 2.);
    assert_eq!(hit_test(&d, on_lane, 8.), Some(Hit::Edge(ba)));

    // Far away from everything.
    assert_eq!(hit_test(&d, (-500., -500.), 8.), None);
}

// ---- painting rules ----------------------------------------------------

#[test]
fn only_back_edges_and_implicit_failures_are_dashed() {
    let mut d = small();
    with_implicit_fail(&mut d, "b");
    let find = |from: &str, to: &str| {
        d.edges
            .iter()
            .find(|e| e.from == from && e.to == to)
            .unwrap()
    };
    assert!(!dashed(find(START_ID, "a")), "the Start arrow is solid");
    assert!(!dashed(find("a", "b")));
    assert!(!dashed(find("b", "$succeeded")));
    assert!(dashed(find("b", "a")), "a back-edge is dashed");
    assert!(
        dashed(find("b", "$failed")),
        "the implicit failure is dashed"
    );
}

/// a fails on a status guard and on `blocked is set`, then hands to b.
fn failing() -> Diagram {
    let guarded = |target: &str, when: Guard| Transition {
        to: target.into(),
        when: Some(when),
    };
    build(&flow(
        "a",
        vec![
            agent(
                "a",
                vec![
                    guarded(
                        FAILED,
                        Guard::Status {
                            is: VisitResult::Failed,
                        },
                    ),
                    guarded(
                        FAILED,
                        Guard::Output {
                            field: "blocked".into(),
                            op: GuardOp::Truthy,
                            value: None,
                        },
                    ),
                    to("b"),
                ],
            ),
            agent(
                "b",
                vec![
                    guarded(
                        FAILED,
                        Guard::Status {
                            is: VisitResult::Failed,
                        },
                    ),
                    to("$succeeded"),
                ],
            ),
        ],
    ))
}

#[test]
fn failure_edges_are_dashed_in_the_failed_tone() {
    let d = failing();
    let at = |from: &str, to: &str| {
        d.edges
            .iter()
            .position(|e| e.from == from && e.to == to)
            .unwrap()
    };
    let fail = &d.edges[at("a", FAILED)];
    assert!(fail.failure);
    let style = edge_style(fail, false, false);
    assert!(style.dashed, "a failure exit is dashed");
    assert_eq!(style.color, failure_tone(), "in the muted failed tone");
    assert!(style.color.a < 1., "muted, not the full failed colour");
    assert_eq!(style.color.h, node_color(NodeStatus::Failed).h);
    assert!(!is_curve(fail), "drawn as its polyline");

    // An ordinary forward edge stays solid and neutral.
    let plain = edge_style(&d.edges[at("a", "b")], false, false);
    assert!(!plain.dashed);
    assert_ne!(plain.color, failure_tone());
    // Taken or selected, a failure exit is highlighted but still dashed.
    for (taken, selected) in [(true, false), (false, true)] {
        let lit = edge_style(fail, taken, selected);
        assert_eq!(lit.color, accent_color());
        assert!(lit.dashed && lit.width > style.width);
    }
    // The implicit failure edge uses the same lane style.
    let mut with_implicit = small();
    with_implicit_fail(&mut with_implicit, "b");
    let implicit = with_implicit
        .edges
        .iter()
        .find(|e| e.implicit && e.to == FAILED)
        .unwrap();
    assert!(implicit.failure);
    assert_eq!(edge_style(implicit, false, false), style);

    // The shared lane is painted once, from the topmost stub to the pill.
    let pill = node(&d, FAILED);
    let a = node(&d, "a");
    let lane = failure_lane(&d).expect("a failure lane");
    assert_eq!(lane.0, (pill.x + pill.w / 2., a.y + a.h / 2.));
    assert_eq!(lane.1, (pill.x + pill.w / 2., pill.y));
    assert_eq!(failure_lane(&small()), None);
}

#[test]
fn hit_testing_finds_failure_edges() {
    let d = failing();
    let fails: Vec<usize> = d
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.from == "a" && e.to == FAILED)
        .map(|(i, _)| i)
        .collect();
    // a's two exits share their whole route; the second carries the label.
    let (plain, blocked) = (fails[0], fails[1]);
    assert_eq!(d.edges[plain].points, d.edges[blocked].points);
    assert_eq!(d.edges[blocked].label.as_deref(), Some("blocked is set"));
    let b_fail = d
        .edges
        .iter()
        .position(|e| e.from == "b" && e.to == FAILED)
        .unwrap();
    let (a, b) = (node(&d, "a"), node(&d, "b"));
    let lane = d.edges[plain].points.last().unwrap().0;

    // Clicking the label selects its edge, anywhere inside the label box.
    let (lx, ly) = d.edges[blocked].label_at;
    assert_eq!(hit_test(&d, (lx, ly), 8.), Some(Hit::Edge(blocked)));
    let (w, h) = label_size("blocked is set");
    let corner = (lx - w / 2. + 1., ly + h / 2. - 1.);
    assert_eq!(hit_test(&d, corner, 0.), Some(Hit::Edge(blocked)));
    // On the shared stub outside the label, the labelled edge wins the tie.
    let stub = (a.x - 3., a.y + a.h / 2. + 2.);
    assert_eq!(hit_test(&d, stub, 8.), Some(Hit::Edge(blocked)));
    // On b's stub, its own exit.
    let stub = ((b.x + lane) / 2., b.y + b.h / 2.);
    assert_eq!(hit_test(&d, stub, 8.), Some(Hit::Edge(b_fail)));
    // On the lane between a and b only a's exits run: the labelled one.
    let between = (lane + 3., (a.y + a.h / 2. + b.y + b.h / 2.) / 2.);
    assert_eq!(hit_test(&d, between, 8.), Some(Hit::Edge(blocked)));
    // Left of the lane, out of tolerance, is nothing.
    assert_eq!(hit_test(&d, (lane - 20., b.y + b.h), 8.), None);
    // A node still wins over everything.
    assert_eq!(
        hit_test(&d, (a.x + 5., a.y + 5.), 8.),
        Some(Hit::Node("a".into()))
    );
}

#[test]
fn taken_edges_are_matched_by_their_index_among_parallel_edges() {
    // Two parallel a -> b edges; the run took the second one.
    let d = build(&flow(
        "a",
        vec![agent("a", vec![to("b"), to("b")]), agent("b", vec![])],
    ));
    let overlay = Overlay {
        taken: [("a".to_string(), "b".to_string(), 1)].into(),
        ..Overlay::default()
    };
    let parallel: Vec<usize> = d
        .edges
        .iter()
        .enumerate()
        .filter(|(_, e)| e.from == "a" && e.to == "b")
        .map(|(i, _)| i)
        .collect();
    assert_eq!(parallel.len(), 2);
    let taken = taken_edges(&d, Some(&overlay));
    assert!(!taken[parallel[0]]);
    assert!(taken[parallel[1]]);
    // With no run, nothing is taken.
    assert!(taken_edges(&d, None).iter().all(|t| !t));
}

#[test]
fn cancelled_nodes_are_dashed_and_set_apart_from_not_reached() {
    use gpui::BorderStyle::{Dashed, Solid};
    let (idle, idle_w, idle_style) = node_border(NodeStatus::NotReached);
    let (cancelled, cancelled_w, cancelled_style) = node_border(NodeStatus::Cancelled);
    assert_eq!(idle_style, Solid);
    assert_eq!(
        cancelled_style, Dashed,
        "Cancelled reads as stopped, not unvisited"
    );
    assert!(
        cancelled_w > idle_w,
        "a reached state has the heavier border"
    );
    assert!(
        cancelled.a > idle.a,
        "the muted tone is still darker than Not reached"
    );
    // It stays a muted ink, not one of the run colours.
    for status in [
        NodeStatus::Running,
        NodeStatus::AwaitingApproval,
        NodeStatus::Completed,
        NodeStatus::Failed,
    ] {
        let (tone, width, style) = node_border(status);
        assert_eq!(style, Solid, "{status:?}");
        assert_eq!(tone, node_color(status), "{status:?}");
        assert_eq!(width, cancelled_w);
        assert_ne!(tone, cancelled);
    }
}

#[test]
fn titles_wrap_to_two_lines_then_ellipsis() {
    // A fake monospace font: 10 px per character.
    let measure = |s: &str| s.chars().count() as f32 * 10.;
    assert_eq!(wrap_title("Short", 100., measure), vec!["Short"]);
    // A third line's worth of text is cut on the second line with `…`.
    let cut = wrap_title("Review the pull request today", 100., measure);
    assert_eq!(cut, vec!["Review the", "pull requ…"]);
    assert!(cut.iter().all(|l| measure(l) <= 100.));
    // Exactly two lines fit: no ellipsis.
    assert_eq!(
        wrap_title("Open the pull", 100., measure),
        vec!["Open the", "pull"]
    );
    // A single word wider than the line is cut on its own line.
    let long = wrap_title("Supercalifragilistic", 100., measure);
    assert_eq!(long, vec!["Supercali…"]);
    assert!(long.iter().all(|l| measure(l) <= 100.));
    assert!(wrap_title("", 100., measure).is_empty());
}

#[test]
fn keyboard_focus_moves_along_edges() {
    // a -> {b, c}; b -> d; c -> d; d -> $succeeded.
    let d = build(&flow(
        "a",
        vec![
            agent("a", vec![to("b"), to("c")]),
            agent("b", vec![to("d")]),
            agent("c", vec![to("d")]),
            agent("d", vec![to("$succeeded")]),
        ],
    ));
    // Nothing focused: any move lands on the initial state.
    assert_eq!(step_focus(&d, None, Nav::Next).as_deref(), Some("a"));
    assert_eq!(step_focus(&d, None, Nav::Previous).as_deref(), Some("a"));
    // Forward along the first outgoing edge, back along the first incoming.
    assert_eq!(step_focus(&d, Some("a"), Nav::Next).as_deref(), Some("b"));
    assert_eq!(step_focus(&d, Some("b"), Nav::Next).as_deref(), Some("d"));
    assert_eq!(
        step_focus(&d, Some("d"), Nav::Previous).as_deref(),
        Some("b")
    );
    // Terminal pills and Start are never focused: no move past the end.
    assert_eq!(step_focus(&d, Some("d"), Nav::Next), None);
    assert_eq!(step_focus(&d, Some("a"), Nav::Previous), None);
    // Left / right walk the siblings on a rank.
    let (left, right) = if node(&d, "b").order < node(&d, "c").order {
        ("b", "c")
    } else {
        ("c", "b")
    };
    assert_eq!(
        step_focus(&d, Some(left), Nav::Right).as_deref(),
        Some(right)
    );
    assert_eq!(
        step_focus(&d, Some(right), Nav::Left).as_deref(),
        Some(left)
    );
    assert_eq!(step_focus(&d, Some(right), Nav::Right), None);
}

// ---- the view ----------------------------------------------------------

fn view(cx: &mut TestAppContext) -> Entity<DiagramView> {
    cx.new(|cx| {
        let mut view = DiagramView::new(cx);
        view.resize(point(0., 0.), AREA);
        view
    })
}

#[gpui::test]
fn status_update_keeps_the_viewport(cx: &mut TestAppContext) {
    let view = view(cx);
    view.update(cx, |v, cx| {
        v.set(small(), Some(running_at("a")), HashSet::new(), 1, cx);
        assert!(v.following);
        let fitted = v.viewport;
        assert_eq!(
            fitted,
            Viewport::fit(size(v.diagram.width, v.diagram.height), AREA, FIT_PADDING)
        );

        // The user pans away; the next status-only update (same structure,
        // new current state, fresh Diagram) must not move or refit.
        v.pan_by(-120., 45.);
        let panned = v.viewport;
        assert_ne!(panned, fitted);
        assert!(!v.following);
        v.set(small(), Some(running_at("b")), HashSet::new(), 1, cx);
        assert_eq!(v.viewport, panned);
        assert!(!v.following, "following stays off after a manual pan");

        // Re-centre turns following back on and brings the view home.
        v.recentre(cx);
        assert!(v.following);
        assert_eq!(v.viewport, fitted);
    });
}

#[gpui::test]
fn structure_change_refits(cx: &mut TestAppContext) {
    let view = view(cx);
    view.update(cx, |v, cx| {
        v.set(small(), None, HashSet::new(), 1, cx);
        v.pan_by(300., 300.);
        v.zoom_at(1.7, point(10., 10.));
        let bigger = chain(6);
        let expected = Viewport::fit(size(bigger.width, bigger.height), AREA, FIT_PADDING);
        v.set(bigger, None, HashSet::new(), 2, cx);
        assert_eq!(v.viewport, expected);
        assert!(v.following, "a refit is a re-centre");
    });
}

#[gpui::test]
fn following_pans_a_new_current_state_into_view(cx: &mut TestAppContext) {
    let view = view(cx);
    view.update(cx, |v, cx| {
        // Twenty steps are too tall for the area even at the minimum zoom.
        v.set(chain(20), Some(running_at("s0")), HashSet::new(), 7, cx);
        let fitted = v.viewport;
        let last = node(&v.diagram, "s19").clone();
        assert!(v.viewport.to_screen((last.x, last.y)).y > AREA.height);

        v.set(chain(20), Some(running_at("s19")), HashSet::new(), 7, cx);
        assert_ne!(v.viewport, fitted);
        let top = v.viewport.to_screen((last.x, last.y));
        let bottom = v.viewport.to_screen((last.x + last.w, last.y + last.h));
        assert!(top.y >= 0. && bottom.y <= AREA.height, "s19 is on screen");
        assert!(top.x >= 0. && bottom.x <= AREA.width);
        assert!(v.following);
    });
}

#[gpui::test]
fn selection_emits_events_and_is_announced(cx: &mut TestAppContext) {
    let view = view(cx);
    let events: Rc<RefCell<Vec<DiagramEvent>>> = Rc::default();
    let sink = events.clone();
    cx.update(|cx| {
        cx.subscribe(&view, move |_, event: &DiagramEvent, _| {
            sink.borrow_mut().push(event.clone())
        })
        .detach()
    });
    view.update(cx, |v, cx| {
        v.set(small(), Some(running_at("a")), HashSet::new(), 1, cx);
        assert_eq!(v.selected(), None);
        assert_eq!(v.aria_label(), "Flow diagram, 2 steps");
        v.select(Some("a".into()), cx);
        assert_eq!(v.selected(), Some("a"));
        assert_eq!(v.aria_label(), "Title a, running");
        // Start and terminal pills cannot be selected.
        v.select(Some(START_ID.into()), cx);
        assert_eq!(v.selected(), Some("a"));
        v.open_selected(cx);
        // A node that disappears from the Flow drops the selection.
        let only_b = build(&flow("b", vec![agent("b", vec![])]));
        v.set(only_b, None, HashSet::new(), 2, cx);
        assert_eq!(v.selected(), None);
    });
    cx.run_until_parked();
    assert_eq!(
        *events.borrow(),
        vec![
            DiagramEvent::Selected(Some("a".into())),
            DiagramEvent::Open("a".into()),
            DiagramEvent::Selected(None),
        ]
    );
}

#[gpui::test]
fn implicit_failure_is_drawn_from_the_current_state(cx: &mut TestAppContext) {
    let view = view(cx);
    view.update(cx, |v, cx| {
        let overlay = Overlay {
            nodes: [("a".to_string(), (NodeStatus::Failed, 1))].into(),
            current: Some("a".into()),
            ended_at: Some(FAILED.into()),
            implicit_fail: true,
            ..Overlay::default()
        };
        // `small` has no Failed pill; the view adds it and the edge.
        v.set(small(), Some(overlay), HashSet::new(), 1, cx);
        assert!(v.diagram.nodes.iter().any(|n| n.id == FAILED));
        let edge = v
            .diagram
            .edges
            .iter()
            .find(|e| e.from == "a" && e.to == FAILED)
            .expect("implicit edge");
        assert!(edge.implicit && dashed(edge));
    });
}

#[gpui::test]
fn a_new_implicit_failed_pill_refits_the_view(cx: &mut TestAppContext) {
    let view = view(cx);
    let failed_at = |id: &str| Overlay {
        nodes: [(id.to_string(), (NodeStatus::Failed, 1))].into(),
        current: Some(id.to_string()),
        ended_at: Some(FAILED.into()),
        implicit_fail: true,
        ..Overlay::default()
    };
    view.update(cx, |v, cx| {
        v.set(chain(3), Some(running_at("s2")), HashSet::new(), 7, cx);
        // The user zooms in and pans away from the bottom of the Flow.
        v.zoom_at(1.7, point(10., 10.));
        v.pan_by(0., 300.);
        assert!(!v.following);
        let panned = v.viewport;

        // The run fails: the Failed pill is added under the deepest node.
        // Same structure_key, but the drawing grew, so it refits.
        v.set(chain(3), Some(failed_at("s2")), HashSet::new(), 7, cx);
        assert!(v.diagram.nodes.iter().any(|n| n.id == FAILED));
        assert_ne!(v.viewport, panned);
        assert_eq!(
            v.viewport,
            Viewport::fit(size(v.diagram.width, v.diagram.height), AREA, FIT_PADDING)
        );
        assert!(v.following, "a refit is a re-centre");
        let pill = node(&v.diagram, FAILED).clone();
        let top = v.viewport.to_screen((pill.x, pill.y));
        let bottom = v.viewport.to_screen((pill.x + pill.w, pill.y + pill.h));
        assert!(
            top.y >= 0. && bottom.y <= AREA.height,
            "the pill is on screen"
        );

        // Further status updates with the pill keep the viewport.
        v.pan_by(0., -30.);
        let panned = v.viewport;
        v.set(chain(3), Some(failed_at("s2")), HashSet::new(), 7, cx);
        assert_eq!(v.viewport, panned);
    });
}

#[gpui::test]
fn changed_badges_expire_after_ten_seconds(cx: &mut TestAppContext) {
    let view = view(cx);
    view.update(cx, |v, cx| {
        v.set(small(), None, ["b".to_string()].into(), 1, cx);
        assert_eq!(v.badged(cx), vec!["b".to_string()]);
    });
    cx.executor()
        .advance_clock(CHANGED_FOR - Duration::from_millis(1));
    cx.run_until_parked();
    view.update(cx, |v, cx| assert_eq!(v.badged(cx), vec!["b".to_string()]));
    cx.executor().advance_clock(Duration::from_millis(2));
    cx.run_until_parked();
    view.update(cx, |v, cx| {
        assert!(v.badged(cx).is_empty());
        assert!(v.changed.is_empty(), "expired badges are dropped");
    });
}

#[gpui::test]
fn render_paints_without_panicking(cx: &mut TestAppContext) {
    let (view, cx) = cx.add_window_view(|_, cx| DiagramView::new(cx));
    view.update(cx, |v, cx| {
        let mut overlay = running_at("b");
        overlay.nodes.insert("a".into(), (NodeStatus::Completed, 3));
        overlay.taken.insert(("a".into(), "b".into(), 0));
        overlay.taken.insert(("b".into(), "a".into(), 0));
        v.set(small(), Some(overlay), ["a".to_string()].into(), 1, cx);
        v.select(Some("b".into()), cx);
    });
    cx.run_until_parked();
    // The canvas measured itself and fitted the diagram during prepaint.
    view.update(cx, |v, _| {
        assert!(v.area.is_some());
        assert!(!v.needs_fit);
    });
    // Four nodes, the selection ring, the visit badge and the changed badge.
    let quads = cx.update(|window, _| window.painted_quads());
    assert!(quads.len() >= 7, "painted {} quads", quads.len());
    assert!(
        quads.iter().any(|q| q.border_color == accent_color()),
        "the selected state wears an accent ring"
    );
    // A failed run with an implicit edge, and a manual pan (Re-centre shown).
    view.update(cx, |v, cx| {
        let overlay = Overlay {
            nodes: [("a".to_string(), (NodeStatus::Failed, 1))].into(),
            current: Some("a".into()),
            ended_at: Some(FAILED.into()),
            implicit_fail: true,
            ..Overlay::default()
        };
        v.set(small(), Some(overlay), HashSet::new(), 1, cx);
        v.pan_by(10., 10.);
        cx.notify();
    });
    cx.run_until_parked();
    // An empty diagram paints too.
    view.update(cx, |v, cx| {
        v.set(build(&flow("missing", vec![])), None, HashSet::new(), 3, cx)
    });
    cx.run_until_parked();
}
