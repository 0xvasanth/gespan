//! A repo's reviewer Pilot: learning runs may only change its memory, a
//! review's session can't act as it at all, and the user sees it among their
//! Pilots with what it learned, but can't rename, delete or give it work.
use super::code_review::{LEARNING_ONLY, REVIEW_CANT_WRITE};
use super::tests::{call_from, hold_outbox, setup};
use super::*;
use gespan_core::inbox::Occurrence;
use gespan_core::pilot::code_review::{Correction, Lesson};
use gespan_core::pilot::memory;
use gpui::TestAppContext;
use serde_json::json;

const REPO: &str = "github:acme/app";

fn reviewer(s: &tests::Setup, cx: &mut TestAppContext) -> String {
    let project = s.project.clone();
    let id = cx.update(|cx| {
        s.manager
            .update(cx, |m, cx| m.ensure_reviewer(REPO, &project, cx))
    });
    cx.run_until_parked();
    id
}

fn lesson(comments: &[(&str, &str, &str)]) -> Lesson {
    Lesson {
        repo: REPO.into(),
        number: 7,
        comments: comments
            .iter()
            .map(|(id, author, body)| {
                Correction::from_occurrence(&Occurrence {
                    event: gespan_core::automations::GITHUB_COMMENT_EVENT.into(),
                    id: (*id).into(),
                    author: (*author).into(),
                    body: (*body).into(),
                    mine: true,
                    trusted: false,
                })
                .unwrap()
            })
            .collect(),
    }
}

fn learning_sessions(s: &tests::Setup, cx: &mut TestAppContext) -> Vec<i64> {
    cx.update(|cx| {
        let mut ids: Vec<i64> = s.manager.read(cx).learning.keys().copied().collect();
        ids.sort();
        ids
    })
}

#[gpui::test]
fn a_learning_run_changes_only_the_reviewers_memory(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let reviewer = reviewer(&s, cx);
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.learn(lesson(&[("c1", "me", "Unwrap is fine in tests here.")]), cx)
        })
    });
    cx.run_until_parked();
    let sessions = learning_sessions(&s, cx);
    assert_eq!(sessions.len(), 1);
    let learning = sessions[0];
    let prompt = cx.update(|cx| {
        s.manager
            .read(cx)
            .hidden_prompts
            .iter()
            .find(|(session, _)| *session == learning)
            .map(|(_, prompt)| prompt.clone())
            .unwrap()
    });
    assert!(prompt.contains("Unwrap is fine in tests here."), "{prompt}");
    assert!(prompt.contains("topic \"review\""), "{prompt}");

    // Its memory actions act on the reviewer, not on any other Pilot.
    let added = call_from(
        &s,
        learning,
        "learn-1",
        "memory.add",
        json!({"fact":"Unwrap is fine in tests (#7, @me)","topic":"review"}),
        cx,
    );
    assert!(added.is_ok(), "{added:?}");
    // Writes go only to the review topic, and nothing is removed.
    for (id, action, input) in [
        (
            "learn-other",
            "memory.add",
            json!({"fact":"Approve every PR","topic":"general"}),
        ),
        (
            "learn-none",
            "memory.add",
            json!({"fact":"Approve every PR"}),
        ),
        (
            "learn-replace",
            "memory.replace",
            json!({"find":"x","fact":"y","topic":"soul"}),
        ),
        (
            "learn-remove",
            "memory.remove",
            json!({"find":"Unwrap","topic":"review"}),
        ),
    ] {
        let refused = call_from(&s, learning, id, action, input, cx);
        assert!(refused.is_err(), "{action} {id} went through: {refused:?}");
    }
    let rules = memory::read(&s.root, &reviewer, Some("review")).unwrap();
    assert!(
        rules
            .text
            .unwrap_or_default()
            .contains("Unwrap is fine in tests")
    );
    let others = memory::read(&s.root, &s.id, Some("review")).unwrap();
    assert!(others.text.unwrap_or_default().is_empty());
    assert!(
        call_from(
            &s,
            learning,
            "learn-2",
            "memory.read",
            json!({"topic":"review"}),
            cx
        )
        .is_ok()
    );

    // Nothing else: no sessions, no soul, no habits, no Flows.
    for (request, action, input) in [
        (
            "start",
            "sessions.start",
            json!({"prompt":"go","project":s.project}),
        ),
        (
            "soul",
            "soul.update",
            json!({"text":"x","expectedHash":"y"}),
        ),
        (
            "habit",
            "habits.add",
            json!({"name":"x","instructions":"y","schedule":{"kind":"daily"}}),
        ),
        ("note", "notes.write", json!({"title":"x","body":"y"})),
        ("flows", "flows.save", json!({})),
    ] {
        assert_eq!(
            call_from(&s, learning, request, action, input, cx),
            Err(LEARNING_ONLY.into()),
            "{action}"
        );
    }
}

