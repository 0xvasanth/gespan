//! Viewport-owned Mermaid previews: no browser, subprocess, or permanent asset cache.
use super::*;
use gpui::{ClipboardItem, Context, IntoElement, RenderImage, RenderOnce, Subscription, Task};
use mermaid_rs_renderer::{LayoutConfig, Theme, compute_layout, parse_mermaid_strict, render_svg};
use std::time::Duration;

const PREVIEW_HEIGHT: f32 = 320.;
const MAX_SOURCE_BYTES: usize = 32 * 1024;
const MAX_SOURCE_LINES: usize = 512;
const MAX_GRAPH_ITEMS: usize = 256;
const MAX_IMAGE_SIDE: f32 = 2048.;

#[derive(IntoElement)]
pub(super) struct Diagram {
    pub id: SharedString,
    pub fence: Arc<Fence>,
    pub selection: Option<TextSelection>,
}

#[derive(Default)]
struct Preview {
    key: Option<(SharedString, bool)>,
    result: Option<Result<Arc<RenderImage>, String>>,
    source: bool,
    task: Option<Task<()>>,
    _release: Option<Subscription>,
}

impl Preview {
    fn clear_image(&mut self, cx: &mut App) {
        if let Some(Ok(image)) = self.result.take() {
            cx.drop_image(image, None);
        }
    }

    fn prepare(&mut self, source: SharedString, dark: bool, cx: &mut Context<Self>) {
        if self.key.as_ref() == Some(&(source.clone(), dark)) {
            return;
        }
        self.task = None;
        self.clear_image(cx);
        self.key = Some((source.clone(), dark));
        if self._release.is_none() {
            self._release = Some(cx.on_release(|this, cx| this.clear_image(cx)));
        }
        let renderer = cx.svg_renderer();
        let executor = cx.background_executor().clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            // Dropping this task on the next source edit cancels the debounce.
            // Completed work belongs only to this revision and theme.
            executor.timer(Duration::from_millis(150)).await;
            let result = executor
                .spawn(async move { rasterize(&source, dark, &renderer) })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.result = Some(result);
                this.task = None;
                cx.notify();
            });
        }));
    }
}

fn diagram_svg(source: &str, dark: bool) -> Result<(String, f32, f32), String> {
    if source.len() > MAX_SOURCE_BYTES || source.lines().count() > MAX_SOURCE_LINES {
        return Err("Diagram is too large to preview. You can still copy its source.".into());
    }
    let parsed = parse_mermaid_strict(source).map_err(|error| error.to_string())?;
    if parsed.graph.nodes.len() + parsed.graph.edges.len() > MAX_GRAPH_ITEMS {
        return Err("Diagram has too many nodes or connections to preview.".into());
    }
    let theme = if dark { Theme::dark() } else { Theme::modern() };
    let config = LayoutConfig::default();
    let layout = compute_layout(&parsed.graph, &theme, &config);
    if let mermaid_rs_renderer::layout::DiagramData::Error(error) = &layout.diagram {
        return Err(error.message.clone());
    }
    let dimensions = mermaid_rs_renderer::measure_svg_dimensions(&layout, &config, None);
    let width = dimensions.width;
    let height = dimensions.height;
    if !width.is_finite() || !height.is_finite() || width <= 0. || height <= 0. {
        return Err("Diagram has invalid dimensions.".into());
    }
    Ok((render_svg(&layout, &theme, &config), width, height))
}

fn rasterize(
    source: &str,
    dark: bool,
    renderer: &gpui::SvgRenderer,
) -> Result<Arc<RenderImage>, String> {
    let (svg, width, height) = diagram_svg(source, dark)?;
    let scale = (MAX_IMAGE_SIDE / width.max(height)).min(2.);
    let svg = renderer
        .parse_svg(svg.as_bytes())
        .map_err(|error| error.to_string())?;
    renderer
        .render_parsed(
            &svg,
            gpui::SvgSize::ExactSize(gpui::size(
                gpui::DevicePixels((width * scale).ceil().clamp(1., MAX_IMAGE_SIDE) as i32),
                gpui::DevicePixels((height * scale).ceil().clamp(1., MAX_IMAGE_SIDE) as i32),
            )),
        )
        .map_err(|error| error.to_string())
}

fn source_preview(fence: &Arc<Fence>) -> Arc<Fence> {
    let mut end = fence.code.len().min(MAX_SOURCE_BYTES);
    if let Some(line) = fence.lines.get(MAX_SOURCE_LINES) {
        end = end.min(line.start);
    }
    while !fence.code.is_char_boundary(end) {
        end -= 1;
    }
    if end == fence.code.len() {
        fence.clone()
    } else {
        Arc::new(Fence::new("mermaid", &fence.code[..end]))
    }
}

