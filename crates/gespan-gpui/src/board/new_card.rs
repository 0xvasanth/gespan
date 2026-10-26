//! The New card modal, opened with ⌘N (Ctrl+N elsewhere) while the board has
//! focus. It makes one card with its main fields: title, column, priority,
//! due date, labels, projects and description. The menus are the ones the card
//! face and drawer use.
use super::{
    BoardView,
    quick::{self, DueChoice},
    style,
};
use crate::{
    input::{self, Input},
    modal,
    popover::{Anchor, AnchorSlot, Menu, MenuAction, MenuEvent, MenuItem, MenuStyle, menu_popover},
    theme::*,
};
use chrono::NaiveDate;
use gespan_core::board::{Service, extras::PRIORITIES};
use gpui::{
    AnyElement, App, Context, Entity, FocusHandle, Focusable, FontWeight, KeyBinding, Role,
    SharedString, Subscription, Toggled, Window, actions, div, prelude::*, px,
};
use std::collections::HashMap;

actions!(board, [OpenNewCard, SubmitNewCard]);

/// Scoped to the `Board` key context, which sits below the shell's
/// `Workspace` one, so the board's ⌘N wins while focus is inside the board.
pub(crate) fn bind(cx: &mut App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(&format!("{modifier}-n"), OpenNewCard, Some("Board")),
        KeyBinding::new(&format!("{modifier}-enter"), SubmitNewCard, Some("Board")),
    ]);
}

/// A property button in the modal, and the menu it opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Property {
    Column,
    Priority,
    Due,
    Labels,
    Projects,
}

struct OpenMenu {
    property: Property,
    menu: Entity<Menu>,
    _events: Subscription,
}

pub(super) struct NewCard {
    pub title: Entity<Input>,
    pub description: Entity<Input>,
    /// The typed date, shown after "Pick a date…".
    pub date: Entity<Input>,
    pub column_id: String,
    pub priority: u8,
    pub due: Option<NaiveDate>,
    pub typing_date: bool,
    pub labels: Vec<String>,
    pub projects: Vec<String>,
    pub create_more: bool,
    pub title_missing: bool,
    pub error: Option<String>,
    anchors: HashMap<Property, AnchorSlot>,
    /// Tab stops after the two text fields, in order.
    pub focus: HashMap<Property, FocusHandle>,
    pub more_focus: FocusHandle,
    pub create_focus: FocusHandle,
    menu: Option<OpenMenu>,
    _subscriptions: Vec<Subscription>,
}

/// The column a new card starts in: the first `todo` column, else the first.
pub(super) fn default_column(columns: &[gespan_core::board::Column]) -> Option<String> {
    let mut ordered: Vec<_> = columns.iter().collect();
    ordered.sort_by_key(|column| column.position);
    ordered
        .iter()
        .find(|column| column.role.as_deref() == Some("todo"))
        .or(ordered.first())
        .map(|column| column.id.clone())
}

/// What the modal writes: the card, then one setter per field that was set.
struct Draft {
    column_id: String,
    title: String,
    body: String,
    priority: u8,
    due: Option<i64>,
    labels: Vec<String>,
    projects: Vec<String>,
}

/// Create the card and apply the fields. `Err` means no card was made; the
/// `Ok` list names each field that could not be saved on the card that was.
fn create(service: &mut Service, draft: &Draft) -> Result<Vec<String>, String> {
    let card = service.card_create(&draft.column_id, &draft.title, &draft.body)?;
    let mut failed = Vec::new();
    let mut note = |what: &str, result: Result<(), String>| {
        if let Err(error) = result {
            failed.push(format!("{what}: {error}"));
        }
    };
    if draft.priority != 0 {
        note("priority", service.set_priority(&card.id, draft.priority));
    }
    if draft.due.is_some() {
        note("due date", service.set_due(&card.id, draft.due));
    }
    if !draft.labels.is_empty() {
        note("labels", service.set_labels(&card.id, &draft.labels));
    }
    if !draft.projects.is_empty() {
        note("projects", service.set_projects(&card.id, &draft.projects));
    }
    Ok(failed)
}

fn date_error(text: &str) -> String {
    format!("“{text}” is not a date like 2026-11-02")
}

