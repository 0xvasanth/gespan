use super::*;
use crate::glass;
use crate::popover::{Align, Anchor, popover};
use gpui::{
    AnyElement, ClipboardItem, Div, FontWeight, Hsla, MouseButton, Pixels, Role, SharedString,
    Stateful, Window, canvas, div, list, prelude::*, px, rgb,
};

pub(super) fn action(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Stateful<Div> {
    plain_action(id, label).hover(|s| s.bg(content(0.07)).text_color(content(1.)))
}
/// An [`action`] without its hover, for callers that set their own.
fn plain_action(id: impl Into<SharedString>, label: impl Into<SharedString>) -> Stateful<Div> {
    let id: SharedString = id.into();
    let label = label.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.clone())
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(content(0.7))
        .cursor_pointer()
        .child(label)
}

fn icon_action(id: impl Into<SharedString>, label: &str, name: &'static str) -> Stateful<Div> {
    icon_action_ink(id, label, name, content(0.7))
}
fn icon_action_ink(
    id: impl Into<SharedString>,
    label: &str,
    name: &'static str,
    ink: Hsla,
) -> Stateful<Div> {
    icon_action_with(id, label, action_icon(name, ink), ink)
}
fn icon_action_with(
    id: impl Into<SharedString>,
    label: &str,
    glyph: impl IntoElement,
    ink: Hsla,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(label.to_owned())
        .h(px(28.))
        .px(px(10.))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(6.))
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(content(0.7))
        .cursor_pointer()
        .hover(move |s| {
            if ink == shell_fill() {
                s.bg(shell_fill().opacity(0.1)).text_color(ink)
            } else {
                s.bg(content(0.07)).text_color(content(1.))
            }
        })
        .child(glyph)
        .child(label.to_owned())
}
fn action_icon(name: &'static str, ink: Hsla) -> gpui::Svg {
    icon(name, 14.).text_color(ink)
}
pub(crate) fn relative_time(iso: &str) -> String {
    if model::timestamp_ms(iso) == 0 {
        return String::new();
    }
    let mut value =
        (((model::timestamp_ms(iso) - chrono::Utc::now().timestamp_millis()) as f64 / 1000.) + 0.5)
            .floor();
    let mut unit = "second";
    for (step, name) in [
        (60., "second"),
        (60., "minute"),
        (24., "hour"),
        (7., "day"),
        (4.34524, "week"),
        (12., "month"),
        (f64::INFINITY, "year"),
    ] {
        unit = name;
        if value.abs() < step {
            break;
        }
        value = ((value / step) + 0.5).floor();
    }
    let amount = value as i64;
    match (unit, amount) {
        ("second", 0) => "now".into(),
        ("day", -1) => "yesterday".into(),
        ("day", 1) => "tomorrow".into(),
        ("week" | "month" | "year", -1) => format!("last {unit}"),
        ("week" | "month" | "year", 1) => format!("next {unit}"),
        _ => {
            let count = amount.unsigned_abs();
            let label = format!("{count} {unit}{}", if count == 1 { "" } else { "s" });
            if amount > 0 {
                format!("in {label}")
            } else {
                format!("{label} ago")
            }
        }
    }
}
fn review_label(state: &str) -> String {
    match state.trim().to_uppercase().as_str() {
        "APPROVED" => "Approved".into(),
        "CHANGES_REQUESTED" => "Changes requested".into(),
        "COMMENTED" => "Commented".into(),
        "DISMISSED" => "Dismissed".into(),
        "PENDING" => "Pending".into(),
        "REVIEW_REQUIRED" => "Review required".into(),
        _ => state.replace('_', " ").to_lowercase(),
    }
}
fn failed_check(state: &str) -> bool {
    state == "fail"
}
fn review_event(comment: &model::Comment) -> bool {
    comment.kind == "review" && comment.body.trim().is_empty() && comment.replies.is_empty()
}
#[derive(Clone)]
pub(super) enum Activity {
    Comment(std::sync::Arc<model::Comment>),
    Commits(Vec<model::Commit>),
}
#[derive(Default)]
pub(super) struct SummaryRows {
    revision: u64,
    discussion: bool,
    keys: Vec<SharedString>,
}
fn timeline_event(name: &str, action: &str, time: &str) -> Div {
    div()
        .h(px(20.))
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .text_size(px(12.))
        .text_color(content(0.5))
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(FontWeight::MEDIUM)
                .text_color(content(1.))
                .child(if name.is_empty() {
                    "ghost".to_owned()
                } else {
                    name.to_owned()
                }),
        )
        .child(action.to_owned())
        .when(!time.is_empty(), |d| {
            d.child("·").child(relative_time(time))
        })
}
pub(super) fn menu_row(
    id: impl Into<SharedString>,
    label: impl Into<SharedString>,
    checked: bool,
) -> Stateful<Div> {
    let id: SharedString = id.into();
    let label = label.into();
    div()
        .id(id)
        .role(Role::MenuItemCheckBox)
        .aria_label(label.clone())
        .h(px(28.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(content(0.8))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.06)))
        .child(
            div()
                .size(px(14.))
                .when(checked, |d| d.child(icon("check", 14.))),
        )
        .child(div().flex_1().min_w_0().truncate().child(label))
}
/// [`menu_action`] with a label and id made at run time.
pub(super) fn menu_action_owned(id: String, label: String, path: &'static str) -> Stateful<Div> {
    let selector = id.clone();
    div()
        .id(SharedString::from(id))
        .debug_selector(move || selector.clone())
        .role(Role::MenuItem)
        .aria_label(label.clone())
        .h(px(28.))
        .px(px(8.))
        .flex()
        .items_center()
        .gap(px(8.))
        .rounded(px(6.))
        .text_size(px(12.))
        .text_color(content(0.8))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.06)))
        .child(icon(path, 14.).text_color(content(0.6)))
        .child(div().flex_1().min_w_0().truncate().child(label))
}
pub(super) fn divider() -> Div {
    div().mx(px(8.)).my(px(6.)).h(px(1.)).bg(content(0.08))
}
pub(super) fn caption(text: impl Into<SharedString>) -> Div {
    div()
        .px(px(8.))
        .pt(px(10.))
        .pb(px(4.))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(content(0.4))
        .child(text.into())
}
pub(super) fn message(text: impl Into<SharedString>) -> Div {
    div()
        .p(px(12.))
        .text_size(px(12.))
        .line_height(px(19.))
        .text_color(content(0.5))
        .child(text.into())
}
/// What waits for its data: a spinner and what it is fetching, over
/// skeleton lines in the shape of what comes.
pub(super) fn loading(label: &'static str) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(13.))
                .line_height(px(20.))
                .text_color(content(0.62))
                .child(crate::right_panel::spinner(label, "loader", 14.))
                .child(label),
        )
        .children([0.92, 0.74, 0.56].map(|width| {
            div()
                .h(px(10.))
                .w(gpui::relative(width))
                .rounded(px(5.))
                .bg(content(0.07))
        }))
}
/// [`loading`] for a whole tab: a glass card on the header's 28px column.
pub(super) fn loading_panel(id: &'static str, label: &'static str) -> Div {
    div().px(px(28.)).pt(px(18.)).child(
        glass::card()
            .debug_selector(move || id.into())
            .px(px(20.))
            .py(px(18.))
            .child(loading(label)),
    )
}
pub(super) fn error(text: impl Into<SharedString>) -> Div {
    message(text).text_color(rgb(0xf87171))
}
pub(super) fn provider_mark(provider: &str) -> gpui::Svg {
    let (path, color) = match provider {
        "linear" => ("inbox-linear", content(0.8)),
        "jira" => ("inbox-jira", rgb(0x2684ff).into()),
        "plane" => ("inbox-plane", content(0.8)),
        "gitlab" => ("inbox-gitlab", rgb(0xfc6d26).into()),
        "azuredevops" => ("inbox-azuredevops", content(0.8)),
        _ => ("inbox-github", content(0.8)),
    };
    icon(path, 14.).text_color(color)
}
fn status_icon(item: &Item) -> &'static str {
    if item.kind == "pr" {
        "git-pull-request"
    } else if item.status() == "Closed" {
        "check-circle"
    } else {
        "circle-dot"
    }
}
pub(super) fn toggle(list: &mut Vec<String>, key: String) {
    if list.contains(&key) {
        list.retain(|value| value != &key);
    } else {
        list.push(key);
    }
}
/// A row's age in its shortest form: "now", "14m", "2h", "3d", "5w", "1y".
pub(super) fn short_age(iso: &str) -> String {
    let at = model::timestamp_ms(iso);
    if at == 0 {
        return String::new();
    }
    let minutes = (chrono::Utc::now().timestamp_millis() - at).max(0) / 60_000;
    match minutes {
        0 => "now".into(),
        m if m < 60 => format!("{m}m"),
        m if m < 60 * 24 => format!("{}h", m / 60),
        m if m < 60 * 24 * 7 => format!("{}d", m / (60 * 24)),
        m if m < 60 * 24 * 365 => format!("{}w", m / (60 * 24 * 7)),
        m => format!("{}y", m / (60 * 24 * 365)),
    }
}
/// The document's status pill: the item's kind and state, green while open.
fn document_status(item: &Item) -> Div {
    let tone = document_tone(item);
    let ink = tone.ink();
    div()
        .flex_none()
        .h(px(22.))
        .pl(px(8.))
        .pr(px(9.))
        .flex()
        .items_center()
        .gap(px(6.))
        .rounded_full()
        .bg(ink.opacity(0.10))
        .border_1()
        .border_color(ink.opacity(0.2))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .text_color(ink)
        .child(icon(status_icon(item), 12.).text_color(ink))
        .child(item.status())
}
/// "vasanth · opened 14m ago", or just the age when the author is unknown.
fn opened_line(author: &str, created_at: &str) -> String {
    let age = short_age(created_at);
    match (author.is_empty(), age.as_str()) {
        (_, "") => author.to_string(),
        (true, "now") => "opened just now".into(),
        (true, age) => format!("opened {age} ago"),
        (false, "now") => format!("{author} · opened just now"),
        (false, age) => format!("{author} · opened {age} ago"),
    }
}
/// The document's status pill: open work reads green, closed red.
fn document_tone(item: &Item) -> glass::Tone {
    match item.status() {
        "Open" | "Merged" => glass::Tone::Ok,
        "Closed" => glass::Tone::Bad,
        _ => glass::Tone::Muted,
    }
}
/// How long a finished check took, else what state it is in.
fn check_time(check: &model::Check) -> String {
    model::ci_repair::check_duration(check.started_at.as_deref(), check.completed_at.as_deref())
        .unwrap_or_else(|| match check.state.as_str() {
            "pending" => "running".into(),
            state => model::ci_repair::state_label(state).to_lowercase(),
        })
}
/// The checks card's order: failures first, the rest as they arrived.
fn card_order(checks: &[model::Check]) -> Vec<model::Check> {
    let mut rows = checks.to_vec();
    rows.sort_by_key(|check| !failed_check(&check.state));
    rows
}
/// The checks card's heading pill: failures counted, else running or passing.
fn checks_overall(checks: &Checks) -> (String, glass::Tone) {
    let counts = model::ci_repair::Counts::checks(&checks.checks);
    if counts.fail > 0 {
        (format!("{} failed", counts.fail), glass::Tone::Bad)
    } else if counts.pending > 0 {
        ("Running".into(), glass::Tone::Running)
    } else if checks.checks.is_empty() {
        ("None".into(), glass::Tone::Muted)
    } else if counts.pass + counts.skipping == checks.checks.len() {
        ("Passing".into(), glass::Tone::Ok)
    } else {
        ("Finished".into(), glass::Tone::Muted)
    }
}
/// The reviewers card's rows and heading pill: who is asked, and the
/// review decision. `None` when the provider gave neither.
/// Who is asked to review, and the review decision as a pill.
type Reviewers = (Vec<String>, Option<(String, glass::Tone)>);
fn reviewers(details: &Details) -> Option<Reviewers> {
    let decision = match details.review_decision.trim().to_uppercase().as_str() {
        "" => None,
        "APPROVED" => Some((review_label("APPROVED"), glass::Tone::Ok)),
        "CHANGES_REQUESTED" => Some((review_label("CHANGES_REQUESTED"), glass::Tone::Bad)),
        other => Some((review_label(other), glass::Tone::Waiting)),
    };
    let requested = details
        .review_requests
        .iter()
        .map(|login| login.trim().to_string())
        .filter(|login| !login.is_empty())
        .collect::<Vec<_>>();
    (!requested.is_empty() || decision.is_some()).then_some((requested, decision))
}
fn files_label(files: usize) -> String {
    format!("{files} file{}", if files == 1 { "" } else { "s" })
}
fn project_name(path: &std::path::Path) -> String {
    path.file_name()
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

fn label_badge(label: &model::Label, compact: bool) -> Div {
    let color = u32::from_str_radix(label.color.trim().trim_start_matches('#'), 16)
        .ok()
        .filter(|_| label.color.trim().trim_start_matches('#').len() == 6);
    div()
        .max_w(px(if compact { 80. } else { 200. }))
        .rounded(px(3.))
        .px(px(6.))
        .py(px(1.))
        .flex()
        .items_center()
        .gap(px(4.))
        .bg(content(0.08))
        .text_size(px(if compact { 10. } else { 11. }))
        .text_color(content(0.5))
        .when_some(color, |d, color| {
            d.child(div().size(px(6.)).rounded_full().bg(rgb(color)))
        })
        .child(div().min_w_0().truncate().child(label.name.clone()))
}

#[derive(Clone, Copy)]
enum CommentStyle {
    Plain,
    Timeline,
    Reply,
}

impl Inbox {
    pub(super) fn project_label(&self, path: &str) -> String {
        self.appearances.get(path).map_or_else(
            || project_name(std::path::Path::new(path)),
            |appearance| appearance.label(path),
        )
    }
    fn project_mark(&self, path: &str, side: f32) -> AnyElement {
        crate::project_settings::project_icon(
            &self.appearances.get(path).cloned().unwrap_or_default(),
            path,
            side,
        )
    }

    fn related_sessions(&self, item: &Item) -> Vec<Session> {
        let work_item = item.work_item();
        self.sessions
            .iter()
            .filter(|s| {
                s.work_item
                    .as_ref()
                    .and_then(|w| w.github())
                    .is_some_and(|work| {
                        (!work.url.is_empty() && work.url == work_item.url)
                            || (work.kind == work_item.kind
                                && work.number == work_item.number
                                && work.repo.eq_ignore_ascii_case(&work_item.repo))
                    })
            })
            .cloned()
            .collect()
    }

    fn detail_view(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.item() else {
            if self.visible_items(cx).is_empty() {
                return div()
                    .debug_selector(|| "inbox-empty-detail".into())
                    .size_full()
                    .child(self.hosts_card(cx))
                    .into_any_element();
            }
            return div()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .child(
                    glass::empty_state(
                        "git-pull-request",
                        "Select a pull request",
                        "Its overview, checks and files open here.",
                    )
                    .debug_selector(|| "inbox-empty-detail".into()),
                )
                .into_any_element();
        };
        let is_pr = item.kind == "pr";
        let details = self.details.clone().unwrap_or_default();
        let base_ref = if details.base_ref_name.is_empty() {
            self.thread
                .as_ref()
                .map(|t| t.base_ref_name.as_str())
                .unwrap_or("")
        } else {
            &details.base_ref_name
        };
        let head_ref = if details.head_ref_name.is_empty() {
            self.thread
                .as_ref()
                .map(|t| t.head_ref_name.as_str())
                .unwrap_or("")
        } else {
            &details.head_ref_name
        };
        let kind = if is_pr {
            if item.provider == "gitlab" {
                "Merge request"
            } else {
                "Pull request"
            }
        } else {
            "Issue"
        };
        let place = if item.team_name.is_empty() {
            item.repo.clone()
        } else {
            item.team_name.clone()
        };
        let more = div()
            .id("inbox-more")
            .debug_selector(|| "inbox-more".into())
            .role(Role::Button)
            .aria_label("More actions")
            .relative()
            .flex_none()
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(9.))
            .bg(if is_light() {
                content(0.06)
            } else {
                white(0.12)
            })
            .border_1()
            .border_color(if is_light() {
                content(0.12)
            } else {
                white(0.18)
            })
            .text_color(content(0.9))
            .cursor_pointer()
            .hover(|s| s.bg(content(0.16)))
            .child(self.action_anchor.probe())
            .child(icon("more-horizontal", 16.))
            .on_click(cx.listener(|this, _, _, cx| this.toggle_menu(cx)));
        let start_session = (!is_pr).then(|| {
            glass::button(
                "inbox-send-agent",
                "Open in new session",
                glass::Button::Primary,
            )
            .h(px(30.))
            .role(Role::Button)
            .aria_label("Open in new session")
            .when(self.detail_loading || self.details.is_none(), |d| {
                d.opacity(0.4)
            })
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.detail_loading && this.details.is_some() {
                    this.start(cx);
                }
            }))
        });
        let choose_project = (!is_pr && item.project_path.is_empty()).then(|| {
            icon_action(
                "inbox-choose-project",
                &self.project_label(&self.start_project.to_string_lossy()),
                "folder",
            )
            .relative()
            .h(px(30.))
            .bg(glass::fill())
            .border_1()
            .border_color(glass::edge())
            .child(icon("chevron-down", 12.))
            .child(self.project_anchor.probe())
            .on_click(cx.listener(|this, _, _, cx| {
                this.project_picker = !this.project_picker;
                cx.notify();
            }))
        });
        let place_line = div()
            .min_w_0()
            .flex_1()
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(13.))
            .text_color(content(0.62))
            .child(document_status(&item).debug_selector(|| "inbox-document-status".into()))
            .child(div().min_w_0().truncate().child(if place.is_empty() {
                format!("{kind} {}", item.reference())
            } else {
                format!("{place} {}", item.reference())
            }))
            .when(!base_ref.is_empty() && !head_ref.is_empty(), |d| {
                let branch = head_ref.to_owned();
                d.child(div().text_color(content(0.45)).child("·")).child(
                    div()
                        .id("inbox-copy-branch")
                        .role(Role::Button)
                        .aria_label("Copy branch name")
                        .min_w_0()
                        .truncate()
                        .cursor_pointer()
                        .font_family(crate::fonts::monospace())
                        .text_size(px(11.5))
                        .hover(|s| s.text_color(content(1.)))
                        .child(format!("{head_ref} → {base_ref}"))
                        .on_click(move |_, _, cx| {
                            cx.write_to_clipboard(ClipboardItem::new_string(branch.clone()))
                        }),
                )
            });
        let diff_pill = self.diff.as_ref().map(|diff| {
            div()
                .debug_selector(|| "inbox-diff-pill".into())
                .flex_none()
                .h(px(22.))
                .px(px(9.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded_full()
                .bg(glass::fill())
                .border_1()
                .border_color(glass::edge())
                .text_size(px(11.5))
                .text_color(content(0.62))
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(diff_foreground(true))
                        .child(format!("+{}", diff.additions)),
                )
                .child(
                    div()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(diff_foreground(false))
                        .child(format!("−{}", diff.deletions)),
                )
                .child(files_label(diff.files.len()))
        });
        let failed = self
            .checks
            .as_ref()
            .map(|checks| {
                checks
                    .checks
                    .iter()
                    .filter(|check| failed_check(&check.state))
                    .count()
            })
            .unwrap_or(0);
        let tabs = is_pr.then(|| {
            div().mt(px(4.)).flex().items_center().gap(px(10.)).child(
                glass::segmented()
                    .id("inbox-pr-tabs")
                    .role(Role::TabList)
                    .aria_label("Pull request view")
                    .children(
                        [
                            (Tab::Summary, "Overview"),
                            (Tab::Checks, "Checks"),
                            (Tab::Code, "Files"),
                        ]
                        .into_iter()
                        .filter(|(tab, _)| *tab != Tab::Checks || item.provider == "github")
                        .map(|(tab, label)| {
                            let count = match tab {
                                Tab::Summary => None,
                                Tab::Code => {
                                    self.diff.as_ref().map(|diff| (diff.files.len(), false))
                                }
                                Tab::Checks => self
                                    .checks
                                    .as_ref()
                                    .map(|checks| (checks.checks.len(), failed > 0)),
                            };
                            glass::segment(
                                SharedString::from(format!("inbox-tab-{label}")),
                                label,
                                self.tab == tab,
                            )
                            .debug_selector(move || format!("inbox-tab-{label}"))
                            .role(Role::Tab)
                            .aria_label(label)
                            .aria_selected(self.tab == tab)
                            .when_some(count.filter(|(n, _)| *n > 0), |d, (n, hot)| {
                                d.child(glass::count(n, hot))
                            })
                            .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
                        }),
                    ),
            )
        });
        let author = if details.author.is_empty() {
            item.author.clone()
        } else {
            details.author.clone()
        };
        let title = div()
            .w_full()
            .flex_shrink_0()
            .px(px(28.))
            .pt(px(22.))
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .child(place_line)
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .children(choose_project)
                            .children(start_session)
                            .child(more),
                    ),
            )
            .child(
                div()
                    .line_clamp(2)
                    .text_size(px(24.))
                    .line_height(px(29.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(content(1.))
                    .child(self.text.text(item.title.clone())),
            )
            .child(
                div()
                    .min_w_0()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .overflow_hidden()
                    .text_size(px(13.))
                    .text_color(content(0.62))
                    .when(!author.is_empty(), |d| {
                        d.child(if details.author_avatar_url.is_empty() {
                            menu::initials_avatar(&author)
                        } else {
                            avatar::person(
                                &item.provider,
                                &author,
                                &details.author_avatar_url,
                                22.,
                                window,
                                cx,
                            )
                            .into_any_element()
                        })
                    })
                    .child(self.text.text(opened_line(&author, &item.created_at)))
                    .children(diff_pill.map(|pill| pill.ml(px(6.))))
                    .children(self.run_pill_view(&item, cx))
                    .when_some(self.action_notice.clone(), |d, notice| {
                        d.child(
                            div()
                                .debug_selector(|| "inbox-action-notice".into())
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.))
                                .text_color(content(0.55))
                                .child(notice),
                        )
                    }),
            )
            .when(!self.related_sessions(&item).is_empty(), |d| {
                d.child(
                    div()
                        .min_w_0()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .overflow_hidden()
                        .child(icon("message-multiple", 14.).text_color(content(0.45)))
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(content(0.45))
                                .child("Related threads"),
                        )
                        .children(self.related_sessions(&item).into_iter().map(|session| {
                            action(format!("inbox-session-{}", session.id), session.title)
                                .h(px(24.))
                                .bg(glass::fill())
                                .border_1()
                                .border_color(glass::edge())
                                .max_w(px(256.))
                                .truncate()
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(Event::OpenSession(session.id))
                                }))
                        })),
                )
            })
            .children(tabs)
            .when(!is_pr, |d| d.pb(px(10.)));
        let body = match self.tab {
            Tab::Summary => {
                let side = [self.checks_card(&item, cx), self.reviewers_card(&item)]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>();
                let side = (!side.is_empty()).then_some(side);
                div()
                    .min_h_0()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .gap(px(18.))
                    .pt(px(18.))
                    .px(px(16.))
                    .child(self.summary_view(&item, window, cx))
                    .children(side.map(|cards| {
                        div()
                            .flex_none()
                            .w(px(300.))
                            .pr(px(12.))
                            .flex()
                            .flex_col()
                            .gap(px(12.))
                            .children(cards)
                    }))
                    .into_any_element()
            }
            Tab::Code => self.diff_view(),
            Tab::Checks => self.checks_panel(cx),
        };
        let ask = glass::card_strong()
            .debug_selector(|| "inbox-ask-bar".into())
            .flex()
            .items_center()
            .gap(px(10.))
            .py(px(8.))
            .pr(px(8.))
            .pl(px(14.))
            .rounded(px(14.))
            .child(icon("message-square", 15.).text_color(rgb(0xc0affc)))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(13.))
                    .text_color(content(0.55))
                    .child(format!("Ask about this {}…", kind.to_lowercase())),
            )
            .child(
                div()
                    .flex_none()
                    .h(px(24.))
                    .px(px(9.))
                    .flex()
                    .items_center()
                    .rounded_full()
                    .bg(glass::fill())
                    .border_1()
                    .border_color(glass::edge())
                    .text_size(px(11.5))
                    .text_color(content(0.75))
                    .child(self.ask_agent(&item, cx)),
            )
            .child(
                glass::button("inbox-ask", "Ask", glass::Button::Primary)
                    .h(px(30.))
                    .role(Role::Button)
                    .aria_label("Ask")
                    .debug_selector(|| "inbox-ask".into())
                    .on_click(cx.listener(|this, _, _, cx| this.open_ask(cx))),
            );
        // Files is where a review is written: its bar takes the Ask bar's
        // place there.
        let review_bar = (self.tab == Tab::Code)
            .then(|| self.review_bar_view(cx))
            .flatten();
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .child(title)
            .when_some(self.detail_error.clone(), |d, text| d.child(error(text)))
            .child(body)
            .map(|d| match review_bar {
                Some(bar) => d.child(bar),
                None => d.child(
                    div()
                        .flex_shrink_0()
                        .px(px(28.))
                        .pt(px(10.))
                        .pb(px(22.))
                        .child(ask),
                ),
            })
            .into_any_element()
    }

    /// The agent an Ask about `item` talks to: the new-session choice for
    /// its project, as the Shell starts it.
    fn ask_agent(&self, item: &Item, cx: &App) -> &'static str {
        let fallback = crate::settings::Preferences::default();
        let preferences = cx
            .try_global::<crate::settings::Preferences>()
            .unwrap_or(&fallback);
        crate::installed_providers::new_session_choice(
            preferences,
            (!item.project_path.is_empty()).then_some(item.project_path.as_str()),
            cx,
        )
        .0
        .label()
    }

    /// Who is asked to review, under the checks; hidden when the provider
    /// says nothing about reviews.
    fn reviewers_card(&self, item: &Item) -> Option<AnyElement> {
        if item.kind != "pr" {
            return None;
        }
        let (requested, decision) = reviewers(self.details.as_ref()?)?;
        Some(
            glass::card()
                .debug_selector(|| "inbox-reviewers-card".into())
                .px(px(14.))
                .py(px(12.))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .child(glass::eyebrow("Reviewers"))
                        .children(decision.map(|(label, tone)| {
                            div().ml_auto().child(glass::status(label, tone).h(px(20.)))
                        })),
                )
                .children(requested.into_iter().map(|login| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(super::menu::initials_avatar(&login))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(12.5))
                                .text_color(content(0.62))
                                .child(login),
                        )
                        .child(glass::status("Requested", glass::Tone::Waiting).h(px(20.)))
                }))
                .into_any_element(),
        )
    }

    /// The Summary's side card: how the checks stand and the first few of
    /// them, failures first, for GitHub pull requests whose checks loaded.
    fn checks_card(&self, item: &Item, cx: &mut Context<Self>) -> Option<AnyElement> {
        const SHOWN: usize = 4;
        if item.kind != "pr" || item.provider != "github" {
            return None;
        }
        let checks = self.checks.as_ref()?;
        let rows = card_order(&checks.checks);
        let (overall, tone) = checks_overall(checks);
        let rest = rows.len().saturating_sub(SHOWN);
        let rest_queued = rows
            .iter()
            .skip(SHOWN)
            .all(|check| check.state == "pending");
        Some(
            glass::card()
                .debug_selector(|| "inbox-checks-card".into())
                .overflow_hidden()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .px(px(14.))
                        .pt(px(12.))
                        .pb(px(8.))
                        .child(glass::eyebrow("Checks"))
                        .child(
                            div()
                                .ml_auto()
                                .child(glass::status(overall, tone).h(px(20.))),
                        ),
                )
                .when(rows.is_empty(), |d| {
                    d.child(message("No checks reported").px(px(14.)).pt(px(0.)))
                })
                .children(rows.iter().take(SHOWN).map(|check| {
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .px(px(14.))
                        .py(px(9.))
                        .border_t_1()
                        .border_color(glass::hairline())
                        .text_size(px(12.5))
                        .text_color(content(0.9))
                        .child(
                            glass::status("", super::checks::state_tone(&check.state))
                                .h(px(18.))
                                .px(px(6.)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .child(check.name.clone()),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_size(px(11.5))
                                .text_color(content(0.5))
                                .child(check_time(check)),
                        )
                }))
                .when(rest > 0, |d| {
                    d.child(
                        div()
                            .id("inbox-checks-card-more")
                            .debug_selector(|| "inbox-checks-card-more".into())
                            .role(Role::Button)
                            .aria_label("Show all checks")
                            .cursor_pointer()
                            .px(px(14.))
                            .py(px(9.))
                            .border_t_1()
                            .border_color(glass::hairline())
                            .text_size(px(12.5))
                            .text_color(content(0.62))
                            .hover(|s| s.text_color(content(1.)))
                            .child(if rest_queued {
                                format!("{rest} more queued")
                            } else {
                                format!("{rest} more")
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.set_tab(Tab::Checks, cx))),
                    )
                })
                .into_any_element(),
        )
    }

    fn comment_card(
        &self,
        item: &Item,
        comment: &model::Comment,
        style: CommentStyle,
        width: Pixels,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let nested = matches!(style, CommentStyle::Reply);
        let timeline = matches!(style, CommentStyle::Timeline);
        let has_body = !comment.body.trim().is_empty();
        let has_replies = !nested && !comment.replies.is_empty();
        let author = if comment.author.is_empty() {
            "ghost"
        } else {
            &comment.author
        };
        let reply_id = if item.provider == "linear" {
            comment.id.clone()
        } else if item.provider == "github" {
            comment.thread_id.clone()
        } else {
            String::new()
        };
        let state = comment.state.trim().to_uppercase();
        let review_color = if state == "APPROVED" {
            rgb(0x4ade80).into()
        } else if state == "CHANGES_REQUESTED" {
            rgb(0xf87171).into()
        } else {
            content(0.5)
        };
        // Comments select after the description (block 0), in the order
        // they were loaded, which is the order they are shown.
        let (block, document) = self
            .comments
            .iter()
            .enumerate()
            .find(|(_, (cached, _))| cached.id == comment.id)
            .map(|(ix, (_, document))| (ix + 1, document.clone()))
            .unzip();
        let clamped = timeline
            && !self.expanded_comments.contains(&comment.id)
            && (comment.body.len() > 1200 || comment.body.lines().count() > 12);
        let expandable =
            timeline && (comment.body.len() > 1200 || comment.body.lines().count() > 12);
        let header = div()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(content(0.5))
            .when(!nested, |d| {
                d.px(px(12.))
                    .py(px(if timeline { 6. } else { 8. }))
                    .min_h(px(if timeline { 36. } else { 0. }))
            })
            .when(!nested && (has_body || has_replies), |d| {
                d.border_b_1().border_color(content(0.07))
            })
            .child(
                div()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(content(1.))
                    .child(if timeline {
                        div().child(author.to_string()).into_any_element()
                    } else {
                        avatar::person(
                            &item.provider,
                            author,
                            &comment.author_avatar_url,
                            20.,
                            window,
                            cx,
                        )
                    }),
            )
            .when(!state.is_empty(), |d| {
                d.child("·")
                    .child(div().text_color(review_color).child(review_label(&state)))
            })
            .when(!comment.path.is_empty(), |d| {
                d.child("·").child(div().min_w_0().truncate().child(format!(
                        "{}{}",
                        comment.path,
                        comment
                            .line
                            .map(|line| format!(":{line}"))
                            .unwrap_or_default()
                    )))
            })
            .when(comment.resolved, |d| {
                d.child("·")
                    .child(div().text_color(rgb(0x4ade80)).child("Resolved"))
            })
            .when(!comment.created_at.is_empty(), |d| {
                d.child("·").child(
                    action(
                        format!("inbox-comment-time-{}", comment.id),
                        relative_time(&comment.created_at),
                    )
                    .h_auto()
                    .px_0()
                    .text_color(content(0.5))
                    .on_click({
                        let url = comment.url.clone();
                        move |_, _, cx| {
                            if !url.is_empty() {
                                cx.open_url(&url);
                            }
                        }
                    }),
                )
            })
            .when(!reply_id.is_empty(), |d| {
                let name = author.to_string();
                d.child("·").child(
                    action(format!("inbox-reply-{}", comment.id), "Reply")
                        .h_auto()
                        .px_0()
                        .text_color(content(0.5))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.reply = Some((reply_id.clone(), name.clone()));
                            this.summary_scroll.scroll_to_end();
                            window.focus(&this.comment.focus_handle(cx), cx);
                            cx.notify();
                        })),
                )
            });
        div()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .when(!nested, |d| {
                d.rounded(px(6.))
                    .border_1()
                    .border_color(content(0.1))
                    .bg(content(0.05))
                    .overflow_hidden()
            })
            .child(header)
            .when(has_body, |d| {
                d.child(
                    div()
                        .when(nested, |d| d.mt(px(8.)))
                        .when(!nested, |d| d.px(px(12.)).py(px(10.)))
                        .child(
                            div()
                                .id(format!("inbox-comment-body-{}", comment.id))
                                .w_full()
                                .when(clamped, |d| d.max_h(px(180.)).overflow_hidden())
                                .when_some(document, |d, document| {
                                    d.child(
                                        document::DocumentElement::new(
                                            document,
                                            if nested {
                                                width
                                            } else {
                                                (width - px(26.)).max(px(1.))
                                            },
                                            clamped.then_some(px(180.)),
                                        )
                                        .selectable(&self.text, block.unwrap_or(0)),
                                    )
                                }),
                        )
                        .when(expandable, |d| {
                            let id = comment.id.clone();
                            d.child(
                                action(
                                    format!("inbox-expand-comment-{}", comment.id),
                                    if clamped { "Show more" } else { "Show less" },
                                )
                                .mt(px(6.))
                                .h_auto()
                                .px_0()
                                .text_color(content(0.5))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        if !this.expanded_comments.remove(&id) {
                                            this.expanded_comments.insert(id.clone());
                                        }
                                        this.summary_scroll.remeasure();
                                        cx.notify();
                                    },
                                )),
                            )
                        }),
                )
            })
            .when(has_replies, |d| {
                d.child(
                    div()
                        .border_t_1()
                        .border_color(content(0.07))
                        .px(px(12.))
                        .children(comment.replies.iter().enumerate().map(|(ix, reply)| {
                            div()
                                .py(px(10.))
                                .when(ix > 0, |d| d.border_t_1().border_color(content(0.07)))
                                .child(self.comment_card(
                                    item,
                                    reply,
                                    CommentStyle::Reply,
                                    (width - px(26.)).max(px(1.)),
                                    window,
                                    cx,
                                ))
                        })),
                )
            })
            .into_any_element()
    }

    fn timeline_rail(&self, node: AnyElement, first: bool, last: bool, card: bool) -> Div {
        div()
            .w(px(20.))
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .child(
                div()
                    .w(px(1.))
                    .h(px(if card { 8. } else { 0. }))
                    .flex_shrink_0()
                    .when(!first, |d| d.bg(content(0.1))),
            )
            .child(
                div()
                    .size(px(20.))
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(node),
            )
            .when(!last, |d| {
                d.child(div().w(px(1.)).flex_1().bg(content(0.1)))
            })
    }

    pub(super) fn rebuild_activity(&mut self) {
        self.activity.clear();
        self.activity_revision += 1;
        let Some(thread) = &self.thread else {
            return;
        };
        let timeline = self
            .item()
            .is_some_and(|item| item.kind == "pr" && item.provider == "github");
        if timeline {
            let mut entries = thread
                .comments
                .iter()
                .map(|comment| {
                    (
                        model::timestamp_ms(&comment.created_at),
                        Activity::Comment(std::sync::Arc::new(comment.clone())),
                    )
                })
                .chain(thread.commits.iter().map(|commit| {
                    (
                        model::timestamp_ms(&commit.committed_date),
                        Activity::Commits(vec![commit.clone()]),
                    )
                }))
                .collect::<Vec<_>>();
            entries.sort_by_key(|(at, _)| *at);
            let mut groups: Vec<Activity> = Vec::new();
            for (_, entry) in entries {
                match entry {
                    Activity::Commits(mut commits) => {
                        if let Some(Activity::Commits(previous)) = groups.last_mut()
                            && previous
                                .last()
                                .is_some_and(|c| c.author == commits[0].author)
                        {
                            previous.append(&mut commits);
                            continue;
                        }
                        groups.push(Activity::Commits(commits));
                    }
                    entry => groups.push(entry),
                }
            }
            self.activity = groups;
        } else {
            self.activity = thread
                .comments
                .iter()
                .cloned()
                .map(std::sync::Arc::new)
                .map(Activity::Comment)
                .collect();
        }
        self.summary_scroll.remeasure();
    }

    fn discussion_row(
        &self,
        item: &Item,
        ix: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let entry = self.activity[ix].clone();
        let timeline = item.kind == "pr" && item.provider == "github";
        let width = self.summary_body_width();
        if !timeline {
            let Activity::Comment(comment) = entry else {
                unreachable!()
            };
            return div()
                .pb(px(if ix + 1 == self.activity.len() {
                    0.
                } else {
                    8.
                }))
                .child(self.comment_card(item, &comment, CommentStyle::Plain, width, window, cx))
                .into_any_element();
        }
        let first = ix == 0;
        let last = ix + 1 == self.activity.len();
        let mut activity = Vec::new();
        match entry {
            Activity::Comment(comment) => {
                let event = review_event(&comment);
                let state = comment.state.to_uppercase();
                let color = if state == "APPROVED" {
                    rgb(0x4ade80).into()
                } else if state == "CHANGES_REQUESTED" {
                    rgb(0xf87171).into()
                } else {
                    content(0.45)
                };
                let node = if event {
                    icon(
                        if state == "APPROVED" {
                            "check-circle"
                        } else if state == "CHANGES_REQUESTED" {
                            "circle-x"
                        } else {
                            "message-multiple"
                        },
                        16.,
                    )
                    .text_color(color)
                    .into_any_element()
                } else {
                    avatar::image(
                        &item.provider,
                        &comment.author,
                        &comment.author_avatar_url,
                        20.,
                        window,
                        cx,
                    )
                };
                let body = if event {
                    let verb = match state.as_str() {
                        "APPROVED" => "approved",
                        "CHANGES_REQUESTED" => "requested changes",
                        "DISMISSED" => "had a review dismissed",
                        _ => "reviewed",
                    };
                    timeline_event(&comment.author, verb, &comment.created_at).into_any_element()
                } else {
                    self.comment_card(
                        item,
                        &comment,
                        CommentStyle::Timeline,
                        (width - px(32.)).max(px(1.)),
                        window,
                        cx,
                    )
                };
                activity.push(
                    div()
                        .min_w_0()
                        .flex()
                        .gap(px(12.))
                        .child(self.timeline_rail(node, first, last, !event))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .when(!last, |d| d.pb(px(12.)))
                                .child(body),
                        )
                        .into_any_element(),
                );
            }
            Activity::Commits(commits) => {
                let author = commits[0].author.clone();
                let time = commits
                    .last()
                    .map(|commit| commit.committed_date.clone())
                    .unwrap_or_default();
                let node = avatar::image(&item.provider, &author, "", 20., window, cx);
                activity.push(
                    div()
                        .min_w_0()
                        .flex()
                        .gap(px(12.))
                        .child(self.timeline_rail(node, first, false, false))
                        .child(div().min_w_0().flex_1().pb(px(12.)).child(timeline_event(
                            &author,
                            &format!(
                                "added {}",
                                if commits.len() == 1 {
                                    "a commit".into()
                                } else {
                                    format!("{} commits", commits.len())
                                }
                            ),
                            &time,
                        )))
                        .into_any_element(),
                );
                let commit_count = commits.len();
                for (commit_ix, commit) in commits.into_iter().enumerate() {
                    let final_commit = last && commit_ix + 1 == commit_count;
                    let url = commit.url.clone();
                    activity.push(
                        div()
                            .min_w_0()
                            .flex()
                            .gap(px(12.))
                            .child(
                                div()
                                    .w(px(20.))
                                    .flex_shrink_0()
                                    .flex()
                                    .flex_col()
                                    .items_center()
                                    .child(div().h(px(6.)).w(px(1.)).bg(content(0.1)))
                                    .child(
                                        div()
                                            .size(px(8.))
                                            .rounded_full()
                                            .border_1()
                                            .border_color(content(0.35)),
                                    )
                                    .when(!final_commit, |d| {
                                        d.child(div().w(px(1.)).flex_1().bg(content(0.1)))
                                    }),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .when(!final_commit, |d| d.pb(px(12.)))
                                    .child(
                                        div()
                                            .id(format!("inbox-commit-{}", commit.oid))
                                            .role(Role::Button)
                                            .aria_label("Open commit")
                                            .h(px(20.))
                                            .flex()
                                            .items_center()
                                            .gap(px(12.))
                                            .text_size(px(12.))
                                            .cursor_pointer()
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .flex_1()
                                                    .truncate()
                                                    .text_color(content(0.7))
                                                    .child(commit.message_headline),
                                            )
                                            .child(
                                                div()
                                                    .text_size(px(11.))
                                                    .font_family("monospace")
                                                    .text_color(content(0.35))
                                                    .child(
                                                        commit
                                                            .oid
                                                            .chars()
                                                            .take(7)
                                                            .collect::<String>(),
                                                    ),
                                            )
                                            .on_click(move |_, _, cx| {
                                                if !url.is_empty() {
                                                    cx.open_url(&url);
                                                }
                                            }),
                                    ),
                            )
                            .into_any_element(),
                    );
                }
            }
        }
        div()
            .min_w_0()
            .flex()
            .flex_col()
            .children(activity)
            .into_any_element()
    }

    fn discussion_heading(
        &self,
        item: &Item,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let thread = self.thread.as_ref()?;
        if thread.comments.is_empty() && thread.commits.is_empty() && !thread.truncated {
            return None;
        }
        let timeline = item.kind == "pr" && item.provider == "github";
        let count = thread
            .comments
            .iter()
            .filter(|comment| !(timeline && review_event(comment)))
            .map(|comment| 1 + comment.replies.len())
            .sum::<usize>();
        Some(
            div()
                .w_full()
                .flex_shrink_0()
                .border_t_1()
                .border_color(content(0.07))
                .pt(px(20.))
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .text_size(px(12.))
                        .text_color(content(0.5))
                        .child(div().text_color(content(0.7)).child(if timeline {
                            "Activity".into()
                        } else {
                            format!(
                                "{count} {}",
                                if count == 1 { "comment" } else { "comments" }
                            )
                        }))
                        .when(timeline, |d| {
                            d.child(format!(
                                "{count} {}{}",
                                if count == 1 { "comment" } else { "comments" },
                                if thread.commits.is_empty() {
                                    String::new()
                                } else {
                                    format!(
                                        " · {} {}",
                                        thread.commits.len(),
                                        if thread.commits.len() == 1 {
                                            "commit"
                                        } else {
                                            "commits"
                                        }
                                    )
                                }
                            ))
                        })
                        .when(thread.truncated, |d| {
                            d.child(format!(
                                "Latest comments · more on {}",
                                model::provider_label(&item.provider)
                            ))
                        }),
                )
                .when_some(self.thread_error.clone(), |d, text| d.child(error(text)))
                .into_any_element(),
        )
    }

    fn summary_body_width(&self) -> Pixels {
        (self.summary_width.get().min(px(1024.)) - px(24.)).max(px(1.))
    }

    fn summary_view(&self, item: &Item, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let discussion = self.thread.as_ref().is_some_and(|thread| {
            !thread.comments.is_empty() || !thread.commits.is_empty() || thread.truncated
        });
        let count = 2 + if discussion {
            1 + self.activity.len()
        } else {
            0
        };
        let mut rows = self.summary_rows.borrow_mut();
        if rows.keys.is_empty()
            || rows.revision != self.activity_revision
            || rows.discussion != discussion
        {
            let mut keys: Vec<SharedString> = vec!["intro".into()];
            if discussion {
                keys.push("activity".into());
                keys.extend(self.activity.iter().map(|activity| match activity {
                    Activity::Comment(comment) => format!("comment:{}", comment.id).into(),
                    Activity::Commits(commits) => format!("push:{}", commits[0].oid).into(),
                }));
            }
            keys.push("composer".into());
            // Retain unchanged prefix/suffix rows so refreshes preserve the
            // reader's item and pixel offset, including a focused composer.
            let prefix = rows
                .keys
                .iter()
                .zip(&keys)
                .take_while(|(old, new)| old == new)
                .count();
            let suffix = rows.keys[prefix..]
                .iter()
                .rev()
                .zip(keys[prefix..].iter().rev())
                .take_while(|(old, new)| old == new)
                .count();
            self.summary_scroll.splice_focusable(
                prefix..rows.keys.len() - suffix,
                (prefix..keys.len() - suffix)
                    .map(|ix| (ix + 1 == count).then(|| self.comment.focus_handle(cx))),
            );
            *rows = SummaryRows {
                revision: self.activity_revision,
                discussion,
                keys,
            };
        }
        drop(rows);
        let width = self.summary_width.clone();
        let entity = cx.entity();
        let item = item.clone();
        div()
            .id(format!("inbox-summary-{}", item.key()))
            .relative()
            .min_h_0()
            .min_w_0()
            .flex_1()
            .child(
                canvas(
                    move |bounds, _, _| {
                        width.set(bounds.size.width);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full(),
            )
            .child(
                list(self.summary_scroll.clone(), move |ix, window, cx| {
                    entity.update(cx, |this, cx| {
                        this.summary_row(&item, ix, count, discussion, window, cx)
                    })
                })
                .size_full(),
            )
            .into_any_element()
    }

    fn summary_row(
        &self,
        item: &Item,
        ix: usize,
        count: usize,
        discussion: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let can_post = !self.posting && !self.comment.read(cx).content.trim().is_empty();
        let body = if ix == 0 {
            glass::card()
                .debug_selector(|| "inbox-description-card".into())
                .mb(px(20.))
                .px(px(20.))
                .py(px(18.))
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(glass::eyebrow("Description"))
                .when(!item.labels.is_empty(), |d| {
                    d.child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(4.))
                            .children(item.labels.iter().map(|label| label_badge(label, false))),
                    )
                })
                .when(self.detail_loading && self.details.is_none(), |d| {
                    d.child(
                        loading("Loading description…")
                            .debug_selector(|| "inbox-loading-description".into()),
                    )
                })
                .when(
                    self.details
                        .as_ref()
                        .is_some_and(|d| d.body.trim().is_empty()),
                    |d| {
                        d.child(
                            div()
                                .text_size(px(13.))
                                .text_color(content(0.45))
                                .child("No description"),
                        )
                    },
                )
                .when_some(self.document.clone(), |d, document| {
                    d.child(
                        div().w_full().flex_shrink_0().child(
                            document::DocumentElement::new(
                                document,
                                // The description card's padding and border.
                                (self.summary_body_width() - px(42.)).max(px(1.)),
                                None,
                            )
                            .selectable(&self.text, 0),
                        ),
                    )
                })
                .when(self.thread.is_none(), |d| {
                    d.when_some(self.thread_error.clone(), |d, text| d.child(error(text)))
                })
                .into_any_element()
        } else if ix + 1 == count {
            div()
                .pb(px(20.))
                .child(
                    div()
                        .w_full()
                        .flex_shrink_0()
                        .border_t_1()
                        .border_color(content(0.07))
                        .pt(px(20.))
                        .flex()
                        .flex_col()
                        .gap(px(8.))
                        .when_some(self.reply.clone(), |d, (_, author)| {
                            d.child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(8.))
                                    .text_size(px(12.))
                                    .text_color(content(0.5))
                                    .child(
                                        div()
                                            .min_w_0()
                                            .truncate()
                                            .child(format!("Replying to {author}")),
                                    )
                                    .child(
                                        icon_button("inbox-cancel-reply", "Cancel reply", "close")
                                            .size(px(20.))
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.reply = None;
                                                this.post_error = None;
                                                cx.notify();
                                            })),
                                    ),
                            )
                        })
                        .child(
                            div()
                                .rounded(px(6.))
                                .border_1()
                                .border_color(content(0.1))
                                .bg(content(0.05))
                                .flex()
                                .flex_col()
                                .child(
                                    div()
                                        .px(px(12.))
                                        .py(px(8.))
                                        .when(self.posting, |d| d.opacity(0.4))
                                        .child(self.comment.clone()),
                                )
                                .child(
                                    div().px(px(8.)).pb(px(8.)).flex().justify_end().child(
                                        action(
                                            "inbox-post-comment",
                                            if self.posting {
                                                "Posting..."
                                            } else if self.reply.is_some() {
                                                "Reply"
                                            } else {
                                                "Comment"
                                            },
                                        )
                                        .px(px(12.))
                                        .bg(content(1.))
                                        .text_color(shell_fill())
                                        .when(!can_post, |d| d.opacity(0.4))
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.post_comment(cx)),
                                        ),
                                    ),
                                ),
                        )
                        .when_some(self.post_error.clone(), |d, text| d.child(error(text))),
                )
                .into_any_element()
        } else if discussion && ix == 1 {
            div()
                .pb(px(if self.activity.is_empty() { 20. } else { 12. }))
                .children(self.discussion_heading(item, window, cx))
                .into_any_element()
        } else {
            div()
                .pb(px(if ix + 2 == count { 20. } else { 0. }))
                .child(self.discussion_row(item, ix - 2, window, cx))
                .into_any_element()
        };
        div()
            .id(("inbox-summary-row", ix))
            .w_full()
            .px(px(12.))
            .child(body)
            .into_any_element()
    }

    fn diff_view(&self) -> AnyElement {
        div()
            .min_h_0()
            .flex_1()
            .flex()
            .flex_col()
            .when(self.extra_loading && self.diff.is_none(), |d| {
                d.child(loading_panel("inbox-loading-changes", "Loading changes…"))
            })
            .when_some(self.extra_error.clone(), |d, text| d.child(error(text)))
            .when(self.diff.is_some(), |d| {
                d.child(div().flex_1().min_h_0().child(self.diff_view.clone()))
            })
            .when(self.diff.as_ref().is_some_and(|diff| diff.truncated), |d| {
                d.child(message(
                    "This diff is truncated. Open the pull request to see all changes.",
                ))
            })
            .into_any_element()
    }

    /// Settings' team list: the same choice as the Inbox's Teams filter.
    pub(crate) fn toggle_linear_team(&mut self, id: String, cx: &mut Context<Self>) {
        toggle(&mut self.state.feed.hidden_linear_teams, id);
        self.filters_changed(true, cx);
    }
    fn filters_changed(&mut self, refetch: bool, cx: &mut Context<Self>) {
        self.save();
        if let Some(item) = self.visible_items(cx).first() {
            self.select(item.key(), cx);
        } else {
            self.selected = None;
        }
        if refetch {
            self.refresh(cx);
        }
        cx.notify();
    }
    fn connection_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        popover(
            "inbox-connections-menu",
            Anchor::Slot(self.connection_anchor.clone()),
        )
        .width(272.)
        .padding(4.)
        .on_dismiss(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.connections_open = false;
                cx.notify();
            });
        })
        .child(caption("CONNECTIONS"))
        .children(model::PROVIDERS.into_iter().map(|provider| {
            let connected = self
                .snapshot
                .connections
                .get(provider)
                .is_some_and(|s| s.connected);
            let label = format!(
                "{} {}",
                if connected { "Manage" } else { "Connect" },
                model::provider_label(provider)
            );
            div()
                .id(SharedString::from(format!("inbox-connect-{provider}")))
                .role(Role::Button)
                .aria_label(label.clone())
                .w_full()
                .h(px(28.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(6.))
                .rounded(px(6.))
                .text_size(px(12.))
                .text_color(content(0.7))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.07)).text_color(content(1.)))
                .child(provider_mark(provider))
                .child(label)
                .on_click(
                    cx.listener(move |this, _, _, cx| this.open_connection(provider.into(), cx)),
                )
        }))
        .into_any_element()
    }

    pub(super) fn connection_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let provider = self.connect_provider.clone().unwrap_or_default();
        let connected = self
            .snapshot
            .connections
            .get(&provider)
            .is_some_and(|s| s.connected);
        let title = format!(
            "{} {}",
            if connected { "Manage" } else { "Connect" },
            model::provider_label(&provider)
        );
        let hint = match provider.as_str() {
            "github" => {
                "Install GitHub CLI, then run gh auth login in a terminal. Check again when signed in."
            }
            "linear" => "Create a personal API key in Linear → Settings → Security & Access.",
            "jira" => "Connect your Jira Cloud site with your Atlassian email and an API token.",
            "plane" => {
                "Connect Plane Cloud or a self-hosted instance with a workspace slug and a personal API key. Plane links tickets to sessions; it lists no issues here."
            }
            "gitlab" => {
                "Connect GitLab.com or your GitLab instance with a personal access token with API access."
            }
            _ => {
                "Connect your ADO organization using a personal access token with Boards and Repos read & write access."
            }
        };
        div()
            .debug_selector(|| "inbox-connect-dialog".into())
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .child(crate::modal::scrim("inbox-connect-scrim"))
            .child(crate::modal::enter(
                "inbox-connect-card",
                div()
                    .relative()
                    .w(px(440.))
                    .max_w_full()
                    .p(px(24.))
                    .rounded(px(crate::modal::RADIUS))
                    .border_1()
                    .border_color(content(crate::modal::BORDER))
                    .child(crate::modal::surface(crate::modal::RADIUS - 1.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(provider_mark(&provider))
                            .child(
                                div()
                                    .flex_1()
                                    .text_size(px(18.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(content(1.))
                                    .child(title),
                            )
                            .child(
                                icon_button("inbox-close-connect", "Close connection", "close")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if !this.connecting {
                                            this.connect_provider = None;
                                            this.connect_token.update(cx, |input, cx| {
                                                input.set_text(String::new(), cx)
                                            });
                                            cx.notify();
                                        }
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(19.))
                            .text_color(content(0.5))
                            .child(hint),
                    )
                    .when(provider != "github" && provider != "linear", |d| {
                        d.child(
                            div()
                                .px(px(10.))
                                .py(px(8.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(content(0.1))
                                .child(self.connect_url.clone()),
                        )
                    })
                    .when(provider == "jira", |d| {
                        d.child(
                            div()
                                .px(px(10.))
                                .py(px(8.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(content(0.1))
                                .child(self.connect_email.clone()),
                        )
                    })
                    .when(provider == "plane", |d| {
                        d.child(
                            div()
                                .px(px(10.))
                                .py(px(8.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(content(0.1))
                                .child(self.connect_workspace.clone()),
                        )
                    })
                    .when(provider != "github", |d| {
                        d.child(
                            div()
                                .px(px(10.))
                                .py(px(8.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(content(0.1))
                                .child(self.connect_token.clone()),
                        )
                    })
                    .when_some(self.connect_error.clone(), |d, text| d.child(error(text)))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .when(connected && provider != "github", |d| {
                                d.child(
                                    action("inbox-disconnect", "Disconnect")
                                        .text_color(rgb(0xf87171))
                                        .on_click(
                                            cx.listener(|this, _, _, cx| this.connect(true, cx)),
                                        ),
                                )
                            })
                            .when(provider == "github", |d| {
                                d.child(
                                    action("inbox-gh-guide", "Installation guide").on_click(
                                        |_, _, cx| cx.open_url("https://cli.github.com/"),
                                    ),
                                )
                            })
                            .child(div().flex_1())
                            .child(
                                crate::modal::disabled(
                                    crate::modal::primary_button(
                                        "inbox-connect-submit",
                                        if self.connecting {
                                            "Connecting…"
                                        } else if provider == "github" {
                                            "Check again"
                                        } else {
                                            "Connect"
                                        },
                                    ),
                                    self.connecting,
                                )
                                .on_click(cx.listener(|this, _, _, cx| this.connect(false, cx))),
                            ),
                    ),
            ))
            .into_any_element()
    }

    fn project_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        popover(
            "inbox-project-menu",
            Anchor::Slot(self.project_anchor.clone()),
        )
        .width(240.)
        .padding(4.)
        .on_dismiss(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.project_picker = false;
                cx.notify();
            });
        })
        .children(self.projects.iter().enumerate().map(|(ix, project)| {
            let path = project.clone();
            menu_row(
                format!("inbox-start-project-{ix}"),
                self.project_label(&project.to_string_lossy()),
                *project == self.start_project,
            )
            .child(self.project_mark(&project.to_string_lossy(), 14.))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.start_project = path.clone();
                this.project_picker = false;
                cx.notify();
            }))
        }))
        .into_any_element()
    }

    fn action_confirmation(&self, cx: &mut Context<Self>) -> AnyElement {
        let action_name = self.confirm_action.clone().unwrap_or_default();
        let details = self.details.clone().unwrap_or_default();
        let source = if details.head_ref_name.is_empty() {
            "this branch".into()
        } else {
            format!("“{}”", details.head_ref_name)
        };
        let destination = if details.base_ref_name.is_empty() {
            "the base branch".into()
        } else {
            format!("“{}”", details.base_ref_name)
        };
        let (title, detail, label, progress) = match action_name.as_str() {
            "merge" => (
                "Merge this pull request?",
                format!(
                    "Every commit from {source} will be added to {destination} with a merge commit."
                ),
                "Merge pull request",
                "Merging…",
            ),
            "squash" => (
                "Squash and merge?",
                format!(
                    "The commits from {source} will be combined into one commit on {destination}."
                ),
                "Squash and merge",
                "Merging…",
            ),
            "rebase" => (
                "Rebase and merge?",
                format!(
                    "The commits from {source} will be rebased individually onto {destination}."
                ),
                "Rebase and merge",
                "Merging…",
            ),
            "close" => (
                "Close this pull request?",
                "The pull request will close without merging. You can reopen it later.".into(),
                "Close pull request",
                "Closing…",
            ),
            "reopen" => (
                "Reopen this pull request?",
                "The pull request will return to the open state.".into(),
                "Reopen pull request",
                "Reopening…",
            ),
            "ready" => (
                "Mark as ready for review?",
                "Reviewers will see that this pull request is ready for feedback.".into(),
                "Ready for review",
                "Updating…",
            ),
            _ => (
                "Convert to draft?",
                "Reviewers will see that this pull request is not ready to merge.".into(),
                "Convert to draft",
                "Converting…",
            ),
        };
        let anchor = self.action_anchor.clone();
        let weak = cx.entity().downgrade();
        popover("inbox-pr-confirmation", Anchor::Slot(anchor))
            .align(Align::End)
            .width(320.)
            .gap(5.)
            .padding(12.)
            .on_dismiss(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    if !this.acting {
                        this.confirm_action = None;
                        this.action_error = None;
                        cx.notify();
                    }
                });
            })
            .child(
                div()
                    .id("inbox-pr-confirmation-dialog")
                    .role(Role::Dialog)
                    .aria_label(title)
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(1.))
                            .child(title),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .text_color(content(0.55))
                            .child(detail),
                    ),
            )
            .when_some(self.action_error.clone(), |d, text| {
                d.child(
                    div()
                        .mt(px(8.))
                        .text_size(px(11.))
                        .text_color(rgb(0xf87171))
                        .child(text),
                )
            })
            .child(
                div()
                    .mt(px(12.))
                    .flex()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        action("inbox-action-cancel", "Cancel")
                            .when(self.acting, |d| d.opacity(0.4))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if !this.acting {
                                    this.confirm_action = None;
                                    this.action_error = None;
                                    cx.notify();
                                }
                            })),
                    )
                    .child(
                        action(
                            "inbox-action-confirm",
                            if self.acting { progress } else { label },
                        )
                        .font_weight(FontWeight::MEDIUM)
                        .bg(if action_name == "close" {
                            Hsla::from(rgb(0xf43f5e)).opacity(0.2)
                        } else if matches!(action_name.as_str(), "merge" | "squash" | "rebase") {
                            Hsla::from(rgb(0x10b981)).opacity(0.2)
                        } else {
                            content(1.)
                        })
                        .text_color(if action_name == "close" {
                            rgb(0xfda4af).into()
                        } else if matches!(action_name.as_str(), "merge" | "squash" | "rebase") {
                            rgb(0x6ee7b7).into()
                        } else {
                            shell_fill()
                        })
                        .when(self.acting, |d| d.opacity(0.6))
                        .on_click(cx.listener(|this, _, _, cx| this.run_action(cx))),
                    ),
            )
            .into_any_element()
    }
}

