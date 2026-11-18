//! The shared tools card: Gespan's MCP hub, the servers found in the agents'
//! own configs, and whether agents should use only the hub's.
use super::{SetupStep, card::SetupCard, ui};
use gespan_core::mcp::{Importable, Survey};
use gpui::{AnyElement, Context, Div, SharedString, Window, div, prelude::*, px, rgb};

pub(super) fn render(
    card: &mut SetupCard,
    _: &mut Window,
    cx: &mut Context<SetupCard>,
) -> AnyElement {
    let skip_setup = ui::skip_link("setup-skip-setup")
        .debug_selector(|| "setup-skip-setup".into())
        .on_click(cx.listener(|card, _, _, cx| card.skip_setup(cx)));
    let back = ui::ghost("setup-back", "Back", 32.)
        .debug_selector(|| "setup-back".into())
        .on_click(cx.listener(|card, _, _, cx| {
            if !card.tools.busy {
                card.go(SetupStep::Agents, cx)
            }
        }));
    let skip = ui::ghost("setup-skip", "Skip", 34.)
        .debug_selector(|| "setup-skip".into())
        .on_click(cx.listener(|card, _, _, cx| card.skip(SetupStep::Look, cx)));
    let ready = card.tools.found.is_some() && !card.tools.busy;
    let next = ui::primary("setup-continue", "Continue", ready)
        .debug_selector(|| "setup-continue".into())
        .on_click(cx.listener(|card, _, _, cx| card.tools_next(cx)));

    let mut body = ui::card()
        .child(ui::header(ui::steps(2), Some(skip_setup)))
        .child(ui::intro(
            "Shared tools.",
            "Gespan runs one copy of each tool server and shares it with every agent \
             session, so sessions stop starting their own.",
        ))
        .child(server_list(card.tools.found.as_ref()))
        .child(switch_row(card.tools.on, cx));
    if let Some(error) = &card.tools.error {
        body = body.child(
            ui::t2(SharedString::from(format!(
                "Couldn't move your servers: {error}"
            )))
            .debug_selector(|| "setup-tools-error".into())
            .text_color(rgb(0xf26d6d)),
        );
    }
    body.child(
        ui::t3(
            "Servers set up inside a project folder aren't moved, and stop loading there \
             while this is on. Add them later in Settings → MCP.",
        )
        .debug_selector(|| "setup-tools-project-note".into()),
    )
    .child(ui::t3(
        "Cursor and Kiro still load their own servers as well.",
    ))
    .child(ui::footer(
        Some(back.into_any_element()),
        vec![skip.into_any_element(), next.into_any_element()],
    ))
    .into_any_element()
}

/// The servers found, one row each, then the ones that can't be moved; a
/// line while looking or when there are none.
fn server_list(found: Option<&Survey>) -> Div {
    let list = ui::list();
    let Some(found) = found else {
        return list.child(ui::row(true).child(ui::t2("Looking for your agents' tool servers…")));
    };
    if found.movable.is_empty() && found.left_behind.is_empty() {
        return list.child(
            ui::row(true)
                .debug_selector(|| "setup-tools-empty".into())
                .child(ui::t2(
                    "None of your agents has tool servers set up yet. \
                     You can add some later in Settings → MCP.",
                )),
        );
    }
    let moved = found.movable.iter().enumerate().map(|(i, server)| {
        let mut title = div()
            .flex()
            .gap(px(8.))
            .child(ui::t1(server.name.clone()))
            .child(ui::t3(format!("{} ({})", server.provider, server.scope)));
        if !server.enabled {
            title = title.child(
                ui::t3(format!("off in {}, stays off", server.provider))
                    .debug_selector(move || format!("setup-tools-off-{i}")),
            );
        }
        ui::row(i == 0)
            .debug_selector(move || format!("setup-tools-row-{i}"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .min_w_0()
                    .child(title)
                    .child(ui::t3(command_line(server)).truncate())
                    .when(!server.env.is_empty(), |column| {
                        // What it's given, by name: approving it hands these over.
                        let names: Vec<&str> = server.env.keys().map(String::as_str).collect();
                        column.child(
                            ui::t3(format!("Gets {}", names.join(", ")))
                                .debug_selector(move || format!("setup-tools-env-{i}")),
                        )
                    }),
            )
    });
    let first_left = found.movable.is_empty();
    let left = found.left_behind.iter().enumerate().map(move |(i, server)| {
        ui::row(first_left && i == 0)
            .debug_selector(move || format!("setup-tools-left-{i}"))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .gap(px(8.))
                            .child(ui::t1(server.name.clone()))
                            .child(ui::t3(server.provider.clone())),
                    )
                    .child(
                        ui::t3(
                            "Can't be moved as written (a variable that isn't set, or an \
                             address Gespan won't send secrets to). Stops loading while this is on.",
                        )
                        .text_color(rgb(0xf2a65a)),
                    ),
            )
    });
    list.children(moved).children(left)
}

