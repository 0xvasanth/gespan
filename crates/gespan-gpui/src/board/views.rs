//! Saved views: what a view carries (search, filter rows, sort, layout), the
//! query behind it, and the tab strip above the columns.
//!
//! Matching, filtering and ordering all come from `Service::query`; this file
//! only caches its answer and draws it.
use super::{BoardView, menu_row};
use crate::{
    input::{self, Input},
    popover::{Anchor, AnchorSlot, popover},
    theme::*,
};
use gespan_core::board::{
    BoardDetail, Card, Service,
    extras::{SortKey, View, ViewSpec},
};
use gpui::{
    AnyElement, App, Context, Entity, Focusable, FontWeight, MouseButton, Role, SharedString,
    Subscription, Window, div, prelude::*, px,
};
use std::collections::{HashMap, HashSet};

/// The layout every view starts with. C4-4 adds list and table; the whole
/// string is stored and restored as it is, so it can carry more.
pub(super) const DEFAULT_LAYOUT: &str = r#"{"layout":"board"}"#;

pub(super) enum Naming {
    Save,
    Rename(String),
}

/// The name field for "Save view" or Rename, with its Enter subscription.
pub(super) struct ViewName {
    mode: Naming,
    input: Entity<Input>,
    _submit: Subscription,
}

/// The saved views of the open board and which one is showing. `baseline` is
/// what the showing view looked like when it was applied or last saved, so
/// "unsaved changes" is a comparison against it.
pub(crate) struct Views {
    pub saved: Vec<View>,
    /// The saved view showing, or `None` for the built-in "All cards".
    pub active: Option<String>,
    pub baseline: ViewSpec,
    pub baseline_layout: String,
    /// The layout JSON the view carries.
    pub layout: String,
    pub(super) naming: Option<ViewName>,
    /// The view whose tab menu is open.
    pub(super) menu: Option<String>,
    anchors: HashMap<String, AnchorSlot>,
    /// The showing view changed elsewhere while untouched here: the reload
    /// that read it applies it again.
    pub(super) reapply: bool,
}

impl Default for Views {
    fn default() -> Self {
        Self {
            saved: Vec::new(),
            active: None,
            baseline: ViewSpec::default(),
            baseline_layout: DEFAULT_LAYOUT.into(),
            layout: DEFAULT_LAYOUT.into(),
            naming: None,
            menu: None,
            anchors: HashMap::new(),
            reapply: false,
        }
    }
}