impl gpui::Render for Inbox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.focus_comment {
            self.focus_comment = false;
            window.focus(&self.comment.focus_handle(cx), cx);
        }
        if self.focus_review {
            self.focus_review = false;
            window.focus(&self.review_body.focus_handle(cx), cx);
        }
        self.sync_arrivals(cx);
        let list = self.list_view(cx);
        let detail = self.detail_view(window, cx);
        let ask = self.ask_panel(window, cx);
        div()
            .id("inbox-view")
            .role(Role::Group)
            .aria_label("Pull requests")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    if this.connect_provider.is_some() && !this.connecting {
                        this.connect_provider = None;
                    } else if this.confirm_action.is_some() && !this.acting {
                        this.confirm_action = None;
                    } else if this.filters_open
                        || this.add_repo_open
                        || this.connections_open
                        || this.project_picker
                        || this.action_menu
                    {
                        this.filters_open = false;
                        this.add_repo_open = false;
                        this.connections_open = false;
                        this.project_picker = false;
                        this.action_menu = false;
                    } else {
                        cx.emit(Event::Close);
                    }
                    cx.stop_propagation();
                    cx.notify();
                }
            }))
            .on_mouse_move(
                cx.listener(|this, event: &gpui::MouseMoveEvent, window, cx| {
                    if let Some((start, width)) = this.resize_start {
                        let max = (f32::from(window.viewport_size().width) * 0.5).clamp(300., 480.);
                        this.state.feed.list_width =
                            (width + f32::from(event.position.x) - start).clamp(300., max);
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.resize_start.take().is_some() {
                        this.save();
                        cx.notify();
                    }
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.resize_start.take().is_some() {
                        this.save();
                        cx.notify();
                    }
                }),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .flex()
                    .child(list)
                    .child(
                        self.text
                            .region("inbox-detail-text", detail)
                            .flex_1()
                            .min_w_0()
                            .min_h_0(),
                    )
                    .children(ask),
            )
            .when_some(self.error.clone(), |d, text| d.child(error(text)))
            .when(self.filters_open, |d| d.child(self.git_filters_menu(cx)))
            .when(self.add_repo_open, |d| d.child(self.add_repo_menu(cx)))
            .when(self.connections_open, |d| d.child(self.connection_menu(cx)))
            .when(self.project_picker, |d| d.child(self.project_menu(cx)))
            .when(self.action_menu, |d| d.child(self.action_menu_view(cx)))
            .when(
                self.connect_provider.is_some() && !self.connect_in_settings,
                |d| {
                    d.child(crate::modal::window_overlay(
                        window,
                        self.connection_dialog(cx),
                    ))
                },
            )
            .when(self.confirm_action.is_some(), |d| {
                d.child(self.action_confirmation(cx))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        AssetSource, Bounds, Render, TestAppContext, WindowBounds, WindowHandle, WindowOptions,
        point, size,
    };
    use std::sync::{Arc, Mutex};

    struct Fixture {
        inbox: Entity<Inbox>,
        item: Item,
    }
    impl Render for Fixture {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.inbox.update(cx, |inbox, cx| {
                div()
                    .size_full()
                    .flex()
                    .flex_col()
                    .child(inbox.summary_view(&self.item, window, cx))
            })
        }
    }
    fn fixture(cx: &mut TestAppContext) -> WindowHandle<Fixture> {
        let window = cx.update(|cx| cx.open_window(WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(point(px(0.), px(0.)), size(px(1100.), px(700.))))),
            ..Default::default()
        }, |_, cx| cx.new(|cx| {
            let item = Item { provider: "github".into(), kind: "pr".into(), state: "open".into(), number: 7, ..Default::default() };
            let inbox = cx.new(|cx| {
                let store = Arc::new(Mutex::new(gespan_core::store::Store::open(std::path::Path::new(":memory:")).unwrap()));
                let mut view = Inbox::new_inner(store, vec![], PathBuf::from("/private/tmp/inbox-regression"), false, cx);
                view.snapshot.items = vec![item.clone()]; view.selected = Some(item.key());
                view.details = Some(Details { body: "Description".into(), ..Default::default() });
                let comments = (0..100).map(|ix| model::Comment {
                    id: format!("comment-{ix}"), body: (0..30).map(|line| format!("### Finding {line}\n\nReview **retry handling** in `src/main.rs` before returning.\n\n")).collect(),
                    created_at: format!("2026-10-22T08:{:02}:00Z", ix % 60), ..Default::default()
                }).collect::<Vec<_>>();
                view.comments = comments.iter().map(|comment| (comment.clone(), cx.new(|cx| document::InboxDocument::new(comment.body.clone(), PathBuf::from("/repo"), cx)))).collect();
                view.thread = Some(Thread { comments, ..Default::default() });
                view.rebuild_activity(); view
            });
            Fixture { inbox, item }
        })).unwrap());
        draw(window, cx);
        window
    }
    fn draw(window: WindowHandle<Fixture>, cx: &mut TestAppContext) {
        window.update(cx, |_, _, cx| cx.notify()).unwrap();
        cx.refresh().unwrap();
        cx.run_until_parked();
    }
    #[gpui::test]
    fn projects_are_labelled_by_their_appearance(cx: &mut TestAppContext) {
        let window = fixture(cx);
        window
            .update(cx, |fixture, _, cx| {
                fixture.inbox.update(cx, |inbox, _| {
                    assert_eq!(inbox.project_label("/work/other"), "other");
                    inbox.appearances.insert(
                        "/work/other".into(),
                        gespan_core::project_appearance::ProjectAppearance {
                            name: "Other app".into(),
                            ..Default::default()
                        },
                    );
                    assert_eq!(inbox.project_label("/work/other"), "Other app");
                });
            })
            .unwrap();
    }
    /// Git lists every open project's pull requests, so switching the active
    /// project keeps the selection and both projects' rows.
    #[gpui::test]
    fn switching_the_active_project_keeps_every_projects_pull_requests(cx: &mut TestAppContext) {
        let window = fixture(cx);
        window
            .update(cx, |fixture, _, cx| {
                fixture.inbox.update(cx, |inbox, cx| {
                    let (a, b) = (PathBuf::from("/work/a"), PathBuf::from("/work/b"));
                    let item = |number, project: &PathBuf| Item {
                        provider: "github".into(),
                        kind: "pr".into(),
                        state: "open".into(),
                        repo: format!("acme/app-{number}"),
                        number,
                        project_path: project.to_string_lossy().into_owned(),
                        ..Default::default()
                    };
                    // Keep the list fetch queued instead of reaching real providers.
                    inbox.loading = true;
                    inbox.state.prefs.set_view("all");
                    inbox.snapshot.items = vec![item(1, &a), item(2, &b)];
                    inbox.set_context(vec![a.clone(), b.clone()], a.clone(), cx);
                    inbox.selected = Some(item(1, &a).key());
                    let scope = inbox.fetch_scope();

                    inbox.set_context(vec![a.clone(), b.clone()], b.clone(), cx);

                    assert_eq!(inbox.selected, Some(item(1, &a).key()));
                    assert_ne!(inbox.fetch_scope(), scope, "a fetch made for A is stale");
                    let mut shown: Vec<String> =
                        inbox.visible_items(cx).iter().map(Item::key).collect();
                    shown.sort();
                    assert_eq!(shown, vec![item(1, &a).key(), item(2, &b).key()]);
                });
            })
            .unwrap();
    }
    #[gpui::test]
    fn long_discussion_materializes_only_viewport_comments_and_reply_focus_reaches_composer(
        cx: &mut TestAppContext,
    ) {
        let window = fixture(cx);
        window
            .update(cx, |fixture, _, cx| {
                fixture.inbox.update(cx, |inbox, cx| {
                    let measured = inbox
                        .comments
                        .iter()
                        .filter(|(_, doc)| doc.read(cx).measured_rows() > 0)
                        .count();
                    assert!(
                        measured > 0 && measured < 10,
                        "only viewport/overscan cards should be laid out, got {measured}"
                    );
                    assert_eq!(inbox.summary_scroll.item_count(), 103);
                    inbox.summary_scroll.scroll_by(px(2500.));
                })
            })
            .unwrap();
        draw(window, cx);
        window
            .update(cx, |fixture, window, cx| {
                fixture.inbox.update(cx, |inbox, cx| {
                    assert!(inbox.summary_scroll.logical_scroll_top().item_ix > 3);
                    inbox.reply = Some(("thread-1".into(), "Reviewer".into()));
                    inbox.summary_scroll.remeasure_items(102..103);
                    inbox.summary_scroll.scroll_to_end();
                    window.focus(&inbox.comment.focus_handle(cx), cx);
                })
            })
            .unwrap();
        draw(window, cx);
        window
            .update(cx, |fixture, window, cx| {
                fixture.inbox.update(cx, |inbox, cx| {
                    assert!(inbox.summary_scroll.logical_scroll_top().item_ix > 90);
                    assert!(inbox.comment.focus_handle(cx).is_focused(window));
                    inbox
                        .comment
                        .update(cx, |input, cx| input.set_text("A reply\n".repeat(8), cx));
                })
            })
            .unwrap();
        draw(window, cx);
        window
            .update(cx, |fixture, window, cx| {
                fixture.inbox.update(cx, |inbox, cx| {
                    assert!(inbox.comment.focus_handle(cx).is_focused(window));
                    assert_eq!(inbox.summary_scroll.item_count(), 103);
                })
            })
            .unwrap();
    }
    #[gpui::test]
    fn expanding_comment_invalidates_its_height_and_preserves_timeline_order(
        cx: &mut TestAppContext,
    ) {
        let window = fixture(cx);
        let (id, previous) = window
            .update(cx, |fixture, _, cx| {
                fixture.inbox.update(cx, |inbox, cx| {
                    let Activity::Comment(comment) = &inbox.activity[0] else {
                        panic!()
                    };
                    let id = comment.id.clone();
                    let doc = &inbox
                        .comments
                        .iter()
                        .find(|(comment, _)| comment.id == id)
                        .unwrap()
                        .1;
                    let previous = doc.read(cx).measured_rows();
                    inbox.expanded_comments.insert(id.clone());
                    inbox.summary_scroll.remeasure();
                    (id, previous)
                })
            })
            .unwrap();
        draw(window, cx);
        window
            .update(cx, |fixture, _, cx| {
                fixture.inbox.update(cx, |inbox, cx| {
                    let doc = &inbox
                        .comments
                        .iter()
                        .find(|(comment, _)| comment.id == id)
                        .unwrap()
                        .1;
                    assert!(doc.read(cx).measured_rows() > previous);
                    let Activity::Comment(first) = &inbox.activity[0] else {
                        panic!()
                    };
                    assert_eq!(first.id, id);
                    assert_eq!(inbox.summary_scroll.logical_scroll_top().item_ix, 0);
                })
            })
            .unwrap();
    }
    #[gpui::test]
    fn arriving_comment_preserves_the_readers_item_and_offset(cx: &mut TestAppContext) {
        let window = fixture(cx);
        window
            .update(cx, |fixture, _, cx| {
                fixture
                    .inbox
                    .update(cx, |inbox, _| inbox.summary_scroll.scroll_by(px(2500.)))
            })
            .unwrap();
        draw(window, cx);
        let before = window
            .update(cx, |fixture, _, cx| {
                fixture.inbox.update(cx, |inbox, cx| {
                    let before = inbox.summary_scroll.logical_scroll_top();
                    let comment = model::Comment {
                        id: "new-comment".into(),
                        body: "A new reply".into(),
                        created_at: "2026-10-22T09:00:00Z".into(),
                        ..Default::default()
                    };
                    let doc = cx.new(|cx| {
                        document::InboxDocument::new(
                            comment.body.clone(),
                            PathBuf::from("/repo"),
                            cx,
                        )
                    });
                    inbox.comments.push((comment.clone(), doc));
                    inbox.thread.as_mut().unwrap().comments.push(comment);
                    inbox.rebuild_activity();
                    before
                })
            })
            .unwrap();
        draw(window, cx);
        window
            .update(cx, |fixture, _, cx| {
                fixture.inbox.update(cx, |inbox, _| {
                    let after = inbox.summary_scroll.logical_scroll_top();
                    assert_eq!(after.item_ix, before.item_ix);
                    assert!((after.offset_in_item - before.offset_in_item).abs() < px(1.));
                    assert_eq!(inbox.summary_scroll.item_count(), 104);
                })
            })
            .unwrap();
    }

    #[test]
    fn toolbar_icons_have_explicit_ink_and_the_original_distinct_assets() {
        for (name, ink) in [
            ("git-merge", shell_fill()),
            ("git-pull-request-draft", content(0.7)),
            ("git-pull-request-closed", content(0.7)),
            ("external-link", content(0.7)),
        ] {
            let mut icon = action_icon(name, ink);
            assert_eq!(icon.style().text.color, Some(ink));
            let bytes = crate::assets::Assets.load(name).unwrap().unwrap();
            assert!(std::str::from_utf8(&bytes).unwrap().contains("<svg"));
        }
    }

    #[gpui::test]
    fn plane_connection_row_connects_and_disconnects(cx: &mut TestAppContext) {
        use gespan_tracker_secrets::{self as secrets, Tracker, testing};
        testing::use_memory_keychain();
        let dir = tempfile::tempdir().unwrap();
        secrets::write(
            Tracker::Plane,
            dir.path(),
            r#"{"base_url":"https://plane.example.com","workspace":"acme","api_key":"k"}"#,
        )
        .unwrap();
        let cwd = dir.path().to_path_buf();
        let inbox = cx.new(|cx| {
            let store = Arc::new(Mutex::new(
                gespan_core::store::Store::open(std::path::Path::new(":memory:")).unwrap(),
            ));
            Inbox::new_inner(store, vec![], cwd, false, cx)
        });
        inbox.update(cx, |inbox, _| {
            // The disconnect is read back through a refresh of this keychain.
            inbox.live_refresh = true;
            let status = inbox.service.status("plane").unwrap();
            inbox.snapshot.connections.insert("plane".into(), status);
        });
        inbox.update(cx, |inbox, cx| {
            let (connected, label) = inbox.connection_statuses()["plane"].clone();
            assert!(connected);
            assert_eq!(label, "https://plane.example.com");
            inbox.open_connection("plane".into(), cx);
            assert_eq!(
                inbox.connect_url.read(cx).content.as_ref(),
                "https://plane.example.com"
            );
            assert_eq!(inbox.connect_workspace.read(cx).content.as_ref(), "acme");
            inbox.connect(false, cx);
            assert_eq!(
                inbox.connect_error.as_deref(),
                Some("Enter a Plane API key")
            );
            inbox.connect(true, cx);
        });
        cx.run_until_parked();
        inbox.update(cx, |inbox, _| {
            assert!(inbox.connect_provider.is_none());
            assert!(!inbox.snapshot.connections["plane"].connected);
            assert!(!inbox.service.status("plane").unwrap().connected);
        });
        assert!(secrets::read(Tracker::Plane, dir.path()).unwrap().is_none());
    }
}

