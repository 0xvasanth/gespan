//! Web links in a thread open in the thread's in-app browser: a new tab of
//! the user's own, shown beside the thread, whether or not agents may use
//! the browser; Cmd+click (Ctrl+click off macOS) keeps the system browser.
use super::*;
use crate::{browser_test_rig::Rig, review::ReviewTab};
use gespan_browser::protocol::{ActionRequest, ActionResult, Outcome, Reason};
use gespan_core::browser::{
    BrowserHost, BrowserMode, BrowserSettings, TabId, TabState, session_key, turn_carries,
};
use gespan_core::store::Store;
use gpui::{Modifiers, TestAppContext, VisualTestContext, WindowHandle};
use serde_json::json;

const URL: &str = "https://example.com/docs";

/// A thread whose answer links to [`URL`] beside inline code, so the link
/// is its own fragment (`rich-prose-3-0`) a click can land on.
pub(super) fn thread_with_link(
    host: Arc<BrowserHost>,
    cx: &mut TestAppContext,
) -> (WindowHandle<Shell>, i64) {
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        crate::browser::install_host(host, cx);
    });
    let mut store = Store::open(std::path::Path::new(":memory:")).unwrap();
    let mut session = store.create("claude", "/work/gespan").unwrap();
    store
        .write_rows(
            session.id,
            0,
            &[gespan_core::transcript::Row {
                role: "assistant".into(),
                text: format!("See `main.rs` and [the docs]({URL})"),
                first: true,
                sent_at: None,
            }],
        )
        .unwrap();
    session.has_messages = true;
    let id = session.id;
    let mut state = Workspace {
        new_layout_boxy: false,
        ..Default::default()
    };
    state.add_project("/work/gespan");
    state.open(id);
    let saved = serde_json::to_string(&state).unwrap();
    let store = Arc::new(Mutex::new(store));
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut shell =
                    Shell::new(store, vec![session], "/work/gespan".into(), Some(saved), cx);
                shell.ensure_session(id, cx);
                shell
            })
        })
        .unwrap()
    });
    window
        .update(cx, |shell, w, cx| shell.open(id, w, cx))
        .unwrap();
    settle(window, cx);
    (window, id)
}

pub(super) fn settle(window: WindowHandle<Shell>, cx: &mut TestAppContext) {
    for _ in 0..3 {
        cx.run_until_parked();
        window.update(cx, |_, w, _| w.refresh()).unwrap();
    }
    cx.run_until_parked();
}

/// Click the link in the transcript, with `modifiers` held.
fn click_link(window: WindowHandle<Shell>, modifiers: Modifiers, cx: &mut TestAppContext) {
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let link = visual
        .debug_bounds("rich-prose-3-0")
        .expect("the link is drawn in the transcript");
    visual.simulate_click(link.center(), modifiers);
    settle(window, cx);
}

fn browser_tabs(shell: &Shell, session: i64, cx: &App) -> Vec<ReviewTab> {
    shell.reviews.get(&session).map_or_else(Vec::new, |review| {
        review
            .read(cx)
            .state
            .tabs
            .iter()
            .filter(|tab| matches!(tab, ReviewTab::Browser { .. }))
            .cloned()
            .collect()
    })
}

/// The thread's tabs in the host, with whether each is the user's own.
pub(super) fn host_tabs(host: &BrowserHost, session: i64) -> Vec<(TabId, bool, TabState)> {
    let key = session_key(session);
    host.state()
        .tabs
        .into_iter()
        .filter(|tab| tab.session_id == key)
        .map(|tab| (tab.tab.tab, tab.user_tab, tab.tab.state))
        .collect()
}

pub(super) fn navigated_to(rig: &Rig, url: &str) -> bool {
    rig.received("Page.navigate", |params| params["url"] == url)
}

fn agent_call(host: &Arc<BrowserHost>, session: i64, id: &str) -> ActionResult {
    host.handle_request(
        &session_key(session),
        ActionRequest {
            request_id: id.into(),
            action: "tabs.list".into(),
            input: json!({}),
        },
    )
}

