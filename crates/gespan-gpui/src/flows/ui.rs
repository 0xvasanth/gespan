//! Small pieces the Flows pages share: status pills, buttons, tooltips.
use crate::theme::*;
use gespan_core::flow::RunStatus;
use gpui::{Context, Hsla, Role, SharedString, Window, div, prelude::*, px, rgb};

pub(super) fn run_status_label(status: RunStatus) -> &'static str {
    match status {
        RunStatus::Running => "Running",
        RunStatus::WaitingApproval => "Needs approval",
        RunStatus::Succeeded => "Succeeded",
        RunStatus::Failed => "Failed",
        RunStatus::Cancelled => "Cancelled",
    }
}

pub(crate) fn status_color(status: &str) -> Hsla {
    match status {
        "running" => accent_color(),
        "waiting-approval" => rgb(0xf59e0b).into(),
        "succeeded" | "completed" => rgb(0x10b981).into(),
        "failed" => rgb(0xf87171).into(),
        _ => content(0.45),
    }
}

/// A small status pill in the status colour.
pub(super) fn status_chip(status: &str, label: &'static str) -> gpui::Div {
    let color = status_color(status);
    let mut wash = color;
    wash.a = 0.12;
    div()
        .flex_shrink_0()
        .px(px(6.))
        .h(px(18.))
        .rounded(px(9.))
        .flex()
        .items_center()
        .bg(wash)
        .text_size(px(10.))
        .font_weight(gpui::FontWeight::MEDIUM)
        .text_color(color)
        .child(label)
}

/// The Automations view's outlined button.
pub(super) fn button(
    id: impl Into<gpui::ElementId>,
    label: impl Into<SharedString>,
) -> gpui::Stateful<gpui::Div> {
    let label: SharedString = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .px(px(10.))
        .h(px(26.))
        .rounded(px(6.))
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(content(0.8))
        .border_1()
        .border_color(content(0.15))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.1)).border_color(content(0.3)))
        .child(label)
}

/// Dims a button that cannot act, keeping its size.
pub(super) fn disabled(button: gpui::Stateful<gpui::Div>, off: bool) -> gpui::Stateful<gpui::Div> {
    if off {
        button.opacity(0.4).cursor_default()
    } else {
        button
    }
}

pub(super) struct Tooltip(pub SharedString);
impl Render for Tooltip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut background = shell_fill();
        background.a = 1.;
        div()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(background)
            .border_1()
            .border_color(content(0.12))
            .text_size(px(11.))
            .line_height(px(16.))
            .text_color(content(0.85))
            .child(self.0.clone())
    }
}

pub(super) fn error_banner(error: String) -> gpui::Div {
    div()
        .m(px(16.))
        .p(px(10.))
        .rounded(px(8.))
        .bg(gpui::rgba(0xf8717114))
        .text_size(px(12.))
        .text_color(rgb(0xfca5a5))
        .child(error)
}