/// What the server runs: its address, or its command and arguments.
fn command_line(server: &Importable) -> String {
    match &server.url {
        Some(url) => url.clone(),
        None => std::iter::once(server.command.as_str())
            .chain(server.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" "),
    }
}

fn switch_row(on: bool, cx: &mut Context<SetupCard>) -> Div {
    ui::list().child(
        ui::row(true)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(ui::t2("Agents use only Gespan's tools"))
                    .child(ui::t3(if on {
                        "The servers above are approved and run through Gespan."
                    } else {
                        "Agents keep loading their own servers. Change this in Settings → MCP."
                    })),
            )
            .child(div().flex_1())
            .child(
                ui::switch("setup-tools-switch", "Agents use only Gespan's tools", on)
                    .on_click(cx.listener(|card, _, _, cx| card.toggle_tools(cx))),
            ),
    )
}

#[cfg(test)]
mod tests {
    use super::super::{SetupStep, card::SetupEvent, test_support::*};
    use gespan_core::mcp::{Importable, LeftBehind, Survey};
    use gespan_core::mcp_catalog::Catalog;
    use gpui::TestAppContext;
    use std::path::{Path, PathBuf};

    fn server(name: &str, command: &str) -> Importable {
        Importable {
            provider: "claude".into(),
            scope: "user".into(),
            name: name.into(),
            command: command.into(),
            args: vec!["-y".into(), "server".into()],
            env: Default::default(),
            url: None,
            enabled: true,
        }
    }

    /// A Tools card whose catalogue lives in `dir` and whose discovery found
    /// `found` (`None`: still looking).
    fn opened(dir: &Path, found: Option<Vec<Importable>>, cx: &mut TestAppContext) -> Opened {
        gespan_tracker_secrets::testing::use_memory_keychain();
        let opened = open(SetupStep::Tools, "/Users/a/code", cx);
        let dir = dir.to_path_buf();
        opened.card.update(cx, |card, cx| {
            card.set_tools_dir(dir);
            if let Some(found) = found {
                card.set_survey(
                    Survey {
                        movable: found,
                        left_behind: Vec::new(),
                    },
                    cx,
                );
            }
        });
        cx.run_until_parked();
        opened
    }

    fn catalogue(dir: &Path) -> PathBuf {
        dir.join("mcp-catalog.json")
    }

    fn last(opened: &Opened) -> Option<SetupEvent> {
        opened.events.borrow().last().cloned()
    }

