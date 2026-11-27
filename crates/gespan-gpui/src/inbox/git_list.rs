//! The Pull requests page's list: the header (title, sync, the Needs me / Mine / All
//! open tabs and the Filters dropdown on one line, search under it), pull
//! requests grouped by who they wait on, 25 to a page, new arrivals held
//! behind a banner, review requests from other repos folded under the list,
//! and the empty state with the connected hosts beside it.
use super::*;
use crate::glass;
use crate::popover::{Align, Anchor, popover};
use gespan_core::inbox::queue::{self as q, Current, Drafts, Extra, Tone as WhyTone, View};
use gpui::{
    AnyElement, CursorStyle, FontWeight, Hsla, MouseButton, Role, SharedString, div, prelude::*,
    px, rgb,
};

/// A row of the Filters dropdown whose choices can open under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FilterRow {
    Repos,
    Host,
    Author,
    Drafts,
    Extra(Extra),
}

/// The hosts a pull request can come from, in the order they are offered.
pub(super) const HOSTS: [&str; 3] = ["github", "gitlab", "azuredevops"];

fn why_ink(tone: WhyTone) -> Hsla {
    match (tone, is_light()) {
        (WhyTone::Hot, false) => rgb(0xf5dc84).into(),
        (WhyTone::Hot, true) => rgb(0x9a6b00).into(),
        (WhyTone::Bad, false) => rgb(0xfda4af).into(),
        (WhyTone::Bad, true) => rgb(0xbe123c).into(),
        (WhyTone::Plain, _) => content(0.62),
    }
}
fn pill_tone(tone: WhyTone) -> glass::Tone {
    match tone {
        WhyTone::Bad => glass::Tone::Bad,
        WhyTone::Hot => glass::Tone::Running,
        WhyTone::Plain => glass::Tone::Ok,
    }
}
/// "2m ago" from milliseconds.
pub(super) fn ago(ms: i64, now: i64) -> String {
    let secs = ((now - ms) / 1000).max(0);
    match secs {
        0..=44 => "just now".into(),
        45..=3599 => format!("{}m ago", (secs / 60).max(1)),
        3600..=86_399 => format!("{}h ago", secs / 3600),
        _ => format!("{}d ago", secs / 86_400),
    }
}
/// "09:12" in local time.
pub(super) fn clock(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|at| at.with_timezone(&chrono::Local).format("%H:%M").to_string())
        .unwrap_or_default()
}
/// "gespan #562 · devon · GitLab · 12m": where it lives, who opened it, the
/// host when it is not GitHub, and its age.
pub(super) fn git_meta(item: &Item) -> String {
    let mut parts = vec![format!(
        "{} {}",
        q::short_repo(&item.repo),
        q::number_label(item)
    )];
    if !item.author.is_empty() {
        parts.push(
            item.author
                .trim_start_matches("app/")
                .trim_end_matches("[bot]")
                .to_string(),
        );
    }
    if item.provider != "github" {
        parts.push(model::provider_label(&item.provider).to_string());
    }
    let age = super::ui::short_age(&item.updated_at);
    if !age.is_empty() {
        parts.push(age);
    }
    parts.join(" · ")
}

fn size_label(item: &Item) -> Option<(String, String)> {
    (item.additions > 0 || item.deletions > 0).then(|| {
        (
            format!("+{}", item.additions),
            format!("−{}", item.deletions),
        )
    })
}