#[cfg(test)]
mod glass_view_tests {
    use super::*;
    use gpui::{
        Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions,
        point, size,
    };

    fn pr(repo: &str, number: i64, title: &str) -> Item {
        Item {
            provider: "github".into(),
            kind: "pr".into(),
            repo: repo.into(),
            number,
            state: "open".into(),
            title: title.into(),
            updated_at: "2026-11-12T10:00:00Z".into(),
            viewer: "me".into(),
            ..Default::default()
        }
    }
    fn check(name: &str, state: &str) -> model::Check {
        model::Check {
            name: name.into(),
            state: state.into(),
            ..Default::default()
        }
    }
    fn checks(states: &[&str]) -> Checks {
        Checks {
            head_oid: "a".repeat(40),
            checks: states
                .iter()
                .enumerate()
                .map(|(ix, state)| check(&format!("job {ix}"), state))
                .collect(),
        }
    }

    #[test]
    fn reviewers_show_who_is_asked_and_the_decision_and_hide_without_either() {
        assert!(reviewers(&Details::default()).is_none());
        let asked = Details {
            review_requests: vec!["cubic".into(), " ".into(), "Core".into()],
            ..Default::default()
        };
        assert_eq!(
            reviewers(&asked),
            Some((vec!["cubic".to_string(), "Core".to_string()], None))
        );
        let tone = |decision: &str| {
            reviewers(&Details {
                review_decision: decision.into(),
                ..Default::default()
            })
            .and_then(|(_, decision)| decision)
        };
        assert_eq!(tone("APPROVED"), Some(("Approved".into(), glass::Tone::Ok)));
        assert_eq!(
            tone("CHANGES_REQUESTED"),
            Some(("Changes requested".into(), glass::Tone::Bad))
        );
        assert_eq!(
            tone("REVIEW_REQUIRED"),
            Some(("Review required".into(), glass::Tone::Waiting))
        );
    }