    #[gpui::test]
    fn tools_skip_goes_to_look_and_records_the_skip(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(tmp.path(), Some(vec![server("a", "npx")]), cx);
        click(&opened, "setup-skip", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Step(SetupStep::Look)));
        assert!(
            opened
                .card
                .read_with(cx, |c, _| c.skipped.contains(&SetupStep::Tools))
        );
        assert!(!catalogue(tmp.path()).exists(), "skip writes nothing");
    }

    #[gpui::test]
    fn tools_back_goes_to_agents(cx: &mut TestAppContext) {
        let opened = open(SetupStep::Tools, "/Users/a/code", cx);
        click(&opened, "setup-back", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Step(SetupStep::Agents)));
    }

    #[gpui::test]
    fn tools_has_the_step_header_and_skip_setup(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(tmp.path(), Some(vec![]), cx);
        assert!(has(&opened, "setup-step-header", cx));
        click(&opened, "setup-skip-setup", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Step(SetupStep::Project)));
        assert!(!catalogue(tmp.path()).exists());
    }

    #[gpui::test]
    fn the_switch_starts_on(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(tmp.path(), Some(vec![]), cx);
        assert!(opened.card.read_with(cx, |c, _| c.tools.on));
        click(&opened, "setup-tools-switch", cx);
        assert!(!opened.card.read_with(cx, |c, _| c.tools.on));
    }

    #[gpui::test]
    fn next_with_the_switch_on_adopts_what_was_shown(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(tmp.path(), Some(vec![server("docs", "npx")]), cx);
        click(&opened, "setup-continue", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Step(SetupStep::Look)));
        let catalog = Catalog::load(tmp.path()).unwrap();
        assert!(catalog.managed);
        assert_eq!(catalog.entries.len(), 1);
        assert!(catalog.entries[0].enabled && catalog.entries[0].approved.is_some());
    }

    #[gpui::test]
    fn next_with_the_switch_off_leaves_the_catalogue_alone(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(tmp.path(), Some(vec![server("docs", "npx")]), cx);
        click(&opened, "setup-tools-switch", cx);
        click(&opened, "setup-continue", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Step(SetupStep::Look)));
        assert!(!catalogue(tmp.path()).exists());
    }

    #[gpui::test]
    fn nothing_found_shows_the_empty_line_and_next_still_turns_hub_only_on(
        cx: &mut TestAppContext,
    ) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(tmp.path(), Some(vec![]), cx);
        assert!(has(&opened, "setup-tools-empty", cx));
        assert!(!has(&opened, "setup-tools-row-0", cx));
        click(&opened, "setup-continue", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Step(SetupStep::Look)));
        assert_eq!(
            Catalog::load(tmp.path()).unwrap(),
            Catalog {
                managed: true,
                ..Catalog::default()
            }
        );
    }

    #[gpui::test]
    fn each_found_server_gets_a_row(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(
            tmp.path(),
            Some(vec![server("docs", "npx"), server("files", "uvx")]),
            cx,
        );
        assert!(has(&opened, "setup-tools-row-0", cx));
        assert!(has(&opened, "setup-tools-row-1", cx));
        assert!(!has(&opened, "setup-tools-row-2", cx));
        assert!(!has(&opened, "setup-tools-empty", cx));
    }

    #[gpui::test]
    fn next_waits_for_discovery(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(tmp.path(), None, cx);
        click(&opened, "setup-continue", cx);
        assert_eq!(opened.card.read_with(cx, |c, _| c.step()), SetupStep::Tools);
        assert!(!catalogue(tmp.path()).exists());
    }

    #[gpui::test]
    fn a_failed_adopt_shows_the_error_and_stays(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        // A folder that can't be made: its parent is a file.
        let blocker = tmp.path().join("file");
        std::fs::write(&blocker, "").unwrap();
        let opened = opened(&blocker.join("data"), Some(vec![]), cx);
        click(&opened, "setup-continue", cx);
        assert_eq!(opened.card.read_with(cx, |c, _| c.step()), SetupStep::Tools);
        assert!(has(&opened, "setup-tools-error", cx));
        // Turning the switch off still gets the user through.
        click(&opened, "setup-tools-switch", cx);
        click(&opened, "setup-continue", cx);
        assert_eq!(last(&opened), Some(SetupEvent::Step(SetupStep::Look)));
    }

    #[gpui::test]
    fn servers_that_cannot_move_are_named(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(tmp.path(), None, cx);
        opened.card.update(cx, |card, cx| {
            card.set_survey(
                Survey {
                    movable: vec![server("docs", "npx")],
                    left_behind: vec![LeftBehind {
                        provider: "claude".into(),
                        name: "github".into(),
                    }],
                },
                cx,
            )
        });
        assert!(has(&opened, "setup-tools-row-0", cx));
        assert!(has(&opened, "setup-tools-left-0", cx));
        assert!(!has(&opened, "setup-tools-empty", cx));
    }

    #[gpui::test]
    fn project_servers_are_called_out(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(tmp.path(), Some(vec![]), cx);
        assert!(has(&opened, "setup-tools-project-note", cx));
    }

    #[gpui::test]
    fn a_server_its_agent_had_off_is_marked_and_stays_off(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let off = Importable {
            enabled: false,
            ..server("old", "uvx")
        };
        let opened = opened(tmp.path(), Some(vec![server("docs", "npx"), off]), cx);
        assert!(!has(&opened, "setup-tools-off-0", cx));
        assert!(has(&opened, "setup-tools-off-1", cx));
        click(&opened, "setup-continue", cx);
        let catalog = Catalog::load(tmp.path()).unwrap();
        let old = catalog.entries.iter().find(|e| e.name == "old").unwrap();
        assert!(!old.enabled && old.approved.is_none());
        let docs = catalog.entries.iter().find(|e| e.name == "docs").unwrap();
        assert!(docs.enabled && docs.approved.is_some());
    }

    #[gpui::test]
    fn leaving_while_servers_move_is_ignored(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(tmp.path(), Some(vec![server("docs", "npx")]), cx);
        opened.card.update(cx, |card, cx| {
            card.tools_next(cx);
            card.skip_setup(cx);
            card.skip(SetupStep::Look, cx);
            card.toggle_tools(cx);
        });
        cx.run_until_parked();
        opened.card.read_with(cx, |card, _| {
            assert_eq!(card.step(), SetupStep::Look);
            assert!(!card.skipped.contains(&SetupStep::Tools));
            assert!(card.tools.on);
        });
        assert!(Catalog::load(tmp.path()).unwrap().managed);
    }

    #[gpui::test]
    fn a_result_after_the_user_left_does_not_pull_them_back(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let opened = opened(tmp.path(), Some(vec![]), cx);
        opened.card.update(cx, |card, cx| {
            card.tools_next(cx);
            // Somewhere else moved the card on before the save finished.
            card.step = SetupStep::Project;
        });
        cx.run_until_parked();
        assert_eq!(
            opened.card.read_with(cx, |c, _| c.step()),
            SetupStep::Project
        );
    }

    #[gpui::test]
    fn a_row_names_the_variables_its_server_gets(cx: &mut TestAppContext) {
        let tmp = tempfile::tempdir().unwrap();
        let mut with_token = server("github", "npx");
        with_token
            .env
            .insert("GITHUB_TOKEN".into(), "secret".into());
        let opened = opened(
            tmp.path(),
            Some(vec![server("docs", "uvx"), with_token]),
            cx,
        );
        assert!(!has(&opened, "setup-tools-env-0", cx));
        assert!(has(&opened, "setup-tools-env-1", cx));
    }
}
