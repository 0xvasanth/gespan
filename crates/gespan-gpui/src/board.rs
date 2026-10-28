//! The Board app view, over the same `board*` tables the Tauri app uses.
//! Writes from the Tauri app (or another window) show up when the view opens,
//! when the window becomes active, and when SQLite's `data_version` moves.
mod agent_fields;
mod archive;
mod card;
mod column;
mod detail;
mod drop;
mod header;
mod keyboard;
pub(crate) mod link_status;
mod list;
mod markdown_toolbar;
mod new_card;
mod quick;
pub(crate) mod run_with;
mod selection;
mod settings;
pub(crate) mod style;
mod swimlanes;
mod switcher;
mod table;
#[cfg(test)]
mod tests;
mod tickets;
pub(crate) mod turns;
mod undo;
mod views;

use crate::{
    input::{self, Input},
    popover::{Anchor, AnchorSlot, popover},
    runtime::{Database, locked},
    theme::*,
    tickets::TicketBackend,
};
use gespan_core::{
    board::{
        BoardDetail, Service,
        extras::{
            BoardSettings, CardExtras, CardLink, DisplayOptions, Label, PriorityScheme, ViewSpec,
        },
    },
    tickets::{TicketRef, Tracker},
};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, KeyDownEvent,
    ListState, Pixels, Role, SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::Duration,
};

const POLL: Duration = Duration::from_secs(2);
/// The board store's error for a card id with no row.
const CARD_NOT_FOUND: &str = "Card not found";
/// How far past each edge of a board column's view cards are laid out.
pub(crate) const CARD_LIST_OVERDRAW: Pixels = px(240.);
pub(crate) const ROLES: [(&str, &str); 4] = [
    ("todo", "To do"),
    ("doing", "Doing"),
    ("review", "Review"),
    ("done", "Done"),
];

/// The board's own keys (⌘N for a new card).
pub(crate) fn bind(cx: &mut App) {
    new_card::bind(cx);
    keyboard::bind(cx);
}

pub(crate) enum Event {
    OpenSession(i64),
    OpenPilot(String),
    /// The drawer asks to link (or edit the link of) this card's ticket.
    LinkTicket(String),
    /// "Run with…" picked: start a session on the card.
    RunWith(run_with::RunRequest),
}

/// What the drawer's primary link row says about the work it points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LinkLabel {
    Found(String),
    Missing,
}

pub(crate) struct Composer {
    pub column_id: String,
    pub input: Entity<Input>,
    /// The field's focus handle: the column's list keeps the composer row
    /// mounted while it has focus, however far the cards push it down.
    pub focus: FocusHandle,
}

enum Menu {
    Column { column_id: String, roles: bool },
}

