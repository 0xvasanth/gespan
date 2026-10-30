//! The in-window menu bar on Windows and Linux (MenuBar.tsx): a strip at the
//! top of the workspace column with the menus macOS shows in its menu bar
//! (`app_menu::menus`). Items dispatch the same actions to the view that had
//! focus, so shortcuts and enabled states follow the live keymap.
use super::*;
use crate::popover::{
    Anchor, AnchorSlot, Menu as MenuView, MenuAction, MenuEvent, MenuItem as Row, menu_popover,
};
use gpui::Action;

#[derive(Default)]
pub(super) struct MenuBar {
    /// Off on macOS, which has the system menu bar.
    pub(super) shown: bool,
    anchors: Vec<AnchorSlot>,
    open: Option<Open>,
}

impl MenuBar {
    pub(super) fn new(shown: bool) -> Self {
        Self {
            shown,
            ..Default::default()
        }
    }
}

struct Open {
    index: usize,
    menu: Entity<MenuView>,
    actions: Vec<Box<dyn Action>>,
    /// The view to give focus back to before an item's action runs.
    focus: Option<FocusHandle>,
    _events: Subscription,
}

/// A menu's rows, and the action behind each row (its id is the index).
pub(super) fn rows(
    menu: &gpui::Menu,
    window: &Window,
    cx: &App,
) -> (Vec<Row>, Vec<Box<dyn Action>>) {
    let mut rows = Vec::new();
    let mut actions = Vec::new();
    for item in &menu.items {
        match item {
            gpui::MenuItem::Separator => {
                if !matches!(rows.last(), None | Some(Row::Separator)) {
                    rows.push(Row::Separator);
                }
            }
            gpui::MenuItem::Action {
                name,
                action,
                checked,
                ..
            } => {
                let mut row = MenuAction::new(actions.len().to_string(), name.clone())
                    .disabled(!window.is_action_available(action.as_ref(), cx))
                    .checked(*checked);
                if let Some(binding) = window.highest_precedence_binding_for_action(action.as_ref())
                {
                    let keys = binding
                        .keystrokes()
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ");
                    row = row.shortcut(crate::settings::shortcut_label(&keys));
                }
                rows.push(row.into());
                actions.push(action.boxed_clone());
            }
            gpui::MenuItem::Submenu(_) | gpui::MenuItem::SystemMenu(_) => {}
        }
    }
    if matches!(rows.last(), Some(Row::Separator)) {
        rows.pop();
    }
    (rows, actions)
}

impl Shell {
    fn bar_menus(&self) -> Vec<gpui::Menu> {
        crate::app_menu::menus(&self.workspace.preferences, false)
    }

    pub(super) fn menu_bar_open(&self) -> Option<usize> {
        self.menu_bar.open.as_ref().map(|open| open.index)
    }

    /// Open menu `index`, closing any other; opening the open one closes it.
    pub(super) fn toggle_bar_menu(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.menu_bar_open() == Some(index) {
            self.close_bar_menu(cx);
            return;
        }
        self.open_bar_menu(index, window, cx);
    }

    pub(super) fn open_bar_menu(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menus = self.bar_menus();
        let Some(menu) = menus.get(index) else {
            return;
        };
        // Enabled states and the action target are the view focused now,
        // not the menu about to take focus.
        let focus = self
            .menu_bar
            .open
            .take()
            .and_then(|open| open.focus)
            .or_else(|| window.focused(cx));
        let (rows, actions) = rows(menu, window, cx);
        let view = cx.new(|cx| MenuView::new(menu.name.clone(), rows, window, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event: &MenuEvent, window, cx| {
            let Some(open) = this.menu_bar.open.take() else {
                return;
            };
            if let Some(focus) = &open.focus {
                window.focus(focus, cx);
            }
            if let MenuEvent::Picked(id, _) = event
                && let Some(action) = id.parse::<usize>().ok().and_then(|ix| open.actions.get(ix))
            {
                window.dispatch_action(action.boxed_clone(), cx);
            }
            cx.notify();
        });
        self.menu_bar.open = Some(Open {
            index,
            menu: view,
            actions,
            focus,
            _events: events,
        });
        cx.notify();
    }

    pub(super) fn close_bar_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu_bar.open.take().is_some() {
            cx.notify();
        }
    }

