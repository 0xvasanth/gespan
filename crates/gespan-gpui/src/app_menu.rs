//! The native menu bar, after the Tauri app's `menu.rs`. Items dispatch GPUI
//! actions, so their shortcuts follow the live keymap and items the focused
//! view cannot handle are disabled.
use crate::{
    Quit,
    file_editor::{Find, Format, ReloadFile, Replace, Save, ToggleAutosave, ToggleWordWrap},
    input,
    shell::*,
};
use gpui::{App, KeyBinding, Menu, MenuItem, actions};

actions!(application, [Hide, HideOthers, ShowAll]);

/// Opens a Help menu link in the browser.
#[derive(Clone, Debug, PartialEq, gpui::Action)]
#[action(namespace = application, no_json)]
pub(crate) struct OpenUrl(pub &'static str);

const REPO: &str = "https://github.com/0xvasanth/gespan";
const WEBSITE: &str = "https://gespan.dev";

pub(crate) fn bind(cx: &mut App) {
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|action: &OpenUrl, cx| cx.open_url(action.0));
    // App-level, so it works with no window in front. Deferred, because the
    // window it was pressed in cannot be read until its dispatch ends.
    cx.on_action(|_: &NewWindow, cx| cx.defer(crate::windows::open_new_window));
    if cfg!(target_os = "macos") {
        cx.bind_keys([
            KeyBinding::new("cmd-h", Hide, None),
            KeyBinding::new("cmd-alt-h", HideOthers, None),
            KeyBinding::new("cmd-m", MinimizeWindow, None),
        ]);
    }
}

pub(crate) fn install(prefs: &crate::settings::Preferences, cx: &mut App) {
    cx.set_menus(menus(prefs, cfg!(target_os = "macos")));
    // Windows would turn this into a jump list, which the Tauri app has not.
    if cfg!(target_os = "macos") {
        cx.set_dock_menu(dock_menu());
    }
}

/// What the Dock icon's menu adds on macOS.
pub(crate) fn dock_menu() -> Vec<MenuItem> {
    vec![MenuItem::action("New Window", NewWindow)]
}

