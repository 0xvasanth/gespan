//! The Board app view, over the same `board*` tables the Tauri app uses.
//! Writes from the Tauri app (or another window) show up when the view opens,
//! when the window becomes active, and when SQLite's `data_version` moves.
mod card;
mod column;
mod detail;
mod drop;
mod header;
pub(crate) mod style;
#[cfg(test)]
mod tests;
pub(crate) mod turns;

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
use rebase_core::board::{
    BoardDetail, Service,
    extras::{CardExtras, Label},
};
use std::{
    collections::{BTreeSet, HashMap},
    path::PathBuf,
    time::Duration,
};

const POLL: Duration = Duration::from_secs(2);
pub(crate) const ROLES: [(&str, &str); 4] = [
    ("todo", "To do"),
    ("doing", "Doing"),
    ("review", "Review"),
    ("done", "Done"),
];

pub(crate) enum Event {
    Close,
    OpenSession(i64),
    OpenPilot(String),
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
    Column { column_id: String, roles: bool },
}

struct CardMenu {
    card_id: String,
    menu: Entity<crate::popover::Menu>,
    _events: Subscription,
}

/// A card on the move. It is also the drag preview.
#[derive(Clone)]
pub(crate) struct DraggedCard {
    card_id: String,
    number: i64,
    title: String,
}

/// A column on the move, dragged by its header.
#[derive(Clone)]
pub(crate) struct DraggedColumn {
    column_id: String,
    index: usize,
    name: String,
}

enum Renaming {
    Column(String),
}

/// What the header's Filter narrows the board to. Empty sets match everything.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Filter {
    pub projects: BTreeSet<String>,
    pub labels: BTreeSet<String>,
    pub priorities: BTreeSet<u8>,
}

impl Filter {
    pub(crate) fn len(&self) -> usize {
        self.projects.len() + self.labels.len() + self.priorities.len()
    }
}

