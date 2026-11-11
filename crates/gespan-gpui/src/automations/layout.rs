//! The page's list (automations grouped by project, each a frosted card with
//! a run sparkline) and the detail's summary cards.
use super::*;
use crate::modal;

/// Days the card sparkline covers, oldest first and today last.
pub(super) const SPARK_DAYS: usize = 10;
const CARD_RADIUS: f32 = 12.;
const SPARK_HEIGHT: f32 = 22.;

/// The editor section a summary card jumps to, in the order the Instructions
/// tab draws them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Section {
    Triggers = 0,
    Instructions = 1,
    Session = 2,
}

pub(super) struct Summary {
    pub key: &'static str,
    pub title: &'static str,
    pub primary: String,
    pub secondary: String,
    pub section: Section,
}

impl Automations {
    /// `automations` by the project they run in: this window's project first,
    /// then the rest by name. Each group keeps the list's order.
    pub(super) fn project_groups<'a>(
        &self,
        automations: &'a [Automation],
    ) -> Vec<(String, Vec<&'a Automation>)> {
        let mut groups: Vec<(String, Vec<&Automation>)> = Vec::new();
        for automation in automations {
            let key = gespan_core::files::path_key(&automation.cwd);
            match groups
                .iter_mut()
                .find(|(cwd, _)| gespan_core::files::path_key(cwd) == key)
            {
                Some((_, items)) => items.push(automation),
                None => groups.push((automation.cwd.clone(), vec![automation])),
            }
        }
        groups.sort_by_cached_key(|(cwd, _)| {
            (
                gespan_core::files::path_key(cwd) != gespan_core::files::path_key(&self.cwd),
                self.project_label(cwd).to_lowercase(),
            )
        });
        groups
    }

    pub(super) fn render_list(
        &self,
        visible: &[Automation],
        query: &str,
        cx: &mut Context<Self>,
    ) -> gpui::Stateful<gpui::Div> {
        let mut list = div()
            .id("automations-list")
            .role(Role::ListBox)
            .aria_label("Automations")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(10.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(6.));
        if visible.is_empty() {
            return list.child(
                div()
                    .px(px(6.))
                    .py(px(32.))
                    .text_size(px(12.))
                    .text_color(content(0.45))
                    .child(if self.loading {
                        "Loading automations…"
                    } else if query.trim().is_empty() {
                        "No automations yet"
                    } else {
                        "No matching automations"
                    }),
            );
        }
        let now = gespan_core::time::now_ms() as i64;
        for (index, (cwd, items)) in self.project_groups(visible).into_iter().enumerate() {
            let selector = format!("automation-group-{index}");
            list = list.child(
                div()
                    .debug_selector(move || selector)
                    .px(px(6.))
                    .pt(px(if index == 0 { 4. } else { 10. }))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(12.))
                    .text_color(content(0.55))
                    .child(self.project_mark(&cwd, 14.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(self.project_label(&cwd)),
                    ),
            );
            for automation in items {
                list = list.child(self.render_card(automation, now, cx));
            }
        }
        list
    }

    fn render_card(&self, automation: &Automation, now: i64, cx: &mut Context<Self>) -> AnyElement {
        let id = automation.id.clone();
        let toggle_id = id.clone();
        let enabled = automation.enabled;
        let selected = !self.gallery && self.selected.as_deref() == Some(&id);
        let kind = Kind::from_id(&automation.harness);
        let model = gespan_core::store::native_model(&automation.model, &automation.harness);
        let mark = automation
            .triggers()
            .first()
            .map_or_else(|| "time".to_string(), |t| t.kind.clone());
        let zeros = [0; SPARK_DAYS];
        let counts = self.sparklines.get(&id).map_or(&zeros[..], Vec::as_slice);
        let selector = format!("automation-card-{id}");
        div()
            .id(SharedString::from(format!("automation-{id}")))
            .debug_selector(move || selector)
            .relative()
            .flex_shrink_0()
            .rounded(px(CARD_RADIUS))
            .border_1()
            .border_color(content(modal::BORDER))
            .child(modal::surface(CARD_RADIUS - 1.))
            .child(
                div()
                    .id(SharedString::from(format!("open-{id}")))
                    .role(Role::Button)
                    .aria_label(format!("Open {}", automation.name))
                    .relative()
                    .rounded(px(CARD_RADIUS - 1.))
                    .when(selected, |d| d.bg(content(0.1)))
                    .when(!selected, |d| d.hover(|s| s.bg(content(0.05))))
                    .p(px(12.))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(
                        div()
                            .pr(px(36.))
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .size(px(30.))
                                    .flex_shrink_0()
                                    .rounded(px(8.))
                                    .bg(content(0.08))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .child(trigger_mark(&mark, 16.)),
                            )
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
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(automation.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(11.))
                                            .text_color(content(0.5))
                                            .child(format!(
                                                "{} · {}",
                                                automation.trigger_label(),
                                                match automation.action.pilot_id() {
                                                    Some(id) => {
                                                        format!("→ {}", self.pilot_label(id))
                                                    }
                                                    None =>
                                                        cached_model_name(kind, model.as_deref())
                                                            .to_string(),
                                                }
                                            )),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .items_end()
                            .justify_between()
                            .gap(px(10.))
                            .child(sparkline(&id, counts))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(content(0.5))
                                    .child(last_run(automation.last_run_at, now)),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.select(id.clone(), cx))),
            )
            .child(
                div().absolute().right(px(12.)).top(px(19.)).child(
                    editor::toggle(SharedString::from(format!("toggle-{toggle_id}")), enabled)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.toggle(toggle_id.clone(), !enabled, cx)
                        })),
                ),
            )
            .into_any_element()
    }

    /// The detail's three cards, read from the editor's current values so
    /// they follow unsaved edits.
    pub(super) fn summary_cards(&self, cx: &App) -> Vec<Summary> {
        let Some(draft) = self.collected(cx) else {
            return Vec::new();
        };
        let project = Summary {
            key: "project",
            title: "Runs in project",
            primary: if draft.cwd.is_empty() {
                "Choose project".into()
            } else {
                self.project_label(&draft.cwd)
            },
            // A Pilot works where it chooses; the project only matches events.
            secondary: if draft.action.is_session() {
                editor::workspace_label(&draft.workspace_mode).into()
            } else {
                "Events are matched here".into()
            },
            section: Section::Session,
        };
        let triggers = draft.triggers.clone().unwrap_or_default();
        let (primary, secondary) = match triggers.first() {
            None => ("Manual only".into(), "Starts only with Run now".into()),
            Some(first) => {
                let primary = match triggers.len() {
                    1 => first.label(),
                    count => format!("{} + {} more", first.label(), count - 1),
                };
                let secondary = if first.kind == "time" {
                    editor::next_run_preview(first.next_at(gespan_core::time::now_ms() as i64))
                } else {
                    let actor = first.actor.trim();
                    let repos = first
                        .repos
                        .iter()
                        .chain(std::iter::once(&first.repo))
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty())
                        .collect::<Vec<_>>();
                    format!(
                        "{} · by {} · {}",
                        source_name(&first.kind),
                        if actor.is_empty() { "anyone" } else { actor },
                        if repos.is_empty() {
                            "any repository".into()
                        } else {
                            repos.join(", ")
                        }
                    )
                };
                (primary, secondary)
            }
        };
        let when = Summary {
            key: "when",
            title: "When",
            primary,
            secondary,
            section: Section::Triggers,
        };
        let kind = Kind::from_id(&draft.harness);
        let model = gespan_core::store::native_model(&draft.model, &draft.harness);
        let effort = self
            .model_controls
            .iter()
            .find(|control| editor::is_effort(&control.id))
            .map(|control| editor::control_label(control, &draft))
            .or_else(|| editor::saved_effort(&draft));
        let mut details = Vec::new();
        if let Some(effort) = effort {
            details.push(format!("Effort {effort}"));
        }
        if draft.harness != "fx" {
            details.push(editor::access_label(&draft.runtime_mode).into());
        }
        if let Some(id) = draft.action.pilot_id() {
            let pilot = Summary {
                key: "agent",
                title: "Pilot",
                primary: if id.is_empty() {
                    "Choose a Pilot".into()
                } else {
                    self.pilot_label(id)
                },
                secondary: "The run is queued on its chat".into(),
                section: Section::Session,
            };
            return vec![project, when, pilot];
        }
        let agent = Summary {
            key: "agent",
            title: "Agent",
            primary: format!(
                "{} · {}",
                kind.label(),
                cached_model_name(kind, model.as_deref())
            ),
            secondary: details.join(" · "),
            section: Section::Instructions,
        };
        vec![project, when, agent]
    }

    pub(super) fn render_summary_cards(&self, cx: &mut Context<Self>) -> AnyElement {
        let cards = self.summary_cards(cx);
        div()
            .flex()
            .gap(px(12.))
            .children(cards.into_iter().map(|card| {
                let section = card.section;
                let selector = format!("automation-summary-{}", card.key);
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector)
                    .role(Role::Button)
                    .aria_label(format!("{}: {}", card.title, card.primary))
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .rounded(px(14.))
                    .border_1()
                    .border_color(content(modal::BORDER))
                    .hover(|s| s.border_color(content(0.2)))
                    .child(modal::surface(13.))
                    .child(
                        div()
                            .relative()
                            .p(px(14.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(
                                div()
                                    .text_size(px(11.))
                                    .text_color(content(0.5))
                                    .child(card.title),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child(card.primary),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.))
                                    .text_color(content(0.5))
                                    .child(card.secondary),
                            ),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.jump_to(section, cx)))
            }))
            .into_any_element()
    }

    /// Shows the Instructions tab scrolled to `section`.
    fn jump_to(&mut self, section: Section, cx: &mut Context<Self>) {
        self.history = false;
        self.choice = None;
        self.settings_scroll.scroll_to_top_of_item(section as usize);
        cx.notify();
    }
}