#[gpui::test]
fn a_link_clicked_in_the_transcript_opens_the_users_tab_beside_the_thread(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    click_link(window, Modifiers::none(), cx);

    let tabs = host_tabs(&rig.host, session);
    assert_eq!(tabs.len(), 1, "{tabs:?}");
    let (tab, user_tab, state) = tabs[0].clone();
    assert!(user_tab, "the user's own tab, not an agent's");
    assert_eq!(state, TabState::Open);
    assert!(navigated_to(&rig, URL));
    assert_eq!(cx.opened_url(), None, "not the system browser");
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(
                browser_tabs(shell, session, cx),
                [ReviewTab::Browser {
                    session,
                    tab: tab.0.clone(),
                    own: true,
                }]
            );
            assert!(
                shell.reviews[&session]
                    .read(cx)
                    .browser_pane_for_tests(&tab.0)
                    .is_some(),
                "its live pane shows beside the thread"
            );
        })
        .unwrap();
}

#[gpui::test]
fn cmd_or_ctrl_click_opens_the_system_browser(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    click_link(window, Modifiers::secondary_key(), cx);
    assert_eq!(cx.opened_url().as_deref(), Some(URL));
    assert!(host_tabs(&rig.host, session).is_empty());
    window
        .update(cx, |shell, _, cx| {
            assert!(browser_tabs(shell, session, cx).is_empty());
        })
        .unwrap();
}

/// The switch is about agents: off, the user's link still opens in the
/// Gespan browser (started for it), and the thread's agent still gets no
/// browser.
#[gpui::test]
fn with_the_switch_off_the_link_opens_and_agents_still_get_nothing(cx: &mut TestAppContext) {
    let rig = Rig::start_managed();
    assert!(!rig.host.state().toggle_on);
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    click_link(window, Modifiers::none(), cx);

    let tabs = host_tabs(&rig.host, session);
    assert!(
        matches!(tabs.as_slice(), [(_, true, TabState::Open)]),
        "{tabs:?}"
    );
    assert!(!rig.host.state().toggle_on, "the switch stays off");
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(browser_tabs(shell, session, cx).len(), 1);
        })
        .unwrap();
    // No turn carries the browser (so the toolkit lists no tools) and an
    // agent call is refused.
    rig.host.turn_started(
        &session_key(session),
        gespan_core::browser::WINDOW,
        "/work/gespan",
        Some(true),
    );
    assert!(!turn_carries(&rig.host.state(), session));
    let refused = agent_call(&rig.host, session, "r1");
    assert_eq!(refused.reason, Some(Reason::Off), "{refused:?}");
}

/// Turning the switch off ends the agent's tab and its pane, but the
/// user's own tab beside the thread keeps showing the page.
#[gpui::test]
fn switching_off_keeps_the_users_tab_live_and_ends_the_agents(cx: &mut TestAppContext) {
    let rig = Rig::start_managed();
    rig.host.set_toggle(true);
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    click_link(window, Modifiers::none(), cx);
    let own = host_tabs(&rig.host, session)[0].0.clone();
    rig.host.turn_started(
        &session_key(session),
        gespan_core::browser::WINDOW,
        "/work/gespan",
        Some(true),
    );
    let opened = rig.host.handle_request(
        &session_key(session),
        ActionRequest {
            request_id: "r1".into(),
            action: "tabs.open".into(),
            input: json!({}),
        },
    );
    assert_eq!(opened.outcome, Outcome::Done, "{opened:?}");
    let agent = host_tabs(&rig.host, session)
        .into_iter()
        .find(|(tab, user, _)| !user && *tab != own)
        .expect("the agent's tab")
        .0;
    window
        .update(cx, |shell, _, cx| {
            shell.sessions[&session]
                .update(cx, |chat, cx| chat.show_browser_tab(agent.0.clone(), cx));
        })
        .unwrap();
    settle(window, cx);

    rig.host.set_toggle(false);
    cx.update(crate::browser::refresh);
    settle(window, cx);
    let states: Vec<_> = host_tabs(&rig.host, session)
        .into_iter()
        .map(|(tab, _, state)| (tab, state))
        .collect();
    assert!(
        states.contains(&(own.clone(), TabState::Open)),
        "{states:?}"
    );
    assert!(
        states.contains(&(agent.clone(), TabState::Revoked)),
        "{states:?}"
    );
    let panes = window
        .update(cx, |shell, _, cx| {
            let review = shell.reviews[&session].read(cx);
            (
                review.browser_pane_for_tests(&own.0).unwrap(),
                review.browser_pane_for_tests(&agent.0).unwrap(),
            )
        })
        .unwrap();
    // The user's pane still gets the page: its live view runs.
    rig.host
        .live_navigate(
            &session_key(session),
            &own,
            &gespan_core::browser::live::LiveNav::Reload,
        )
        .expect("the user's tab is still theirs to use");
    cx.update(|cx| panes.1.update(cx, |pane, cx| pane.pull_for_tests(cx)));
    cx.update(|cx| {
        assert_ne!(
            panes.0.read(cx).ended,
            Some(gespan_core::browser::Ended::TabGone)
        );
    });
    assert!(
        rig.host
            .live_navigate(
                &session_key(session),
                &agent,
                &gespan_core::browser::live::LiveNav::Reload
            )
            .is_err()
    );
}