    /// MenuBar.tsx: `h-7` titles with a hover wash; with a menu open, hovering
    /// another title moves to it.
    pub(super) fn menu_bar(&mut self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.menu_bar.shown {
            return None;
        }
        let menus = self.bar_menus();
        self.menu_bar
            .anchors
            .resize_with(menus.len(), AnchorSlot::default);
        let open = self.menu_bar_open();
        let titles = menus.iter().enumerate().map(|(index, menu)| {
            let anchor = self.menu_bar.anchors[index].clone();
            let active = open == Some(index);
            div()
                .id(("menu-bar", index))
                .debug_selector(move || format!("menu-bar-{index}"))
                .relative()
                .h(px(20.))
                .px(px(8.))
                .flex()
                .items_center()
                .rounded(px(4.))
                .text_size(px(12.))
                .text_color(crate::theme::content(if active { 1. } else { 0.7 }))
                .when(active, |d| d.bg(crate::theme::content(0.1)))
                .hover(|d| {
                    d.bg(crate::theme::content(0.1))
                        .text_color(crate::theme::content(1.))
                })
                .child(anchor.probe())
                .child(menu.name.clone())
                .on_click(
                    cx.listener(move |this, _, window, cx| this.toggle_bar_menu(index, window, cx)),
                )
                .on_hover(cx.listener(move |this, hovered: &bool, window, cx| {
                    if *hovered && this.menu_bar_open().is_some_and(|open| open != index) {
                        this.open_bar_menu(index, window, cx);
                    }
                }))
        });
        let popover = self.menu_bar.open.as_ref().map(|open| {
            menu_popover(
                &open.menu,
                Anchor::Slot(self.menu_bar.anchors[open.index].clone()),
                cx,
            )
            .gap(2.)
        });
        Some(
            div()
                .id("menu-bar")
                .h(px(28.))
                .w_full()
                .flex_shrink_0()
                .flex()
                .items_center()
                .gap(px(2.))
                .px(px(8.))
                .border_b_1()
                .border_color(crate::right_panel::stroke())
                .bg(crate::theme::content(0.05))
                .children(titles)
                .children(popover)
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gespan_core::store::Store;
    use gpui::{TestAppContext, WindowHandle};
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    fn names(menus: &[gpui::Menu]) -> Vec<&str> {
        menus.iter().map(|menu| menu.name.as_ref()).collect()
    }

    fn labels(menu: &gpui::Menu) -> Vec<String> {
        menu.items
            .iter()
            .map(|item| match item {
                gpui::MenuItem::Action { name, .. } => name.to_string(),
                gpui::MenuItem::Separator => "-".into(),
                _ => "?".into(),
            })
            .collect()
    }

    fn find<'a>(menus: &'a [gpui::Menu], name: &str) -> &'a gpui::Menu {
        menus.iter().find(|menu| menu.name == name).unwrap()
    }

    #[test]
    fn macos_keeps_its_app_and_window_menus() {
        let menus = crate::app_menu::menus(&Default::default(), true);
        assert_eq!(
            names(&menus),
            ["Gespan Native", "File", "Edit", "View", "Window", "Help"]
        );
    }

    #[test]
    fn help_links_to_the_website_then_github() {
        for mac in [true, false] {
            let menus = crate::app_menu::menus(&Default::default(), mac);
            let help = labels(find(&menus, "Help"));
            let website = help.iter().position(|l| l == "Gespan Website").unwrap();
            assert_eq!(help[website + 1], "View on GitHub");
        }
        // The link goes to the site, not the repository.
        let menus = crate::app_menu::menus(&Default::default(), true);
        let actions: Vec<_> = find(&menus, "Help")
            .items
            .iter()
            .filter_map(|item| match item {
                gpui::MenuItem::Action { name, action, .. }
                    if name.as_ref() == "Gespan Website" =>
                {
                    action
                        .as_any()
                        .downcast_ref::<crate::app_menu::OpenUrl>()
                        .map(|u| u.0)
                }
                _ => None,
            })
            .collect();
        assert_eq!(actions, ["https://gespan.dev"]);
    }

