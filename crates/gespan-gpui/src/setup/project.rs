//! The project card: the launch folder as a suggestion, the folder picker
//! and a remote machine's folder. Required, so there is no Skip.
use super::{
    SetupStep,
    card::{SetupCard, SetupEvent},
    rules::suggested_folder,
    ui,
};
use crate::theme::{content, icon};
use gespan_core::project_appearance::ProjectAppearance;
use gpui::{
    AnyElement, Context, FontWeight, Role, SharedString, Stateful, Window, div, prelude::*, px, rgb,
};
use std::path::Path;

pub(super) fn render(
    card: &mut SetupCard,
    _: &mut Window,
    cx: &mut Context<SetupCard>,
) -> AnyElement {
    let suggestion = suggested_folder(&card.launch, &card.home);
    let picked = card.folder.clone();
    let other = picked
        .clone()
        .filter(|folder| Some(folder) != suggestion.as_ref());

    let mut body = ui::card()
        .child(ui::header(ui::steps(3), None))
        .child(ui::intro(
            "Pick a project to start in.",
            "Any folder works, a git repository works best. Gespan opens here from now on. \
         Add more projects later from the project menu.",
        ));
    if let Some(line) = defaults_line(card) {
        body = body.child(
            ui::t2(line)
                .px(px(14.))
                .py(px(10.))
                .rounded(px(10.))
                .bg(ui::inset())
                .border_1()
                .border_color(ui::inset_edge()),
        );
    }
    if let Some(path) = suggestion {
        let selected = picked.as_deref() == Some(path.as_str());
        body = body.child(
            div()
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(ui::t3("Opened from"))
                .child(
                    ui::list().child(
                        folder_row("setup-suggested", &path, &card.home, selected)
                            .debug_selector(|| "setup-suggested".into())
                            .on_click(cx.listener(move |card, _, _, cx| {
                                if card.folder.as_deref() == Some(path.as_str()) {
                                    card.folder = None;
                                    cx.notify();
                                } else {
                                    card.set_folder(path.clone(), cx);
                                }
                            })),
                    ),
                ),
        );
    }

    let continue_label: SharedString = match &picked {
        Some(folder) => format!("Open {}", folder_name(folder)).into(),
        None => "Open project".into(),
    };
    let mut right = Vec::new();
    if picked.is_none() {
        right.push(ui::t3("Pick a folder to finish").into_any_element());
    }
    right.push(
        ui::primary("setup-continue", continue_label, picked.is_some())
            .debug_selector(|| "setup-continue".into())
            .when(picked.is_some(), |button| {
                button.on_click(cx.listener(|card, _, _, cx| card.finish(cx)))
            })
            .into_any_element(),
    );

    let shortcut = kbd(if cfg!(target_os = "macos") {
        "⌘O"
    } else {
        "Ctrl O"
    });
    // A folder picked from the picker shows in its own tile, tinted.
    let choose = match &other {
        Some(path) => {
            let (name, detail) = describe(path, &card.home);
            choice(
                "setup-choose-folder",
                icon("folder", 18.)
                    .text_color(content(1.))
                    .into_any_element(),
                name,
                detail,
                true,
                Some(shortcut),
            )
        }
        None => choice(
            "setup-choose-folder",
            icon("folder", 18.)
                .text_color(content(1.))
                .into_any_element(),
            "Choose a folder…".into(),
            "Pick any folder on this Mac".into(),
            false,
            Some(shortcut),
        ),
    };
    let choose = choose.debug_selector(|| "setup-choose-folder".into());
    body.child(
        div()
            .flex()
            .gap(px(10.))
            .child(choose.on_click(cx.listener(|_, _, _, cx| cx.emit(SetupEvent::ChooseFolder))))
            .child(
                choice(
                    "setup-choose-remote",
                    monitor().into_any_element(),
                    "Folder on a machine…".into(),
                    "A remote machine you've added".into(),
                    false,
                    None,
                )
                .debug_selector(|| "setup-choose-remote".into())
                .on_click(cx.listener(|_, _, _, cx| cx.emit(SetupEvent::ChooseRemoteFolder))),
            ),
    )
    .child(ui::footer(
        Some(
            ui::ghost("setup-back", "Back", 32.)
                .debug_selector(|| "setup-back".into())
                .on_click(cx.listener(|card, _, _, cx| card.go(SetupStep::Look, cx)))
                .into_any_element(),
        ),
        right,
    ))
    .into_any_element()
}