pub(crate) struct BoardView {
    store: Database,
    db_path: PathBuf,
    service: Option<Service>,
    focus: FocusHandle,
    visible: bool,
    pub(crate) open_board_id: Option<String>,
    pub(crate) detail: Option<BoardDetail>,
    pub(crate) dragging: bool,
    pub(crate) pending_reload: bool,
    pub(crate) last_data_version: i64,
    links: HashMap<String, LinkLabel>,
    pub(crate) composer: Option<Composer>,
    pub(crate) card_detail: Option<detail::CardDetail>,
    menu: Option<Menu>,
    renaming: Option<(Renaming, Entity<Input>)>,
    filter_anchor: AnchorSlot,
    pub(crate) extras: HashMap<String, CardExtras>,
    pub(crate) labels: Vec<Label>,
    /// Workspace projects as (path, name), for tagging and filtering.
    projects: Vec<(String, String)>,
    search: Entity<Input>,
    pub(crate) filter: Filter,
    filter_open: bool,
    column_anchors: HashMap<String, AnchorSlot>,
    card_anchors: HashMap<String, AnchorSlot>,
    dragged_card: Option<String>,
    /// The column and slot a dragged card would land in.
    pub(crate) drop_target: Option<(String, usize)>,
    /// One focus handle per card, so cards are tab stops.
    pub(crate) card_focus: HashMap<String, FocusHandle>,
    card_menu: Option<CardMenu>,
    /// The slot a dragged column would land in.
    column_drop: Option<usize>,
    /// The last write's error, or a message about a rejected edit.
    error: Option<String>,
    /// Why the last reload failed; cleared by the next one that succeeds.
    load_error: Option<String>,
    /// Runs between a reload's reads and the end of the load, so a test can
    /// commit from another connection exactly there.
    #[cfg(test)]
    pub(crate) after_load_reads: Option<Box<dyn FnMut()>>,
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
        let search = cx.new(|cx| Input::field("Search tasks", "Search tasks", content(0.4), cx));
        let searching = cx.observe(&search, |_, _, cx| cx.notify());
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
            open_board_id: None,
            detail: None,
            dragging: false,
            pending_reload: false,
            last_data_version: 0,
            links: HashMap::new(),
            composer: None,
            card_detail: None,
            menu: None,
            renaming: None,
            filter_anchor: AnchorSlot::default(),
            extras: HashMap::new(),
            labels: Vec::new(),
            projects: Vec::new(),
            search,
            filter: Filter::default(),
            filter_open: false,
            column_anchors: HashMap::new(),
            card_anchors: HashMap::new(),
            dragged_card: None,
            drop_target: None,
            card_focus: HashMap::new(),
            card_menu: None,
            column_drop: None,
            error: None,
            load_error: None,
            #[cfg(test)]
            after_load_reads: None,
            _poll: poll,
            _subscriptions: vec![activation, searching],
        }
    }

    /// The view came on screen: start polling and show the latest data.
    pub(crate) fn shown(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        self.reload(cx);
    }

    /// A drag that ended where the board never heard the release (over the
    /// sidebar, outside the window) still ends: GPUI's own drag state decides.
    /// True when it cleared one.
    fn forget_finished_drag(&mut self, cx: &App) -> bool {
        if !self.dragging || cx.has_active_drag() {
            return false;
        }
        self.dragging = false;
        self.dragged_card = None;
        self.drop_target = None;
        self.column_drop = None;
        true
    }

    /// The view left the screen: stop polling and drop transient editors.
    pub(crate) fn hidden(&mut self, cx: &mut Context<Self>) {
        self.forget_finished_drag(cx);
        self.visible = false;
        self.composer = None;
        self.menu = None;
        self.renaming = None;
        self.filter_open = false;
        cx.notify();
    }

    pub(crate) fn set_dragging(&mut self, dragging: bool, cx: &mut Context<Self>) {
        self.dragging = dragging;
        if !dragging && self.pending_reload {
            self.reload(cx);
        }
    }

    /// Re-read the boards, the open board and its links. Deferred while a drag
    /// is in progress so a reload never yanks a card out from under the pointer.
    pub(crate) fn reload(&mut self, cx: &mut Context<Self>) {
        self.forget_finished_drag(cx);
        if self.dragging {
            self.pending_reload = true;
            return;
        }
        self.pending_reload = false;
        self.load_error = self.load(cx).err();
        self.sync_card_detail(cx);
        cx.notify();
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        if self.forget_finished_drag(cx) && self.pending_reload {
            self.reload(cx);
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

    fn load(&mut self, cx: &App) -> Result<(), String> {
        let service = self.service()?;
        // Read the version first: a commit landing during the reads below
        // then shows up as a newer version on the next poll and reloads again.
        let version = service.data_version()?;
        let board = service.default_board()?;
        let detail = service.get(&board.id)?;
        let extras = service.extras(&board.id)?;
        let labels = service.labels(&board.id)?;
        #[cfg(test)]
        if let Some(hook) = self.after_load_reads.as_mut() {
            hook();
        }
        self.last_data_version = version;
        self.open_board_id = Some(board.id);
        self.detail = Some(detail);
        self.extras = extras;
        self.labels = labels;
        self.links = self.resolve_links(cx);
        self.sync_handles(cx);
        Ok(())
    }

    /// One anchor per column and card, and one focus handle per card, for
    /// exactly the cards and columns on the board now.
    fn sync_handles(&mut self, cx: &App) {
        let Some(detail) = &self.detail else {
            return;
        };
        self.column_anchors
            .retain(|id, _| detail.columns.iter().any(|column| &column.id == id));
        self.card_anchors
            .retain(|id, _| detail.cards.iter().any(|card| &card.id == id));
        self.card_focus
            .retain(|id, _| detail.cards.iter().any(|card| &card.id == id));
        for column in &detail.columns {
            self.column_anchors.entry(column.id.clone()).or_default();
        }
        for card in &detail.cards {
            self.card_anchors.entry(card.id.clone()).or_default();
            self.card_focus
                .entry(card.id.clone())
                .or_insert_with(|| cx.focus_handle());
        }
    }

    fn resolve_links(&self, cx: &App) -> HashMap<String, LinkLabel> {
        let Some(detail) = &self.detail else {
            return HashMap::new();
        };
        let Ok(store) = locked(&self.store) else {
            return HashMap::new();
        };
        let sessions = store.sessions().unwrap_or_default();
        let pilots = turns::pilots(cx);
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
                    "pilot" => pilots
                        .iter()
                        .find(|(pilot, _)| pilot == id)
                        .map(|(_, name)| LinkLabel::Found(name.clone()))
                        .unwrap_or(LinkLabel::Missing),
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
    ) -> bool {
        let result = self.service().and_then(action);
        let ok = result.is_ok();
        self.error = result.err();
        self.reload(cx);
        ok
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
            Renaming::Column(id) => {
                let role = self.column(&id).and_then(|column| column.role.clone());
                self.write(cx, |service| {
                    service.column_update(&id, &name, role.as_deref())
                });
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

    /// Menus take the board's focus so Escape closes them wherever focus was.
    fn open_column_menu(&mut self, column_id: String, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
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

    fn begin_card_drag(&mut self, card_id: String) {
        self.dragging = true;
        self.dragged_card = Some(card_id);
        self.menu = None;
    }

    fn begin_column_drag(&mut self) {
        self.dragging = true;
        self.menu = None;
    }

    /// A drag ended without a drop on the board (or after one): forget it and
    /// catch up on anything that changed meanwhile.
    fn end_drag(&mut self, cx: &mut Context<Self>) {
        self.dragged_card = None;
        self.drop_target = None;
        self.column_drop = None;
        self.set_dragging(false, cx);
        cx.notify();
    }

    fn card_drag_over(
        &mut self,
        column_id: &str,
        bounds: gpui::Bounds<gpui::Pixels>,
        pointer: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !bounds.contains(&pointer) {
            return;
        }
        let Some(detail) = &self.detail else {
            return;
        };
        let mut cards: Vec<_> = detail
            .cards
            .iter()
            .filter(|card| card.column_id == column_id && self.card_visible(card, cx))
            .collect();
        cards.sort_by_key(|card| card.position);
        let measured: Vec<_> = cards
            .iter()
            .filter_map(|card| self.card_anchors.get(&card.id)?.get())
            .collect();
        let tops: Vec<_> = measured.iter().map(|b| b.top()).collect();
        let heights: Vec<_> = measured.iter().map(|b| b.size.height).collect();
        let slot = drop::card_drop_index(&tops, &heights, pointer.y);
        let target = Some((column_id.to_string(), slot));
        if self.drop_target != target {
            self.drop_target = target;
            cx.notify();
        }
    }

    /// A slot among the cards on screen, as a slot in the whole column: before
    /// the visible card it points at, or after the last visible one.
    fn full_slot(&self, column_id: &str, slot: usize, cx: &App) -> usize {
        let Some(detail) = &self.detail else {
            return slot;
        };
        let mut cards: Vec<_> = detail
            .cards
            .iter()
            .filter(|card| card.column_id == column_id)
            .collect();
        cards.sort_by_key(|card| card.position);
        let visible: Vec<usize> = cards
            .iter()
            .enumerate()
            .filter(|(_, card)| self.card_visible(card, cx))
            .map(|(index, _)| index)
            .collect();
        match visible.get(slot) {
            Some(&index) => index,
            None => visible.last().map_or(cards.len(), |&last| last + 1),
        }
    }

    fn drop_card(&mut self, card: &DraggedCard, column_id: &str, cx: &mut Context<Self>) {
        let slot = match &self.drop_target {
            Some((target, slot)) if target == column_id => self.full_slot(column_id, *slot, cx),
            _ => self.detail.as_ref().map_or(0, |d| {
                d.cards.iter().filter(|c| c.column_id == column_id).count()
            }),
        };
        let resolved = self
            .detail
            .as_ref()
            .and_then(|detail| drop::resolve_card_drop(detail, &card.card_id, column_id, slot));
        self.dragging = false;
        self.dragged_card = None;
        self.drop_target = None;
        match resolved {
            Some((column, index)) => {
                let card_id = card.card_id.clone();
                self.write(cx, |service| {
                    service.card_move(&card_id, &column, index, "user")
                });
            }
            None => self.end_drag(cx),
        }
    }

    fn column_drag_over(
        &mut self,
        index: usize,
        bounds: gpui::Bounds<gpui::Pixels>,
        pointer: gpui::Point<gpui::Pixels>,
        cx: &mut Context<Self>,
    ) {
        if !bounds.contains(&pointer) {
            return;
        }
        let slot = if pointer.x < bounds.center().x {
            index
        } else {
            index + 1
        };
        if self.column_drop != Some(slot) {
            self.column_drop = Some(slot);
            cx.notify();
        }
    }

    fn drop_column(&mut self, column: &DraggedColumn, cx: &mut Context<Self>) {
        let resolved = self
            .column_drop
            .and_then(|slot| drop::resolve_column_drop(column.index, slot));
        self.dragging = false;
        self.column_drop = None;
        match resolved {
            Some(index) => {
                let column_id = column.column_id.clone();
                self.write(cx, |service| service.column_move(&column_id, index));
            }
            None => self.end_drag(cx),
        }
    }

    fn move_card_to(&mut self, card_id: String, column_id: String, cx: &mut Context<Self>) {
        self.menu = None;
        self.write(cx, |service| {
            service.card_move(&card_id, &column_id, usize::MAX, "user")
        });
    }

    /// The card's actions in the app's keyboard menu: open it, or move it to
    /// another column. Reached by clicking "…" or pressing Enter on a card.
    pub(crate) fn open_card_actions(
        &mut self,
        card_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(detail) = &self.detail else {
            return;
        };
        let current = detail
            .cards
            .iter()
            .find(|card| card.id == card_id)
            .map(|card| card.column_id.clone());
        let mut items = vec![
            crate::popover::MenuAction::new("open", "Open card").into(),
            crate::popover::MenuItem::Separator,
        ];
        let mut targets = HashMap::new();
        for (index, column) in detail.columns.iter().enumerate() {
            if Some(&column.id) == current.as_ref() {
                continue;
            }
            let id = format!("move-{index}");
            targets.insert(id.clone(), column.id.clone());
            items.push(
                crate::popover::MenuAction::new(id, format!("Move to {}", column.name)).into(),
            );
        }
        self.menu = None;
        let menu = cx.new(|cx| crate::popover::Menu::new("Card actions", items, window, cx));
        let events = cx.subscribe_in(
            &menu,
            window,
            move |this, _, event: &crate::popover::MenuEvent, window, cx| {
                let Some(open) = this.card_menu.take() else {
                    return;
                };
                if let crate::popover::MenuEvent::Picked(id, _) = event {
                    if id.as_ref() == "open" {
                        this.open_card_detail(open.card_id, window, cx);
                    } else if let Some(column_id) = targets.get(id.as_ref()) {
                        this.move_card_to(open.card_id, column_id.clone(), cx);
                    }
                }
                cx.notify();
            },
        );
        self.card_menu = Some(CardMenu {
            card_id,
            menu,
            _events: events,
        });
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, cx: &mut Context<Self>) {
        if event.keystroke.key != "escape" {
            return;
        }
        cx.stop_propagation();
        // Innermost first: transient editors and menus, then the drawer.
        if self.composer.take().is_some()
            || std::mem::take(&mut self.filter_open)
            || self.renaming.take().is_some()
            || self.menu.take().is_some()
            || self.close_detail_layer(cx)
            || self.close_card_detail(cx)
        {
            cx.notify();
        } else {
            cx.emit(Event::Close);
        }
    }

    fn render_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let menu = self.menu.as_ref()?;
        let weak = cx.weak_entity();
        let (id, anchor) = match menu {
            Menu::Column { column_id, .. } => (
                "board-column-menu",
                self.column_anchors
                    .get(column_id)
                    .cloned()
                    .unwrap_or_default(),
            ),
        };
        let list = match menu {
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

impl gpui::Render for DraggedCard {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(column::COLUMN_WIDTH - 24.))
            .px(px(12.))
            .py(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(content(0.16))
            .bg(pane_fill())
            .shadow_lg()
            .flex()
            .flex_col()
            .gap(px(6.))
            .child(
                div()
                    .text_size(px(13.))
                    .line_clamp(2)
                    .child(self.title.clone()),
            )
            .child(
                div()
                    .font_family("Menlo")
                    .text_size(px(11.))
                    .text_color(content(0.45))
                    .child(format!("#{}", self.number)),
            )
    }
}

impl gpui::Render for DraggedColumn {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .w(px(column::COLUMN_WIDTH))
            .h(px(36.))
            .px(px(10.))
            .rounded(px(8.))
            .border_1()
            .border_color(content(0.16))
            .bg(pane_fill())
            .shadow_lg()
            .flex()
            .items_center()
            .text_size(px(13.))
            .font_weight(FontWeight::MEDIUM)
            .child(self.name.clone())
    }
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
        self.forget_finished_drag(cx);
        let root = div()
            .id("board-view")
            .debug_selector(|| "board-view".into())
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| this.on_key(event, cx)))
            .on_mouse_up(
                gpui::MouseButton::Left,
                cx.listener(|this, _, _, cx| {
                    if this.dragging {
                        this.end_drag(cx);
                    }
                }),
            )
            .size_full()
            .flex();
        let error = self
            .error
            .clone()
            .or_else(|| self.load_error.clone())
            .map(|error| {
                div()
                    .id("board-error")
                    .debug_selector(|| "board-error".into())
                    .px_4()
                    .py_1()
                    .text_size(px(12.))
                    .text_color(diff_foreground(false))
                    .child(error)
            });
        let Some(detail) = self.detail.as_ref() else {
            return root.child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(self.render_header(cx))
                    .children(error),
            );
        };
        let columns: Vec<AnyElement> = detail
            .columns
            .iter()
            .enumerate()
            .map(|(index, column)| self.render_column(index, column, detail, cx))
            .collect();
        let menu = self.render_menu(cx);
        let card_menu = self.card_menu.as_ref().map(|open| {
            let anchor = self
                .card_anchors
                .get(&open.card_id)
                .cloned()
                .unwrap_or_default();
            crate::popover::menu_popover(&open.menu, Anchor::Slot(anchor), cx)
        });
        let card_detail = self.render_card_detail(cx);
        let filter = self.render_filter(cx);
        let board = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(self.render_header(cx))
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
            );
        root.child(board)
            .children(card_detail)
            .children(filter)
            .children(menu)
            .children(card_menu)
    }
}
