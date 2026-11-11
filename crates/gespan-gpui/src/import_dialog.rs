//! The Import sessions window: one tab per agent with sessions on disk, a
//! search field, a This folder / All folders toggle and a ticked list. The
//! shell opens it from the project's menu and does the import itself when
//! [`Event::Import`] arrives.
use crate::{
    input::{self, Input},
    modal,
    popover::AnchorSlot,
    tab_tile::{Face, TileSize, mark_tile},
    theme::*,
};
use chrono::{Local, TimeZone};
use gespan_core::{
    files::path_key,
    provider::Kind,
    session_import::{self, Candidate, Placement, Source},
    store::Session,
};
use gpui::{
    AnyElement, App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    KeyDownEvent, Role, SharedString, Subscription, Window, div, prelude::*, px,
};
use std::{collections::HashSet, path::Path, sync::Arc};

pub(crate) enum Event {
    Close,
    Import(Vec<Candidate>),
}

/// The agent's brand tile on its tab.
const TAB_TILE: TileSize = TileSize {
    w: 16.,
    h: 14.,
    radius: 4.,
};
/// Rows counted per trip off the UI thread, so counts fill in as they come.
const COUNT_BATCH: usize = 8;
const DAY_MS: u64 = 86_400_000;
const MINUTE_MS: u64 = 60_000;
const HOUR_MS: u64 = 3_600_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Scope {
    ThisFolder,
    AllFolders,
}

struct Row {
    candidate: Candidate,
    placement: Placement,
    /// Gespan already holds it, live or archived.
    imported: bool,
}

struct Tab {
    kind: Kind,
    rows: Vec<Row>,
    loading: bool,
}

pub(crate) struct ImportDialog {
    project: String,
    tabs: Vec<Tab>,
    active: usize,
    /// Ticked rows by `(agent, provider id)`, across every tab.
    selected: HashSet<(Kind, String)>,
    scope: Scope,
    query: Entity<Input>,
    focus: FocusHandle,
    /// The laid-out dialog, so the shell can centre it.
    bounds: AnchorSlot,
    _query_events: Subscription,
}
impl EventEmitter<Event> for ImportDialog {}
impl Focusable for ImportDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// The main checkout a linked worktree belongs to.
pub(crate) fn worktree_root(path: &str) -> Option<String> {
    gespan_core::git::linked_worktree_root(Path::new(path))
        .map(|root| root.to_string_lossy().into_owned())
}

fn rows(found: Vec<(Candidate, Placement)>, sessions: &[Session]) -> Vec<Row> {
    found
        .into_iter()
        .map(|(candidate, placement)| Row {
            imported: session_import::already_imported(
                sessions,
                candidate.kind,
                &candidate.provider_id,
            ),
            candidate,
            placement,
        })
        .collect()
}

/// The project a row would land in, if any.
fn placed_in(placement: &Placement) -> Option<&str> {
    match placement {
        Placement::Project { project } | Placement::Worktree { project, .. } => Some(project),
        Placement::Skip(_) => None,
    }
}

/// Local midnight of the day holding `now`.
fn today_start(now: u64) -> u64 {
    Local
        .timestamp_millis_opt(now as i64)
        .single()
        .and_then(|time| time.date_naive().and_hms_opt(0, 0, 0))
        .and_then(|midnight| midnight.and_local_timezone(Local).earliest())
        .map_or(now - now % DAY_MS, |midnight| {
            midnight.timestamp_millis().max(0) as u64
        })
}

/// When a row was last used: how long ago today, the date before that.
fn when(at: u64, now: u64) -> String {
    if at >= today_start(now) {
        let ago = now.saturating_sub(at);
        return match ago {
            0..MINUTE_MS => "now".into(),
            MINUTE_MS..HOUR_MS => format!("{}m ago", ago / MINUTE_MS),
            _ => format!("{}h ago", ago / HOUR_MS),
        };
    }
    let Some(time) = Local.timestamp_millis_opt(at as i64).single() else {
        return String::new();
    };
    let this_year = Local
        .timestamp_millis_opt(now as i64)
        .single()
        .is_some_and(|today| today.format("%Y").to_string() == time.format("%Y").to_string());
    time.format(if this_year { "%b %-d" } else { "%b %-d, %Y" })
        .to_string()
}