fn same_json(a: &str, b: &str) -> bool {
    match (
        serde_json::from_str::<serde_json::Value>(a),
        serde_json::from_str::<serde_json::Value>(b),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// A stored spec that no longer parses shows everything rather than failing.
fn parse_spec(json: &str) -> ViewSpec {
    serde_json::from_str(json).unwrap_or_default()
}

impl BoardView {
    /// The search text and the filter rows and sort, as one spec.
    pub(crate) fn current_spec(&self, cx: &App) -> ViewSpec {
        // A row with no values filters nothing, so it isn't part of the spec.
        let rows = (self.spec.rows.iter())
            .filter(|row| !row.values.is_empty())
            .cloned()
            .collect();
        ViewSpec {
            query: self.search.read(cx).content.trim().to_string(),
            rows,
            sort: self.spec.sort,
        }
    }

    /// Whether the board is ordered by something other than the columns' own
    /// order. Sorted columns can't be reordered by dragging inside them.
    pub(crate) fn sorted(&self) -> bool {
        self.spec.sort.key != SortKey::Manual
    }

    /// Whether the search, filter or sort changed from the showing view.
    pub(crate) fn view_dirty(&self, cx: &App) -> bool {
        self.current_spec(cx) != self.views.baseline
            || !same_json(&self.views.layout, &self.views.baseline_layout)
    }

    /// Re-run the query and cache the cards it matched and their order.
    pub(super) fn refresh_query(&mut self, cx: &App) -> Result<(), String> {
        let result = self.run_query(cx);
        // A selection holds only cards on screen: ones the query now hides drop.
        self.prune_selection();
        self.sync_column_lists();
        result
    }

    fn run_query(&mut self, cx: &App) -> Result<(), String> {
        let spec = self.current_spec(cx);
        self.last_query = spec.query.clone();
        let Some(board_id) = self.open_board_id.clone() else {
            self.matched = None;
            self.rank.clear();
            return Ok(());
        };
        if spec.query.is_empty() && spec.rows.is_empty() && spec.sort.key == SortKey::Manual {
            self.matched = None;
            self.rank.clear();
            return Ok(());
        }
        // The cards whose linked session or Pilot has an open turn, or waits
        // on the user (ruling C5-m), for the `linked: running` filter.
        let links = super::turns::running_links(cx);
        let reads = self.reads.clone();
        let ids = self.service().and_then(|service| {
            let running = if links.is_empty() {
                HashSet::new()
            } else {
                reads.note("running_cards");
                service.running_cards(&board_id, &links)?
            };
            reads.note("query");
            service.query(&board_id, &spec, &running)
        })?;
        self.rank = ids
            .iter()
            .enumerate()
            .map(|(index, id)| (id.clone(), index))
            .collect();
        self.matched = Some(ids.into_iter().collect());
        Ok(())
    }

    /// `refresh_query` after the user changed the spec: a failure shows.
    pub(super) fn requery(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = self.refresh_query(cx) {
            self.matched = None;
            self.rank.clear();
            self.error = Some(error);
            self.sync_column_lists();
        }
        cx.notify();
    }

    /// Whether a card passes the search and the filter.
    pub(crate) fn card_matches(&self, card: &Card) -> bool {
        self.matched
            .as_ref()
            .is_none_or(|matched| matched.contains(&card.id))
    }

    /// A column's visible cards, top to bottom: the query's order while
    /// sorting, otherwise the column's own.
    pub(crate) fn column_cards<'a>(
        &self,
        detail: &'a BoardDetail,
        column_id: &str,
    ) -> Vec<&'a Card> {
        let mut cards: Vec<&Card> = detail
            .cards
            .iter()
            .filter(|card| card.column_id == column_id && self.card_matches(card))
            .collect();
        cards.sort_by_key(|card| {
            (
                self.rank.get(&card.id).copied().unwrap_or(usize::MAX),
                card.position,
            )
        });
        cards
    }

    /// Back to "All cards" with nothing searched, filtered or sorted.
    pub(super) fn reset_view(&mut self, cx: &mut Context<Self>) {
        self.views.active = None;
        self.views.baseline = ViewSpec::default();
        self.views.baseline_layout = DEFAULT_LAYOUT.into();
        self.views.layout = DEFAULT_LAYOUT.into();
        self.views.naming = None;
        self.views.menu = None;
        self.apply_spec(ViewSpec::default(), cx);
    }

    fn apply_spec(&mut self, spec: ViewSpec, cx: &mut Context<Self>) {
        let query = spec.query.clone();
        self.spec = ViewSpec {
            query: String::new(),
            ..spec
        };
        self.search
            .update(cx, |input, cx| input.set_text(query, cx));
        self.requery(cx);
    }

    fn set_saved(&mut self, saved: Vec<View>) {
        self.views
            .anchors
            .retain(|id, _| saved.iter().any(|v| &v.id == id));
        for view in &saved {
            self.views.anchors.entry(view.id.clone()).or_default();
        }
        self.views.saved = saved;
    }

    /// Take the saved views a reload read. A showing view deleted elsewhere
    /// leaves the current search, filter and sort in place under "All cards".
    pub(super) fn sync_views(&mut self, saved: Vec<View>, cx: &App) {
        let dirty = self.view_dirty(cx);
        self.set_saved(saved);
        let stored = self
            .views
            .active
            .as_ref()
            .and_then(|id| self.views.saved.iter().find(|view| &view.id == id))
            .map(|view| (parse_spec(&view.spec_json), view.layout_json.clone()));
        if let Some((spec, layout)) = stored
            && (spec != self.views.baseline || !same_json(&layout, &self.views.baseline_layout))
        {
            // Changed elsewhere. An untouched board shows the new version;
            // local edits stay, and now differ from what is saved.
            self.views.baseline = spec;
            self.views.baseline_layout = layout;
            self.views.reapply = !dirty;
        }
        let gone = self
            .views
            .active
            .as_ref()
            .is_some_and(|id| !self.views.saved.iter().any(|view| &view.id == id));
        if gone {
            self.views.active = None;
            self.views.baseline = ViewSpec::default();
            self.views.baseline_layout = DEFAULT_LAYOUT.into();
        }
        if self
            .views
            .menu
            .as_ref()
            .is_some_and(|id| !self.views.saved.iter().any(|view| &view.id == id))
        {
            self.views.menu = None;
        }
    }

    /// Show a saved view, or "All cards" for `None`. Unsaved changes to the
    /// view being left are dropped.
    pub(crate) fn select_view(&mut self, id: Option<String>, cx: &mut Context<Self>) {
        let (spec, layout) = match &id {
            None => (ViewSpec::default(), DEFAULT_LAYOUT.to_string()),
            Some(id) => match self.views.saved.iter().find(|view| &view.id == id) {
                Some(view) => (parse_spec(&view.spec_json), view.layout_json.clone()),
                None => return,
            },
        };
        self.views.active = id;
        self.views.baseline = spec.clone();
        self.views.baseline_layout = layout.clone();
        self.views.layout = layout;
        self.views.menu = None;
        self.views.naming = None;
        self.apply_spec(spec, cx);
    }

    /// Run a view write; on success reload and hand back what it returned.
    fn view_write<T>(
        &mut self,
        cx: &mut Context<Self>,
        action: impl FnOnce(&mut Service) -> Result<T, String>,
    ) -> Option<T> {
        match self.service().and_then(action) {
            Ok(value) => {
                self.error = None;
                self.reload(cx);
                Some(value)
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                None
            }
        }
    }

    fn spec_json(&self, cx: &App) -> String {
        serde_json::to_string(&self.current_spec(cx)).unwrap_or_else(|_| "{}".into())
    }

    /// Save the current search, filter, sort and layout as a new view and
    /// show it. False when the store refused the name or the spec.
    fn save_view_as(&mut self, name: &str, cx: &mut Context<Self>) -> bool {
        let Some(board_id) = self.open_board_id.clone() else {
            return false;
        };
        let (spec, layout) = (self.spec_json(cx), self.views.layout.clone());
        let Some(view) = self.view_write(cx, |service| {
            service.view_create(&board_id, name, &spec, &layout)
        }) else {
            return false;
        };
        self.adopt(view, cx);
        true
    }

    /// Put a view just created into the tab strip and show it, even when the
    /// reload that would list it is waiting on a drag.
    fn adopt(&mut self, view: View, cx: &mut Context<Self>) {
        if !self.views.saved.iter().any(|saved| saved.id == view.id) {
            let mut saved = self.views.saved.clone();
            saved.push(view.clone());
            self.set_saved(saved);
        }
        self.select_view(Some(view.id), cx);
    }

    /// Write the current search, filter, sort and layout over the showing view.
    pub(crate) fn save_view_changes(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.views.active.clone() else {
            return;
        };
        let (spec, layout) = (self.spec_json(cx), self.views.layout.clone());
        if self
            .view_write(cx, |service| {
                service.view_update(&id, None, Some(&spec), Some(&layout))
            })
            .is_some()
        {
            self.views.baseline = self.current_spec(cx);
            self.views.baseline_layout = layout;
            cx.notify();
        }
    }

    /// Drop the unsaved changes and show the view as it was saved.
    pub(crate) fn revert_view_changes(&mut self, cx: &mut Context<Self>) {
        let active = self.views.active.clone();
        self.select_view(active, cx);
    }

    pub(crate) fn duplicate_view(&mut self, id: &str, cx: &mut Context<Self>) {
        self.views.menu = None;
        let Some(source) = self.views.saved.iter().find(|view| view.id == id).cloned() else {
            return;
        };
        let Some(board_id) = self.open_board_id.clone() else {
            return;
        };
        let name = format!("{} copy", source.name);
        // The showing view's unsaved edits go into the copy; the original
        // keeps its stored version (the copy is shown, so it is not left
        // dirty).
        let (spec, layout) = if self.views.active.as_deref() == Some(id) {
            (self.spec_json(cx), self.views.layout.clone())
        } else {
            (source.spec_json.clone(), source.layout_json.clone())
        };
        if let Some(view) = self.view_write(cx, |service| {
            service.view_create(&board_id, &name, &spec, &layout)
        }) {
            self.adopt(view, cx);
        }
    }

    /// Delete a saved view. The built-in "All cards" is not one, so it has
    /// no id to delete.
    pub(crate) fn delete_view(&mut self, id: &str, cx: &mut Context<Self>) {
        self.views.menu = None;
        if !self.views.saved.iter().any(|view| view.id == id) {
            return;
        }
        let was_active = self.views.active.as_deref() == Some(id);
        let owned = id.to_string();
        if self
            .view_write(cx, |service| service.view_delete(&owned))
            .is_some()
        {
            let saved = self
                .views
                .saved
                .iter()
                .filter(|view| view.id != id)
                .cloned()
                .collect();
            self.set_saved(saved);
            if was_active {
                self.select_view(None, cx);
            }
        }
    }

    fn begin_naming(
        &mut self,
        mode: Naming,
        current: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input = cx.new(|cx| Input::field("View name", "View name", content(0.4), cx));
        input.update(cx, |input, cx| input.set_text(current.to_string(), cx));
        let submit = cx.subscribe(&input, |this, _, _: &input::Submitted, cx| {
            this.commit_naming(cx)
        });
        window.focus(&input.focus_handle(cx), cx);
        self.views.menu = None;
        self.views.naming = Some(ViewName {
            mode,
            input,
            _submit: submit,
        });
        cx.notify();
    }

    fn commit_naming(&mut self, cx: &mut Context<Self>) {
        let Some(naming) = self.views.naming.take() else {
            return;
        };
        let name = naming.input.read(cx).content.trim().to_string();
        if name.is_empty() {
            cx.notify();
            return;
        }
        let done = match &naming.mode {
            Naming::Save => self.save_view_as(&name, cx),
            Naming::Rename(id) => {
                let id = id.clone();
                self.view_write(cx, |service| {
                    service.view_update(&id, Some(&name), None, None)
                })
                .is_some()
            }
        };
        if !done {
            // Keep the rejected name in the field to correct.
            self.views.naming = Some(naming);
        }
        cx.notify();
    }

    pub(super) fn render_views(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let dirty = self.view_dirty(cx);
        let tab = |id: SharedString, label: SharedString, on: bool| {
            div()
                .id(id.clone())
                .debug_selector(move || id.to_string())
                .role(Role::Tab)
                .aria_label(label.clone())
                .h(px(24.))
                .px_2()
                .rounded_md()
                .flex()
                .items_center()
                .gap(px(4.))
                .text_size(px(12.))
                .text_color(content(if on { 0.95 } else { 0.55 }))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.06)))
                .when(on, |d| d.bg(content(0.07)).font_weight(FontWeight::MEDIUM))
                .child(label)
        };
        let small_button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .role(Role::Button)
                .aria_label(label)
                .h(px(22.))
                .px(px(6.))
                .rounded_md()
                .flex()
                .items_center()
                .text_size(px(11.))
                .text_color(content(0.7))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.08)))
                .child(label)
        };
        let mut strip = div()
            .id("board-views")
            .debug_selector(|| "board-views".into())
            .role(Role::TabList)
            .h(px(32.))
            .flex_shrink_0()
            .px_4()
            .flex()
            .items_center()
            .gap(px(4.))
            .border_b_1()
            .border_color(content(0.07))
            .child(
                tab(
                    "board-view-tab-all".into(),
                    "All cards".into(),
                    self.views.active.is_none(),
                )
                .on_click(cx.listener(|this, _, _, cx| this.select_view(None, cx))),
            );
        for (index, view) in self.views.saved.iter().enumerate() {
            let on = self.views.active.as_ref() == Some(&view.id);
            let select_id = view.id.clone();
            let menu_id = view.id.clone();
            let anchor = self
                .views
                .anchors
                .get(&view.id)
                .cloned()
                .unwrap_or_default();
            strip =
                strip.child(
                    tab(
                        format!("board-view-tab-{index}").into(),
                        view.name.clone().into(),
                        on,
                    )
                    .relative()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.select_view(Some(select_id.clone()), cx)
                    }))
                    .child(anchor.probe())
                    .when(on && dirty, |d| {
                        d.child(
                            div()
                                .id(format!("board-view-dot-{index}"))
                                .debug_selector(move || format!("board-view-dot-{index}"))
                                .aria_label("Unsaved changes")
                                .text_color(accent_color())
                                .child("•"),
                        )
                    })
                    .child(
                        div()
                            .id(format!("board-view-menu-{index}"))
                            .debug_selector(move || format!("board-view-menu-{index}"))
                            .role(Role::Button)
                            .aria_label("View options")
                            .size(px(16.))
                            .rounded_md()
                            .flex()
                            .items_center()
                            .justify_center()
                            .hover(|s| s.bg(content(0.1)))
                            .child(icon("more-horizontal", 11.).text_color(content(0.5)))
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.views.menu = Some(menu_id.clone());
                                cx.notify();
                            })),
                    ),
                );
        }
        if self.views.active.is_some() && dirty {
            strip = strip
                .child(
                    small_button("board-view-save-changes", "Save")
                        .on_click(cx.listener(|this, _, _, cx| this.save_view_changes(cx))),
                )
                .child(
                    small_button("board-view-revert", "Revert")
                        .on_click(cx.listener(|this, _, _, cx| this.revert_view_changes(cx))),
                );
        }
        strip = strip
            .child(div().flex_1())
            .child(self.render_layout_switch(cx));
        strip = match &self.views.naming {
            Some(naming) => strip.child(
                div()
                    .id("board-view-name")
                    .debug_selector(|| "board-view-name".into())
                    .w(px(160.))
                    .h(px(24.))
                    .px_2()
                    .rounded_md()
                    .border_1()
                    .border_color(content(0.12))
                    .flex()
                    .items_center()
                    .child(div().flex_1().min_w_0().child(naming.input.clone())),
            ),
            None => strip.child(small_button("board-view-new", "Save view").on_click(
                cx.listener(|this, _, window, cx| this.begin_naming(Naming::Save, "", window, cx)),
            )),
        };
        strip
    }

    pub(super) fn render_view_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let id = self.views.menu.clone()?;
        let view = self.views.saved.iter().find(|view| view.id == id)?;
        let anchor = self.views.anchors.get(&id).cloned().unwrap_or_default();
        let weak = cx.weak_entity();
        let (rename_id, duplicate_id, delete_id) = (id.clone(), id.clone(), id);
        let name = view.name.clone();
        Some(
            popover("board-view-menu", Anchor::Slot(anchor))
                .width(180.)
                .on_dismiss(move |_, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.views.menu = None;
                        cx.notify();
                    });
                })
                .child(
                    div()
                        .p(px(4.))
                        .flex()
                        .flex_col()
                        .child(menu_row("board-view-rename".into(), "Rename").on_click(
                            cx.listener(move |this, _, window, cx| {
                                this.begin_naming(
                                    Naming::Rename(rename_id.clone()),
                                    &name,
                                    window,
                                    cx,
                                )
                            }),
                        ))
                        .child(
                            menu_row("board-view-duplicate".into(), "Duplicate").on_click(
                                cx.listener(move |this, _, _, cx| {
                                    this.duplicate_view(&duplicate_id, cx)
                                }),
                            ),
                        )
                        .child(
                            menu_row("board-view-delete".into(), "Delete")
                                .text_color(diff_foreground(false))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.delete_view(&delete_id, cx)
                                })),
                        ),
                )
                .into_any_element(),
        )
    }
}
