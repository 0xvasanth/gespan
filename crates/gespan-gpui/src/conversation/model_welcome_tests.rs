use super::*;
use crate::model_welcome::Kind as Welcome;
use gespan_core::store::Store;
use gpui::{TestAppContext, WindowHandle};
use std::sync::Mutex;

fn conversation(cx: &mut TestAppContext) -> WindowHandle<Conversation> {
    let store = Store::open(Path::new(":memory:")).unwrap();
    let summary = store.create("claude", "/missing/welcome-project").unwrap();
    cx.update(|cx| {
        input::bind(cx);
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut view = Conversation::new(summary, Arc::new(Mutex::new(store)), cx);
                view.loaded = true;
                view
            })
        })
        .unwrap()
    })
}

fn playing(window: WindowHandle<Conversation>, cx: &mut TestAppContext) -> Option<Welcome> {
    window
        .update(cx, |view, _, _| {
            view.model_welcome.as_ref().map(|(run, _)| run.kind)
        })
        .unwrap()
}

#[gpui::test]
fn picking_opus_5_5_plays_its_welcome_over_the_session_until_it_ends(cx: &mut TestAppContext) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            assert!(view.apply_model_pick(
                &ModelPicked(Kind::Claude, Some("claude-opus-5-5".into())),
                cx
            ));
        })
        .unwrap();
    assert_eq!(playing(window, cx), Some(Welcome::Opus));
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.simulate_resize(gpui::size(px(900.), px(700.)));
    visual.run_until_parked();
    let layer = visual
        .debug_bounds("model-welcome")
        .expect("the scene is drawn");
    assert_eq!(
        layer.size,
        gpui::size(px(900.), px(700.)),
        "over the whole session"
    );
    let composer = visual.debug_bounds("composer-box").expect("the composer");
    // The composer stays usable: the scene takes no input.
    visual.simulate_click(composer.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |view, window, cx| {
            assert!(view.input.focus_handle(cx).is_focused(window));
        })
        .unwrap();
    visual.executor().advance_clock(Welcome::Opus.duration());
    visual.run_until_parked();
    assert_eq!(playing(window, visual), None);
    assert!(visual.debug_bounds("model-welcome").is_none());
}

#[gpui::test]
fn astra_has_its_own_scene_and_other_models_end_a_playing_one(cx: &mut TestAppContext) {
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            view.apply_model_pick(&ModelPicked(Kind::Codex, Some("gpt-6-astra".into())), cx);
        })
        .unwrap();
    assert_eq!(playing(window, cx), Some(Welcome::Astra));
    window
        .update(cx, |view, _, cx| {
            view.apply_model_pick(&ModelPicked(Kind::Claude, Some("claude-opus-5".into())), cx);
        })
        .unwrap();
    assert_eq!(playing(window, cx), None);
    // Picking it again restarts the full scene rather than its remainder.
    window
        .update(cx, |view, _, cx| {
            view.apply_model_pick(
                &ModelPicked(Kind::Claude, Some("claude-opus-5-5".into())),
                cx,
            );
        })
        .unwrap();
    cx.executor().advance_clock(Welcome::Opus.duration() / 2);
    window
        .update(cx, |view, _, cx| {
            view.apply_model_pick(
                &ModelPicked(Kind::Claude, Some("claude-opus-5-5".into())),
                cx,
            );
        })
        .unwrap();
    cx.executor()
        .advance_clock(Welcome::Opus.duration() * 3 / 4);
    cx.run_until_parked();
    assert_eq!(playing(window, cx), Some(Welcome::Opus));
}

#[gpui::test]
fn reduced_motion_skips_the_welcome(cx: &mut TestAppContext) {
    cx.update(|cx| cx.set_reduce_motion(true));
    let window = conversation(cx);
    window
        .update(cx, |view, _, cx| {
            view.apply_model_pick(
                &ModelPicked(Kind::Claude, Some("claude-opus-5-5".into())),
                cx,
            );
        })
        .unwrap();
    assert_eq!(playing(window, cx), None);
}
