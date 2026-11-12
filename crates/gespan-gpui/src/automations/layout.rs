//! The page's list (automations grouped by project, each a frosted card with
//! a run sparkline) and the detail's summary cards.
use super::*;
use crate::glass;

/// Days the card sparkline covers, oldest first and today last.
pub(super) const SPARK_DAYS: usize = 10;
const CARD_RADIUS: f32 = 12.;
const SPARK_HEIGHT: f32 = 16.;
/// An empty day's stub.
const EMPTY_BAR: f32 = 3.;

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
                    .pt(px(if index == 0 { 2. } else { 8. }))
                    .pb(px(2.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(self.project_swatch(&cwd))
                    .child(
                        glass::eyebrow(self.project_label(&cwd))
                            .line_height(px(14.))
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
        let glyph = automation
            .triggers()
            .first()
            .map_or("clock", |t| trigger_glyph(&t.kind, &t.event));
        let zeros = [0; SPARK_DAYS];
        let counts = self.sparklines.get(&id).map_or(&zeros[..], Vec::as_slice);
        let clean = [false; SPARK_DAYS];
        let failed = self.spark_failed.get(&id).map_or(&clean[..], Vec::as_slice);
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
                            .child(tile(
                                30.,
                                icon(glyph, 14.)
                                    .text_color(content(0.92))
                                    .into_any_element(),
                            ))
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
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(automation.name.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_size(px(11.5))
                                            .line_height(px(16.))
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
                            .gap(px(8.))
                            .child(sparkline(&id, counts, failed))
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
                div().absolute().right(px(12.)).top(px(18.)).child(
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

/// One bar per day, its height following the count and its colour the
/// day's outcome: green for runs, rose when one of them failed. Empty days
/// draw as a short stub so the row keeps its rhythm.
fn sparkline(id: &str, counts: &[usize], failed: &[bool]) -> gpui::Stateful<gpui::Div> {
    let peak = counts.iter().copied().max().unwrap_or(0).max(1);
    let total: usize = counts.iter().sum();
    div()
        .id(SharedString::from(format!("automation-spark-{id}")))
        .debug_selector({
            let selector = format!("automation-spark-{id}");
            move || selector
        })
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
            let outcome = match (count, failed.get(day).copied().unwrap_or(false)) {
                (0, _) => DayOutcome::Empty,
                (_, true) => DayOutcome::Failed,
                _ => DayOutcome::Ran,
            };
            div()
                .debug_selector(move || selector)
                .w(px(4.))
                .rounded(px(2.))
                .h(px(if count == 0 {
                    EMPTY_BAR
                } else {
                    4. + (SPARK_HEIGHT - 4.) * count as f32 / peak as f32
                }))
                .bg(outcome.color())
        }))
}

/// How a sparkline day went.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum DayOutcome {
    Empty,
    Ran,
    Failed,
}

impl DayOutcome {
    fn color(self) -> gpui::Hsla {
        match (self, is_light()) {
            (Self::Empty, false) => white(0.12),
            (Self::Empty, true) => content(0.12),
            (Self::Ran, false) => gpui::rgba(0x6ee7b7bf).into(),
            (Self::Ran, true) => glass::Tone::Ok.ink().opacity(0.75),
            (Self::Failed, false) => gpui::rgba(0xfda4afd9).into(),
            (Self::Failed, true) => glass::Tone::Bad.ink().opacity(0.85),
        }
    }
}

/// Per sparkline day, oldest first: whether any run that day failed.
pub(super) fn failed_days(history: &[AutomationRun], days: usize, now: i64) -> Vec<bool> {
    let failed = history
        .iter()
        .filter(|run| run.status == "failed")
        .cloned()
        .collect::<Vec<_>>();
    automations::runs_per_day(&failed, days, now)
        .into_iter()
        .map(|count| count > 0)
        .collect()
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
        1440..WEEK_MINUTES => format!("{}d ago", minutes / 1440),
        _ => chrono::DateTime::from_timestamp_millis(at)
            .map(|date| {
                date.with_timezone(&chrono::Local)
                    .format("%-d %b")
                    .to_string()
            })
            .unwrap_or_default(),
    }
}