/// The menus, for the macOS menu bar or (`mac` false) the in-window menu bar
/// on Windows and Linux. There the app menu's items move to File and Help,
/// Hide and the Window menu are dropped, and the window's own buttons
/// minimize and maximize.
pub(crate) fn menus(prefs: &crate::settings::Preferences, mac: bool) -> Vec<Menu> {
    let mut menus = vec![
        Menu::new("Gespan Native").items([
            MenuItem::action("About Gespan Native", About),
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::action("Check for Updates…", CheckForUpdates),
            MenuItem::separator(),
            MenuItem::action("Hold Everything", ToggleHold),
            MenuItem::separator(),
            MenuItem::action("Hide Gespan Native", Hide),
            MenuItem::action("Hide Others", HideOthers),
            MenuItem::action("Show All", ShowAll),
            MenuItem::separator(),
            MenuItem::action("Quit Gespan Native", Quit),
        ]),
        Menu::new("File").items([
            MenuItem::action("New Window", NewWindow),
            MenuItem::action("Open Project…", OpenProject),
            MenuItem::action("Search…", FocusSearch),
            MenuItem::action("Go to File…", GoToFile),
            MenuItem::action("Command Palette…", OpenCommandPalette),
            MenuItem::action("Find in Files…", SearchFiles),
            MenuItem::separator(),
            MenuItem::action("Save", Save),
            MenuItem::action("Reload File", ReloadFile),
            MenuItem::action("Autosave", ToggleAutosave).checked(prefs.autosave),
            MenuItem::separator(),
            MenuItem::action("New Tab", NewCodex),
            MenuItem::action("New Claude Tab", NewClaude),
            MenuItem::action("New Terminal", NewTerminal),
            MenuItem::action("Split Pane Right", SplitRight),
            MenuItem::action("Split Pane Down", SplitDown),
            MenuItem::action("Close Pane", CloseTab),
            MenuItem::action("Close Other Tabs", CloseOtherTabs),
            MenuItem::action("Close All Tabs", CloseAllTabs),
            MenuItem::separator(),
            MenuItem::action("Previous Tab", PreviousTab),
            MenuItem::action("Next Tab", NextTab),
            MenuItem::action("Back", VisitBack),
            MenuItem::action("Forward", VisitForward),
        ]),
        Menu::new("Edit").items([
            MenuItem::action("Undo", input::Undo),
            MenuItem::action("Redo", input::Redo),
            MenuItem::separator(),
            MenuItem::action("Cut", input::Cut),
            MenuItem::action("Copy", input::Copy),
            MenuItem::action("Paste", input::Paste),
            MenuItem::action("Select All", input::SelectAll),
            MenuItem::separator(),
            MenuItem::action("Find", Find),
            MenuItem::action("Replace", Replace),
            MenuItem::action("Format Document", Format),
            MenuItem::action("Word Wrap", ToggleWordWrap).checked(prefs.file_word_wrap),
            MenuItem::separator(),
            MenuItem::action("Fold", input::FoldCode),
            MenuItem::action("Unfold", input::UnfoldCode),
            MenuItem::action("Fold All", input::FoldAll),
            MenuItem::action("Unfold All", input::UnfoldAll),
        ]),
        Menu::new("View").items([
            MenuItem::action("Toggle Sidebar", ToggleSidebar),
            MenuItem::action("Toggle Session Sidebar", ToggleRightPanel),
            MenuItem::action("Toggle Layout", ToggleLayout),
            MenuItem::action("Toggle Workspace Style", ToggleWorkspaceStyle),
            MenuItem::action("Inbox", OpenInbox),
            MenuItem::action("Notes", OpenNotes),
            MenuItem::action("Toggle Terminal", ToggleTerminal),
            MenuItem::action("Switch Model…", SwitchModel),
            MenuItem::separator(),
            MenuItem::action("Previous Session", PreviousSession),
            MenuItem::action("Next Session", NextSession),
            MenuItem::action("Previous Project", PreviousProject),
            MenuItem::action("Next Project", NextProject),
            MenuItem::separator(),
            MenuItem::action("Focus Left", FocusPaneLeft),
            MenuItem::action("Focus Right", FocusPaneRight),
            MenuItem::action("Focus Up", FocusPaneUp),
            MenuItem::action("Focus Down", FocusPaneDown),
            MenuItem::separator(),
            MenuItem::action("Zoom In", ZoomIn),
            MenuItem::action("Zoom Out", ZoomOut),
            MenuItem::action("Reset Zoom", ResetZoom),
            MenuItem::action("Reload", Reload),
            MenuItem::separator(),
            MenuItem::action("Sidebar Appearance…", SidebarAppearance),
        ]),
        // AppKit adds its window list here once the menu is named "Window".
        Menu::new("Window").items([
            MenuItem::action("Minimize", MinimizeWindow),
            MenuItem::action("Zoom", ZoomWindow),
        ]),
        Menu::new("Help").items([
            MenuItem::action("Report an issue…", crate::report::ReportIssue),
            MenuItem::separator(),
            MenuItem::action("Gespan Website", OpenUrl(WEBSITE)),
            MenuItem::action("View on GitHub", OpenUrl(REPO)),
            MenuItem::separator(),
            MenuItem::action(
                "Report a Bug…",
                OpenUrl(concat!(
                    "https://github.com/0xvasanth/gespan",
                    "/issues/new?template=bug_report.yml"
                )),
            ),
            MenuItem::action(
                "Request a Feature…",
                OpenUrl(concat!(
                    "https://github.com/0xvasanth/gespan",
                    "/issues/new?template=feature_request.yml"
                )),
            ),
        ]),
    ];
    if !mac {
        menus.remove(0);
        menus.retain(|menu| menu.name != "Window");
        let file = &mut menus[0].items;
        file.extend([
            MenuItem::separator(),
            MenuItem::action("Settings…", OpenSettings),
            MenuItem::action("Hold Everything", ToggleHold),
            MenuItem::separator(),
            MenuItem::action("Quit", Quit),
        ]);
        if let Some(help) = menus.iter_mut().find(|menu| menu.name == "Help") {
            help.items.splice(
                0..0,
                [
                    MenuItem::action("About Gespan Native", About),
                    MenuItem::action("Check for Updates…", CheckForUpdates),
                    MenuItem::separator(),
                ],
            );
        }
    }
    menus
}
