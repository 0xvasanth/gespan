//! The globe beside the composer's pills: shown while the thread has agent
//! tabs, listing them with "Show in Gespan".
use super::*;
use crate::browser_test_rig::Rig;
use gespan_core::store::Store;
use gpui::{TestAppContext, VisualTestContext, WindowHandle};
use std::sync::Mutex;

fn conversation(
    rig: &Rig,
    cx: &mut TestAppContext,
) -> (WindowHandle<Conversation>, i64, VisualTestContext) {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let mut summary = store.create("codex", "/missing/browser-globe").unwrap();
    summary.provider = "fixture".into();
    let id = summary.id;
    let host = rig.host.clone();
    let window = cx.update(|cx| {
        input::bind(cx);
        crate::browser::install_host(host, cx);
        cx.set_global(crate::settings::Preferences::default());
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                view.loaded = true;
                view.summary.has_messages = true;
                view
            })
        })
        .unwrap()
    });
    let mut visual = VisualTestContext::from_window(window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    (window, id, visual)
}

/// The host changed: the app reads it again, as following it would.
fn refresh(visual: &mut VisualTestContext) {
    visual.update(|_, cx| crate::browser::refresh(cx));
    visual.run_until_parked();
    visual.refresh().unwrap();
}

fn leak(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

#[gpui::test]
fn the_globe_shows_only_while_the_thread_has_agent_tabs(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (_window, id, mut visual) = conversation(&rig, cx);
    assert!(
        visual.debug_bounds("composer-browser").is_none(),
        "no tab, no globe"
    );
    // Another thread's tab is not this one's.
    rig.open_tab(id + 100);
    refresh(&mut visual);
    assert!(visual.debug_bounds("composer-browser").is_none());
    let tab = rig.open_tab(id);
    refresh(&mut visual);
    assert!(
        visual.debug_bounds("composer-browser").is_some(),
        "the thread's tab brings the globe"
    );
    // Closed, the tab leaves the globe with it.
    rig.host.close_tab_by_user(&tab).unwrap();
    refresh(&mut visual);
    assert!(visual.debug_bounds("composer-browser").is_none());
}

#[gpui::test]
fn show_in_gespan_asks_for_the_tab_beside_the_thread(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, id, mut visual) = conversation(&rig, cx);
    let tab = rig.open_tab(id);
    refresh(&mut visual);
    let asked = Arc::new(Mutex::new(Vec::new()));
    let opened = Arc::new(Mutex::new(Vec::new()));
    window
        .update(&mut visual, |_, _, cx| {
            let view = cx.entity();
            let (asked, opened) = (asked.clone(), opened.clone());
            cx.subscribe(&view, move |_, _, event: &OpenSessionChanges, _| {
                asked.lock().unwrap().push(event.0.clone());
            })
            .detach();
            cx.subscribe(&view, move |_, _, event: &BrowserTabOpened, _| {
                opened.lock().unwrap().push(event.0.clone());
            })
            .detach();
        })
        .unwrap();
    let globe = visual.debug_bounds("composer-browser").unwrap();
    visual.simulate_click(globe.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    let show = visual
        .debug_bounds(leak(format!("browser-show-{}", tab.0)))
        .expect("the menu lists the tab with Show in Gespan");
    visual.simulate_click(show.center(), Default::default());
    visual.run_until_parked();
    assert_eq!(
        *asked.lock().unwrap(),
        [crate::review::ReviewTab::Browser {
            session: id,
            tab: tab.0.clone(),
        }]
    );
    assert!(
        visual.debug_bounds("composer-browser-menu").is_none(),
        "the menu closes"
    );
    assert!(
        opened.lock().unwrap().is_empty(),
        "a tab already there when the thread opened is not news"
    );
}

#[gpui::test]
fn each_tab_the_agent_opens_is_told_once(cx: &mut TestAppContext) {
    let rig = Rig::start();
    let (window, id, mut visual) = conversation(&rig, cx);
    let opened = Arc::new(Mutex::new(Vec::new()));
    window
        .update(&mut visual, |_, _, cx| {
            let opened = opened.clone();
            cx.subscribe(&cx.entity(), move |_, _, event: &BrowserTabOpened, _| {
                opened.lock().unwrap().push(event.0.clone());
            })
            .detach();
        })
        .unwrap();
    let first = rig.open_tab(id);
    refresh(&mut visual);
    // Nothing changed for the thread: no news.
    refresh(&mut visual);
    rig.open_tab(id + 100);
    refresh(&mut visual);
    let second = rig.open_tab(id);
    refresh(&mut visual);
    assert_eq!(*opened.lock().unwrap(), [first.0, second.0]);
}