/// Closing the pane of the user's own tab closes the page, as in a browser.
#[gpui::test]
fn closing_the_users_tab_pane_closes_the_tab(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    click_link(window, Modifiers::none(), cx);
    let own = host_tabs(&rig.host, session)[0].0.clone();
    let target = rig
        .host
        .policy()
        .read()
        .lease(&own)
        .unwrap()
        .target_id
        .clone();
    window
        .update(cx, |shell, _, cx| {
            shell.reviews[&session].update(cx, |r, cx| r.close_active(cx));
        })
        .unwrap();
    settle(window, cx);
    assert!(
        rig.received("Target.closeTarget", |params| params["targetId"]
            == target.0.as_str())
    );
}

/// No browser to start: the click is never lost, the system browser opens
/// the link and the thread says why.
#[gpui::test]
fn without_a_browser_to_start_the_link_goes_to_the_system_browser(cx: &mut TestAppContext) {
    let host = BrowserHost::for_tests(BrowserSettings {
        mode: BrowserMode::Managed,
        ..BrowserSettings::default()
    });
    let (window, session) = thread_with_link(host.clone(), cx);
    click_link(window, Modifiers::none(), cx);
    assert_eq!(cx.opened_url().as_deref(), Some(URL));
    assert!(host.state().tabs.is_empty());
    window
        .update(cx, |shell, _, cx| {
            assert!(browser_tabs(shell, session, cx).is_empty());
            let notice = shell.sessions[&session]
                .read(cx)
                .link_notice()
                .map(str::to_owned)
                .expect("the thread says why");
            assert!(notice.contains("system browser"), "{notice}");
        })
        .unwrap();
}

/// A link in the dock's terminal opens in the thread the user is looking at.
#[gpui::test]
fn a_terminal_link_opens_in_the_thread_on_screen(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    window
        .update(cx, |shell, _, cx| {
            shell.open_thread_link(None, "http://localhost:5174/".into(), cx)
        })
        .unwrap();
    settle(window, cx);
    let tabs = host_tabs(&rig.host, session);
    assert!(matches!(tabs.as_slice(), [(_, true, _)]), "{tabs:?}");
    assert!(navigated_to(&rig, "http://localhost:5174/"));
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(browser_tabs(shell, session, cx).len(), 1);
        })
        .unwrap();
}

fn closed_in_browser(rig: &Rig, target: &str) -> bool {
    rig.received("Target.closeTarget", |params| params["targetId"] == target)
}

fn target_of(rig: &Rig, tab: &TabId) -> String {
    rig.host
        .policy()
        .read()
        .lease(tab)
        .unwrap()
        .target_id
        .0
        .clone()
}

/// A chat with no pane in the workspace (a Pilot's, Inbox Ask): nothing
/// could show the page, so the system browser opens the link and no
/// browser starts for it.
#[gpui::test]
fn a_link_in_a_chat_with_no_pane_goes_to_the_system_browser(cx: &mut TestAppContext) {
    let rig = Rig::start_managed();
    let (window, _) = thread_with_link(rig.host.clone(), cx);
    let pilot_chat = 9_999;
    window
        .update(cx, |shell, _, cx| {
            assert!(shell.workspace.tab_of(pilot_chat).is_none());
            shell.open_thread_link(Some(pilot_chat), URL.into(), cx);
            assert!(shell.opening_links.is_empty(), "nothing in flight");
        })
        .unwrap();
    settle(window, cx);
    assert_eq!(cx.opened_url().as_deref(), Some(URL));
    assert_ne!(
        rig.host.connection().state,
        gespan_core::browser::ConnectionState::Connected,
        "no browser started for it"
    );
    assert_eq!(
        rig.host.policy().read().leases().count(),
        0,
        "no tab opened"
    );
    window
        .update(cx, |shell, _, cx| {
            assert!(browser_tabs(shell, pilot_chat, cx).is_empty());
            assert!(shell.pending_links.is_empty());
        })
        .unwrap();
}

