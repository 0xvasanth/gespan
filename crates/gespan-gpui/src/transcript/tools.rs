//! Compact semantic tool captions with clickable file targets.
use super::{TranscriptView, tasks};
use crate::theme::*;
use gespan_core::tool::{CommandStep, ToolCall, ToolState};
use gpui::{AnyElement, Context, Div, Role, div, prelude::*, px, rgb};

/// Shell plumbing that says nothing about what a chained command was for.
fn is_noise(step: &CommandStep) -> bool {
    step.action == "Run"
        && matches!(
            step.command.split_whitespace().next(),
            Some("cd" | "pwd" | "echo" | "printf" | "true" | "false" | "sleep" | "clear")
        )
}

/// `git status --short` reads as `git status`; flags and paths are detail.
fn command_label(command: &str) -> String {
    let mut words = command.split_whitespace();
    let program = words.next().unwrap_or("");
    let program = program.rsplit('/').next().unwrap_or(program);
    match words.next() {
        Some(sub)
            if !sub.starts_with('-')
                && !sub.contains(['/', '.', '=', '\'', '"'])
                && sub.len() <= 16 =>
        {
            format!("{program} {sub}")
        }
        _ => program.to_owned(),
    }
}

/// A shared fixed box keeps overflow counts aligned with the pattern pills.
fn search_pill() -> Div {
    div()
        .flex()
        .min_w_0()
        .h(px(24.))
        .items_center()
        .px_2()
        .border_1()
        .border_color(content(0.))
        .rounded(px(6.))
        .bg(content(0.06))
}

impl TranscriptView {
    pub(super) fn tool(
        &mut self,
        tool: ToolCall,
        start: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        self.tool_in(tool, start, None, None, cx)
    }

