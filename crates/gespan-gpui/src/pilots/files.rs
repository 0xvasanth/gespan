use super::*;
use gespan_core::pilot::files;

impl Pilots {
    pub(crate) fn reset_defaults(
        &mut self,
        id: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Task<Result<(), String>> {
        let root = self.root.clone();
        let id = id.to_owned();
        let target = id.clone();
        let work = cx.background_executor().spawn(async move {
            let current = files::load(&root, &target, None)?;
            files::save(
                &root,
                &target,
                None,
                "SOUL.md",
                pilot::DEFAULT_SOUL,
                Some(&current.soul_hash),
            )?;
            files::load(&root, &target, None)
        });
        cx.spawn(async move |this, cx| {
            let files = work.await?;
            this.update(cx, |this, cx| {
                let pilot = this.roster.get_mut(&id).ok_or("Pilot no longer exists")?;
                pilot.name.clear();
                let key = format!("{id}:SOUL.md");
                this.fields.remove(&format!("file:{key}"));
                this.file_versions.remove(&key);
                if this.active.as_deref() == Some(&id) {
                    this.files = Some(files);
                }
                this.configure_chat(&id, cx);
                this.save(cx);
                Ok(())
            })
            .map_err(|e| e.to_string())?
        })
    }
    pub(super) fn file_editor(
        &mut self,
        path: &str,
        text: String,
        hash: String,
        cx: &mut Context<Self>,
    ) -> Entity<Input> {
        let key = format!("{}:{path}", self.active.as_deref().unwrap_or_default());
        let field_key = format!("file:{key}");
        if let Some((old, old_hash)) = self.file_versions.get(&key)
            && old_hash != &hash
            && !self.saving_files.contains(&key)
            && let Some(field) = self
                .fields
                .get(&field_key)
                .filter(|f| f.read(cx).content.as_str() == old)
        {
            field.update(cx, |f, cx| f.set_text(text.clone(), cx));
            self.file_versions
                .insert(key.clone(), (text.clone(), hash.clone()));
        }
        self.file_versions
            .entry(key)
            .or_insert((text.clone(), hash));
        self.field(&field_key, "Markdown", text, true, cx)
    }
    /// Setup › Soul: SOUL.md read as a document, with Read and Edit modes.
    /// Long instructions stay readable in one 680px column.
    pub(super) fn soul_setup(&mut self, pilot: &Pilot, cx: &mut Context<Self>) -> AnyElement {
        let editing = self.soul_editing;
        let text = self.files.as_ref().map(|files| {
            let key = format!("file:{}:SOUL.md", pilot.id);
            self.fields
                .get(&key)
                .map(|f| f.read(cx).content.to_string())
                .unwrap_or_else(|| files.soul.clone().unwrap_or_default())
        });
        let words = text.as_deref().map_or(0, |t| t.split_whitespace().count());
        let header = div()
            .debug_selector(|| "pilot-soul-header".into())
            .flex_shrink_0()
            .px(px(28.))
            .py(px(14.))
            .border_b_1()
            .border_color(content(0.06))
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .child("Soul"),
                    )
                    .child(
                        div()
                            .debug_selector(|| "pilot-soul-words".into())
                            .text_size(px(11.5))
                            .text_color(content(0.54))
                            .child(format!(
                                "SOUL.md · read before every run · {words} {}",
                                if words == 1 { "word" } else { "words" }
                            )),
                    ),
            )
            .child(div().flex_1())
            .child(
                crate::glass::segmented()
                    .id("pilot-soul-mode")
                    .role(Role::TabList)
                    .aria_label("Soul view")
                    .children([("Read", false), ("Edit", true)].map(|(label, edit)| {
                        crate::glass::segment(
                            SharedString::from(format!("pilot-soul-{label}")),
                            label,
                            editing == edit,
                        )
                        .debug_selector(move || format!("pilot-soul-{label}"))
                        .role(Role::Tab)
                        .aria_label(label)
                        .aria_selected(editing == edit)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if this.soul_editing && !edit {
                                this.save_open_file(cx);
                            }
                            this.soul_editing = edit;
                            cx.notify();
                        }))
                    })),
            );
        let body = if editing {
            div()
                .flex_1()
                .min_h_0()
                .px(px(16.))
                .flex()
                .flex_col()
                .child(
                    div()
                        .max_w(px(712.))
                        .flex_1()
                        .min_h_0()
                        .flex()
                        .flex_col()
                        .child(self.soul_page(pilot, cx)),
                )
                .into_any_element()
        } else {
            let mut column = div().max_w(px(680.)).w_full().flex().flex_col().gap(px(8.));
            match text.as_deref() {
                None => column = column.child(div().text_size(px(12.)).text_color(content(0.45)).child("Loading…")),
                Some(t) if t.trim().is_empty() => {
                    column = column.child(
                        div()
                            .text_size(px(13.))
                            .text_color(content(0.54))
                            .child("No standing instructions yet. Choose Edit to write who it is and what it should always keep in mind."),
                    )
                }
                Some(t) => {
                    for (ix, row) in crate::markdown_model::parse_full(t)
                        .blocks
                        .iter()
                        .flat_map(|b| crate::markdown::rows(&b.block))
                        .enumerate()
                    {
                        column = column.child(div().w_full().min_w_0().child(crate::markdown::body(
                            &row,
                            format!("pilot-soul-row-{ix}").into(),
                            crate::markdown::Tone::Normal,
                            None,
                            std::time::Instant::now(),
                        )));
                    }
                }
            }
            div()
                .id("pilot-soul-read")
                .debug_selector(|| "pilot-soul-read".into())
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(28.))
                .pt(px(18.))
                .pb(px(28.))
                .child(column)
                .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(header)
            .child(body)
            .into_any_element()
    }
    pub(super) fn soul_page(&mut self, pilot: &Pilot, cx: &mut Context<Self>) -> AnyElement {
        let body = if let Some(files) = self.files.clone() {
            let editor = self.file_editor(
                "SOUL.md",
                files.soul.unwrap_or_default(),
                files.soul_hash,
                cx,
            );
            editor.update(cx, |v, cx| {
                v.set_placeholder("Who it is, and what it should always keep in mind", cx)
            });
            let focus = editor.focus_handle(cx);
            div()
                .id("pilot-soul-editor")
                .debug_selector(|| "pilot-soul-editor".into())
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px_3()
                .py_3()
                .font_family(crate::fonts::monospace())
                .text_size(px(13.))
                .line_height(px(20.))
                .text_color(content(0.85))
                .on_mouse_down(MouseButton::Left, move |_, w, cx| w.focus(&focus, cx))
                .child(editor)
                .into_any_element()
        } else {
            div()
                .flex_1()
                .p_6()
                .text_center()
                .text_size(px(12.))
                .text_color(content(0.45))
                .child("Loading…")
                .into_any_element()
        };
        let _ = pilot;
        div()
            .id("pilot-soul-page")
            .debug_selector(|| "pilot-soul-page".into())
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(Self::settings_page_header("Soul"))
            .child(body)
            .into_any_element()
    }
    pub(super) fn settings_page_header(title: &'static str) -> gpui::Div {
        div()
            .h(px(40.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .pl_4()
            .pr(px(44.))
            .border_b_1()
            .border_color(content(0.04))
            .text_size(px(13.))
            .font_weight(gpui::FontWeight::MEDIUM)
            .child(div().flex_1().min_w_0().child(title))
    }
    pub(super) fn bind_settings_blurs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let fields = self
            .fields
            .iter()
            .filter(|(key, _)| key.starts_with("file:"))
            .map(|(_, field)| field.clone())
            .collect::<Vec<_>>();
        for field in fields {
            if self.field_blurs.insert(field.entity_id()) {
                self.observations.push(cx.on_blur(
                    &field.focus_handle(cx),
                    window,
                    |this, _, cx| this.save_open_file(cx),
                ));
            }
        }
    }
    pub(super) fn save_open_file(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.active.clone() else {
            return;
        };
        let prefix = format!("{id}:");
        for (key, (old, hash)) in self.file_versions.clone() {
            let Some(path) = key.strip_prefix(&prefix).map(str::to_owned) else {
                continue;
            };
            let Some(field) = self.fields.get(&format!("file:{key}")) else {
                continue;
            };
            let text = field.read(cx).content.to_string();
            if text == old || hash.is_empty() || !self.saving_files.insert(key.clone()) {
                continue;
            }
            let root = self.root.clone();
            let id = id.clone();
            let target = id.clone();
            let content = text.clone();
            let work = cx.background_executor().spawn(async move {
                files::save(&root, &target, None, &path, &content, Some(&hash))
            });
            cx.spawn(async move |this, cx| {
                let result = work.await;
                let _ = this.update(cx, |this, cx| {
                    this.saving_files.remove(&key);
                    match result {
                        Ok(hash) => {
                            this.file_versions.insert(key, (text, hash));
                            this.reload_files(&id, cx);
                        }
                        Err(e) => {
                            this.error = Some(format!(
                                "Could not save: {e}. Your edits are still in the editor."
                            ))
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    pub(super) fn file_work(
        &mut self,
        id: String,
        work: impl FnOnce() -> Result<(), String> + Send + 'static,
        cx: &mut Context<Self>,
    ) {
        let work = cx.background_executor().spawn(async move { work() });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(()) => {
                        this.reload_files(&id, cx);
                    }
                    Err(e) => this.error = Some(e),
                }
                this.changed(cx);
            });
        })
        .detach();
    }
    pub(super) fn reset_conversation(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt = window.prompt(
            gpui::PromptLevel::Warning,
            "Reset conversation?",
            Some("The chat will be cleared. Its soul, memory and habits stay."),
            &["Cancel", "Reset"],
            cx,
        );
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            if prompt.await.ok() != Some(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if let Some(view) = this.chats.remove(&id) {
                    view.update(cx, |v, cx| v.pilot_stop(cx));
                }
                this.chat_choices.remove(&id);
                if let Some(pilot) = this.roster.get_mut(&id) {
                    pilot.session_id = None;
                    pilot.unavailable_session = None;
                    pilot.seen_rows = 0;
                    pilot.outbox.clear();
                }
                this.save(cx);
                this.ensure_chat(&id, cx);
            });
        })
        .detach();
    }
    /// Confirm, then put the pilot's soul and name back to the defaults.
    pub(super) fn reset_pilot(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self.roster.get(id).map(|m| m.label()) else {
            return;
        };
        let prompt = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Reset {name} to its defaults?"),
            Some("Its soul goes back to the default and its name too. Changes to its soul can't be recovered. Its conversation, projects, memory and habits are kept."),
            &["Cancel", "Reset"],
            cx,
        );
        let id = id.to_string();
        let handle = window.window_handle();
        cx.spawn(async move |this, cx| {
            if prompt.await.ok() != Some(1) {
                return;
            }
            let reset = this.update(cx, |this, cx| this.reset_defaults(&id, cx));
            let Ok(reset) = reset else { return };
            let result = reset.await;
            let _ = this.update(cx, |this, cx| this.changed(cx));
            if let Err(e) = result
                && let Ok(ack) = handle.update(cx, |_, window, cx| {
                    window.prompt(
                        gpui::PromptLevel::Critical,
                        &format!("Couldn't reset {name}"),
                        Some(&e),
                        &["OK"],
                        cx,
                    )
                })
            {
                let _ = ack.await;
            }
        })
        .detach();
    }
    pub(super) fn delete_pilot(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let prompt = window.prompt(
            gpui::PromptLevel::Warning,
            "Delete pilot?",
            Some("Its scheduled habits stop. Sessions and notes it created stay saved."),
            &["Cancel", "Delete"],
            cx,
        );
        let id = id.to_string();
        cx.spawn(async move |this, cx| {
            if prompt.await.ok() != Some(1) {
                return;
            }
            let _ = this.update(cx, |this, cx| {
                if let Some(view) = this.chats.remove(&id) {
                    view.update(cx, |v, cx| v.pilot_stop(cx));
                }
                this.chat_choices.remove(&id);
                for run in this.running.values().filter(|r| r.pilot == id) {
                    run.view.update(cx, |v, cx| v.pilot_stop(cx));
                }
                this.roster.pilots.retain(|m| m.id != id);
                this.roadmaps.remove(&id);
                if this.active.as_ref() == Some(&id) {
                    this.active = None;
                }
                this.save(cx);
            });
        })
        .detach();
    }
}
