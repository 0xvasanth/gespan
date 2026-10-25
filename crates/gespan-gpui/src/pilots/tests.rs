use super::*;
use gespan_core::store::Store;
use gpui::{TestAppContext, WindowHandle};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex, mpsc};
struct Setup {
    window: WindowHandle<Shell>,
    manager: Entity<Pilots>,
    id: String,
    project: String,
    session: i64,
    root: PathBuf,
    store: Database,
}

#[gpui::test]
fn missing_document_uses_error_cards_without_moving_content(cx: &mut TestAppContext) {
    let s = setup(cx);
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    visual.refresh().unwrap();
    let body = visual.debug_bounds("main-pane-frame").unwrap();
    s.manager.update(visual, |m, cx| {
        m.failed_reports.insert("failed-report".into());
        m.open_document("deleted-document", cx);
    });
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert_eq!(
        s.manager
            .read_with(visual, |m, _| m.error.clone())
            .as_deref(),
        Some("Document unavailable. It may have been deleted.")
    );
    let error = visual.debug_bounds("pilot-error").unwrap();
    let viewport = s
        .window
        .update(visual, |_, w, _| w.viewport_size())
        .unwrap();
    assert_eq!(error.top(), px(12.));
    assert_eq!(error.right(), viewport.width - px(12.));
    assert_eq!(error.size.width, px(360.));
    assert_eq!(visual.debug_bounds("main-pane-frame").unwrap(), body);

    let close = visual.debug_bounds("pilot-error-close").unwrap();
    visual.simulate_click(close.center(), Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("pilot-error").is_none());
    s.manager.read_with(visual, |m, _| {
        assert!(m.error.is_none());
        assert!(m.failed_reports.is_empty());
        assert!(
            m.settings.is_some(),
            "dismissing an error keeps settings open"
        );
        assert_eq!(m.active.as_deref(), Some(s.id.as_str()));
    });
}

#[gpui::test]
fn saved_desktop_pilot_opens_in_actual_manager_without_replacing_its_conversation(
    cx: &mut TestAppContext,
) {
    let db = Store::open_gespan(std::path::Path::new(":memory:")).unwrap();
    let session = db.create("codex", "/saved-project").unwrap();
    let original = db.persisted_session_id(session.id).unwrap();
    let settings = Settings {
        model: Some("gpt-6.1-sol".into()),
        access: AccessMode::Auto,
        ..Default::default()
    };
    db.configure_agent(session.id, "codex", &settings).unwrap();
    let mut roster = Roster::default();
    roster.pilots.push(Pilot {
        id: "desktop-original-pilot".into(),
        session_id: Some(session.id),
        name: "Big Boss".into(),
        mascot: "ghost".into(),
        color: 0xe25a9e,
        projects: vec!["/saved-project".into()],
        ..Default::default()
    });
    roster.save(&db).unwrap();
    let summaries = db.sessions().unwrap();
    let store = Arc::new(Mutex::new(db));
    let database = store.clone();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Shell::new(database, summaries, "/saved-project".into(), None, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    let manager = window
        .update(cx, |shell, _, _| shell.pilot_manager())
        .unwrap();
    window
        .update(cx, |_, window, cx| {
            manager.update(cx, |m, cx| m.open("desktop-original-pilot", window, cx));
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        manager.update(cx, |m, cx| {
            m.ensure_chat("desktop-original-pilot", cx);
            let pilot = m.current().unwrap();
            assert_eq!(pilot.id, "desktop-original-pilot");
            assert_eq!(pilot.session_id, Some(session.id));
            assert_eq!(pilot.name, "Big Boss");
            assert_eq!(pilot.mascot, "ghost");
            let chat = m.chats[&pilot.id].read(cx);
            assert_eq!(chat.summary.id, session.id);
            assert_eq!(chat.summary.provider, "codex");
            assert_eq!(chat.settings.model.as_deref(), Some("gpt-6.1-sol"));
            assert_eq!(m.chats.len(), 1);
        });
    });
    let db = locked(&store).unwrap();
    assert_eq!(db.sessions().unwrap().len(), 1);
    assert_eq!(db.persisted_session_id(session.id).unwrap(), original);
}

#[gpui::test]
fn settings_reset_restores_only_pilot_soul_and_name(cx: &mut TestAppContext) {
    let s = setup(cx);
    pilot::files::save(&s.root, &s.id, None, "SOUL.md", "# Custom soul", None).unwrap();
    pilot::files::save(&s.root, &s.id, None, "MEMORY.md", "Keep this memory", None).unwrap();
    let before = cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.roster.get_mut(&s.id).unwrap().name = "Custom name".into();
            m.save(cx);
            m.roster.get(&s.id).unwrap().clone()
        })
    });
    cx.run_until_parked();
    let _reset = cx.update(|cx| s.manager.update(cx, |m, cx| m.reset_defaults(&s.id, cx)));
    cx.run_until_parked();
    let roster = Roster::load(&locked(&s.store).unwrap()).unwrap();
    let after = roster.get(&s.id).unwrap();
    assert!(after.name.is_empty());
    assert_eq!(after.session_id, before.session_id);
    assert_eq!(after.projects, before.projects);
    assert_eq!(after.habit_sessions, before.habit_sessions);
    let files = pilot::files::load(&s.root, &s.id, None).unwrap();
    assert_eq!(files.soul.as_deref(), Some(pilot::DEFAULT_SOUL));
    assert_eq!(files.memory, "Keep this memory");
}