    pub(super) fn tool_in(
        &mut self,
        tool: ToolCall,
        start: usize,
        scope: Option<&str>,
        width: Option<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some(panel) = tasks::render(self, &tool, start, width, cx) {
            return panel;
        }
        let identity = scope.map_or_else(|| start.to_string(), |scope| format!("{start}:{scope}"));
        if tool.is_milestone() {
            let running = tool.state == ToolState::Running;
            let failed = tool.state == ToolState::Failed;
            let label = tool.summary().0;
            return div()
                .id(gpui::SharedString::from(format!("tool:{identity}")))
                .debug_selector(move || format!("tool-{start}"))
                .role(Role::Group)
                .aria_label(label.clone())
                .flex()
                .w_full()
                .min_w_0()
                .items_center()
                .gap_2()
                .py_1()
                .line_height(px(20.))
                .text_color(if failed {
                    rgb(0xf87171).into()
                } else {
                    content(0.55)
                })
                .child(if running {
                    div()
                        .size(px(14.))
                        .flex_shrink_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(crate::session_mark::spinner(
                            gpui::SharedString::from(format!("compaction-loader:{identity}")),
                            crate::pulse::Rate::Half,
                        ))
                        .into_any_element()
                } else {
                    let glyph = match tool.state {
                        ToolState::Completed => "check",
                        ToolState::Failed => "close",
                        _ => "minus",
                    };
                    icon(glyph, 14.)
                        .text_color(if failed {
                            rgb(0xf87171).into()
                        } else {
                            content(0.45)
                        })
                        .into_any_element()
                })
                .child(div().min_w_0().truncate().child(if running {
                    crate::activity::shimmer(label, self.started_at.elapsed().as_secs_f32())
                } else {
                    div().child(label).into_any_element()
                }))
                .into_any_element();
        }
        let shown = tool.presentation_in(self.file_link_base.as_deref());
        let visual = tool.visual_step().is_some();
        let failed = matches!(tool.state, ToolState::Failed | ToolState::Cancelled);
        let is_search =
            matches!(shown.action.as_str(), "Find" | "Search") && !shown.target.is_empty();
        let action = if is_search { "Search" } else { &shown.action };
        let is_file = matches!(shown.action.as_str(), "Read" | "Edit" | "Write" | "List");
        let target_path = if is_file {
            shown.file_path.as_deref().map(|path| {
                let (path, location) = crate::file_editor::file_location(
                    path,
                    self.file_link_base.as_deref().unwrap_or(""),
                );
                if location.is_none() {
                    shown
                        .start_line
                        .map_or(path.clone(), |line| format!("{path}:{line}"))
                } else {
                    location.map_or(path.clone(), |(line, col)| format!("{path}:{line}:{col}"))
                }
            })
        } else {
            None
        };
        let link = |path: &str, line: Option<u64>| {
            let (path, location) = crate::file_editor::file_location(
                path,
                self.file_link_base.as_deref().unwrap_or(""),
            );
            match location {
                Some((line, col)) => format!("{path}:{line}:{col}"),
                None => line.map_or(path.clone(), |line| format!("{path}:{line}")),
            }
        };
        let chain = shown.action == "Run" && !shown.steps.is_empty();
        let caption = match (&shown.description, chain) {
            (Some(description), true) => description.clone(),
            _ => format!("{action} {}", shown.target),
        };
        let state_label = match tool.state {
            ToolState::Running => "Running",
            ToolState::Completed => "Completed",
            ToolState::Failed => "Failed",
            ToolState::Cancelled => "Cancelled",
        };
        // Chained commands and failures share one disclosure: steps, then error.
        let error_key = format!("tool-detail:{start}:{}", tool.id);
        let error_open = self.agent_panels.part_open(&error_key, false);
        let has_error = failed && !tool.output.trim().is_empty();
        let has_detail = has_error || chain;
        let chips = if chain {
            let mut seen = std::collections::HashSet::new();
            let chips: Vec<_> = shown
                .steps
                .iter()
                .filter(|step| !is_noise(step))
                .filter_map(|step| {
                    let (label, path) = match step.action {
                        "Read" | "List" => (
                            step.target
                                .rsplit(['/', '\\'])
                                .find(|part| !part.is_empty())
                                .unwrap_or(&step.target)
                                .to_owned(),
                            step.file_path
                                .as_deref()
                                .map(|path| link(path, step.start_line)),
                        ),
                        "Find" => (step.target.clone(), None),
                        _ => (command_label(&step.command), None),
                    };
                    seen.insert((step.action, label.clone()))
                        .then_some((step.action, label, path))
                })
                .collect();
            chips
        } else {
            Vec::new()
        };
        let steps = if chain {
            shown
                .steps
                .iter()
                .map(|step| {
                    let path = step
                        .file_path
                        .as_deref()
                        .map(|path| link(path, step.start_line));
                    (step.op, step.command.clone(), path)
                })
                .collect()
        } else {
            Vec::new()
        };
        let error_detail = tool.output.clone();
        let edit_preview = matches!(shown.action.as_str(), "Edit" | "Write")
            .then(|| tool.edit_preview())
            .flatten()
            .filter(|preview| preview.shows_change())
            .map(|preview| super::edit_preview::Open {
                key: format!("edit-preview:{identity}"),
                label: shown.target.clone(),
                path: target_path.clone(),
                state: tool.state.clone(),
                preview,
            });
        let preview_anchor = edit_preview
            .as_ref()
            .map(|open| self.edit_preview_anchor(&open.key));
        let summary = div()
            .id(gpui::SharedString::from(format!("tool:{identity}")))
            .debug_selector(move || format!("tool-{start}"))
            .role(Role::Group)
            .aria_label(format!("{state_label} tool call: {caption}"))
            .flex()
            .w_full()
            .min_w_0()
            .items_center()
            .gap_1p5()
            .py_1()
            .line_height(px(20.))
            // React's activity rows are always bare: no status glyph, only
            // a failure stays marked. Gespan's own tools carry its logo.
            .when(visual, |d| d.child(crate::activity::glyph("gespan", 14.)))
            .when(shown.action != "Run", |d| {
                d.child(
                    div()
                        .flex_shrink_0()
                        .text_color(if failed {
                            rgb(0xf87171).into()
                        } else {
                            content(0.5)
                        })
                        .child(action.to_owned()),
                )
            })
            .child(
                div().min_w_0().flex_1().flex().child(
                    div()
                        .id(gpui::SharedString::from(format!("tool-target:{identity}")))
                        .debug_selector(move || format!("tool-target-{start}"))
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap_1()
                        // A chain's description is prose; only its chips are
                        // code. A chart's title is prose too.
                        .when(!chain && !visual, |d| {
                            d.font_family(crate::fonts::monospace()).text_size(px(12.))
                        })
                        .text_color(if failed {
                            rgb(0xff6467).into()
                        } else {
                            content(0.7)
                        })
                        .when(is_search, |d| {
                            let patterns = shown.search_patterns();
                            let hidden = patterns.len().saturating_sub(2);
                            d.children(patterns.iter().take(2).enumerate().map(
                                |(index, pattern)| {
                                    search_pill()
                                        .debug_selector(move || {
                                            format!("tool-search-pill-{start}-{index}")
                                        })
                                        .child(
                                            div().min_w_0().truncate().child((*pattern).to_owned()),
                                        )
                                },
                            ))
                            .when(hidden > 0, |d| {
                                d.child(
                                    search_pill()
                                        .id(gpui::SharedString::from(format!(
                                            "tool-search-more:{identity}"
                                        )))
                                        .debug_selector(move || format!("tool-search-more-{start}"))
                                        .flex_shrink_0()
                                        .bg(content(0.))
                                        .border_color(content(0.12))
                                        .text_color(if failed {
                                            rgb(0xff6467).into()
                                        } else {
                                            content(0.5)
                                        })
                                        .aria_label(format!(
                                            "{hidden} more patterns: {}",
                                            patterns[2..].join(", ")
                                        ))
                                        .child(format!("+{hidden}")),
                                )
                            })
                        })
                        .when(is_file, |d| {
                            d.rounded(px(4.)).px_1().bg(content(0.06)).child(
                                if shown.action == "List" {
                                    icon("folder", 16.).into_any_element()
                                } else {
                                    crate::activity::file_icon(&shown.target, 16.)
                                },
                            )
                        })
                        .when_some(preview_anchor, |d, anchor| {
                            d.relative().child(anchor.probe())
                        })
                        .when_some(edit_preview, |d, open| {
                            d.on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                                this.hover_edit_chip(open.clone(), *hovered, cx);
                            }))
                        })
                        .when_some(target_path, |d, path| {
                            d.role(Role::Button)
                                .aria_label(format!("Open {path}"))
                                .cursor_pointer()
                                .hover(|style| style.text_color(rgb(0x7dd3fc)))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    cx.stop_propagation();
                                    this.hide_edit_preview(cx);
                                    window.dispatch_action(
                                        Box::new(crate::file_editor::OpenPath(path.clone())),
                                        cx,
                                    );
                                }))
                        })
                        .when(!is_search && !chain, |d| {
                            d.child(div().min_w_0().truncate().child(shown.target))
                        })
                        .when(chain, |d| {
                            let hidden = chips.len().saturating_sub(3);
                            let rest: Vec<_> = chips
                                .iter()
                                .skip(3)
                                .map(|(_, label, _)| label.clone())
                                .collect();
                            d.gap_1p5()
                                .when_some(shown.description.clone(), |d, description| {
                                    d.child(
                                        div()
                                            .min_w(px(64.))
                                            .flex_shrink(1.)
                                            .truncate()
                                            .child(description),
                                    )
                                })
                                .children(chips.into_iter().take(3).enumerate().map(
                                    |(index, (action, label, path))| {
                                        search_pill()
                                            .id(gpui::SharedString::from(format!(
                                                "tool-step-chip:{identity}:{index}"
                                            )))
                                            .debug_selector(move || {
                                                format!("tool-step-chip-{start}-{index}")
                                            })
                                            .max_w(px(200.))
                                            .flex_shrink(1.)
                                            .font_family(crate::fonts::monospace())
                                            .text_size(px(12.))
                                            .gap_1()
                                            .px_1p5()
                                            .when(matches!(action, "Read" | "List"), |d| {
                                                d.child(if action == "List" {
                                                    icon("folder", 14.).into_any_element()
                                                } else {
                                                    crate::activity::file_icon(&label, 14.)
                                                })
                                            })
                                            .when(action == "Find", |d| {
                                                d.child(
                                                    icon("search", 12.)
                                                        .text_color(content(0.45))
                                                        .flex_shrink_0(),
                                                )
                                            })
                                            .child(div().min_w_0().truncate().child(label.clone()))
                                            .when_some(path, |d, path| {
                                                d.role(Role::Button)
                                                    .aria_label(format!("Open {path}"))
                                                    .cursor_pointer()
                                                    .hover(|style| style.text_color(rgb(0x7dd3fc)))
                                                    .on_click(move |_, window, cx| {
                                                        cx.stop_propagation();
                                                        window.dispatch_action(
                                                            Box::new(crate::file_editor::OpenPath(
                                                                path.clone(),
                                                            )),
                                                            cx,
                                                        );
                                                    })
                                            })
                                    },
                                ))
                                .when(hidden > 0, |d| {
                                    d.child(
                                        search_pill()
                                            .id(gpui::SharedString::from(format!(
                                                "tool-step-more:{identity}"
                                            )))
                                            .debug_selector(move || {
                                                format!("tool-step-more-{start}")
                                            })
                                            .flex_shrink_0()
                                            .font_family(crate::fonts::monospace())
                                            .text_size(px(12.))
                                            .bg(content(0.))
                                            .border_color(content(0.12))
                                            .text_color(if failed {
                                                rgb(0xff6467).into()
                                            } else {
                                                content(0.5)
                                            })
                                            .aria_label(format!(
                                                "{hidden} more steps: {}",
                                                rest.join(", ")
                                            ))
                                            .child(format!("+{hidden}")),
                                    )
                                })
                        }),
                ),
            )
            .when(failed, |d| {
                d.child(icon("close", 14.).text_color(rgb(0xff6467)).flex_shrink_0())
            })
            .when_some(tool.elapsed_label(), |d, label| {
                d.child(
                    div()
                        .debug_selector(move || format!("tool-elapsed-{start}"))
                        .flex_shrink_0()
                        .text_size(px(12.))
                        .text_color(content(0.4))
                        .child(label),
                )
            })
            .when(has_detail, |d| {
                let toggle_key = error_key.clone();
                d.cursor_pointer()
                    .child(
                        div()
                            .id("error-toggle")
                            .role(Role::Button)
                            .aria_expanded(error_open)
                            .aria_label(format!(
                                "{} {} for {caption}",
                                if error_open { "Hide" } else { "Show" },
                                if has_error { "error details" } else { "steps" }
                            ))
                            .p_1()
                            .child(
                                icon(
                                    if error_open {
                                        "chevron-down"
                                    } else {
                                        "chevron-right"
                                    },
                                    14.,
                                )
                                .text_color(if has_error {
                                    rgb(0xf87171).into()
                                } else {
                                    content(0.4)
                                }),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.toggle_agent_part(toggle_key.clone(), !error_open, start, cx);
                    }))
            });
        div()
            .w_full()
            .min_w_0()
            .flex()
            .flex_col()
            .child(summary)
            .when(chain && error_open, |d| {
                d.child(
                    div()
                        .debug_selector(move || format!("tool-steps-{start}"))
                        .min_w_0()
                        .pb_2()
                        .flex()
                        .flex_col()
                        .font_family(crate::fonts::monospace())
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .children(steps.into_iter().enumerate().map(
                            |(index, (op, command, path))| {
                                div()
                                    .id(gpui::SharedString::from(format!(
                                        "tool-step:{identity}:{index}"
                                    )))
                                    .flex()
                                    .min_w_0()
                                    .gap_2()
                                    .child(
                                        div()
                                            .w(px(16.))
                                            .flex_shrink_0()
                                            .flex()
                                            .justify_end()
                                            .text_color(content(0.3))
                                            .child(op.unwrap_or("")),
                                    )
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .text_color(content(0.6))
                                            .child(command),
                                    )
                                    .when_some(path, |d, path| {
                                        d.role(Role::Button)
                                            .aria_label(format!("Open {path}"))
                                            .cursor_pointer()
                                            .hover(|style| style.text_color(rgb(0x7dd3fc)))
                                            .on_click(move |_, window, cx| {
                                                cx.stop_propagation();
                                                window.dispatch_action(
                                                    Box::new(crate::file_editor::OpenPath(
                                                        path.clone(),
                                                    )),
                                                    cx,
                                                );
                                            })
                                    })
                            },
                        )),
                )
            })
            .when(has_error && error_open, |d| {
                d.child(
                    div()
                        .min_w_0()
                        .pb_2()
                        .font_family(crate::fonts::monospace())
                        .text_size(px(12.))
                        .line_height(px(20.))
                        .text_color(rgb(0xf87171))
                        .child(error_detail),
                )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcript::Body;
    use gespan_core::transcript::Row;
    use gpui::{TestAppContext, point};

    fn activity_rows(steps: usize) -> Vec<Row> {
        (1..=steps)
            .map(|n| {
                let mut call = ToolCall::new(&format!("call-{n}"), "Bash");
                call.input = format!(r#"{{"command":"echo step {n}"}}"#);
                call.state = ToolState::Completed;
                Row {
                    role: "tool".into(),
                    text: serde_json::to_string(&call).unwrap(),
                    first: true,
                    sent_at: None,
                }
            })
            .collect()
    }

    fn shown_tools(view: &TranscriptView) -> usize {
        view.rows
            .iter()
            .filter(|row| matches!(row.body, Body::Tool { .. }))
            .count()
    }

    #[gpui::test]
    fn activity_list_opens_its_commands_and_keeps_them_open(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, cx| TranscriptView::activity(activity_rows(2), cx));
        window
            .update(cx, |view, _, cx| {
                assert_eq!(shown_tools(view), 2);
                view.patch(0, activity_rows(3), cx);
                assert_eq!(shown_tools(view), 3);
            })
            .unwrap();
    }

    #[gpui::test]
    fn activity_list_steps_in_new_commands_only(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, cx| TranscriptView::activity(activity_rows(2), cx));
        window
            .update(cx, |view, _, cx| {
                assert!(view.motions.is_empty(), "opening the list shows it at once");
                view.patch(0, activity_rows(3), cx);
                let stepped: Vec<_> = view
                    .rows
                    .iter()
                    .filter(|row| view.motions.get(&row.key).is_some_and(|m| m.step))
                    .map(|row| row.key.0)
                    .collect();
                assert_eq!(stepped, [2]);
            })
            .unwrap();
    }

    #[gpui::test]
    fn activity_list_stays_closed_once_the_user_closes_it(cx: &mut TestAppContext) {
        let window = cx.add_window(|_, cx| TranscriptView::activity(activity_rows(2), cx));
        window
            .update(cx, |view, _, cx| {
                view.toggle_phase(0, false, cx);
                view.patch(0, activity_rows(3), cx);
                assert!(!view.phase_open(0, false));
            })
            .unwrap();
    }

    #[gpui::test]
    fn search_pills_and_overflow_count_share_a_single_compact_row(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|_| TranscriptView::new())
            })
            .unwrap()
        });
        window.update(cx, |view, _, cx| {
            let mut call = ToolCall::new("search", "Bash");
            call.input = r#"{"command":"rg -n 'model_settings_menu|render_model_settings|self.models|models.clone' src"}"#.into();
            call.state = ToolState::Completed;
            view.patch(0, vec![
                Row { role: "user".into(), text: "Find model settings".into(), first: true, sent_at: None },
                Row { role: "tool".into(), text: serde_json::to_string(&call).unwrap(), first: true, sent_at: None },
            ], cx);
            view.work_folds.insert(1, true);
            view.phases.insert(1, true);
            view.rebuild(0, cx);
        }).unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        for width in [900., 320.] {
            visual.simulate_resize(gpui::size(px(width), px(600.)));
            visual.run_until_parked();
            let row = visual.debug_bounds("tool-1").expect("search row");
            let first = visual
                .debug_bounds("tool-search-pill-1-0")
                .expect("first pattern");
            let second = visual
                .debug_bounds("tool-search-pill-1-1")
                .expect("second pattern");
            let more = visual
                .debug_bounds("tool-search-more-1")
                .expect("overflow count");
            assert_eq!(first.size.height, px(24.));
            assert_eq!(second.size.height, first.size.height);
            assert_eq!(more.size.height, first.size.height);
            assert_eq!(more.center().y, first.center().y);
            assert!(more.right() <= row.right());
            assert!(visual.debug_bounds("tool-search-pill-1-2").is_none());
            assert!(visual.debug_bounds("tool-input-toggle-1").is_none());
            visual.simulate_click(more.center(), Default::default());
            visual.run_until_parked();
            assert_eq!(
                visual.debug_bounds("tool-1").unwrap().size.height,
                row.size.height
            );
        }
    }

    #[gpui::test]
    fn chained_commands_show_step_chips_and_expand_into_steps(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|_| TranscriptView::new())
            })
            .unwrap()
        });
        window.update(cx, |view, _, cx| {
            let mut call = ToolCall::new("chain", "Bash");
            call.input = serde_json::json!({
                "command": "cd /repo && git status --short && cat package.json && sed -n '1,260p' scripts/build-native.mjs && sed -n '1,260p' scripts/dev-native.mjs",
                "description": "Inspect the native build scripts",
            })
            .to_string();
            call.state = ToolState::Completed;
            view.patch(0, vec![
                Row { role: "user".into(), text: "Check the build".into(), first: true, sent_at: None },
                Row { role: "tool".into(), text: serde_json::to_string(&call).unwrap(), first: true, sent_at: None },
            ], cx);
            view.work_folds.insert(1, true);
            view.phases.insert(1, true);
            view.rebuild(0, cx);
        }).unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        for width in [900., 320.] {
            visual.simulate_resize(gpui::size(px(width), px(600.)));
            visual.run_until_parked();
            let row = visual.debug_bounds("tool-1").expect("chain row");
            // `cd` is plumbing: git status, package.json, build-native.mjs, then +1.
            let first = visual
                .debug_bounds("tool-step-chip-1-0")
                .expect("first chip");
            let more = visual.debug_bounds("tool-step-more-1").expect("overflow");
            assert!(visual.debug_bounds("tool-step-chip-1-2").is_some());
            assert!(visual.debug_bounds("tool-step-chip-1-3").is_none());
            assert_eq!(more.center().y, first.center().y);
            assert!(more.right() <= row.right());
            assert!(row.size.height <= px(32.));
            assert!(visual.debug_bounds("tool-steps-1").is_none());
        }
        let row = visual.debug_bounds("tool-1").unwrap();
        visual.simulate_click(
            point(row.right() - px(8.), row.center().y),
            Default::default(),
        );
        visual.run_until_parked();
        let steps = visual.debug_bounds("tool-steps-1").expect("expanded steps");
        assert!(steps.size.height >= px(100.));
    }

    #[gpui::test]
    fn grouped_tool_rows_stay_compact_when_clicked(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|_| TranscriptView::new())
            })
            .unwrap()
        });
        window
            .update(cx, |view, _, cx| {
                let row = |role: &str, text: String| Row {
                    role: role.into(),
                    text,
                    first: true,
                    sent_at: None,
                };
                let mut call = ToolCall::new("read", "commandExecution");
                call.input = r#"{"command":"/bin/zsh -lc 'cat src/lib.rs'","cwd":"/repo"}"#.into();
                call.output = "pub fn main() {}".into();
                call.state = ToolState::Completed;
                let mut failed = ToolCall::new("run", "Bash");
                failed.input = r#"{"command":"npm run build"}"#.into();
                failed.output = "Build failed".into();
                failed.state = ToolState::Failed;
                view.patch(
                    0,
                    vec![
                        row("user", "Inspect".into()),
                        row("tool", serde_json::to_string(&call).unwrap()),
                        row("tool", serde_json::to_string(&failed).unwrap()),
                        row("assistant", "Done".into()),
                    ],
                    cx,
                );
                view.work_folds.insert(1, true);
                view.phases.insert(1, true);
                view.rebuild(0, cx);
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let caption = visual
            .debug_bounds("tool-1")
            .expect("the grouped tool is visible");
        // Clicking the caption does not open raw command/output details.
        visual.simulate_click(
            point(caption.right() - px(4.), caption.center().y),
            Default::default(),
        );
        visual.run_until_parked();
        window
            .update(visual, |view, _, _| {
                assert!(!view.expanded.contains(&1));
                let calls: Vec<_> = view
                    .rows
                    .iter()
                    .filter_map(|row| match &row.body {
                        Body::Tool { call, .. } => Some(call),
                        _ => None,
                    })
                    .collect();
                assert_eq!(calls.len(), 2);
                assert_eq!(calls[0].presentation().action, "Read");
                assert_eq!(calls[1].state, ToolState::Failed);
            })
            .unwrap();
        assert!(visual.debug_bounds("tool-target-1").is_some());
        assert!(visual.debug_bounds("tool-input-toggle-1").is_none());
    }

    #[gpui::test]
    fn reasoning_and_tools_share_the_phase_rail_and_command_count(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|_| TranscriptView::new())
            })
            .unwrap()
        });
        window
            .update(cx, |view, _, cx| {
                let row = |role: &str, text: &str| Row {
                    role: role.into(),
                    text: text.into(),
                    first: true,
                    sent_at: None,
                };
                let tool = |id: &str, name: &str, input: &str| {
                    let mut call = ToolCall::new(id, name);
                    call.input = input.into();
                    call.state = ToolState::Completed;
                    row("tool", &serde_json::to_string(&call).unwrap())
                };
                view.patch(
                    0,
                    vec![
                        row("user", "Check"),
                        row("assistant", "I'll inspect the connector."),
                        tool("run1", "Bash", r#"{"command":"pwd"}"#),
                        tool("read", "Read", r#"{"file_path":"src/transcript.rs"}"#),
                        row("reasoning", " \n"),
                        tool("find", "Grep", r#"{"pattern":"fn tool"}"#),
                        row(
                            "reasoning",
                            "**The connector is shared.**\n\nCheck its callers.",
                        ),
                        tool("run2", "Bash", r#"{"command":"cargo test"}"#),
                        row("assistant", "Done"),
                    ],
                    cx,
                );
                view.work_folds.insert(1, true);
                view.phases.insert(2, true);
                view.expanded.insert(6);
                view.rebuild(0, cx);
                let phases: Vec<_> = view
                    .rows
                    .iter()
                    .filter_map(|row| match &row.body {
                        Body::Phase { title, group, .. } => Some((title.as_str(), *group)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(phases, [("Ran 2 commands · Explored the project", 2)]);
                assert!(!view.rows.iter().any(|row| row.key.0 == 4));
                let thought: Vec<_> = view.rows.iter().filter(|row| row.key.0 == 6).collect();
                assert!(
                    thought
                        .iter()
                        .any(|row| matches!(row.body, Body::Reasoning))
                );
                assert!(
                    thought
                        .iter()
                        .any(|row| matches!(row.body, Body::Markdown(_)))
                );
                assert!(thought.iter().all(|row| row.phase_group() == Some(2)));
                assert_eq!(
                    view.rows
                        .iter()
                        .filter(|row| matches!(row.body, Body::Tool { .. }))
                        .count(),
                    4
                );
            })
            .unwrap();
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("thinking-4").is_none());
        assert!(visual.debug_bounds("thinking-6").is_some());
        assert!(visual.debug_bounds("tool-7").is_some());
    }

    #[gpui::test]
    fn first_assistant_prose_replaces_all_streamed_initial_thoughts(cx: &mut TestAppContext) {
        cx.update(|cx| cx.set_reduce_motion(true));
        let view = cx.new(|_| TranscriptView::new());
        view.update(cx, |view, cx| {
            let row = |role: &str, text: &str| Row {
                role: role.into(),
                text: text.into(),
                first: true,
                sent_at: None,
            };
            view.streaming(true, cx);
            view.patch(
                0,
                vec![
                    row("user", "Check"),
                    row("reasoning", "Plan one"),
                    row("reasoning", "Plan two"),
                ],
                cx,
            );
            view.phases.insert(1, true);
            view.rebuild(0, cx);
            assert_eq!(
                view.rows
                    .iter()
                    .filter(|r| matches!(r.body, Body::Reasoning))
                    .count(),
                2
            );
            view.patch(3, vec![row("assistant", "I'll inspect the connector.")], cx);
            assert!(
                !view
                    .rows
                    .iter()
                    .any(|r| matches!(r.body, Body::Reasoning | Body::Phase { .. }))
            );
        });
    }

    #[gpui::test]
    fn a_settled_lone_step_keeps_its_loader_until_the_next_one_arrives(cx: &mut TestAppContext) {
        let view = cx.new(|_| TranscriptView::new());
        let row = |role: &str, text: String| Row {
            role: role.into(),
            text,
            first: true,
            sent_at: None,
        };
        let step = |id: &str, state: ToolState| {
            let mut call = ToolCall::new(id, "Bash");
            call.input = r#"{"command":"ls"}"#.into();
            call.state = state;
            row("tool", serde_json::to_string(&call).unwrap())
        };
        let lone = |view: &TranscriptView| {
            view.rows.iter().find_map(|r| match &r.body {
                Body::Tool {
                    group: None, live, ..
                } => Some(*live),
                _ => None,
            })
        };
        view.update(cx, |view, cx| {
            // Inspect the handoff state without waiting for paced prose, and
            // keep the work expanded so the settled tool row remains visible.
            view.smooth_reveal = false;
            view.work_folds.insert(1, true);
            view.streaming(true, cx);
            view.patch(
                0,
                vec![
                    row("user", "List files".into()),
                    step("first", ToolState::Completed),
                ],
                cx,
            );
            assert_eq!(lone(view), Some(true), "settled but still the live end");

            view.patch(2, vec![step("second", ToolState::Running)], cx);
            assert!(
                view.rows
                    .iter()
                    .any(|r| matches!(r.body, Body::Phase { .. })),
                "the second step hands the loader to its group header"
            );

            view.patch(1, vec![step("first", ToolState::Completed)], cx);
            view.patch(2, vec![row("assistant", String::new())], cx);
            assert_eq!(lone(view), Some(true), "an empty answer keeps the wait");

            view.patch(2, vec![row("assistant", "Done".into())], cx);
            assert_eq!(lone(view), Some(false), "an answer after it ends the wait");

            cx.set_reduce_motion(true);
            view.work_folds.insert(1, false);
            view.rebuild(1, cx);
            assert_eq!(lone(view), None, "collapsed work hides the settled step");
        });
    }

    fn edit_tools(
        cx: &mut TestAppContext,
        calls: Vec<ToolCall>,
    ) -> gpui::WindowHandle<TranscriptView> {
        cx.update(|cx| cx.set_reduce_motion(true));
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|_| TranscriptView::new())
            })
            .unwrap()
        });
        window
            .update(cx, |view, _, cx| {
                let mut rows = vec![Row {
                    role: "user".into(),
                    text: "Change it".into(),
                    first: true,
                    sent_at: None,
                }];
                rows.extend(calls.iter().map(|call| Row {
                    role: "tool".into(),
                    text: serde_json::to_string(call).unwrap(),
                    first: true,
                    sent_at: None,
                }));
                let count = rows.len();
                view.patch(0, rows, cx);
                for ix in 1..count {
                    view.work_folds.insert(ix, true);
                    view.phases.insert(ix, true);
                }
                view.rebuild(0, cx);
            })
            .unwrap();
        window
    }

    #[gpui::test]
    fn hovering_an_edit_chip_previews_its_proposed_diff_then_closes(cx: &mut TestAppContext) {
        let mut edit = ToolCall::new("edit", "Edit");
        edit.input = serde_json::json!({"file_path":"/repo/src/lib.rs","old_string":"let a = 1;\nlet b = 2;","new_string":"let a = 1;\nlet b = 3;"}).to_string();
        let mut read = ToolCall::new("read", "Read");
        read.input = r#"{"file_path":"/repo/src/main.rs"}"#.into();
        read.state = ToolState::Completed;
        let window = edit_tools(cx, vec![edit, read]);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(gpui::size(px(900.), px(600.)));
        visual.run_until_parked();
        let chip = visual.debug_bounds("tool-target-1").expect("edit chip");
        visual.simulate_mouse_move(chip.center(), None, Default::default());
        visual.run_until_parked();
        let open = |visual: &mut gpui::VisualTestContext| {
            window
                .update(visual, |view, _, _| {
                    view.open_edit_preview()
                        .map(|open| (open.key.clone(), open.description()))
                })
                .unwrap()
        };
        assert_eq!(open(visual), None, "it waits before opening");
        visual
            .executor()
            .advance_clock(super::super::edit_preview::OPEN_DELAY);
        visual.run_until_parked();
        assert_eq!(
            open(visual),
            Some(("edit-preview:1".into(), "Proposed changes"))
        );
        let card = visual
            .debug_bounds("edit-preview")
            .expect("the popover is drawn");
        let lines = visual
            .debug_bounds("edit-preview-lines")
            .expect("its lines");
        assert!(
            card.top() >= chip.bottom(),
            "below the chip: {card:?} {chip:?}"
        );
        // Context, the removed line and the added one.
        assert!(lines.size.height >= px(54.), "{lines:?}");
        // Moving onto the card keeps it open.
        visual.simulate_mouse_move(lines.center(), None, Default::default());
        visual
            .executor()
            .advance_clock(super::super::edit_preview::CLOSE_DELAY * 2);
        visual.run_until_parked();
        assert!(open(visual).is_some(), "the card holds it open");

        // Leaving the chip and the card closes it.
        visual.simulate_mouse_move(gpui::point(px(890.), px(590.)), None, Default::default());
        visual
            .executor()
            .advance_clock(super::super::edit_preview::CLOSE_DELAY);
        visual.run_until_parked();
        assert_eq!(open(visual), None);
        assert!(visual.debug_bounds("edit-preview").is_none());
        // A Read chip has no preview.
        let read = visual.debug_bounds("tool-target-2").expect("read chip");
        visual.simulate_mouse_move(read.center(), None, Default::default());
        visual
            .executor()
            .advance_clock(super::super::edit_preview::OPEN_DELAY * 2);
        visual.run_until_parked();
        assert_eq!(open(visual), None);
        assert!(visual.debug_bounds("edit-preview").is_none());
    }

    #[gpui::test]
    fn a_write_and_a_failed_edit_say_what_their_preview_shows(cx: &mut TestAppContext) {
        let mut write = ToolCall::new("write", "Write");
        write.input = r#"{"file_path":"/repo/notes.md","content":"one\ntwo\n"}"#.into();
        write.state = ToolState::Completed;
        let mut failed = ToolCall::new("edit", "Edit");
        failed.input = r#"{"file_path":"/repo/a.rs","old_string":"a","new_string":"b"}"#.into();
        failed.state = ToolState::Failed;
        let mut unchanged = ToolCall::new("same", "Edit");
        unchanged.input = r#"{"file_path":"/repo/b.rs","old_string":"a","new_string":"a"}"#.into();
        unchanged.state = ToolState::Completed;
        let window = edit_tools(cx, vec![write, failed, unchanged]);
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.simulate_resize(gpui::size(px(900.), px(600.)));
        visual.run_until_parked();
        for (row, expected) in [
            (1, Some("Written content · previous contents unavailable")),
            (2, Some("Attempted changes · tool did not complete")),
            (3, None),
        ] {
            let chip = visual
                .debug_bounds(Box::leak(format!("tool-target-{row}").into_boxed_str()))
                .expect("chip");
            visual.simulate_mouse_move(chip.center(), None, Default::default());
            visual
                .executor()
                .advance_clock(super::super::edit_preview::OPEN_DELAY);
            visual.run_until_parked();
            let shown = window
                .update(visual, |view, _, _| {
                    view.open_edit_preview().map(|open| open.description())
                })
                .unwrap();
            assert_eq!(shown, expected, "row {row}");
            // Away, so the card no longer covers the next row.
            visual.simulate_mouse_move(gpui::point(px(890.), px(590.)), None, Default::default());
            visual
                .executor()
                .advance_clock(super::super::edit_preview::CLOSE_DELAY);
            visual.run_until_parked();
        }
    }
}
