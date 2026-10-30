//! The board's last gaps with Tauri: Alt+↑/↓ on subtasks, focusable list and
//! table rows, the template picker with no boards, and the date field on a
//! card face.
use super::*;

fn focus_subtask(f: &Fixture, visual: &mut VisualTestContext, id: &str) {
    f.window
        .update(visual, |view, window, cx| {
            let handle = view.card_detail.as_ref().unwrap().subtask_focus[id].clone();
            window.focus(&handle, cx);
        })
        .unwrap();
    visual.run_until_parked();
}

#[gpui::test]
fn alt_arrows_move_the_focused_subtask_and_focus_stays_on_it(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    let ids = add_subtasks(&f, &card, &["one", "two", "three"]);
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    focus_subtask(&f, visual, &ids[1]);

    visual.simulate_keystrokes("alt-up");
    visual.run_until_parked();
    assert_eq!(subtask_titles(&f, &card), ["two", "one", "three"]);
    // Focus followed the row, so the next press moves the same subtask.
    visual.simulate_keystrokes("alt-down alt-down");
    visual.run_until_parked();
    assert_eq!(subtask_titles(&f, &card), ["one", "three", "two"]);
    // The ends stay put.
    visual.simulate_keystrokes("alt-down");
    visual.run_until_parked();
    assert_eq!(subtask_titles(&f, &card), ["one", "three", "two"]);
    // Only Alt alone moves it.
    visual.simulate_keystrokes("up shift-alt-up cmd-alt-up");
    visual.run_until_parked();
    assert_eq!(subtask_titles(&f, &card), ["one", "three", "two"]);
}

#[gpui::test]
fn alt_arrows_do_nothing_with_no_subtask_focused(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    add_subtasks(&f, &card, &["one", "two"]);
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-1");
    visual.simulate_keystrokes("alt-up alt-down");
    visual.run_until_parked();
    assert_eq!(subtask_titles(&f, &card), ["one", "two"]);
}

#[gpui::test]
fn list_and_table_rows_are_tab_stops_that_enter_opens(cx: &mut TestAppContext) {
    for layout in ["board-layout-list", "board-layout-table"] {
        let f = seeded(cx, &["First", "Second"]);
        let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
        click(visual, layout);
        f.window
            .update(visual, |view, window, cx| window.focus(&view.focus, cx))
            .unwrap();
        visual.run_until_parked();
        // The header's controls come first, then the rows.
        let focused = (0..40)
            .find_map(|_| {
                let focused = f
                    .window
                    .update(visual, |view, window, cx| {
                        window.focus_next(cx);
                        view.focused_card(window, cx)
                    })
                    .unwrap();
                visual.run_until_parked();
                focused
            })
            .unwrap_or_else(|| panic!("{layout}: Tab reaches a row"));
        visual.simulate_keystrokes("enter");
        visual.run_until_parked();
        drawer(&f, visual, |view, _| {
            assert_eq!(
                view.card_detail.as_ref().map(|d| d.card_id.clone()),
                Some(focused.clone()),
                "{layout}"
            );
        });
    }
}

#[gpui::test]
fn the_board_shortcuts_act_on_a_focused_row(cx: &mut TestAppContext) {
    let f = seeded(cx, &["First"]);
    let card = card_id(&f, cx, "First");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-layout-list");
    f.window
        .update(visual, |view, window, cx| {
            window.focus(&view.card_focus[&card].clone(), cx)
        })
        .unwrap();
    visual.run_until_parked();
    visual.simulate_keystrokes("e");
    visual.run_until_parked();
    drawer(&f, visual, |view, _| assert!(view.card_detail.is_some()));
}

#[gpui::test]
fn pick_a_date_on_a_card_face_stays_on_the_board(cx: &mut TestAppContext) {
    let f = seeded(cx, &["Task"]);
    let card = card_id(&f, cx, "Task");
    let day = chrono::NaiveDate::from_ymd_opt(2030, 3, 14).unwrap();
    Service::open(&f.db)
        .unwrap()
        .set_due(&card, super::super::style::day_start(day))
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    click(visual, "board-card-due-1");
    click(visual, "menu-action-due-pick");
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-due-entry").is_some());
    drawer(&f, visual, |view, cx| {
        assert!(view.card_detail.is_none(), "the drawer stays closed");
        assert_eq!(
            view.due_entry
                .as_ref()
                .unwrap()
                .input
                .read(cx)
                .content
                .as_ref(),
            "2030-03-14"
        );
    });

    // A mistake keeps the field open and changes nothing.
    retype(visual, "board-due-entry", "next tuesday");
    press_enter(visual);
    drawer(&f, visual, |view, _| {
        assert!(view.due_entry.is_some());
        assert!(view.error.as_deref().unwrap().contains("next tuesday"));
    });
    assert_eq!(
        extras_of(&f, &card).due_at,
        super::super::style::day_start(day)
    );

    retype(visual, "board-due-entry", "2031-01-02");
    press_enter(visual);
    drawer(&f, visual, |view, _| assert!(view.due_entry.is_none()));
    let picked = chrono::NaiveDate::from_ymd_opt(2031, 1, 2).unwrap();
    assert_eq!(
        extras_of(&f, &card).due_at,
        super::super::style::day_start(picked)
    );

    // Escape backs out; an emptied field clears the date.
    click(visual, "board-card-due-1");
    click(visual, "menu-action-due-pick");
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    drawer(&f, visual, |view, _| assert!(view.due_entry.is_none()));
    assert!(extras_of(&f, &card).due_at.is_some());
    click(visual, "board-card-due-1");
    click(visual, "menu-action-due-pick");
    retype(visual, "board-due-entry", "");
    press_enter(visual);
    assert_eq!(extras_of(&f, &card).due_at, None);
}

#[gpui::test]
fn no_boards_offers_a_template_picker_and_makes_nothing_by_itself(cx: &mut TestAppContext) {
    let f = open_seeded(cx, false);
    assert!(boards(&f).is_empty(), "opening the board creates no board");
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-no-boards").is_some());
    assert!(visual.debug_bounds("board-switcher").is_none());

    // The board's shortcuts have no board to act on.
    f.window
        .update(visual, |view, window, cx| window.focus(&view.focus, cx))
        .unwrap();
    visual.simulate_keystrokes("n / e delete");
    visual.run_until_parked();
    drawer(&f, visual, |view, _| assert!(view.new_card.is_none()));

    // Backing out leaves it empty.
    click(visual, "board-create-first");
    assert!(visual.debug_bounds("board-template-blank").is_some());
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(boards(&f).is_empty());

    click(visual, "board-create-first");
    click(visual, "board-template-agent-pipeline");
    visual.simulate_input("Agents");
    press_enter(visual);
    assert_eq!(open_name(&f, visual), "Agents");
    let names: Vec<String> = column_names(&f, visual).into_iter().map(|c| c.0).collect();
    assert_eq!(
        names,
        ["Queued", "Running", "Needs review", "Merged", "Done"]
    );
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-no-boards").is_none());
}

#[gpui::test]
fn a_board_made_in_the_other_app_replaces_the_empty_state(cx: &mut TestAppContext) {
    let f = open_seeded(cx, false);
    Service::open(&f.db)
        .unwrap()
        .create_from_template("Elsewhere", "bug-triage")
        .unwrap();
    reload(&f, cx);
    let visual = &mut VisualTestContext::from_window(f.window.into(), cx);
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("board-no-boards").is_none());
    assert_eq!(open_name(&f, visual), "Elsewhere");
}
