//! The Board app view, over the same `board*` tables the Tauri app uses.
//! Writes from the Tauri app (or another window) show up when the view opens,
//! when the window becomes active, and when SQLite's `data_version` moves.
mod card;
mod column;
#[cfg(test)]
mod tests;

use crate::{
    input::{self, Input},
    popover::{Anchor, AnchorSlot, popover},
    runtime::{Database, locked},
    theme::*,
};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyDownEvent, Role,
    SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use rebase_core::board::{Board, BoardDetail, Service};
use std::{collections::HashMap, path::PathBuf, time::Duration};

const POLL: Duration = Duration::from_secs(2);
const OPEN_BOARD_SETTING: &str = "native-board-open";
pub(crate) const ROLES: [(&str, &str); 4] = [
    ("todo", "To do"),
    ("doing", "Doing"),
    ("review", "Review"),
    ("done", "Done"),
];

pub(crate) enum Event {
    Close,
}

/// What a card's link chip says about the work it points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkLabel {
    Found(String),
    Missing,
}

pub(crate) struct Composer {
    pub column_id: String,
    pub input: Entity<Input>,
}

enum Menu {
    Switcher,
    Column { column_id: String, roles: bool },
}

enum Renaming {
    Board(String),
    Column(String),
}

pub(crate) struct BoardView {
    store: Database,
    db_path: PathBuf,
    service: Option<Service>,
    focus: FocusHandle,
    visible: bool,
    pub(crate) boards: Vec<Board>,
    pub(crate) open_board_id: Option<String>,
    pub(crate) detail: Option<BoardDetail>,
    pub(crate) dragging: bool,
    pub(crate) pending_reload: bool,
    pub(crate) last_data_version: i64,
    links: HashMap<String, LinkLabel>,
    pub(crate) composer: Option<Composer>,
    menu: Option<Menu>,
    renaming: Option<(Renaming, Entity<Input>)>,
    switcher_anchor: AnchorSlot,
    column_anchors: HashMap<String, AnchorSlot>,
    error: Option<String>,
    _poll: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<Event> for BoardView {}
impl Focusable for BoardView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl BoardView {
    pub(crate) fn new(
        store: Database,
        db_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let poll = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(POLL).await;
                if this.update(cx, |this, cx| this.poll(cx)).is_err() {
                    break;
                }
            }
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if window.is_window_active() && this.visible {
                this.reload(cx);
            }
        });
        Self {
            store,
            db_path,
            service: None,
            focus: cx.focus_handle(),
            visible: false,
            boards: Vec::new(),
            open_board_id: None,
            detail: None,
            dragging: false,
            pending_reload: false,
            last_data_version: 0,
            links: HashMap::new(),
            composer: None,
            menu: None,
            renaming: None,
            switcher_anchor: AnchorSlot::default(),
            column_anchors: HashMap::new(),
            error: None,
            _poll: poll,
            _subscriptions: vec![activation],
        }
    }

    /// The view came on screen: start polling and show the latest data.
    pub(crate) fn shown(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.reload(cx);
    }

    /// The view left the screen: stop polling and drop transient editors.
    pub(crate) fn hidden(&mut self, cx: &mut Context<Self>) {
        self.visible = false;
        self.composer = None;
        self.menu = None;
        self.renaming = None;
        cx.notify();
    }

    #[allow(dead_code)]
    pub(crate) fn set_dragging(&mut self, dragging: bool, cx: &mut Context<Self>) {
        self.dragging = dragging;
        if !dragging && self.pending_reload {
            self.reload(cx);
        }
    }

    /// Re-read the boards, the open board and its links. Deferred while a drag
    /// is in progress so a reload never yanks a card out from under the pointer.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        if self.dragging {
            self.pending_reload = true;
            return;
        }
        self.pending_reload = false;
        if let Err(error) = self.load() {
            self.error = Some(error);
        }
        cx.notify();
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        let Ok(service) = self.service() else {
            return;
        };
        if service
            .data_version()
            .is_ok_and(|version| version != self.last_data_version)
        {
            self.reload(cx);
        }
    }

    fn service(&mut self) -> Result<&mut Service, String> {
        if self.service.is_none() {
            self.service = Some(Service::open(&self.db_path)?);
        }
        Ok(self.service.as_mut().expect("opened above"))
    }

    fn load(&mut self) -> Result<(), String> {
        let wanted = self
            .open_board_id
            .clone()
            .or_else(|| self.remembered_board());
        let service = self.service()?;
        let boards = service.list()?;
        let open = wanted
            .filter(|id| boards.iter().any(|board| &board.id == id))
            .or_else(|| boards.first().map(|board| board.id.clone()));
        let detail = match &open {
            Some(id) => Some(service.get(id)?),
            None => None,
        };
        self.last_data_version = service.data_version()?;
        self.boards = boards;
        self.open_board_id = open;
        self.detail = detail;
        self.links = self.resolve_links();
        Ok(())
    }

    fn remembered_board(&self) -> Option<String> {
        locked(&self.store).ok()?.setting(OPEN_BOARD_SETTING).ok()?
    }

    fn remember_board(&self) {
        if let (Some(id), Ok(store)) = (&self.open_board_id, locked(&self.store)) {
            let _ = store.save_setting(OPEN_BOARD_SETTING, id);
        }
    }

    fn resolve_links(&self) -> HashMap<String, LinkLabel> {
        let Some(detail) = &self.detail else {
            return HashMap::new();
        };
        let Ok(store) = locked(&self.store) else {
            return HashMap::new();
        };
        let sessions = store.sessions().unwrap_or_default();
        detail
            .cards
            .iter()
            .filter_map(|card| {
                let (kind, id) = (card.link_kind.as_deref()?, card.link_id.as_deref()?);
                let label = match kind {
                    "session" => store
                        .resolve_existing_session_id(id)
                        .and_then(|native| sessions.iter().find(|s| s.id == native))
                        .map(|session| LinkLabel::Found(session.title.clone()))
                        .unwrap_or(LinkLabel::Missing),
                    "pilot" => LinkLabel::Found("Pilot".into()),
                    _ => LinkLabel::Missing,
                };
                Some((card.id.clone(), label))
            })
            .collect()
    }

    pub(crate) fn link_label(&self, card_id: &str) -> Option<LinkLabel> {
        self.links.get(card_id).cloned()
    }

    /// Why a column can't be deleted, or `None` when it can.
    pub(crate) fn delete_blocked(&self, column_id: &str) -> Option<String> {
        let count = self
            .detail
            .as_ref()?
            .cards
            .iter()
            .filter(|card| card.column_id == column_id)
            .count();
        (count > 0).then(|| {
            let plural = if count == 1 { "card" } else { "cards" };
            format!("Column has {count} {plural}")
        })
    }

    /// Run a write, then show the result (or its error) from a fresh reload.
    fn write(
        &mut self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut Service) -> Result<(), String>,
    ) {
        match self.service().and_then(action) {
            Ok(()) => self.error = None,
            Err(error) => self.error = Some(error),
        }
        self.reload(cx);
    }

    fn create_board(&mut self, cx: &mut Context<Self>) {
        match self.service().and_then(|service| service.create("Board")) {
            Ok(board) => {
                self.open_board_id = Some(board.id);
                self.remember_board();
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
        self.reload(cx);
    }

    fn open_board(&mut self, board_id: String, cx: &mut Context<Self>) {
        self.open_board_id = Some(board_id);
        self.remember_board();
        self.menu = None;
        self.composer = None;
        self.reload(cx);
    }

    fn delete_board(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(board) = self.detail.as_ref().map(|detail| detail.board.clone()) else {
            return;
        };
        self.menu = None;
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            &format!("Delete “{}”?", board.name),
            Some("Its columns, cards and activity are removed."),
            &["Delete", "Cancel"],
            cx,
        );
        cx.spawn(async move |this, cx| {
            if answer.await == Ok(0) {
                let _ = this.update(cx, |this, cx| {
                    this.open_board_id = None;
                    this.write(cx, |service| service.delete(&board.id));
                });
            }
        })
        .detach();
    }

    fn add_column(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(board_id) = self.open_board_id.clone() else {
            return;
        };
        match self
            .service()
            .and_then(|service| service.column_create(&board_id, "New column", None))
        {
            Ok(column) => {
                self.reload(cx);
                self.start_rename(Renaming::Column(column.id), "New column", window, cx);
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
            }
        }
    }

    pub(crate) fn open_composer(
        &mut self,
        column_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| Input::field("New card", "Card title", content(0.4), cx));
        self._subscriptions
            .push(cx.subscribe(&input, |this, _, _: &input::Submitted, cx| {
                this.commit_composer(cx)
            }));
        window.focus(&input.focus_handle(cx), cx);
        self.composer = Some(Composer { column_id, input });
        self.menu = None;
        cx.notify();
    }

    fn commit_composer(&mut self, cx: &mut Context<Self>) {
        let Some(composer) = &self.composer else {
            return;
        };
        let title = composer.input.read(cx).content.trim().to_string();
        if title.is_empty() {
            return;
        }
        let column_id = composer.column_id.clone();
        self.composer = None;
        self.write(cx, |service| {
            service.card_create(&column_id, &title, "").map(drop)
        });
    }

    fn start_rename(
        &mut self,
        target: Renaming,
        current: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| Input::field("Name", "Name", content(0.4), cx));
        input.update(cx, |input, cx| input.set_text(current.to_string(), cx));
        self._subscriptions
            .push(cx.subscribe(&input, |this, _, _: &input::Submitted, cx| {
                this.commit_rename(cx)
            }));
        window.focus(&input.focus_handle(cx), cx);
        self.renaming = Some((target, input));
        self.menu = None;
        cx.notify();
    }

    fn commit_rename(&mut self, cx: &mut Context<Self>) {
        let Some((target, input)) = self.renaming.take() else {
            return;
        };
        let name = input.read(cx).content.trim().to_string();
        if name.is_empty() {
            cx.notify();
            return;
        }
        match target {
            Renaming::Board(id) => self.write(cx, |service| service.rename(&id, &name)),
            Renaming::Column(id) => {
                let role = self.column(&id).and_then(|column| column.role.clone());
                self.write(cx, |service| {
                    service.column_update(&id, &name, role.as_deref())
                })
            }
        }
    }

    fn set_role(&mut self, column_id: String, role: Option<&str>, cx: &mut Context<Self>) {
        let Some(name) = self.column(&column_id).map(|column| column.name.clone()) else {
            return;
        };
        self.menu = None;
        self.write(cx, |service| service.column_update(&column_id, &name, role));
    }

    fn delete_column(&mut self, column_id: String, cx: &mut Context<Self>) {
        if self.delete_blocked(&column_id).is_some() {
            return;
        }
        self.menu = None;
        self.write(cx, |service| service.column_delete(&column_id));
    }

    fn open_column_menu(&mut self, column_id: String, cx: &mut Context<Self>) {
        self.menu = Some(Menu::Column {
            column_id,
            roles: false,
        });
        cx.notify();
    }

    fn column(&self, column_id: &str) -> Option<&rebase_core::board::Column> {
        self.detail
            .as_ref()?
            .columns
            .iter()
            .find(|column| column.id == column_id)
    }

    fn renaming_column(&self, column_id: &str) -> Option<Entity<Input>> {
        match &self.renaming {
            Some((Renaming::Column(id), input)) if id == column_id => Some(input.clone()),
            _ => None,
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key != "escape" {
            return;
        }
        cx.stop_propagation();
        if self.composer.take().is_some()
            || self.renaming.take().is_some()
            || self.menu.take().is_some()
        {
            cx.notify();
        } else {
            cx.emit(Event::Close);
        }
    }

    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let name: SharedString = self
            .detail
            .as_ref()
            .map(|detail| detail.board.name.clone())
            .unwrap_or_default()
            .into();
        let board_rename = match &self.renaming {
            Some((Renaming::Board(_), input)) => Some(input.clone()),
            _ => None,
        };
        div()
            .h(px(PANE_HEADER_HEIGHT))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap_2()
            .border_b_1()
            .border_color(content(0.07))
            .child(match board_rename {
                Some(input) => div().w(px(220.)).child(input).into_any_element(),
                None => div()
                    .id("board-switcher")
                    .debug_selector(|| "board-switcher".into())
                    .relative()
                    .role(Role::Button)
                    .aria_label("Switch board")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .flex()
                    .items_center()
                    .gap_1()
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .child(self.switcher_anchor.probe())
                    .child(icon("kanban", 14.).text_color(content(0.6)))
                    .child(
                        div()
                            .text_size(px(13.))
                            .font_weight(FontWeight::MEDIUM)
                            .child(name),
                    )
                    .child(icon("chevron-down", 12.).text_color(content(0.45)))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.menu = Some(Menu::Switcher);
                        cx.notify();
                    }))
                    .into_any_element(),
            })
            .child(div().flex_1())
            .child(
                div()
                    .id("board-new")
                    .debug_selector(|| "board-new".into())
                    .role(Role::Button)
                    .aria_label("New board")
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .text_size(px(12.))
                    .text_color(content(0.7))
                    .cursor_pointer()
                    .hover(|s| s.bg(content(0.06)))
                    .child("New board")
                    .on_click(cx.listener(|this, _, _, cx| this.create_board(cx))),
            )
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let weak = cx.weak_entity();
        let (id, anchor) = match menu {
            Menu::Switcher => ("board-switcher-menu", self.switcher_anchor.clone()),
            Menu::Column { column_id, .. } => (
                "board-column-menu",
                self.column_anchors
                    .get(column_id)
                    .cloned()
                    .unwrap_or_default(),
            ),
        };
        let list = match menu {
            Menu::Switcher => self.switcher_items(cx),
            Menu::Column {
                column_id,
                roles: true,
            } => self.role_items(column_id.clone(), cx),
            Menu::Column { column_id, .. } => self.column_items(column_id.clone(), cx),
        };
        Some(
            popover(id, Anchor::Slot(anchor))
                .width(200.)
                .on_dismiss(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.menu = None;
                        cx.notify();
                    });
                })
                .child(div().p(px(4.)).flex().flex_col().children(list))
                .into_any_element(),
        )
    }

    fn switcher_items(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let mut items: Vec<AnyElement> = self
            .boards
            .iter()
            .map(|board| {
                let id = board.id.clone();
                let open = self.open_board_id.as_ref() == Some(&board.id);
                menu_row(
                    format!("board-pick-{}", board.id).into(),
                    board.name.clone(),
                )
                .when(open, |d| d.bg(content(0.06)))
                .on_click(cx.listener(move |this, _, _, cx| this.open_board(id.clone(), cx)))
                .into_any_element()
            })
            .collect();
        if let Some(detail) = &self.detail {
            let name = detail.board.name.clone();
            let board_id = detail.board.id.clone();
            items.push(div().h(px(1.)).my_1().bg(content(0.07)).into_any_element());
            items.push(
                menu_row("board-rename".into(), "Rename board")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start_rename(Renaming::Board(board_id.clone()), &name, window, cx)
                    }))
                    .into_any_element(),
            );
            items.push(
                menu_row("board-delete".into(), "Delete board")
                    .text_color(diff_foreground(false))
                    .on_click(cx.listener(|this, _, window, cx| this.delete_board(window, cx)))
                    .into_any_element(),
            );
        }
        items
    }

    fn role_items(&self, column_id: String, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let current = self.column(&column_id).and_then(|c| c.role.clone());
        ROLES
            .into_iter()
            .map(|(role, label)| (Some(role), label))
            .chain([(None, "No role")])
            .map(|(role, label)| {
                let id = column_id.clone();
                let selected = current.as_deref() == role;
                menu_row(
                    format!("board-role-{}", role.unwrap_or("none")).into(),
                    label,
                )
                .when(selected, |d| d.bg(content(0.06)))
                .on_click(cx.listener(move |this, _, _, cx| this.set_role(id.clone(), role, cx)))
                .into_any_element()
            })
            .collect()
    }

    fn column_items(&self, column_id: String, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let name = self
            .column(&column_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();
        let blocked = self.delete_blocked(&column_id);
        let (rename_id, role_id, delete_id) = (column_id.clone(), column_id.clone(), column_id);
        vec![
            menu_row("board-column-rename".into(), "Rename")
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.start_rename(Renaming::Column(rename_id.clone()), &name, window, cx)
                }))
                .into_any_element(),
            menu_row("board-column-role".into(), "Set role")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.menu = Some(Menu::Column {
                        column_id: role_id.clone(),
                        roles: true,
                    });
                    cx.notify();
                }))
                .into_any_element(),
            menu_row("board-column-delete".into(), "Delete")
                .map(|d| match blocked {
                    Some(reason) => d
                        .text_color(content(0.35))
                        .cursor_default()
                        .aria_description(reason.clone())
                        .tooltip(move |_, cx| {
                            cx.new(|_| BoardTooltip(reason.clone().into())).into()
                        }),
                    None => d.text_color(diff_foreground(false)),
                })
                .on_click(
                    cx.listener(move |this, _, _, cx| this.delete_column(delete_id.clone(), cx)),
                )
                .into_any_element(),
        ]
    }
}