    #[test]
    fn in_the_window_the_app_menu_moves_to_file_and_help() {
        let menus = crate::app_menu::menus(&Default::default(), false);
        assert_eq!(names(&menus), ["File", "Edit", "View", "Help"]);
        let file = labels(find(&menus, "File"));
        assert_eq!(file[..2], ["New Window", "Open Project…"]);
        assert_eq!(
            &file[file.len() - 4..],
            ["Settings…", "Hold Everything", "-", "Quit"]
        );
        let help = labels(find(&menus, "Help"));
        assert_eq!(
            &help[..3],
            ["About Gespan Native", "Check for Updates…", "-"]
        );
        assert!(help.iter().any(|label| label == "Gespan Website"));
        // Nothing hides the app or its others: those are macOS-only.
        let all: Vec<String> = menus.iter().flat_map(labels).collect();
        assert!(!all.iter().any(|label| label.starts_with("Hide")));
    }

    fn shell(cx: &mut TestAppContext) -> WindowHandle<Shell> {
        let store = Arc::new(Mutex::new(Store::open(Path::new(":memory:")).unwrap()));
        cx.update(crate::shell::bind_keys);
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| {
                    let mut shell = Shell::new(store, vec![], "/missing/menu-bar".into(), None, cx);
                    shell.menu_bar.shown = true;
                    shell
                })
            })
            .unwrap()
        });
        cx.run_until_parked();
        window
    }

    fn index(window: WindowHandle<Shell>, name: &str, cx: &mut TestAppContext) -> usize {
        window
            .update(cx, |shell, _, _| {
                shell
                    .bar_menus()
                    .iter()
                    .position(|menu| menu.name == name)
                    .unwrap()
            })
            .unwrap()
    }

    type RowView = (String, String, bool, bool, Option<String>);

    /// The rows menu `ix` shows: id, label, disabled, checked, shortcut.
    fn menu_rows(
        window: WindowHandle<Shell>,
        ix: usize,
        cx: &mut TestAppContext,
    ) -> (Vec<RowView>, bool) {
        window
            .update(cx, |shell, window, cx| {
                let (rows, _) = rows(&shell.bar_menus()[ix], window, cx);
                let clean = !matches!(rows.last(), Some(Row::Separator))
                    && !rows
                        .windows(2)
                        .any(|w| matches!(w, [Row::Separator, Row::Separator]));
                let views = rows
                    .iter()
                    .filter_map(|item| match item {
                        Row::Action(action) => Some((
                            action.id.to_string(),
                            action.label.to_string(),
                            action.disabled,
                            action.checked,
                            action.shortcut.as_ref().map(ToString::to_string),
                        )),
                        _ => None,
                    })
                    .collect();
                (views, clean)
            })
            .unwrap()
    }

    #[gpui::test]
    fn only_shown_off_macos(cx: &mut TestAppContext) {
        let store = Arc::new(Mutex::new(Store::open(Path::new(":memory:")).unwrap()));
        cx.update(crate::shell::bind_keys);
        let window = cx.update(|cx| {
            cx.open_window(Default::default(), |_, cx| {
                cx.new(|cx| Shell::new(store, vec![], "/missing/menu-bar".into(), None, cx))
            })
            .unwrap()
        });
        cx.run_until_parked();
        let shown = window
            .update(cx, |shell, _, _| shell.menu_bar.shown)
            .unwrap();
        assert_eq!(shown, !cfg!(target_os = "macos"));
    }

    #[gpui::test]
    fn clicking_a_title_opens_its_menu_and_again_closes_it(cx: &mut TestAppContext) {
        let window = shell(cx);
        let view = index(window, "View", cx);
        window
            .update(cx, |shell, window, cx| {
                shell.toggle_bar_menu(view, window, cx)
            })
            .unwrap();
        assert_eq!(
            window.update(cx, |s, _, _| s.menu_bar_open()).unwrap(),
            Some(view)
        );
        let labels: Vec<String> = menu_rows(window, view, cx)
            .0
            .into_iter()
            .map(|row| row.1)
            .collect();
        assert!(labels.contains(&"Toggle Sidebar".to_string()));
        window
            .update(cx, |shell, window, cx| {
                shell.toggle_bar_menu(view, window, cx)
            })
            .unwrap();
        assert_eq!(
            window.update(cx, |s, _, _| s.menu_bar_open()).unwrap(),
            None
        );
    }

    #[gpui::test]
    fn rows_show_shortcuts_and_checked_settings(cx: &mut TestAppContext) {
        let window = shell(cx);
        let file = index(window, "File", cx);
        window
            .update(cx, |shell, window, cx| {
                shell.workspace.preferences.autosave = true;
                shell.toggle_bar_menu(file, window, cx)
            })
            .unwrap();
        let (rows, clean) = menu_rows(window, file, cx);
        let row = |label: &str| rows.iter().find(|row| row.1 == label).unwrap().clone();
        assert!(row("Autosave").3, "Autosave is checked when it is on");
        assert!(!row("New Terminal").3);
        let palette = row("Command Palette…")
            .4
            .expect("bound actions show their keys");
        assert!(palette.ends_with('P'), "{palette}");
        assert!(clean, "rows are separated, never doubled or trailing");
    }

    #[gpui::test]
    fn hovering_another_title_moves_an_open_menu_there(cx: &mut TestAppContext) {
        let window = shell(cx);
        let (file, view) = (index(window, "File", cx), index(window, "View", cx));
        window
            .update(cx, |shell, window, cx| {
                shell.open_bar_menu(file, window, cx);
                shell.open_bar_menu(view, window, cx);
            })
            .unwrap();
        assert_eq!(
            window.update(cx, |s, _, _| s.menu_bar_open()).unwrap(),
            Some(view)
        );
    }

    #[gpui::test]
    fn picking_an_item_runs_its_action_and_closes_the_menu(cx: &mut TestAppContext) {
        let window = shell(cx);
        let view = index(window, "View", cx);
        let before = window
            .update(cx, |s, _, _| s.workspace.sidebar_visible)
            .unwrap();
        window
            .update(cx, |shell, window, cx| {
                shell.toggle_bar_menu(view, window, cx)
            })
            .unwrap();
        let id = menu_rows(window, view, cx)
            .0
            .into_iter()
            .find(|row| row.1 == "Toggle Sidebar")
            .unwrap()
            .0;
        window
            .update(cx, |shell, _, cx| {
                let menu = shell.menu_bar.open.as_ref().unwrap().menu.clone();
                menu.update(cx, |_, cx| cx.emit(MenuEvent::Picked(id.into(), None)));
            })
            .unwrap();
        cx.run_until_parked();
        let (after, open) = window
            .update(cx, |s, _, _| {
                (s.workspace.sidebar_visible, s.menu_bar_open())
            })
            .unwrap();
        assert_eq!(after, !before, "Toggle Sidebar ran");
        assert_eq!(open, None);
    }

    #[gpui::test]
    fn dismissing_runs_nothing(cx: &mut TestAppContext) {
        let window = shell(cx);
        let view = index(window, "View", cx);
        let before = window
            .update(cx, |s, _, _| s.workspace.sidebar_visible)
            .unwrap();
        window
            .update(cx, |shell, window, cx| {
                shell.toggle_bar_menu(view, window, cx);
                let menu = shell.menu_bar.open.as_ref().unwrap().menu.clone();
                menu.update(cx, |_, cx| cx.emit(MenuEvent::Dismissed));
            })
            .unwrap();
        cx.run_until_parked();
        let (after, open) = window
            .update(cx, |s, _, _| {
                (s.workspace.sidebar_visible, s.menu_bar_open())
            })
            .unwrap();
        assert_eq!((after, open), (before, None));
    }
}
