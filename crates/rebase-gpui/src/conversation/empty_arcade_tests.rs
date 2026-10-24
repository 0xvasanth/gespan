use super::*;

#[gpui::test]
fn a_game_in_play_hides_the_question_and_composer_beneath_it(cx: &mut gpui::TestAppContext) {
    let store = Arc::new(std::sync::Mutex::new(
        rebase_core::store::Store::open(Path::new(":memory:")).unwrap(),
    ));
    let summary = locked(&store)
        .unwrap()
        .create("claude", "/fixture")
        .unwrap();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| {
                let mut view = Conversation::new(summary, store, cx);
                view.loaded = true;
                view
            })
        })
        .unwrap()
    });
    let view = window.root(cx).unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
    visual.refresh().unwrap();
    // Idle, the attract loop sits under the question and the composer.
    assert!(visual.debug_bounds("empty-session-arcade").is_some());
    assert!(visual.debug_bounds("empty-session-title").is_some());
    assert!(visual.debug_bounds("composer-box").is_some());

    let arcade = view.read_with(visual, |view, _| view.arcade.as_ref().unwrap().0.clone());
    arcade.update(visual, |arcade, cx| arcade.set_playing(true, cx));
    visual.run_until_parked();
    visual.refresh().unwrap();
    // The playing arcade fills the pane with a translucent fill, so nothing
    // may be drawn beneath it.
    assert!(visual.debug_bounds("empty-session-arcade").is_some());
    assert!(visual.debug_bounds("empty-session-title").is_none());
    assert!(visual.debug_bounds("composer-box").is_none());

    arcade.update(visual, |arcade, cx| arcade.set_playing(false, cx));
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("empty-session-title").is_some());
    assert!(visual.debug_bounds("composer-box").is_some());
}
