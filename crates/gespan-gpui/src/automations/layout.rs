//! The page's list (automations grouped by project, each a frosted card with
//! a run sparkline) and the detail's summary cards.
use super::*;
use crate::glass;

/// Days the card sparkline covers, oldest first and today last.
pub(super) const SPARK_DAYS: usize = 10;
const CARD_RADIUS: f32 = 12.;
const SPARK_HEIGHT: f32 = 16.;

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
            .px(px(12.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .gap(px(8.));
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
                    .pt(px(if index == 0 { 2. } else { 10. }))
                    .pb(px(2.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(self.project_mark(&cwd, 14.))
                    .child(
                        glass::eyebrow(self.project_label(&cwd))
                            .flex_1()
                            .min_w_0()
                            .truncate(),
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
        // The selected automation is an accent-edged card; the rest are
        // plain glass.
        glass::card()
            .id(SharedString::from(format!("automation-{id}")))
            .debug_selector(move || selector)
            .relative()
            .flex_shrink_0()
            .rounded(px(CARD_RADIUS))
            .when(selected, |d| {
                d.border_color(accent_color().opacity(0.45))
                    .bg(accent_color().opacity(0.08))
            })
            .child(
                div()
                    .id(SharedString::from(format!("open-{id}")))
                    .role(Role::Button)
                    .aria_label(format!("Open {}", automation.name))
                    .aria_selected(selected)
                    .relative()
                    .rounded(px(CARD_RADIUS - 1.))
                    .when(!selected, |d| d.hover(|s| s.bg(glass::hover_fill())))
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
                            .child(icon_tile(&mark, 30., 14.))
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
                                            .text_size(px(11.5))
                                            .text_color(content(0.55))
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
                            .items_center()
                            .justify_between()
                            .gap(px(10.))
                            .child(sparkline(&id, counts))
                            .child(last_run_note(
                                &id,
                                automation.last_run_status.as_deref(),
                                automation.last_run_at,
                                now,
                            )),
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
            details.push(editor::access_label(editor::effective_mode(&draft)).into());
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
                // A new automation's project is picked in the header, so its
                // card opens the picker; a saved one's project is fixed.
                let project =
                    card.key == "project" && self.editor.as_ref().is_some_and(|e| !e.saved);
                let selector = format!("automation-summary-{}", card.key);
                glass::card()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector)
                    .role(Role::Button)
                    .aria_label(format!("{}: {}", card.title, card.primary))
                    .cursor_pointer()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .hover(|s| s.bg(glass::fill_strong()))
                    .child(
                        div()
                            .relative()
                            .p(px(14.))
                            .flex()
                            .flex_col()
                            .gap(px(4.))
                            .child(glass::eyebrow(card.title))
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
                    .on_click(cx.listener(move |this, _, window, cx| {
                        if project {
                            this.choose_project(window, cx);
                        } else {
                            this.jump_to(section, cx);
                        }
                    }))
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
        .gap(px(2.))
        .children(counts.iter().enumerate().map(|(day, &count)| {
            let selector = format!("automation-spark-{id}-{day}");
            div()
                .debug_selector(move || selector)
                .w(px(4.))
                .rounded(px(2.))
                .h(px(if count == 0 {
                    2.
                } else {
                    4. + (SPARK_HEIGHT - 4.) * count as f32 / peak as f32
                }))
                .bg(if count == 0 {
                    content(0.12)
                } else {
                    accent_color().opacity(0.8)
                })
        }))
}

fn last_run(at: Option<i64>, now: i64) -> String {
    match at.filter(|at| *at > 0) {
        None => "Never run".into(),
        Some(at) => format!("Last run {}", ago(at, now)),
    }
}

/// How long before `now` the run at `at` was: "5m ago", or a date once it is
/// a week old.
fn ago(at: i64, now: i64) -> String {
    let minutes = (now - at).max(0) / 60_000;
    match minutes {
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
    }
}

/// The card's latest-run pill: the result in words and its tone, so colour
/// is never the only cue. `None` when there is no known result to show.
pub(super) fn last_run_pill(
    status: Option<&str>,
    at: Option<i64>,
    now: i64,
) -> Option<(String, glass::Tone)> {
    use glass::Tone;
    let when = at.filter(|at| *at > 0).map(|at| ago(at, now));
    let pill = match (status?, when) {
        ("running", _) => ("Running now".into(), Tone::Running),
        ("pending", _) => ("Queued".into(), Tone::Running),
        ("succeeded", Some(when)) => (format!("Last run {when}"), Tone::Ok),
        ("failed", Some(when)) => (format!("Failed {when}"), Tone::Bad),
        ("skipped", Some(when)) => (format!("Skipped {when}"), Tone::Waiting),
        ("cancelled", Some(when)) => (format!("Cancelled {when}"), Tone::Muted),
        _ => return None,
    };
    Some(pill)
}

/// "Last run …" beneath a card: a status pill once the latest result is
/// known, plain text before that.
fn last_run_note(id: &str, status: Option<&str>, at: Option<i64>, now: i64) -> AnyElement {
    let id = id.to_owned();
    match last_run_pill(status, at, now) {
        Some((label, tone)) => glass::status(label, tone)
            .h(px(18.))
            .text_size(px(11.))
            .debug_selector(move || format!("automation-last-run-pill-{id}"))
            .into_any_element(),
        None => div()
            .debug_selector(move || format!("automation-last-run-{id}"))
            .truncate()
            .text_size(px(11.5))
            .text_color(content(0.55))
            .child(last_run(at, now))
            .into_any_element(),
    }
}

/// A rounded glass tile holding a trigger's mark.
pub(super) fn icon_tile(kind: &str, side: f32, glyph: f32) -> gpui::Div {
    tile(side, trigger_mark(kind, glyph))
}

/// A gallery glyph in the card's ink.
pub(super) fn icon_glyph(path: &'static str, side: f32) -> AnyElement {
    icon(path, side).text_color(content(0.9)).into_any_element()
}

/// A rounded glass tile around a glyph: a card's icon plate.
pub(super) fn tile(side: f32, glyph: AnyElement) -> gpui::Div {
    div()
        .size(px(side))
        .flex_shrink_0()
        .rounded(px(side * 0.27))
        .bg(glass::fill_strong())
        .border_1()
        .border_color(glass::edge())
        .flex()
        .items_center()
        .justify_center()
        .child(glyph)
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

    #[test]
    fn last_run_pill_names_the_result_in_words() {
        use glass::Tone;
        let now = 1_800_000_000_000;
        let hours_ago = Some(now - 6 * 3_600_000);
        let pill = |status| last_run_pill(Some(status), hours_ago, now);
        assert_eq!(
            pill("succeeded"),
            Some(("Last run 6h ago".into(), Tone::Ok))
        );
        // Anything but success says what happened, so colour is never the
        // only cue.
        assert_eq!(pill("failed"), Some(("Failed 6h ago".into(), Tone::Bad)));
        assert_eq!(
            pill("skipped"),
            Some(("Skipped 6h ago".into(), Tone::Waiting))
        );
        assert_eq!(
            pill("cancelled"),
            Some(("Cancelled 6h ago".into(), Tone::Muted))
        );
        // A live run is about now, whenever it was scheduled.
        assert_eq!(pill("running"), Some(("Running now".into(), Tone::Running)));
        assert_eq!(pill("pending"), Some(("Queued".into(), Tone::Running)));
        assert_eq!(
            last_run_pill(Some("running"), None, now),
            Some(("Running now".into(), Tone::Running))
        );
        // No result yet, an unknown status, or a finished result with no time
        // fall back to the plain "Last run …" line.
        assert_eq!(last_run_pill(None, hours_ago, now), None);
        assert_eq!(pill("exploded"), None);
        assert_eq!(last_run_pill(Some("failed"), None, now), None);
        assert_eq!(last_run_pill(Some("succeeded"), Some(0), now), None);
    }
}