impl Inbox {
    /// Every repository the list reads: your projects' remotes and the ones
    /// added by hand, minus those switched off in Settings › Pull requests.
    pub(super) fn your_repos(&self) -> Vec<(String, String)> {
        let mut repos: Vec<(String, String)> = Vec::new();
        let mut add = |provider: &str, repo: &str| {
            let key = q::repo_key(provider, repo);
            if !repo.is_empty()
                && !self.state.prefs.hidden_repos.contains(&key)
                && !repos.iter().any(|(p, r)| q::repo_key(p, r) == key)
            {
                repos.push((provider.to_string(), repo.to_string()));
            }
        };
        for repo in &self.snapshot.repos {
            if HOSTS.contains(&repo.provider.as_str()) {
                add(&repo.provider, &repo.repo);
            }
        }
        for repo in &self.state.prefs.added_repos {
            add(&repo.provider, &repo.repo);
        }
        repos
    }
    fn connected_hosts(&self) -> Vec<&'static str> {
        HOSTS
            .into_iter()
            .filter(|host| {
                self.snapshot
                    .connections
                    .get(*host)
                    .is_some_and(|s| s.connected)
            })
            .collect()
    }

    pub(super) fn set_git_view(&mut self, id: String, cx: &mut Context<Self>) {
        if self.state.prefs.view == id {
            return;
        }
        self.state.prefs.set_view(&id);
        self.filter_row = None;
        self.adding_filter = false;
        self.naming_view = false;
        self.page = 0;
        self.git_filters_changed(cx);
    }
    pub(super) fn change_filters(
        &mut self,
        change: impl FnOnce(&mut q::Filters),
        cx: &mut Context<Self>,
    ) {
        self.state.prefs.update_filters(change);
        self.page = 0;
        self.git_filters_changed(cx);
    }
    fn git_filters_changed(&mut self, cx: &mut Context<Self>) {
        self.save();
        self.sync_arrivals(cx);
        let visible = self.visible_items(cx);
        if self
            .selected
            .as_ref()
            .is_none_or(|key| !visible.iter().any(|item| &item.key() == key))
        {
            match visible.first() {
                Some(item) => self.select(item.key(), cx),
                None => self.deselect(cx),
            }
        }
        cx.notify();
    }
    pub(super) fn save_view(&mut self, cx: &mut Context<Self>) {
        let name = self.view_name.read(cx).content.to_string();
        if self.state.prefs.save_view(&name).is_some() {
            self.naming_view = false;
            self.view_name
                .update(cx, |input, cx| input.set_text(String::new(), cx));
            self.git_filters_changed(cx);
        }
    }
    /// "Show" on the banner: the held-back pull requests join the list.
    fn show_arrivals(&mut self, cx: &mut Context<Self>) {
        let rows = self
            .state
            .prefs
            .visible(&self.snapshot.items, self.search.read(cx).content.as_ref());
        self.arrivals
            .show(&rows, gespan_core::time::now_ms() as i64);
        self.page = 0;
        cx.notify();
    }
    pub(crate) fn add_repo(&mut self, provider: String, repo: String, cx: &mut Context<Self>) {
        let key = q::repo_key(&provider, &repo);
        self.state
            .prefs
            .hidden_repos
            .retain(|hidden| *hidden != key);
        if !self
            .state
            .prefs
            .added_repos
            .iter()
            .any(|added| q::repo_key(&added.provider, &added.repo) == key)
        {
            self.state
                .prefs
                .added_repos
                .push(q::AddedRepo { provider, repo });
        }
        self.add_repo_open = false;
        self.save();
        self.refresh(cx);
        cx.notify();
    }

    pub(super) fn list_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let prefs = &self.state.prefs;
        let search_text = self.search.read(cx).content.to_string();
        let rows = prefs.visible(&self.snapshot.items, &search_text);
        let (shown, held) = self.arrivals.split(&self.view_scope(cx), rows);
        let page = q::page(&shown, self.page);
        let connected = self.connected_hosts();
        let now = gespan_core::time::now_ms() as i64;

        let synced = if self.loading {
            "Syncing…".to_string()
        } else {
            self.synced_ms
                .map(|at| format!("Synced {}", ago(at, now)))
                .unwrap_or_default()
        };
        let refresh = glass::icon_button(
            "git-refresh",
            if self.loading { "loader" } else { "refresh-cw" },
        )
        .debug_selector(|| "git-refresh".into())
        .role(Role::Button)
        .aria_label("Refresh")
        .size(px(28.))
        .bg(gpui::transparent_black())
        .border_color(gpui::transparent_black())
        .on_click(cx.listener(|this, _, _, cx| {
            this.refresh(cx);
            this.load_detail(false, cx);
        }));
        let title = div()
            .flex()
            .items_center()
            .gap(px(6.))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_size(px(22.))
                    .line_height(px(26.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(content(1.))
                    .child("Pull requests"),
            )
            .child(
                div()
                    .debug_selector(|| "git-synced".into())
                    .text_size(px(11.5))
                    .text_color(content(0.5))
                    .child(synced),
            )
            .child(refresh);

        let current = prefs.current();
        let tabs = glass::segmented()
            .id("git-views")
            .debug_selector(|| "git-views".into())
            .role(Role::TabList)
            .aria_label("Views")
            .min_w_0()
            .flex_shrink(1.)
            .children(View::TABS.into_iter().map(|view| {
                let selected = current == Current::Builtin(view);
                let count = prefs.count(&self.snapshot.items, view);
                glass::segment(
                    SharedString::from(format!("git-view-{}", view.id())),
                    view.label(),
                    selected,
                )
                .debug_selector(move || format!("git-view-{}", view.id()))
                .role(Role::Tab)
                .aria_label(format!("{}, {count}", view.label()))
                .aria_selected(selected)
                .px(px(9.))
                .gap(px(4.))
                .min_w_0()
                .child(glass::count(count, view == View::NeedsMe && count > 0).flex_none())
                .on_click(
                    cx.listener(move |this, _, _, cx| this.set_git_view(view.id().into(), cx)),
                )
            }));
        let active = prefs.filters().active();
        let filters = glass::button("git-filters", "Filters", glass::Button::Glass)
            .debug_selector(|| "git-filters".into())
            .role(Role::Button)
            .aria_label(if active > 0 {
                format!("Filters, {active} on")
            } else {
                "Filters".into()
            })
            .aria_expanded(self.filters_open)
            .relative()
            .flex_none()
            .ml_auto()
            .h(px(32.))
            .px(px(9.))
            .gap(px(6.))
            .child(self.filter_anchor.probe())
            .when(active > 0, |d| d.child(glass::count(active, false)))
            .child(icon("chevron-down", 11.).text_color(content(0.7)))
            .on_click(cx.listener(|this, _, _, cx| {
                this.filters_open = !this.filters_open;
                this.filter_row = None;
                this.adding_filter = false;
                cx.notify();
            }));
        let view_line = div()
            .flex()
            .items_center()
            .gap(px(8.))
            .min_w_0()
            .child(tabs)
            .when_some(
                match current {
                    Current::Saved(saved) => Some(saved.name.clone()),
                    Current::Builtin(View::Bots) => Some("Bots".into()),
                    Current::Builtin(_) => None,
                },
                |d, name| {
                    d.child(
                        div()
                            .debug_selector(|| "git-current-view".into())
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(content(0.75))
                            .child(name),
                    )
                },
            )
            .child(filters);
        let search = glass::field()
            .debug_selector(|| "git-search".into())
            .child(icon("search", 14.).text_color(content(0.55)))
            .child(div().flex_1().min_w_0().child(self.search.clone()))
            .child(
                div()
                    .flex_none()
                    .px(px(5.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(glass::edge())
                    .font_family(crate::fonts::monospace())
                    .text_size(px(10.5))
                    .text_color(content(0.5))
                    .child("/"),
            );

        let mut list: Vec<AnyElement> = Vec::new();
        if held > 0 {
            list.push(
                div()
                    .id("git-new-banner")
                    .debug_selector(|| "git-new-banner".into())
                    .role(Role::Button)
                    .aria_label(format!(
                        "{held} new since {}, show",
                        clock(self.arrivals.since_ms())
                    ))
                    .flex_none()
                    .mx(px(4.))
                    .mb(px(6.))
                    .h(px(30.))
                    .px(px(12.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(9.))
                    .bg(accent_color().opacity(0.14))
                    .border_1()
                    .border_color(accent_color().opacity(0.3))
                    .text_size(px(12.))
                    .text_color(content(0.9))
                    .cursor_pointer()
                    .child(div().flex_1().child(format!(
                        "{held} new since {}",
                        clock(self.arrivals.since_ms())
                    )))
                    .child(div().text_color(accent_color()).child("Show"))
                    .on_click(cx.listener(|this, _, _, cx| this.show_arrivals(cx)))
                    .into_any_element(),
            );
        }
        for (group, items) in &page.groups {
            let total = page
                .group_totals
                .iter()
                .find(|(g, _)| g == group)
                .map_or(items.len(), |(_, n)| *n);
            let id = group.id();
            list.push(
                div()
                    .debug_selector(move || format!("git-group-{id}"))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(12.))
                    .pt(px(12.))
                    .pb(px(6.))
                    .child(glass::eyebrow(group.label()))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(content(0.62))
                            .child(total.to_string()),
                    )
                    .into_any_element(),
            );
            list.extend(items.iter().map(|item| self.git_row(item, cx)));
        }
        if page.groups.is_empty() {
            list.push(self.empty_list(&connected, cx));
        }
        if page.index + 1 == page.pages {
            list.extend(self.other_repos_fold(cx));
        }
        let hidden_bots = prefs.hidden_bots(&self.snapshot.items);
        if !page.groups.is_empty() || hidden_bots > 0 {
            list.push(
                div()
                    .debug_selector(|| "git-list-foot".into())
                    .flex_none()
                    .px(px(12.))
                    .pt(px(10.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.5))
                    .text_color(content(0.5))
                    .child(match (page.total, hidden_bots) {
                        (n, 0) => format!("{n} pull request{}", if n == 1 { "" } else { "s" }),
                        (n, b) => format!(
                            "{n} pull request{} · {b} bot PR{} hidden",
                            if n == 1 { "" } else { "s" },
                            if b == 1 { "" } else { "s" }
                        ),
                    })
                    .when(hidden_bots > 0, |d| {
                        d.child(
                            div()
                                .id("git-show-bots")
                                .debug_selector(|| "git-show-bots".into())
                                .role(Role::Button)
                                .aria_label("Show bots")
                                .cursor_pointer()
                                .text_color(accent_color())
                                .child("Show bots")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_git_view(View::Bots.id().into(), cx)
                                })),
                        )
                    })
                    .into_any_element(),
            );
        }

        let pager = self.pager(&page, held, cx);
        div()
            .debug_selector(|| "inbox-list".into())
            .relative()
            .w(px(self.state.feed.list_width.clamp(300., 480.)))
            .h_full()
            .flex_shrink_0()
            .border_r_1()
            .border_color(content(0.06))
            .bg(if is_light() {
                content(0.03)
            } else {
                black(0.12)
            })
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_shrink_0()
                    .px(px(14.))
                    .pt(px(18.))
                    .pb(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(title)
                    .child(view_line)
                    .child(search),
            )
            .child(
                div()
                    .id("inbox-list-scroll")
                    .role(Role::ListBox)
                    .aria_label("Pull requests")
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .px(px(8.))
                    .pb(px(10.))
                    .flex()
                    .flex_col()
                    .gap(px(1.))
                    .children(list),
            )
            .child(pager)
            .child(
                div()
                    .id("inbox-list-resize")
                    .role(Role::Splitter)
                    .aria_label("Resize the pull request list")
                    .absolute()
                    .right(px(-3.))
                    .top_0()
                    .bottom_0()
                    .w(px(6.))
                    .cursor(CursorStyle::ResizeLeftRight)
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &gpui::MouseDownEvent, _, cx| {
                            this.resize_start =
                                Some((f32::from(event.position.x), this.state.feed.list_width));
                            cx.stop_propagation();
                        }),
                    ),
            )
            .into_any_element()
    }

    fn pager(&self, page: &q::Page<'_>, held: usize, cx: &mut Context<Self>) -> AnyElement {
        let mut bar = div()
            .debug_selector(|| "git-pager".into())
            .flex_none()
            .flex()
            .items_center()
            .gap(px(6.))
            .px(px(14.))
            .py(px(10.))
            .border_t_1()
            .border_color(content(0.06))
            .text_size(px(12.))
            .text_color(content(0.62));
        if page.total == 0 {
            return bar
                .child(format!(
                    "{} · checks again every {} min",
                    if self.loading {
                        "Syncing…".to_string()
                    } else {
                        "Up to date".into()
                    },
                    if self.active { 2 } else { 5 }
                ))
                .into_any_element();
        }
        bar = bar.child(
            div()
                .debug_selector(|| "git-page-range".into())
                .flex_1()
                .min_w_0()
                .truncate()
                .child(format!(
                    "{}–{} of {}{}",
                    page.start + 1,
                    page.end,
                    page.total,
                    if held > 0 {
                        format!(" + {held} new")
                    } else {
                        String::new()
                    }
                )),
        );
        if page.pages > 1 {
            let last = page.pages - 1;
            let index = page.index;
            let step = |id: &'static str,
                        glyph: &'static str,
                        label: &'static str,
                        to: usize,
                        enabled: bool| {
                div()
                    .id(id)
                    .debug_selector(move || id.into())
                    .role(Role::Button)
                    .aria_label(label)
                    .size(px(28.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(px(7.))
                    .when(!enabled, |d| d.opacity(0.35))
                    .when(enabled, |d| {
                        d.cursor_pointer()
                            .hover(|s| s.bg(content(0.08)))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.page = to;
                                cx.notify();
                            }))
                    })
                    .child(icon(glyph, 13.).text_color(content(0.7)))
            };
            bar = bar.child(step(
                "git-page-prev",
                "chevron-left",
                "Previous page",
                index.saturating_sub(1),
                index > 0,
            ));
            // At most five numbers, around the current page.
            let from = index.saturating_sub(2).min(page.pages.saturating_sub(5));
            for n in from..(from + 5).min(page.pages) {
                bar = bar.child(
                    div()
                        .id(SharedString::from(format!("git-page-{n}")))
                        .debug_selector(move || format!("git-page-{n}"))
                        .role(Role::Button)
                        .aria_label(format!("Page {}", n + 1))
                        .aria_selected(n == index)
                        .min_w(px(28.))
                        .h(px(28.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded(px(7.))
                        .cursor_pointer()
                        .when(n == index, |d| d.bg(content(0.10)).text_color(content(1.)))
                        .hover(|s| s.bg(content(0.08)))
                        .child((n + 1).to_string())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.page = n;
                            cx.notify();
                        })),
                );
            }
            bar = bar.child(step(
                "git-page-next",
                "chevron-right",
                "Next page",
                (index + 1).min(last),
                index < last,
            ));
        }
        bar.into_any_element()
    }

    /// One pull request: its title (bold while unread), why it is here, where
    /// it lives, and its checks and size.
    fn git_row(&self, item: &Item, cx: &mut Context<Self>) -> AnyElement {
        let key = item.key();
        let active = self.selected.as_ref() == Some(&key);
        let unread = self.state.feed.seen.unseen(item);
        let waiting = q::waiting(item);
        let pill = q::check_pill(item);
        let size = size_label(item);
        div()
            .id(SharedString::from(format!("inbox-item-{key}")))
            .debug_selector({
                let key = key.clone();
                move || format!("inbox-item-{key}")
            })
            .role(Role::Button)
            .aria_label(format!("{}, {}", item.title, waiting.why))
            .aria_selected(active)
            .relative()
            .flex_none()
            .px(px(12.))
            .py(px(10.))
            .flex()
            .items_start()
            .gap(px(11.))
            .rounded(px(12.))
            .border_1()
            .cursor_pointer()
            .when(active, |d| {
                d.bg(glass::selected_fill())
                    .border_color(glass::edge())
                    .shadow(vec![
                        gpui::BoxShadow::new(px(0.), px(1.), white(0.08)).inset(),
                    ])
            })
            .when(!active, |d| {
                d.border_color(gpui::transparent_black())
                    .hover(|s| s.bg(glass::hover_fill()))
            })
            .when(unread, |d| {
                d.child(
                    div()
                        .debug_selector({
                            let key = key.clone();
                            move || format!("inbox-unread-{key}")
                        })
                        .absolute()
                        .left(px(-1.))
                        .top(px(17.))
                        .size(px(6.))
                        .rounded_full()
                        .bg(accent_color()),
                )
            })
            .child(div().mt(px(1.)).child(if item.author.is_empty() {
                div()
                    .size(px(22.))
                    .rounded(px(7.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(content(0.06))
                    .child(super::ui::provider_mark(&item.provider).size(px(12.)))
                    .into_any_element()
            } else {
                super::menu::initials_avatar(item.author.trim_start_matches("app/"))
            }))
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
                            .font_weight(if unread {
                                FontWeight::SEMIBOLD
                            } else {
                                FontWeight::NORMAL
                            })
                            .text_color(content(if unread { 1. } else { 0.85 }))
                            .child(item.title.clone()),
                    )
                    .child(
                        div()
                            .debug_selector({
                                let key = key.clone();
                                move || format!("git-why-{key}")
                            })
                            .truncate()
                            .text_size(px(11.5))
                            .line_height(px(16.))
                            .text_color(why_ink(waiting.tone))
                            .child(waiting.why.clone()),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(px(11.5))
                            .line_height(px(16.))
                            .text_color(content(0.5))
                            .child(git_meta(item)),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_col()
                    .items_end()
                    .gap(px(5.))
                    .children(pill.map(|(label, tone)| {
                        let key = key.clone();
                        glass::status(label, pill_tone(tone))
                            .h(px(20.))
                            .debug_selector(move || format!("inbox-row-status-{key}"))
                    }))
                    .children(size.map(|(add, del)| {
                        div()
                            .flex()
                            .gap(px(6.))
                            .text_size(px(11.5))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(div().text_color(diff_foreground(true)).child(add))
                            .child(div().text_color(diff_foreground(false)).child(del))
                    })),
            )
            .on_click({
                let key = key.clone();
                cx.listener(move |this, _, _, cx| this.select(key.clone(), cx))
            })
            .into_any_element()
    }

    /// The list when the view is empty: why, and where to go instead.
    fn empty_list(&self, connected: &[&'static str], cx: &mut Context<Self>) -> AnyElement {
        if connected.is_empty() && !self.loading {
            return glass::empty_state(
                "git-pull-request",
                "Connect a git host",
                "Git lists pull requests from GitHub, GitLab and Azure DevOps.",
            )
            .debug_selector(|| "inbox-empty-list".into())
            .py(px(48.))
            .px(px(12.))
            .child(
                glass::button(
                    "git-open-connections",
                    "Open Connections",
                    glass::Button::Glass,
                )
                .debug_selector(|| "git-open-connections".into())
                .role(Role::Button)
                .aria_label("Open Connections")
                .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::OpenConnections))),
            )
            .into_any_element();
        }
        if self.loading && self.snapshot.items.is_empty() {
            return super::ui::message("Loading pull requests…")
                .debug_selector(|| "inbox-empty-list".into())
                .into_any_element();
        }
        let prefs = &self.state.prefs;
        let searching = !self.search.read(cx).content.is_empty();
        if prefs.filters().active() > 0 || searching {
            return div()
                .debug_selector(|| "inbox-empty-list".into())
                .flex()
                .flex_col()
                .items_center()
                .gap(px(10.))
                .py(px(40.))
                .child(super::ui::message("No pull requests match these filters"))
                .child(
                    glass::button("git-clear-filters", "Clear filters", glass::Button::Glass)
                        .role(Role::Button)
                        .aria_label("Clear filters")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.search
                                .update(cx, |input, cx| input.set_text(String::new(), cx));
                            this.change_filters(|f| *f = q::Filters::default(), cx);
                        })),
                )
                .into_any_element();
        }
        let repos = self.your_repos().len();
        let (title, hint) = match prefs.base() {
            View::NeedsMe => (
                "Nothing needs you",
                format!(
                    "No reviews requested and none of your pull requests are blocked, across your {repos} repo{}.",
                    if repos == 1 { "" } else { "s" }
                ),
            ),
            View::Mine => (
                "No pull requests of yours",
                "Open ones you author show here.".into(),
            ),
            View::Bots => (
                "No bot pull requests",
                "dependabot and renovate show here.".into(),
            ),
            View::All => (
                "No open pull requests",
                format!(
                    "None are open across your {repos} repo{}.",
                    if repos == 1 { "" } else { "s" }
                ),
            ),
        };
        let mine = prefs.count(&self.snapshot.items, View::Mine);
        div()
            .debug_selector(|| "inbox-empty-list".into())
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.))
            .px(px(30.))
            .py(px(48.))
            .text_center()
            .child(
                div()
                    .size(px(44.))
                    .rounded(px(12.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(content(0.07))
                    .border_1()
                    .border_color(glass::edge())
                    .child(icon("check", 20.).text_color(rgb(0x6ee7b7))),
            )
            .child(
                div()
                    .text_size(px(18.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(content(1.))
                    .child(title),
            )
            .child(
                div()
                    .max_w(px(280.))
                    .text_size(px(12.))
                    .line_height(px(17.))
                    .text_color(content(0.62))
                    .child(hint),
            )
            .when(prefs.base() == View::NeedsMe, |d| {
                d.child(
                    div()
                        .mt(px(6.))
                        .flex()
                        .gap(px(8.))
                        .child(
                            glass::button(
                                "git-empty-mine",
                                format!("Mine · {mine} waiting on others"),
                                glass::Button::Glass,
                            )
                            .debug_selector(|| "git-empty-mine".into())
                            .role(Role::Button)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.set_git_view(View::Mine.id().into(), cx)
                            })),
                        )
                        .child(
                            glass::button("git-empty-all", "All open", glass::Button::Ghost)
                                .debug_selector(|| "git-empty-all".into())
                                .role(Role::Button)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.set_git_view(View::All.id().into(), cx)
                                })),
                        ),
                )
            })
            .into_any_element()
    }

    /// "N review requests in other repos": folded until Show, with Add repo.
    fn other_repos_fold(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        if !self.state.prefs.other_repos {
            return vec![];
        }
        let (rows, repos) = q::other_requests(&self.snapshot.items);
        if rows.is_empty() {
            return vec![];
        }
        let n = rows.len();
        let names = repos.join(", ");
        let mut out = vec![
            div()
                .debug_selector(|| "git-other-repos".into())
                .flex_none()
                .mt(px(10.))
                .mx(px(4.))
                .px(px(12.))
                .py(px(9.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(10.))
                .border_1()
                .border_color(glass::edge())
                .bg(glass::fill())
                .text_size(px(12.))
                .text_color(content(0.75))
                .child(div().flex_1().min_w_0().truncate().child(format!(
                    "{n} review request{} in other repos · {names}",
                    if n == 1 { "" } else { "s" }
                )))
                .child(
                    div()
                        .id("git-other-repos-show")
                        .debug_selector(|| "git-other-repos-show".into())
                        .role(Role::Button)
                        .aria_label(if self.others_open { "Hide" } else { "Show" })
                        .flex_none()
                        .cursor_pointer()
                        .text_color(accent_color())
                        .child(if self.others_open { "Hide" } else { "Show" })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.others_open = !this.others_open;
                            cx.notify();
                        })),
                )
                .child(
                    div()
                        .id("git-other-repos-add")
                        .debug_selector(|| "git-other-repos-add".into())
                        .role(Role::Button)
                        .aria_label("Add repo")
                        .relative()
                        .flex_none()
                        .cursor_pointer()
                        .text_color(accent_color())
                        .child(self.add_repo_anchor.probe())
                        .child("Add repo")
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.add_repo_open = !this.add_repo_open;
                            cx.notify();
                        })),
                )
                .into_any_element(),
        ];
        if self.others_open {
            out.extend(rows.into_iter().map(|item| self.git_row(item, cx)));
        }
        out
    }

    /// The repositories the other-repos row can add, one per row.
    pub(super) fn add_repo_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let (rows, repos) = q::other_requests(&self.snapshot.items);
        popover(
            "git-add-repo-menu",
            Anchor::Slot(self.add_repo_anchor.clone()),
        )
        .align(Align::End)
        .width(260.)
        .padding(4.)
        .on_dismiss(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.add_repo_open = false;
                cx.notify();
            });
        })
        .child(super::ui::caption("ADD TO YOUR REPOS"))
        .children(repos.into_iter().enumerate().map(|(ix, repo)| {
            let provider = rows
                .iter()
                .find(|item| item.repo.eq_ignore_ascii_case(&repo))
                .map(|item| item.provider.clone())
                .unwrap_or_else(|| "github".into());
            let label = repo.clone();
            super::ui::menu_action_owned(format!("git-add-repo-{ix}"), label, "plus").on_click(
                cx.listener(move |this, _, _, cx| {
                    this.add_repo(provider.clone(), repo.clone(), cx)
                }),
            )
        }))
        .into_any_element()
    }

    /// The detail side when the list is empty: the hosts Git reads.
    pub(super) fn hosts_card(&self, cx: &mut Context<Self>) -> AnyElement {
        let repos = self.your_repos();
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(px(14.))
            .child(
                glass::card()
                    .debug_selector(|| "git-hosts".into())
                    .w(px(460.))
                    .px(px(20.))
                    .py(px(18.))
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(glass::eyebrow("Connected git hosts"))
                    .children(HOSTS.into_iter().map(|host| {
                        let status = self.snapshot.connections.get(host);
                        let connected = status.is_some_and(|s| s.connected);
                        let count = repos.iter().filter(|(p, _)| p == host).count();
                        let account = status
                            .map(|s| {
                                [&s.organization, &s.site, &s.url]
                                    .into_iter()
                                    .find(|v| !v.is_empty())
                                    .cloned()
                                    .unwrap_or_default()
                            })
                            .unwrap_or_default();
                        let line = if !connected {
                            "Not connected".to_string()
                        } else {
                            let repos =
                                format!("{count} repo{}", if count == 1 { "" } else { "s" });
                            if account.is_empty() {
                                repos
                            } else {
                                format!("{account} · {repos}")
                            }
                        };
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                glass::status(
                                    model::provider_label(host),
                                    if connected {
                                        glass::Tone::Ok
                                    } else {
                                        glass::Tone::Muted
                                    },
                                )
                                .h(px(20.)),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(px(12.))
                                    .text_color(content(0.62))
                                    .child(line),
                            )
                            .when(!connected, |d| {
                                d.child(
                                    div()
                                        .id(SharedString::from(format!("git-connect-{host}")))
                                        .debug_selector(move || format!("git-connect-{host}"))
                                        .role(Role::Link)
                                        .aria_label(format!(
                                            "Connect {}",
                                            model::provider_label(host)
                                        ))
                                        .cursor_pointer()
                                        .text_size(px(12.5))
                                        .text_color(accent_color())
                                        .child("Connect")
                                        .on_click(cx.listener(|_, _, _, cx| {
                                            cx.emit(Event::OpenConnections)
                                        })),
                                )
                            })
                    })),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(11.5))
                    .text_color(content(0.5))
                    .child(icon("kanban", 13.).text_color(content(0.5)))
                    .child("Issues and tickets live on Board once a tracker is linked."),
            )
            .into_any_element()
    }

    pub(super) fn git_filters_menu(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        let prefs = &self.state.prefs;
        let filters = prefs.filters();
        let view_name = match prefs.current() {
            Current::Saved(saved) => saved.name.clone(),
            Current::Builtin(view) => view.label().to_string(),
        };
        let mut rows: Vec<AnyElement> = vec![
            div()
                .flex()
                .items_center()
                .px(px(8.))
                .pt(px(8.))
                .pb(px(4.))
                .child(
                    div()
                        .flex_1()
                        .text_size(px(12.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(content(0.9))
                        .child(format!("Filters on {view_name}")),
                )
                .when(filters.active() > 0, |d| {
                    d.child(
                        div()
                            .id("git-filters-clear")
                            .debug_selector(|| "git-filters-clear".into())
                            .role(Role::Button)
                            .aria_label("Clear filters")
                            .cursor_pointer()
                            .text_size(px(12.))
                            .text_color(accent_color())
                            .child("Clear")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.change_filters(|f| *f = q::Filters::default(), cx)
                            })),
                    )
                })
                .into_any_element(),
        ];
        let your = self.your_repos();
        let shown_repos = your
            .iter()
            .filter(|(p, r)| !filters.hidden_repos.contains(&q::repo_key(p, r)))
            .count();
        let repos_value = if shown_repos == your.len() {
            format!("Your {}", your.len())
        } else {
            format!("{shown_repos} of {}", your.len())
        };
        let list_or = |values: &[String], all: &str| {
            if values.is_empty() {
                all.to_string()
            } else {
                values.join(", ")
            }
        };
        let hosts_value = if filters.hosts.is_empty() {
            "All".into()
        } else {
            filters
                .hosts
                .iter()
                .map(|h| model::provider_label(h).to_string())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let mut entries = vec![
            (FilterRow::Repos, "Repos".to_string(), repos_value),
            (FilterRow::Host, "Host".into(), hosts_value),
            (
                FilterRow::Author,
                "Author".into(),
                list_or(&filters.authors, "Anyone"),
            ),
            (
                FilterRow::Drafts,
                "Drafts".into(),
                filters.drafts.label().to_string(),
            ),
        ];
        for extra in &filters.extras {
            let value = match extra {
                Extra::Label => list_or(&filters.labels, "Any"),
                Extra::Reviewer => list_or(&filters.reviewers, "Anyone"),
                Extra::Checks | Extra::Size => {
                    let chosen = if *extra == Extra::Checks {
                        &filters.checks
                    } else {
                        &filters.sizes
                    };
                    if chosen.is_empty() {
                        "Any".into()
                    } else {
                        extra
                            .fixed_options()
                            .iter()
                            .filter(|(id, _)| chosen.iter().any(|c| c == id))
                            .map(|(_, label)| label.split(" (").next().unwrap_or(label).to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                }
                Extra::Updated => extra
                    .fixed_options()
                    .iter()
                    .find(|(id, _)| *id == filters.updated)
                    .map_or("Any time".into(), |(_, label)| label.to_string()),
            };
            entries.push((FilterRow::Extra(*extra), extra.label().to_string(), value));
        }
        for (row, label, value) in entries {
            let open = self.filter_row == Some(row);
            let id = filter_row_id(row);
            rows.push(
                div()
                    .id(SharedString::from(format!("git-filter-{id}")))
                    .debug_selector(move || format!("git-filter-{id}"))
                    .role(Role::Button)
                    .aria_label(format!("{label}: {value}"))
                    .aria_expanded(open)
                    .h(px(30.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .hover(|s| s.bg(content(0.06)))
                    .when(open, |d| d.bg(content(0.06)))
                    .child(
                        div()
                            .w(px(64.))
                            .flex_none()
                            .text_color(content(0.55))
                            .child(label),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_color(content(0.95))
                            .child(value),
                    )
                    .when(matches!(row, FilterRow::Extra(_)), |d| {
                        let FilterRow::Extra(extra) = row else {
                            unreachable!()
                        };
                        d.child(
                            div()
                                .id(SharedString::from(format!("git-filter-remove-{id}")))
                                .debug_selector(move || format!("git-filter-remove-{id}"))
                                .role(Role::Button)
                                .aria_label("Remove filter")
                                .cursor_pointer()
                                .child(icon("x", 12.).text_color(content(0.5)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.filter_row = None;
                                    this.change_filters(|f| f.remove_extra(extra), cx);
                                })),
                        )
                    })
                    .child(
                        icon(
                            if open {
                                "chevron-down"
                            } else {
                                "chevron-right"
                            },
                            12.,
                        )
                        .text_color(content(0.5)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filter_row = (this.filter_row != Some(row)).then_some(row);
                        this.adding_filter = false;
                        cx.notify();
                    }))
                    .into_any_element(),
            );
            if open {
                rows.extend(self.filter_options(row, &filters, cx));
            }
        }
        rows.push(
            div()
                .id("git-add-filter")
                .debug_selector(|| "git-add-filter".into())
                .role(Role::Button)
                .aria_label("Add filter")
                .aria_expanded(self.adding_filter)
                .h(px(30.))
                .px(px(8.))
                .flex()
                .items_center()
                .gap(px(8.))
                .rounded(px(6.))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.06)))
                .text_size(px(12.))
                .child(icon("plus", 13.).text_color(accent_color()))
                .child(div().text_color(accent_color()).child("Add filter"))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_color(content(0.45))
                        .child("label, reviewer, checks, size, updated"),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    this.adding_filter = !this.adding_filter;
                    this.filter_row = None;
                    cx.notify();
                }))
                .into_any_element(),
        );
        if self.adding_filter {
            for extra in Extra::ALL
                .into_iter()
                .filter(|e| !filters.extras.contains(e))
            {
                let label = extra.label();
                rows.push(
                    super::ui::menu_action_owned(
                        format!("git-add-filter-{}", label.to_lowercase()),
                        label.to_string(),
                        "plus",
                    )
                    .ml(px(16.))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.adding_filter = false;
                        this.filter_row = Some(FilterRow::Extra(extra));
                        this.change_filters(|f| f.extras.push(extra), cx);
                    }))
                    .into_any_element(),
                );
            }
        }
        rows.push(super::ui::divider().into_any_element());
        rows.push(super::ui::caption("SAVED VIEWS").into_any_element());
        let bots = prefs.count(&self.snapshot.items, View::Bots);
        let mut views: Vec<(String, String, usize, bool)> =
            vec![(View::Bots.id().to_string(), "Bots".into(), bots, false)];
        for saved in &prefs.saved {
            let mut probe = prefs.clone();
            probe.set_view(&format!("saved:{}", saved.id));
            let n = probe.visible(&self.snapshot.items, "").len();
            views.push((format!("saved:{}", saved.id), saved.name.clone(), n, true));
        }
        for (ix, (id, name, n, removable)) in views.into_iter().enumerate() {
            let on = prefs.view == id;
            let open_id = id.clone();
            rows.push(
                div()
                    .id(SharedString::from(format!("git-saved-view-{ix}")))
                    .debug_selector(move || format!("git-saved-view-{ix}"))
                    .role(Role::MenuItemRadio)
                    .aria_label(name.clone())
                    .aria_selected(on)
                    .h(px(28.))
                    .px(px(8.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .rounded(px(6.))
                    .cursor_pointer()
                    .text_size(px(12.))
                    .text_color(content(0.85))
                    .hover(|s| s.bg(content(0.06)))
                    .when(on, |d| d.bg(content(0.08)))
                    .child(div().flex_1().min_w_0().truncate().child(name))
                    .child(div().text_color(content(0.5)).child(n.to_string()))
                    .when(removable, |d| {
                        let saved = id.trim_start_matches("saved:").to_string();
                        d.child(
                            div()
                                .id(SharedString::from(format!("git-saved-view-delete-{ix}")))
                                .debug_selector(move || format!("git-saved-view-delete-{ix}"))
                                .role(Role::Button)
                                .aria_label("Delete view")
                                .cursor_pointer()
                                .child(icon("trash", 12.).text_color(content(0.5)))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.state.prefs.delete_view(&saved);
                                    this.git_filters_changed(cx);
                                })),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.filters_open = false;
                        this.set_git_view(open_id.clone(), cx);
                    }))
                    .into_any_element(),
            );
        }
        rows.push(super::ui::divider().into_any_element());
        rows.push(if self.naming_view {
            div()
                .px(px(6.))
                .py(px(4.))
                .flex()
                .items_center()
                .gap(px(6.))
                .child(
                    glass::field()
                        .debug_selector(|| "git-view-name".into())
                        .flex_1()
                        .h(px(28.))
                        .child(div().flex_1().min_w_0().child(self.view_name.clone())),
                )
                .child(
                    glass::button("git-view-save", "Save", glass::Button::Primary)
                        .debug_selector(|| "git-view-save".into())
                        .role(Role::Button)
                        .aria_label("Save view")
                        .h(px(28.))
                        .px(px(10.))
                        .on_click(cx.listener(|this, _, _, cx| this.save_view(cx))),
                )
                .into_any_element()
        } else {
            div()
                .flex()
                .items_center()
                .px(px(8.))
                .py(px(6.))
                .text_size(px(11.5))
                .child(
                    div()
                        .flex_1()
                        .text_color(content(0.5))
                        .child(if prefs.keep_filters {
                            "Kept for this view"
                        } else {
                            "Cleared when Gespan restarts"
                        }),
                )
                .child(
                    div()
                        .id("git-save-as-view")
                        .debug_selector(|| "git-save-as-view".into())
                        .role(Role::Button)
                        .aria_label("Save as view")
                        .cursor_pointer()
                        .text_color(accent_color())
                        .child("Save as view…")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.naming_view = true;
                            window.focus(&this.view_name.focus_handle(cx), cx);
                            cx.notify();
                        })),
                )
                .into_any_element()
        });
        rows.push(
            super::ui::menu_action_owned(
                "inbox-read-all".into(),
                "Mark all as read".into(),
                "check-2",
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.filters_open = false;
                this.mark_all_read(cx);
            }))
            .into_any_element(),
        );
        popover(
            "inbox-filter-menu",
            Anchor::Slot(self.filter_anchor.clone()),
        )
        .align(Align::End)
        .width(320.)
        .max_height(620.)
        .padding(4.)
        .on_dismiss(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.filters_open = false;
                this.naming_view = false;
                cx.notify();
            });
        })
        .child(
            div()
                .id("git-filter-scroll")
                .overflow_y_scroll()
                .children(rows),
        )
        .into_any_element()
    }

    /// The choices under an open filter row.
    fn filter_options(
        &self,
        row: FilterRow,
        filters: &q::Filters,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let option = |id: String, label: String, checked: bool| {
            let selector = id.clone();
            super::ui::menu_row(id, label, checked)
                .debug_selector(move || selector.clone())
                .ml(px(16.))
        };
        let prs: Vec<&Item> = self
            .snapshot
            .items
            .iter()
            .filter(|item| q::listed(item))
            .collect();
        let distinct = |values: Vec<String>| {
            let mut out: Vec<String> = Vec::new();
            for value in values {
                if !value.is_empty() && !out.iter().any(|v| v.eq_ignore_ascii_case(&value)) {
                    out.push(value);
                }
            }
            out.sort_by_key(|v| v.to_lowercase());
            out.truncate(30);
            out
        };
        match row {
            FilterRow::Repos => self
                .your_repos()
                .into_iter()
                .enumerate()
                .map(|(ix, (provider, repo))| {
                    let key = q::repo_key(&provider, &repo);
                    let on = !filters.hidden_repos.contains(&key);
                    option(format!("git-filter-repo-{ix}"), repo, on)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let key = key.clone();
                            this.change_filters(
                                move |f| super::ui::toggle(&mut f.hidden_repos, key),
                                cx,
                            )
                        }))
                        .into_any_element()
                })
                .collect(),
            FilterRow::Host => HOSTS
                .into_iter()
                .map(|host| {
                    option(
                        format!("git-filter-host-{host}"),
                        model::provider_label(host).to_string(),
                        filters.hosts.iter().any(|h| h == host),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_filters(
                            |f| super::ui::toggle(&mut f.hosts, host.to_string()),
                            cx,
                        )
                    }))
                    .into_any_element()
                })
                .collect(),
            FilterRow::Drafts => Drafts::ALL
                .into_iter()
                .map(|drafts| {
                    option(
                        format!(
                            "git-filter-drafts-{}",
                            drafts.label().to_lowercase().replace(' ', "-")
                        ),
                        drafts.label().to_string(),
                        filters.drafts == drafts,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_filters(|f| f.drafts = drafts, cx)
                    }))
                    .into_any_element()
                })
                .collect(),
            FilterRow::Author
            | FilterRow::Extra(Extra::Label)
            | FilterRow::Extra(Extra::Reviewer) => {
                let values = distinct(match row {
                    FilterRow::Author => prs
                        .iter()
                        .map(|item| {
                            item.author
                                .trim_start_matches("app/")
                                .trim_end_matches("[bot]")
                                .to_string()
                        })
                        .collect(),
                    FilterRow::Extra(Extra::Label) => prs
                        .iter()
                        .flat_map(|item| item.labels.iter().map(|l| l.name.clone()))
                        .collect(),
                    _ => prs
                        .iter()
                        .flat_map(|item| {
                            item.requested_reviewers
                                .iter()
                                .cloned()
                                .chain(item.reviews.iter().map(|r| r.login.clone()))
                        })
                        .collect(),
                });
                if values.is_empty() {
                    return vec![super::ui::message("Nothing to choose yet").into_any_element()];
                }
                values
                    .into_iter()
                    .enumerate()
                    .map(|(ix, value)| {
                        let chosen = match row {
                            FilterRow::Author => &filters.authors,
                            FilterRow::Extra(Extra::Label) => &filters.labels,
                            _ => &filters.reviewers,
                        };
                        let on = chosen.iter().any(|v| v.eq_ignore_ascii_case(&value));
                        option(
                            format!("git-filter-{}-{ix}", filter_row_id(row)),
                            value.clone(),
                            on,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let value = value.clone();
                            this.change_filters(
                                move |f| {
                                    let list = match row {
                                        FilterRow::Author => &mut f.authors,
                                        FilterRow::Extra(extra) => f.values(extra),
                                        _ => unreachable!(),
                                    };
                                    super::ui::toggle(list, value)
                                },
                                cx,
                            )
                        }))
                        .into_any_element()
                    })
                    .collect()
            }
            FilterRow::Extra(extra) => extra
                .fixed_options()
                .iter()
                .map(|(id, label)| {
                    let id = *id;
                    let on = match extra {
                        Extra::Updated => filters.updated == id,
                        Extra::Checks => filters.checks.iter().any(|c| c == id),
                        _ => filters.sizes.iter().any(|c| c == id),
                    };
                    option(
                        format!("git-filter-{}-{id}", filter_row_id(row)),
                        label.to_string(),
                        on,
                    )
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.change_filters(
                            move |f| match extra {
                                Extra::Updated => {
                                    f.updated = if f.updated == id {
                                        String::new()
                                    } else {
                                        id.into()
                                    }
                                }
                                other => super::ui::toggle(f.values(other), id.to_string()),
                            },
                            cx,
                        )
                    }))
                    .into_any_element()
                })
                .collect(),
        }
    }
}

fn filter_row_id(row: FilterRow) -> &'static str {
    match row {
        FilterRow::Repos => "repos",
        FilterRow::Host => "host",
        FilterRow::Author => "author",
        FilterRow::Drafts => "drafts",
        FilterRow::Extra(Extra::Label) => "label",
        FilterRow::Extra(Extra::Reviewer) => "reviewer",
        FilterRow::Extra(Extra::Checks) => "checks",
        FilterRow::Extra(Extra::Size) => "size",
        FilterRow::Extra(Extra::Updated) => "updated",
    }
}