/// A folder with the user's home shortened to `~`.
fn folder(cwd: &str) -> String {
    gespan_core::accounts::home()
        .and_then(|home| {
            Path::new(cwd)
                .strip_prefix(&home)
                .ok()
                .map(|rest| format!("~/{}", rest.display()))
        })
        .unwrap_or_else(|| cwd.to_owned())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl ImportDialog {
    /// Opens with a tab per source; each fills in from disk off the UI
    /// thread, then its message counts follow.
    pub(crate) fn new(
        project: String,
        projects: Vec<String>,
        sessions: Vec<Session>,
        sources: Vec<Source>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tabs = sources
            .iter()
            .map(|source| Tab {
                kind: source.kind,
                rows: Vec::new(),
                loading: true,
            })
            .collect();
        let mut this = Self::with_tabs(project, tabs, window, cx);
        let projects = Arc::new(projects);
        let sessions = Arc::new(sessions);
        for (ix, source) in sources.into_iter().enumerate() {
            this.load(ix, source, projects.clone(), sessions.clone(), cx);
        }
        this
    }

    /// Opens on rows already found and placed, skipping the disk.
    #[cfg(test)]
    fn from_rows(
        project: String,
        tabs: Vec<(Kind, Vec<(Candidate, Placement)>)>,
        sessions: &[Session],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let tabs = tabs
            .into_iter()
            .map(|(kind, found)| Tab {
                kind,
                rows: rows(found, sessions),
                loading: false,
            })
            .collect();
        Self::with_tabs(project, tabs, window, cx)
    }

    fn with_tabs(
        project: String,
        tabs: Vec<Tab>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let query =
            cx.new(|cx| Input::field("Search sessions", "Search sessions", content(0.4), cx));
        let query_events = cx.subscribe(&query, |_, _, _: &input::Changed, cx| cx.notify());
        Self {
            project,
            tabs,
            active: 0,
            selected: HashSet::new(),
            scope: Scope::ThisFolder,
            query,
            focus: cx.focus_handle(),
            bounds: AnchorSlot::default(),
            _query_events: query_events,
        }
    }

    /// Every row found, on every tab, in list order.
    #[cfg(test)]
    pub(crate) fn found_candidates(&self) -> Vec<Candidate> {
        self.tabs
            .iter()
            .flat_map(|tab| tab.rows.iter().map(|row| row.candidate.clone()))
            .collect()
    }

    /// The dialog's height at the last layout.
    pub(crate) fn measured_height(&self) -> Option<f32> {
        self.bounds.get().map(|bounds| bounds.size.height.as_f32())
    }

    /// Lists and places `source`'s sessions in the background, then counts
    /// their messages a batch at a time. `place` asks git about worktrees,
    /// so it stays off the UI thread too.
    fn load(
        &mut self,
        ix: usize,
        source: Source,
        projects: Arc<Vec<String>>,
        sessions: Arc<Vec<Session>>,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(async move |this, cx| {
            let executor = cx.background_executor().clone();
            let found = executor
                .spawn(async move {
                    let placed = session_import::list(source.kind, &source.home)
                        .into_iter()
                        .map(|candidate| {
                            let placement =
                                session_import::place(&candidate.cwd, &projects, worktree_root);
                            (candidate, placement)
                        })
                        .collect();
                    rows(placed, &sessions)
                })
                .await;
            let uncounted: Vec<Candidate> = found
                .iter()
                .filter(|row| row.candidate.messages.is_none())
                .map(|row| row.candidate.clone())
                .collect();
            let shown = this.update(cx, |this, cx| {
                this.tabs[ix].rows = found;
                this.tabs[ix].loading = false;
                cx.notify();
            });
            if shown.is_err() {
                return;
            }
            for batch in uncounted.chunks(COUNT_BATCH) {
                let batch = batch.to_vec();
                let counts = executor
                    .spawn(async move {
                        batch
                            .into_iter()
                            .map(|c| {
                                let n = session_import::count_messages(&c);
                                (c.provider_id, n)
                            })
                            .collect::<Vec<_>>()
                    })
                    .await;
                let counted = this.update(cx, |this, cx| {
                    for (id, n) in counts {
                        if let Some(row) = this.tabs[ix]
                            .rows
                            .iter_mut()
                            .find(|row| row.candidate.provider_id == id)
                        {
                            row.candidate.messages = n;
                        }
                    }
                    cx.notify();
                });
                if counted.is_err() {
                    return;
                }
            }
        })
        .detach();
    }

    fn in_scope(&self, row: &Row) -> bool {
        match self.scope {
            Scope::AllFolders => true,
            Scope::ThisFolder => {
                placed_in(&row.placement).is_some_and(|p| path_key(p) == path_key(&self.project))
            }
        }
    }

    /// Rows of tab `ix` the current scope shows, before searching.
    fn tab_count(&self, ix: usize) -> usize {
        self.tabs[ix]
            .rows
            .iter()
            .filter(|row| self.in_scope(row))
            .count()
    }

    /// The active tab's rows in scope whose title holds the search.
    fn visible(&self, cx: &App) -> Vec<&Row> {
        let query = self.query.read(cx).content.trim().to_lowercase();
        self.tabs.get(self.active).map_or_else(Vec::new, |tab| {
            tab.rows
                .iter()
                .filter(|row| self.in_scope(row))
                .filter(|row| {
                    query.is_empty() || row.candidate.title.to_lowercase().contains(&query)
                })
                .collect()
        })
    }

    fn is_ticked(&self, candidate: &Candidate) -> bool {
        self.selected
            .contains(&(candidate.kind, candidate.provider_id.clone()))
    }

    fn toggle(&mut self, kind: Kind, id: String, cx: &mut Context<Self>) {
        let key = (kind, id);
        if !self.selected.remove(&key) {
            self.selected.insert(key);
        }
        cx.notify();
    }

    /// Every ticked row, tab by tab, in list order.
    fn ticked(&self) -> Vec<Candidate> {
        self.tabs
            .iter()
            .flat_map(|tab| &tab.rows)
            .filter(|row| !row.imported && self.is_ticked(&row.candidate))
            .map(|row| row.candidate.clone())
            .collect()
    }

    /// The footer's count and its Import button's label.
    fn footer(&self) -> (String, String) {
        let n = self.ticked().len();
        (
            format!("{n} selected"),
            format!("Import {}", plural(n, "session", "sessions")),
        )
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let picked = self.ticked();
        if !picked.is_empty() {
            cx.emit(Event::Import(picked));
        }
    }

    fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.scope = scope;
        cx.notify();
    }

    fn header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_shrink_0()
            .flex()
            .items_start()
            .gap_2()
            .px_4()
            .pt(px(14.))
            .child(
                div()
                    .min_w_0()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(modal::title("Import sessions"))
                    .child(
                        div()
                            .text_size(px(12.))
                            .line_height(px(17.))
                            .text_color(content(0.5))
                            .child(
                                "Bring sessions you ran in other tools into Gespan. They keep working in both places.",
                            ),
                    ),
            )
            .child(
                icon_button("import-close", "Close", "close")
                    .debug_selector(|| "import-close".into())
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Close))),
            )
    }

    fn tab_strip(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_shrink_0()
            .flex()
            .gap(px(2.))
            .px_4()
            .pt_3()
            .border_b_1()
            .border_color(content(0.08))
            .children(self.tabs.iter().enumerate().map(|(ix, tab)| {
                let on = ix == self.active;
                let id = tab.kind.id();
                let selector = format!("import-tab-{id}");
                div()
                    .id(SharedString::from(selector.clone()))
                    .debug_selector(move || selector)
                    .role(Role::Tab)
                    .aria_label(tab.kind.label())
                    .px(px(12.))
                    .py(px(7.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded_t(px(7.))
                    .border_1()
                    .text_size(px(12.))
                    .cursor_pointer()
                    .map(|d| {
                        if on {
                            d.bg(content(0.04))
                                .border_color(content(0.08))
                                .text_color(content(0.95))
                        } else {
                            d.border_color(gpui::transparent_black())
                                .text_color(content(0.55))
                                .hover(|s| s.text_color(content(0.85)))
                        }
                    })
                    .child(mark_tile(
                        &TAB_TILE,
                        Face::provider(id),
                        provider_glyph(id, 10.),
                    ))
                    .child(tab.kind.label())
                    .when(!tab.loading, |d| {
                        d.child(
                            div()
                                .text_size(px(10.))
                                .text_color(content(0.4))
                                .child(self.tab_count(ix).to_string()),
                        )
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.active = ix;
                        cx.notify();
                    }))
            }))
    }

    fn scope_button(&self, scope: Scope, cx: &mut Context<Self>) -> impl IntoElement {
        let (id, label) = match scope {
            Scope::ThisFolder => ("import-scope-this", "This folder"),
            Scope::AllFolders => ("import-scope-all", "All folders"),
        };
        let on = self.scope == scope;
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Button)
            .aria_label(label)
            .px(px(10.))
            .h_full()
            .flex()
            .items_center()
            .text_size(px(12.))
            .cursor_pointer()
            .map(|d| {
                if on {
                    d.bg(content(0.1)).text_color(content(0.95))
                } else {
                    d.text_color(content(0.55))
                        .hover(|s| s.text_color(content(0.85)))
                }
            })
            .child(label)
            .on_click(cx.listener(move |this, _, _, cx| this.set_scope(scope, cx)))
    }

    fn bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py(px(10.))
            .bg(content(0.04))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h(px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .rounded(px(7.))
                    .border_1()
                    .border_color(content(0.1))
                    .bg(content(0.03))
                    .child(icon("search", 12.))
                    .child(div().flex_1().min_w_0().child(self.query.clone())),
            )
            .child(
                div()
                    .flex_shrink_0()
                    .h(px(28.))
                    .flex()
                    .rounded(px(7.))
                    .border_1()
                    .border_color(content(0.1))
                    .overflow_hidden()
                    .child(self.scope_button(Scope::ThisFolder, cx))
                    .child(self.scope_button(Scope::AllFolders, cx)),
            )
    }

    fn group(label: &'static str) -> AnyElement {
        let selector = format!("import-group-{}", label.to_lowercase());
        div()
            .debug_selector(move || selector)
            .px_2()
            .pt_2()
            .pb_1()
            .text_size(px(10.))
            .font_weight(FontWeight::MEDIUM)
            .text_color(content(0.4))
            .child(label.to_uppercase())
            .into_any_element()
    }

    fn row(&self, row: &Row, now: u64, cx: &mut Context<Self>) -> AnyElement {
        let c = &row.candidate;
        let id = c.provider_id.clone();
        let ticked = !row.imported && self.is_ticked(c);
        let all = self.scope == Scope::AllFolders;
        let no_project = all && matches!(row.placement, Placement::Skip(_));
        let mut meta: Vec<String> = Vec::new();
        if let Some(n) = c.messages {
            meta.push(plural(n as usize, "message", "messages"));
        }
        if let Some(branch) = &c.branch {
            meta.push(branch.clone());
        }
        if all {
            meta.push(folder(&c.cwd));
        }
        let selector = format!("import-row-{id}");
        let checkbox = div()
            .size(px(15.))
            .flex_none()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(4.))
            .border(px(1.5))
            .map(|d| {
                if ticked {
                    let tick = format!("{selector}-ticked");
                    d.debug_selector(move || tick)
                        .bg(accent_color())
                        .border_color(accent_color())
                        .child(icon("check", 10.).text_color(white(1.)))
                } else {
                    d.border_color(content(0.3))
                        .when(row.imported, |d| d.opacity(0.35))
                }
            });
        let trailing = if row.imported {
            let badge = format!("{selector}-imported");
            div()
                .debug_selector(move || badge)
                .flex_shrink_0()
                .px(px(6.))
                .py(px(1.))
                .rounded_full()
                .bg(content(0.08))
                .text_size(px(10.))
                .text_color(content(0.6))
                .child("Imported")
        } else {
            div()
                .flex_shrink_0()
                .text_size(px(11.))
                .text_color(content(0.45))
                .whitespace_nowrap()
                .child(when(c.updated_at, now))
        };
        let kind = c.kind;
        div()
            .id(SharedString::from(format!("{}-{selector}", kind.id())))
            .debug_selector(move || selector.clone())
            .role(Role::CheckBox)
            .aria_label(c.title.clone())
            .flex()
            .items_center()
            .gap(px(10.))
            .px_2()
            .py(px(7.))
            .rounded(px(7.))
            .when(ticked, |d| d.bg(accent_color().opacity(0.08)))
            .when(!row.imported, |d| {
                d.cursor_pointer()
                    .when(!ticked, |d| d.hover(|s| s.bg(content(0.04))))
                    .on_click(cx.listener(move |this, _, _, cx| this.toggle(kind, id.clone(), cx)))
            })
            .child(checkbox)
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
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(content(if row.imported { 0.55 } else { 0.9 }))
                            .child(c.title.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .gap_1()
                            .min_w_0()
                            .text_size(px(11.))
                            .text_color(content(0.45))
                            .child(div().min_w_0().truncate().child(meta.join(" · ")))
                            .when(no_project, |d| {
                                let note = format!("import-row-{}-no-project", c.provider_id);
                                d.child(
                                    div()
                                        .debug_selector(move || note)
                                        .flex_shrink_0()
                                        .text_color(content(0.35))
                                        .child("· No project"),
                                )
                            }),
                    ),
            )
            .child(trailing)
            .into_any_element()
    }

    fn list(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let list = div()
            .id("import-list")
            .h(px(300.))
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .px_2()
            .pb_2()
            .bg(content(0.04));
        let Some(tab) = self.tabs.get(self.active) else {
            return list.child(Self::empty(
                "No other agents' sessions were found on this computer.",
            ));
        };
        if tab.loading {
            return list.child(Self::empty("Looking for sessions…"));
        }
        let visible = self.visible(cx);
        if visible.is_empty() {
            let agent = tab.kind.label();
            let note = if self.tab_count(self.active) > 0 {
                "No sessions match your search.".to_owned()
            } else if self.scope == Scope::ThisFolder {
                format!("No {agent} sessions in this folder.")
            } else {
                format!("No {agent} sessions found.")
            };
            return list.child(Self::empty(note));
        }
        let now = gespan_core::time::now_ms();
        let start = today_start(now);
        let (today, earlier): (Vec<&Row>, Vec<&Row>) = visible
            .into_iter()
            .partition(|row| row.candidate.updated_at >= start);
        let mut children = Vec::new();
        for (label, rows) in [("Today", today), ("Earlier", earlier)] {
            if rows.is_empty() {
                continue;
            }
            children.push(Self::group(label));
            children.extend(rows.into_iter().map(|row| self.row(row, now, cx)));
        }
        list.children(children)
    }

    fn empty(note: impl Into<SharedString>) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .items_center()
            .justify_center()
            .text_size(px(12.))
            .text_color(content(0.45))
            .child(note.into())
    }

    fn footer_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let (count, label) = self.footer();
        let none = self.ticked().is_empty();
        div()
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_between()
            .px_4()
            .py_3()
            .border_t_1()
            .border_color(content(0.08))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(content(0.5))
                    .child(count),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(
                        modal::ghost_button("import-cancel", "Cancel")
                            .debug_selector(|| "import-cancel".into())
                            .cursor_pointer()
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(Event::Close))),
                    )
                    .child(
                        modal::disabled(modal::primary_button("import-confirm", label), none)
                            .debug_selector(|| "import-confirm".into())
                            .on_click(cx.listener(|this, _, _, cx| this.import(cx))),
                    ),
            )
    }
}

