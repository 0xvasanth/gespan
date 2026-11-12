use super::*;
use crate::glass;

impl Automations {
    pub(super) fn render_gallery(&self, cx: &mut Context<Self>) -> AnyElement {
        let categories = [
            ("popular", "Popular"),
            ("review", "Code Review"),
            ("security", "Security"),
            ("incidents", "Incidents & Triage"),
            ("research", "Data & Research"),
            ("environment", "Environment"),
        ];
        let mut cards = vec![
            // A dashed glass card: an empty frame to start from.
            template_card(
                "automation-scratch".into(),
                "Start from scratch",
                layout::icon_glyph("plus", 16., content(0.92)),
                "Write your own instructions and choose a trigger.",
                None,
            )
            .debug_selector(|| "automation-scratch".into())
            .border_dashed()
            .border_color(if is_light() {
                content(0.18)
            } else {
                white(0.16)
            })
            .bg(if is_light() { white(0.35) } else { white(0.02) })
            .on_click(cx.listener(|this, _, _, cx| this.begin(None, cx)))
            .into_any_element(),
        ];
        for template in self.templates.iter().filter(|t| {
            if self.category == "popular" {
                t.popular
            } else {
                t.category == self.category
            }
        }) {
            let selected = template.clone();
            let path = match template.icon.as_str() {
                "alert" => "alert-circle",
                "lock" => "lock",
                "file" => "file",
                "check" => "check",
                "pr" => "git-pull-request",
                "inbox" => "inbox",
                "terminal" => "terminal",
                "note" => "note",
                "gauge" => "gauge",
                _ => "search",
            };
            let selector = format!("template-{}", template.id);
            let schedule = (
                template.trigger["kind"]
                    .as_str()
                    .unwrap_or("time")
                    .to_string(),
                template.trigger_label.clone(),
            );
            cards.push(
                template_card(
                    SharedString::from(format!("template-{}", template.id)),
                    template.name.clone(),
                    layout::icon_glyph(path, 16., eyebrow_ink()),
                    template.description.clone(),
                    Some(schedule),
                )
                .debug_selector(move || selector)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if selected.setup.as_deref() == Some("review") {
                        this.open_review_setup(cx);
                    } else {
                        this.begin(Some(selected.clone()), cx);
                    }
                }))
                .into_any_element(),
            );
        }
        // CSS's two-column card grid, 960px of cards at most, set against the list.
        let mut grid = div().flex().flex_col().gap(px(14.));
        let mut iter = cards.into_iter();
        while let Some(left) = iter.next() {
            let right = iter.next();
            grid = grid.child(
                div()
                    .flex()
                    .gap(px(14.))
                    .child(div().flex_1().min_w_0().flex().child(left))
                    .child(div().flex_1().min_w_0().flex().children(right)),
            );
        }
        let tabs = glass::segmented()
            .id("automation-categories")
            .role(Role::TabList)
            .aria_label("Category")
            .children(categories.map(|(id, label)| {
                let selected = self.category == id;
                glass::segment(
                    SharedString::from(format!("automation-category-{id}")),
                    label,
                    selected,
                )
                .debug_selector(move || format!("automation-category-{id}"))
                .role(Role::Tab)
                .aria_label(label)
                .aria_selected(selected)
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.category = id.into();
                    cx.notify();
                }))
            }));
        div()
            .id("automation-gallery")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .w_full()
                    .max_w(px(1032.))
                    .px(px(36.))
                    .pt(px(26.))
                    .pb(px(40.))
                    .flex()
                    .flex_col()
                    .gap(px(18.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .gap(px(6.))
                            .child(
                                glass::eyebrow("New automation")
                                    .line_height(px(13.))
                                    .text_color(eyebrow_ink()),
                            )
                            .child(
                                div()
                                    .text_size(px(24.))
                                    .line_height(px(29.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("Pick an example or start from scratch."),
                            ),
                    )
                    .child(div().flex().child(tabs))
                    .child(grid),
            )
            .into_any_element()
    }
}

/// The gallery eyebrow's violet, darker on a light page so it still reads.
fn eyebrow_ink() -> gpui::Hsla {
    if is_light() {
        rgb(0x6d4fd8).into()
    } else {
        rgb(0xc0affc).into()
    }
}

/// One gallery card: an icon tile, the title, what it does and, for an
/// example, its schedule (trigger kind and label) as a glass pill along the
/// bottom. The title is also the card's accessible name.
fn template_card(
    id: SharedString,
    title: impl Into<SharedString>,
    glyph: AnyElement,
    description: impl Into<SharedString>,
    schedule: Option<(String, String)>,
) -> gpui::Stateful<gpui::Div> {
    let title = title.into();
    glass::card()
        .id(id)
        .role(Role::Button)
        .aria_label(title.clone())
        .cursor_pointer()
        .w_full()
        .min_h(px(156.))
        .p(px(18.))
        .flex()
        .flex_col()
        .gap(px(10.))
        .hover(|s| s.bg(glass::fill_strong()))
        .child(layout::tile(32., glyph))
        .child(
            div()
                .text_size(px(14.5))
                .line_height(px(18.))
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title),
        )
        .child(
            div()
                .text_size(px(12.))
                .line_height(px(17.))
                .text_color(content(0.68))
                .child(description.into()),
        )
        .when_some(schedule, |d, (kind, label)| {
            d.child(
                div().mt_auto().flex().child(
                    div()
                        .h(px(24.))
                        .px(px(11.))
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .rounded_full()
                        .bg(glass::fill())
                        .border_1()
                        .border_color(glass::edge())
                        .text_size(px(12.))
                        .text_color(content(0.68))
                        .child(trigger_mark(&kind, 12.))
                        .child(label),
                ),
            )
        })
}
