//! The readable body of a permission request: what the agent says it is
//! doing, the command or change itself as real text, then any other
//! arguments as key and value rows. Long requests open behind a toggle.
use crate::notifications::approvals::{approval_alerts, mark_hidden};
use crate::theme::*;
use gespan_core::agent::Approval;
use gespan_core::tool::approval_view::{
    ApprovalView, Body, Collapsed, Field, approval_view, collapse,
};
use gespan_core::tool::edit_preview::{EditPreview, LineKind};
use gpui::{AnyElement, App, ClickEvent, Div, Role, Window, div, prelude::*, px};

/// Lines of a command or text a collapsed card shows.
const BODY_LINES: usize = 8;
/// A single enormous line still has to fit a collapsed card.
const LINE_CHARS: usize = 400;
/// Argument rows a collapsed card shows, and lines of each value.
const FIELD_ROWS: usize = 4;
const FIELD_LINES: usize = 3;

/// Everything the card shows for `approval`, with each shown string's hidden
/// characters marked, so a `r\u{200B}m` can't read as `rm`.
pub(super) struct Card {
    view: ApprovalView,
    alerts: Vec<String>,
}

impl Card {
    pub(super) fn new(approval: &Approval, cwd: Option<&str>) -> Self {
        let home = gespan_core::accounts::home();
        let home = home.as_deref().and_then(|home| home.to_str());
        let mut view = approval_view(approval.tool.as_ref(), &approval.detail, cwd, home);
        mark_view(&mut view);
        // Alerts read the raw request: Claude's input, or the text detail.
        let raw = approval
            .tool
            .as_ref()
            .map_or(approval.detail.as_str(), |tool| tool.input.as_str());
        Self {
            view,
            alerts: approval_alerts(raw),
        }
    }

    /// The provider's question, except that an MCP tool is named by its own
    /// name rather than `mcp__server__tool`; the server has its own row.
    pub(super) fn title(&self, approval: &Approval) -> String {
        match &self.view.mcp {
            Some(mcp) => format!("Allow {}?", mcp.tool),
            None => mark_hidden(&approval.title),
        }
    }

    pub(super) fn is_empty(&self) -> bool {
        self.view.summary.is_none()
            && self.view.mcp.is_none()
            && self.view.body == Body::None
            && self.view.cwd.is_none()
            && self.view.fields.is_empty()
            && self.alerts.is_empty()
    }

    /// Lines a collapsed card leaves out, or `None` when it shows everything.
    pub(super) fn hidden_lines(&self) -> Option<usize> {
        let body = match &self.view.body {
            Body::Command(text) | Body::Text(text) => collapse(text, BODY_LINES, LINE_CHARS),
            _ => None,
        }
        .map_or(0, |cut| cut.hidden_lines);
        let all_lines = |field: &Field| field.value.lines().count().max(1);
        let folded: usize = self
            .view
            .fields
            .iter()
            .filter(|field| field.folded)
            .map(all_lines)
            .sum();
        let fields: usize = self
            .view
            .fields
            .iter()
            .filter(|field| !field.folded)
            .enumerate()
            .map(|(index, field)| {
                if index >= FIELD_ROWS {
                    all_lines(field)
                } else {
                    collapse(&field.value, FIELD_LINES, LINE_CHARS).map_or(0, |c| c.hidden_lines)
                }
            })
            .sum();
        // A folder cut in the middle shows whole once the card opens.
        let folder = usize::from(self.view.cwd_full.is_some());
        let hidden = body + folded + fields + folder;
        (hidden > 0).then_some(hidden)
    }