fn menu_row(id: SharedString, label: impl Into<SharedString>) -> gpui::Stateful<gpui::Div> {
    let label = label.into();
    let selector = id.clone();
    div()
        .id(id)
        .debug_selector(move || selector.to_string())
        .role(Role::MenuItem)
        .aria_label(label.clone())
        .px(px(8.))
        .py(px(6.))
        .rounded(px(6.))
        .text_size(px(13.))
        .cursor_pointer()
        .hover(|s| s.bg(content(0.07)))
        .child(label)
}

struct BoardTooltip(SharedString);
impl gpui::Render for BoardTooltip {
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

use gpui::AnyElement;

impl gpui::Render for BoardView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = div()
            .id("board-view")
            .debug_selector(|| "board-view".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.on_key(event, cx)))
            .size_full()
            .flex()
            .flex_col();
        let error = self.error.clone().map(|error| {
            div()
                .id("board-error")
                .debug_selector(|| "board-error".into())
                .px_4()
                .py_1()
                .text_size(px(12.))
                .text_color(diff_foreground(false))
                .child(error)
        });
        let Some(detail) = self.detail.clone() else {
            return root.child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(content(0.5))
                            .child("No boards yet"),
                    )
                    .child(
                        div()
                            .id("board-create")
                            .debug_selector(|| "board-create".into())
                            .role(Role::Button)
                            .aria_label("Create board")
                            .px_3()
                            .py_1()
                            .rounded_md()
                            .border_1()
                            .border_color(content(0.1))
                            .text_size(px(12.))
                            .cursor_pointer()
                            .hover(|s| s.bg(content(0.06)))
                            .child("Create board")
                            .on_click(cx.listener(|this, _, _, cx| this.create_board(cx))),
                    )
                    .children(error),
            );
        };
        for column in &detail.columns {
            self.column_anchors.entry(column.id.clone()).or_default();
        }
        let columns: Vec<AnyElement> = detail
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| self.render_column(index, column, &detail, cx))
            .collect();
        let menu = self.render_menu(cx);
        root.child(self.render_header(cx))
            .children(error)
            .child(
                div()
                    .id("board-columns")
                    .flex_1()
                    .min_h_0()
                    .overflow_x_scroll()
                    .px_4()
                    .py_3()
                    .flex()
                    .items_start()
                    .gap_2()
                    .children(columns)
                    .child(
                        icon_button("board-add-column", "Add column", "plus")
                            .debug_selector(|| "board-add-column".into())
                            .flex_shrink_0()
                            .on_click(
                                cx.listener(|this, _, window, cx| this.add_column(window, cx)),
                            ),
                    ),
            )
            .children(menu)
    }
}