impl BoardView {
    /// Open the modal on the open board, or put the cursor back in its title.
    pub(crate) fn open_new_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(form) = &self.new_card {
            window.focus(&form.title.focus_handle(cx), cx);
            return;
        }
        let Some(column_id) = self
            .detail
            .as_ref()
            .and_then(|detail| default_column(&detail.columns))
        else {
            return;
        };
        // Whatever was open under the modal closes with it in front.
        self.menu = None;
        self.card_menu = None;
        self.composer = None;
        self.renaming = None;
        self.filter_open = false;
        self.open_in_open = false;
        self.switcher = None;
        self.archive_open = false;
        let title = cx.new(|cx| Input::field("Title", "Card title", content(0.4), cx));
        let description =
            cx.new(|cx| Input::multiline_field("Description", "Add a description…", cx));
        let date = cx.new(|cx| Input::field("Due date", "YYYY-MM-DD", content(0.4), cx));
        for field in [&title, &description, &date] {
            field.update(cx, |input, _| input.tab_stop());
        }
        let subscriptions = vec![
            cx.subscribe_in(
                &title,
                window,
                |this, _, _: &input::Submitted, window, cx| this.submit_new_card(window, cx),
            ),
            cx.subscribe_in(
                &description,
                window,
                |this, _, _: &input::Submitted, window, cx| this.submit_new_card(window, cx),
            ),
            // Typing in the title takes the "Add a title" message away.
            cx.subscribe(&title, |this, _, _: &input::Changed, cx| {
                if let Some(form) = &mut this.new_card
                    && form.title_missing
                {
                    form.title_missing = false;
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &date,
                window,
                |this, _, _: &input::Submitted, window, cx| this.apply_typed_date(window, cx),
            ),
        ];
        window.focus(&title.focus_handle(cx), cx);
        self.new_card = Some(NewCard {
            title,
            description,
            date,
            column_id,
            priority: 0,
            due: None,
            typing_date: false,
            labels: Vec::new(),
            projects: Vec::new(),
            create_more: false,
            title_missing: false,
            error: None,
            anchors: HashMap::new(),
            focus: [
                Property::Column,
                Property::Priority,
                Property::Due,
                Property::Labels,
                Property::Projects,
            ]
            .into_iter()
            .map(|property| (property, cx.focus_handle().tab_stop(true)))
            .collect(),
            more_focus: cx.focus_handle().tab_stop(true),
            create_focus: cx.focus_handle().tab_stop(true),
            menu: None,
            _subscriptions: subscriptions,
        });
        cx.notify();
    }

    /// Close the modal, giving the keys back to the board. True when it was open.
    pub(super) fn close_new_card(
        &mut self,
        window: Option<&mut Window>,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.new_card.take().is_none() {
            return false;
        }
        if let Some(window) = window {
            window.focus(&self.focus, cx);
        }
        cx.notify();
        true
    }

    /// Enter in the date field: take the date and go back to the title.
    fn apply_typed_date(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.read_typed_date(cx)
            && let Some(form) = &self.new_card
        {
            window.focus(&form.title.focus_handle(cx), cx);
        }
        cx.notify();
    }

    /// Move a typed date into the form. False (with the reason shown) when it
    /// is not a date.
    fn read_typed_date(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(form) = &mut self.new_card else {
            return true;
        };
        if !form.typing_date {
            return true;
        }
        let text = form.date.read(cx).content.trim().to_string();
        if text.is_empty() {
            return true;
        }
        match NaiveDate::parse_from_str(&text, "%Y-%m-%d") {
            Ok(day) => {
                form.due = Some(day);
                form.typing_date = false;
                form.error = None;
                true
            }
            Err(_) => {
                form.error = Some(date_error(&text));
                false
            }
        }
    }

    pub(super) fn submit_new_card(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = &mut self.new_card else {
            return;
        };
        let title = form.title.read(cx).content.trim().to_string();
        if title.is_empty() {
            form.title_missing = true;
            window.focus(&form.title.focus_handle(cx), cx);
            cx.notify();
            return;
        }
        if !self.read_typed_date(cx) {
            cx.notify();
            return;
        }
        let Some(form) = &mut self.new_card else {
            return;
        };
        let due = match form.due.map(|day| (day, style::day_start(day))) {
            Some((day, None)) => {
                form.error = Some(format!("{day} does not exist in this time zone"));
                cx.notify();
                return;
            }
            Some((_, at)) => at,
            None => None,
        };
        let draft = Draft {
            column_id: form.column_id.clone(),
            title,
            body: form.description.read(cx).content.trim_end().to_string(),
            priority: form.priority,
            due,
            labels: form.labels.clone(),
            projects: form.projects.clone(),
        };
        let created = self.service().map(|service| create(service, &draft));
        let failed = match created {
            Ok(Ok(failed)) => failed,
            Ok(Err(error)) | Err(error) => {
                if let Some(form) = &mut self.new_card {
                    form.error = Some(error);
                }
                cx.notify();
                return;
            }
        };
        self.reload(cx);
        let message = (!failed.is_empty())
            .then(|| format!("Card created, but not saved: {}", failed.join("; ")));
        let Some(form) = &mut self.new_card else {
            return;
        };
        if form.create_more {
            // The next card starts blank; the rest carries over.
            form.error = message;
            form.title_missing = false;
            let (title, description) = (form.title.clone(), form.description.clone());
            title.update(cx, |input, cx| input.set_text(String::new(), cx));
            description.update(cx, |input, cx| input.set_text(String::new(), cx));
            window.focus(&title.focus_handle(cx), cx);
            cx.notify();
        } else {
            self.close_new_card(Some(window), cx);
            self.error = message;
            cx.notify();
        }
    }

    fn toggle_create_more(&mut self, cx: &mut Context<Self>) {
        if let Some(form) = &mut self.new_card {
            form.create_more = !form.create_more;
            cx.notify();
        }
    }

    fn open_new_card_menu(
        &mut self,
        property: Property,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (Some(form), Some(detail)) = (&self.new_card, &self.detail) else {
            return;
        };
        let (title, items) = match property {
            Property::Column => (
                "Column",
                detail
                    .columns
                    .iter()
                    .enumerate()
                    .map(|(index, column)| {
                        MenuAction::new(format!("column-{index}"), column.name.clone())
                            .checked(column.id == form.column_id)
                            .into()
                    })
                    .collect(),
            ),
            Property::Priority => ("Priority", quick::priority_items()),
            Property::Due => ("Due date", quick::due_items()),
            Property::Labels => ("Labels", multi_keep(self.label_menu_items(&form.labels))),
            Property::Projects => (
                "Projects",
                multi_keep(self.project_menu_items(&form.projects)),
            ),
        };
        let menu = cx.new(|cx| Menu::new(title, items, window, cx).style(MenuStyle::Explorer));
        let events = cx.subscribe_in(
            &menu,
            window,
            move |this, menu, event: &MenuEvent, window, cx| match event {
                MenuEvent::Picked(id, _) => {
                    let keeps_open = matches!(property, Property::Labels | Property::Projects);
                    if !keeps_open && let Some(form) = &mut this.new_card {
                        form.menu = None;
                    }
                    this.pick_new_card(property, id, menu, window, cx);
                    cx.notify();
                }
                MenuEvent::Dismissed => {
                    if let Some(form) = &mut this.new_card {
                        form.menu = None;
                    }
                    cx.notify();
                }
            },
        );
        if let Some(form) = &mut self.new_card {
            form.menu = Some(OpenMenu {
                property,
                menu,
                _events: events,
            });
        }
        cx.notify();
    }

    fn label_menu_items(&self, selected: &[String]) -> Vec<MenuItem> {
        if self.labels.is_empty() {
            return vec![MenuItem::Note("No labels on this board yet".into())];
        }
        quick::label_items(&self.labels, selected)
    }

    fn project_menu_items(&self, selected: &[String]) -> Vec<MenuItem> {
        let choices = self.project_choices();
        if choices.is_empty() {
            return vec![MenuItem::Note("No projects yet".into())];
        }
        choices
            .iter()
            .enumerate()
            .map(|(index, (path, name))| {
                MenuAction::new(format!("project-{index}"), name.clone())
                    .checked(selected.contains(path))
                    .into()
            })
            .collect()
    }

    fn pick_new_card(
        &mut self,
        property: Property,
        id: &str,
        menu: &Entity<Menu>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let choices = self.project_choices();
        let columns: Vec<String> = self
            .detail
            .as_ref()
            .map(|detail| detail.columns.iter().map(|c| c.id.clone()).collect())
            .unwrap_or_default();
        let labels = self.labels.clone();
        let Some(form) = &mut self.new_card else {
            return;
        };
        match property {
            Property::Column => {
                let picked = id
                    .strip_prefix("column-")
                    .and_then(|index| index.parse::<usize>().ok())
                    .and_then(|index| columns.get(index));
                if let Some(column_id) = picked {
                    form.column_id = column_id.clone();
                }
            }
            Property::Priority => {
                if let Some(priority) = quick::parse_priority(id) {
                    form.priority = priority;
                }
            }
            Property::Due => match quick::parse_due(id, chrono::Local::now().date_naive()) {
                Some(choice @ (DueChoice::Day(_) | DueChoice::Clear)) => {
                    form.due = match choice {
                        DueChoice::Day(day) => Some(day),
                        _ => None,
                    };
                    form.typing_date = false;
                    form.error = None;
                    // Text typed before the pick must not come back later.
                    let date = form.date.clone();
                    date.update(cx, |input, cx| input.set_text(String::new(), cx));
                }
                Some(DueChoice::Pick) => {
                    form.typing_date = true;
                    let focus = form.date.focus_handle(cx);
                    window.focus(&focus, cx);
                }
                None => {}
            },
            Property::Labels => {
                if let Some(label_id) = id.strip_prefix("label-") {
                    form.labels = quick::toggled(&form.labels, label_id);
                    let items = multi_keep(quick::label_items(&labels, &form.labels));
                    menu.update(cx, |menu, cx| menu.replace_items(items, cx));
                }
            }
            Property::Projects => {
                let picked = id
                    .strip_prefix("project-")
                    .and_then(|index| index.parse::<usize>().ok())
                    .and_then(|index| choices.get(index));
                if let Some((path, _)) = picked {
                    form.projects = quick::toggled(&form.projects, path);
                    let items = multi_keep(
                        choices
                            .iter()
                            .enumerate()
                            .map(|(index, (path, name))| {
                                MenuAction::new(format!("project-{index}"), name.clone())
                                    .checked(form.projects.contains(path))
                                    .into()
                            })
                            .collect(),
                    );
                    menu.update(cx, |menu, cx| menu.replace_items(items, cx));
                }
            }
        }
    }

    pub(super) fn render_new_card(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.new_card.as_ref()?;
        let detail = self.detail.as_ref()?;
        let anchor = |property: Property| form.anchors.get(&property).cloned().unwrap_or_default();
        let column_name = detail
            .columns
            .iter()
            .find(|column| column.id == form.column_id)
            .map_or("Column".to_string(), |column| column.name.clone());
        let names = |ids: &[String], known: &dyn Fn(&str) -> Option<String>, none: &str| match ids {
            [] => none.to_string(),
            [one] => known(one).unwrap_or_else(|| none.to_string()),
            many => format!("{} selected", many.len()),
        };
        let labels = names(
            &form.labels,
            &|id| {
                self.labels
                    .iter()
                    .find(|label| label.id == id)
                    .map(|label| label.name.clone())
            },
            "Labels",
        );
        let projects = names(
            &form.projects,
            &|path| Some(self.project_name(path)),
            "Projects",
        );
        let priority = if form.priority == 0 {
            "Priority".to_string()
        } else {
            PRIORITIES[form.priority as usize].to_string()
        };
        let due = form.due.map_or("Due date".to_string(), |day| {
            day.format("%b %-d, %Y").to_string()
        });
        let pill = |id: &'static str, property: Property, label: String, set: bool| {
            let selector = id;
            div()
                .id(id)
                .debug_selector(move || selector.to_string())
                .role(Role::Button)
                .aria_label(label.clone())
                .relative()
                .h(px(28.))
                .px(px(10.))
                .rounded(px(8.))
                .border_1()
                .border_color(content(0.1))
                .flex()
                .items_center()
                .text_size(px(12.))
                .text_color(content(if set { 0.9 } else { 0.55 }))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.06)))
                .when_some(form.focus.get(&property), |d, focus| {
                    d.track_focus(focus)
                        .tab_index(0)
                        .focus_visible(|s| s.border_color(accent_color()))
                })
                .on_key_down(
                    cx.listener(move |this, event: &gpui::KeyDownEvent, window, cx| {
                        if matches!(event.keystroke.key.as_str(), "enter" | "space" | "down") {
                            cx.stop_propagation();
                            this.open_new_card_menu(property, window, cx);
                        }
                    }),
                )
                .child(label)
                .child(anchor(property).probe())
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.open_new_card_menu(property, window, cx)
                }))
        };
        let more = form.create_more;
        let checkbox = div()
            .id("new-card-more")
            .debug_selector(|| "new-card-more".into())
            .role(Role::CheckBox)
            .aria_label("Create more")
            .aria_toggled(if more { Toggled::True } else { Toggled::False })
            .flex()
            .items_center()
            .gap(px(8.))
            .text_size(px(12.))
            .text_color(content(0.7))
            .cursor_pointer()
            .track_focus(&form.more_focus)
            .tab_index(0)
            .focus_visible(|s| s.text_color(content(1.)))
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                if event.keystroke.key == "space" {
                    cx.stop_propagation();
                    this.toggle_create_more(cx);
                }
            }))
            .on_click(cx.listener(|this, _, window, cx| {
                this.toggle_create_more(cx);
                // A click leaves the cursor where the user was typing.
                if let Some(form) = &this.new_card {
                    window.focus(&form.title.focus_handle(cx), cx);
                }
            }))
            .child(
                div()
                    .size(px(14.))
                    .rounded(px(4.))
                    .border_1()
                    .border_color(content(0.3))
                    .flex()
                    .items_center()
                    .justify_center()
                    .when(more, |d| {
                        d.bg(content(0.9))
                            .child(icon("check", 9.).text_color(pane_fill()))
                    }),
            )
            .child("Create more");
        let message = |id: &'static str, text: SharedString| {
            div()
                .id(id)
                .debug_selector(move || id.to_string())
                .text_size(px(12.))
                .text_color(diff_foreground(false))
                .child(text)
        };
        let body = modal::card_body()
            .gap(px(12.))
            .child(modal::title("New card"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .id("new-card-title")
                            .debug_selector(|| "new-card-title".into())
                            .h(px(32.))
                            .px(px(10.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(if form.title_missing {
                                diff_foreground(false)
                            } else {
                                content(0.12)
                            })
                            .flex()
                            .items_center()
                            .child(form.title.clone()),
                    )
                    .when(form.title_missing, |d| {
                        d.child(message("new-card-title-error", "Add a title".into()))
                    }),
            )
            .child(
                div()
                    .id("new-card-description")
                    .debug_selector(|| "new-card-description".into())
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(content(0.12))
                    .child(form.description.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap(px(6.))
                    .child(pill("new-card-column", Property::Column, column_name, true))
                    .child(pill(
                        "new-card-priority",
                        Property::Priority,
                        priority,
                        form.priority != 0,
                    ))
                    .child(pill("new-card-due", Property::Due, due, form.due.is_some()))
                    .child(pill(
                        "new-card-labels",
                        Property::Labels,
                        labels,
                        !form.labels.is_empty(),
                    ))
                    .child(pill(
                        "new-card-projects",
                        Property::Projects,
                        projects,
                        !form.projects.is_empty(),
                    )),
            )
            .when(form.typing_date, |d| {
                d.child(
                    div()
                        .id("new-card-date")
                        .debug_selector(|| "new-card-date".into())
                        .h(px(28.))
                        .px(px(10.))
                        .rounded(px(8.))
                        .border_1()
                        .border_color(content(0.12))
                        .flex()
                        .items_center()
                        .child(form.date.clone()),
                )
            })
            .children(
                form.error
                    .clone()
                    .map(|error| message("new-card-error", error.into())),
            )
            .child(
                div()
                    .mt(px(4.))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(checkbox)
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(8.))
                            .child(
                                modal::ghost_button("new-card-cancel", "Cancel")
                                    .debug_selector(|| "new-card-cancel".into())
                                    .cursor_pointer()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.close_new_card(Some(window), cx);
                                    })),
                            )
                            .child(
                                modal::primary_button("new-card-create", "Create card")
                                    .debug_selector(|| "new-card-create".into())
                                    .font_weight(FontWeight::MEDIUM)
                                    .cursor_pointer()
                                    .track_focus(&form.create_focus)
                                    .tab_index(0)
                                    .focus_visible(|s| s.border_1().border_color(accent_color()))
                                    .on_key_down(cx.listener(
                                        |this, event: &gpui::KeyDownEvent, window, cx| {
                                            if matches!(
                                                event.keystroke.key.as_str(),
                                                "enter" | "space"
                                            ) {
                                                cx.stop_propagation();
                                                this.submit_new_card(window, cx);
                                            }
                                        },
                                    ))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.submit_new_card(window, cx)
                                    })),
                            ),
                    ),
            );
        let weak = cx.weak_entity();
        let card = modal::card().w(px(520.)).child(body);
        let menu = form.menu.as_ref().map(|open| {
            menu_popover(&open.menu, Anchor::Slot(anchor(open.property)), cx).into_any_element()
        });
        Some(
            div()
                .id("new-card")
                .debug_selector(|| "new-card".into())
                .absolute()
                .inset_0()
                .child(modal::overlay("new-card", card, move |window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.close_new_card(Some(window), cx);
                    });
                }))
                .children(menu)
                .into_any_element(),
        )
    }
}

/// Rows of a multi-pick menu stay open after each pick.
fn multi_keep(items: Vec<MenuItem>) -> Vec<MenuItem> {
    items
        .into_iter()
        .map(|item| match item {
            MenuItem::Action(mut action) => {
                action.keep_open = true;
                MenuItem::Action(action)
            }
            other => other,
        })
        .collect()
}
