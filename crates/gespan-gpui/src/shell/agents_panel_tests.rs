//! The sidebar's Agents footer and panel: what it counts, the order it
//! lists agents in, and the six-row height rule, in a real shell.
use super::agents_panel::*;
use super::*;
use gespan_core::store::Store;
use gpui::{TestAppContext, VisualTestContext, WindowHandle};

const CWD: &str = "/work/gespan";

fn agent(id: i64, state: AgentState, started_at: Option<u64>, updated_at: u64) -> Agent {
    Agent {
        id,
        pilot: None,
        title: format!("Agent {id}"),
        provider: "claude".into(),
        origin: "Claude Code".into(),
        cwd: CWD.into(),
        state,
        asks: false,
        started_at,
        updated_at,
    }
}

fn counts(needs_you: usize, running: usize, idle: usize) -> Counts {
    Counts {
        needs_you,
        running,
        idle,
    }
}

#[test]
fn agents_list_needs_you_then_running_then_idle() {
    let mut agents = vec![
        agent(1, AgentState::Idle, None, 100),
        agent(2, AgentState::Running, Some(300), 0),
        agent(3, AgentState::NeedsYou, Some(500), 0),
        agent(4, AgentState::Idle, None, 900),
        agent(5, AgentState::Running, Some(100), 0),
        agent(6, AgentState::NeedsYou, Some(200), 0),
        // A turn whose start is unknown runs after the timed ones.
        agent(7, AgentState::Running, None, 0),
    ];
    sort_agents(&mut agents);
    let order: Vec<i64> = agents.iter().map(|a| a.id).collect();
    // Waiting and running: longest first. Idle: most recent first.
    assert_eq!(order, vec![6, 3, 5, 2, 7, 4, 1]);
}

#[test]
fn footer_and_header_counts_read_for_each_case() {
    // Nothing running: a grey dot and the idle count.
    assert_eq!(counts(0, 0, 4).footer_text(), "4 agents idle");
    assert_eq!(counts(0, 0, 1).footer_text(), "1 agent idle");
    assert_eq!(counts(0, 0, 0).footer_text(), "No agents");
    assert_eq!(counts(0, 0, 4).dot(), Dot::Idle);
    // Some running: blue, with idle after.
    assert_eq!(counts(0, 2, 2).footer_text(), "2 running · 2 idle");
    assert_eq!(counts(0, 3, 0).footer_text(), "3 running");
    assert_eq!(counts(0, 1, 0).dot(), Dot::Running);
    // One needs you: yellow, and the waiting count replaces idle.
    assert_eq!(counts(1, 7, 12).footer_text(), "7 running · 1 needs you");
    assert_eq!(counts(1, 0, 3).footer_text(), "1 needs you");
    assert_eq!(counts(1, 7, 12).dot(), Dot::Wait);
    assert_eq!(
        counts(1, 7, 12).footer_parts(),
        vec![
            ("7 running".to_owned(), Tone::Strong),
            ("1 needs you".to_owned(), Tone::Wait),
        ]
    );
    // The open panel's header names every group that has agents.
    assert_eq!(
        counts(1, 7, 12).header_text(),
        "7 running · 1 needs you · 12 idle"
    );
    assert_eq!(counts(0, 2, 2).header_text(), "2 running · 2 idle");
    assert_eq!(counts(0, 0, 0).header_text(), "Nothing open");
    let agents = [
        agent(1, AgentState::NeedsYou, None, 0),
        agent(2, AgentState::Running, None, 0),
        agent(3, AgentState::Running, None, 0),
        agent(4, AgentState::Idle, None, 0),
    ];
    assert_eq!(Counts::of(&agents), counts(1, 2, 1));
}

#[test]
fn the_list_hugs_up_to_six_agents_then_holds_six_rows_and_scrolls() {
    for rows in 0..=LIST_ROWS {
        assert_eq!(list_height(rows), None, "{rows} agents hug their rows");
    }
    for rows in [LIST_ROWS + 1, 20, 200] {
        assert_eq!(list_height(rows), Some(LIST_HEIGHT), "{rows} agents scroll");
    }
}

#[test]
fn running_time_reads_seconds_minutes_and_hours() {
    assert_eq!(elapsed(0), "1s");
    assert_eq!(elapsed(45_900), "45s");
    assert_eq!(elapsed(59_999), "59s");
    assert_eq!(elapsed(60_000), "1m");
    assert_eq!(elapsed(6 * 60_000 + 30_000), "6m");
    assert_eq!(elapsed(59 * 60_000 + 59_000), "59m");
    assert_eq!(elapsed(3_600_000), "1h");
    assert_eq!(elapsed(3_600_000 + 5 * 60_000), "1h 5m");
}

