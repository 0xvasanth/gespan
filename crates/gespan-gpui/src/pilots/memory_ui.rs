use super::*;
use gpui::FontWeight;
use pilot::memory;

/// A topic note's line in the Memory list: its first line of prose, so the
/// list says what each note is about.
pub(super) fn topic_summary(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.trim_start_matches("- ").trim().to_string())
        .filter(|line| !line.is_empty())
}

impl Pilots {
    /// Setup › Memory: its files down a 240px column (MEMORY.md, then each
    /// topic note), and the chosen file at full width beside them.
    pub(super) fn memory_page(
        &mut self,
        pilot: &Pilot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let loaded = self.files.is_some();
        let open = self
            .memory_file
            .clone()
            .filter(|t| self.memory_topics.iter().any(|(name, _)| name == t));
        let mut entries = vec![(
            None::<String>,
            SharedString::from("MEMORY.md"),
            SharedString::from("Index of what it knows"),
        )];
        entries.extend(self.memory_topics.iter().map(|(name, file)| {
            let summary = file
                .text
                .as_deref()
                .and_then(topic_summary)
                .unwrap_or_else(|| format!("memory/{name}.md"));
            (Some(name.clone()), name.clone().into(), summary.into())
        }));
        let count = entries.len();
        let list = div()
            .id("pilot-memory-files")
            .debug_selector(|| "pilot-memory-files".into())
            .role(Role::List)
            .aria_label("Memory files")
            .w(px(240.))
            .flex_shrink_0()
            .h_full()
            .overflow_y_scroll()
            .px(px(10.))
            .py(px(14.))
            .border_r_1()
            .border_color(crate::glass::hairline())
            .flex()
            .flex_col()
            .gap(px(2.))
            .child(
                div()
                    .flex()
                    .items_center()
                    .px(px(6.))
                    .pb(px(8.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .child("Memory"),
                    )
                    .when(loaded, |d| {
                        d.child(
                            div()
                                .ml(px(8.))
                                .text_size(px(11.5))
                                .line_height(px(16.))
                                .text_color(content(0.54))
                                .child(if count == 1 {
                                    "1 file".to_string()
                                } else {
                                    format!("{count} files")
                                }),
                        )
                    }),
            )
            .children(
                entries
                    .into_iter()
                    .enumerate()
                    .map(|(ix, (topic, name, summary))| {
                        let on = topic == open;
                        div()
                            .id(("pilot-memory-file", ix))
                            .debug_selector(move || format!("pilot-memory-file-{ix}"))
                            .role(Role::ListItem)
                            .aria_label(name.clone())
                            .aria_selected(on)
                            .cursor_pointer()
                            .flex_shrink_0()
                            .px(px(10.))
                            .py(px(8.))
                            .rounded(px(10.))
                            .flex()
                            .items_start()
                            .gap(px(10.))
                            .map(|d| {
                                if on {
                                    d.bg(crate::glass::selected_fill()).shadow(vec![
                                        gpui::BoxShadow::new(
                                            px(0.),
                                            px(1.),
                                            crate::theme::white(0.08),
                                        )
                                        .inset(),
                                    ])
                                } else {
                                    d.hover(|s| s.bg(crate::glass::hover_fill()))
                                }
                            })
                            .child(div().pt(px(1.)).child(
                                icon("file", 15.).text_color(content(if on { 0.86 } else { 0.54 })),
                            ))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .flex()
                                    .flex_col()
                                    .gap(px(2.))
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(13.))
                                            .line_height(px(18.))
                                            .text_color(content(1.))
                                            .child(name),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(11.5))
                                            .line_height(px(16.))
                                            .text_color(content(0.54))
                                            .child(summary),
                                    ),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_memory_file(topic.clone(), cx)
                            }))
                    }),
            );
        let body = match open {
            Some(topic) => self.memory_topic(pilot, &topic, cx),
            None => self.memory_index(pilot, window, cx),
        };
        div()
            .id("pilot-memory-page")
            .debug_selector(|| "pilot-memory-page".into())
            .size_full()
            .flex()
            .child(list)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .child(body),
            )
            .into_any_element()
    }
    /// Shows `topic` (or MEMORY.md for `None`), saving an edit in progress.
    pub(super) fn open_memory_file(&mut self, topic: Option<String>, cx: &mut Context<Self>) {
        if self.memory_file == topic {
            return;
        }
        self.save_open_file(cx);
        self.memory_file = topic;
        self.memory_topic_editing = false;
        self.memory_adding = false;
        self.memory_edit = None;
        cx.notify();
    }
    /// The open file's head: its name, one line about it, then its actions.
    fn memory_file_header(
        name: SharedString,
        about: String,
        actions: impl IntoIterator<Item = AnyElement>,
    ) -> gpui::Div {
        div()
            .debug_selector(|| "pilot-memory-header".into())
            .flex_shrink_0()
            .px(px(28.))
            .py(px(14.))
            .border_b_1()
            .border_color(crate::glass::hairline())
            .flex()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .truncate()
                            .text_size(px(13.))
                            .line_height(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(content(1.))
                            .child(name),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .line_height(px(16.))
                            .text_color(content(0.54))
                            .child(about),
                    ),
            )
            .child(div().flex().gap(px(6.)).children(actions))
    }
    /// MEMORY.md: its facts, each editable or forgettable, and the gauge of
    /// how much of it loads into every turn.
    fn memory_index(
        &mut self,
        pilot: &Pilot,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let files = self.files.clone();
        let mut rows = Vec::new();
        if self.memory_adding {
            rows.push(self.memory_editor(pilot, None, window, cx));
        }
        if let Some(files) = &files {
            rows.extend(self.memory_facts(pilot, &files.memory, window, cx));
        }
        let budget = files.as_ref().map(|files| {
            let loaded = memory::within_budget(&files.memory);
            let lines = loaded.lines().count();
            let dropped = files.memory.lines().count().saturating_sub(lines);
            let share = (lines as f32 / memory::MAX_LINES as f32)
                .max(loaded.len() as f32 / memory::MAX_BYTES as f32)
                .min(1.);
            (lines, dropped, share)
        });
        let about = match budget {
            Some((_, dropped, _)) if dropped > 0 => {
                format!("Loads into every turn · {dropped} lines not loading")
            }
            Some((lines, _, _)) => {
                format!(
                    "Loads into every turn · {lines} / {} lines",
                    memory::MAX_LINES
                )
            }
            None => "Loads into every turn".into(),
        };
        let add = files.is_some().then(|| {
            crate::glass::button("pilot-add-memory", "Add", crate::glass::Button::Glass)
                .debug_selector(|| "pilot-add-memory".into())
                .role(Role::Button)
                .aria_label("Add memory")
                .h(px(28.))
                .text_size(px(12.))
                .when(self.memory_adding, |d| d.opacity(0.5))
                .on_click(cx.listener(move |this, _, window, cx| {
                    if this.memory_adding {
                        return;
                    }
                    this.memory_edit = None;
                    this.memory_adding = true;
                    this.error = None;
                    let editor = this.field("memory-fact", "New memory", String::new(), false, cx);
                    editor.update(cx, |f, cx| f.set_text(String::new(), cx));
                    window.focus(&editor.focus_handle(cx), cx);
                    cx.notify();
                }))
                .into_any_element()
        });
        let gauge = budget.map(|(lines, dropped, share)| {
            let tone = if dropped > 0 {
                gpui::Hsla::from(rgb(0xfb2c36)).opacity(0.7)
            } else if share >= 0.8 {
                gpui::Hsla::from(rgb(0xffb900)).opacity(0.7)
            } else {
                content(0.3)
            };
            div()
                .flex_shrink_0()
                .border_t_1()
                .border_color(crate::glass::hairline())
                .px(px(28.))
                .py(px(10.))
                .flex()
                .items_center()
                .gap_2()
                .text_size(px(11.))
                .line_height(px(16.))
                .text_color(content(0.45))
                .child(
                    div()
                        .flex_1()
                        .max_w(px(640.))
                        .h(px(4.))
                        .rounded_full()
                        .overflow_hidden()
                        .bg(content(0.08))
                        .child(
                            div()
                                .h_full()
                                .w(gpui::relative(share))
                                .rounded_full()
                                .bg(tone),
                        ),
                )
                .child(if dropped > 0 {
                    format!("{dropped} lines not loading")
                } else {
                    format!("{lines} / {} lines", memory::MAX_LINES)
                })
        });
        let empty = |text: &'static str| {
            div()
                .px_2()
                .py_6()
                .text_size(px(12.))
                .text_color(content(0.45))
                .child(text)
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(Self::memory_file_header("MEMORY.md".into(), about, add))
            .child(
                div()
                    .id("pilot-memory-facts")
                    .debug_selector(|| "pilot-memory-facts".into())
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(20.))
                    .py(px(8.))
                    .child(
                        div()
                            .max_w(px(656.))
                            .flex()
                            .flex_col()
                            .gap_px()
                            .when(files.is_none(), |d| d.child(empty("Loading…")))
                            .when(files.is_some() && rows.is_empty(), |d| {
                                d.child(empty("Nothing remembered yet"))
                            })
                            .children(rows),
                    ),
            )
            .children(gauge)
            .into_any_element()
    }
    /// A topic note under memory/: read as a document, or edited in place.
    fn memory_topic(&mut self, pilot: &Pilot, topic: &str, cx: &mut Context<Self>) -> AnyElement {
        let path = format!("memory/{topic}.md");
        let Some(file) = self
            .memory_topics
            .iter()
            .find(|(name, _)| name == topic)
            .map(|(_, file)| file.clone())
        else {
            return div().into_any_element();
        };
        let editing = self.memory_topic_editing;
        let key = format!("file:{}:{path}", pilot.id);
        let text = self
            .fields
            .get(&key)
            .map(|f| f.read(cx).content.to_string())
            .unwrap_or_else(|| file.text.clone().unwrap_or_default());
        let words = text.split_whitespace().count();
        let about = format!(
            "{path} · read when it needs it · {words} {}",
            if words == 1 { "word" } else { "words" }
        );
        let toggle = crate::glass::button(
            "pilot-memory-topic-edit",
            if editing { "Done" } else { "Edit" },
            crate::glass::Button::Glass,
        )
        .debug_selector(|| "pilot-memory-topic-edit".into())
        .role(Role::Button)
        .aria_label(if editing {
            "Done editing"
        } else {
            "Edit topic"
        })
        .h(px(28.))
        .text_size(px(12.))
        .on_click(cx.listener(|this, _, _, cx| {
            if this.memory_topic_editing {
                this.save_open_file(cx);
            }
            this.memory_topic_editing = !this.memory_topic_editing;
            cx.notify();
        }))
        .into_any_element();
        let body = if editing {
            let editor = self.file_editor(&path, file.text.unwrap_or_default(), file.hash, cx);
            let focus = editor.focus_handle(cx);
            div()
                .id("pilot-memory-topic-editor")
                .debug_selector(|| "pilot-memory-topic-editor".into())
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(28.))
                .py(px(12.))
                .font_family(crate::fonts::monospace())
                .text_size(px(13.))
                .line_height(px(20.))
                .text_color(content(0.85))
                .on_mouse_down(MouseButton::Left, move |_, w, cx| w.focus(&focus, cx))
                .child(div().max_w(px(640.)).child(editor))
                .into_any_element()
        } else {
            let mut column = div().max_w(px(640.)).w_full().flex().flex_col().gap(px(8.));
            if text.trim().is_empty() {
                column = column.child(
                    div()
                        .text_size(px(13.))
                        .text_color(content(0.54))
                        .child("This note is empty."),
                );
            }
            for (ix, row) in crate::markdown_model::parse_full(&text)
                .blocks
                .iter()
                .flat_map(|b| crate::markdown::rows(&b.block))
                .enumerate()
            {
                column = column.child(div().w_full().min_w_0().child(crate::markdown::body(
                    &row,
                    format!("pilot-memory-topic-row-{ix}").into(),
                    crate::markdown::Tone::Normal,
                    None,
                    std::time::Instant::now(),
                )));
            }
            div()
                .id("pilot-memory-topic-read")
                .debug_selector(|| "pilot-memory-topic-read".into())
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .px(px(28.))
                .pt(px(14.))
                .pb(px(28.))
                .child(column)
                .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .child(Self::memory_file_header(
                topic.to_string().into(),
                about,
                [toggle],
            ))
            .child(body)
            .into_any_element()
    }
    fn memory_icon(dim: bool) -> gpui::Div {
        div()
            .size(px(28.))
            .flex_shrink_0()
            .rounded_md()
            .bg(content(0.06))
            .flex()
            .items_center()
            .justify_center()
            .child(icon("sticky-note", 14.).text_color(content(if dim { 0.3 } else { 0.6 })))
    }
    fn memory_editor(
        &mut self,
        pilot: &Pilot,
        original: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = original
            .map(|line| {
                format!(
                    "memory-edit:{}:{}",
                    pilot.id,
                    pilot::files::content_hash(line)
                )
            })
            .unwrap_or_else(|| "memory-fact".into());
        let editor = self.field(
            &key,
            if original.is_some() {
                "Edit memory"
            } else {
                "New memory"
            },
            original.map(memory::fact).unwrap_or_default(),
            false,
            cx,
        );
        editor.update(cx, |f, cx| {
            f.set_placeholder("Something it should remember", cx)
        });
        if self.field_blurs.insert(editor.entity_id()) {
            let (pilot, row, field) = (pilot.id.clone(), original.map(str::to_owned), key.clone());
            self.observations.push(cx.subscribe(
                &editor,
                move |this, _, _: &input::Submitted, cx| {
                    this.commit_memory_input(&pilot, &field, row.as_deref(), false, cx)
                },
            ));
            let (pilot, row, field) = (
                self.active.clone().unwrap_or_default(),
                original.map(str::to_owned),
                key.clone(),
            );
            self.observations.push(cx.on_blur(
                &editor.focus_handle(cx),
                window,
                move |this, _, cx| {
                    this.commit_memory_input(&pilot, &field, row.as_deref(), true, cx)
                },
            ));
        }
        div()
            .id(SharedString::from(format!("memory-editor-{key}")))
            .flex()
            .items_start()
            .gap_2p5()
            .rounded_md()
            .bg(content(0.05))
            .px_2()
            .py_2()
            .child(Self::memory_icon(false))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .py(px(5.))
                    .text_color(content(0.9))
                    .child(editor),
            )
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    this.memory_adding = false;
                    this.memory_edit = None;
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .into_any_element()
    }
    pub(super) fn memory_facts(
        &mut self,
        pilot: &Pilot,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut result = Vec::new();
        for (index, line) in text
            .lines()
            .enumerate()
            .filter(|(_, l)| !l.trim().is_empty() && !l.trim().starts_with('#'))
        {
            if self.memory_edit.as_deref() == Some(line) {
                result.push(self.memory_editor(pilot, Some(line), window, cx));
                continue;
            }
            let struck = line.contains("~~");
            let fact = memory::fact(line);
            let date = line
                .trim_start_matches("- ")
                .trim_start_matches("~~")
                .get(..10)
                .and_then(|day| chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d").ok())
                .map(|day| day.format("%b %-d").to_string());
            let edit = line.to_string();
            let remove = edit.clone();
            let edit_button = edit.clone();
            result.push(
                div()
                    .id(("pilot-memory-fact", index))
                    .group("memory-fact")
                    .rounded_md()
                    .flex()
                    .items_center()
                    .hover(|s| s.bg(content(0.05)))
                    .child(
                        div()
                            .id(("memory-fact-body", index))
                            .role(Role::Button)
                            .aria_label(format!("Edit memory: {fact}"))
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2p5()
                            .px_2()
                            .py_2()
                            .child(Self::memory_icon(struck))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_size(px(12.))
                                    .line_height(px(20.))
                                    .text_color(content(if struck { 0.35 } else { 0.85 }))
                                    .child(
                                        div()
                                            .truncate()
                                            .when(struck, |d| d.line_through())
                                            .child(fact),
                                    )
                                    .when_some(date, |d, date| {
                                        d.child(
                                            div()
                                                .text_size(px(11.))
                                                .line_height(px(16.))
                                                .text_color(content(0.4))
                                                .child(date),
                                        )
                                    }),
                            )
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if struck {
                                    return;
                                }
                                this.memory_adding = false;
                                this.memory_edit = Some(edit.clone());
                                this.error = None;
                                let key = format!(
                                    "memory-edit:{}:{}",
                                    this.active.as_deref().unwrap_or_default(),
                                    pilot::files::content_hash(&edit)
                                );
                                let field =
                                    this.field(&key, "Edit memory", memory::fact(&edit), false, cx);
                                window.focus(&field.focus_handle(cx), cx);
                                cx.notify();
                            })),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .flex()
                            .items_center()
                            .pr_2()
                            .opacity(0.)
                            .group_hover("memory-fact", |s| s.opacity(1.))
                            .when(!struck, |d| {
                                d.child(
                                    icon_button(
                                        "pilot-memory-edit",
                                        "Edit memory fact",
                                        "pencil-edit",
                                    )
                                    .on_click(cx.listener(
                                        move |this, _, window, cx| {
                                            this.memory_adding = false;
                                            this.memory_edit = Some(edit_button.clone());
                                            let key = format!(
                                                "memory-edit:{}:{}",
                                                this.active.as_deref().unwrap_or_default(),
                                                pilot::files::content_hash(&edit_button)
                                            );
                                            let field = this.field(
                                                &key,
                                                "Edit memory",
                                                memory::fact(&edit_button),
                                                false,
                                                cx,
                                            );
                                            window.focus(&field.focus_handle(cx), cx);
                                            cx.notify();
                                        },
                                    )),
                                )
                            })
                            .child(
                                icon_button("pilot-memory-forget", "Forget memory fact", "trash")
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.edit_memory_fact(&remove, None, cx)
                                    })),
                            ),
                    )
                    .into_any_element(),
            );
        }
        if let Some(original) = self.memory_edit.clone()
            && !text.lines().any(|line| line == original)
        {
            result.push(self.memory_editor(pilot, Some(&original), window, cx));
        }
        result
    }
    fn commit_memory_input(
        &mut self,
        id: &str,
        key: &str,
        original: Option<&str>,
        close: bool,
        cx: &mut Context<Self>,
    ) {
        let active_editor = if original.is_some() {
            self.memory_edit.as_deref() == original
        } else {
            self.memory_adding
        };
        if !active_editor || self.active.as_deref() != Some(id) {
            return;
        }
        let text = self
            .value(key, cx)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if text.is_empty() {
            if original.is_some() {
                self.memory_edit = None;
            } else {
                self.memory_adding = false;
            }
            cx.notify();
            return;
        }
        if !self.memory_pending.insert(key.to_string()) {
            return;
        }
        let (root, id, key, original) = (
            self.root.clone(),
            id.to_string(),
            key.to_string(),
            original.map(str::to_owned),
        );
        let (target, content) = (id.clone(), text.clone());
        let edited = original.clone();
        let work = cx.background_executor().spawn(async move {
            memory::edit(&root, &target, None, |memory| {
                if let Some(original) = &edited {
                    memory::edit_line(memory, original, Some(&content))
                } else {
                    Ok(memory::add(
                        memory,
                        &memory::entry(&content, &memory::date(), None)?,
                    ))
                }
            })
            .map(|_| ())
        });
        cx.spawn(async move |this, cx| {
            let result = work.await;
            let _ = this.update(cx, |this, cx| {
                this.memory_pending.remove(&key);
                match result {
                    Ok(()) => {
                        if original.is_some() {
                            this.memory_edit = None;
                        } else if close {
                            this.memory_adding = false;
                        } else if let Some(field) = this.fields.get(&key)
                            && field
                                .read(cx)
                                .content
                                .split_whitespace()
                                .collect::<Vec<_>>()
                                .join(" ")
                                == text
                        {
                            field.update(cx, |f, cx| f.set_text(String::new(), cx));
                        }
                        this.error = None;
                        this.reload_files(&id, cx);
                    }
                    Err(error) => this.error = Some(error),
                }
                this.changed(cx);
            });
        })
        .detach();
    }
    fn edit_memory_fact(
        &mut self,
        original: &str,
        replacement: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let Some(id) = self.active.clone() else {
            return;
        };
        let (root, target, original) = (self.root.clone(), id.clone(), original.to_string());
        self.file_work(
            id,
            move || {
                memory::edit(&root, &target, None, |text| {
                    memory::edit_line(text, &original, replacement.as_deref())
                })
                .map(|_| ())
            },
            cx,
        );
    }
}