/// One bar per day, its height following the count; empty days draw as a
/// flat stub so the row keeps its rhythm.
fn sparkline(id: &str, counts: &[usize]) -> gpui::Stateful<gpui::Div> {
    let peak = counts.iter().copied().max().unwrap_or(0).max(1);
    let total: usize = counts.iter().sum();
    div()
        .id(SharedString::from(format!("automation-spark-{id}")))
        .role(Role::Image)
        .aria_label(match total {
            0 => format!("No runs in the last {SPARK_DAYS} days"),
            1 => format!("1 run in the last {SPARK_DAYS} days"),
            n => format!("{n} runs in the last {SPARK_DAYS} days"),
        })
        .h(px(SPARK_HEIGHT))
        .flex_shrink_0()
        .flex()
        .items_end()
        .gap(px(3.))
        .children(counts.iter().enumerate().map(|(day, &count)| {
            let selector = format!("automation-spark-{id}-{day}");
            div()
                .debug_selector(move || selector)
                .w(px(5.))
                .rounded(px(2.))
                .h(px(if count == 0 {
                    2.
                } else {
                    4. + (SPARK_HEIGHT - 4.) * count as f32 / peak as f32
                }))
                .bg(if count == 0 {
                    content(0.14)
                } else {
                    accent_color()
                })
        }))
}