impl Render for ImportDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("import-dialog")
            .debug_selector(|| "import-dialog".into())
            .role(Role::Dialog)
            .aria_label("Import sessions")
            .track_focus(&self.focus)
            .relative()
            .flex()
            .flex_col()
            .text_color(content(1.))
            .child(self.bounds.layout_probe())
            .on_key_down(cx.listener(|_, event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.emit(Event::Close);
                    window.prevent_default();
                    cx.stop_propagation();
                }
            }))
            .child(self.header(cx))
            .child(self.tab_strip(cx))
            .child(self.bar(cx))
            .child(self.list(cx))
            .child(self.footer_bar(cx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::session_import::Skip;
    use gpui::{Subscription, TestAppContext, VisualTestContext, WindowHandle};
    use std::{cell::RefCell, rc::Rc};

    const PROJECT: &str = "/work/gespan";
    const OTHER: &str = "/work/other";
    const DAY: u64 = 86_400_000;

    #[derive(Debug, PartialEq)]
    enum Seen {
        Close,
        Import(Vec<String>),
    }
    type Log = Rc<RefCell<Vec<Seen>>>;

    fn candidate(kind: Kind, id: &str, title: &str, cwd: &str, age: u64) -> Candidate {
        Candidate {
            kind,
            provider_id: id.into(),
            cwd: cwd.into(),
            title: title.into(),
            branch: Some("main".into()),
            updated_at: gespan_core::time::now_ms() - age,
            messages: Some(4),
            path: format!("/sessions/{id}").into(),
        }
    }
    fn here() -> Placement {
        Placement::Project {
            project: PROJECT.into(),
        }
    }
    fn there() -> Placement {
        Placement::Project {
            project: OTHER.into(),
        }
    }
    fn nowhere() -> Placement {
        Placement::Skip(Skip::NoProject)
    }
    fn session(kind: Kind, id: &str, archived: bool) -> Session {
        Session {
            id: 1,
            title: "Already here".into(),
            provider: kind.id().into(),
            cwd: PROJECT.into(),
            provider_id: Some(id.into()),
            has_messages: true,
            updated_at: 0,
            created_at: 0,
            archived,
            pinned: false,
            work_item: None,
        }
    }
    /// Claude: two rows in this project (one in a linked worktree), one in
    /// another project, one with no project, one already imported. Codex:
    /// one here, one with no project.
    fn tabs() -> Vec<(Kind, Vec<(Candidate, Placement)>)> {
        let worktree = Placement::Worktree {
            project: PROJECT.into(),
            worktree: format!("{PROJECT}/.worktrees/ci"),
        };
        vec![
            (
                Kind::Claude,
                vec![
                    (
                        candidate(
                            Kind::Claude,
                            "c-today",
                            "Refactor the auth middleware",
                            PROJECT,
                            0,
                        ),
                        here(),
                    ),
                    (
                        candidate(
                            Kind::Claude,
                            "c-tree",
                            "Why is the release build slow?",
                            PROJECT,
                            2 * DAY,
                        ),
                        worktree,
                    ),
                    (
                        candidate(
                            Kind::Claude,
                            "c-other",
                            "Fix the other tests",
                            OTHER,
                            3 * DAY,
                        ),
                        there(),
                    ),
                    (
                        candidate(
                            Kind::Claude,
                            "c-loose",
                            "Scratch experiment",
                            "/tmp/loose",
                            4 * DAY,
                        ),
                        nowhere(),
                    ),
                    (
                        candidate(
                            Kind::Claude,
                            "c-done",
                            "Terminal turn sync spike",
                            PROJECT,
                            5 * DAY,
                        ),
                        here(),
                    ),
                ],
            ),
            (
                Kind::Codex,
                vec![
                    (
                        candidate(
                            Kind::Codex,
                            "x-here",
                            "Add pagination to the sessions API",
                            PROJECT,
                            DAY,
                        ),
                        here(),
                    ),
                    (
                        candidate(
                            Kind::Codex,
                            "x-loose",
                            "Write the Windows installer",
                            "/tmp/w",
                            6 * DAY,
                        ),
                        nowhere(),
                    ),
                ],
            ),
        ]
    }
    fn open(cx: &mut TestAppContext) -> (WindowHandle<ImportDialog>, Log, Subscription) {
        cx.update(|cx| {
            cx.set_reduce_motion(true);
            crate::input::bind(cx);
        });
        let sessions = [session(Kind::Claude, "c-done", true)];
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |window, cx| {
                cx.new(|cx| ImportDialog::from_rows(PROJECT.into(), tabs(), &sessions, window, cx))
            })
            .unwrap()
        });
        let log = Log::default();
        let output = log.clone();
        let events = window
            .update(cx, |_, _, cx| {
                cx.subscribe(&cx.entity(), move |_, _, event: &Event, _| {
                    output.borrow_mut().push(match event {
                        Event::Close => Seen::Close,
                        Event::Import(picked) => {
                            Seen::Import(picked.iter().map(|c| c.provider_id.clone()).collect())
                        }
                    })
                })
            })
            .unwrap();
        (window, log, events)
    }
    fn shown(visual: &mut VisualTestContext, selector: &'static str) -> bool {
        visual.refresh().unwrap();
        visual.debug_bounds(selector).is_some()
    }
    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        visual.refresh().unwrap();
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
    }
    fn footer(
        window: WindowHandle<ImportDialog>,
        visual: &mut VisualTestContext,
    ) -> (String, String) {
        window
            .update(visual, |dialog, _, _| dialog.footer())
            .unwrap()
    }

    #[gpui::test]
    fn import_dialog_shows_a_tab_per_source_with_counts(cx: &mut TestAppContext) {
        let (window, _log, _events) = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert!(shown(visual, "import-tab-claude"));
        assert!(shown(visual, "import-tab-codex"));
        assert!(!shown(visual, "import-tab-cursor"));
        // This folder counts the project's own rows, worktree and imported
        // ones included.
        window
            .update(visual, |dialog, _, _| {
                assert_eq!((dialog.tab_count(0), dialog.tab_count(1)), (3, 1));
            })
            .unwrap();
        click(visual, "import-scope-all");
        window
            .update(visual, |dialog, _, _| {
                assert_eq!((dialog.tab_count(0), dialog.tab_count(1)), (5, 2));
            })
            .unwrap();
    }

    #[gpui::test]
    fn import_dialog_this_folder_hides_other_projects_rows(cx: &mut TestAppContext) {
        let (window, _log, _events) = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert!(shown(visual, "import-row-c-today"));
        assert!(
            shown(visual, "import-row-c-tree"),
            "a linked worktree is this folder"
        );
        assert!(!shown(visual, "import-row-c-other"));
        assert!(!shown(visual, "import-row-c-loose"));
        // Today's row sits under Today; the older ones under Earlier.
        let y = |visual: &mut VisualTestContext, selector| {
            visual.debug_bounds(selector).unwrap().origin.y
        };
        let today = y(visual, "import-group-today");
        let fresh = y(visual, "import-row-c-today");
        let earlier = y(visual, "import-group-earlier");
        let older = y(visual, "import-row-c-tree");
        assert!(today < fresh && fresh < earlier && earlier < older);
    }

    #[gpui::test]
    fn import_dialog_all_folders_marks_no_project_rows(cx: &mut TestAppContext) {
        let (window, _log, _events) = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        click(visual, "import-scope-all");
        assert!(shown(visual, "import-row-c-other"));
        assert!(shown(visual, "import-row-c-loose"));
        assert!(shown(visual, "import-row-c-loose-no-project"));
        assert!(!shown(visual, "import-row-c-other-no-project"));
        assert!(!shown(visual, "import-row-c-today-no-project"));
        click(visual, "import-row-c-loose");
        assert_eq!(footer(window, visual).0, "1 selected");
        click(visual, "import-scope-this");
        assert!(!shown(visual, "import-row-c-loose"));
    }

    #[gpui::test]
    fn import_dialog_already_imported_rows_cannot_be_ticked(cx: &mut TestAppContext) {
        let (window, log, _events) = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert!(shown(visual, "import-row-c-done-imported"));
        assert!(!shown(visual, "import-row-c-today-imported"));
        click(visual, "import-row-c-done");
        assert_eq!(footer(window, visual).0, "0 selected");
        click(visual, "import-confirm");
        assert!(log.borrow().is_empty());
    }

    #[gpui::test]
    fn import_dialog_selection_survives_tab_switches_and_counts_in_footer(cx: &mut TestAppContext) {
        let (window, _log, _events) = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        assert_eq!(
            footer(window, visual),
            ("0 selected".into(), "Import 0 sessions".into())
        );
        click(visual, "import-row-c-today");
        click(visual, "import-tab-codex");
        assert!(!shown(visual, "import-row-c-today"));
        click(visual, "import-row-x-here");
        assert_eq!(
            footer(window, visual),
            ("2 selected".into(), "Import 2 sessions".into())
        );
        click(visual, "import-tab-claude");
        assert!(shown(visual, "import-row-c-today-ticked"));
        click(visual, "import-row-c-today");
        assert!(!shown(visual, "import-row-c-today-ticked"));
        assert_eq!(
            footer(window, visual),
            ("1 selected".into(), "Import 1 session".into())
        );
    }

    #[gpui::test]
    fn import_dialog_search_filters_titles(cx: &mut TestAppContext) {
        let (window, _log, _events) = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |dialog, _, cx| {
                dialog
                    .query
                    .update(cx, |query, cx| query.set_text("AUTH".into(), cx))
            })
            .unwrap();
        visual.run_until_parked();
        assert!(shown(visual, "import-row-c-today"));
        assert!(!shown(visual, "import-row-c-tree"));
        assert!(!shown(visual, "import-group-earlier"));
    }

    #[gpui::test]
    fn import_dialog_import_emits_the_ticked_candidates(cx: &mut TestAppContext) {
        let (window, log, _events) = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        click(visual, "import-confirm");
        assert!(log.borrow().is_empty(), "nothing ticked, nothing to import");
        click(visual, "import-row-c-tree");
        click(visual, "import-row-c-today");
        click(visual, "import-tab-codex");
        click(visual, "import-row-x-here");
        click(visual, "import-confirm");
        assert_eq!(
            *log.borrow(),
            vec![Seen::Import(vec![
                "c-today".into(),
                "c-tree".into(),
                "x-here".into()
            ])]
        );
    }

    #[gpui::test]
    fn import_dialog_escape_closes(cx: &mut TestAppContext) {
        let (window, log, _events) = open(cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        window
            .update(visual, |dialog, window, cx| {
                window.focus(&dialog.focus_handle(cx), cx)
            })
            .unwrap();
        visual.simulate_keystrokes("escape");
        visual.run_until_parked();
        assert_eq!(*log.borrow(), vec![Seen::Close]);
        click(visual, "import-cancel");
        click(visual, "import-close");
        assert_eq!(*log.borrow(), vec![Seen::Close, Seen::Close, Seen::Close]);
    }

    #[gpui::test]
    fn import_dialog_loads_sources_off_the_ui_thread(cx: &mut TestAppContext) {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("app");
        std::fs::create_dir_all(&project).unwrap();
        let project = project.to_string_lossy().into_owned();
        let home = root.path().join("claude");
        let line = |cwd: &str, text: &str| {
            serde_json::json!({
                "type": "user", "uuid": "u1", "parentUuid": null,
                "cwd": cwd, "gitBranch": "main",
                "message": {"role": "user", "content": text},
            })
            .to_string()
        };
        for (id, cwd, text) in [
            ("in-app", project.as_str(), "Ship the import window"),
            ("elsewhere", "/nowhere/at/all", "Somewhere else"),
        ] {
            let path = home.join("projects/-x").join(format!("{id}.jsonl"));
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, line(cwd, text) + "\n").unwrap();
        }
        cx.update(crate::input::bind);
        let sources = vec![Source {
            kind: Kind::Claude,
            home,
        }];
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |window, cx| {
                cx.new(|cx| {
                    ImportDialog::new(
                        project.clone(),
                        vec![project.clone()],
                        Vec::new(),
                        sources,
                        window,
                        cx,
                    )
                })
            })
            .unwrap()
        });
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        assert!(shown(visual, "import-tab-claude"));
        assert!(shown(visual, "import-row-in-app"));
        assert!(!shown(visual, "import-row-elsewhere"));
        window
            .update(visual, |dialog, _, _| {
                assert_eq!(dialog.tab_count(0), 1);
                let row = &dialog.tabs[0].rows[0];
                assert_eq!(row.candidate.messages, Some(1), "messages are counted");
            })
            .unwrap();
        click(visual, "import-scope-all");
        assert!(shown(visual, "import-row-elsewhere-no-project"));
    }
}