const WEEK_MINUTES: i64 = 10_080;

/// The card's "Last run" as a green pill: only for a success within the last
/// week. Anything older, failed or cancelled reads as the plain line, and
/// the bars carry the failures.
pub(super) fn recent_success(status: Option<&str>, at: Option<i64>, now: i64) -> Option<String> {
    let at = at.filter(|at| *at > 0)?;
    (status == Some("succeeded") && (now - at).max(0) / 60_000 < WEEK_MINUTES)
        .then(|| format!("Last run {}", ago(at, now)))
}

/// "Last run …" beneath a card: a pill right after the bars for a recent
/// success, else plain text against the card's right edge.
fn last_run_note(id: &str, status: Option<&str>, at: Option<i64>, now: i64) -> AnyElement {
    let id = id.to_owned();
    match recent_success(status, at, now) {
        Some(label) => glass::status(label, glass::Tone::Ok)
            .h(px(18.))
            .text_size(px(11.))
            .debug_selector(move || format!("automation-last-run-pill-{id}"))
            .into_any_element(),
        None => div()
            .debug_selector(move || format!("automation-last-run-{id}"))
            .ml_auto()
            .min_w_0()
            .truncate()
            .text_size(px(11.5))
            .line_height(px(16.))
            .text_color(content(0.55))
            .child(last_run(at, now))
            .into_any_element(),
    }
}

/// The glyph on an automation's tile: what starts it, not who hosts it.
pub(super) fn trigger_glyph(kind: &str, event: &str) -> &'static str {
    if kind == "time" {
        "clock"
    } else if event.contains("issue") {
        "alert-circle"
    } else if event.contains("pull_request") {
        "git-pull-request"
    } else if event.contains("push") || event.contains("commit") {
        "git-commit"
    } else {
        "zap"
    }
}

/// A gallery glyph in the given ink.
pub(super) fn icon_glyph(path: &'static str, side: f32, ink: gpui::Hsla) -> AnyElement {
    icon(path, side).text_color(ink).into_any_element()
}

/// The `.tile` plate: a 9px-rounded glass square around a glyph.
pub(super) fn tile(side: f32, glyph: AnyElement) -> gpui::Div {
    div()
        .size(px(side))
        .flex_shrink_0()
        .rounded(px(9.))
        .bg(if is_light() {
            glass::fill_strong()
        } else {
            white(0.07)
        })
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
    fn only_a_recent_success_reads_as_a_pill() {
        let now = 1_800_000_000_000;
        let hours_ago = Some(now - 6 * 3_600_000);
        assert_eq!(
            recent_success(Some("succeeded"), hours_ago, now),
            Some("Last run 6h ago".into())
        );
        // Six days is still recent; eight is a date on the plain line.
        let six_days = Some(now - 6 * 86_400_000);
        assert_eq!(
            recent_success(Some("succeeded"), six_days, now),
            Some("Last run 6d ago".into())
        );
        assert_eq!(
            recent_success(Some("succeeded"), Some(now - 8 * 86_400_000), now),
            None
        );
        // Failures and cancellations show on the bars, not as a pill.
        for status in ["failed", "cancelled", "skipped", "running", "pending"] {
            assert_eq!(
                recent_success(Some(status), hours_ago, now),
                None,
                "{status}"
            );
        }
        assert_eq!(recent_success(None, hours_ago, now), None);
        assert_eq!(recent_success(Some("succeeded"), None, now), None);
        assert_eq!(recent_success(Some("succeeded"), Some(0), now), None);
    }

    #[test]
    fn tile_glyph_follows_what_starts_the_automation() {
        assert_eq!(trigger_glyph("github", "issue_opened"), "alert-circle");
        assert_eq!(trigger_glyph("linear", "issue_created"), "alert-circle");
        assert_eq!(
            trigger_glyph("github", "pull_request_opened"),
            "git-pull-request"
        );
        assert_eq!(trigger_glyph("time", ""), "clock");
        assert_eq!(trigger_glyph("gitlab", "something_new"), "zap");
    }
}