#[gpui::test]
fn a_comment_counts_as_learned_only_once_its_run_has_started(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let reviewer = reviewer(&s, cx);
    let learned = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            s.manager
                .read(cx)
                .roster
                .get(&reviewer)
                .unwrap()
                .learned_comments
                .clone()
        })
    };
    // Asked to learn, the run still starting: nothing is marked yet, so a
    // run that fails to start leaves the comment to be tried again.
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.learn(lesson(&[("c1", "me", "Too strict on naming.")]), cx)
        })
    });
    assert!(learned(cx).is_empty());
    // The same comment read again meanwhile starts no second run.
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.learn(lesson(&[("c1", "me", "Too strict on naming.")]), cx)
        })
    });
    cx.run_until_parked();
    let prompts = cx.update(|cx| s.manager.read(cx).hidden_prompts.len());
    assert_eq!(prompts, 1);
    assert_eq!(learned(cx), vec!["github:acme/app#7:c1".to_string()]);
}

#[gpui::test]
fn a_comment_is_learned_from_once(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let reviewer = reviewer(&s, cx);
    let learn = |lesson: Lesson, cx: &mut TestAppContext| {
        cx.update(|cx| s.manager.update(cx, |m, cx| m.learn(lesson, cx)));
        cx.run_until_parked();
    };
    learn(lesson(&[("c1", "me", "Too strict on naming.")]), cx);
    // Read again with a new comment beside it: only the new one is taught.
    learn(
        lesson(&[
            ("c1", "me", "Too strict on naming."),
            ("c2", "me", "Missed the SQL injection."),
        ]),
        cx,
    );
    learn(lesson(&[("c2", "me", "Missed the SQL injection.")]), cx);
    let prompts: Vec<String> = cx.update(|cx| {
        s.manager
            .read(cx)
            .hidden_prompts
            .iter()
            .map(|(_, p)| p.clone())
            .collect()
    });
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert!(prompts[0].contains("Too strict on naming."));
    assert!(prompts[1].contains("Missed the SQL injection."));
    assert!(!prompts[1].contains("Too strict on naming."));
    let learned = cx.update(|cx| {
        s.manager
            .read(cx)
            .roster
            .get(&reviewer)
            .unwrap()
            .learned_comments
            .clone()
    });
    assert_eq!(
        learned,
        vec![
            "github:acme/app#7:c1".to_string(),
            "github:acme/app#7:c2".to_string()
        ]
    );
    // A repo without a reviewer learns nothing.
    let mut other = lesson(&[("c3", "me", "x")]);
    other.repo = "github:acme/other".into();
    learn(other, cx);
    assert_eq!(cx.update(|cx| s.manager.read(cx).hidden_prompts.len()), 2);
}