#[gpui::test]
fn detail_pages_slide_over_the_overview_and_back_without_recreating_content(
    cx: &mut TestAppContext,
) {
    for kind in ["markdown", "html", "session"] {
        cx.update(|cx| cx.set_reduce_motion(true));
        let s = setup(cx);
        let session = s
            .window
            .update(cx, |shell, _, _| shell.workspace.active.unwrap())
            .unwrap();
        s.window
            .update(cx, |_, _, cx| {
                s.manager.update(cx, |m, cx| {
                    m.settings = None;
                    m.overview_tab = OverviewTab::Documents;
                    m.roster
                        .get_mut(&s.id)
                        .unwrap()
                        .documents
                        .push(pilot::Document {
                            id: "slide-document".into(),
                            kind: kind.into(),
                            title: "Slide document".into(),
                            summary: String::new(),
                            body: "# Document\n\nKeep this reader alive.".into(),
                            created_at: 1,
                            updated_at: 1,
                        });
                    cx.notify();
                });
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
        visual.run_until_parked();
        visual.update(|_, cx| cx.set_reduce_motion(false));
        let action = if kind == "session" {
            json!({"type":"session", "sessionId":session})
        } else {
            json!({"type":"document", "documentId":"slide-document"})
        };
        s.window
            .update(visual, |_, _, cx| {
                s.manager
                    .update(cx, |m, cx| m.card_action(&s.id, &action, cx))
            })
            .unwrap();
        visual.run_until_parked();
        let page: &'static str = Box::leak(
            format!(
                "pilot-detail-page-{}",
                if kind == "session" {
                    format!("session:{session}")
                } else {
                    "document:slide-document".into()
                }
            )
            .into_boxed_str(),
        );
        let advance = |visual: &mut gpui::VisualTestContext, ms| {
            let details = s.manager.read_with(visual, |m, _| m.details.clone());
            details.update(visual, |d, cx| d.elapsed(Duration::from_millis(ms), cx));
            visual
                .update_window(s.window.into(), |_, w, cx| {
                    w.simulate_next_frame(cx);
                    w.draw(cx).clear(cx);
                })
                .unwrap();
        };
        advance(visual, 70);
        let stack = visual.debug_bounds("pilot-detail-stack").unwrap();
        let incoming = visual.debug_bounds(page).unwrap();
        assert!(incoming.left() > stack.left() && incoming.left() < stack.right());
        assert_eq!(
            incoming.size, stack.size,
            "sliding pages keep their full layout size"
        );
        assert!(visual.debug_bounds("pilot-detail-base").unwrap().left() < stack.left());
        advance(visual, 1000);
        assert_eq!(visual.debug_bounds(page).unwrap().left(), stack.left());
        let reader = s.manager.read_with(visual, |m, cx| match kind {
            "html" => m.html_documents["slide-document"].entity_id(),
            "session" => m
                .owner
                .upgrade()
                .unwrap()
                .read(cx)
                .pilot_session_entity(session)
                .unwrap()
                .entity_id(),
            _ => m.documents["slide-document"].entity_id(),
        });
        assert_eq!(
            s.manager
                .read_with(visual, |m, cx| m.native_detail_content(cx)),
            kind == "html"
        );
        let back = visual.debug_bounds("pilot-page-back").unwrap().center();
        visual.simulate_click(back, Default::default());
        visual.run_until_parked();
        advance(visual, 70);
        let outgoing = visual
            .debug_bounds(page)
            .expect("Back keeps the outgoing page mounted");
        assert!(outgoing.left() > stack.left() && outgoing.left() < stack.right());
        advance(visual, 1000);
        assert!(visual.debug_bounds(page).is_none());
        assert_eq!(
            visual.debug_bounds("pilot-detail-base").unwrap().left(),
            stack.left()
        );
        assert!(
            !s.manager
                .read_with(visual, |m, cx| m.native_detail_content(cx))
        );

        visual.update(|_, cx| cx.set_reduce_motion(true));
        s.window
            .update(visual, |_, _, cx| {
                s.manager
                    .update(cx, |m, cx| m.card_action(&s.id, &action, cx))
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(visual.debug_bounds(page).unwrap().left(), stack.left());
        s.manager.read_with(visual, |m, cx| {
            let reopened = match kind {
                "html" => m.html_documents["slide-document"].entity_id(),
                "session" => m
                    .owner
                    .upgrade()
                    .unwrap()
                    .read(cx)
                    .pilot_session_entity(session)
                    .unwrap()
                    .entity_id(),
                _ => m.documents["slide-document"].entity_id(),
            };
            assert_eq!(reader, reopened);
        });
        visual.update(|_, cx| cx.set_reduce_motion(false));
        s.window
            .update(visual, |_, w, cx| {
                s.manager.update(cx, |m, cx| {
                    m.close(cx);
                    m.open(&s.id, w, cx);
                });
            })
            .unwrap();
        visual.run_until_parked();
        assert!(
            visual.debug_bounds(page).is_none(),
            "returning to a Pilot must not replay an old page's exit"
        );
        assert_eq!(
            visual.debug_bounds("pilot-detail-base").unwrap().left(),
            stack.left()
        );
    }
}
fn setup(cx: &mut TestAppContext) -> Setup {
    let dir = std::env::temp_dir().join(format!("native-pilots-{}", pilot::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let project = dir.to_string_lossy().to_string();
    let store = Arc::new(Mutex::new(Store::open(&dir.join("native.sqlite")).unwrap()));
    let db = store.clone();
    let cwd = project.clone();
    let window = cx.update(|cx| {
        cx.open_window(Default::default(), |_, cx| {
            cx.new(|cx| Shell::new(db, Vec::new(), cwd, None, cx))
        })
        .unwrap()
    });
    cx.run_until_parked();
    let manager = window
        .update(cx, |shell, _, cx| {
            shell.workspace.right_panel_open = true;
            cx.notify();
            shell.pilot_manager()
        })
        .unwrap();
    let handle: gpui::AnyWindowHandle = window.into();
    handle
        .update(cx, |_, w, cx| {
            manager.update(cx, |m, cx| m.create(vec![project.clone()], w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    let (id, session, root) = cx.update(|cx| {
        let m = manager.read(cx);
        let pilot = m.current().unwrap();
        (pilot.id.clone(), pilot.session_id.unwrap(), m.root.clone())
    });
    Setup {
        window,
        manager,
        id,
        project,
        session,
        root,
        store,
    }
}
fn call(
    s: &Setup,
    request: &str,
    action: &str,
    input: Value,
    cx: &mut TestAppContext,
) -> mpsc::Receiver<Result<Value, String>> {
    let (reply, rx) = mpsc::channel();
    let call = pilot::bridge::Call {
        session: s.session,
        request_id: request.into(),
        action: action.into(),
        input,
        reply,
    };
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| m.app_call(call, w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    rx
}
#[gpui::test]
fn settings_match_react_dialog_and_left_navigation_geometry(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    let dialog = visual.debug_bounds("pilot-settings-dialog").unwrap();
    let nav = visual.debug_bounds("pilot-settings-navigation").unwrap();
    let viewport = s
        .window
        .update(visual, |_, w, _| w.viewport_size())
        .unwrap();
    assert_eq!(
        dialog.size.width + px(2.),
        px(880.).min(viewport.width - px(24.))
    );
    assert!((dialog.center().x - viewport.width / 2.).abs() <= px(1.));
    assert_eq!(nav.size.width, px(192.));
    for section in ["General", "Soul", "Memory"] {
        let row = visual
            .debug_bounds(Box::leak(
                format!("pilot-settings-{section}").into_boxed_str(),
            ))
            .unwrap();
        let icon = visual
            .debug_bounds(Box::leak(
                format!("pilot-settings-icon-{section}").into_boxed_str(),
            ))
            .unwrap();
        let label = visual
            .debug_bounds(Box::leak(
                format!("pilot-settings-label-{section}").into_boxed_str(),
            ))
            .unwrap();
        assert_eq!(row.size.height, px(32.));
        assert_eq!(icon.left() - row.left(), px(10.));
        assert_eq!(label.left() - icon.right(), px(10.));
    }
    assert_eq!(
        visual.debug_bounds("pilot-look").unwrap().size,
        gpui::size(px(80.), px(80.))
    );
    assert_eq!(
        visual.debug_bounds("pilot-name-field").unwrap().size,
        gpui::size(px(256.), px(32.))
    );
}

#[gpui::test]
fn settings_color_popover_follows_its_mascot_when_the_window_resizes(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| {
        crate::shell::bind_keys(cx);
        crate::popover::bind(cx);
    });
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    let face = visual.debug_bounds("pilot-look").unwrap();
    visual.simulate_click(face.center(), Default::default());
    visual.run_until_parked();
    for size in [
        gpui::size(px(1300.), px(900.)),
        gpui::size(px(1050.), px(760.)),
    ] {
        visual.simulate_resize(size);
        visual.run_until_parked();
        let face = visual.debug_bounds("pilot-look").unwrap();
        let picker = visual.debug_bounds("pilot-settings-look-picker").unwrap();
        assert!((picker.center().x - face.center().x).abs() <= px(1.));
        assert!(picker.top() >= face.bottom());
    }
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    visual.update(|_, cx| {
        let m = s.manager.read(cx);
        assert!(m.picker.is_none());
        assert!(m.settings.is_some());
    });
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    visual.update(|_, cx| assert!(s.manager.read(cx).settings.is_none()));
}

#[gpui::test]
fn memory_editor_saves_with_enter_and_stays_open_for_the_next_fact(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(input::bind);
    s.window
        .update(cx, |_, _, cx| {
            s.manager.update(cx, |m, cx| {
                m.settings = Some(SettingsSection::Memory);
                m.memory_adding = true;
                cx.notify();
            })
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    s.window
        .update(visual, |_, w, cx| {
            let field = s.manager.read(cx).fields["memory-fact"].clone();
            w.focus(&field.focus_handle(cx), cx);
        })
        .unwrap();
    visual.simulate_input("Keep UI aligned with React.");
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert!(
        pilot::files::read(&s.root, &s.id, None, "MEMORY.md")
            .unwrap()
            .text
            .unwrap()
            .contains("Keep UI aligned with React.")
    );
    visual.update(|_, cx| {
        let m = s.manager.read(cx);
        assert!(m.memory_adding);
        assert!(m.fields["memory-fact"].read(cx).content.is_empty());
    });
}

#[gpui::test]
fn overview_activity_returns_to_its_previous_tab_and_uses_the_shared_reveal(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    s.window
        .update(cx, |_, window, cx| {
            s.manager.update(cx, |m, cx| {
                m.select_overview(OverviewTab::Documents, cx);
                assert!(m.overview_reveal.is_some());
                m.select_overview(OverviewTab::Activity, cx);
                assert!(m.overview_previous == OverviewTab::Documents);
                m.select_overview(m.overview_previous, cx);
                assert!(m.overview_tab == OverviewTab::Documents);
                cx.set_reduce_motion(true);
                assert!(
                    m.overview_reveal
                        .as_ref()
                        .unwrap()
                        .progress(OverviewTab::Documents, window, cx)
                        .is_none()
                );
            });
        })
        .unwrap();
}

#[gpui::test]
fn floating_chat_opens_after_releasing_the_manager_and_shares_the_runtime(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| s.manager.update(cx, |m, cx| m.open_floating(cx)));
    cx.run_until_parked();
    cx.update(|cx| s.manager.update(cx, |m, cx| m.open_floating(cx)));
    cx.run_until_parked();
    cx.update(|cx| {
        let manager = s.manager.read(cx);
        assert_eq!(manager.floating.len(), 1);
        assert_eq!(manager.chats.len(), 1);
        assert!(manager.chats.contains_key(&s.id));
    });
}

#[gpui::test]
fn note_actions_preserve_tags_when_updating_the_body(cx: &mut TestAppContext) {
    let s = setup(cx);
    let note = call(
        &s,
        "note-create",
        "notes.write",
        json!({"title":"QA note","body":"First","tags":["work","pilot"]}),
        cx,
    )
    .try_recv()
    .unwrap()
    .unwrap();
    let updated = call(
        &s,
        "note-update",
        "notes.write",
        json!({"id":note["id"],"title":"QA note","body":"Second"}),
        cx,
    )
    .try_recv()
    .unwrap()
    .unwrap();
    assert_eq!(updated["tags"], json!(["work", "pilot"]));
    assert_eq!(updated["body"], "Second");
}
#[gpui::test]
fn identity_and_context_editors_survive_switches_and_save_to_their_own_files(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.settings = Some(SettingsSection::Soul);
            let pilot = m.current().unwrap().clone();
            let _ = m.soul_page(&pilot, cx);
            let key = format!("file:{}:SOUL.md", s.id);
            m.fields[&key].update(cx, |f, cx| {
                f.set_text("# Soul\nKeep replies concise.\n".into(), cx)
            });
            m.save_open_file(cx);
        })
    });
    cx.run_until_parked();
    assert_eq!(
        pilot::files::read(&s.root, &s.id, None, "SOUL.md")
            .unwrap()
            .text
            .as_deref(),
        Some("# Soul\nKeep replies concise.\n")
    );
    let saved = Roster::load(&locked(&s.store).unwrap()).unwrap();
    assert_eq!(saved.pilots.len(), 1);
    assert_eq!(saved.pilots[0].session_id, Some(s.session));
    assert!(saved.hidden_sessions.contains(&s.session));
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(cx, |_, w, cx| {
            s.manager
                .update(cx, |m, cx| m.create(vec![s.project.clone()], w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            let pilot = m.current().unwrap().clone();
            assert_ne!(pilot.id, s.id);
            assert_ne!(pilot.mascot, m.roster.get(&s.id).unwrap().mascot);
            m.settings = Some(SettingsSection::Soul);
            let _ = m.soul_page(&pilot, cx);
            let key = format!("file:{}:SOUL.md", pilot.id);
            assert!(
                !m.fields[&key]
                    .read(cx)
                    .content
                    .contains("Keep replies concise")
            );
        })
    });
}
#[gpui::test]
fn app_actions_are_project_scoped_and_mutation_retries_are_idempotent(cx: &mut TestAppContext) {
    let s = setup(cx);
    assert!(
        call(
            &s,
            "outside",
            "sessions.list",
            json!({"project":"/not-assigned"}),
            cx
        )
        .try_recv()
        .unwrap()
        .is_err()
    );
    let input = json!({"name":"Morning digest","instructions":"Summarize changes without editing","schedule":{"kind":"daily","time":"09:00"}});
    let first = call(&s, "habit", "habits.add", input.clone(), cx)
        .try_recv()
        .unwrap()
        .unwrap();
    let retry = call(&s, "habit", "habits.add", input, cx)
        .try_recv()
        .unwrap()
        .unwrap();
    assert_eq!(first, retry);
    assert_eq!(habits::load(&s.root, &s.id).unwrap().len(), 1);
    assert!(
        call(&s, "habit", "habits.add", json!({"name":"different"}), cx)
            .try_recv()
            .unwrap()
            .is_err()
    );
    let start=call(&s,"draft","sessions.start",json!({"project":s.project,"prompt":"Review the project","title":"Review draft","draft":true}),cx).try_recv().unwrap().unwrap();
    let child = start["sessionId"].as_i64().unwrap();
    cx.update(|cx| {
        let m = s.manager.read(cx);
        assert_eq!(m.roster.get(&s.id).unwrap().launches.len(), 1);
        let owner = m.owner.upgrade().unwrap();
        let view = owner.read(cx).pilot_session_entity(child).unwrap();
        assert_eq!(view.read(cx).input.read(cx).content, "Review the project");
        assert!(!view.read(cx).busy);
        assert!(
            !m.hidden_session(child),
            "Pilot-started sessions are visible"
        );
    });
}
#[gpui::test]
fn queued_messages_are_saved_before_dispatch_and_do_not_run_while_blocked(cx: &mut TestAppContext) {
    let s = setup(cx);
    let message = pilot::new_id();
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.roster.get_mut(&s.id).unwrap().outbox.push(Message {
                sent_at: None,
                id: message.clone(),
                text: "Keep this queued".into(),
                internal: false,
                error: Some("Retry required".into()),
                attachments: Vec::new(),
                report: Vec::new(),
            });
            m.save(cx);
            m.configure_chat(&s.id, cx);
        })
    });
    cx.run_until_parked();
    let saved = Roster::load(&locked(&s.store).unwrap()).unwrap();
    assert_eq!(saved.pilots[0].outbox[0].id, message);
    assert_eq!(
        locked(&s.store)
            .unwrap()
            .load(s.session)
            .unwrap()
            .rows
            .len(),
        0
    );
}

#[gpui::test]
fn chat_composer_keeps_attach_under_the_box_and_opens_the_model_list(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| m.dismiss_settings(w, cx));
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    let field = visual.debug_bounds("pilot-composer").unwrap();
    let send = visual.debug_bounds("pilot-send").unwrap();
    let attach = visual.debug_bounds("pilot-attach").unwrap();
    let model = visual.debug_bounds("pilot-model").unwrap();
    // PilotComposer with controls: send inside the box, attach and the model
    // on the row beneath it, the model at its right end.
    assert!(field.contains(&send.center()));
    assert!(attach.top() >= field.bottom());
    assert!(model.top() >= field.bottom());
    assert!(model.left() > attach.right());
    assert_eq!(send.size, gpui::size(px(26.), px(26.)));
    assert_eq!(attach.size, gpui::size(px(26.), px(26.)));
    let chat = visual.update(|_, cx| s.manager.read(cx).chat().unwrap());
    handle
        .update(visual, |_, w, cx| {
            chat.update(cx, |c, cx| c.toggle_model_menu(w, cx))
        })
        .unwrap();
    visual.run_until_parked();
    visual.update(|_, cx| {
        let (models, menu) = chat.read(cx).pilot_menus_open();
        assert!(models, "the trigger opens the model list itself");
        assert!(!menu, "not the session composer's settings menu");
    });
}

#[gpui::test]
fn details_tabs_share_the_session_titlebar_in_both_layouts(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        crate::shell::bind_keys(cx);
    });
    let session = s
        .window
        .update(cx, |shell, w, cx| {
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                m.focus_active(w, cx);
                cx.notify();
            });
            shell.workspace.active.unwrap()
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    for classic in [false, true] {
        s.window
            .update(visual, |shell, _, cx| {
                shell.workspace.new_layout_boxy = classic;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        let panel = visual.debug_bounds("right-panel-frame").unwrap();
        let tabs = visual.debug_bounds("pilot-overview-tabs").unwrap();
        let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
        let title_tabs = visual.debug_bounds("title-tab-strip").unwrap();
        assert!((tabs.top() - title_tabs.top()).abs() <= px(0.5));
        assert_eq!(tabs.size.height, px(TITLE_BAR_HEIGHT));
        assert_eq!(
            tabs.left(),
            panel.left() + px(if classic { 6. } else { 7. })
        );
        let activity = visual.debug_bounds("pilot-activity").unwrap();
        let search = visual.debug_bounds("titlebar-file-search").unwrap();
        assert_eq!(search.left() - activity.right(), px(6.));
        assert_eq!(toggle.left() - search.right(), px(4.));
        assert!(tabs.bottom() <= panel.top() + px(if classic { TITLE_BAR_HEIGHT } else { 0. }));
        assert_eq!(
            visual.debug_bounds("pilot-session-list").unwrap().top(),
            panel.top() + px(if classic { TITLE_BAR_HEIGHT } else { 0. })
        );
        assert!(
            visual.debug_bounds("pilot-overview-toggle").is_none(),
            "only the shell owns the panel toggle"
        );
        assert!(title_tabs.right() <= tabs.left());
        s.window
            .update(visual, |shell, w, cx| shell.open(session, w, cx))
            .unwrap();
        visual.run_until_parked();
        if !s
            .window
            .update(visual, |shell, _, _| shell.workspace.right_panel_open)
            .unwrap()
        {
            let button = visual.debug_bounds("toggle-right-panel").unwrap();
            visual.simulate_click(button.center(), Default::default());
            visual.run_until_parked();
        }
        assert_eq!(
            visual.debug_bounds("toggle-right-panel").unwrap(),
            toggle,
            "the toggle stays put when changing tab types"
        );
        let selector = if classic {
            "right-panel-explorer"
        } else {
            "project-tabs-separator"
        };
        assert_eq!(
            visual.debug_bounds(selector).unwrap().center().y,
            tabs.center().y
        );
        s.window
            .update(visual, |_, w, cx| {
                s.manager.update(cx, |m, cx| m.open(&s.id, w, cx))
            })
            .unwrap();
        visual.run_until_parked();
    }
}

#[gpui::test]
fn titlebar_details_tabs_switch_pages_and_keep_activity_and_habit_actions(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    s.window
        .update(cx, |shell, _, cx| {
            shell.workspace.right_panel_width = 400.;
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                cx.notify();
            })
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    for (selector, tab) in [
        ("pilot-overview-Documents", OverviewTab::Documents),
        ("pilot-overview-Habits", OverviewTab::Habits),
    ] {
        let button = visual.debug_bounds(selector).unwrap();
        visual.simulate_click(button.center(), Default::default());
        visual.run_until_parked();
        assert!(s.manager.read_with(visual, |m, _| m.overview_tab == tab));
        assert!(visual.debug_bounds(selector).unwrap().size.width > px(26.));
    }
    let add = visual.debug_bounds("pilot-new-habit").unwrap();
    visual.simulate_click(add.center(), Default::default());
    visual.run_until_parked();
    assert!(
        s.manager
            .read_with(visual, |m, _| matches!(m.route, Some(Route::NewHabit)))
    );
    assert!(visual.debug_bounds("pilot-overview-tabs").is_some());
    let sessions = visual.debug_bounds("pilot-overview-Sessions").unwrap();
    visual.simulate_click(sessions.center(), Default::default());
    visual.run_until_parked();
    assert!(s.manager.read_with(visual, |m, _| m.route.is_none()
        && m.overview_tab == OverviewTab::Sessions));
    for expected in [OverviewTab::Activity, OverviewTab::Sessions] {
        let activity = visual.debug_bounds("pilot-activity").unwrap();
        visual.simulate_click(activity.center(), Default::default());
        visual.run_until_parked();
        assert!(
            s.manager
                .read_with(visual, |m, _| m.overview_tab == expected)
        );
    }
}

#[gpui::test]
fn titlebar_divider_resizes_pilot_details_and_closed_panels_keep_the_corner(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    s.window
        .update(cx, |shell, _, cx| {
            // The titlebar divider belongs to the rounded frame.
            shell.workspace.new_layout_boxy = false;
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                cx.notify();
            })
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    for width in [220., 280., 400., 500.] {
        s.window
            .update(visual, |shell, _, cx| {
                shell.workspace.right_panel_width = width;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        assert_eq!(
            visual
                .debug_bounds("project-tabs-separator")
                .unwrap()
                .left(),
            visual.debug_bounds("right-panel-frame").unwrap().left()
        );
        let activity = visual.debug_bounds("pilot-activity").unwrap();
        let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
        assert!(
            activity.right() < toggle.left(),
            "Pilot controls fit the session sidebar's minimum width"
        );
    }
    let start = visual
        .debug_bounds("titlebar-right-panel-resize")
        .unwrap()
        .center();
    let end = start - gpui::point(px(40.), px(0.));
    visual.simulate_mouse_down(start, MouseButton::Left, Default::default());
    visual.simulate_mouse_move(end, MouseButton::Left, Default::default());
    visual.simulate_mouse_up(end, MouseButton::Left, Default::default());
    visual.run_until_parked();
    s.window
        .update(visual, |shell, _, _| {
            assert_eq!(shell.workspace.right_panel_width, 540.);
        })
        .unwrap();
    assert_eq!(
        visual
            .debug_bounds("project-tabs-separator")
            .unwrap()
            .left(),
        visual.debug_bounds("right-panel-frame").unwrap().left()
    );
    for classic in [false, true] {
        s.window
            .update(visual, |shell, _, cx| {
                shell.workspace.new_layout_boxy = classic;
                shell.git.update(cx, |g, cx| g.set_project("~".into(), cx));
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        let open = visual.debug_bounds("toggle-right-panel").unwrap();
        visual.simulate_click(open.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("pilot-overview-tabs").is_none());
        assert!(visual.debug_bounds("pilot-overview-panel").is_none());
        assert_eq!(visual.debug_bounds("toggle-right-panel").unwrap(), open);
        visual.simulate_click(open.center(), Default::default());
        visual.run_until_parked();
        assert!(visual.debug_bounds("pilot-overview-tabs").is_some());
    }
}

#[gpui::test]
fn opening_a_session_card_uses_the_shared_sidebar_state(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let session = s
        .window
        .update(cx, |shell, _, cx| {
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                cx.notify();
            });
            shell.workspace.active.unwrap()
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    let toggle = visual.debug_bounds("toggle-right-panel").unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();
    assert!(visual.debug_bounds("right-panel-frame").is_none());
    s.window
        .update(visual, |_, _, cx| {
            s.manager.update(cx, |m, cx| {
                m.card_action(&s.id, &json!({"type":"session", "sessionId":session}), cx);
            })
        })
        .unwrap();
    visual.run_until_parked();
    let details = visual.debug_bounds("right-panel-frame").unwrap();
    assert!(s.manager.read_with(
        visual,
        |m, _| matches!(m.route, Some(Route::Session(id)) if id == session)
    ));
    s.window
        .update(visual, |shell, w, cx| {
            assert!(shell.workspace.right_panel_open);
            shell.open(session, w, cx);
        })
        .unwrap();
    visual.run_until_parked();
    assert_eq!(visual.debug_bounds("right-panel-frame").unwrap(), details);
}

#[gpui::test]
fn long_habit_draft_keeps_fields_and_schedule_inside_the_scrolling_page(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| {
        cx.set_reduce_motion(true);
        input::bind(cx);
        crate::popover::bind(cx);
    });
    s.window
        .update(cx, |shell, _, cx| {
            shell.workspace.right_panel_width = 220.;
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                m.overview_tab = OverviewTab::Habits;
                m.route = Some(Route::NewHabit);
                cx.notify();
            });
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    s.manager.update(visual, |m, cx| {
        m.fields["habit-name"].update(cx, |f, cx| {
            f.set_text(
                "A long but valid habit name that fills the narrow panel".into(),
                cx,
            )
        });
        m.fields["habit-instructions"].update(cx, |f, cx| {
            f.set_text(
                "Report useful updates on the current project.\n".repeat(65),
                cx,
            )
        });
        m.fields["habit-kind"].update(cx, |f, cx| f.set_text("weekly".into(), cx));
        cx.notify();
    });
    visual.run_until_parked();
    let body = visual.debug_bounds("pilot-habit-editor").unwrap();
    let name = visual.debug_bounds("pilot-habit-name-field").unwrap();
    let schedule = visual.debug_bounds("pilot-habit-schedule").unwrap();
    let prompt = visual.debug_bounds("pilot-habit-prompt-field").unwrap();
    let create = visual.debug_bounds("pilot-habit-save").unwrap();
    assert!(name.size.height >= px(36.));
    assert!(schedule.top() >= name.bottom());
    assert!(prompt.top() > schedule.bottom());
    assert!(prompt.size.height >= px(300.));
    assert!(prompt.size.height <= px(328.));
    for bounds in [name, schedule, prompt] {
        assert!(bounds.left() >= body.left());
        assert!(bounds.right() <= body.right() + px(1.));
    }
    assert!(create.bottom() <= body.top());
    let day = visual.debug_bounds("pilot-habit-weekday-picker").unwrap();
    assert!(day.left() >= schedule.left());
    assert!(day.right() <= schedule.right() + px(1.));
    visual.simulate_click(day.center(), Default::default());
    visual.run_until_parked();
    // Escape returns focus to the trigger; arrow keys reopen the list with
    // the current weekday highlighted, and Enter retains that selection.
    visual.simulate_keystrokes("escape");
    visual.run_until_parked();
    assert!(s.manager.read_with(visual, |m, _| m.picker.is_none()));
    visual.simulate_keystrokes("down");
    visual.run_until_parked();
    assert!(s.manager.read_with(visual, |m, _| m.picker.is_some()));
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    assert_eq!(
        s.manager
            .read_with(visual, |m, cx| m.value("habit-weekday", cx)),
        "1"
    );
    assert!(s.manager.read_with(visual, |m, _| m.picker.is_none()));
    visual.simulate_click(day.center(), Default::default());
    visual.run_until_parked();
    let saturday = visual.debug_bounds("menu-action-6").unwrap();
    visual.simulate_click(saturday.center(), Default::default());
    visual.run_until_parked();
    assert_eq!(
        s.manager
            .read_with(visual, |m, cx| m.value("habit-weekday", cx)),
        "6"
    );
    // Invalid schedules keep the full draft in place instead of navigating away.
    s.manager.update(visual, |m, cx| {
        m.fields["habit-time"].update(cx, |f, cx| f.set_text("25:00".into(), cx));
    });
    visual.run_until_parked();
    visual.simulate_click(create.center(), Default::default());
    visual.run_until_parked();
    assert!(
        s.manager
            .read_with(visual, |m, _| m.route == Some(Route::NewHabit))
    );
    assert!(s.manager.read_with(visual, |m, cx| {
        m.fields["habit-instructions"].read(cx).content.len() > 2000
    }));
    assert!(habits::load(&s.root, &s.id).unwrap().is_empty());
    s.manager.update(visual, |m, cx| {
        m.fields["habit-time"].update(cx, |f, cx| f.set_text("09:00".into(), cx));
    });
    visual.run_until_parked();
    visual.simulate_click(create.center(), Default::default());
    visual.run_until_parked();
    let saved = habits::load(&s.root, &s.id).unwrap();
    assert_eq!(saved.len(), 1);
    assert!(saved[0].instructions.len() > 2000);
    assert_eq!(saved[0].schedule.schedule_kind, "weekly");
    assert_eq!(saved[0].schedule.day_of_week, 6);
    assert!(s.manager.read_with(visual, |m, _| m.route.is_none()));
}

#[gpui::test]
fn habit_detail_preserves_long_content_history_and_actions_without_edit_fields(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    let mut h = Habit::new(
        "A long habit title that should truncate before the action buttons".into(),
        "Review changes and report anything that needs attention.".into(),
        habits::Schedule::default(),
        gespan_core::time::now_ms(),
    )
    .unwrap();
    h.id = "layout-history".into();
    h.runs.push(habits::Run {
        at: gespan_core::time::now_ms() - 60_000,
        duration_ms: 72_000,
        outcome: "posted".into(),
        report: Some("**Result**\n\nThe project is ready to review.".into()),
        error: None,
    });
    habits::update(&s.root, &s.id, |all| {
        all.push(h.clone());
        Ok(())
    })
    .unwrap();
    s.window
        .update(cx, |shell, _, cx| {
            shell.workspace.right_panel_width = 280.;
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                m.overview_tab = OverviewTab::Habits;
                m.habits = vec![h.clone()];
                m.route = Some(Route::Habit(h.id.clone()));
                cx.notify();
            });
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    assert!(
        s.manager
            .read_with(visual, |m, _| !m.fields.contains_key("habit-name"))
    );
    let body = visual.debug_bounds("pilot-habit-editor").unwrap();
    let description = visual.debug_bounds("pilot-habit-instructions").unwrap();
    let row_id = format!("pilot-habit-history-{}-{}", h.id, h.runs[0].at);
    let row = visual
        .debug_bounds(Box::leak(row_id.into_boxed_str()))
        .unwrap();
    assert!(row.top() > description.bottom());
    assert!(description.right() <= body.right());
    for selector in [
        "pilot-habit-run",
        "pilot-habit-toggle",
        "pilot-habit-delete",
    ] {
        let action = visual.debug_bounds(selector).unwrap();
        assert!(action.bottom() <= body.top());
        assert!(action.right() <= body.right());
    }
    visual.simulate_click(row.center(), Default::default());
    visual.run_until_parked();
    assert!(
        visual
            .debug_bounds(Box::leak(
                format!("pilot-habit-report-{}-{}", h.id, h.runs[0].at).into_boxed_str()
            ))
            .is_some()
    );
    let toggle = visual.debug_bounds("pilot-habit-toggle").unwrap();
    visual.simulate_click(toggle.center(), Default::default());
    visual.run_until_parked();
    assert!(!habits::load(&s.root, &s.id).unwrap()[0].enabled);
    assert!(
        s.manager
            .read_with(visual, |m, _| m.route == Some(Route::Habit(h.id.clone())))
    );
    let remove = visual.debug_bounds("pilot-habit-delete").unwrap();
    visual.simulate_click(remove.center(), Default::default());
    visual.run_until_parked();
    assert!(habits::load(&s.root, &s.id).unwrap().is_empty());
    assert!(s.manager.read_with(visual, |m, _| m.route.is_none()));
}

#[gpui::test]
fn new_habit_click_transfers_composer_focus_after_the_animated_page_mounts(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(false));
    s.window
        .update(cx, |_, window, cx| {
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                m.overview_tab = OverviewTab::Habits;
                m.route = None;
                m.focus_active(window, cx);
                cx.notify();
            });
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    s.window
        .update(visual, |_, window, cx| {
            s.manager.read_with(cx, |m, cx| {
                assert!(
                    m.chat()
                        .unwrap()
                        .read(cx)
                        .input
                        .focus_handle(cx)
                        .is_focused(window)
                );
            });
        })
        .unwrap();
    let advance_focus_frame = |visual: &mut gpui::VisualTestContext| {
        visual
            .update_window(s.window.into(), |_, window, cx| {
                // Use the untyped window context: callbacks and drawing may
                // update the root Shell, so it must not already be borrowed.
                // Parking executor tasks does not deliver platform frame callbacks.
                // Flush the requested next frame before drawing its focused field.
                assert!(window.simulate_next_frame(cx) > 0);
                window.draw(cx).clear(cx);
            })
            .unwrap();
        visual.run_until_parked();
    };
    // Follow the actual header action, with the old composer still focused.
    let add = visual.debug_bounds("pilot-new-habit").unwrap();
    visual.simulate_click(add.center(), Default::default());
    visual.run_until_parked();
    advance_focus_frame(visual);
    s.window
        .update(visual, |_, window, cx| {
            s.manager.read_with(cx, |m, cx| {
                assert_eq!(m.route, Some(Route::NewHabit));
                assert!(m.fields["habit-name"].focus_handle(cx).is_focused(window));
                assert!(
                    !m.chat()
                        .unwrap()
                        .read(cx)
                        .input
                        .focus_handle(cx)
                        .is_focused(window)
                );
            });
        })
        .unwrap();
    let page = "pilot-detail-page-new-habit";
    assert!(visual.debug_bounds(page).is_some());
    // Complete the real page transition, then leave/reopen through its controls.
    for reopen in [false, true] {
        visual
            .update_window(s.window.into(), |_, window, cx| {
                let details = s.manager.read(cx).details.clone();
                details.update(cx, |v, cx| v.elapsed(std::time::Duration::from_secs(1), cx));
                window.draw(cx).clear(cx);
            })
            .unwrap();
        let back = visual.debug_bounds("pilot-page-back").unwrap();
        visual.simulate_click(back.center(), Default::default());
        visual.run_until_parked();
        assert!(s.manager.read_with(visual, |m, _| m.route.is_none()));
        if !reopen {
            let add = visual.debug_bounds("pilot-new-habit").unwrap();
            visual.simulate_click(add.center(), Default::default());
            visual.run_until_parked();
            advance_focus_frame(visual);
            s.window
                .update(visual, |_, window, cx| {
                    s.manager.read_with(cx, |m, cx| {
                        assert!(m.fields["habit-name"].focus_handle(cx).is_focused(window))
                    });
                })
                .unwrap();
        }
    }
}

#[gpui::test]
fn sidebar_pilot_only_marks_unread_replies_and_swaps_the_mark_for_options_on_hover(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    s.window
        .update(cx, |_, _, cx| {
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                m.close(cx);
            });
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    let unread_selector: &'static str =
        Box::leak(format!("pilot-rail-unread-{}", s.id).into_boxed_str());
    let options_selector: &'static str =
        Box::leak(format!("pilot-rail-options-{}", s.id).into_boxed_str());
    let row_selector: &'static str = Box::leak(format!("pilot-rail-{}", s.id).into_boxed_str());
    let away = gpui::point(px(5.), px(5.));
    visual.simulate_mouse_move(away, None, Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    let row = visual.debug_bounds(row_selector).unwrap();
    assert!(visual.debug_bounds(unread_selector).is_none());
    assert!(visual.debug_bounds(options_selector).is_none());
    // Check the painted output too: an idle Pilot used to paint a gray
    // status dot here, even with no unread reply and no pointer over it.
    s.window
        .update(visual, |_, window, _| {
            let scale = window.scale_factor();
            assert!(!window.painted_quads().iter().any(|quad| {
                let bounds = quad.bounds;
                bounds.left().0 >= f32::from(row.right() - px(28.)) * scale
                    && bounds.right().0 <= f32::from(row.right() - px(4.)) * scale
                    && bounds.top().0 >= f32::from(row.top()) * scale
                    && bounds.bottom().0 <= f32::from(row.bottom()) * scale
                    && (3. * scale..=8. * scale).contains(&bounds.size.width.0)
                    && (3. * scale..=8. * scale).contains(&bounds.size.height.0)
            }));
        })
        .unwrap();

    let post = |role, text: &str, visual: &mut gpui::VisualTestContext| {
        s.window
            .update(visual, |_, _, cx| {
                let chat = s.manager.read(cx).chats[&s.id].clone();
                chat.update(cx, |v, cx| v.pilot_post(role, text.into(), cx))
                    .unwrap();
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
    };
    post("user", "My question", visual);
    assert!(visual.debug_bounds(unread_selector).is_none());
    post("assistant", "A new reply", visual);
    let unread = visual
        .debug_bounds(unread_selector)
        .expect("an unseen assistant reply has an accent circle");
    assert_eq!(unread.size, gpui::size(px(6.), px(6.)));
    assert!(visual.debug_bounds(options_selector).is_none());
    let before_hover = visual.debug_bounds(row_selector).unwrap();
    visual.simulate_mouse_move(before_hover.center(), None, Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(
        visual.debug_bounds(unread_selector).is_none(),
        "options replace the unread circle on hover"
    );
    let options = visual.debug_bounds(options_selector).unwrap();
    assert_eq!(options.size, gpui::size(px(24.), px(24.)));
    assert_eq!(options.center(), unread.center());
    assert_eq!(visual.debug_bounds(row_selector).unwrap(), before_hover);
    visual.simulate_click(options.center(), Default::default());
    visual.run_until_parked();
    s.manager.read_with(visual, |m, _| {
        assert!(m.menu.is_some(), "the hover action opens Pilot options");
        assert!(m.active.is_none(), "options must not open the Pilot");
    });

    s.window
        .update(visual, |_, window, cx| {
            s.manager.update(cx, |m, cx| {
                m.menu = None;
                m.open(&s.id, window, cx);
            });
        })
        .unwrap();
    visual.simulate_mouse_move(away, None, Default::default());
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds(unread_selector).is_none());
    s.window
        .update(visual, |_, _, cx| {
            s.manager.update(cx, |m, cx| {
                assert_eq!(
                    m.roster.get(&s.id).unwrap().seen_rows,
                    m.chats[&s.id].read(cx).pilot_rows(cx).len()
                );
                m.close(cx);
            });
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(
        visual.debug_bounds(unread_selector).is_none(),
        "a reply stays read when the Pilot closes"
    );
}

#[gpui::test]
fn sidebar_right_click_opens_pilot_menu_outside_the_pilot_and_edits_its_look(
    cx: &mut TestAppContext,
) {
    cx.update(input::bind);
    let s = setup(cx);
    cx.update(|cx| cx.set_reduce_motion(true));
    // Leave the Pilot, as when a session is open: only the rail remains.
    s.window
        .update(cx, |_, _, cx| {
            s.manager.update(cx, |m, cx| {
                m.settings = None;
                m.active = None;
                cx.notify();
            })
        })
        .unwrap();
    let visual = &mut gpui::VisualTestContext::from_window(s.window.into(), cx);
    visual.run_until_parked();
    let row = s
        .manager
        .read_with(visual, |m, _| m.rail_bounds[&s.id].get())
        .expect("the sidebar lists the Pilot");
    let at = row.center();
    visual.simulate_event(gpui::MouseDownEvent {
        position: at,
        button: MouseButton::Right,
        modifiers: Default::default(),
        click_count: 1,
        first_mouse: false,
    });
    visual.simulate_event(gpui::MouseUpEvent {
        position: at,
        button: MouseButton::Right,
        modifiers: Default::default(),
        click_count: 1,
    });
    visual.run_until_parked();
    // PilotMenu opens beside the pointer with ten mascots and nine presets
    // plus the custom swatch, all in single rows.
    let first = visual
        .debug_bounds("pilot-mascot-0")
        .expect("right-click opens the Pilot's menu");
    let last = visual.debug_bounds("pilot-mascot-9").unwrap();
    assert_eq!(first.top(), last.top());
    assert_eq!(first.size.height, px(32.));
    assert!(first.left() >= at.x);
    let custom = visual.debug_bounds("pilot-custom-color").unwrap();
    assert_eq!(
        visual.debug_bounds("pilot-color-0").unwrap().top(),
        custom.top()
    );
    let pink = visual.debug_bounds("pilot-color-4").unwrap().center();
    visual.simulate_click(pink, Default::default());
    visual.run_until_parked();
    let color = s
        .manager
        .read_with(visual, |m, _| m.roster.get(&s.id).unwrap().color);
    assert_eq!(color, pilot::COLORS_PILOT[4]);
    // Enter in the name field commits and closes the menu.
    let name = visual.debug_bounds("pilot-options-name").unwrap();
    visual.simulate_click(name.center(), Default::default());
    visual.simulate_keystrokes("enter");
    visual.run_until_parked();
    s.manager
        .read_with(visual, |m, _| assert!(m.menu.is_none()));
    visual.refresh().unwrap();
    visual
        .update_window(s.window.into(), |_, window, cx| {
            window.simulate_next_frame(cx);
            window.draw(cx).clear(cx);
        })
        .unwrap();
    assert!(visual.debug_bounds("pilot-mascot-0").is_none());
}

#[gpui::test]
fn a_pilot_turn_begun_inside_a_pilots_update_moves_the_pilots_card(cx: &mut TestAppContext) {
    let s = setup(cx);
    let store = s.store.clone();
    let db = store.lock().unwrap().database_path().to_path_buf();
    let mut service = gespan_core::board::Service::open(&db).unwrap();
    let board = service.create("Work").unwrap();
    let backlog = service.get(&board.id).unwrap().columns[0].id.clone();
    let card = service.card_create(&backlog, "Pilot work", "").unwrap();
    service
        .card_link(&card.id, Some("pilot"), Some(&s.id))
        .unwrap();
    // Pilot chats start turns from inside a Pilots update.
    cx.update(|cx| {
        s.manager.update(cx, |_, cx| {
            crate::board::turns::turn_began(store.clone(), s.session, 1, cx)
        })
    });
    cx.run_until_parked();
    let detail = service.get(&board.id).unwrap();
    let column = detail
        .columns
        .iter()
        .find(|c| c.id == detail.cards[0].column_id)
        .unwrap();
    assert_eq!(column.name, "In Progress");
}
/// Two monitored children started from the Pilot chat's current turn.
fn delegated_pair(s: &Setup, cx: &mut TestAppContext) -> (i64, i64) {
    let mut children = Vec::new();
    for (request, title) in [("child-a", "Child A"), ("child-b", "Child B")] {
        let start = call(
            s,
            request,
            "sessions.start",
            json!({"project":s.project,"prompt":"Do the work","title":title,"draft":true}),
            cx,
        )
        .try_recv()
        .unwrap()
        .unwrap();
        children.push(start["sessionId"].as_i64().unwrap());
    }
    cx.update(|cx| {
        s.manager.update(cx, |m, _| {
            let pilot = m.roster.get_mut(&s.id).unwrap();
            for launch in &mut pilot.launches {
                launch.notify_on_complete = true;
            }
            // A failed message at the head holds the queue, so reports stay visible in the
            // outbox instead of starting a provider turn.
            pilot.outbox.push(Message {
                sent_at: None,
                id: pilot::new_id(),
                text: "Held".into(),
                internal: false,
                error: Some("Retry required".into()),
                attachments: Vec::new(),
                report: Vec::new(),
            });
        })
    });
    (children[0], children[1])
}
fn set_child(s: &Setup, child: i64, busy: bool, cx: &mut TestAppContext) {
    cx.update(|cx| {
        let owner = s.manager.read(cx).owner.upgrade().unwrap();
        let view = owner.read(cx).pilot_session_entity(child).unwrap();
        view.update(cx, |v, _| {
            v.busy = busy;
            v.summary.has_messages = true;
        });
    });
}
fn launch_of(s: &Setup, child: i64, cx: &mut TestAppContext) -> pilot::Launch {
    cx.update(|cx| {
        s.manager
            .read(cx)
            .roster
            .get(&s.id)
            .unwrap()
            .launches
            .iter()
            .find(|l| l.session_id == child)
            .cloned()
            .unwrap()
    })
}
/// Completion reports queued for the Pilot chat.
fn delegated_reports(s: &Setup, cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        s.manager
            .read(cx)
            .roster
            .get(&s.id)
            .unwrap()
            .outbox
            .iter()
            .filter(|q| q.internal && q.text.starts_with("[Delegated work completed]"))
            .map(|q| q.text.clone())
            .collect()
    })
}
const ONE_STOPPED: &str = "[Delegated work completed]\nA session you delegated has stopped.";
#[gpui::test]
fn a_finished_child_reports_while_its_sibling_keeps_running(cx: &mut TestAppContext) {
    let s = setup(cx);
    let (a, b) = delegated_pair(&s, cx);
    set_child(&s, a, false, cx);
    set_child(&s, b, true, cx);
    cx.update(|cx| s.manager.update(cx, |m, cx| m.check_completions(cx)));
    cx.run_until_parked();
    let reports = delegated_reports(&s, cx);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(reports[0].starts_with(ONE_STOPPED), "{}", reports[0]);
    assert!(reports[0].contains(&format!("Session {a}:")));
    assert!(!reports[0].contains(&format!("Session {b}:")));
    assert!(launch_of(&s, a, cx).completed);
    assert!(!launch_of(&s, b, cx).completed);

    // A's report is still queued, so B joins it instead of becoming a second turn.
    set_child(&s, b, false, cx);
    cx.update(|cx| s.manager.update(cx, |m, cx| m.check_completions(cx)));
    cx.run_until_parked();
    let reports = delegated_reports(&s, cx);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_both(&reports[0], a, b);
    assert!(launch_of(&s, b, cx).completed);
}
const TWO_STOPPED: &str = "[Delegated work completed]\n2 sessions you delegated have stopped.";
fn assert_both(report: &str, a: i64, b: i64) {
    assert!(report.starts_with(TWO_STOPPED), "{report}");
    let (at, bt) = (
        report.find(&format!("Session {a}:")).expect("names A"),
        report.find(&format!("Session {b}:")).expect("names B"),
    );
    assert!(
        at < bt,
        "a report lists sessions in the order they were picked up and appends later ones; A came first"
    );
    assert_eq!(report.matches("[Delegated work completed]").count(), 1);
}
#[gpui::test]
fn a_sibling_that_stops_while_a_report_is_built_joins_that_report(cx: &mut TestAppContext) {
    let s = setup(cx);
    let (a, b) = delegated_pair(&s, cx);
    set_child(&s, a, false, cx);
    set_child(&s, b, true, cx);
    cx.update(|cx| s.manager.update(cx, |m, cx| m.check_completions(cx)));
    // B stops, and the chat checks again, before A's report has been built.
    set_child(&s, b, false, cx);
    cx.update(|cx| s.manager.update(cx, |m, cx| m.check_completions(cx)));
    cx.run_until_parked();
    let reports = delegated_reports(&s, cx);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_both(&reports[0], a, b);
    assert!(launch_of(&s, a, cx).completed && launch_of(&s, b, cx).completed);
}
#[gpui::test]
fn a_session_dismissed_while_its_report_is_built_never_reports(cx: &mut TestAppContext) {
    let s = setup(cx);
    let (a, b) = delegated_pair(&s, cx);
    set_child(&s, a, false, cx);
    set_child(&s, b, true, cx);
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.check_completions(cx);
            // The Pilot stops, archives or deletes A before the report lands.
            let launches = &mut m.roster.get_mut(&s.id).unwrap().launches;
            launches
                .iter_mut()
                .find(|l| l.session_id == a)
                .unwrap()
                .dismissed = true;
        })
    });
    cx.run_until_parked();
    assert!(delegated_reports(&s, cx).is_empty());
    assert!(!launch_of(&s, a, cx).completed);

    // A later sibling still reports, alone.
    set_child(&s, b, false, cx);
    cx.update(|cx| s.manager.update(cx, |m, cx| m.check_completions(cx)));
    cx.run_until_parked();
    let reports = delegated_reports(&s, cx);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(reports[0].starts_with(ONE_STOPPED), "{}", reports[0]);
    assert!(reports[0].contains(&format!("Session {b}:")));
    assert!(!reports[0].contains(&format!("Session {a}:")));
}
#[gpui::test]
fn a_rearmed_launch_is_not_completed_by_a_stale_report(cx: &mut TestAppContext) {
    let s = setup(cx);
    let (a, b) = delegated_pair(&s, cx);
    set_child(&s, a, false, cx);
    set_child(&s, b, true, cx);
    let rearmed = cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.check_completions(cx);
            // `sessions.send` re-arms the launch for a new turn while the report is being built.
            let launches = &mut m.roster.get_mut(&s.id).unwrap().launches;
            let launch = launches.iter_mut().find(|l| l.session_id == a).unwrap();
            launch.origin_turn += 1;
            launch.completed = false;
            launch.origin_turn
        })
    });
    set_child(&s, a, true, cx);
    cx.run_until_parked();
    assert_eq!(delegated_reports(&s, cx).len(), 1);
    let launch = launch_of(&s, a, cx);
    assert_eq!(launch.origin_turn, rearmed);
    assert!(
        !launch.completed,
        "the stale report must not close the new turn"
    );
    // B shares the reported turn but was not in the report.
    assert!(!launch_of(&s, b, cx).completed);
}
fn mailbox_texts(run: &RunningHabit) -> Vec<String> {
    run.mailbox.iter().map(|m| m.text.clone()).collect()
}
/// A habit run, standing in a third session, that launched both delegated children.
fn habit_run(s: &Setup, cx: &mut TestAppContext) -> i64 {
    let start = call(
        s,
        "habit-run",
        "sessions.start",
        json!({"project":s.project,"prompt":"Scheduled work","title":"Habit run","draft":true}),
        cx,
    )
    .try_recv()
    .unwrap()
    .unwrap();
    let run = start["sessionId"].as_i64().unwrap();
    cx.update(|cx| {
        let owner = s.manager.read(cx).owner.upgrade().unwrap();
        let view = owner.read(cx).pilot_session_entity(run).unwrap();
        s.manager.update(cx, |m, _| {
            let pilot = m.roster.get_mut(&s.id).unwrap();
            pilot.launches.retain(|l| l.session_id != run);
            for launch in &mut pilot.launches {
                launch.origin_turn = ((run as u64) << 32) | 1;
            }
            m.running.insert(
                run,
                RunningHabit {
                    pilot: s.id.clone(),
                    habit: "habit".into(),
                    view,
                    started: 0,
                    launched: 0,
                    failure: None,
                    waiting_since: None,
                    cards: Vec::new(),
                    mailbox: Vec::new(),
                },
            );
        })
    });
    run
}
#[gpui::test]
fn habit_children_report_to_the_habit_mailbox_one_session_at_a_time(cx: &mut TestAppContext) {
    let s = setup(cx);
    let (a, b) = delegated_pair(&s, cx);
    let run = habit_run(&s, cx);
    set_child(&s, a, false, cx);
    set_child(&s, b, true, cx);
    // The run picks up another turn while the report is built, so the report waits in its mailbox.
    let report = |s: &Setup, cx: &mut TestAppContext| {
        cx.update(|cx| {
            s.manager.update(cx, |m, cx| {
                m.check_completions(cx);
                m.running[&run].view.update(cx, |v, _| v.busy = true);
            })
        });
        cx.run_until_parked();
        cx.update(|cx| {
            s.manager.update(cx, |m, _| {
                let run = m.running.get_mut(&run).unwrap();
                let texts = mailbox_texts(run);
                run.mailbox.clear();
                texts
            })
        })
    };
    let mailbox = report(&s, cx);
    assert_eq!(mailbox.len(), 1, "{mailbox:?}");
    assert!(mailbox[0].starts_with(ONE_STOPPED), "{}", mailbox[0]);
    assert!(mailbox[0].contains(&format!("Session {a}:")));
    assert!(!mailbox[0].contains(&format!("Session {b}:")));
    assert!(
        delegated_reports(&s, cx).is_empty(),
        "habit reports skip the Pilot chat"
    );

    // Idle with B still unreported: the run stays alive to receive B.
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.running[&run].view.update(cx, |v, _| v.busy = false);
            m.habit_changed(run, cx);
        })
    });
    cx.run_until_parked();
    assert!(cx.update(|cx| s.manager.read(cx).running.contains_key(&run)));

    set_child(&s, b, false, cx);
    let mailbox = report(&s, cx);
    assert_eq!(mailbox.len(), 1, "{mailbox:?}");
    assert!(mailbox[0].contains(&format!("Session {b}:")));
    assert!(!mailbox[0].contains(&format!("Session {a}:")));
}
#[gpui::test]
fn habit_reports_waiting_in_the_mailbox_merge_into_one(cx: &mut TestAppContext) {
    let s = setup(cx);
    let (a, b) = delegated_pair(&s, cx);
    let run = habit_run(&s, cx);
    let busy = |s: &Setup, busy: bool, cx: &mut TestAppContext| {
        cx.update(|cx| {
            s.manager.update(cx, |m, cx| {
                m.running[&run].view.update(cx, |v, _| v.busy = busy)
            })
        })
    };
    set_child(&s, a, false, cx);
    set_child(&s, b, true, cx);
    cx.update(|cx| s.manager.update(cx, |m, cx| m.check_completions(cx)));
    busy(&s, true, cx);
    cx.run_until_parked();
    // The run is between turns but its mailbox has not been handed out yet.
    busy(&s, false, cx);
    set_child(&s, b, false, cx);
    cx.update(|cx| s.manager.update(cx, |m, cx| m.check_completions(cx)));
    busy(&s, true, cx);
    cx.run_until_parked();
    let mailbox = cx.update(|cx| mailbox_texts(&s.manager.read(cx).running[&run]));
    assert_eq!(mailbox.len(), 1, "{mailbox:?}");
    assert_both(&mailbox[0], a, b);
}
/// Both children stopped and reported in one message that is still queued for the Pilot chat.
fn queued_pair_report(s: &Setup, cx: &mut TestAppContext) -> (i64, i64) {
    let (a, b) = delegated_pair(s, cx);
    set_child(s, a, false, cx);
    set_child(s, b, false, cx);
    cx.update(|cx| s.manager.update(cx, |m, cx| m.check_completions(cx)));
    cx.run_until_parked();
    let reports = delegated_reports(s, cx);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert_both(&reports[0], a, b);
    (a, b)
}
fn dismiss(s: &Setup, action: &str, session: i64, cx: &mut TestAppContext) {
    call(
        s,
        &format!("{action}-{session}"),
        action,
        json!({"sessionId":session}),
        cx,
    )
    .try_recv()
    .unwrap()
    .unwrap();
}
#[gpui::test]
fn a_queued_report_drops_a_session_the_pilot_dismisses(cx: &mut TestAppContext) {
    let s = setup(cx);
    let (a, b) = queued_pair_report(&s, cx);
    dismiss(&s, "sessions.stop", a, cx);
    let reports = delegated_reports(&s, cx);
    assert_eq!(reports.len(), 1, "{reports:?}");
    assert!(reports[0].starts_with(ONE_STOPPED), "{}", reports[0]);
    assert!(reports[0].contains(&format!("Session {b}:")));
    assert!(!reports[0].contains(&format!("Session {a}:")));
    // Dismissing the last session left in it drops the report.
    dismiss(&s, "sessions.archive", b, cx);
    assert!(delegated_reports(&s, cx).is_empty());
}
#[gpui::test]
fn a_handed_over_report_is_never_rewritten(cx: &mut TestAppContext) {
    let s = setup(cx);
    let (a, b) = delegated_pair(&s, cx);
    set_child(&s, a, false, cx);
    set_child(&s, b, true, cx);
    cx.update(|cx| s.manager.update(cx, |m, cx| m.check_completions(cx)));
    cx.run_until_parked();
    // The chat has taken A's report: it is in flight, though still in the outbox.
    let handed = cx.update(|cx| {
        s.manager.update(cx, |m, _| {
            let report = m
                .roster
                .get(&s.id)
                .unwrap()
                .outbox
                .iter()
                .find(|q| q.text.starts_with("[Delegated work completed]"))
                .cloned()
                .unwrap();
            m.inflight.insert(s.id.clone(), report.clone());
            report
        })
    });
    let outbox = |s: &Setup, cx: &mut TestAppContext| {
        cx.update(|cx| s.manager.read(cx).roster.get(&s.id).unwrap().outbox.clone())
    };
    let unchanged = |s: &Setup, cx: &mut TestAppContext| {
        outbox(s, cx)
            .iter()
            .any(|q| q.id == handed.id && q.text == handed.text)
    };
    // B arrives on its own instead of joining A's report.
    set_child(&s, b, false, cx);
    cx.update(|cx| s.manager.update(cx, |m, cx| m.check_completions(cx)));
    cx.run_until_parked();
    assert!(unchanged(&s, cx), "{:?}", outbox(&s, cx));
    let reports = delegated_reports(&s, cx);
    assert_eq!(reports.len(), 2, "{reports:?}");
    let own = reports.iter().find(|r| **r != handed.text).unwrap();
    assert!(own.starts_with(ONE_STOPPED), "{own}");
    assert!(own.contains(&format!("Session {b}:")) && !own.contains(&format!("Session {a}:")));
    // Dismissing A afterwards leaves the handed-over report alone too.
    dismiss(&s, "sessions.stop", a, cx);
    assert!(unchanged(&s, cx), "{:?}", outbox(&s, cx));
}
#[gpui::test]
fn a_habit_mailbox_report_drops_a_session_the_pilot_dismisses(cx: &mut TestAppContext) {
    let s = setup(cx);
    let (a, b) = delegated_pair(&s, cx);
    let run = habit_run(&s, cx);
    set_child(&s, a, false, cx);
    set_child(&s, b, false, cx);
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.check_completions(cx);
            m.running[&run].view.update(cx, |v, _| v.busy = true);
        })
    });
    cx.run_until_parked();
    let mailbox = |s: &Setup, cx: &mut TestAppContext| {
        cx.update(|cx| s.manager.read(cx).running.get(&run).map(mailbox_texts))
    };
    assert_both(&mailbox(&s, cx).unwrap()[0], a, b);
    dismiss(&s, "sessions.stop", a, cx);
    let texts = mailbox(&s, cx).unwrap();
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert!(texts[0].starts_with(ONE_STOPPED), "{}", texts[0]);
    assert!(texts[0].contains(&format!("Session {b}:")));
    assert!(!texts[0].contains(&format!("Session {a}:")));
    // Once the run is idle, dismissing the last reported session ends it instead of leaving
    // it waiting for a delivery that will never come.
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.running[&run].view.update(cx, |v, _| v.busy = false)
        })
    });
    dismiss(&s, "sessions.delete", b, cx);
    assert_eq!(mailbox(&s, cx), None, "the run finished");
}
/// Start a draft child and return its id.
fn start_child(s: &Setup, request: &str, cx: &mut TestAppContext) -> i64 {
    call(
        s,
        request,
        "sessions.start",
        json!({"project":s.project,"prompt":"Do the work","title":request,"draft":true}),
        cx,
    )
    .try_recv()
    .unwrap()
    .unwrap()["sessionId"]
        .as_i64()
        .unwrap()
}
fn groups_of(s: &Setup, cx: &mut TestAppContext) -> gespan_core::session_groups::Groups {
    cx.update(|cx| {
        let owner = s.manager.read(cx).owner.upgrade().unwrap();
        owner.read(cx).pilot_groups(&s.project)
    })
}
fn set_groups(s: &Setup, groups: gespan_core::session_groups::Groups, cx: &mut TestAppContext) {
    cx.update(|cx| {
        let owner = s.manager.read(cx).owner.upgrade().unwrap();
        owner.update(cx, |o, cx| {
            o.pilot_apply_groups(s.project.clone(), groups, cx)
        });
    });
}
#[gpui::test]
fn every_started_session_is_visible_and_in_the_pilots_folder(cx: &mut TestAppContext) {
    let s = setup(cx);
    cx.update(|cx| {
        let pilot = s.manager.read(cx).roster.get(&s.id).unwrap();
        assert!(!pilot.show_started_sessions_in_sidebar && !pilot.use_sidebar_folders);
    });
    let child = start_child(&s, "child", cx);
    let (hidden, label) = cx.update(|cx| {
        let m = s.manager.read(cx);
        (
            m.hidden_session(child),
            m.roster.get(&s.id).unwrap().label(),
        )
    });
    assert!(!hidden, "a Pilot-started session must show in the sidebar");
    let groups = groups_of(&s, cx);
    let folder = groups.folder_of(child).expect("filed in a folder");
    assert_eq!(folder.id, format!("pilot-{}", s.id));
    assert_eq!(folder.name, label);
    // A habit run's session stays hidden.
    cx.update(|cx| {
        s.manager.update(cx, |m, _| {
            m.roster.hidden_sessions.push(child);
        });
        assert!(s.manager.read(cx).hidden_session(child));
    });
}
#[gpui::test]
fn renaming_the_pilot_keeps_one_folder(cx: &mut TestAppContext) {
    let s = setup(cx);
    let first = start_child(&s, "first", cx);
    cx.update(|cx| {
        s.manager.update(cx, |m, _| {
            m.roster.get_mut(&s.id).unwrap().name = "Renamed Pilot".into();
        })
    });
    let second = start_child(&s, "second", cx);
    let groups = groups_of(&s, cx);
    let id = format!("pilot-{}", s.id);
    assert_eq!(groups.folder_of(first).unwrap().id, id);
    assert_eq!(groups.folder_of(second).unwrap().id, id);
    assert_eq!(groups.folder(&id).unwrap().name, "Renamed Pilot");
    assert_eq!(groups.folders.len(), 1, "{:?}", groups.folders);
}
#[gpui::test]
fn a_legacy_folder_matched_by_name_migrates_to_the_stable_id(cx: &mut TestAppContext) {
    let s = setup(cx);
    let old = start_child(&s, "old", cx);
    let label = cx.update(|cx| s.manager.read(cx).roster.get(&s.id).unwrap().label());
    // Put the earlier session in a folder the way older versions filed it.
    let mut groups = groups_of(&s, cx);
    groups.remove_sessions(&[old]);
    let legacy = groups.create_folder(&[old], "legacy-id".into()).unwrap();
    groups.rename_folder(&legacy, &label);
    groups.set_color(&legacy, Some(3));
    set_groups(&s, groups, cx);
    let new = start_child(&s, "new", cx);
    let groups = groups_of(&s, cx);
    let id = format!("pilot-{}", s.id);
    assert_eq!(groups.folders.len(), 1, "{:?}", groups.folders);
    assert!(groups.folder("legacy-id").is_none());
    let folder = groups.folder(&id).unwrap();
    assert_eq!(folder.name, label);
    assert!(folder.session_ids.contains(&old) && folder.session_ids.contains(&new));
}
#[gpui::test]
fn a_follow_up_files_a_loose_session_but_leaves_a_foldered_one_alone(cx: &mut TestAppContext) {
    let s = setup(cx);
    let loose = start_child(&s, "loose", cx);
    let kept = start_child(&s, "kept", cx);
    let mut groups = groups_of(&s, cx);
    groups.remove_sessions(&[loose, kept]);
    let user = groups.create_folder(&[kept], "user-folder".into()).unwrap();
    set_groups(&s, groups, cx);
    assert!(groups_of(&s, cx).folder_of(loose).is_none());
    // Drafts leave their prompt in the input; clear it so a follow-up is accepted.
    cx.update(|cx| {
        let owner = s.manager.read(cx).owner.upgrade().unwrap();
        for child in [loose, kept] {
            let view = owner.read(cx).pilot_session_entity(child).unwrap();
            let input = view.read(cx).input.clone();
            input.update(cx, |i, cx| i.set_text(String::new(), cx));
            // Keep the follow-up queued (history "still loading") instead of starting a provider turn.
            view.update(cx, |v, _| v.hold_history_for_test());
        }
    });
    for (request, session) in [("send-loose", loose), ("send-kept", kept)] {
        call(
            &s,
            request,
            "sessions.send",
            json!({"sessionId":session,"prompt":"Next step","notifyOnComplete":true}),
            cx,
        )
        .try_recv()
        .unwrap()
        .unwrap();
    }
    let groups = groups_of(&s, cx);
    assert_eq!(
        groups.folder_of(loose).unwrap().id,
        format!("pilot-{}", s.id)
    );
    assert_eq!(groups.folder_of(kept).unwrap().id, user);
}
#[gpui::test]
fn launches_left_loose_by_older_versions_are_filed_once_at_startup(cx: &mut TestAppContext) {
    let s = setup(cx);
    let loose = start_child(&s, "loose", cx);
    let kept = start_child(&s, "kept", cx);
    let folder = format!("pilot-{}", s.id);
    // Older versions left launches out of folders; the user has since filed one of them.
    let mut groups = groups_of(&s, cx);
    groups.dissolve(&folder);
    let user = groups.create_folder(&[kept], "user-folder".into()).unwrap();
    assert!(groups.folder_of(loose).is_none());
    s.store
        .lock()
        .unwrap()
        .save_groups(&s.project, &groups)
        .unwrap();
    // A launch whose session no longer exists is not filed.
    let gone = 999_999;
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            let pilot = m.roster.get_mut(&s.id).unwrap();
            let mut launch = pilot.launches[0].clone();
            launch.session_id = gone;
            pilot.launches.push(launch);
            m.save(cx);
        })
    });
    cx.run_until_parked();
    let restart = |cx: &mut TestAppContext| {
        let (db, cwd) = (s.store.clone(), s.project.clone());
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Shell::new(db, Vec::new(), cwd, None, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        window
            .update(cx, |shell, _, _| shell.pilot_groups(&s.project))
            .unwrap()
    };
    let filed = restart(cx);
    assert_eq!(filed.folder_of(loose).unwrap().id, folder);
    assert_eq!(filed.folder_of(kept).unwrap().id, user);
    assert!(filed.folder_of(gone).is_none());
    assert_eq!(
        s.store.lock().unwrap().groups(&s.project).unwrap(),
        filed,
        "the filing is saved"
    );
    assert_eq!(restart(cx), filed, "a second pass changes nothing");
}
/// A two-agent Pilot Flow blueprint in `cwd`, as a Pilot would send it.
fn flow_blueprint(id: &str, cwd: &str) -> Value {
    let agent = |id: &str, to: &str| {
        json!({"kind":"agent","id":id,"title":id.to_uppercase(),"harness":"codex","model":"",
            "brief":format!("{id} {{{{inputs.task}}}}"),"outputs":[],"transitions":[{"to":to}],"maxVisits":3})
    };
    json!({"id":id,"name":format!("Blueprint {id}"),"description":"Plan then build","cwd":cwd,
        "inputs":[{"name":"task","label":"Task","required":true}],"initial":"plan",
        "states":[agent("plan","build"),agent("build","$succeeded")],"access":"auto"})
}
fn flows_ok(
    s: &Setup,
    request: &str,
    action: &str,
    input: Value,
    cx: &mut TestAppContext,
) -> Value {
    call(s, request, action, input, cx)
        .try_recv()
        .unwrap()
        .unwrap_or_else(|e| panic!("{action}: {e}"))
}
fn flows_err(
    s: &Setup,
    request: &str,
    action: &str,
    input: Value,
    cx: &mut TestAppContext,
) -> String {
    match call(s, request, action, input, cx).try_recv().unwrap() {
        Ok(value) => panic!("{action} succeeded: {value}"),
        Err(error) => error,
    }
}
/// Every Flow launch is held from here on, so no session or provider turn starts.
fn hold_flow_launches(s: &Setup, cx: &mut TestAppContext) {
    s.window
        .update(cx, |shell, _, _| shell.hold_flow_launches_for_test())
        .unwrap();
}
/// The prompts of the held Flow launches, oldest first.
fn held_flow_prompts(s: &Setup, cx: &mut TestAppContext) -> Vec<String> {
    s.window
        .update(cx, |shell, _, _| {
            shell
                .held_flow_launches_for_test()
                .into_iter()
                .map(|l| l.prompt)
                .collect()
        })
        .unwrap()
}
#[gpui::test]
fn flows_actions_round_trip_for_the_owning_pilot(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_flow_launches(&s, cx);
    // Cancelling the goal's only run rolls the goal up; keep that queued.
    hold_outbox(&s, cx);
    let saved = flows_ok(
        &s,
        "save",
        "flows.save",
        json!({"flow":flow_blueprint("", &s.project)}),
        cx,
    );
    let flow_id = saved["flow"]["id"].as_str().unwrap().to_owned();
    assert!(!flow_id.is_empty(), "a new Flow gets an id");
    assert_eq!(saved["flow"]["owner"], json!(s.id));
    let list = flows_ok(&s, "list", "flows.list", json!({}), cx);
    assert_eq!(
        list,
        json!({"flows":[{"id":flow_id,"name":"Blueprint ","description":"Plan then build",
            "states":["plan","build"],"updatedAt":saved["flow"]["updatedAt"]}]})
    );
    let read = flows_ok(&s, "read", "flows.read", json!({"flowId":flow_id}), cx);
    assert_eq!(read, saved["flow"]);

    let start =
        json!({"flowId":flow_id,"goal":"g1","task":"Fix login","inputs":{"task":"fix login"}});
    let run_id = flows_ok(&s, "start", "flows.start", start.clone(), cx)["runId"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        flows_ok(&s, "start", "flows.start", start, cx)["runId"],
        json!(run_id),
        "a retried start is the same run"
    );
    let held = held_flow_prompts(&s, cx);
    assert_eq!(held.len(), 1);
    assert!(held[0].contains("plan fix login"), "{held:?}");
    assert_eq!(
        flows_ok(&s, "runs-g1", "flows.runs", json!({"goal":"g1"}), cx),
        json!({"runs":[{"runId":run_id,"flowId":flow_id,"goal":"g1","task":"Fix login",
            "status":"running","currentState":"plan","error":null}]})
    );
    assert_eq!(
        flows_ok(&s, "runs-g2", "flows.runs", json!({"goal":"g2"}), cx),
        json!({"runs":[]})
    );
    assert_eq!(
        flows_ok(&s, "runs-all", "flows.runs", json!({}), cx)["runs"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let detail = flows_ok(&s, "run", "flows.run", json!({"runId":run_id}), cx);
    assert_eq!(detail["run"]["id"], json!(run_id));
    assert_eq!(
        detail["run"]["tag"],
        json!({"goal":"g1","task":"Fix login"})
    );
    assert_eq!(
        detail["visits"],
        json!([{"seq":1,"stateId":"plan","status":"running","sessionId":null,"outputs":null,"error":null}])
    );

    let mut edited = detail["run"]["flow"].clone();
    edited["states"][1]["brief"] = json!("build {{inputs.task}} with tests");
    assert_eq!(
        flows_ok(
            &s,
            "edit",
            "flows.editRun",
            json!({"runId":run_id,"flow":edited,"note":"add tests"}),
            cx
        ),
        json!({"edited":true})
    );
    let mut entered = edited.clone();
    entered["states"][0]["brief"] = json!("plan differently");
    assert!(
        flows_err(
            &s,
            "edit-entered",
            "flows.editRun",
            json!({"runId":run_id,"flow":entered,"note":"no"}),
            cx
        )
        .contains("State \"plan\" has already run and cannot be changed")
    );
    let detail = flows_ok(&s, "run-edited", "flows.run", json!({"runId":run_id}), cx);
    assert_eq!(
        detail["run"]["flow"]["states"][1]["brief"],
        json!("build {{inputs.task}} with tests")
    );
    assert_eq!(detail["run"]["edits"][0]["note"], json!("add tests"));
    assert_eq!(
        flows_err(
            &s,
            "delete-active",
            "flows.delete",
            json!({"flowId":flow_id}),
            cx
        ),
        "Flow has an active run"
    );

    assert_eq!(
        flows_ok(&s, "cancel", "flows.cancel", json!({"runId":run_id}), cx),
        json!({"cancelled":true})
    );
    let runs = flows_ok(&s, "runs-cancelled", "flows.runs", json!({}), cx);
    assert_eq!(runs["runs"][0]["status"], json!("cancelled"));
    assert_eq!(
        flows_err(
            &s,
            "resume-nowhere",
            "flows.resume",
            json!({"runId":run_id,"stateId":"nowhere"}),
            cx
        ),
        "This run cannot resume from nowhere"
    );
    assert_eq!(
        flows_ok(
            &s,
            "resume",
            "flows.resume",
            json!({"runId":run_id,"stateId":"build"}),
            cx
        ),
        json!({"resumed":true})
    );
    let held = held_flow_prompts(&s, cx);
    assert_eq!(held.len(), 2);
    assert!(
        held[1].contains("build fix login with tests"),
        "the resumed state runs as edited: {held:?}"
    );
    assert_eq!(
        flows_ok(&s, "runs-resumed", "flows.runs", json!({}), cx)["runs"][0]["currentState"],
        json!("build")
    );
    flows_ok(
        &s,
        "cancel-again",
        "flows.cancel",
        json!({"runId":run_id}),
        cx,
    );
    assert_eq!(
        flows_ok(&s, "delete", "flows.delete", json!({"flowId":flow_id}), cx),
        json!({"deleted":true})
    );
    assert_eq!(
        flows_ok(&s, "list-empty", "flows.list", json!({}), cx),
        json!({"flows":[]})
    );
    assert_eq!(
        flows_err(&s, "run-deleted", "flows.run", json!({"runId":run_id}), cx),
        "Run not found"
    );
}
#[gpui::test]
fn flows_actions_never_reveal_other_pilots_or_user_flows(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_flow_launches(&s, cx);
    let flow = |id: &str, owner: Option<&str>| {
        let mut flow = flow_blueprint(id, &s.project);
        flow["owner"] = json!(owner);
        flow["createdAt"] = json!(0);
        flow["updatedAt"] = json!(0);
        serde_json::from_value::<gespan_core::flow::Flow>(flow).unwrap()
    };
    let (theirs, user) = (flow("theirs", Some("other-pilot")), flow("user", None));
    let their_name = theirs.name.clone();
    let inputs = HashMap::from([("task".to_string(), "x".to_string())]);
    let (their_run, user_run) = s
        .window
        .update(cx, |shell, _, cx| {
            let store = shell.pilot_flows_store().unwrap();
            for flow in [&theirs, &user] {
                store.lock().unwrap().upsert(flow).unwrap();
            }
            let tag = gespan_core::flow::RunTag {
                goal: "g1".into(),
                task: "t1".into(),
            };
            let their_run = shell
                .start_pilot_flow("other-pilot", "theirs", inputs.clone(), tag, cx)
                .unwrap();
            let user_run = shell.start_flow(&user, &inputs, cx).unwrap();
            (their_run.id, user_run.id)
        })
        .unwrap();
    cx.run_until_parked();
    assert_eq!(
        flows_ok(&s, "list", "flows.list", json!({}), cx),
        json!({"flows":[]})
    );
    assert_eq!(
        flows_ok(&s, "runs", "flows.runs", json!({}), cx),
        json!({"runs":[]})
    );
    assert_eq!(
        flows_ok(&s, "runs-goal", "flows.runs", json!({"goal":"g1"}), cx),
        json!({"runs":[]})
    );
    let mut n = 0;
    let mut refused = |action: &str, input: Value, error: &str, cx: &mut TestAppContext| {
        n += 1;
        assert_eq!(
            flows_err(&s, &format!("foreign-{n}"), action, input.clone(), cx),
            error,
            "{action} {input}"
        );
    };
    for flow_id in ["theirs", "user", "nowhere"] {
        refused(
            "flows.read",
            json!({"flowId":flow_id}),
            "Flow not found",
            cx,
        );
        refused(
            "flows.delete",
            json!({"flowId":flow_id}),
            "Flow not found",
            cx,
        );
        refused(
            "flows.start",
            json!({"flowId":flow_id,"goal":"g","task":"t","inputs":{"task":"x"}}),
            "Flow not found",
            cx,
        );
    }
    for flow_id in ["theirs", "user"] {
        refused(
            "flows.save",
            json!({"flow":flow_blueprint(flow_id, &s.project)}),
            "Flow not found",
            cx,
        );
    }
    for run_id in [their_run.as_str(), user_run.as_str(), "nowhere"] {
        refused("flows.run", json!({"runId":run_id}), "Run not found", cx);
        refused("flows.cancel", json!({"runId":run_id}), "Run not found", cx);
        refused(
            "flows.resume",
            json!({"runId":run_id,"stateId":"plan"}),
            "Run not found",
            cx,
        );
        refused(
            "flows.editRun",
            json!({"runId":run_id,"flow":flow_blueprint("theirs", &s.project),"note":"n"}),
            "Run not found",
            cx,
        );
    }
    // Neither foreign Flow was taken over or changed.
    s.window
        .update(cx, |shell, _, _| {
            let store = shell.pilot_flows_store().unwrap();
            let store = store.lock().unwrap();
            let saved: Vec<_> = store
                .list()
                .unwrap()
                .into_iter()
                .map(|f| (f.id, f.owner))
                .collect();
            assert_eq!(saved.len(), 2, "{saved:?}");
            let their_flow = saved.iter().find(|(id, _)| id == "theirs").unwrap();
            assert_eq!(their_flow.1.as_deref(), Some("other-pilot"));
            let their_saved = store.owned_flow("other-pilot", "theirs").unwrap().unwrap();
            assert_eq!(their_saved.name, their_name);
            let user_flow = saved.iter().find(|(id, _)| id == "user").unwrap();
            assert_eq!(user_flow.1, None);
            assert_eq!(
                store.run(&their_run).unwrap().unwrap().status,
                gespan_core::flow::RunStatus::Running,
                "another Pilot's run is untouched"
            );
        })
        .unwrap();
    assert_eq!(
        held_flow_prompts(&s, cx).len(),
        2,
        "only the two runs set up here launched"
    );
}
#[gpui::test]
fn flows_save_rejects_approval_states_and_forces_the_owner(cx: &mut TestAppContext) {
    let s = setup(cx);
    let mut gated = flow_blueprint("", &s.project);
    gated["states"][0]["transitions"] = json!([{"to":"review"}]);
    gated["states"].as_array_mut().unwrap().push(
        json!({"kind":"approval","id":"review","title":"Review","message":"Check it",
            "transitions":[{"to":"build"}],"maxVisits":3}),
    );
    let error = flows_err(&s, "gated", "flows.save", json!({"flow":gated}), cx);
    assert!(error.starts_with("Flow is invalid: "), "{error}");
    assert!(
        error.contains("State \"review\": approval states are not allowed in a Pilot Flow"),
        "{error}"
    );
    let mut claimed = flow_blueprint("mine", &s.project);
    claimed["owner"] = json!("other-pilot");
    let saved = flows_ok(&s, "claimed", "flows.save", json!({"flow":claimed}), cx);
    assert_eq!(saved["flow"]["owner"], json!(s.id));
    let mut unowned = flow_blueprint("mine", &s.project);
    unowned["owner"] = Value::Null;
    unowned["name"] = json!("Renamed");
    let saved = flows_ok(&s, "unowned", "flows.save", json!({"flow":unowned}), cx);
    assert_eq!(saved["flow"]["owner"], json!(s.id));
    assert_eq!(
        saved["flow"]["name"],
        json!("Renamed"),
        "an update of its own Flow"
    );
    s.window
        .update(cx, |shell, _, _| {
            let store = shell.pilot_flows_store().unwrap();
            let all = store.lock().unwrap().list().unwrap();
            assert_eq!(all.len(), 1, "the approval Flow was not saved");
            assert_eq!(all[0].owner.as_deref(), Some(s.id.as_str()));
        })
        .unwrap();
    let error = flows_err(
        &s,
        "elsewhere",
        "flows.save",
        json!({"flow":flow_blueprint("", "/not-assigned")}),
        cx,
    );
    assert_eq!(error, "Flow project is outside the assigned projects");
    let error = flows_err(&s, "shape", "flows.save", json!({"flow":{"name":"x"}}), cx);
    assert!(error.starts_with("flow is not a valid Flow:"), "{error}");
}
#[gpui::test]
fn flows_start_requires_goal_and_task(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_flow_launches(&s, cx);
    let saved = flows_ok(
        &s,
        "save",
        "flows.save",
        json!({"flow":flow_blueprint("", &s.project)}),
        cx,
    );
    let flow_id = saved["flow"]["id"].clone();
    let inputs = json!({"task":"x"});
    let long = "é".repeat(301);
    for (request, input, error) in [
        (
            "no-goal",
            json!({"flowId":flow_id,"task":"t","inputs":inputs}),
            "Provide goal (up to 300 characters)",
        ),
        (
            "no-task",
            json!({"flowId":flow_id,"goal":"g","inputs":inputs}),
            "Provide task (up to 300 characters)",
        ),
        (
            "blank-goal",
            json!({"flowId":flow_id,"goal":"  ","task":"t","inputs":inputs}),
            "Provide goal (up to 300 characters)",
        ),
        (
            "long-task",
            json!({"flowId":flow_id,"goal":"g","task":long,"inputs":inputs}),
            "Provide task (up to 300 characters)",
        ),
    ] {
        assert_eq!(
            flows_err(&s, request, "flows.start", input, cx),
            error,
            "{request}"
        );
    }
    assert!(held_flow_prompts(&s, cx).is_empty(), "nothing started");
    let error = flows_err(
        &s,
        "no-input",
        "flows.start",
        json!({"flowId":flow_id,"goal":"g","task":"t"}),
        cx,
    );
    assert!(
        error.contains("task"),
        "a required Flow input is still required: {error}"
    );
    let at_limit = "é".repeat(300);
    let run = flows_ok(
        &s,
        "at-limit",
        "flows.start",
        json!({"flowId":flow_id,"goal":at_limit,"task":"t","inputs":inputs}),
        cx,
    );
    assert!(run["runId"].as_str().is_some_and(|id| !id.is_empty()));
    assert_eq!(held_flow_prompts(&s, cx).len(), 1);
}
/// Starts a run of `flow` (a Pilot's when `pilot` is set, else the user's) and cancels it as
/// its session is created, so no provider turn starts. Returns the sessions it created.
fn run_and_cancel(
    s: &Setup,
    flow: gespan_core::flow::Flow,
    pilot: bool,
    cx: &mut TestAppContext,
) -> Vec<i64> {
    // The cancelled Pilot run alerts its Pilot; keep that from starting a turn.
    hold_outbox(s, cx);
    let before = s
        .window
        .update(cx, |shell, _, cx| {
            shell
                .pilot_session_views(cx)
                .into_iter()
                .map(|v| v.id)
                .collect::<Vec<_>>()
        })
        .unwrap();
    s.window
        .update(cx, |shell, _, cx| {
            let store = shell.pilot_flows_store().unwrap();
            let flow = store.lock().unwrap().upsert(&flow).unwrap();
            let inputs = HashMap::from([("task".to_string(), "x".to_string())]);
            let run = if pilot {
                let tag = gespan_core::flow::RunTag {
                    goal: "g".into(),
                    task: "Fix the login bug".into(),
                };
                shell.start_pilot_flow(&s.id, &flow.id, inputs, tag, cx)
            } else {
                shell.start_flow(&flow, &inputs, cx)
            }
            .unwrap();
            shell.cancel_flow_run(&run.id, cx).unwrap();
        })
        .unwrap();
    cx.run_until_parked();
    s.window
        .update(cx, |shell, _, cx| {
            shell
                .pilot_session_views(cx)
                .into_iter()
                .map(|v| v.id)
                .filter(|id| !before.contains(id))
                .collect()
        })
        .unwrap()
}
fn blueprint(id: &str, owner: Option<&str>, s: &Setup) -> gespan_core::flow::Flow {
    let mut flow = flow_blueprint(id, &s.project);
    flow["owner"] = json!(owner);
    flow["createdAt"] = json!(0);
    flow["updatedAt"] = json!(0);
    serde_json::from_value(flow).unwrap()
}
#[gpui::test]
fn a_pilot_runs_step_sessions_are_visible_and_in_the_pilots_folder(cx: &mut TestAppContext) {
    let s = setup(cx);
    let launches = |s: &Setup, cx: &mut TestAppContext| {
        cx.update(|cx| s.manager.read(cx).roster.get(&s.id).unwrap().launches.len())
    };
    let before = launches(&s, cx);
    let created = run_and_cancel(&s, blueprint("mine", Some(&s.id), &s), true, cx);
    assert_eq!(created.len(), 1, "the visit's session");
    let session = created[0];
    let title = s
        .window
        .update(cx, |shell, _, cx| {
            let listed = shell.pilot_session_views(cx);
            listed
                .iter()
                .find(|v| v.id == session)
                .expect("listed")
                .title
                .clone()
        })
        .unwrap();
    let hidden = cx.update(|cx| s.manager.read(cx).hidden_session(session));
    assert!(!hidden, "the step session shows in the sidebar");
    assert_eq!(title, "Fix the login bug · PLAN");
    let groups = groups_of(&s, cx);
    assert_eq!(
        groups.folder_of(session).expect("filed").id,
        format!("pilot-{}", s.id)
    );
    assert_eq!(launches(&s, cx), before, "reported by Flow events instead");
}
#[gpui::test]
fn user_flow_step_sessions_are_not_filed(cx: &mut TestAppContext) {
    let s = setup(cx);
    let created = run_and_cancel(&s, blueprint("user", None, &s), false, cx);
    assert_eq!(created.len(), 1, "the visit's session");
    assert!(groups_of(&s, cx).folder_of(created[0]).is_none());
}
// Flow step events: side reviews, alerts and goal roll-ups.
use crate::shell::pilot_flows::PilotFlowEvent;
use gespan_core::flow::{FlowRun, FlowVisit, RunStatus, RunTag, VisitStatus};
use gespan_core::pilot::flow_notes::{MAX_REVIEWS, NoteKind};
const REVIEW_CONTRACT: &str = "Review only this step. You cannot change anything. Reply with one first line: NO_CHANGE, SUGGEST_STEP_CHANGE, NEEDS_ATTENTION or BLUEPRINT_IDEA, then at most a short paragraph explaining it.";
/// A failed message at the head of the outbox holds it, so Flow updates stay
/// visible there instead of starting a provider turn.
fn hold_outbox(s: &Setup, cx: &mut TestAppContext) {
    cx.update(|cx| {
        s.manager.update(cx, |m, _| {
            m.roster.get_mut(&s.id).unwrap().outbox.push(Message {
                sent_at: None,
                id: pilot::new_id(),
                text: "Held".into(),
                internal: false,
                error: Some("Retry required".into()),
                attachments: Vec::new(),
                report: Vec::new(),
            });
        })
    });
}
fn flow_run(s: &Setup, id: &str, goal: &str, task: &str) -> FlowRun {
    FlowRun {
        id: id.into(),
        flow_id: "blueprint".into(),
        flow: blueprint("blueprint", Some(&s.id), s),
        status: RunStatus::Running,
        inputs: HashMap::new(),
        worktree_cwd: None,
        current_state: Some("build".into()),
        error: None,
        created_at: 0,
        finished_at: None,
        tag: Some(RunTag {
            goal: goal.into(),
            task: task.into(),
        }),
        edits: Vec::new(),
    }
}
fn visit(run: &str, state: &str, status: VisitStatus, error: Option<&str>) -> FlowVisit {
    FlowVisit {
        id: format!("{run}-{state}"),
        run_id: run.into(),
        seq: 1,
        state_id: state.into(),
        status,
        prompt: None,
        session_id: None,
        outputs: None,
        error: error.map(str::to_owned),
        retried: false,
        note: None,
        started_at: 0,
        finished_at: None,
    }
}
fn step(run: &FlowRun, visit: FlowVisit) -> PilotFlowEvent {
    PilotFlowEvent::Step {
        run: run.clone(),
        visit,
    }
}
fn finished(run_id: &str, status: RunStatus) -> PilotFlowEvent {
    PilotFlowEvent::Finished {
        run_id: run_id.into(),
        status,
    }
}
/// Hands `events` to the Pilot in one batch, as the Shell does.
fn flow_events(s: &Setup, events: Vec<PilotFlowEvent>, cx: &mut TestAppContext) {
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            for event in events {
                m.flow_event(&s.id, event, cx);
            }
        })
    });
    cx.run_until_parked();
}
fn review_sessions(s: &Setup, cx: &mut TestAppContext) -> Vec<i64> {
    cx.update(|cx| {
        let mut ids: Vec<i64> = s.manager.read(cx).reviews.keys().copied().collect();
        ids.sort();
        ids
    })
}
/// Ends a review's turn with `reply`, as its provider would.
fn answer_review(s: &Setup, session: i64, reply: &str, cx: &mut TestAppContext) {
    cx.update(|cx| {
        let view = s.manager.read(cx).reviews[&session].view.clone();
        view.update(cx, |v, cx| {
            let rows = vec![
                gespan_core::transcript::Row {
                    role: "user".into(),
                    text: "Review this step".into(),
                    first: true,
                    sent_at: None,
                },
                gespan_core::transcript::Row {
                    role: "assistant".into(),
                    text: reply.into(),
                    first: true,
                    sent_at: None,
                },
            ];
            v.transcript.update(cx, |t, cx| t.patch(0, rows, cx));
            v.busy = false;
            cx.emit(SessionChanged);
        });
    });
    cx.run_until_parked();
}
fn set_main_busy(s: &Setup, busy: bool, cx: &mut TestAppContext) {
    cx.update(|cx| {
        let chat = s.manager.read(cx).chats[&s.id].clone();
        chat.update(cx, |v, cx| {
            v.busy = busy;
            cx.emit(SessionChanged);
        });
    });
    cx.run_until_parked();
}
/// The internal Flow updates queued for the main Pilot chat.
fn flow_updates(s: &Setup, cx: &mut TestAppContext) -> Vec<String> {
    cx.update(|cx| {
        s.manager
            .read(cx)
            .roster
            .get(&s.id)
            .unwrap()
            .outbox
            .iter()
            .filter(|m| m.internal && m.text.starts_with("[Flow update]"))
            .map(|m| m.text.clone())
            .collect()
    })
}
fn pilot_state(s: &Setup, cx: &mut TestAppContext) -> pilot::Pilot {
    cx.update(|cx| s.manager.read(cx).roster.get(&s.id).unwrap().clone())
}
fn call_from(
    s: &Setup,
    session: i64,
    request: &str,
    action: &str,
    input: Value,
    cx: &mut TestAppContext,
) -> Result<Value, String> {
    let (reply, rx) = mpsc::channel();
    let call = pilot::bridge::Call {
        session,
        request_id: request.into(),
        action: action.into(),
        input,
        reply,
    };
    let handle: gpui::AnyWindowHandle = s.window.into();
    handle
        .update(cx, |_, w, cx| {
            s.manager.update(cx, |m, cx| m.app_call(call, w, cx))
        })
        .unwrap();
    cx.run_until_parked();
    rx.try_recv().unwrap()
}
/// Saves the blueprint and starts one held run per task under `goal`.
fn start_runs(s: &Setup, goal: &str, tasks: &[&str], cx: &mut TestAppContext) -> Vec<String> {
    hold_flow_launches(s, cx);
    let saved = flows_ok(
        s,
        &format!("save-{goal}"),
        "flows.save",
        json!({"flow":flow_blueprint("", &s.project)}),
        cx,
    );
    let flow_id = saved["flow"]["id"].clone();
    tasks
        .iter()
        .map(|task| {
            flows_ok(
                s,
                &format!("start-{goal}-{task}"),
                "flows.start",
                json!({"flowId":flow_id,"goal":goal,"task":task,"inputs":{"task":task}}),
                cx,
            )["runId"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}
#[gpui::test]
fn a_completed_step_starts_a_hidden_review_and_never_touches_the_main_chat_when_nothing_changes(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    // The step's own session, whose reply the review reads.
    let step_session = {
        let mut db = locked(&s.store).unwrap();
        let summary = db.create("codex", &s.project).unwrap();
        db.write_rows(
            summary.id,
            0,
            &[
                gespan_core::transcript::Row {
                    role: "user".into(),
                    text: "plan the API".into(),
                    first: true,
                    sent_at: None,
                },
                gespan_core::transcript::Row {
                    role: "assistant".into(),
                    text: "Planned three endpoints.".into(),
                    first: true,
                    sent_at: None,
                },
            ],
        )
        .unwrap();
        summary.id
    };
    let main_rows = cx.update(|cx| {
        let m = s.manager.read(cx);
        m.chats[&s.id].read(cx).pilot_rows(cx).len()
    });
    let run = flow_run(&s, "r1", "Ship login", "Add the API");
    let mut done = visit("r1", "plan", VisitStatus::Completed, None);
    done.session_id = Some(step_session.to_string());
    flow_events(&s, vec![step(&run, done)], cx);

    let reviews = review_sessions(&s, cx);
    assert_eq!(reviews.len(), 1, "one review per completed step");
    let review = reviews[0];
    let (title, hidden, prompt, label) = cx.update(|cx| {
        let m = s.manager.read(cx);
        (
            m.reviews[&review].view.read(cx).summary.title.clone(),
            m.hidden_session(review) && m.roster.hidden_sessions.contains(&review),
            m.hidden_prompts
                .iter()
                .find(|(id, _)| *id == review)
                .map(|(_, p)| p.clone())
                .unwrap(),
            m.roster.get(&s.id).unwrap().label(),
        )
    });
    assert_eq!(title, format!("{label} · review · Add the API"));
    assert!(hidden, "the review session stays out of the sidebar");
    assert!(prompt.contains("Task: Add the API"), "{prompt}");
    assert!(
        prompt.contains("Reply:\nPlanned three endpoints."),
        "{prompt}"
    );
    assert!(prompt.ends_with(REVIEW_CONTRACT), "{prompt}");
    assert_ne!(review, s.session, "never the main chat");

    answer_review(&s, review, "NO_CHANGE\nThe plan is sound.", cx);
    assert!(
        review_sessions(&s, cx).is_empty(),
        "the finished review is removed"
    );
    let pilot = pilot_state(&s, cx);
    assert_eq!(pilot.flow_notes.len(), 1);
    assert_eq!(pilot.flow_notes[0].kind, NoteKind::NoChange);
    assert_eq!(pilot.flow_notes[0].run_id, "r1");
    assert_eq!(pilot.flow_notes[0].state_id, "plan");
    assert!(pilot.flow_alerts.is_empty());
    assert!(pilot.outbox.is_empty(), "nothing is sent to the main chat");
    let rows = cx.update(|cx| {
        let m = s.manager.read(cx);
        m.chats[&s.id].read(cx).pilot_rows(cx).len()
    });
    assert_eq!(rows, main_rows, "the main chat is untouched");
}
#[gpui::test]
fn review_suggestions_and_failures_while_busy_arrive_as_one_message_when_idle(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    set_main_busy(&s, true, cx);
    let mut runs: Vec<FlowRun> = (1..=4)
        .map(|i| flow_run(&s, &format!("r{i}"), "Ship login", &format!("Task {i}")))
        .collect();
    // Each failed or stopped step ended its run.
    runs[0].status = RunStatus::Failed;
    runs[1].status = RunStatus::Failed;
    runs[2].status = RunStatus::Cancelled;
    flow_events(
        &s,
        vec![step(
            &runs[0],
            visit(
                "r1",
                "plan",
                VisitStatus::Failed,
                Some("No transition matched"),
            ),
        )],
        cx,
    );
    flow_events(
        &s,
        vec![
            step(
                &runs[1],
                visit("r2", "build", VisitStatus::Failed, Some("Tests failed")),
            ),
            step(
                &runs[2],
                visit("r3", "plan", VisitStatus::Cancelled, Some("Stopped")),
            ),
            step(&runs[3], visit("r4", "plan", VisitStatus::Completed, None)),
        ],
        cx,
    );
    let review = review_sessions(&s, cx)[0];
    answer_review(
        &s,
        review,
        "SUGGEST_STEP_CHANGE\nPut tests in the build brief.",
        cx,
    );
    assert!(
        flow_updates(&s, cx).is_empty(),
        "nothing reaches a busy chat"
    );
    assert_eq!(pilot_state(&s, cx).flow_alerts.len(), 4);
    let saved = Roster::load(&locked(&s.store).unwrap()).unwrap();
    assert_eq!(
        saved.get(&s.id).unwrap().flow_alerts.len(),
        4,
        "the waiting alerts survive a restart"
    );

    set_main_busy(&s, false, cx);
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    let update = &updates[0];
    assert!(
        update.starts_with("[Flow update]\n4 updates from your Flows."),
        "{update}"
    );
    for part in [
        "No transition matched",
        "Tests failed",
        "task \"Task 3\" was stopped",
        "Put tests in the build brief.",
    ] {
        assert!(update.contains(part), "missing {part:?} in {update}");
    }
    assert_eq!(update.matches("\n\n---\n\n").count(), 3);
    assert!(pilot_state(&s, cx).flow_alerts.is_empty());
    let saved = Roster::load(&locked(&s.store).unwrap()).unwrap();
    assert!(saved.get(&s.id).unwrap().flow_alerts.is_empty());
}
#[gpui::test]
fn no_more_than_three_reviews_run_at_once_and_the_queue_drains(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let events = (1..=5)
        .map(|i| {
            let id = format!("r{i}");
            step(
                &flow_run(&s, &id, "Ship login", &format!("Task {i}")),
                visit(&id, "plan", VisitStatus::Completed, None),
            )
        })
        .collect();
    flow_events(&s, events, cx);
    let queued =
        |s: &Setup, cx: &mut TestAppContext| cx.update(|cx| s.manager.read(cx).review_queue.len());
    assert_eq!(review_sessions(&s, cx).len(), MAX_REVIEWS);
    assert_eq!(queued(&s, cx), 2);
    let first = review_sessions(&s, cx)[0];
    answer_review(&s, first, "NO_CHANGE", cx);
    assert_eq!(
        review_sessions(&s, cx).len(),
        MAX_REVIEWS,
        "a freed slot is refilled"
    );
    assert_eq!(queued(&s, cx), 1);
    let mut answered = 1;
    while let Some(&next) = review_sessions(&s, cx).first() {
        answer_review(&s, next, "NO_CHANGE", cx);
        answered += 1;
        assert!(review_sessions(&s, cx).len() <= MAX_REVIEWS);
    }
    assert_eq!(answered, 5);
    assert_eq!(queued(&s, cx), 0);
    let pilot = pilot_state(&s, cx);
    let mut reviewed: Vec<_> = pilot.flow_notes.iter().map(|n| n.run_id.clone()).collect();
    reviewed.sort();
    assert_eq!(reviewed, ["r1", "r2", "r3", "r4", "r5"]);
    assert_eq!(cx.update(|cx| s.manager.read(cx).hidden_prompts.len()), 5);
    assert!(
        flow_updates(&s, cx).is_empty(),
        "no change means no message"
    );
}
#[gpui::test]
fn a_review_that_runs_too_long_is_a_review_problem_not_an_alert(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let run = flow_run(&s, "r1", "Ship login", "Add the API");
    flow_events(
        &s,
        vec![step(
            &run,
            visit("r1", "plan", VisitStatus::Completed, None),
        )],
        cx,
    );
    let review = review_sessions(&s, cx)[0];
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.reviews.get_mut(&review).unwrap().started = 0;
            m.check_review_timeouts(cx);
        })
    });
    cx.run_until_parked();
    assert!(review_sessions(&s, cx).is_empty());
    let pilot = pilot_state(&s, cx);
    assert_eq!(pilot.flow_notes[0].kind, NoteKind::ReviewProblem);
    assert_eq!(pilot.flow_notes[0].text, "Review timed out");
    assert!(
        pilot.flow_alerts.is_empty(),
        "a review's own failure is not the task's problem"
    );
    assert!(flow_updates(&s, cx).is_empty());
}
/// The main chat takes the queued Flow updates and starts the turn that reads them.
fn take_flow_updates(s: &Setup, cx: &mut TestAppContext) {
    cx.update(|cx| {
        s.manager.update(cx, |m, _| {
            m.roster
                .get_mut(&s.id)
                .unwrap()
                .outbox
                .retain(|m| !(m.internal && m.text.starts_with("[Flow update]")))
        })
    });
    set_main_busy(s, true, cx);
}
#[gpui::test]
fn a_goal_rolls_up_once_when_its_last_run_ends(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let runs = start_runs(
        &s,
        "Ship login",
        &["Add the API", "Fix the form", "Write docs"],
        cx,
    );
    let goals = pilot_state(&s, cx).goals;
    assert_eq!(goals.len(), 1);
    assert_eq!(goals[0].name, "Ship login");
    assert_eq!(
        goals[0].runs, runs,
        "flows.start records each run under its goal"
    );
    assert!(!goals[0].reported);

    flow_events(
        &s,
        vec![
            finished(&runs[0], RunStatus::Succeeded),
            finished(&runs[1], RunStatus::Succeeded),
        ],
        cx,
    );
    assert!(flow_updates(&s, cx).is_empty(), "one run is still going");
    assert!(!pilot_state(&s, cx).goals[0].reported);

    flow_events(&s, vec![finished(&runs[2], RunStatus::Succeeded)], cx);
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(
        updates[0].starts_with("[Flow update]\n\n[Goal finished] Ship login"),
        "{}",
        updates[0]
    );
    for task in ["Add the API", "Fix the form", "Write docs"] {
        assert!(
            updates[0].contains(&format!("- {task}: succeeded")),
            "{task} in {}",
            updates[0]
        );
    }
    assert!(pilot_state(&s, cx).goals[0].reported);

    flow_events(&s, vec![finished(&runs[2], RunStatus::Succeeded)], cx);
    assert_eq!(
        flow_updates(&s, cx).len(),
        1,
        "a duplicate Finished is ignored"
    );
    assert!(pilot_state(&s, cx).flow_alerts.is_empty());
}
#[gpui::test]
fn interrupted_and_user_stopped_steps_alert_the_pilot(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let runs = start_runs(&s, "Ship login", &["Add the API", "Fix the form"], cx);
    // Startup recovery fails the interrupted visit; its Finished comes first.
    let mut interrupted = flow_run(&s, &runs[0], "Ship login", "Add the API");
    interrupted.status = RunStatus::Failed;
    interrupted.error = Some("Interrupted by app restart".into());
    let mut recovered = visit(
        &runs[0],
        "build",
        VisitStatus::Failed,
        Some("Interrupted by app restart"),
    );
    recovered.session_id = Some("41".into());
    // The user stopped the other task's step session, which ended its run.
    let mut stopped = flow_run(&s, &runs[1], "Ship login", "Fix the form");
    stopped.status = RunStatus::Cancelled;
    flow_events(
        &s,
        vec![
            finished(&runs[0], RunStatus::Failed),
            step(&interrupted, recovered),
            step(
                &stopped,
                visit(
                    &runs[1],
                    "plan",
                    VisitStatus::Cancelled,
                    Some("Stopped by the user"),
                ),
            ),
            finished(&runs[1], RunStatus::Cancelled),
        ],
        cx,
    );
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    let update = &updates[0];
    assert!(
        update.starts_with("[Flow update]\n2 updates from your Flows."),
        "{update}"
    );
    assert_eq!(
        update.matches("Use flows.run").count(),
        2,
        "one alert per task, none doubled by its Finished: {update}"
    );
    for part in [
        "task \"Add the API\" failed",
        "Error: Interrupted by app restart",
        "Session: 41",
        &format!("Use flows.run {} to inspect", runs[0]),
        "task \"Fix the form\" was stopped",
        "Error: Stopped by the user",
    ] {
        assert!(update.contains(part), "missing {part:?} in {update}");
    }
    assert!(
        !update.contains("[Goal finished]"),
        "the Pilot gets to recover first: {update}"
    );
    assert!(!pilot_state(&s, cx).goals[0].reported);

    // The Pilot reads the failures and ends its turn without recovering.
    take_flow_updates(&s, cx);
    assert!(flow_updates(&s, cx).is_empty(), "nothing during that turn");
    set_main_busy(&s, false, cx);
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    for part in [
        "[Flow update]\n\n[Goal finished] Ship login",
        "- Add the API: failed",
        "- Fix the form: cancelled",
    ] {
        assert!(
            updates[0].contains(part),
            "missing {part:?} in {}",
            updates[0]
        );
    }
    assert!(pilot_state(&s, cx).goals[0].reported);
}
#[gpui::test]
fn a_run_that_fails_before_any_step_still_alerts_but_a_pilot_cancel_does_not(
    cx: &mut TestAppContext,
) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let runs = start_runs(&s, "Ship login", &["Add the API"], cx);
    // A visit that fails before its session launches has no step event.
    flow_events(&s, vec![finished(&runs[0], RunStatus::Failed)], cx);
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(
        updates[0].starts_with("[Flow update]\n\nA step of task"),
        "the failure comes alone: {}",
        updates[0]
    );
    assert!(
        updates[0].contains("task \"Add the API\" failed"),
        "{}",
        updates[0]
    );
    assert!(
        updates[0].contains(&format!("Use flows.run {} to inspect", runs[0])),
        "{}",
        updates[0]
    );
    take_flow_updates(&s, cx);
    set_main_busy(&s, false, cx);
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(
        updates[0].starts_with("[Flow update]\n\n[Goal finished] Ship login"),
        "{}",
        updates[0]
    );
    take_flow_updates(&s, cx);
    set_main_busy(&s, false, cx);

    let other = start_runs(&s, "Tidy up", &["Remove dead code"], cx);
    flows_ok(&s, "cancel", "flows.cancel", json!({"runId":other[0]}), cx);
    cx.run_until_parked();
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(
        updates[0].starts_with("[Flow update]\n\n[Goal finished] Tidy up"),
        "the Pilot's own cancel only ends the goal, at once: {}",
        updates[0]
    );
    assert!(!updates[0].contains("Use flows.run"), "{}", updates[0]);
}
#[gpui::test]
fn reviews_cannot_mutate(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let run = flow_run(&s, "r1", "Ship login", "Add the API");
    flow_events(
        &s,
        vec![step(
            &run,
            visit("r1", "plan", VisitStatus::Completed, None),
        )],
        cx,
    );
    let review = review_sessions(&s, cx)[0];
    for (request, action, input) in [
        (
            "edit",
            "flows.editRun",
            json!({"runId":"r1","flow":flow_blueprint("x", &s.project),"note":"n"}),
        ),
        (
            "remember",
            "memory.add",
            json!({"fact":"Reviews write memory"}),
        ),
        ("forget", "memory.remove", json!({"find":"anything"})),
        (
            "start",
            "sessions.start",
            json!({"prompt":"go","project":s.project}),
        ),
        (
            "resume",
            "flows.resume",
            json!({"runId":"r1","stateId":"plan"}),
        ),
    ] {
        assert_eq!(
            call_from(&s, review, request, action, input, cx),
            Err("Reviews are read-only; report your suggestion instead".into()),
            "{action}"
        );
    }
    assert!(
        call_from(&s, review, "runs", "flows.runs", json!({}), cx).is_ok(),
        "reading runs is allowed"
    );
    assert!(call_from(&s, review, "memory", "memory.read", json!({}), cx).is_ok());
    let memory = gespan_core::pilot::memory::read(&s.root, &s.id, None).unwrap();
    assert!(
        !serde_json::to_string(&memory)
            .unwrap()
            .contains("Reviews write memory"),
        "memory has one writer"
    );
}
#[gpui::test]
fn a_failed_step_that_routes_onward_raises_no_failure_alert(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    // The failure transition sent the run on to another state.
    let run = flow_run(&s, "r1", "Ship login", "Add the API");
    flow_events(
        &s,
        vec![step(
            &run,
            visit("r1", "build", VisitStatus::Failed, Some("Tests failed")),
        )],
        cx,
    );
    assert!(pilot_state(&s, cx).flow_alerts.is_empty());
    assert!(flow_updates(&s, cx).is_empty(), "the run goes on");
}
#[gpui::test]
fn a_run_that_ends_after_a_routed_failure_alerts_once(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let runs = start_runs(&s, "Ship login", &["Add the API"], cx);
    let routed = flow_run(&s, &runs[0], "Ship login", "Add the API");
    flow_events(
        &s,
        vec![step(
            &routed,
            visit(&runs[0], "plan", VisitStatus::Failed, Some("Tests failed")),
        )],
        cx,
    );
    // Later the run ends failed with no failing visit (its visits ran out).
    flow_events(&s, vec![finished(&runs[0], RunStatus::Failed)], cx);
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert_eq!(
        updates[0].matches("Use flows.run").count(),
        1,
        "{}",
        updates[0]
    );
    assert!(
        updates[0].contains("task \"Add the API\" failed"),
        "{}",
        updates[0]
    );
    assert!(
        !updates[0].contains("Tests failed"),
        "the alert is the run's end, not the routed step: {}",
        updates[0]
    );
}
#[gpui::test]
fn reviews_never_inherit_the_main_chats_usage_limit(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    // Off screen: the limited Pilot chat's usage notice trips a GPUI debug
    // assertion when it renders (a separate issue).
    cx.update(|cx| s.manager.update(cx, |m, cx| m.close(cx)));
    s.window
        .update(cx, |shell, _, cx| {
            shell.workspace.right_panel_open = false;
            cx.notify();
        })
        .unwrap();
    cx.run_until_parked();
    cx.update(|cx| {
        let chat = s.manager.read(cx).chats[&s.id].clone();
        chat.update(cx, |v, _| v.settings.usage_limit = Some(Default::default()));
    });
    let run = flow_run(&s, "r1", "Ship login", "Add the API");
    flow_events(
        &s,
        vec![step(
            &run,
            visit("r1", "plan", VisitStatus::Completed, None),
        )],
        cx,
    );
    let reviews = review_sessions(&s, cx);
    assert_eq!(reviews.len(), 1);
    let limited = cx.update(|cx| {
        s.manager.read(cx).reviews[&reviews[0]]
            .view
            .read(cx)
            .settings
            .usage_limit
            .is_some()
    });
    assert!(!limited, "the review can start its own turn");
}
#[gpui::test]
fn a_review_that_cannot_start_frees_its_slot_and_never_holds_the_goal(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let tasks = ["Task 1", "Task 2", "Task 3", "Task 4"];
    let runs = start_runs(&s, "Ship login", &tasks, cx);
    cx.update(|cx| s.manager.update(cx, |m, _| m.hidden_dispatch_fails = true));
    let events = runs
        .iter()
        .zip(tasks)
        .map(|(id, task)| {
            step(
                &flow_run(&s, id, "Ship login", task),
                visit(id, "plan", VisitStatus::Completed, None),
            )
        })
        .collect();
    flow_events(&s, events, cx);
    assert!(review_sessions(&s, cx).is_empty(), "no review holds a slot");
    assert_eq!(
        cx.update(|cx| s.manager.read(cx).review_queue.len()),
        0,
        "the queue drains"
    );
    let pilot = pilot_state(&s, cx);
    assert_eq!(pilot.flow_notes.len(), 4);
    for note in &pilot.flow_notes {
        assert_eq!(note.kind, NoteKind::ReviewProblem);
        assert_eq!(note.text, "Review could not start");
    }
    assert!(pilot.flow_alerts.is_empty(), "not a task alert");
    assert!(flow_updates(&s, cx).is_empty());

    flow_events(
        &s,
        runs.iter()
            .map(|id| finished(id, RunStatus::Succeeded))
            .collect(),
        cx,
    );
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(
        updates[0].starts_with("[Flow update]\n\n[Goal finished] Ship login"),
        "{}",
        updates[0]
    );
    assert!(
        updates[0].contains("Task 1 · plan: Review problem: Review could not start"),
        "{}",
        updates[0]
    );
}
#[gpui::test]
fn a_review_whose_turn_ends_without_a_reply_finishes_at_once(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let run = flow_run(&s, "r1", "Ship login", "Add the API");
    flow_events(
        &s,
        vec![step(
            &run,
            visit("r1", "plan", VisitStatus::Completed, None),
        )],
        cx,
    );
    let review = review_sessions(&s, cx)[0];
    // The provider failed before it wrote anything.
    cx.update(|cx| {
        let view = s.manager.read(cx).reviews[&review].view.clone();
        view.update(cx, |v, cx| {
            v.busy = false;
            cx.emit(SessionChanged);
        });
    });
    cx.run_until_parked();
    assert!(review_sessions(&s, cx).is_empty(), "the slot is free");
    let pilot = pilot_state(&s, cx);
    assert_eq!(pilot.flow_notes.len(), 1);
    assert_eq!(pilot.flow_notes[0].kind, NoteKind::ReviewProblem);
    assert_eq!(pilot.flow_notes[0].text, "The review returned no reply");
    assert!(pilot.flow_alerts.is_empty());
}
#[gpui::test]
fn a_finished_review_drops_its_subscription(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let before = cx.update(|cx| s.manager.read(cx).observations.len());
    for id in ["r1", "r2"] {
        let run = flow_run(&s, id, "Ship login", "Add the API");
        flow_events(
            &s,
            vec![step(&run, visit(id, "plan", VisitStatus::Completed, None))],
            cx,
        );
    }
    let reviews = review_sessions(&s, cx);
    answer_review(&s, reviews[0], "NO_CHANGE", cx);
    cx.update(|cx| {
        s.manager.update(cx, |m, cx| {
            m.reviews.get_mut(&reviews[1]).unwrap().started = 0;
            m.check_review_timeouts(cx);
        })
    });
    cx.run_until_parked();
    assert!(review_sessions(&s, cx).is_empty());
    assert_eq!(
        cx.update(|cx| s.manager.read(cx).observations.len()),
        before,
        "answered and timed-out reviews keep no subscription"
    );
}
#[gpui::test]
fn flows_resume_refuses_a_run_whose_project_was_unassigned(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let runs = start_runs(&s, "Ship login", &["Add the API"], cx);
    flows_ok(&s, "cancel", "flows.cancel", json!({"runId":runs[0]}), cx);
    cx.update(|cx| {
        s.manager.update(cx, |m, _| {
            m.roster.get_mut(&s.id).unwrap().projects = vec!["/elsewhere".into()];
        })
    });
    assert_eq!(
        flows_err(
            &s,
            "resume",
            "flows.resume",
            json!({"runId":runs[0],"stateId":"build"}),
            cx
        ),
        "Flow project is outside the assigned projects"
    );
    assert_eq!(held_flow_prompts(&s, cx).len(), 1, "nothing resumed");
    assert_eq!(
        flows_ok(&s, "runs", "flows.runs", json!({}), cx)["runs"][0]["status"],
        json!("cancelled")
    );
}
/// Starts a goal of two runs: the first succeeds, the user stops the second.
fn goal_with_a_stopped_run(s: &Setup, cx: &mut TestAppContext) -> Vec<String> {
    let runs = start_runs(s, "Ship login", &["Add the API", "Fix the form"], cx);
    flow_events(s, vec![finished(&runs[0], RunStatus::Succeeded)], cx);
    s.window
        .update(cx, |shell, _, cx| {
            shell.cancel_flow_run(&runs[1], cx).unwrap()
        })
        .unwrap();
    cx.run_until_parked();
    let updates = flow_updates(s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(
        updates[0].contains("task \"Fix the form\""),
        "{}",
        updates[0]
    );
    assert!(
        !updates[0].contains("[Goal finished]"),
        "the roll-up never rides with the failure it asks to recover: {}",
        updates[0]
    );
    runs
}
#[gpui::test]
fn a_goal_rolls_up_once_after_the_turn_that_read_its_failure_ends(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    goal_with_a_stopped_run(&s, cx);
    // Still undelivered: the chat going idle changes nothing.
    set_main_busy(&s, true, cx);
    set_main_busy(&s, false, cx);
    assert_eq!(flow_updates(&s, cx).len(), 1);
    assert!(!pilot_state(&s, cx).goals[0].reported);

    take_flow_updates(&s, cx);
    assert!(flow_updates(&s, cx).is_empty(), "not while that turn runs");
    set_main_busy(&s, false, cx);
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(
        updates[0].starts_with("[Flow update]\n\n[Goal finished] Ship login"),
        "{}",
        updates[0]
    );
    assert!(
        updates[0].contains("- Fix the form: cancelled"),
        "{}",
        updates[0]
    );
    assert!(pilot_state(&s, cx).goals[0].reported);

    take_flow_updates(&s, cx);
    set_main_busy(&s, false, cx);
    assert!(flow_updates(&s, cx).is_empty(), "the roll-up comes once");
}
#[gpui::test]
fn a_goal_whose_failed_run_the_pilot_resumes_waits_for_that_run(cx: &mut TestAppContext) {
    let s = setup(cx);
    hold_outbox(&s, cx);
    let runs = goal_with_a_stopped_run(&s, cx);
    take_flow_updates(&s, cx);
    // The turn that read the failure resumes the run.
    flows_ok(
        &s,
        "resume",
        "flows.resume",
        json!({"runId":runs[1],"stateId":"build"}),
        cx,
    );
    set_main_busy(&s, false, cx);
    assert!(flow_updates(&s, cx).is_empty(), "the goal stays open");
    assert!(!pilot_state(&s, cx).goals[0].reported);

    flow_events(&s, vec![finished(&runs[1], RunStatus::Succeeded)], cx);
    let updates = flow_updates(&s, cx);
    assert_eq!(updates.len(), 1, "{updates:?}");
    assert!(
        updates[0].starts_with("[Flow update]\n\n[Goal finished] Ship login"),
        "{}",
        updates[0]
    );
    assert!(
        updates[0].contains("- Fix the form: succeeded"),
        "{}",
        updates[0]
    );
}