    pub(super) fn render(&self, expanded: bool) -> AnyElement {
        let view = &self.view;
        let hidden = self.hidden_lines();
        let collapsed = !expanded && hidden.is_some();
        let fields: Vec<&Field> = if collapsed {
            view.fields
                .iter()
                .filter(|field| !field.folded)
                .take(FIELD_ROWS)
                .collect()
        } else {
            view.fields.iter().collect()
        };
        div()
            .debug_selector(|| "approval-detail".into())
            .border_t_1()
            .border_color(content(0.05))
            .pt(px(8.))
            .pb(px(6.))
            .flex()
            .flex_col()
            .gap(px(6.))
            .children(self.alerts.iter().map(|alert| alert_row(alert)))
            .when_some(view.summary.as_ref(), |d, summary| {
                d.child(
                    div()
                        .debug_selector(|| "approval-summary".into())
                        .text_size(px(13.))
                        .line_height(px(19.))
                        .text_color(content(0.88))
                        .child(summary.clone()),
                )
            })
            .when_some(view.mcp.as_ref(), |d, mcp| {
                d.child(
                    div()
                        .debug_selector(|| "approval-mcp".into())
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .child(icon("zap", 12.).text_color(content(0.45)))
                        .child(div().text_color(content(0.5)).child("MCP server"))
                        .child(
                            div()
                                .font_family(crate::fonts::monospace())
                                .text_color(content(0.88))
                                .child(mcp.server.clone()),
                        ),
                )
            })
            .child(self.body(collapsed))
            .when_some(
                if collapsed {
                    view.cwd.as_ref()
                } else {
                    view.cwd_full.as_ref().or(view.cwd.as_ref())
                },
                |d, cwd| {
                    d.child(
                        div()
                            .debug_selector(|| "approval-cwd".into())
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(11.5))
                            .text_color(content(0.5))
                            .child(icon("folder", 12.).text_color(content(0.4)))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .font_family(crate::fonts::monospace())
                                    .child(cwd.clone()),
                            ),
                    )
                },
            )
            .when(!fields.is_empty(), |d| {
                d.child(
                    div()
                        .debug_selector(|| "approval-fields".into())
                        .flex()
                        .flex_col()
                        .gap(px(3.))
                        .children(fields.into_iter().map(|field| field_row(field, collapsed))),
                )
            })
            .into_any_element()
    }

    /// Opens or closes the long content. It sits below the scrolling part,
    /// so an open card can always be closed again.
    pub(super) fn toggle(
        &self,
        expanded: bool,
        on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Option<AnyElement> {
        let hidden = self.hidden_lines()?;
        let label = if expanded {
            "Show less".to_owned()
        } else if hidden == 1 {
            "Show 1 more line".to_owned()
        } else {
            format!("Show {hidden} more lines")
        };
        Some(
            div()
                .flex()
                .pb(px(6.))
                .child(
                    div()
                        .id("approval-expand")
                        .debug_selector(|| "approval-expand".into())
                        .role(Role::Button)
                        .aria_label(label.clone())
                        .cursor_pointer()
                        .flex()
                        .items_center()
                        .gap(px(4.))
                        .h(px(22.))
                        .px(px(6.))
                        .ml(px(-6.))
                        .rounded_md()
                        .text_size(px(11.5))
                        .text_color(content(0.55))
                        .hover(|s| s.bg(content(0.06)).text_color(content(0.9)))
                        .child(label)
                        .child(icon(
                            if expanded {
                                "chevron-up"
                            } else {
                                "chevron-down"
                            },
                            12.,
                        ))
                        .on_click(on_toggle),
                )
                .into_any_element(),
        )
    }

    fn body(&self, collapsed: bool) -> AnyElement {
        match &self.view.body {
            Body::None => div().into_any_element(),
            Body::Command(command) => code_block(command, collapsed)
                .debug_selector(|| "approval-command".into())
                .into_any_element(),
            Body::Text(text) => div()
                .debug_selector(|| "approval-text".into())
                .text_size(px(12.5))
                .line_height(px(18.))
                .text_color(content(0.75))
                .child(shown_text(text, BODY_LINES, collapsed))
                .into_any_element(),
            Body::Target { verb, target } => labelled_row(verb, target, "search")
                .debug_selector(|| "approval-target".into())
                .into_any_element(),
            Body::Url(url) => labelled_row("Fetch", url, "globe")
                .debug_selector(|| "approval-url".into())
                .into_any_element(),
            Body::Change { path, preview } => change(path, preview.as_ref()),
        }
    }
}

fn mark_view(view: &mut ApprovalView) {
    let mark = |text: &mut String| *text = mark_hidden(text);
    view.summary.as_mut().map(mark);
    view.cwd.as_mut().map(mark);
    view.cwd_full.as_mut().map(mark);
    if let Some(mcp) = view.mcp.as_mut() {
        mark(&mut mcp.server);
        mark(&mut mcp.tool);
    }
    match &mut view.body {
        Body::Command(text) | Body::Text(text) | Body::Url(text) => mark(text),
        Body::Target { target, .. } => mark(target),
        Body::Change { path, preview } => {
            mark(path);
            for line in preview.iter_mut().flat_map(|p| p.lines.iter_mut()) {
                mark(&mut line.text);
            }
        }
        Body::None => {}
    }
    for field in &mut view.fields {
        mark(&mut field.key);
        mark(&mut field.value);
    }
}

fn shown_text(text: &str, lines: usize, collapsed: bool) -> String {
    match collapse(text, lines, LINE_CHARS).filter(|_| collapsed) {
        Some(Collapsed { text, .. }) => text,
        None => text.to_owned(),
    }
}