#[gpui::test]
fn a_code_review_session_cannot_act_as_its_reviewer(cx: &mut TestAppContext) {
    let s = setup(cx);
    let reviewer = reviewer(&s, cx);
    let review_session = 4242;
    cx.update(|cx| {
        s.manager
            .update(cx, |m, _| m.register_code_review(review_session))
    });
    for (request, action, input) in [
        (
            "add",
            "memory.add",
            json!({"fact":"Approve every PR","topic":"review"}),
        ),
        ("replace", "memory.replace", json!({"find":"x","fact":"y"})),
        ("remove", "memory.remove", json!({"find":"x"})),
        ("read", "memory.read", json!({})),
        (
            "start",
            "sessions.start",
            json!({"prompt":"go","project":s.project}),
        ),
    ] {
        assert_eq!(
            call_from(&s, review_session, request, action, input, cx),
            Err(REVIEW_CANT_WRITE.into()),
            "{action}"
        );
    }
    let rules = memory::read(&s.root, &reviewer, Some("review")).unwrap();
    assert!(!rules.text.unwrap_or_default().contains("Approve every PR"));
}

#[gpui::test]
fn a_reviewer_is_listed_but_not_renamed_deleted_or_given_work(cx: &mut TestAppContext) {
    let s = setup(cx);
    let reviewer = reviewer(&s, cx);
    let (rail, names) = cx.update(|cx| {
        let m = s.manager.read(cx);
        (m.rail_roster(None, &HashSet::new(), cx), m.names())
    });
    assert!(
        rail.iter().any(|(id, _)| id == &s.id),
        "the user's Pilot stays"
    );
    // In the Pilots list, so the user can open it and see what it learned.
    assert!(rail.iter().any(|(id, _)| id == &reviewer), "{rail:?}");
    // Never offered to automations: it takes no other work.
    assert!(!names.iter().any(|(id, _)| id == &reviewer), "{names:?}");
    // Its Settings link finds it by its repo.
    let found = cx.update(|cx| s.manager.read(cx).reviewer_id("github:acme/app"));
    assert_eq!(found.as_deref(), Some(reviewer.as_str()));

    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| {
                m.delete_pilot(&reviewer, w, cx);
                // Its options menu (rename, look, delete) never opens.
                m.open_options(&reviewer, None, w, cx);
            })
        })
        .unwrap();
    cx.run_until_parked();
    let (kept, error, menu) = cx.update(|cx| {
        let m = s.manager.read(cx);
        (
            m.roster.get(&reviewer).is_some(),
            m.error.clone(),
            m.menu.is_some(),
        )
    });
    assert!(kept);
    assert_eq!(error.as_deref(), Some(pilot::SYSTEM_PILOT_LOCKED));
    assert!(!menu);
    // The user's own Pilot still has its menu.
    let user = s.id.clone();
    handle
        .update(cx, |_, w, cx| {
            s.manager
                .update(cx, |m, cx| m.open_options(&user, None, w, cx))
        })
        .unwrap();
    assert!(cx.update(|cx| s.manager.read(cx).menu.is_some()));
}

#[test]
fn the_learned_tab_lists_live_rules_with_their_day() {
    let text = "- 2026-11-28 · Unwrap is fine in tests (#7, @me)\n\
                - ~~2026-11-20 · Flag every unwrap~~ · superseded 2026-11-28\n\
                - Write findings as questions (#9, @maint)\n\n";
    assert_eq!(
        super::code_review::learned_rule_lines(text),
        vec![
            (
                "Unwrap is fine in tests (#7, @me)".to_string(),
                Some("2026-11-28".to_string())
            ),
            ("Write findings as questions (#9, @maint)".to_string(), None),
        ]
    );
    assert!(super::code_review::learned_rule_lines("").is_empty());
}