impl RenderOnce for Diagram {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            SharedString::from(format!("{}-mermaid-state", self.id)),
            cx,
            |_, _| Preview::default(),
        );
        state.update(cx, |state, cx| {
            state.prepare(self.fence.code.clone(), !is_light(), cx)
        });
        let preview = state.read(cx);
        let has_diagram = matches!(preview.result, Some(Ok(_)));
        let show_source = preview.source || !has_diagram;
        let mut status = match &preview.result {
            None => Some("Rendering diagram…".to_owned()),
            Some(Err(error)) => Some(format!("Unable to preview Mermaid: {error}")),
            Some(Ok(_)) => None,
        };
        let media = if show_source {
            let fence = source_preview(&self.fence);
            if !Arc::ptr_eq(&fence, &self.fence) {
                status =
                    Some("Source preview is truncated. Copy includes the complete source.".into());
            }
            div()
                .id("mermaid-source")
                .debug_selector(|| "mermaid-source".into())
                .size_full()
                .overflow_scroll()
                .py_2()
                .child(code_lines_selected(
                    &fence,
                    0..fence.lines.len(),
                    None,
                    "",
                    false,
                    self.selection,
                ))
                .into_any_element()
        } else if let Some(Ok(image)) = &preview.result {
            gpui::img(image.clone())
                .absolute()
                .inset_0()
                .debug_selector(|| "mermaid-image".into())
                .role(Role::Image)
                .aria_label(format!("Mermaid diagram\n{}", self.fence.code))
                .size_full()
                .object_fit(gpui::ObjectFit::Contain)
                .into_any_element()
        } else {
            div().into_any_element()
        };
        let copied = self.fence.code.clone();
        div()
            .id(self.id)
            .w_full()
            .min_w_0()
            .rounded_lg()
            .overflow_hidden()
            .border_1()
            .border_color(content(0.1))
            .bg(content(0.03))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .h(px(36.))
                    .text_size(px(12.))
                    .text_color(content(0.6))
                    .child(div().flex_1().child("Mermaid"))
                    .when(has_diagram, |d| {
                        d.child(
                            div()
                                .id("mermaid-toggle")
                                .debug_selector(|| "mermaid-toggle".into())
                                .role(Role::Button)
                                .aria_label(if show_source {
                                    "Show diagram"
                                } else {
                                    "Show source"
                                })
                                .cursor_pointer()
                                .hover(|s| s.text_color(content(1.)))
                                .child(if show_source { "Diagram" } else { "Source" })
                                .on_click(move |_, _, cx| {
                                    state.update(cx, |state, cx| {
                                        state.source = !state.source;
                                        cx.notify();
                                    });
                                    cx.stop_propagation();
                                }),
                        )
                    })
                    .child(
                        div()
                            .id("mermaid-copy")
                            .debug_selector(|| "mermaid-copy".into())
                            .role(Role::Button)
                            .aria_label("Copy Mermaid source")
                            .cursor_pointer()
                            .hover(|s| s.text_color(content(1.)))
                            .child("Copy")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    copied.to_string(),
                                ));
                                cx.stop_propagation();
                            }),
                    ),
            )
            // Keep measurements stable as async rendering completes, including
            // in Inbox's retained document layout and the transcript list.
            .child(
                div()
                    .w_full()
                    .h(px(PREVIEW_HEIGHT))
                    .flex()
                    .flex_col()
                    .border_t_1()
                    .border_color(content(0.1))
                    .when_some(status, |d, status| {
                        d.child(
                            div()
                                .px_3()
                                .py_1()
                                .flex_shrink_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(content(0.5))
                                .child(status),
                        )
                    })
                    .child(
                        div()
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .min_w_0()
                            .overflow_hidden()
                            .child(media),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture;
    impl gpui::Render for Fixture {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(px(640.)).child(Diagram {
                id: "diagram-fixture".into(),
                fence: Arc::new(Fence::new("mermaid", "graph TD; A-->B")),
                selection: None,
            })
        }
    }

    #[gpui::test]
    fn preview_buttons_switch_source_and_copy_without_changing_height(
        cx: &mut gpui::TestAppContext,
    ) {
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| cx.new(|_| Fixture))
                .unwrap()
        });
        let visual = &mut gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(visual.debug_bounds("mermaid-source").is_some());
        visual.executor().advance_clock(Duration::from_millis(200));
        visual.run_until_parked();
        let image = visual.debug_bounds("mermaid-image").unwrap();
        assert!(image.size.width > px(0.) && image.size.height > px(0.));
        let button = visual.debug_bounds("mermaid-toggle").unwrap();
        visual.simulate_click(button.center(), Default::default());
        visual.run_until_parked();
        assert_eq!(
            visual.debug_bounds("mermaid-source").unwrap().size.height,
            image.size.height
        );
        assert!(visual.debug_bounds("mermaid-image").is_none());
        let copy = visual.debug_bounds("mermaid-copy").unwrap();
        visual.simulate_click(copy.center(), Default::default());
        visual.run_until_parked();
        assert_eq!(
            visual
                .update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text()))
                .as_deref(),
            Some("graph TD; A-->B")
        );
        let button = visual.debug_bounds("mermaid-toggle").unwrap();
        visual.simulate_click(button.center(), Default::default());
        visual.run_until_parked();
        assert_eq!(visual.debug_bounds("mermaid-image").unwrap(), image);
    }

    #[test]
    fn renders_common_diagrams_in_both_themes() {
        for source in [
            "flowchart LR\n A[Start] --> B{Ready?}\n B -->|Yes| C[Done]",
            "sequenceDiagram\n Alice->>Bob: Hello\n Bob-->>Alice: Hi",
            "classDiagram\n Animal <|-- Duck\n Animal : +int age",
            "stateDiagram-v2\n [*] --> Ready\n Ready --> [*]",
            "erDiagram\n CUSTOMER ||--o{ ORDER : places",
        ] {
            for dark in [false, true] {
                let (svg, width, height) = diagram_svg(source, dark).unwrap();
                assert!(svg.contains("<svg"));
                assert!(width > 0. && height > 0.);
            }
        }
        assert_ne!(
            diagram_svg("graph TD; A-->B", false).unwrap().0,
            diagram_svg("graph TD; A-->B", true).unwrap().0
        );
    }

    #[test]
    fn invalid_and_oversized_diagrams_return_errors() {
        assert!(diagram_svg("not a diagram", false).is_err());
        assert!(diagram_svg(&"x".repeat(MAX_SOURCE_BYTES + 1), false).is_err());
        assert!(diagram_svg(&"\n".repeat(MAX_SOURCE_LINES + 1), false).is_err());
        let large = format!(
            "graph TD\n{}",
            (0..MAX_GRAPH_ITEMS)
                .map(|n| format!(" A{n}-->A{}\n", n + 1))
                .collect::<String>()
        );
        assert!(diagram_svg(&large, false).is_err());
        let original = Arc::new(Fence::new("mermaid", &"é\n".repeat(MAX_SOURCE_BYTES)));
        let display = source_preview(&original);
        assert!(display.code.len() <= MAX_SOURCE_BYTES);
        assert!(display.lines.len() <= MAX_SOURCE_LINES);
        assert!(original.code.starts_with(display.code.as_ref()));
        assert_eq!(original.code.len(), MAX_SOURCE_BYTES * 3);
    }

    #[test]
    fn mermaid_is_one_row_and_keeps_source_and_container_context() {
        let source = format!("graph TD\n{}", " A-->B\n".repeat(PART_LINES * 2));
        let block = Block::BlockQuote {
            children: vec![Block::CodeBlock {
                language: Some("mermaid title=diagram.mmd".into()),
                code: source.clone(),
            }],
        };
        let rows = rows(&block);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].quote, 1);
        assert_eq!(selection_text(&rows[0]), source);
        assert!(matches!(rows[0].part, Part::Mermaid { .. }));
        assert_eq!(
            super::super::rows(&Block::CodeBlock {
                language: Some("rust".into()),
                code: "line\n".repeat(PART_LINES * 2),
            })
            .len(),
            2
        );
    }

    #[test]
    fn streamed_fences_preserve_literal_source_before_and_after_closing() {
        let code = "flowchart TD\n A[\"café **literal**\"] --> B\n";
        let open = format!("```mermaid\n{code}");
        let mut parser = crate::markdown_model::MarkdownParser::new();
        for text in [open.clone(), format!("{open}```\n")] {
            parser.set_text(&text);
            let tree = parser.display_tree();
            let row = rows(&tree.blocks[0].block).remove(0);
            assert!(matches!(row.part, Part::Mermaid { .. }));
            assert_eq!(row_text(&row), code);
        }
    }

    #[gpui::test]
    fn source_updates_cancel_pending_work_and_theme_changes_replace_pixels(
        cx: &mut gpui::TestAppContext,
    ) {
        let preview = cx.new(|_| Preview::default());
        preview.update(cx, |this, cx| {
            this.prepare("not a diagram".into(), false, cx)
        });
        cx.run_until_parked();
        preview.update(cx, |this, cx| {
            this.prepare("graph TD; A-->B".into(), false, cx)
        });
        cx.run_until_parked();
        preview.read_with(cx, |this, _| assert!(this.result.is_none()));
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        let first = preview.read_with(cx, |this, _| {
            this.result.as_ref().unwrap().as_ref().unwrap().clone()
        });
        let size = first.size(0);
        assert!(size.width.0 > 0 && size.height.0 > 0);
        assert!(size.width.0 <= MAX_IMAGE_SIDE as i32 && size.height.0 <= MAX_IMAGE_SIDE as i32);
        preview.update(cx, |this, cx| {
            this.prepare("graph TD; A-->B".into(), false, cx)
        });
        preview.read_with(cx, |this, _| {
            assert!(Arc::ptr_eq(
                this.result.as_ref().unwrap().as_ref().unwrap(),
                &first
            ))
        });
        preview.update(cx, |this, cx| {
            this.prepare("graph TD; A-->B".into(), true, cx)
        });
        preview.read_with(cx, |this, _| assert!(this.result.is_none()));
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        preview.read_with(cx, |this, _| {
            assert!(!Arc::ptr_eq(
                this.result.as_ref().unwrap().as_ref().unwrap(),
                &first
            ))
        });
        preview.update(cx, |this, cx| {
            this.prepare("not a diagram".into(), true, cx)
        });
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        preview.read_with(cx, |this, _| assert!(matches!(this.result, Some(Err(_)))));
    }
}