pub(super) struct CardMenu {
    card_id: String,
    /// Where the menu opens when it isn't under the card's own "…".
    anchor: Option<Anchor>,
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

/// A card lifted with Space: where Enter or Space would drop it. `slot` counts
/// the visible cards in `column_id`, including the lifted one.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct KeyboardDrag {
    pub card_id: String,
    pub column_id: String,
    pub slot: usize,
    /// The swimlane the card would drop into, on a board with lanes.
    pub lane: Option<String>,
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

/// The names of the Service reads a load or poll made, in order, so tests
/// can check a reload's reads don't grow with the board. Records nothing
/// outside tests.
#[derive(Clone, Default)]
pub(crate) struct ReadLog(#[cfg(test)] Rc<RefCell<Vec<&'static str>>>);

impl ReadLog {
    fn note(&self, _read: &'static str) {
        #[cfg(test)]
        self.0.borrow_mut().push(_read);
    }

    /// The reads since the last `take`.
    #[cfg(test)]
    pub(crate) fn take(&self) -> Vec<&'static str> {
        std::mem::take(&mut *self.0.borrow_mut())
    }
}

pub(crate) struct BoardView {
    store: Database,
    db_path: PathBuf,
    service: Option<Service>,
    focus: FocusHandle,
    visible: bool,
    pub(crate) open_board_id: Option<String>,
    /// The board the user picked this run; `load` prefers it.
    selected_board_id: Option<String>,
    /// Every board, for the switcher.
    boards: Vec<gespan_core::board::Board>,
    switcher: Option<switcher::Switcher>,
    switcher_anchor: AnchorSlot,
    switcher_focus: FocusHandle,
    pub(crate) detail: Option<BoardDetail>,
    pub(crate) dragging: bool,
    pub(crate) pending_reload: bool,
    /// Set by a load that had to open another board because the open one
    /// was deleted elsewhere; the reload then resets the board's UI.
    fell_back: bool,
    pub(crate) last_data_version: i64,
    /// Every non-ticket link of the board's live cards.
    link_rows: Vec<CardLink>,
    /// One chip per link, by card id, rebuilt from the reads below.
    pub(crate) chips: HashMap<String, Vec<link_status::LinkChip>>,
    /// The newest started/needs_input/completed/failed event per card.
    last_events: HashMap<String, String>,
    /// Linked sessions that exist, by session key.
    link_sessions: HashMap<String, link_status::SessionInfo>,
    /// Every Pilot as (id, name), `None` when there is no roster to read.
    pilot_names: Option<Vec<(String, String)>>,
    /// What the last background lookup found about PR and worktree links.
    link_extras: HashMap<(String, String), link_status::LinkExtra>,
    /// Where PR and worktree facts come from (`gh` and Git, or a test's).
    pub(crate) link_source: Arc<dyn link_status::LinkSource>,
    /// The board the PR and worktree lookup last ran for; cleared when the
    /// view hides, so opening the board looks again.
    status_board: Option<String>,
    status_task: Option<Task<()>>,
    /// A load asked for a PR and worktree lookup.
    status_due: bool,
    /// How many lookups started, for tests.
    pub(crate) status_lookups: usize,
    /// The running links and waiting sessions the chips and query last saw.
    turns_seen: (Vec<(String, String)>, BTreeSet<String>),
    /// The newest settled turn this view has looked at.
    settle_seen: u64,
    /// Blocked cards, by card id, to the numbers of their open blockers.
    pub(crate) blocked: BTreeMap<String, Vec<i64>>,
    /// Agent fields computed from each card's linked sessions.
    pub(crate) agent: HashMap<String, agent_fields::AgentValues>,
    run_options: run_with::RunOptions,
    pub(crate) run_picker: Option<run_with::RunPicker>,
    pub(crate) run_confirm: Option<run_with::RunConfirm>,
    /// The ticket each linked card shows a chip for.
    card_tickets: HashMap<String, TicketRef>,
    ticket_backend: Option<Arc<dyn TicketBackend>>,
    /// Trackers with saved credentials, once read.
    connected_trackers: Option<HashSet<Tracker>>,
    /// Cards whose last ticket read found the tracker disconnected.
    stale_tickets: HashSet<String>,
    /// The newest ticket read per card: (sequence, tracker, key).
    ticket_requests: HashMap<String, (u64, Tracker, String)>,
    ticket_sequence: u64,
    pub(crate) composer: Option<Composer>,
    new_card: Option<new_card::NewCard>,
    pub(crate) card_detail: Option<detail::CardDetail>,
    menu: Option<Menu>,
    renaming: Option<(Renaming, Entity<Input>)>,
    /// Enter in the card composer and the rename field; one each, replaced
    /// when the field is reopened.
    composer_submit: Option<Subscription>,
    rename_submit: Option<Subscription>,
    filter_anchor: AnchorSlot,
    pub(crate) extras: HashMap<String, CardExtras>,
    pub(crate) labels: Vec<Label>,
    /// The board's custom fields, for filter rows.
    pub(crate) fields: Vec<gespan_core::board::extras::Field>,
    /// Workspace projects as (path, name), for tagging and filtering.
    projects: Vec<(String, String)>,
    search: Entity<Input>,
    /// The filter rows and sort the board shows; the search text lives in
    /// `search` and joins them in `current_spec`.
    pub(crate) spec: ViewSpec,
    /// Cards `Service::query` matched, or `None` while nothing narrows.
    matched: Option<HashSet<String>>,
    /// A card's place in the query's order, for the sorted columns.
    rank: HashMap<String, usize>,
    /// Everything the list and table show for each card, by card id.
    index: HashMap<String, gespan_core::board::extras::CardIndexRow>,
    /// The cards picked for a bulk change.
    pub(crate) selection: selection::Selection,
    /// The bulk bar's menus and the layouts' own (columns, group by).
    pop: Option<selection::PopMenu>,
    pop_anchors: selection::PopAnchors,
    /// The table title being typed.
    table_edit: Option<table::TableEdit>,
    /// A table column being dragged wider or narrower.
    resizing: Option<table::Resize>,
    /// The search text the last query ran with.
    last_query: String,
    pub(crate) views: views::Views,
    filter_open: bool,
    /// The filter row showing its values to pick from.
    filter_row: Option<usize>,
    sort_open: bool,
    sort_anchor: AnchorSlot,
    sort_focus: FocusHandle,
    /// "Open cards in" (setting `board-card-open`): cards open as a full page
    /// instead of the drawer.
    pub(crate) open_in_page: bool,
    open_in_open: bool,
    open_in_anchor: AnchorSlot,
    /// The header's Filter and New task buttons are tab stops.
    filter_focus: FocusHandle,
    new_task_focus: FocusHandle,
    /// The open drawer's own focus: a click inside it lands here, not on the
    /// board, so the board's plain keys can tell the two apart.
    drawer_focus: FocusHandle,
    column_anchors: HashMap<String, AnchorSlot>,
    card_anchors: HashMap<String, AnchorSlot>,
    dragged_card: Option<String>,
    /// A card lifted from the keyboard; reloads wait for it like a mouse drag.
    pub(crate) kb_drag: Option<KeyboardDrag>,
    /// Ends the lift when focus leaves the lifted card.
    kb_blur: Option<Subscription>,
    /// The column and slot a dragged card would land in.
    pub(crate) drop_target: Option<(String, usize)>,
    /// One focus handle per card, so cards are tab stops.
    pub(crate) card_focus: HashMap<String, FocusHandle>,
    /// The card that had keyboard focus at the last render.
    focused_card_id: Option<String>,
    /// A focused card a reload or filter hid, and the card in its column that
    /// takes focus instead on the next render.
    refocus: Option<(String, String)>,
    /// The card whose drawer the column lists keep mounted.
    drawer_pinned: Option<String>,
    /// Whether the drawer card's row is spliced with the card's own focus
    /// handle (it has keyboard focus) rather than the drawer's.
    drawer_card_focused: bool,
    /// A column item a reveal scrolled to on estimated heights: (column,
    /// item, frames left to check it is fully in view).
    settling: Option<(String, usize, u8)>,
    /// What the last board-layout keyboard drag step said, for the status
    /// element screen readers read.
    drag_announcement: Option<String>,
    card_menu: Option<CardMenu>,
    /// One tab stop per collapsed column's strip.
    pub(crate) collapsed_focus: HashMap<String, FocusHandle>,
    /// One windowed list per column: only the cards in (or near) view mount.
    /// Item `i` is the column's `i`th visible card; the last item is the
    /// composer (or "Add card").
    pub(crate) column_lists: HashMap<String, ListState>,
    /// The visible card ids each column's list was last synced with.
    column_ids: HashMap<String, Vec<String>>,
    /// Each card's place in `detail.cards`, for the list items.
    card_slots: HashMap<String, usize>,
    /// The last painted height of each column item (card id, or a column's
    /// footer key), so a short column is exactly as tall as its cards.
    item_heights: Rc<RefCell<HashMap<String, Pixels>>>,
    /// Moves whenever `item_heights`, the column ids or the estimates
    /// change, so each column's summed height is redone only then.
    heights_version: Rc<std::cell::Cell<u64>>,
    /// Each column's body height as (version, estimate, height) when summed.
    body_heights: RefCell<HashMap<String, (u64, Pixels, Pixels)>>,
    /// The mean row height the shown cards' faces lay out, under a display:
    /// the first guess for unpainted cards before any has been measured.
    face_estimate: Option<(DisplayOptions, Pixels)>,
    /// The display options the lists were measured with.
    list_display: Option<DisplayOptions>,
    /// The mean painted card row height seen under each display, the first
    /// guess for unpainted cards when that display is back.
    learned_heights: Vec<(DisplayOptions, Pixels)>,
    /// Cards rendered since the start of the last board render.
    #[cfg(test)]
    pub(crate) rendered_cards: std::cell::Cell<usize>,
    /// The last parsed `views.layout`, with the JSON it was parsed from.
    layout_cache: RefCell<Option<(String, list::LayoutSpec)>>,
    /// How many times the view's `layout_json` was parsed.
    #[cfg(test)]
    pub(crate) layout_parses: std::cell::Cell<usize>,
    /// `(live, archived)` card counts per column, read with each reload.
    column_counts: HashMap<String, (usize, usize)>,
    /// The slot a dragged column would land in.
    column_drop: Option<usize>,
    /// The last write's error, or a message about a rejected edit.
    error: Option<String>,
    /// The error shown when the drawer's card went away elsewhere with text
    /// still unsaved, and that text, which the error's Copy button copies.
    pub(crate) orphaned_text: Option<(String, String)>,
    /// Why the last reload failed; cleared by the next one that succeeds.
    load_error: Option<String>,
    /// The undo toast and any delete it is holding.
    pub(crate) undo: undo::UndoController,
    /// Whether the Archive view is open, and the archived cards it read.
    archive_open: bool,
    archived: Vec<gespan_core::board::Card>,
    /// The settings page, when open, and the open board's settings.
    settings: Option<settings::Page>,
    pub(crate) board_settings: BoardSettings,
    /// False until the board's settings were read; the automation switches
    /// and rules stay disabled meanwhile.
    pub(crate) settings_loaded: bool,
    pub(crate) priority: PriorityScheme,
    /// The display options in force, refreshed at the start of each render.
    pub(crate) display: DisplayOptions,
    /// Values on the board's cards for text, number and date fields, as the
    /// stored JSON, by field id.
    field_seen: HashMap<String, Vec<String>>,
    /// The free-form value field in the filter list.
    filter_free: Entity<Input>,
    /// The swimlane cell (lane key, column id) a drag would drop into.
    pub(crate) lane_drop: Option<(String, String)>,
    /// Swimlanes as one windowed list of rows (a lane's header, then bands
    /// of `LANE_BAND` cards per cell); `None` on a board without lanes.
    pub(crate) lane_list: Option<ListState>,
    /// The rows, lanes and cells (shown card ids per `(lane, column)`) the
    /// lane list was last synced with, computed once per render.
    lane_rows_cache: Vec<swimlanes::LaneRow>,
    lanes_cache: Vec<swimlanes::Lane>,
    pub(crate) lane_cell_ids: swimlanes::LaneCells,
    /// Bumped whenever what the lanes are built from may have changed (the
    /// cards, the query, the board's fields and settings, the projects), so
    /// the lane list rebuilds only then, not on every render.
    pub(crate) lane_inputs_version: u64,
    /// The inputs version and lane mode the lane list was last built for.
    lane_synced: Option<(u64, String)>,
    /// How many times the lane list rebuilt its lanes and cells.
    #[cfg(test)]
    pub(crate) lane_rebuilds: std::cell::Cell<usize>,
    /// The focus handle each lane row keeps itself mounted by.
    lane_row_handles: Vec<Option<FocusHandle>>,
    /// Each lane header's focus, where focus goes when a filter hides every
    /// card of the focused card's cell.
    pub(crate) lane_header_focus: HashMap<String, FocusHandle>,
    /// A lane card a reveal scrolled to, and frames left to check it.
    lane_settling: Option<(String, u8)>,
    /// A focused card a reload or filter hid, and the lane whose header
    /// takes focus instead on the next render.
    lane_header_refocus: Option<(String, String)>,
    /// A card a keyboard lane drop moved, revealed once the lanes hold it.
    lane_reveal_pending: Option<String>,
    /// The board whose default view was applied; it applies once per board.
    default_view_applied: Option<String>,
    pending_default_view: Option<String>,
    /// Runs between a reload's reads and the end of the load, so a test can
    /// commit from another connection exactly there.
    #[cfg(test)]
    pub(crate) after_load_reads: Option<Box<dyn FnMut()>>,
    /// The Service reads loads and polls made, for tests that count them.
    pub(crate) reads: ReadLog,
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
        let searching = cx.observe(&search, |this, _, cx| {
            if this.query(cx) != this.last_query.trim().to_lowercase() {
                this.requery(cx);
            }
            cx.notify();
        });
        let activation = cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.cancel_keyboard_drag(cx);
            } else if this.visible {
                this.reload(cx);
            }
        });
        let open_in_page = locked(&store)
            .and_then(|db| db.setting(header::CARD_OPEN_SETTING))
            .ok()
            .flatten()
            .as_deref()
            == Some("page");
        let filter_free = cx.new(|cx| Input::field("Value", "Type a value", content(0.4), cx));
        let free_submit = cx.subscribe(&filter_free, |this, _, _: &input::Submitted, cx| {
            this.submit_filter_free(cx)
        });
        let live_turns = cx.observe_global::<turns::BoardTurns>(|this, cx| this.turns_changed(cx));
        let settle_seen = turns::settle_seq(cx);
        Self {
            store,
            db_path,
            service: None,
            focus: cx.focus_handle(),
            visible: false,
            open_board_id: None,
            selected_board_id: None,
            boards: Vec::new(),
            switcher: None,
            switcher_anchor: AnchorSlot::default(),
            switcher_focus: cx.focus_handle().tab_stop(true),
            detail: None,
            dragging: false,
            pending_reload: false,
            fell_back: false,
            last_data_version: 0,
            link_rows: Vec::new(),
            chips: HashMap::new(),
            last_events: HashMap::new(),
            link_sessions: HashMap::new(),
            pilot_names: None,
            link_extras: HashMap::new(),
            link_source: Arc::new(link_status::GitSource),
            status_board: None,
            status_task: None,
            status_due: false,
            status_lookups: 0,
            turns_seen: (Vec::new(), BTreeSet::new()),
            settle_seen,
            blocked: BTreeMap::new(),
            agent: HashMap::new(),
            run_options: run_with::RunOptions::default(),
            run_picker: None,
            run_confirm: None,
            card_tickets: HashMap::new(),
            ticket_backend: None,
            connected_trackers: None,
            stale_tickets: HashSet::new(),
            ticket_requests: HashMap::new(),
            ticket_sequence: 0,
            composer: None,
            new_card: None,
            card_detail: None,
            menu: None,
            renaming: None,
            composer_submit: None,
            rename_submit: None,
            filter_anchor: AnchorSlot::default(),
            extras: HashMap::new(),
            labels: Vec::new(),
            fields: Vec::new(),
            projects: Vec::new(),
            search,
            spec: ViewSpec::default(),
            matched: None,
            rank: HashMap::new(),
            index: HashMap::new(),
            selection: selection::Selection::default(),
            pop: None,
            pop_anchors: selection::PopAnchors::default(),
            table_edit: None,
            resizing: None,
            last_query: String::new(),
            views: views::Views::default(),
            filter_open: false,
            filter_row: None,
            sort_open: false,
            sort_anchor: AnchorSlot::default(),
            sort_focus: cx.focus_handle().tab_stop(true),
            open_in_page,
            open_in_open: false,
            open_in_anchor: AnchorSlot::default(),
            filter_focus: cx.focus_handle().tab_stop(true),
            new_task_focus: cx.focus_handle().tab_stop(true),
            drawer_focus: cx.focus_handle(),
            column_anchors: HashMap::new(),
            card_anchors: HashMap::new(),
            dragged_card: None,
            kb_drag: None,
            kb_blur: None,
            drop_target: None,
            card_focus: HashMap::new(),
            focused_card_id: None,
            refocus: None,
            drawer_pinned: None,
            drawer_card_focused: false,
            drag_announcement: None,
            settling: None,
            card_menu: None,
            collapsed_focus: HashMap::new(),
            column_lists: HashMap::new(),
            column_ids: HashMap::new(),
            card_slots: HashMap::new(),
            item_heights: Rc::default(),
            heights_version: Rc::default(),
            body_heights: RefCell::default(),
            face_estimate: None,
            list_display: None,
            learned_heights: Vec::new(),
            #[cfg(test)]
            rendered_cards: std::cell::Cell::new(0),
            layout_cache: RefCell::default(),
            #[cfg(test)]
            layout_parses: std::cell::Cell::new(0),
            column_counts: HashMap::new(),
            column_drop: None,
            error: None,
            load_error: None,
            orphaned_text: None,
            undo: undo::UndoController::default(),
            archive_open: false,
            archived: Vec::new(),
            settings: None,
            board_settings: BoardSettings::default(),
            settings_loaded: false,
            priority: PriorityScheme::default(),
            display: DisplayOptions::default(),
            field_seen: HashMap::new(),
            filter_free,
            lane_drop: None,
            lane_list: None,
            lane_rows_cache: Vec::new(),
            lanes_cache: Vec::new(),
            lane_cell_ids: HashMap::new(),
            lane_inputs_version: 0,
            lane_synced: None,
            #[cfg(test)]
            lane_rebuilds: std::cell::Cell::new(0),
            lane_row_handles: Vec::new(),
            lane_header_focus: HashMap::new(),
            lane_settling: None,
            lane_header_refocus: None,
            lane_reveal_pending: None,
            default_view_applied: None,
            pending_default_view: None,
            #[cfg(test)]
            after_load_reads: None,
            reads: ReadLog::default(),
            _poll: poll,
            _subscriptions: vec![activation, searching, free_submit, live_turns],
        }
    }

    #[cfg(test)]
    pub(crate) fn new_card_open(&self) -> bool {
        self.new_card.is_some()
    }

    #[cfg(test)]
    pub(crate) fn switcher_open(&self) -> bool {
        self.switcher.is_some()
    }

    /// The view came on screen: start polling and show the latest data.
    pub(crate) fn shown(&mut self, cx: &mut Context<Self>) {
        self.visible = true;
        // Coming on screen is opening the board: PRs and worktrees are looked
        // up again, even after a reload that ran while it was hidden.
        self.status_board = None;
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
        self.lane_drop = None;
        true
    }

    /// The view left the screen: stop polling and drop transient editors.
    pub(crate) fn hidden(&mut self, cx: &mut Context<Self>) {
        self.forget_finished_drag(cx);
        self.visible = false;
        self.cancel_keyboard_drag(cx);
        self.drag_announcement = None;
        self.composer = None;
        self.new_card = None;
        self.menu = None;
        self.card_menu = None;
        self.pop = None;
        self.table_edit = None;
        self.resizing = None;
        self.renaming = None;
        self.filter_open = false;
        self.sort_open = false;
        self.views.menu = None;
        self.views.naming = None;
        self.open_in_open = false;
        self.switcher = None;
        self.settings = None;
        self.run_picker = None;
        self.run_confirm = None;
        // Opening the board again looks PRs and worktrees up again.
        self.status_board = None;
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
        if self.dragging || self.kb_drag.is_some() {
            self.pending_reload = true;
            return;
        }
        self.pending_reload = false;
        self.load_error = self.load(cx).err();
        if std::mem::take(&mut self.fell_back) {
            self.reset_after_fallback(cx);
        }
        if std::mem::take(&mut self.status_due) && self.load_error.is_none() {
            self.refresh_link_status(cx);
        }
        if self.load_error.is_none() && std::mem::take(&mut self.views.reapply) {
            let active = self.views.active.clone();
            self.select_view(active, cx);
        }
        if self.load_error.is_none()
            && let Some(view) = self.pending_default_view.take()
        {
            self.select_view(Some(view), cx);
        }
        self.sync_card_detail(cx);
        if self.load_error.is_none() && self.card_detail.is_some() {
            self.refresh_detail_lists();
        }
        cx.notify();
    }

    /// The open board was deleted elsewhere and `load` opened another: what
    /// belonged to the old board goes, as on a board switch. The undo toast
    /// goes too (a held delete went with the board), and so do the drawer,
    /// the composer, menus, the selection, the filter and the search.
    fn reset_after_fallback(&mut self, cx: &mut Context<Self>) {
        self.commit_pending(cx);
        self.card_detail = None;
        self.switcher = None;
        self.pop = None;
        self.table_edit = None;
        self.clear_selection();
        self.reset_view(cx);
        self.settings = None;
        self.filter_open = false;
        self.sort_open = false;
        self.card_menu = None;
        self.menu = None;
        self.composer = None;
        self.new_card = None;
        self.renaming = None;
    }

    fn poll(&mut self, cx: &mut Context<Self>) {
        if !self.visible {
            return;
        }
        if self.forget_finished_drag(cx) && self.pending_reload {
            self.reload(cx);
            return;
        }
        let lifted = self.kb_drag.as_ref().map(|drag| drag.card_id.clone());
        if let (Some(card_id), Some(board_id)) = (lifted, self.open_board_id.clone()) {
            // Reloads wait for a lift, but a lifted card deleted, archived or
            // moved to another board elsewhere ends it. Only that card is
            // read, not the whole board.
            if self.lifted_card_gone(&card_id, &board_id) {
                self.cancel_keyboard_drag(cx);
                self.reload(cx);
                return;
            }
        }
        let reads = self.reads.clone();
        let Ok(service) = self.service() else {
            return;
        };
        reads.note("data_version");
        if service
            .data_version()
            .is_ok_and(|version| version != self.last_data_version)
        {
            self.reload(cx);
        }
    }

    /// Whether the lifted card left the board elsewhere: deleted, archived
    /// or moved to another board. A read that fails for another reason
    /// keeps the lift.
    fn lifted_card_gone(&mut self, card_id: &str, board_id: &str) -> bool {
        let reads = self.reads.clone();
        let Ok(service) = self.service() else {
            return false;
        };
        reads.note("card");
        match service.card(card_id) {
            Ok(card) if card.board_id != board_id => true,
            Ok(_) => {
                reads.note("card_meta");
                service
                    .card_meta(card_id)
                    .is_ok_and(|meta| meta.archived_at.is_some())
            }
            Err(error) => error == CARD_NOT_FOUND,
        }
    }

    /// Adds a card to the first column, as a test fixture outside this module.
    #[cfg(test)]
    pub(crate) fn add_test_card(&mut self, title: &str, cx: &mut Context<Self>) {
        let column = self.detail.as_ref().expect("a board is open").columns[0]
            .id
            .clone();
        self.service()
            .and_then(|service| service.card_create(&column, title, ""))
            .expect("the card is created");
        self.reload(cx);
    }

    fn service(&mut self) -> Result<&mut Service, String> {
        if self.service.is_none() {
            self.service = Some(Service::open(&self.db_path)?);
        }
        Ok(self.service.as_mut().expect("opened above"))
    }

    fn load(&mut self, cx: &App) -> Result<(), String> {
        // The remembered board only matters until the user picks one.
        let remembered = match self.selected_board_id {
            Some(_) => None,
            None => locked(&self.store)
                .and_then(|db| db.setting(switcher::SETTING))
                .ok()
                .flatten(),
        };
        let wanted = self.selected_board_id.clone().or(remembered);
        let archive_open = self.archive_open;
        let reads = self.reads.clone();
        let service = self.service()?;
        // Read the version first: a commit landing during the reads below
        // then shows up as a newer version on the next poll and reloads again.
        reads.note("data_version");
        let version = service.data_version()?;
        reads.note("list");
        let boards = service.list()?;
        let board = match wanted.and_then(|id| boards.iter().find(|b| b.id == id).cloned()) {
            Some(board) => board,
            None => {
                reads.note("default_board");
                service.default_board()?
            }
        };
        let boards = if boards.iter().any(|b| b.id == board.id) {
            boards
        } else {
            reads.note("list");
            service.list()?
        };
        reads.note("get");
        let detail = service.get(&board.id)?;
        // Every column's counts in one read, however many columns there are.
        reads.note("column_card_counts");
        let column_counts = service.column_card_counts(&board.id)?;
        reads.note("extras");
        let extras = service.extras(&board.id)?;
        reads.note("labels");
        let labels = service.labels(&board.id)?;
        reads.note("board_tickets");
        let card_tickets = service.board_tickets(&board.id)?;
        reads.note("views");
        let views = service.views(&board.id)?;
        reads.note("list_fields");
        let fields = service.list_fields(&board.id)?;
        // A settings row that can't be read is not fatal to the board, but the
        // automation section must not write over what it could not see.
        reads.note("settings");
        let settings_read = service.settings(&board.id);
        let settings_loaded = settings_read.is_ok();
        let board_settings = settings_read.unwrap_or_default();
        let mut field_seen = HashMap::new();
        for field in fields
            .iter()
            .filter(|field| matches!(field.kind.as_str(), "text" | "number" | "date" | "url"))
        {
            reads.note("field_distinct_values");
            field_seen.insert(
                field.id.clone(),
                service.field_distinct_values(&board.id, &field.id, 50)?,
            );
        }
        reads.note("card_index");
        let index: HashMap<_, _> = service
            .card_index(&board.id)?
            .into_iter()
            .map(|row| (row.card_id.clone(), row))
            .collect();
        let archived = if archive_open {
            reads.note("archived");
            service.archived(&board.id)?
        } else {
            Vec::new()
        };
        reads.note("board_links");
        let link_rows = service.board_links(&board.id)?;
        reads.note("blocked");
        let blocked = service.blocked(&board.id)?;
        // Only the "failed" chip state needs this; a read that fails leaves
        // the chips without it rather than failing the whole board.
        reads.note("latest_run_events");
        let last_events = service
            .latest_run_events(&board.id)
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "reading the board's run events failed");
                HashMap::new()
            });
        #[cfg(test)]
        if let Some(hook) = self.after_load_reads.as_mut() {
            hook();
        }
        self.last_data_version = version;
        // The open board went away elsewhere and another one opens instead.
        // A board the user picked (or a fresh start) is not a fallback.
        self.fell_back = self
            .open_board_id
            .as_ref()
            .is_some_and(|open| *open != board.id)
            && self.selected_board_id.as_ref() != Some(&board.id);
        self.selected_board_id = Some(board.id.clone());
        self.open_board_id = Some(board.id);
        self.boards = boards;
        self.detail = Some(detail);
        self.column_counts = column_counts;
        self.extras = extras;
        self.index = index;
        self.labels = labels;
        self.card_tickets = card_tickets;
        self.archived = archived;
        self.link_rows = link_rows;
        self.blocked = blocked;
        self.last_events = last_events;
        self.read_link_sessions(cx)?;
        self.rebuild_chips(cx);
        self.sync_handles(cx);
        self.prune_selection();
        if self
            .table_edit
            .as_ref()
            .is_some_and(|edit| self.card(&edit.card_id).is_none())
        {
            // The card was deleted or moved away elsewhere.
            self.table_edit = None;
        }
        self.field_seen = field_seen;
        self.fields = fields;
        self.priority = gespan_core::board::extras::priority_scheme(&board_settings);
        self.sync_views(views, cx);
        // A board opens on its default view once. A view deleted since, or
        // never saved, leaves "All cards".
        let board_id = self.open_board_id.clone();
        if self.default_view_applied != board_id {
            self.default_view_applied = board_id;
            self.pending_default_view = board_settings
                .default_view_id
                .clone()
                .filter(|id| self.views.saved.iter().any(|view| &view.id == id));
        }
        self.board_settings = board_settings;
        self.settings_loaded = settings_loaded;
        if self.visible && self.status_board != self.open_board_id {
            // The board opened (or another was picked): look PRs and
            // worktrees up once the load is done.
            self.status_due = true;
        }
        // A label or column deleted elsewhere is dropped by the query itself,
        // so a saved filter never hides every card unseen.
        self.refresh_query(cx)
    }

    /// One anchor per column and card, and one focus handle per card, for
    /// exactly the cards and columns on the board now.
    fn sync_handles(&mut self, cx: &App) {
        let Some(detail) = &self.detail else {
            return;
        };
        let columns: HashSet<&str> = detail.columns.iter().map(|c| c.id.as_str()).collect();
        let cards: HashSet<&str> = detail.cards.iter().map(|c| c.id.as_str()).collect();
        self.column_anchors
            .retain(|id, _| columns.contains(id.as_str()));
        self.card_anchors
            .retain(|id, _| cards.contains(id.as_str()));
        self.card_focus.retain(|id, _| cards.contains(id.as_str()));
        self.collapsed_focus
            .retain(|id, _| columns.contains(id.as_str()));
        // Painted heights of cards and columns no longer on the board.
        self.item_heights.borrow_mut().retain(|key, _| {
            cards.contains(key.as_str())
                || column::footer_column(key).is_some_and(|id| columns.contains(id))
        });
        self.heights_version.set(self.heights_version.get() + 1);
        for column in &detail.columns {
            self.column_anchors.entry(column.id.clone()).or_default();
            self.collapsed_focus
                .entry(column.id.clone())
                .or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
        for card in &detail.cards {
            self.card_anchors.entry(card.id.clone()).or_default();
            self.card_focus
                .entry(card.id.clone())
                .or_insert_with(|| cx.focus_handle().tab_stop(true));
        }
    }

    /// The drawer's primary link: the session or Pilot it names, or missing.
    pub(crate) fn link_label(&self, card_id: &str) -> Option<LinkLabel> {
        let card = self.card(card_id)?;
        let (kind, id) = (card.link_kind.as_deref()?, card.link_id.as_deref()?);
        Some(self.link_target(kind, id))
    }

    /// The session or Pilot a link names: its title, or missing when the
    /// work is gone (or the link is of another kind).
    pub(crate) fn link_target(&self, kind: &str, id: &str) -> LinkLabel {
        let found = match kind {
            "session" => self
                .link_sessions
                .get(id)
                .map(|session| session.title.clone()),
            "pilot" => self
                .pilot_names
                .as_ref()
                .and_then(|pilots| pilots.iter().find(|(pilot, _)| pilot == id))
                .map(|(_, name)| name.clone()),
            _ => None,
        };
        found.map_or(LinkLabel::Missing, LinkLabel::Found)
    }

    /// Why a column can't be deleted, or `None` when it can. Archived cards
    /// count: the store refuses to delete a column that still holds any.
    pub(crate) fn delete_blocked(&self, column_id: &str) -> Option<String> {
        let detail = self.detail.as_ref()?;
        let (live, archived) = self
            .column_counts
            .get(column_id)
            .copied()
            .unwrap_or_else(|| {
                let live = detail
                    .cards
                    .iter()
                    .filter(|card| card.column_id == column_id)
                    .count();
                (live, 0)
            });
        // A card whose delete the toast is holding goes when the column does.
        let held = self.undo.pending_delete_count_in(column_id);
        let count = live + archived - held.min(archived);
        (count > 0).then(|| {
            let plural = if count == 1 { "card" } else { "cards" };
            format!("Move or delete its {count} {plural} first")
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
                self.error = None;
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
        self.composer_submit = Some(cx.subscribe(&input, |this, _, _: &input::Submitted, cx| {
            this.commit_composer(cx)
        }));
        let focus = input.focus_handle(cx);
        window.focus(&focus, cx);
        // The composer is the column's last row: hand the list its focus
        // handle and bring it into view.
        if let (Some(count), Some(state)) = (
            self.column_ids.get(&column_id).map(Vec::len),
            self.column_lists.get(&column_id),
        ) {
            state.splice_focusable(count..count + 1, [Some(focus.clone())]);
            self.reveal_item(&column_id, count);
        }
        self.composer = Some(Composer {
            column_id,
            input,
            focus,
        });
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
        // The field is cleared only once the card exists, so a rejected title
        // stays to correct. The composer stays open (and focused) for the next
        // card; Escape closes it.
        let input = composer.input.clone();
        if self.write(cx, |service| {
            service.card_create(&column_id, &title, "").map(drop)
        }) {
            input.update(cx, |input, cx| input.set_text(String::new(), cx));
        }
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
        self.rename_submit = Some(cx.subscribe(&input, |this, _, _: &input::Submitted, cx| {
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
        match &target {
            Renaming::Column(id) => {
                let role = self.column(id).and_then(|column| column.role.clone());
                if !self.write(cx, |service| {
                    service.column_update(id, &name, role.as_deref())
                }) {
                    // Keep the rejected name in the field to correct.
                    self.renaming = Some((target, input));
                }
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
        if self.undo.pending_delete_count_in(&column_id) > 0 {
            self.commit_pending(cx);
        }
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

    fn column(&self, column_id: &str) -> Option<&gespan_core::board::Column> {
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
        self.kb_drag = None;
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
        self.lane_drop = None;
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
        if self.is_collapsed(column_id) {
            // A strip has no slots: a drop lands at the end of the column.
            let target = Some((column_id.to_string(), 0));
            if self.drop_target != target {
                self.drop_target = target;
                cx.notify();
            }
            return;
        }
        let Some(detail) = &self.detail else {
            return;
        };
        let cards = self.column_cards(detail, column_id);
        if self.sorted() {
            // Sorted columns keep their order: a card can't be placed inside
            // its own column, only appended to another.
            let own = self
                .dragged_card
                .as_ref()
                .is_some_and(|id| cards.iter().any(|card| &card.id == id));
            let target = (!own).then(|| (column_id.to_string(), cards.len()));
            if self.drop_target != target {
                self.drop_target = target;
                cx.notify();
            }
            return;
        }
        // Only the cards the column's window painted have bounds; slots still
        // count every card, so map the slot among painted cards back to the
        // visible list. A focused card kept mounted off screen paints outside
        // the column's view and is left out.
        let viewport = self
            .column_lists
            .get(column_id)
            .map(|state| state.viewport_bounds());
        let measured: Vec<(usize, gpui::Bounds<gpui::Pixels>)> = cards
            .iter()
            .enumerate()
            .filter_map(|(index, card)| Some((index, self.card_anchors.get(&card.id)?.get()?)))
            .filter(|(_, bounds)| viewport.is_none_or(|view| view.intersects(bounds)))
            .collect();
        let tops: Vec<_> = measured.iter().map(|(_, b)| b.top()).collect();
        let heights: Vec<_> = measured.iter().map(|(_, b)| b.size.height).collect();
        let slot = match drop::card_drop_index(&tops, &heights, pointer.y) {
            slot if slot < measured.len() => measured[slot].0,
            _ => measured.last().map_or(cards.len(), |(index, _)| index + 1),
        };
        let target = Some((column_id.to_string(), slot));
        if self.drop_target != target {
            self.drop_target = target;
            cx.notify();
        }
    }

    /// A slot among the cards on screen, as a slot in the whole column: before
    /// the visible card it points at, or after the last visible one.
    fn full_slot(&self, column_id: &str, slot: usize) -> usize {
        let Some(detail) = &self.detail else {
            return slot;
        };
        let mut cards: Vec<_> = detail
            .cards
            .iter()
            .filter(|card| card.column_id == column_id)
            .collect();
        cards.sort_by_key(|card| card.position);
        if self.is_collapsed(column_id) || self.sorted() {
            return cards.len();
        }
        let visible: Vec<usize> = cards
            .iter()
            .enumerate()
            .filter(|(_, card)| self.card_matches(card))
            .map(|(index, _)| index)
            .collect();
        match visible.get(slot) {
            Some(&index) => index,
            None => visible.last().map_or(cards.len(), |&last| last + 1),
        }
    }

    /// Where a drop lands, or `None` when it changes nothing. A sorted board
    /// has no order inside a column to change.
    fn resolve_drop(&self, card_id: &str, column_id: &str, slot: usize) -> Option<(String, usize)> {
        let detail = self.detail.as_ref()?;
        let inside = detail
            .cards
            .iter()
            .any(|card| card.id == card_id && card.column_id == column_id);
        if self.sorted() && inside {
            return None;
        }
        drop::resolve_card_drop(detail, card_id, column_id, slot)
    }

    fn drop_card(&mut self, card: &DraggedCard, column_id: &str, cx: &mut Context<Self>) {
        let slot = match &self.drop_target {
            Some((target, slot)) if target == column_id => self.full_slot(column_id, *slot),
            _ => self.detail.as_ref().map_or(0, |d| {
                d.cards.iter().filter(|c| c.column_id == column_id).count()
            }),
        };
        let resolved = self.resolve_drop(&card.card_id, column_id, slot);
        self.dragging = false;
        self.dragged_card = None;
        self.drop_target = None;
        match resolved {
            Some((column, index)) => {
                self.move_with_undo(&card.card_id, &column, index, cx);
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
        self.move_with_undo(&card_id, &column_id, usize::MAX, cx);
    }

    /// The card's actions in the app's keyboard menu: open it, or move it to
    /// another column. Reached by clicking "…" or pressing Enter on a card.
    pub(crate) fn open_card_actions(
        &mut self,
        card_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The menu anchors to the card: a focused card scrolled out of its
        // column's window comes back into view first.
        self.reveal_card(&card_id);
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
        let more_anchor = self.card_anchors.get(&card_id).cloned().unwrap_or_default();
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
        // With no Move rows (a one-column board) the separator after Open
        // already divides it from More….
        if !targets.is_empty() {
            items.push(crate::popover::MenuItem::Separator);
        }
        items.push(crate::popover::MenuAction::new("more", "More…").into());
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
                    } else if id.as_ref() == "more" {
                        let anchor = Anchor::Slot(more_anchor.clone());
                        this.open_more_menu(open.card_id, anchor, window, cx);
                    } else if let Some(column_id) = targets.get(id.as_ref()) {
                        this.move_card_to(open.card_id, column_id.clone(), cx);
                    }
                }
                cx.notify();
            },
        );
        self.card_menu = Some(CardMenu {
            card_id,
            anchor: None,
            menu,
            _events: events,
        });
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key != "escape" {
            return;
        }
        cx.stop_propagation();
        // A lifted card is the innermost layer: Escape puts it back, alone.
        if self.kb_drag.is_some() {
            self.cancel_keyboard_drag(cx);
            cx.notify();
            return;
        }
        // Innermost first: transient editors and menus, then the drawer.
        if self.cancel_run_confirm(cx)
            || self.close_run_picker(Some(window), cx)
            || self.close_new_card(Some(window), cx)
            || self.composer.take().is_some()
            || std::mem::take(&mut self.filter_open)
            || std::mem::take(&mut self.sort_open)
            || self.views.menu.take().is_some()
            || self.views.naming.take().is_some()
            || std::mem::take(&mut self.open_in_open)
            || self.switcher.take().is_some()
            || self.settings_escape(window, cx)
            || self.renaming.take().is_some()
            || self.pop.take().is_some()
            || self.table_edit.take().is_some()
            || self.menu.take().is_some()
            || self.close_detail_layer(cx)
            || self.leave_full_page(cx)
            || self.close_card_detail(cx)
            || self.clear_selection()
        {
            cx.notify();
        }
        // With nothing left to close, Escape does nothing, as in the Tauri
        // board: it never leaves the Board (its tab or sidebar entry does).
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
        let done_column = self.done_column_id();
        let is_done = done_column.as_deref() == Some(column_id.as_str());
        let can_mark_done = done_column.is_some() && !is_done;
        let (rename_id, role_id, delete_id) =
            (column_id.clone(), column_id.clone(), column_id.clone());
        let (collapse_id, priority_id, due_id) =
            (column_id.clone(), column_id.clone(), column_id.clone());
        let (done_id, archive_id, move_id) = (column_id.clone(), column_id.clone(), column_id);
        let divider = || {
            div()
                .my(px(4.))
                .h(px(1.))
                .bg(content(0.08))
                .into_any_element()
        };
        let mut items = vec![
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
            menu_row("menu-collapse".into(), "Collapse column")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.set_collapsed(collapse_id.clone(), true, cx)
                }))
                .into_any_element(),
            divider(),
            menu_row("menu-sort-priority".into(), "Sort by priority")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sort_column(priority_id.clone(), quick::Sort::Priority, cx)
                }))
                .into_any_element(),
            menu_row("menu-sort-due".into(), "Sort by due date")
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.sort_column(due_id.clone(), quick::Sort::DueDate, cx)
                }))
                .into_any_element(),
        ];
        if can_mark_done {
            items.push(
                menu_row("menu-mark-done".into(), "Mark all as done")
                    .on_click(
                        cx.listener(move |this, _, _, cx| this.mark_all_done(done_id.clone(), cx)),
                    )
                    .into_any_element(),
            );
        }
        if is_done {
            items.push(
                menu_row("menu-archive-done".into(), "Archive completed")
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.archive_completed(archive_id.clone(), cx)
                    }))
                    .into_any_element(),
            );
        }
        items.push(divider());
        items.push(
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
        );
        items.extend(self.move_then_delete_items(&move_id, cx));
        items
    }

    /// A column that holds only live cards offers "Move cards to…" each other
    /// column, then deletes itself. Archived cards still block the delete
    /// (C5-l), so nothing is offered then.
    fn move_then_delete_items(&self, column_id: &str, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(detail) = &self.detail else {
            return Vec::new();
        };
        let live = detail
            .cards
            .iter()
            .filter(|card| card.column_id == column_id)
            .count();
        let held = self.undo.pending_delete_count_in(column_id);
        let archived = self.column_counts.get(column_id).map_or(0, |c| c.1);
        if live == 0 || archived > held {
            return Vec::new();
        }
        let mut items = vec![
            div()
                .id("board-column-move-cards")
                .debug_selector(|| "board-column-move-cards".into())
                .px(px(8.))
                .pt(px(6.))
                .pb(px(2.))
                .text_size(px(12.))
                .text_color(content(0.5))
                .child("Move cards to…")
                .into_any_element(),
        ];
        for (index, column) in detail.columns.iter().enumerate() {
            if column.id == column_id {
                continue;
            }
            let (from, to) = (column_id.to_string(), column.id.clone());
            items.push(
                menu_row(
                    format!("board-column-move-delete-{index}").into(),
                    column.name.clone(),
                )
                .pl(px(16.))
                .aria_label(format!(
                    "Move the cards to {}, then delete the column",
                    column.name
                ))
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.header_delete_moving_cards(&from, &to, cx)
                }))
                .into_any_element(),
            );
        }
        items
    }

    /// Move a column's live cards to the end of `target`, in their order,
    /// then delete the column. A column with archived cards is refused
    /// first, so nothing moves for a delete that can't happen.
    fn header_delete_moving_cards(
        &mut self,
        column_id: &str,
        target: &str,
        cx: &mut Context<Self>,
    ) {
        self.menu = None;
        let archived = self.column_counts.get(column_id).map_or(0, |c| c.1);
        let held = self.undo.pending_delete_count_in(column_id);
        if archived > held {
            let left = archived - held;
            let plural = if left == 1 { "card" } else { "cards" };
            self.error = Some(format!(
                "Couldn't delete the column: it has {left} archived {plural}. Delete them from the Archive first"
            ));
            cx.notify();
            return;
        }
        if held > 0 {
            self.commit_pending(cx);
        }
        let ids: Vec<String> = self.detail.as_ref().map_or_else(Vec::new, |detail| {
            let mut cards: Vec<_> = detail
                .cards
                .iter()
                .filter(|card| card.column_id == column_id)
                .collect();
            cards.sort_by_key(|card| card.position);
            cards.into_iter().map(|card| card.id.clone()).collect()
        });
        self.write(cx, |service| {
            service.move_many(&ids, target, usize::MAX, "user")?;
            service.column_delete(column_id)
        });
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.forget_finished_drag(cx);
        self.display = self.effective_display();
        #[cfg(test)]
        self.rendered_cards.set(0);
        // Reads the bounds the last frame painted, before they are cleared.
        self.settle_reveal(window, cx);
        self.settle_lane_reveal(window, cx);
        // Bounds are only good for the frame that painted them: a card
        // scrolled out of a column's window keeps none.
        for anchor in self.card_anchors.values() {
            anchor.set(None);
        }
        if self.list_display.as_ref() != Some(&self.display) {
            // A density or card-face change: every card height is stale.
            if let Some(old) = self.list_display.clone() {
                self.learn_item_height(&old);
                self.remeasure_column_lists();
                self.remeasure_lane_list();
                self.item_heights.borrow_mut().clear();
                self.heights_version.set(self.heights_version.get() + 1);
            }
            self.list_display = Some(self.display.clone());
        }
        if self.detail.as_ref().is_some_and(|detail| {
            detail
                .columns
                .iter()
                .any(|column| !self.column_lists.contains_key(&column.id))
        }) {
            self.sync_column_lists();
        }
        self.pin_drawer_card();
        self.sync_lane_list(cx);
        self.follow_lane_header_refocus(window, cx);
        self.follow_card_focus(window, cx);
        // Focus may have moved onto (or off) the drawer's own card.
        self.pin_drawer_card();
        self.sync_lane_handles();
        let root =
            div()
                .id("board-view")
                .key_context("Board")
                .debug_selector(|| "board-view".into())
                .track_focus(&self.focus)
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                    this.on_key(event, window, cx)
                }))
                .on_action(cx.listener(|this, _: &new_card::OpenNewCard, window, cx| {
                    this.open_new_card(window, cx)
                }))
                .on_action(cx.listener(|this, _: &keyboard::NewCardKey, window, cx| {
                    if this.shortcuts_blocked() || this.drawer_holds_keys(window, cx) {
                        cx.propagate();
                    } else {
                        this.open_new_card(window, cx);
                    }
                }))
                .on_action(cx.listener(|this, _: &keyboard::FocusSearch, window, cx| {
                    if this.shortcuts_blocked() {
                        cx.propagate();
                    } else {
                        window.focus(&this.search.focus_handle(cx), cx);
                    }
                }))
                .on_action(
                    cx.listener(|this, _: &keyboard::OpenFocusedCard, window, cx| {
                        if !this.open_focused_card(window, cx) {
                            cx.propagate();
                        }
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &keyboard::DeleteSelected, window, cx| {
                        if !this.delete_focused_or_selected(window, cx) {
                            cx.propagate();
                        }
                    }),
                )
                .on_action(
                    cx.listener(|this, _: &new_card::SubmitNewCard, window, cx| {
                        if this.new_card.is_some() {
                            this.submit_new_card(window, cx);
                        } else {
                            cx.propagate();
                        }
                    }),
                )
                .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                    if this.resizing.is_some() {
                        if event.dragging() {
                            this.drag_resize(f32::from(event.position.x), cx);
                        } else {
                            this.resizing = None;
                        }
                    }
                }))
                .on_mouse_up(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, cx| {
                        this.resizing = None;
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
                // Text typed into a drawer whose card went away elsewhere.
                let unsaved = self
                    .orphaned_text
                    .as_ref()
                    .filter(|(shown, _)| *shown == error)
                    .map(|(_, text)| text.clone());
                div()
                    .id("board-error")
                    .debug_selector(|| "board-error".into())
                    .px_4()
                    .py_1()
                    .flex()
                    .items_start()
                    .gap_2()
                    .text_size(px(12.))
                    .text_color(diff_foreground(false))
                    .child(div().flex_1().min_w_0().child(error))
                    .children(unsaved.map(|text| {
                        div()
                            .id("board-error-copy")
                            .debug_selector(|| "board-error-copy".into())
                            .role(Role::Button)
                            .aria_label("Copy unsaved text")
                            .flex_shrink_0()
                            .px_2()
                            .rounded_md()
                            .cursor_pointer()
                            .text_color(content(0.8))
                            .hover(|s| s.bg(content(0.06)))
                            .child("Copy")
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    text.clone(),
                                ));
                            })
                    }))
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
            let anchor = open.anchor.clone().unwrap_or_else(|| {
                Anchor::Slot(
                    self.card_anchors
                        .get(&open.card_id)
                        .cloned()
                        .unwrap_or_default(),
                )
            });
            crate::popover::menu_popover(&open.menu, anchor, cx)
        });
        let mut card_detail = self.render_card_detail(cx);
        let filter = self.render_filter(cx);
        let sort = self.render_sort(cx);
        let view_menu = self.render_view_menu(cx);
        let chips = self.render_chips(cx);
        let open_in = self.render_open_in(cx);
        let switcher = self.render_switcher(cx);
        let archive = self.render_archive(cx);
        let undo_toast = self.render_undo_toast(cx);
        let new_card = self.render_new_card(cx);
        let run_picker = self.render_run_picker(cx);
        let run_confirm = self.render_run_confirm(cx);
        let pop_menu = self.render_pop_menu(cx);
        let selection_bar = self.render_selection_bar(cx);
        let layout = self.layout_kind();
        let full_page = self.card_detail.as_ref().is_some_and(|d| d.full_page());
        let settings_page = self.render_settings(cx);
        let on_settings = settings_page.is_some();
        let has_lanes = self.lane_mode().is_some();
        let lanes = (!on_settings && layout == list::Layout::Board && has_lanes)
            .then(|| self.render_swimlanes(cx))
            .flatten();
        let board = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .flex_col()
            .child(self.render_header(cx))
            .when(!on_settings, |d| d.child(self.render_views(cx)))
            .when(!on_settings, |d| d.children(chips))
            .children(error)
            .children(settings_page)
            .when(!on_settings && full_page, |d| {
                d.children(card_detail.take())
            })
            .when(
                !on_settings && !full_page && layout == list::Layout::List,
                |d| d.child(self.render_list(cx)),
            )
            .when(
                !on_settings && !full_page && layout == list::Layout::Table,
                |d| d.child(self.render_table(cx)),
            )
            .when(!full_page, |d| d.children(lanes))
            .when(
                !on_settings && !full_page && layout == list::Layout::Board && !has_lanes,
                |d| {
                    d.child(
                        div()
                            .id("board-columns")
                            .debug_selector(|| "board-columns".into())
                            .flex_1()
                            .min_h_0()
                            .overflow_x_scroll()
                            .px_4()
                            .py_3()
                            .flex()
                            .items_start()
                            .gap_2()
                            .child(self.render_drag_announce())
                            .children(columns)
                            .child(
                                icon_button("board-add-column", "Add column", "plus")
                                    .debug_selector(|| "board-add-column".into())
                                    .flex_shrink_0()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.add_column(window, cx)
                                    })),
                            ),
                    )
                },
            );
        root.child(board)
            .child(self.lift_press_watch(cx))
            .children(archive)
            .children(card_detail)
            .children(filter)
            .children(sort)
            .children(view_menu)
            .children(open_in)
            .children(switcher)
            .children(menu)
            .children(card_menu)
            .children(pop_menu)
            .children(selection_bar)
            .children(undo_toast)
            .children(new_card)
            .children(run_picker)
            .children(run_confirm)
    }
}