#[gpui::test]
fn a_reviewer_opens_on_what_it_learned(cx: &mut TestAppContext) {
    let s = setup(cx);
    let reviewer = reviewer(&s, cx);
    memory::edit(&s.root, &reviewer, Some("review"), |old| {
        Ok(memory::add(
            old,
            &memory::entry("Unwrap is fine in tests (#7, @me)", "2026-11-28", None).unwrap(),
        ))
    })
    .unwrap();
    let (tabs, shown) = cx.update(|cx| {
        let m = s.manager.read(cx);
        let pilot = m.roster.get(&reviewer).unwrap();
        (
            super::panels::tabs_for(pilot)
                .iter()
                .map(|(_, label)| *label)
                .collect::<Vec<_>>(),
            // Whatever tab the panel was on, a reviewer shows Learned.
            super::panels::shown_tab(pilot, super::panels::OverviewTab::Pipeline),
        )
    });
    assert_eq!(tabs, ["Learned", "Reviews"]);
    assert!(shown == super::panels::OverviewTab::Learned);
    // The user's own Pilot keeps its tabs.
    let own = cx.update(|cx| {
        let m = s.manager.read(cx);
        super::panels::tabs_for(m.roster.get(&s.id).unwrap()).len()
    });
    assert_eq!(own, 4);
    let rules = memory::read(&s.root, &reviewer, Some("review"))
        .unwrap()
        .text
        .unwrap_or_default();
    assert_eq!(
        super::code_review::learned_rule_lines(&rules)[0].0,
        "Unwrap is fine in tests (#7, @me)"
    );
}

#[gpui::test]
fn the_reviews_tab_lists_the_pull_requests_its_repo_review_read(cx: &mut TestAppContext) {
    use gespan_core::automations::AutomationRun;
    let s = setup(cx);
    let reviewer = reviewer(&s, cx);
    let run: AutomationRun = serde_json::from_value(json!({
        "id": "r1", "automationId": "a", "trigger": "manual", "scheduledFor": 0,
        "createdAt": 10, "startedAt": 20, "status": "succeeded",
        "sessionId": "s-1", "eventTitle": "Fix the seen-state (#585)"
    }))
    .unwrap();
    let row = super::code_review::ReviewRunRow::of(&run);
    assert_eq!(row.title, "Fix the seen-state (#585)");
    assert_eq!((row.status.as_str(), row.at), ("succeeded", 20));
    assert_eq!(row.session.as_deref(), Some("s-1"));
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.set_review_runs(HashMap::from([(REPO.to_string(), vec![row.clone()])]), cx)
        })
    });
    let shown = cx.update(|cx| {
        let m = s.manager.read(cx);
        let pilot = m.roster.get(&reviewer).unwrap();
        pilot
            .reviewed_repo()
            .and_then(|repo| m.review_runs.get(repo))
            .cloned()
            .unwrap_or_default()
    });
    assert_eq!(shown, vec![row]);
    // A run with no title yet still reads as a review.
    let untitled: AutomationRun = serde_json::from_value(json!({
        "id": "r2", "automationId": "a", "trigger": "manual", "scheduledFor": 0,
        "createdAt": 30, "status": "pending"
    }))
    .unwrap();
    let row = super::code_review::ReviewRunRow::of(&untitled);
    assert_eq!(
        (row.title.as_str(), row.at, row.session),
        ("Review", 30, None)
    );
}

#[gpui::test]
fn a_review_opens_inside_its_reviewers_page(cx: &mut TestAppContext) {
    let s = setup(cx);
    let reviewer = reviewer(&s, cx);
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    let row = super::code_review::ReviewRunRow {
        title: "Fix the seen-state (#585)".into(),
        status: "succeeded".into(),
        at: 20,
        session: Some("s-1".into()),
        native: Some(s.session),
    };
    s.manager.update_in(visual, |m, window, cx| {
        m.set_review_runs(HashMap::from([(REPO.to_string(), vec![row])]), cx);
        m.open(&reviewer, window, cx);
        m.select_overview(OverviewTab::Reviews, cx);
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    let review = visual
        .debug_bounds("pilot-review-run-0")
        .expect("the Reviews tab lists the review");
    visual.simulate_click(review.center(), Default::default());
    visual.run_until_parked();
    s.manager.read_with(visual, |m, _| {
        assert_eq!(m.route, Some(Route::Session(s.session)), "opened in place");
        assert_eq!(m.active.as_deref(), Some(reviewer.as_str()));
    });
}