fn last_run(at: Option<i64>, now: i64) -> String {
    let Some(at) = at.filter(|at| *at > 0) else {
        return "Never run".into();
    };
    let minutes = (now - at).max(0) / 60_000;
    let label = match minutes {
        0 => "just now".into(),
        1..60 => format!("{minutes}m ago"),
        60..1440 => format!("{}h ago", minutes / 60),
        1440..10_080 => format!("{}d ago", minutes / 1440),
        _ => chrono::DateTime::from_timestamp_millis(at)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%-d %b")
                    .to_string()
            })
            .unwrap_or_default(),
    };
    format!("Last run {label}")
}

fn source_name(kind: &str) -> &str {
    match kind {
        "github" => "GitHub",
        "gitlab" => "GitLab",
        "linear" => "Linear",
        "jira" => "Jira",
        "azuredevops" => "Azure DevOps",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_run_reads_relative_to_now() {
        let now = 1_800_000_000_000;
        assert_eq!(last_run(None, now), "Never run");
        assert_eq!(last_run(Some(0), now), "Never run");
        assert_eq!(last_run(Some(now - 30_000), now), "Last run just now");
        assert_eq!(last_run(Some(now - 5 * 60_000), now), "Last run 5m ago");
        assert_eq!(last_run(Some(now - 3 * 3_600_000), now), "Last run 3h ago");
        assert_eq!(last_run(Some(now - 2 * 86_400_000), now), "Last run 2d ago");
        // A clock a little behind the run never reads as the future.
        assert_eq!(last_run(Some(now + 60_000), now), "Last run just now");
        assert!(!last_run(Some(now - 30 * 86_400_000), now).contains("ago"));
    }
}