/// "Using defaults: …" naming what the skipped cards left as it was: the
/// agent for Agents; theme and glass for Look. `None` when nothing
/// was skipped.
fn defaults_line(card: &SetupCard) -> Option<String> {
    let mut parts: Vec<&str> = Vec::new();
    if card.skipped.contains(&SetupStep::Agents) {
        parts.push(
            card.default_agent
                .map_or("no agent yet", |kind| kind.label()),
        );
    }
    if card.skipped.contains(&SetupStep::Look) {
        parts.push(match card.prefs.theme.as_str() {
            "system" => "System theme",
            "light" => "Light theme",
            _ => "Dark theme",
        });
        parts.push(if card.prefs.inset_glass {
            "glass on"
        } else {
            "glass off"
        });
    }
    (!parts.is_empty()).then(|| format!("Using defaults: {}", parts.join(", ")))
}

/// The last path component, or the whole path for a root.
fn folder_name(path: &str) -> String {
    Path::new(path.trim_end_matches(['/', '\\']))
        .file_name()
        .map_or_else(
            || path.to_string(),
            |name| name.to_string_lossy().into_owned(),
        )
}

/// `path` with the home folder written as `~`.
fn abbreviate(path: &str, home: &str) -> String {
    let home = home.trim_end_matches(['/', '\\']);
    if home.is_empty() {
        return path.to_string();
    }
    match path.strip_prefix(home) {
        Some("") => "~".into(),
        Some(rest) if rest.starts_with(['/', '\\']) => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// A folder's name, and its path with the branch when git metadata for it
/// is already cached.
fn describe(path: &str, home: &str) -> (SharedString, SharedString) {
    let mut detail = abbreviate(path, home);
    // Only the in-memory cache: no git process on the render path.
    if let Some(branch) = crate::git_cache::get(path).and_then(|snapshot| snapshot.current) {
        detail = format!("{detail} · git, on {branch}");
    }
    (folder_name(path).into(), detail.into())
}

/// The suggested project's row: colour tile, name over the path, and an
/// accent check and tint when selected.
fn folder_row(id: &'static str, path: &str, home: &str, selected: bool) -> Stateful<gpui::Div> {
    let (name, detail) = describe(path, home);
    let tile = rgb(ProjectAppearance::default().color(path));
    ui::row(true)
        .id(id)
        .role(Role::Button)
        .aria_label(name.clone())
        .aria_selected(selected)
        .cursor_pointer()
        .when(selected, |row| row.bg(ui::picked()))
        .when(!selected, |row| row.hover(|s| s.bg(content(0.03))))
        .child(div().size(px(30.)).flex_shrink_0().rounded(px(8.)).bg(tile))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                .child(ui::t1(name).font_weight(FontWeight::SEMIBOLD).truncate())
                .child(ui::t3(detail).truncate()),
        )
        .when(selected, |row| {
            row.child(icon("check", 16.).text_color(ui::accent(1.)))
        })
}

/// A pick tile: a dashed edge, or the accent's solid edge and tint once
/// picked; a glyph tile, title over a sub-line, and an optional chip.
fn choice(
    id: &'static str,
    glyph: AnyElement,
    title: SharedString,
    sub: SharedString,
    on: bool,
    chip: Option<gpui::Div>,
) -> Stateful<gpui::Div> {
    div()
        .id(id)
        .role(Role::Button)
        .aria_label(title.clone())
        .aria_selected(on)
        .cursor_pointer()
        .flex_1()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(12.))
        .p(px(14.))
        .rounded(px(14.))
        .border_1()
        .map(|d| {
            if on {
                d.border_color(ui::accent(0.45)).bg(ui::accent(0.08))
            } else {
                d.border_dashed()
                    .border_color(content(0.16))
                    .bg(crate::glass::fill())
                    .hover(|s| s.bg(content(0.05)))
            }
        })
        .child(ui::tile().child(glyph))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .gap(px(2.))
                // The picked folder's own selector, for tests.
                .when(on, |d| d.debug_selector(|| "setup-picked".into()))
                .child(ui::t1(title).font_weight(FontWeight::SEMIBOLD).truncate())
                .child(ui::t3(sub).truncate()),
        )
        .children(chip)
}