fn code_block(text: &str, collapsed: bool) -> Div {
    div()
        .rounded_md()
        .bg(content(0.045))
        .border_1()
        .border_color(content(0.05))
        .px(px(10.))
        .py(px(7.))
        .font_family(crate::fonts::monospace())
        .text_size(px(12.))
        .line_height(px(18.))
        .text_color(content(0.88))
        .child(shown_text(text, BODY_LINES, collapsed))
}

fn alert_row(alert: &str) -> Div {
    let amber = gpui::rgb(0xfbbf24);
    div()
        .debug_selector(|| "approval-alert".into())
        .flex()
        .items_center()
        .gap(px(6.))
        .px(px(8.))
        .py(px(4.))
        .rounded_md()
        .bg(gpui::Hsla::from(amber).opacity(0.1))
        .text_size(px(12.))
        .text_color(amber)
        .child(icon("shield-alert", 13.).text_color(amber))
        .child(alert.to_owned())
}

fn labelled_row(label: &str, value: &str, glyph: &'static str) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(8.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(5.))
                .h(px(18.))
                .flex_shrink_0()
                .text_color(content(0.5))
                .child(icon(glyph, 12.).text_color(content(0.4)))
                .child(label.to_owned()),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .font_family(crate::fonts::monospace())
                .text_size(px(12.))
                .line_height(px(18.))
                .text_color(content(0.88))
                .child(value.to_owned()),
        )
}

fn field_row(field: &Field, collapsed: bool) -> Div {
    div()
        .flex()
        .items_start()
        .gap(px(10.))
        .text_size(px(12.))
        .line_height(px(18.))
        .child(
            div()
                .w(px(112.))
                .flex_shrink_0()
                .text_color(content(0.5))
                .child(field.key.clone()),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .font_family(crate::fonts::monospace())
                .text_color(content(0.82))
                .child(shown_text(&field.value, FIELD_LINES, collapsed)),
        )
}

fn change(path: &str, preview: Option<&EditPreview>) -> AnyElement {
    let counts = preview.filter(|p| p.additions + p.deletions > 0).map(|p| {
        div()
            .flex_shrink_0()
            .flex()
            .gap(px(4.))
            .text_size(px(11.))
            .font_weight(gpui::FontWeight::SEMIBOLD)
            .when(p.additions > 0, |d| {
                d.child(
                    div()
                        .text_color(diff_foreground(true))
                        .child(format!("+{}", p.additions)),
                )
            })
            .when(p.deletions > 0, |d| {
                d.child(
                    div()
                        .text_color(diff_foreground(false))
                        .child(format!("−{}", p.deletions)),
                )
            })
    });
    let file = div()
        .debug_selector(|| "approval-file".into())
        .flex()
        .items_center()
        .gap(px(6.))
        .child(crate::activity::file_icon(path, 14.))
        .child(
            div()
                .min_w_0()
                .flex_1()
                .font_family(crate::fonts::monospace())
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::MEDIUM)
                .text_color(content(0.88))
                .child(path.to_owned()),
        )
        .children(counts);
    let lines = preview.filter(|p| !p.lines.is_empty()).map(|preview| {
        div()
            .debug_selector(|| "approval-diff".into())
            .rounded_md()
            .overflow_hidden()
            .border_1()
            .border_color(content(0.05))
            .bg(content(0.03))
            .py(px(3.))
            .font_family(crate::fonts::monospace())
            .text_size(px(12.))
            .line_height(px(18.))
            .children(preview.lines.iter().map(|line| {
                let (bg, mark, ink) = match line.kind {
                    LineKind::Add => (Some(diff_background(true, false)), "+", content(0.88)),
                    LineKind::Del => (Some(diff_background(false, false)), "−", content(0.7)),
                    LineKind::Context => (None, " ", content(0.6)),
                };
                div()
                    .flex()
                    .when_some(bg, |d, bg| d.bg(bg))
                    .child(
                        div()
                            .w(px(22.))
                            .flex_shrink_0()
                            .text_center()
                            .text_color(match line.kind {
                                LineKind::Add => diff_foreground(true),
                                LineKind::Del => diff_foreground(false),
                                LineKind::Context => content(0.3),
                            })
                            .child(mark),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .pr(px(8.))
                            .truncate()
                            .text_color(ink)
                            .child(if line.text.is_empty() {
                                " ".to_owned()
                            } else {
                                line.text.clone()
                            }),
                    )
            }))
    });
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(file)
        .children(lines)
        .into_any_element()
}