/// A shell with `n` chats that have talked to an agent, all open.
fn shell_with(n: usize, cx: &mut TestAppContext) -> (WindowHandle<Shell>, Vec<i64>) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let store = Store::open(std::path::Path::new(":memory:")).unwrap();
    let mut summaries = Vec::new();
    for ix in 0..n {
        let mut summary = store
            .create(if ix % 2 == 0 { "claude" } else { "codex" }, CWD)
            .unwrap();
        summary.title = format!("Agent chat {ix}");
        summary.has_messages = true;
        summary.updated_at = 1_700_000_000_000 + ix as u64;
        summaries.push(summary);
    }
    let ids: Vec<i64> = summaries.iter().map(|s| s.id).collect();
    let store = Arc::new(Mutex::new(store));
    cx.update(crate::shell::bind_keys);
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut shell = Shell::new(store, summaries, CWD.into(), None, cx);
                for &id in &ids {
                    shell.ensure_session(id, cx);
                }
                shell
            })
        })
        .unwrap()
    });
    cx.simulate_window_resize(window.into(), gpui::size(px(1400.), px(900.)));
    cx.run_until_parked();
    (window, ids)
}

/// Starts or ends a turn the way a chat does: through `SessionChanged`,
/// with no manual repaint, so the footer must follow the event alone.
fn set_busy(window: WindowHandle<Shell>, id: i64, busy: bool, cx: &mut VisualTestContext) {
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, cx| {
                view.busy = busy;
                cx.emit(SessionChanged);
            })
        })
        .unwrap();
    cx.run_until_parked();
}

fn needs_you(window: WindowHandle<Shell>, id: i64, cx: &mut VisualTestContext) {
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&id].update(cx, |view, cx| {
                view.busy = true;
                view.set_approvals_for_test(vec![gespan_core::agent::Approval {
                    id: "a1".into(),
                    title: "Run a command?".into(),
                    detail: "git status".into(),
                    can_remember: false,
                    tool: None,
                }]);
                cx.emit(SessionChanged);
            })
        })
        .unwrap();
    cx.run_until_parked();
}

fn footer_text(window: WindowHandle<Shell>, cx: &mut VisualTestContext) -> String {
    window
        .update(cx, |shell, _, cx| {
            Counts::of(&shell.open_agents(cx)).footer_text()
        })
        .unwrap()
}

fn click(selector: &'static str, cx: &mut VisualTestContext) {
    let bounds = cx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} is drawn"));
    cx.simulate_click(bounds.center(), Default::default());
    cx.run_until_parked();
}

fn open_panel(cx: &mut VisualTestContext) {
    click("agents-footer", cx);
    assert!(
        cx.debug_bounds("agents-panel").is_some(),
        "the panel opened"
    );
}

#[gpui::test]
fn a_single_running_agent_shows_in_the_footer(cx: &mut TestAppContext) {
    // The old card needed two agents before it showed at all.
    let (window, ids) = shell_with(2, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    assert!(visual.debug_bounds("agents-footer").is_some());
    assert_eq!(footer_text(window, visual), "2 agents idle");
    set_busy(window, ids[0], true, visual);
    assert_eq!(footer_text(window, visual), "1 running · 1 idle");
    open_panel(visual);
    assert!(visual.debug_bounds("agents-group-running").is_some());
    assert!(
        visual
            .debug_bounds(Box::leak(format!("agents-row-{}", ids[0]).into_boxed_str()))
            .is_some(),
        "the one running agent is listed"
    );
    // The turn ends: the same agent moves to idle, and nothing is dropped.
    set_busy(window, ids[0], false, visual);
    assert_eq!(footer_text(window, visual), "2 agents idle");
    assert!(visual.debug_bounds("agents-group-running").is_none());
    assert!(visual.debug_bounds("agents-group-idle").is_some());
}

#[gpui::test]
fn every_agent_is_listed_past_the_old_cap_in_needs_you_running_idle_order(cx: &mut TestAppContext) {
    let (window, ids) = shell_with(20, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    for &id in &ids[1..8] {
        set_busy(window, id, true, visual);
    }
    needs_you(window, ids[12], visual);
    assert_eq!(footer_text(window, visual), "7 running · 1 needs you");
    let listed = window
        .update(visual, |shell, _, cx| shell.open_agents(cx))
        .unwrap();
    assert_eq!(listed.len(), 20, "no agent is dropped");
    assert_eq!(listed[0].id, ids[12]);
    assert_eq!(listed[0].state, AgentState::NeedsYou);
    assert!(listed[1..8].iter().all(|a| a.state == AgentState::Running));
    assert!(listed[8..].iter().all(|a| a.state == AgentState::Idle));
    open_panel(visual);
    let top = |selector: &'static str, visual: &mut VisualTestContext| {
        visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector}"))
            .top()
    };
    let (waiting, running, idle) = (
        top("agents-group-needs-you", visual),
        top("agents-group-running", visual),
        top("agents-group-idle", visual),
    );
    assert!(waiting < running && running < idle);
    // The waiting row comes first and says what it waits for.
    let first = visual
        .debug_bounds(Box::leak(
            format!("agents-row-{}", ids[12]).into_boxed_str(),
        ))
        .unwrap();
    assert!(first.top() > waiting && first.top() < running);
}