    #[test]
    fn the_checks_card_and_meta_line_read_like_the_design() {
        assert_eq!(
            checks_overall(&checks(&["pass", "pending"])),
            ("Running".to_string(), glass::Tone::Running)
        );
        assert_eq!(
            checks_overall(&checks(&["pass", "fail", "fail"])),
            ("2 failed".to_string(), glass::Tone::Bad)
        );
        assert_eq!(
            checks_overall(&checks(&["pass", "skipping"])),
            ("Passing".to_string(), glass::Tone::Ok)
        );
        assert_eq!(check_time(&check("e2e", "pending")), "running");
        assert_eq!(check_time(&check("lint", "fail")), "failed");
        let ago = (chrono::Utc::now() - chrono::Duration::minutes(14)).to_rfc3339();
        assert_eq!(opened_line("vasanth", &ago), "vasanth · opened 14m ago");
        // Without an author, or a time, the line keeps what it knows.
        assert_eq!(opened_line("", &ago), "opened 14m ago");
        assert_eq!(opened_line("vasanth", ""), "vasanth");
    }

    #[test]
    fn row_ages_use_the_largest_whole_unit() {
        let ago = |minutes: i64| {
            let at = chrono::Utc::now() - chrono::Duration::minutes(minutes);
            short_age(&at.to_rfc3339())
        };
        assert_eq!(ago(0), "now");
        assert_eq!(ago(14), "14m");
        assert_eq!(ago(59), "59m");
        assert_eq!(ago(60), "1h");
        assert_eq!(ago(60 * 24 - 1), "23h");
        assert_eq!(ago(60 * 24), "1d");
        assert_eq!(ago(60 * 24 * 7), "1w");
        assert_eq!(ago(60 * 24 * 400), "1y");
        // A clock ahead of ours reads as now, and a missing time as nothing.
        assert_eq!(ago(-30), "now");
        assert_eq!(short_age(""), "");
    }