/// The shortcut chip: mono 10.5, 1×5 padding, radius 4, the glass edge.
fn kbd(label: &'static str) -> gpui::Div {
    div()
        .flex_shrink_0()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(crate::glass::edge())
        .font_family(crate::fonts::monospace())
        .text_size(px(10.5))
        .text_color(ui::faint())
        .child(label)
}

/// The canvas's 18pt monitor glyph (a 14×9 screen on a stand in a 20 box),
/// drawn with boxes since the asset set has no monitor icon.
fn monitor() -> gpui::Div {
    let ink = content(1.);
    let s = 18. / 20.;
    div()
        .relative()
        .size(px(18.))
        .flex_shrink_0()
        .child(
            div()
                .absolute()
                .left(px(3. * s - 0.75))
                .top(px(4. * s - 0.75))
                .w(px(14. * s + 1.5))
                .h(px(9. * s + 1.5))
                .rounded(px(1.5 * s + 0.75))
                .border(px(1.5))
                .border_color(ink),
        )
        .child(
            div()
                .absolute()
                .left(px(10. * s - 0.75))
                .top(px(13. * s))
                .w(px(1.5))
                .h(px(3. * s))
                .bg(ink),
        )
        .child(
            div()
                .absolute()
                .left(px(7. * s - 0.75))
                .top(px(16. * s - 0.75))
                .w(px(6. * s + 1.5))
                .h(px(1.5))
                .rounded_full()
                .bg(ink),
        )
}

#[cfg(test)]
mod tests {
    use super::super::{SetupStep, card::SetupEvent, test_support::*};
    use super::{abbreviate, defaults_line, folder_name};
    use gespan_core::provider::Kind;
    use gpui::TestAppContext;

    fn finishes(opened: &Opened) -> Vec<String> {
        opened
            .events
            .borrow()
            .iter()
            .filter_map(|event| match event {
                SetupEvent::Finish(path) => Some(path.clone()),
                _ => None,
            })
            .collect()
    }