/// The thread leaves the workspace while its link is opening: no pane would
/// ever show or close the user's tab, so it closes, and the system browser
/// opens the link so the click is not lost.
#[gpui::test]
fn a_thread_that_leaves_while_its_link_opens_falls_back_to_the_system_browser(
    cx: &mut TestAppContext,
) {
    let rig = Rig::start_managed();
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    window
        .update(cx, |shell, window, cx| {
            shell.open_thread_link(Some(session), URL.into(), cx);
            assert_eq!(notice(shell, session, cx).as_deref(), Some(OPENING));
            shell.close_tabs(&[session], None, window, cx);
            assert!(shell.workspace.tab_of(session).is_none());
        })
        .unwrap();
    assert_eq!(cx.opened_url(), None, "still opening in the in-app browser");
    settle(window, cx);
    assert_eq!(cx.opened_url().as_deref(), Some(URL));
    let lease = rig
        .host
        .policy()
        .read()
        .leases()
        .find(|lease| lease.owner == session_key(session))
        .cloned()
        .expect("the tab was opened");
    assert!(lease.user_tab);
    assert_eq!(lease.state, TabState::Closed);
    assert!(
        closed_in_browser(&rig, &lease.target_id.0),
        "the orphan tab closed"
    );
    window
        .update(cx, |shell, _, cx| {
            assert!(browser_tabs(shell, session, cx).is_empty());
            assert!(shell.pending_links.is_empty());
            assert!(shell.opening_links.is_empty());
            if shell.sessions.contains_key(&session) {
                assert_eq!(notice(shell, session, cx), None, "no opening line left");
            }
        })
        .unwrap();
}

/// Closing the window closes the user's own tabs shown in it; an agent's
/// tab stays the agent's.
#[gpui::test]
fn closing_the_window_closes_its_users_tabs(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    click_link(window, Modifiers::none(), cx);
    let own = host_tabs(&rig.host, session)[0].0.clone();
    let agent = rig.open_tab(session);
    let (own_target, agent_target) = (target_of(&rig, &own), target_of(&rig, &agent));
    window.update(cx, |_, w, _| w.remove_window()).unwrap();
    cx.run_until_parked();
    assert!(
        closed_in_browser(&rig, &own_target),
        "the user's tab closed"
    );
    assert!(
        !rig.server
            .received()
            .iter()
            .any(|r| r.method == "Target.closeTarget" && r.params["targetId"] == agent_target),
        "the agent's tab stays"
    );
}

fn notice(shell: &Shell, session: i64, cx: &App) -> Option<String> {
    shell.sessions[&session]
        .read(cx)
        .link_notice()
        .map(str::to_owned)
}

const OPENING: &str = "Opening in the in-app browser…";

/// The first link can wait for the browser to start: the thread says it is
/// opening until the pane opens, and a repeat click meanwhile opens no
/// second tab.
#[gpui::test]
fn a_link_says_it_is_opening_and_a_repeat_click_opens_no_second_tab(cx: &mut TestAppContext) {
    let rig = Rig::start_managed();
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    window
        .update(cx, |shell, _, cx| {
            shell.open_thread_link(Some(session), URL.into(), cx);
            assert_eq!(notice(shell, session, cx).as_deref(), Some(OPENING));
            // Clicked again before the first has opened.
            shell.open_thread_link(Some(session), URL.into(), cx);
        })
        .unwrap();
    settle(window, cx);
    let tabs = host_tabs(&rig.host, session);
    assert_eq!(tabs.len(), 1, "one tab for the two clicks: {tabs:?}");
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(browser_tabs(shell, session, cx).len(), 1);
            assert_eq!(notice(shell, session, cx), None, "the pane opened");
        })
        .unwrap();
    // Once it has opened, a new click is a new tab, as in a browser.
    click_link(window, Modifiers::none(), cx);
    assert_eq!(host_tabs(&rig.host, session).len(), 2);
}