    fn open(
        cx: &mut TestAppContext,
        setup: impl FnOnce(&mut Inbox) + 'static,
    ) -> WindowHandle<Inbox> {
        cx.update(|cx| cx.set_global(crate::settings::Preferences::default()));
        let store = gespan_core::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let store = std::sync::Arc::new(std::sync::Mutex::new(store));
        let window = cx.update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1300.), px(800.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let mut inbox =
                            Inbox::new_inner(store, vec![], "/missing/inbox".into(), false, cx);
                        setup(&mut inbox);
                        inbox
                    })
                },
            )
            .unwrap()
        });
        cx.run_until_parked();
        window
    }
    fn visual(window: WindowHandle<Inbox>, cx: &mut TestAppContext) -> VisualTestContext {
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        visual.refresh().unwrap();
        visual
    }
    fn connect(inbox: &mut Inbox) {
        inbox.snapshot.connections.insert(
            "github".into(),
            model::Status {
                connected: true,
                ..Default::default()
            },
        );
        // A project, so the list is not asking for one.
        inbox.projects = vec![PathBuf::from("/missing/inbox")];
    }

    #[gpui::test]
    fn without_a_connection_both_sides_show_their_empty_state(cx: &mut TestAppContext) {
        let window = open(cx, |_| {});
        let mut visual = visual(window, cx);
        assert!(visual.debug_bounds("inbox-empty-list").is_some());
        assert!(visual.debug_bounds("inbox-empty-detail").is_some());
    }

    #[gpui::test]
    fn a_group_label_keeps_its_height_above_its_first_row(cx: &mut TestAppContext) {
        // A list long enough to scroll squeezed its labels: the rows do not
        // shrink, so the label gave up its height and the first row's plate
        // was drawn over the text. Short lists, selected or not, narrow or
        // wide, must keep the label whole too.
        for (count, select, width) in [
            (40, true, 300.),
            (40, false, 480.),
            (2, true, 480.),
            (2, false, 300.),
        ] {
            let items: Vec<Item> = (1..=count)
                .map(|n| {
                    pr(
                        "langfuse/langfuse",
                        n,
                        &format!("fix(datasets): change {n}"),
                    )
                })
                .collect();
            let first_key = items[0].key();
            let first = first_key.clone();
            let window = open(cx, move |inbox| {
                connect(inbox);
                inbox.state.prefs.set_view("all");
                inbox.state.feed.list_width = width;
                inbox.snapshot.items = items;
                if select {
                    inbox.selected = Some(first.clone());
                }
            });
            let mut visual = visual(window, cx);
            let label = visual
                .debug_bounds("git-group-others")
                .expect("the group label is on screen");
            let row = visual
                .debug_bounds(Box::leak(
                    format!("inbox-item-{first_key}").into_boxed_str(),
                ))
                .expect("the first row is on screen");
            let case = format!("{count} rows, selected {select}, width {width}");
            assert!(
                f32::from(label.size.height) >= 12. + 11. + 6.,
                "label squeezed to {:?} ({case})",
                label.size.height
            );
            assert!(
                label.bottom() <= row.top(),
                "label ends at {:?}, below the first row's top {:?} ({case})",
                label.bottom(),
                row.top()
            );
        }
    }
    #[gpui::test]
    fn the_view_tabs_and_filters_share_one_line_inside_the_list_at_every_width(
        cx: &mut TestAppContext,
    ) {
        // Needs me · Mine · All open with big counts, then Filters, must
        // stay on one line inside the list's padding at the narrowest and
        // the widest list.
        for (width, reviews) in [(300., true), (340., false), (480., true)] {
            let mut items: Vec<Item> = (1..=972)
                .map(|n| pr("0xvasanth/gespan", n, &format!("Change {n}")))
                .collect();
            if reviews {
                for item in items.iter_mut().take(120) {
                    item.requested_reviewers = vec!["me".into()];
                }
            }
            let window = open(cx, move |inbox| {
                connect(inbox);
                inbox.state.feed.list_width = width;
                inbox.snapshot.items = items;
            });
            let mut visual = visual(window, cx);
            let list = visual
                .debug_bounds("inbox-list")
                .expect("the list is on screen");
            let tabs = visual
                .debug_bounds("git-views")
                .expect("the view tabs show");
            let filters = visual.debug_bounds("git-filters").expect("Filters shows");
            let search = visual.debug_bounds("git-search").expect("search shows");
            let case = format!("width {width}, reviews {reviews}");
            assert!(
                filters.right() <= list.right() - px(14.) + px(0.5),
                "Filters ends at {:?}, past the list's padding ({case})",
                filters.right()
            );
            assert!(
                tabs.right() <= filters.left() + px(0.5),
                "tabs run into Filters ({case})"
            );
            assert!(
                (tabs.center().y - filters.center().y).abs() < px(2.),
                "tabs and Filters are one line ({case})"
            );
            assert!(
                search.top() >= filters.bottom(),
                "search sits under them ({case})"
            );
            for id in ["git-view-needs_me", "git-view-mine", "git-view-all"] {
                let tab = visual.debug_bounds(id).expect("each tab shows");
                assert!(
                    tab.right() <= tabs.right() + px(0.5) && tab.left() >= tabs.left() - px(0.5),
                    "{id} spills out of the tabs ({case})"
                );
            }
        }
    }
    #[gpui::test]
    fn rows_sit_under_who_they_wait_on_and_say_why(cx: &mut TestAppContext) {
        let mine_failing = Item {
            author: "me".into(),
            check_summary: model::CheckSummary {
                total: 3,
                passed: 2,
                failed: 1,
                failed_names: vec!["Rust tests (Windows)".into()],
                ..Default::default()
            },
            reviews: vec![model::Review {
                login: "mira".into(),
                state: "changes_requested".into(),
            }],
            ..pr("acme/web", 1, "Rail: Git takes Inbox's place")
        };
        let asked = Item {
            author: "devon".into(),
            requested_reviewers: vec!["me".into()],
            additions: 318,
            deletions: 204,
            ..pr("acme/api", 2, "Toasty: typed seen-state")
        };
        let waiting = Item {
            author: "me".into(),
            requested_reviewers: vec!["sam".into()],
            ..pr("acme/web", 3, "Datasets keep the pinned version")
        };
        let ready = Item {
            author: "lena".into(),
            review_decision: "approved".into(),
            reviews: vec![model::Review {
                login: "me".into(),
                state: "approved".into(),
            }],
            ..pr("acme/web", 4, "Pair a phone")
        };
        let keys = [mine_failing.key(), asked.key(), waiting.key(), ready.key()];
        let unread_key = asked.key();
        let window = open(cx, move |inbox| {
            connect(inbox);
            inbox.snapshot.items = vec![mine_failing, asked, waiting, ready];
            inbox.state.feed.seen.seeded = true;
            for (ix, item) in inbox.snapshot.items.iter().enumerate() {
                if ix != 1 {
                    inbox.state.feed.seen.mark(item);
                }
            }
        });
        let mut visual = visual(window, cx);
        let leak = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
        let top = |visual: &mut VisualTestContext, selector: &'static str| {
            visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is not on screen"))
                .origin
                .y
        };
        let you = top(&mut visual, "git-group-you");
        let others = top(&mut visual, "git-group-others");
        let ready_group = top(&mut visual, "git-group-ready");
        let rows = keys
            .clone()
            .map(|key| top(&mut visual, leak(format!("inbox-item-{key}"))));
        assert!(you < rows[0] && you < rows[1] && rows[0].max(rows[1]) < others);
        assert!(others < rows[2] && rows[2] < ready_group && ready_group < rows[3]);
        // Each row's second line says why it is there.
        let why = |visual: &mut VisualTestContext, key: &str| {
            window
                .update(visual, |inbox, _, _| {
                    let item = inbox
                        .snapshot
                        .items
                        .iter()
                        .find(|i| i.key() == key)
                        .unwrap();
                    gespan_core::inbox::queue::waiting(item).why
                })
                .unwrap()
        };
        assert_eq!(
            why(&mut visual, &keys[0]),
            "mira requested changes · Rust tests (Windows) failed"
        );
        assert_eq!(why(&mut visual, &keys[1]), "devon asked for your review");
        assert_eq!(why(&mut visual, &keys[2]), "waiting on sam");
        assert_eq!(
            why(&mut visual, &keys[3]),
            "approved by you · ready to merge"
        );
        for key in &keys {
            assert!(
                visual
                    .debug_bounds(leak(format!("git-why-{key}")))
                    .is_some()
            );
        }
        assert!(
            visual
                .debug_bounds(leak(format!("inbox-unread-{unread_key}")))
                .is_some()
        );
        assert!(
            visual
                .debug_bounds(leak(format!("inbox-unread-{}", keys[0])))
                .is_none()
        );
        assert!(visual.debug_bounds("inbox-empty-list").is_none());
    }
    #[gpui::test]
    fn a_selected_pull_request_shows_its_checks_beside_the_description(cx: &mut TestAppContext) {
        let item = Item {
            requested_reviewers: vec!["me".into()],
            check_summary: model::CheckSummary {
                total: 6,
                passed: 4,
                failed: 1,
                pending: 1,
                ..Default::default()
            },
            ..pr("acme/web", 9, "Change")
        };
        let key = item.key();
        let window = open(cx, move |inbox| {
            connect(inbox);
            inbox.selected = Some(item.key());
            inbox.snapshot.items = vec![item];
            inbox.details = Some(Details {
                body: "What it changes".into(),
                ..Default::default()
            });
            inbox.checks = Some(checks(&[
                "pass", "fail", "pending", "pass", "pass", "skipping",
            ]));
        });
        let mut visual = visual(window, cx);
        assert!(visual.debug_bounds("inbox-empty-detail").is_none());
        assert!(visual.debug_bounds("inbox-ask-bar").is_some());
        assert!(visual.debug_bounds("inbox-ask").is_some());
        let description = visual.debug_bounds("inbox-description-card").unwrap();
        let card = visual.debug_bounds("inbox-checks-card").unwrap();
        // Two columns: the checks card to the right of the description.
        assert!(card.origin.x >= description.origin.x + description.size.width);
        // The row's pill reads its checks from the list.
        let row_pill = Box::leak(format!("inbox-row-status-{key}").into_boxed_str());
        assert!(visual.debug_bounds(row_pill).is_some());
        assert!(visual.debug_bounds("inbox-reviewers-card").is_none());
        // Four checks show; the other two are a link to the Checks tab.
        let more = visual.debug_bounds("inbox-checks-card-more").unwrap();
        visual.simulate_click(more.center(), Default::default());
        visual.run_until_parked();
        let tab = window.update(cx, |inbox, _, _| inbox.tab).unwrap();
        assert!(tab == Tab::Checks);
    }

    #[gpui::test]
    fn each_tab_loads_in_a_card_on_the_content_column_until_its_data_arrives(
        cx: &mut TestAppContext,
    ) {
        let item = pr("acme/web", 9, "Change");
        // Files and Checks: a glass card whose left edge is the header's
        // column, not a bare line left of it.
        for (tab, selector) in [
            (Tab::Code, "inbox-loading-changes"),
            (Tab::Checks, "inbox-loading-checks"),
        ] {
            let item = item.clone();
            let window = open(cx, move |inbox| {
                connect(inbox);
                inbox.selected = Some(item.key());
                inbox.snapshot.items = vec![item];
                inbox.details = Some(Details::default());
                inbox.tab = tab;
                inbox.extra_loading = true;
            });
            let mut visual = visual(window, cx);
            let card = visual
                .debug_bounds(selector)
                .unwrap_or_else(|| panic!("{selector} is not on screen"));
            let header = visual.debug_bounds("inbox-document-status").unwrap();
            assert_eq!(card.left(), header.left(), "{selector} is off the column");
            assert!(
                card.size.height > px(60.),
                "{selector} has no skeleton lines"
            );
            window
                .update(cx, |inbox, _, cx| {
                    inbox.extra_loading = false;
                    if tab == Tab::Code {
                        inbox.diff = Some(Diff::default());
                    } else {
                        inbox.checks = Some(checks(&["pass"]));
                    }
                    cx.notify();
                })
                .unwrap();
            visual.run_until_parked();
            visual.refresh().unwrap();
            assert!(visual.debug_bounds(selector).is_none(), "{selector} stayed");
        }
        // Overview: the description card holds the loading lines.
        let window = open(cx, move |inbox| {
            connect(inbox);
            inbox.selected = Some(item.key());
            inbox.snapshot.items = vec![item];
            inbox.detail_loading = true;
        });
        let mut visual = visual(window, cx);
        let card = visual.debug_bounds("inbox-description-card").unwrap();
        let lines = visual.debug_bounds("inbox-loading-description").unwrap();
        assert!(card.contains(&lines.origin) && lines.bottom() <= card.bottom());
        window
            .update(cx, |inbox, _, cx| {
                inbox.detail_loading = false;
                inbox.details = Some(Details {
                    body: "What it changes".into(),
                    ..Default::default()
                });
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
        assert!(visual.debug_bounds("inbox-loading-description").is_none());
    }

    #[gpui::test]
    fn requested_reviewers_get_their_card_under_the_checks(cx: &mut TestAppContext) {
        let item = pr("acme/web", 9, "Change");
        let window = open(cx, move |inbox| {
            connect(inbox);
            inbox.selected = Some(item.key());
            inbox.snapshot.items = vec![item];
            inbox.details = Some(Details {
                review_requests: vec!["cubic".into()],
                ..Default::default()
            });
            inbox.checks = Some(checks(&["pass"]));
        });
        let mut visual = visual(window, cx);
        let checks = visual.debug_bounds("inbox-checks-card").unwrap();
        let reviewers = visual.debug_bounds("inbox-reviewers-card").unwrap();
        assert!(reviewers.origin.y >= checks.origin.y + checks.size.height);
        assert_eq!(reviewers.origin.x, checks.origin.x);
    }

    #[gpui::test]
    fn an_issue_has_no_checks_card(cx: &mut TestAppContext) {
        let issue = Item {
            kind: "issue".into(),
            ..pr("acme/web", 4, "A bug")
        };
        let window = open(cx, move |inbox| {
            connect(inbox);
            inbox.selected = Some(issue.key());
            inbox.snapshot.items = vec![issue];
            inbox.details = Some(Details::default());
            inbox.checks = Some(checks(&["pass"]));
        });
        let mut visual = visual(window, cx);
        assert!(visual.debug_bounds("inbox-description-card").is_some());
        assert!(visual.debug_bounds("inbox-checks-card").is_none());
        assert!(visual.debug_bounds("inbox-ask-bar").is_some());
    }

    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    #[gpui::test]
    fn the_header_reads_git_with_its_sync_and_no_source_picker(cx: &mut TestAppContext) {
        let window = open(cx, |inbox| {
            connect(inbox);
            inbox.synced_ms = Some(gespan_core::time::now_ms() as i64 - 120_000);
            inbox.snapshot.items = vec![pr("acme/web", 1, "one")];
        });
        let mut visual = visual(window, cx);
        assert!(visual.debug_bounds("git-refresh").is_some());
        assert!(visual.debug_bounds("git-synced").is_some());
        // The Inbox's source pill, "+" and filter-in-search are gone.
        for gone in [
            "inbox-source",
            "inbox-add-connection",
            "inbox-filter",
            "inbox-diff-full",
        ] {
            assert!(visual.debug_bounds(gone).is_none(), "{gone} is still drawn");
        }
        assert_eq!(super::git_list::ago(0, 120_000), "2m ago");
        assert_eq!(super::git_list::ago(0, 10_000), "just now");
        assert_eq!(super::git_list::ago(0, 3 * 3_600_000), "3h ago");
    }
    #[gpui::test]
    fn mark_all_as_read_sits_in_the_filters_dropdown_and_closes_it(cx: &mut TestAppContext) {
        let one = pr("acme/web", 1, "one");
        let two = pr("acme/web", 2, "two");
        let items = vec![one.clone(), two.clone()];
        let window = open(cx, move |inbox| {
            connect(inbox);
            inbox.snapshot.items = items;
            // Seeded with nothing seen yet: every item reads as unread.
            inbox.state.feed.seen.seeded = true;
        });
        let mut visual = visual(window, cx);
        let unseen = |visual: &mut VisualTestContext| {
            window
                .update(visual, |inbox, _, _| inbox.unread_count())
                .unwrap()
        };
        assert_eq!(unseen(&mut visual), 2);
        click(&mut visual, "git-filters");
        click(&mut visual, "inbox-read-all");
        assert_eq!(unseen(&mut visual), 0);
        assert!(
            !window
                .update(&mut visual, |inbox, _, _| inbox.filters_open)
                .unwrap()
        );
        assert!(visual.debug_bounds("inbox-read-all").is_none());
    }
    /// The detail pane's title, byline and description select and copy,
    /// while its buttons keep the pointer.
    #[gpui::test]
    fn the_detail_pane_text_selects_and_copies(cx: &mut TestAppContext) {
        use crate::selectable::test_support::{copy, cursor_at, drag};
        cx.update(crate::input::bind);
        let item = Item {
            title: "Fix flaky retries".into(),
            ..pr("gespan/app", 12, "Fix flaky retries")
        };
        let key = item.key();
        let window = open(cx, move |inbox| {
            connect(inbox);
            inbox.snapshot.items = vec![item];
            inbox.selected = Some(key);
            inbox.details = Some(Details {
                body: "Retries flake on slow runners.".into(),
                ..Default::default()
            });
        });
        window
            .update(cx, |inbox, _, cx| {
                inbox.document = Some(cx.new(|cx| {
                    document::InboxDocument::description(
                        "Retries **flake** on slow runners.".into(),
                        PathBuf::from("/missing/inbox"),
                        cx,
                    )
                }));
                cx.notify();
            })
            .unwrap();
        let mut visual = visual(window, cx);
        let cx = &mut visual;
        cx.run_until_parked();
        let bounds = |needle: &str, cx: &mut VisualTestContext| {
            window
                .update(cx, |inbox, _, _| inbox.text.bounds_for_test(needle))
                .unwrap()
                .unwrap_or_else(|| panic!("{needle:?} is selectable"))
        };
        let title = bounds("Fix flaky retries", cx);
        assert_eq!(
            cursor_at(cx, title.center()),
            Some(gpui::CursorStyle::IBeam)
        );
        let more = cx.debug_bounds("inbox-more").expect("the more button");
        assert_eq!(
            cursor_at(cx, more.center()),
            Some(gpui::CursorStyle::PointingHand)
        );
        drag(
            cx,
            point(title.left() + px(1.), title.center().y),
            point(title.right() + px(2.), title.center().y),
        );
        assert_eq!(copy(cx).as_deref(), Some("Fix flaky retries"));
        // From the title into the description, across the markdown.
        let body = bounds("slow runners.", cx);
        drag(
            cx,
            point(title.left() + px(1.), title.center().y),
            point(body.right() + px(2.), body.center().y),
        );
        let copied = copy(cx).expect("a selection across the pane copies");
        assert!(copied.starts_with("Fix flaky retries"), "{copied:?}");
        assert!(
            copied.ends_with("Retries flake on slow runners."),
            "{copied:?}"
        );
    }
}