    #[gpui::test]
    fn project_has_no_skip(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Project, "/Users/a/langfuse", cx);
        assert!(!has(&opened, "setup-skip", cx));
        assert!(!has(&opened, "setup-skip-setup", cx));
        // The card did draw: its actions are there.
        assert!(has(&opened, "setup-continue", cx));
        assert!(has(&opened, "setup-back", cx));
    }

    #[gpui::test]
    fn project_cannot_finish_without_a_folder(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Project, "/", cx);
        assert!(!has(&opened, "setup-suggested", cx));
        click(&opened, "setup-continue", cx);
        assert!(finishes(&opened).is_empty());
        assert_eq!(
            opened.card.read_with(cx, |card, _| card.step()),
            SetupStep::Project
        );
    }

    #[gpui::test]
    fn project_finishes_with_the_suggested_folder(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Project, "/Users/a/langfuse", cx);
        click(&opened, "setup-suggested", cx);
        assert_eq!(
            opened
                .card
                .read_with(cx, |card, _| card.folder.clone())
                .as_deref(),
            Some("/Users/a/langfuse")
        );
        click(&opened, "setup-continue", cx);
        assert_eq!(finishes(&opened), vec!["/Users/a/langfuse".to_string()]);
    }

    #[gpui::test]
    fn suggested_row_toggles_off(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Project, "/Users/a/langfuse", cx);
        click(&opened, "setup-suggested", cx);
        click(&opened, "setup-suggested", cx);
        assert_eq!(
            opened.card.read_with(cx, |card, _| card.folder.clone()),
            None
        );
        click(&opened, "setup-continue", cx);
        assert!(finishes(&opened).is_empty());
    }

    #[gpui::test]
    fn a_picked_folder_fills_the_choose_tile_and_finishes(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Project, "/Users/a/langfuse", cx);
        assert!(!has(&opened, "setup-picked", cx));
        opened
            .card
            .update(cx, |card, cx| card.set_folder("/Users/a/other".into(), cx));
        assert!(has(&opened, "setup-picked", cx));
        assert!(has(&opened, "setup-suggested", cx));
        // The tinted tile is still the picker: clicking it picks again.
        click(&opened, "setup-choose-folder", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::ChooseFolder)
        );
        click(&opened, "setup-continue", cx);
        assert_eq!(finishes(&opened), vec!["/Users/a/other".to_string()]);
    }

    #[gpui::test]
    fn dock_launch_shows_no_suggestion(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Project, "/Users/a", cx);
        assert!(!has(&opened, "setup-suggested", cx));
        assert!(has(&opened, "setup-choose-folder", cx));
    }

    #[gpui::test]
    fn choose_folder_emits_choose_folder(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Project, "/Users/a/langfuse", cx);
        click(&opened, "setup-choose-folder", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::ChooseFolder)
        );
    }

    #[gpui::test]
    fn choose_remote_emits_choose_remote_folder(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Project, "/Users/a/langfuse", cx);
        click(&opened, "setup-choose-remote", cx);
        assert_eq!(
            opened.events.borrow().last(),
            Some(&SetupEvent::ChooseRemoteFolder)
        );
    }

    #[gpui::test]
    fn back_goes_to_look(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Project, "/Users/a/langfuse", cx);
        click(&opened, "setup-back", cx);
        assert_eq!(
            opened.card.read_with(cx, |card, _| card.step()),
            SetupStep::Look
        );
    }

    #[gpui::test]
    fn defaults_line_lists_only_skipped_cards(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Project, "/Users/a/langfuse", cx);
        opened.card.update(cx, |card, _| {
            card.skipped = vec![];
            assert_eq!(defaults_line(card), None);

            card.default_agent = Some(Kind::Claude);
            card.skipped = vec![SetupStep::Agents];
            assert_eq!(
                defaults_line(card),
                Some(format!("Using defaults: {}", Kind::Claude.label()))
            );

            card.default_agent = None;
            assert_eq!(
                defaults_line(card).as_deref(),
                Some("Using defaults: no agent yet")
            );

            card.skipped = vec![SetupStep::Look];
            card.prefs.theme = "system".into();
            card.prefs.inset_glass = true;
            assert_eq!(
                defaults_line(card).as_deref(),
                Some("Using defaults: System theme, glass on")
            );

            card.default_agent = Some(Kind::Claude);
            card.skipped = vec![SetupStep::Agents, SetupStep::Look];
            card.prefs.theme = "light".into();
            card.prefs.inset_glass = false;
            assert_eq!(
                defaults_line(card),
                Some(format!(
                    "Using defaults: {}, Light theme, glass off",
                    Kind::Claude.label()
                ))
            );
        });
    }

    #[test]
    fn paths_show_home_as_tilde_and_name_the_last_folder() {
        assert_eq!(
            abbreviate("/Users/a/Developer/langfuse", "/Users/a"),
            "~/Developer/langfuse"
        );
        assert_eq!(abbreviate("/Users/a", "/Users/a/"), "~");
        // A sibling whose name starts with the home folder's is not inside it.
        assert_eq!(abbreviate("/Users/ab/x", "/Users/a"), "/Users/ab/x");
        assert_eq!(abbreviate("/opt/x", ""), "/opt/x");
        assert_eq!(folder_name("/Users/a/langfuse"), "langfuse");
        assert_eq!(folder_name("/Users/a/langfuse/"), "langfuse");
        assert_eq!(folder_name("/"), "/");
    }
}