/// Two different links in flight both open; the opening line stays until
/// the last of them has.
#[gpui::test]
fn different_links_in_flight_each_open(cx: &mut TestAppContext) {
    let rig = Rig::start_managed();
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    window
        .update(cx, |shell, _, cx| {
            shell.open_thread_link(Some(session), URL.into(), cx);
            shell.open_thread_link(Some(session), "https://example.com/other".into(), cx);
        })
        .unwrap();
    settle(window, cx);
    assert_eq!(host_tabs(&rig.host, session).len(), 2);
    assert!(navigated_to(&rig, "https://example.com/other"));
    window
        .update(cx, |shell, _, cx| {
            assert_eq!(notice(shell, session, cx), None);
            assert!(shell.opening_links.is_empty());
        })
        .unwrap();
}

/// No browser to start: the opening line gives way to the fallback's, and
/// the link is not stuck in flight (the next click tries again).
#[gpui::test]
fn a_failed_open_replaces_the_opening_line_and_frees_the_link(cx: &mut TestAppContext) {
    let host = BrowserHost::for_tests(BrowserSettings {
        mode: BrowserMode::Managed,
        ..BrowserSettings::default()
    });
    let (window, session) = thread_with_link(host.clone(), cx);
    window
        .update(cx, |shell, _, cx| {
            shell.open_thread_link(Some(session), URL.into(), cx);
            assert_eq!(notice(shell, session, cx).as_deref(), Some(OPENING));
        })
        .unwrap();
    settle(window, cx);
    assert_eq!(cx.opened_url().as_deref(), Some(URL));
    window
        .update(cx, |shell, _, cx| {
            let shown = notice(shell, session, cx).expect("the thread says why");
            assert!(shown.contains("system browser"), "{shown}");
            assert!(shell.opening_links.is_empty(), "not stuck in flight");
            shell.open_thread_link(Some(session), URL.into(), cx);
            assert_eq!(notice(shell, session, cx).as_deref(), Some(OPENING));
        })
        .unwrap();
    settle(window, cx);
    window
        .update(cx, |shell, _, cx| {
            assert!(shell.opening_links.is_empty());
            assert!(notice(shell, session, cx).is_some_and(|n| n.contains("system browser")));
        })
        .unwrap();
}

/// Closing the thread's tab (the thread leaves the workspace) closes the
/// user's own tabs it opened from links; its agent's tab stays the agent's.
#[gpui::test]
fn closing_the_threads_tab_closes_its_users_tabs(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    click_link(window, Modifiers::none(), cx);
    let own = host_tabs(&rig.host, session)[0].0.clone();
    let agent = rig.open_tab(session);
    let (own_target, agent_target) = (target_of(&rig, &own), target_of(&rig, &agent));
    window
        .update(cx, |shell, window, cx| {
            shell.close_tabs(&[session], None, window, cx)
        })
        .unwrap();
    settle(window, cx);
    window
        .update(cx, |shell, _, _| {
            assert!(shell.workspace.tab_of(session).is_none(), "the tab closed")
        })
        .unwrap();
    assert!(
        closed_in_browser(&rig, &own_target),
        "the user's tab closed"
    );
    assert_eq!(
        rig.host
            .policy()
            .read()
            .lease(&own)
            .map(|lease| lease.state),
        Some(TabState::Closed)
    );
    assert!(
        !rig.server
            .received()
            .iter()
            .any(|r| r.method == "Target.closeTarget" && r.params["targetId"] == agent_target),
        "the agent's tab stays"
    );
    assert_eq!(
        rig.host
            .policy()
            .read()
            .lease(&agent)
            .map(|lease| lease.state),
        Some(TabState::Open)
    );
}

/// Closing another thread's tab leaves this thread's user tabs alone.
#[gpui::test]
fn closing_another_threads_tab_keeps_this_threads_users_tabs(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, session) = thread_with_link(rig.host.clone(), cx);
    click_link(window, Modifiers::none(), cx);
    let own = host_tabs(&rig.host, session)[0].0.clone();
    let other = window
        .update(cx, |shell, window, cx| {
            let summary = locked(&shell.store)
                .unwrap()
                .create("claude", "/work/gespan")
                .unwrap();
            let other = summary.id;
            shell.add(summary, cx);
            shell.open(other, window, cx);
            other
        })
        .unwrap();
    settle(window, cx);
    window
        .update(cx, |shell, window, cx| {
            shell.close_tabs(&[other], None, window, cx)
        })
        .unwrap();
    settle(window, cx);
    assert_eq!(
        rig.host
            .policy()
            .read()
            .lease(&own)
            .map(|lease| lease.state),
        Some(TabState::Open)
    );
}