#[gpui::test]
fn the_panel_hugs_a_few_agents_and_scrolls_inside_six_rows_for_many(cx: &mut TestAppContext) {
    let (window, ids) = shell_with(4, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    set_busy(window, ids[0], true, visual);
    open_panel(visual);
    let list = visual.debug_bounds("agents-list").unwrap();
    assert!(
        list.size.height < px(LIST_HEIGHT),
        "four agents hug their rows: {:?}",
        list.size.height
    );
    let last = visual
        .debug_bounds(Box::leak(format!("agents-row-{}", ids[3]).into_boxed_str()))
        .unwrap();
    assert!(
        last.bottom() <= list.bottom(),
        "the last row fits without scrolling"
    );

    let (window, ids) = shell_with(20, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    for &id in &ids[..7] {
        set_busy(window, id, true, visual);
    }
    open_panel(visual);
    let panel = visual.debug_bounds("agents-panel").unwrap();
    let list = visual.debug_bounds("agents-list").unwrap();
    let footer = visual.debug_bounds("agents-footer").unwrap();
    assert_eq!(list.size.height, px(LIST_HEIGHT), "the list holds six rows");
    // Header and the panel's own footer stay inside the panel, which sits
    // above the sidebar footer instead of pushing it.
    assert!(panel.top() < list.top() && list.bottom() < panel.bottom());
    assert!(
        visual
            .debug_bounds("agents-header-counts")
            .unwrap()
            .bottom()
            <= list.top()
    );
    let hold = visual.debug_bounds("agents-hold-all").unwrap();
    assert!(hold.top() >= list.bottom() && hold.bottom() <= panel.bottom());
    assert!(panel.bottom() < footer.top());
    // Scrolling the list moves its rows, not the header or the actions.
    let row = Box::leak(format!("agents-row-{}", ids[0]).into_boxed_str());
    let before = visual.debug_bounds(row).unwrap().top();
    visual.simulate_event(gpui::ScrollWheelEvent {
        position: list.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.), px(-200.))),
        ..Default::default()
    });
    visual.run_until_parked();
    let after = visual.debug_bounds(row).map(|b| b.top());
    assert!(after.is_none_or(|top| top < before), "the list scrolled");
    assert_eq!(visual.debug_bounds("agents-list").unwrap(), list);
    assert_eq!(visual.debug_bounds("agents-hold-all").unwrap(), hold);
}

#[gpui::test]
fn the_footer_toggles_the_panel_and_escape_closes_it(cx: &mut TestAppContext) {
    let (window, ids) = shell_with(3, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    set_busy(window, ids[1], true, visual);
    let footer = visual.debug_bounds("agents-footer").unwrap();
    open_panel(visual);
    assert_eq!(
        visual.debug_bounds("agents-footer").unwrap(),
        footer,
        "the panel overlays the list; the footer stays put"
    );
    click("agents-footer", visual);
    assert!(visual.debug_bounds("agents-panel").is_none());
    open_panel(visual);
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(visual.debug_bounds("agents-panel").is_none());
    assert!(
        !window
            .update(visual, |shell, _, _| shell.agents_open)
            .unwrap()
    );
}

#[gpui::test]
fn clicking_an_agent_opens_its_chat_and_closes_the_panel(cx: &mut TestAppContext) {
    let (window, ids) = shell_with(3, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    set_busy(window, ids[2], true, visual);
    open_panel(visual);
    click(
        Box::leak(format!("agents-row-{}", ids[2]).into_boxed_str()),
        visual,
    );
    window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.workspace.active, Some(ids[2]));
            assert!(!shell.agents_open);
        })
        .unwrap();
    assert!(visual.debug_bounds("agents-panel").is_none());
}

#[gpui::test]
fn hold_all_in_the_panel_asks_for_a_second_click_then_holds(cx: &mut TestAppContext) {
    let (window, ids) = shell_with(2, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    set_busy(window, ids[0], true, visual);
    open_panel(visual);
    click("agents-hold-all", visual);
    assert_eq!(visual.update(|_, cx| crate::hold::phase(cx)), None);
    click("agents-hold-all", visual);
    assert!(
        visual.update(|_, cx| crate::hold::phase(cx)).is_some(),
        "the second click holds everything"
    );
}

#[gpui::test]
fn turning_activity_off_hides_the_footer_and_closes_the_panel(cx: &mut TestAppContext) {
    let (window, ids) = shell_with(2, cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    set_busy(window, ids[0], true, visual);
    open_panel(visual);
    window
        .update(visual, |shell, _, cx| {
            shell.workspace.preferences.working_agents = false;
            shell.invalidate_chrome(cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert!(visual.debug_bounds("agents-footer").is_none());
    assert!(visual.debug_bounds("agents-panel").is_none());
    assert!(
        !window
            .update(visual, |shell, _, _| shell.agents_open)
            .unwrap()
    );
}
