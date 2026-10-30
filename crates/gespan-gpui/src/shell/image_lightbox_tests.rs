use super::*;
use base64::Engine;
use gpui::{TestAppContext, VisualTestContext, point};

#[gpui::test]
fn image_lightbox_opens_from_composer_and_sent_images_and_restores_focus(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::input::bind(cx);
        crate::image_lightbox::bind(cx);
        cx.set_reduce_motion(true);
    });
    let (window, _, active) = super::tests::window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let path = std::env::temp_dir().join(format!(
        "gespan-lightbox-{}.png",
        gespan_core::pilot::new_id()
    ));
    std::fs::write(&path, base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=").unwrap()).unwrap();
    let input = window
        .update(visual, |shell, window, cx| {
            let input = shell.sessions[&active].read(cx).input.clone();
            shell.sessions[&active].update(cx, |view, cx| {
                view.input
                    .update(cx, |input, cx| input.set_text("Keep this draft".into(), cx));
                cx.notify();
            });
            window.focus(&input.focus_handle(cx), cx);
            input
        })
        .unwrap();
    visual.update(|_, cx| {
        cx.write_to_clipboard(ClipboardItem {
            entries: vec![gpui::ClipboardEntry::ExternalPaths(gpui::ExternalPaths(
                vec![path.clone()].into(),
            ))],
        })
    });
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    visual.simulate_keystrokes(&format!("{modifier}-v"));
    visual.run_until_parked();
    visual.refresh().unwrap();
    let thumbnail = visual.debug_bounds("composer-image-0").unwrap();
    visual.simulate_click(thumbnail.center(), Default::default());
    assert_eq!(
        visual.debug_bounds("image-lightbox").unwrap().size,
        window.update(visual, |_, w, _| w.viewport_size()).unwrap()
    );
    visual.simulate_keystrokes("escape");
    assert!(visual.debug_bounds("image-lightbox").is_none());
    window
        .update(visual, |shell, w, cx| {
            assert!(input.focus_handle(cx).is_focused(w));
            assert_eq!(input.read(cx).content.as_ref(), "Keep this draft");
            assert!(shell.image_lightbox.is_none());
        })
        .unwrap();

    // The remove button overlaps the thumbnail but must not open its preview.
    let thumbnail = visual.debug_bounds("composer-image-0").unwrap();
    visual.simulate_click(
        point(thumbnail.right() - px(4.), thumbnail.top() + px(4.)),
        Default::default(),
    );
    assert!(visual.debug_bounds("composer-image-0").is_none());
    assert!(visual.debug_bounds("image-lightbox").is_none());

    let data = base64::engine::general_purpose::STANDARD.encode(br##"<svg xmlns="http://www.w3.org/2000/svg" width="640" height="480"><rect width="640" height="480" fill="#4da3f5"/></svg>"##);
    window.update(visual, |shell, _, cx| {
        shell.sessions[&active].update(cx, |view, cx| {
            view.summary.has_messages = true;
            cx.notify();
        });
        let transcript = shell.sessions[&active].read(cx).transcript.clone();
        transcript.update(cx, |view, cx| view.patch(0, vec![
            gespan_core::transcript::Row { role: "user".into(), text: "Look at this".into(), first: true, sent_at: None },
            gespan_core::transcript::Row { role: gespan_core::transcript::attachments::ROLE.into(), text: serde_json::json!([{"kind":"image", "name":"Persisted image", "mimeType":"image/svg+xml", "data":data}]).to_string(), first: true, sent_at: None },
        ], cx));
    }).unwrap();
    visual.refresh().unwrap();
    let sent = visual.debug_bounds("transcript-image-0").unwrap();
    visual.simulate_click(sent.center(), Default::default());
    assert!(visual.debug_bounds("image-lightbox").is_some());
    assert!(visual.debug_bounds("lightbox-image").is_some());
    // A press on the backdrop closes immediately, just like onMouseDown in React.
    visual.simulate_mouse_down(point(px(4.), px(4.)), MouseButton::Left, Default::default());
    visual.simulate_mouse_up(point(px(4.), px(4.)), MouseButton::Left, Default::default());
    assert!(visual.debug_bounds("image-lightbox").is_none());
    let sent = visual.debug_bounds("transcript-image-0").unwrap();
    visual.simulate_click(sent.center(), Default::default());
    let close = visual.debug_bounds("close-image-preview").unwrap();
    visual.simulate_click(close.center(), Default::default());
    assert!(visual.debug_bounds("image-lightbox").is_none());
    std::fs::remove_file(path).unwrap();
}

#[gpui::test]
fn a_generated_image_shows_in_the_transcript_and_opens_full_screen(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::input::bind(cx);
        crate::image_lightbox::bind(cx);
        cx.set_reduce_motion(true);
    });
    let (window, _, active) = super::tests::window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let data_dir = tempfile::tempdir().unwrap();
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";
    let image = gespan_core::generated_image::save(
        data_dir.path(),
        png,
        "generated-image",
        Some("A fox".into()),
    )
    .unwrap();
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&active].update(cx, |view, cx| {
                view.summary.has_messages = true;
                cx.notify();
            });
            let transcript = shell.sessions[&active].read(cx).transcript.clone();
            transcript.update(cx, |view, cx| {
                view.patch(
                    0,
                    vec![
                        gespan_core::transcript::Row {
                            role: "user".into(),
                            text: "Draw a fox".into(),
                            first: true,
                            sent_at: None,
                        },
                        gespan_core::transcript::Row {
                            role: "assistant".into(),
                            text: "Here it is.".into(),
                            first: true,
                            sent_at: None,
                        },
                        gespan_core::transcript::Row {
                            role: gespan_core::generated_image::ROLE.into(),
                            text: serde_json::to_string(&image).unwrap(),
                            first: true,
                            sent_at: None,
                        },
                    ],
                    cx,
                )
            });
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    let shown = visual
        .debug_bounds("generated-image")
        .expect("the image is in the transcript");
    assert!(shown.size.height > px(0.));
    visual.simulate_click(shown.center(), Default::default());
    assert!(
        visual.debug_bounds("image-lightbox").is_some(),
        "a click opens it full screen"
    );
    visual.simulate_keystrokes("escape");
    assert!(visual.debug_bounds("image-lightbox").is_none());

    // A missing file keeps its row and says so instead of failing the transcript.
    std::fs::remove_file(&image.path).unwrap();
    window
        .update(visual, |shell, _, cx| {
            let transcript = shell.sessions[&active].read(cx).transcript.clone();
            transcript.update(cx, |view, cx| {
                let mut gone = image.clone();
                gone.path = data_dir
                    .path()
                    .join("missing.png")
                    .to_string_lossy()
                    .into_owned();
                view.patch(
                    2,
                    vec![gespan_core::transcript::Row {
                        role: gespan_core::generated_image::ROLE.into(),
                        text: serde_json::to_string(&gone).unwrap(),
                        first: true,
                        sent_at: None,
                    }],
                    cx,
                )
            });
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    assert!(visual.debug_bounds("generated-image").is_some());
}

#[gpui::test]
fn a_reply_shows_its_local_image_and_opens_its_local_video(cx: &mut TestAppContext) {
    cx.update(|cx| {
        crate::input::bind(cx);
        crate::image_lightbox::bind(cx);
        cx.set_reduce_motion(true);
    });
    let (window, _, active) = super::tests::window(cx);
    let visual = &mut VisualTestContext::from_window(window.into(), cx);
    let folder = tempfile::tempdir().unwrap();
    let shot = folder.path().join("shot.png");
    std::fs::write(&shot, base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=").unwrap()).unwrap();
    std::fs::write(folder.path().join("clip.mp4"), b"not really a video").unwrap();
    let reply = format!(
        "Before and after:\n\n![Screenshot]({})\n\n[the recording](clip.mp4)\n",
        shot.display()
    );
    window
        .update(visual, |shell, _, cx| {
            shell.sessions[&active].update(cx, |view, cx| {
                view.summary.has_messages = true;
                cx.notify();
            });
            let transcript = shell.sessions[&active].read(cx).transcript.clone();
            transcript.update(cx, |view, cx| {
                view.file_link_base = Some(folder.path().to_string_lossy().into_owned());
                view.patch(
                    0,
                    vec![
                        gespan_core::transcript::Row {
                            role: "user".into(),
                            text: "Show me".into(),
                            first: true,
                            sent_at: None,
                        },
                        gespan_core::transcript::Row {
                            role: "assistant".into(),
                            text: reply,
                            first: true,
                            sent_at: None,
                        },
                    ],
                    cx,
                )
            });
        })
        .unwrap();
    visual.run_until_parked();
    visual.refresh().unwrap();
    // The list measures rows on its first frames; settle before clicking.
    visual.run_until_parked();
    visual.refresh().unwrap();
    let video = visual
        .debug_bounds("local-video")
        .expect("the video replaces its link");
    visual.simulate_click(video.center(), Default::default());
    visual.run_until_parked();
    window
        .update(visual, |shell, _, cx| {
            let opened = shell
                .active_review(cx)
                .and_then(|review| review.read(cx).active_editor());
            assert_eq!(
                opened.map(|editor| editor.read(cx).path.clone()),
                Some(
                    folder
                        .path()
                        .join("clip.mp4")
                        .to_string_lossy()
                        .into_owned()
                ),
                "the video opens in the file viewer, resolved against the session folder"
            );
        })
        .unwrap();
    let image = visual
        .debug_bounds("local-image")
        .expect("the image replaces its link");
    assert_eq!((image.size.width, image.size.height), (px(320.), px(180.)));
    visual.simulate_click(image.center(), Default::default());
    assert!(
        visual.debug_bounds("image-lightbox").is_some(),
        "the image opens full screen"
    );
    visual.simulate_keystrokes("escape");
    assert!(visual.debug_bounds("image-lightbox").is_none());
}
