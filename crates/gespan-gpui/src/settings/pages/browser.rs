//! Settings → Browser, ported from upstream's page: the switch, thread
//! access and the folders agents may upload from. Agents always use
//! Gespan's built-in browser (its own kept profile, headless, picked from
//! the installed Chromium browsers), so the page offers no browser choice.
//! This build offers "Always on" access only; the Allow bar that "Ask"
//! needs comes later, with the threads-and-tabs list. The live pane beside
//! the thread ships now and is not configured here.
use crate::{
    settings::{
        SettingsView,
        widgets::{glass_secondary, note, red},
    },
    theme::*,
};
use gespan_core::browser::{AccessMode, BrowserSettings, BrowserState, view};
use gpui::{AnyElement, Context, SharedString, div, prelude::*, px};

/// The group's description: what the browser is and where it shows.
pub(super) const DESCRIPTION: &str = "Agents open their own tabs in Gespan's built-in browser, shown beside the thread. It keeps its own profile in Gespan's data folder, so sites you sign in to there stay signed in. Links you click in a thread open here too; with the switch on, agents use the same sign-ins and can work in the tabs you opened from that thread.";

/// The switch's description.
pub(super) const SWITCH_HELP: &str = "Gespan remembers this switch, and nothing starts until an agent needs the browser. Turning it off stops every agent action and closes the agents' tabs; pages you opened from links stay open.";

/// Thread access's description.
pub(super) const ACCESS_HELP: &str = "While the switch is on, a thread or Inbox Ask can use the browser for the messages you type in it, including those you send from your phone. Pilot chats and their habit runs, turns an agent or automation starts, orchestration and remote threads never can.";

/// Thread access this build offers. "Ask in the thread" joins it with the
/// Allow bar; until then a turn whose access would be Ask gets no browser.
// TODO(phase 2): offer Ask again and stop forcing Always here and in
// `save_browser`. Decide then how to treat a stored Always that phase 1
// wrote on its own (every change and the switch save it), which the user
// never chose.
pub(super) const ACCESS: [(AccessMode, &str, &str); 1] =
    [(AccessMode::Always, "always", "Always on")];

/// What went wrong with the browser in one line, when something did: no
/// Chromium browser to start, a start that failed, an old browser or a
/// policy. Off, ready, starting, running and closed are no news.
pub(super) fn problem_line(state: &BrowserState) -> Option<String> {
    if !view::connection_problem(state) {
        return None;
    }
    let message = view::connection_view(state).message;
    Some(match view::connection_detail(state) {
        Some(detail) if detail != message => format!("{message} {detail}"),
        _ => message,
    })
}

/// The problem line's button: Retry, when trying again can help (a browser
/// installed since, a start that failed once), as the connection offers it.
fn problem_actions(state: &BrowserState, cx: &mut Context<SettingsView>) -> Vec<AnyElement> {
    if view::connection_view(state).action != Some(view::Action::Retry) {
        return Vec::new();
    }
    vec![
        glass_secondary("browser-retry", "Retry", false, false)
            .debug_selector(|| "settings-browser-retry".into())
            .child("Retry")
            .on_click(cx.listener(|_, _, _, cx| {
                crate::browser::command(cx, |host| {
                    host.connect();
                    Ok(())
                })
            }))
            .into_any_element(),
    ]
}

/// The switch's label and its Beta badge.
/// Agents can use the browser right now: the switch is on and it is
/// connected. Starting up, idle and every problem state say nothing.
fn ready(state: &BrowserState) -> bool {
    state.toggle_on && state.connection.state == gespan_core::browser::ConnectionState::Connected
}

/// The switch's label: a 40pt globe tile with the title (and `ready`'s
/// pill, shown only while the browser is connected) and its help beside it.
fn switch_label(ready: bool) -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap(px(14.))
        .child(
            div()
                .debug_selector(|| "settings-browser-tile".into())
                .size(px(40.))
                .flex_none()
                .flex()
                .items_center()
                .justify_center()
                .rounded(px(11.))
                .bg(content(0.07))
                .border_1()
                .border_color(crate::glass::edge())
                .child(icon("globe", 18.).text_color(content(0.8))),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child("Let agents use the browser")
                        .when(ready, |d| {
                            d.child(
                                crate::glass::status("Ready", crate::glass::Tone::Ok)
                                    .debug_selector(|| "settings-browser-ready".into()),
                            )
                        })
                        .child(
                            crate::glass::beta("Beta")
                                .debug_selector(|| "settings-browser-beta".into()),
                        ),
                )
                .child(
                    div()
                        .mt(px(2.))
                        .text_size(px(12.))
                        .line_height(px(18.))
                        .font_weight(gpui::FontWeight::NORMAL)
                        .text_color(content(0.62))
                        .child(SWITCH_HELP),
                ),
        )
        .into_any_element()
}

impl SettingsView {
    pub(super) fn browser_page(&mut self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(state) = crate::browser::state(cx) else {
            return vec![self.group(
                Some("browser"),
                true,
                "Browser access",
                None,
                None,
                vec![note(
                    "Browser access could not start in this window. Quit and reopen Gespan to try again.",
                )],
            )];
        };
        let error = crate::browser::get(cx).and_then(|browser| browser.read(cx).error.clone());
        let mut page = Vec::new();
        if let Some(error) = error {
            page.push(
                div()
                    .debug_selector(|| "settings-browser-error".into())
                    .mb_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(red(0.3))
                    .bg(red(0.05))
                    .px_3()
                    .py_2()
                    .text_size(px(12.))
                    .text_color(red(1.))
                    .child(error)
                    .into_any_element(),
            );
        }
        page.push(self.browser_access_group(&state, cx));
        page.push(self.browser_uploads_group(&state, cx));
        page
    }

    /// Save a change to the browser's settings. The change applies to what
    /// the host holds when the command runs, so quick changes in a row all
    /// stick. Access stays "Always on", the only choice this build offers,
    /// whatever a later build stored.
    fn save_browser(
        &mut self,
        cx: &mut Context<Self>,
        change: impl FnOnce(&mut BrowserSettings) + Send + 'static,
    ) {
        crate::browser::command(cx, move |host| {
            let mut settings = host.state().settings;
            change(&mut settings);
            // TODO(phase 2): see `ACCESS`; this overwrites whatever access
            // was stored, so phase 2 must not take a stored Always as the
            // user's choice.
            settings.access = AccessMode::Always;
            host.set_settings(settings);
            Ok(())
        });
    }

    fn browser_access_group(&mut self, state: &BrowserState, cx: &mut Context<Self>) -> AnyElement {
        let mut rows = vec![self.row(
            Some("browser-toggle"),
            true,
            switch_label(ready(state)),
            None,
            vec![self.toggle(
                "browser-toggle",
                "Let agents use the browser",
                state.toggle_on,
                false,
                cx,
                |_, on, cx| {
                    crate::browser::command(cx, move |host| {
                        let settings = host.state().settings;
                        if on && settings.access != AccessMode::Always {
                            host.set_settings(BrowserSettings {
                                access: AccessMode::Always,
                                ..settings
                            });
                        }
                        host.set_toggle(on);
                        Ok(())
                    })
                },
            )],
        )];
        if let Some(problem) = problem_line(state) {
            rows.push(self.row(
                Some("browser-connection"),
                false,
                "Browser",
                Some(problem.into()),
                problem_actions(state, cx),
            ));
        }
        rows.push(self.row(
            Some("browser-access"),
            false,
            "Thread access",
            Some(ACCESS_HELP.into()),
            vec![self.segmented(
                "browser-access",
                "Thread access",
                "always",
                &ACCESS.map(|(_, value, label)| (value, label)),
                cx,
                |this, _, cx| this.save_browser(cx, move |_| {}),
            )],
        ));
        self.group(
            Some("browser"),
            true,
            "Browser access",
            Some(DESCRIPTION.into()),
            None,
            rows,
        )
    }

    fn pick_upload_roots(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some("Folders agents may upload from".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let _ = this.update(cx, |this, cx| {
                this.save_browser(cx, move |settings| {
                    for path in paths {
                        let path = path.to_string_lossy().into_owned();
                        if !settings.upload_roots.contains(&path) {
                            settings.upload_roots.push(path);
                        }
                    }
                })
            });
        })
        .detach();
    }

    fn browser_uploads_group(
        &mut self,
        state: &BrowserState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let settings = &state.settings;
        let mut rows = vec![self.row(
            Some("browser-uploads"),
            true,
            "Upload folders",
            Some("Agents can upload files from the thread's project folder and from these folders.".into()),
            vec![crate::settings::widgets::icon_button_labelled(
                "browser-add-roots",
                "plus",
                "Add folder…",
                crate::glass::Button::Glass,
            )
                .role(gpui::Role::Button)
                .aria_label("Add folder…")
                .h(px(30.))
                .on_click(cx.listener(|this, _, _, cx| this.pick_upload_roots(cx)))
                .into_any_element()],
        )];
        if !settings.upload_roots.is_empty() {
            rows.push(
                div()
                    .flex()
                    .flex_col()
                    .children(settings.upload_roots.iter().map(|root| {
                        let remove = root.clone();
                        div()
                            .px_4()
                            .py(px(9.))
                            .border_t_1()
                            .border_color(crate::glass::hairline())
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(icon("folder", 14.).text_color(content(0.55)))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .font_family(crate::fonts::monospace())
                                    .text_size(px(11.))
                                    .text_color(content(0.65))
                                    .child(root.clone()),
                            )
                            .child(
                                glass_secondary(
                                    SharedString::from(format!("browser-remove-root-{root}")),
                                    format!("Remove {root}"),
                                    false,
                                    false,
                                )
                                .child(icon("x", 12.))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        let remove = remove.clone();
                                        this.save_browser(cx, move |settings| {
                                            settings.upload_roots.retain(|entry| *entry != remove)
                                        });
                                    },
                                )),
                            )
                    }))
                    .into_any_element(),
            );
        }
        rows.push(self.row(
            Some("browser-dotfiles"),
            false,
            "Allow hidden files in uploads",
            Some("Files and folders whose name starts with a dot. SSH, cloud and Docker credentials stay blocked either way.".into()),
            vec![self.toggle(
                "browser-dotfiles",
                "Allow hidden files in uploads",
                settings.allow_dotfiles,
                false,
                cx,
                |this, allow, cx| this.save_browser(cx, move |settings| settings.allow_dotfiles = allow),
            )],
        ));
        self.group(
            Some("browser-uploads"),
            false,
            "Uploads",
            Some("Where an agent may pick files to upload to a page.".into()),
            None,
            rows,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Preferences, Section, SettingsContext, model::GROUPS};
    use gespan_core::browser::{BrowserChoice, BrowserHost, BrowserMode, ConnectionState};
    use gpui::{
        Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions,
        point, size,
    };
    use std::{path::Path, sync::Arc};

    /// A browser module of the test's own on `dir`, as the app would open
    /// it there, with the switch turned off first so each test turns it on
    /// itself. Turning the switch on starts nothing: the in-app browser
    /// starts only when an agent needs it.
    fn host_on(dir: &Path) -> Arc<BrowserHost> {
        let host = gespan_core::browser::open(dir).unwrap();
        host.set_toggle(false);
        host
    }

    #[gpui::test]
    fn a_fresh_install_starts_with_the_browser_on(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let host = gespan_core::browser::open(dir.path()).unwrap();
        let fresh = host.state();
        assert!(fresh.toggle_on, "no saved settings: the switch starts on");
        assert_eq!(fresh.settings.access, AccessMode::Always);
        assert_eq!(
            fresh.connection.state,
            ConnectionState::Idle,
            "nothing starts yet"
        );
        let (_, mut visual) = page(host.clone(), cx);
        assert!(state(&mut visual).toggle_on);

        // Turning it off is a saved choice the next launch keeps.
        click(&mut visual, "settings-toggle-browser-toggle");
        assert!(!host.state().toggle_on);
        let restarted = gespan_core::browser::open(dir.path()).unwrap();
        assert!(!restarted.state().toggle_on);
        assert_eq!(restarted.connection().state, ConnectionState::Off);
    }

    /// What Settings used to offer for the browser, all gone: the mode
    /// picker, the warning and how-to of "Your own browser", its Connect
    /// button, which browser to start or connect to, and the window switch.
    const REMOVED: [&str; 7] = [
        "settings-row-browser-mode",
        "settings-select-browser-mode",
        "settings-browser-warning",
        "settings-browser-attach-steps",
        "settings-browser-connect",
        "settings-row-browser-choice",
        "settings-row-browser-show-window",
    ];

    fn page(
        host: Arc<BrowserHost>,
        cx: &mut TestAppContext,
    ) -> (WindowHandle<SettingsView>, VisualTestContext) {
        let window = cx.update(|cx| {
            crate::browser::install_host(host, cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1200.), px(1600.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let mut view = SettingsView::new(
                            Section::Browser,
                            Preferences::default(),
                            SettingsContext::default(),
                            cx,
                        );
                        view.discovery_task = None;
                        view.discovery_loading = false;
                        view
                    })
                },
            )
            .unwrap()
        });
        let mut visual = VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        visual.refresh().unwrap();
        (window, visual)
    }

    /// The new layout, rounded (`false`) or boxy (`true`).
    fn set_layout(window: WindowHandle<SettingsView>, visual: &mut VisualTestContext, boxy: bool) {
        window
            .update(visual, |view, _, cx| {
                view.ctx.new_layout_boxy = boxy;
                cx.notify();
            })
            .unwrap();
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is shown"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    fn shown(visual: &mut VisualTestContext, selector: &'static str) -> bool {
        visual.debug_bounds(selector).is_some()
    }

    fn state(visual: &mut VisualTestContext) -> gespan_core::browser::BrowserState {
        visual.update(|_, cx| crate::browser::state(cx).unwrap())
    }

    /// What stays on the page, and nothing that was removed.
    fn assert_only_what_stays(visual: &mut VisualTestContext, layout: &str) {
        for kept in [
            "settings-toggle-browser-toggle",
            "settings-browser-beta",
            "settings-row-browser-access",
            "settings-browser-access-always",
            "settings-row-browser-uploads",
            "settings-row-browser-dotfiles",
        ] {
            assert!(shown(visual, kept), "{layout}: {kept} is shown");
        }
        for gone in REMOVED {
            assert!(!shown(visual, gone), "{layout}: {gone} is gone");
        }
    }

    #[gpui::test]
    fn browser_section_listed_under_agents(cx: &mut TestAppContext) {
        let (group, sections) = GROUPS.iter().find(|(group, _)| *group == "Agents").unwrap();
        assert_eq!(*group, "Agents");
        let mcp = sections.iter().position(|s| *s == Section::Mcp).unwrap();
        assert_eq!(sections.get(mcp + 1), Some(&Section::Browser));
        assert_eq!(
            GROUPS
                .iter()
                .flat_map(|(_, s)| s.iter())
                .filter(|s| **s == Section::Browser)
                .count(),
            1,
            "in one group only"
        );
        assert_eq!(Section::Browser.label(), "Browser");
        assert!(
            crate::settings::model::search("chrome", "")
                .iter()
                .any(|r| r.section == Section::Browser)
        );
        // Opening the section draws its page.
        let dir = tempfile::tempdir().unwrap();
        let (_, mut visual) = page(host_on(dir.path()), cx);
        assert!(shown(&mut visual, "settings-toggle-browser-toggle"));
    }

    /// The page offers the switch, thread access and uploads, and no choice
    /// of browser, in both layouts, with the switch off and on.
    #[gpui::test]
    fn the_page_offers_no_browser_choice_in_either_layout(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let host = host_on(dir.path());
        let (window, mut visual) = page(host.clone(), cx);
        let settings = state(&mut visual).settings;
        assert!(!state(&mut visual).toggle_on);
        assert_eq!(settings.mode, BrowserMode::Managed);
        assert!(!settings.show_window);
        for boxy in [false, true] {
            let layout = if boxy { "boxy" } else { "rounded" };
            set_layout(window, &mut visual, boxy);
            assert_only_what_stays(&mut visual, layout);
            // Nothing is wrong, so no connection line.
            assert!(
                !shown(&mut visual, "settings-row-browser-connection"),
                "{layout}"
            );
        }
        // On (the old mode showed its warning here): still nothing else.
        click(&mut visual, "settings-toggle-browser-toggle");
        assert!(host.state().toggle_on, "the switch reached the host");
        assert_eq!(host.state().settings.access, AccessMode::Always);
        for boxy in [false, true] {
            let layout = if boxy { "boxy" } else { "rounded" };
            set_layout(window, &mut visual, boxy);
            assert_only_what_stays(&mut visual, layout);
            assert!(
                !shown(&mut visual, "settings-row-browser-connection"),
                "{layout}"
            );
        }
        host.set_toggle(false);
    }

    /// No browser can start: the page says why in one line, in both
    /// layouts, and the line goes once the switch is off.
    #[gpui::test]
    fn a_failed_start_is_the_only_connection_line(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let host = host_on(dir.path());
        host.set_executable_for_tests(dir.path().join("no-such-browser"));
        let (window, mut visual) = page(host.clone(), cx);
        click(&mut visual, "settings-toggle-browser-toggle");
        host.connect();
        settle(&mut visual, &host, ConnectionState::StalePort);
        let line = problem_line(&state(&mut visual)).expect("a problem");
        assert!(
            line.starts_with("The Gespan browser could not start."),
            "{line}"
        );
        for boxy in [false, true] {
            set_layout(window, &mut visual, boxy);
            assert!(
                shown(&mut visual, "settings-row-browser-connection"),
                "boxy {boxy}"
            );
            assert!(
                !shown(&mut visual, "settings-browser-connect"),
                "boxy {boxy}"
            );
        }
        click(&mut visual, "settings-toggle-browser-toggle");
        assert!(!host.state().toggle_on);
        assert!(!shown(&mut visual, "settings-row-browser-connection"));
    }

    /// The line only for real errors, with the no-browser case worded for
    /// the user.
    #[gpui::test]
    fn the_problem_line_is_only_for_real_errors(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (_, mut visual) = page(host_on(dir.path()), cx);
        let mut state = state(&mut visual);
        for quiet in [
            ConnectionState::Off,
            ConnectionState::Idle,
            ConnectionState::Discovering,
            ConnectionState::Connected,
            ConnectionState::Disconnected,
        ] {
            state.connection.state = quiet;
            assert_eq!(problem_line(&state), None, "{quiet:?}");
        }
        state.connection.state = ConnectionState::StalePort;
        state.connection.detail = Some(gespan_browser::NO_BROWSER.into());
        state.connection.last_error = Some(gespan_browser::NO_BROWSER.into());
        assert_eq!(
            problem_line(&state).as_deref(),
            Some(
                "The Gespan browser could not start. No Chromium-based browser is installed on \
                 this computer. Install Chrome, Edge, Brave, Vivaldi, Opera or Arc, then try again."
            )
        );
    }

    /// Ready is claimed only while the browser is actually connected.
    #[gpui::test]
    fn ready_shows_only_while_connected(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (_, mut visual) = page(host_on(dir.path()), cx);
        let mut state = state(&mut visual);
        state.toggle_on = true;
        for other in [
            ConnectionState::Off,
            ConnectionState::Idle,
            ConnectionState::Discovering,
            ConnectionState::Disconnected,
            ConnectionState::StalePort,
        ] {
            state.connection.state = other;
            assert!(!ready(&state), "{other:?}");
        }
        state.connection.state = ConnectionState::Connected;
        assert!(ready(&state));
        state.toggle_on = false;
        assert!(!ready(&state), "switched off");
    }

    /// What the page says describes only the in-app browser.
    #[test]
    fn the_page_text_describes_only_the_in_app_browser() {
        assert!(DESCRIPTION.contains("built-in browser"), "{DESCRIPTION}");
        assert!(DESCRIPTION.contains("beside the thread"), "{DESCRIPTION}");
        // Off closes the agents' tabs (and the browser when none of the
        // user's is left): the old "leaves the agents' tabs open" is wrong.
        let switch = SWITCH_HELP.to_lowercase();
        assert!(!switch.contains("leaves the agents"), "{SWITCH_HELP}");
        assert!(!switch.contains("tabs open"), "{SWITCH_HELP}");
        assert!(switch.contains("closes the agents' tabs"), "{SWITCH_HELP}");
        assert!(
            switch.contains("pages you opened from links stay open"),
            "{SWITCH_HELP}"
        );
        for text in [DESCRIPTION, SWITCH_HELP, ACCESS_HELP] {
            let lower = text.to_lowercase();
            for gone in [
                "your own browser",
                "chrome://inspect",
                "remote debugging",
                "private",
                "show browser window",
            ] {
                assert!(!lower.contains(gone), "{gone:?} in {text}");
            }
        }
    }

    /// Thread access says which turns carry the browser as the turns are
    /// marked: typed sends (the phone's too) do, a Pilot chat's never do
    /// (its messages launch untyped), whatever the thread.
    #[test]
    fn access_help_names_the_turns_that_carry_the_browser() {
        let text = ACCESS_HELP.to_lowercase();
        assert!(!text.contains("every thread"), "{ACCESS_HELP}");
        assert!(text.contains("you type"), "{ACCESS_HELP}");
        assert!(text.contains("phone"), "{ACCESS_HELP}");
        assert!(text.contains("pilot chats"), "{ACCESS_HELP}");
    }

    /// The page tells the user that their link tabs share the browser with
    /// the agents: the same sign-ins, and the tabs of the thread.
    #[test]
    fn the_page_says_agents_share_the_users_sign_ins_and_tabs() {
        let text = DESCRIPTION.to_lowercase();
        assert!(
            text.contains("links you click in a thread open here"),
            "{text}"
        );
        assert!(text.contains("with the switch on"), "{text}");
        assert!(text.contains("same sign-ins"), "{text}");
        assert!(
            text.contains("can work in the tabs you opened from that thread"),
            "{text}"
        );
    }

    /// The problem line offers Retry, in both layouts, and Retry tries to
    /// start the browser again: one installed since then starts.
    #[gpui::test]
    fn retry_on_the_problem_line_starts_a_browser_installed_since(cx: &mut TestAppContext) {
        let rig = crate::browser_test_rig::Rig::start_managed();
        let host = rig.host.clone();
        let empty = tempfile::tempdir().unwrap();
        host.set_profile_dir(empty.path().to_path_buf());
        host.set_executable_for_tests(empty.path().join("no-such-browser"));
        let (window, mut visual) = page(host.clone(), cx);
        click(&mut visual, "settings-toggle-browser-toggle");
        host.connect();
        settle(&mut visual, &host, ConnectionState::StalePort);
        for boxy in [false, true] {
            set_layout(window, &mut visual, boxy);
            assert!(shown(&mut visual, "settings-browser-retry"), "boxy {boxy}");
        }
        // A browser is there now (the fake one, on the kept profile).
        host.set_profile_dir(rig.server.profile_dir());
        click(&mut visual, "settings-browser-retry");
        settle(&mut visual, &host, ConnectionState::Connected);
        assert!(!shown(&mut visual, "settings-row-browser-connection"));
        assert!(!shown(&mut visual, "settings-browser-retry"));
        host.set_toggle(false);
    }

    /// Wait for the host's connection thread, then show what it left.
    fn settle(visual: &mut VisualTestContext, host: &BrowserHost, want: ConnectionState) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while host.connection().state != want {
            assert!(
                std::time::Instant::now() < deadline,
                "{:?}",
                host.connection()
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        visual.update(|_, cx| crate::browser::refresh(cx));
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    #[gpui::test]
    fn ask_is_not_offered_yet(cx: &mut TestAppContext) {
        assert_eq!(ACCESS, [(AccessMode::Always, "always", "Always on")]);
        let dir = tempfile::tempdir().unwrap();
        let host = host_on(dir.path());
        assert_eq!(
            host.state().settings.access,
            AccessMode::Always,
            "a fresh install allows every regular thread"
        );
        let (_, mut visual) = page(host.clone(), cx);
        assert!(shown(&mut visual, "settings-row-browser-access"));
        assert!(shown(&mut visual, "settings-browser-access-always"));
        assert!(!shown(&mut visual, "settings-browser-access-ask"));
        // Whatever a later build stored, any change here keeps Always on.
        click(&mut visual, "settings-toggle-browser-dotfiles");
        assert!(host.state().settings.allow_dotfiles);
        assert_eq!(host.state().settings.access, AccessMode::Always);
    }

    #[gpui::test]
    fn quick_changes_in_a_row_all_stick(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let host = host_on(dir.path());
        let (window, mut visual) = page(host.clone(), cx);
        // Two changes before either command has run: the second must not
        // save over the first with what the page showed.
        window
            .update(&mut visual, |view, _, cx| {
                view.save_browser(cx, |s| s.allow_dotfiles = true);
                view.save_browser(cx, |s| s.upload_roots.push("/work".into()));
            })
            .unwrap();
        visual.run_until_parked();
        let settings = host.state().settings;
        assert!(settings.allow_dotfiles);
        assert_eq!(settings.upload_roots, vec!["/work".to_string()]);
    }

    /// A save from this page never brings back a browser choice: whatever
    /// it carries, the host keeps the in-app browser.
    #[gpui::test]
    fn a_save_keeps_the_in_app_browser(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let host = host_on(dir.path());
        let (window, mut visual) = page(host.clone(), cx);
        window
            .update(&mut visual, |view, _, cx| {
                view.save_browser(cx, |s| {
                    s.mode = BrowserMode::Attach;
                    s.show_window = true;
                    s.browser = BrowserChoice::Channel {
                        channel: "brave".into(),
                    };
                    s.allow_dotfiles = true;
                });
            })
            .unwrap();
        visual.run_until_parked();
        let settings = host.state().settings;
        assert_eq!(settings.mode, BrowserMode::Managed);
        assert!(!settings.show_window);
        assert_eq!(settings.browser, BrowserChoice::Auto);
        assert!(settings.allow_dotfiles);
    }

    #[gpui::test]
    fn settings_survive_a_restart(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let host = host_on(dir.path());
        let (_, mut visual) = page(host.clone(), cx);
        click(&mut visual, "settings-toggle-browser-toggle");
        click(&mut visual, "settings-toggle-browser-dotfiles");
        let before = host.state();
        assert!(before.toggle_on);
        assert!(before.settings.allow_dotfiles);

        // The next launch opens the same data directory.
        let restarted = gespan_core::browser::open(dir.path()).unwrap();
        let after = restarted.state();
        assert!(after.toggle_on);
        assert!(after.settings.allow_dotfiles);
        assert_eq!(after.settings.mode, BrowserMode::Managed);
        assert_eq!(after.settings.access, AccessMode::Always);
        assert!(!after.settings.show_window);
        assert_eq!(after.connection.state, ConnectionState::Idle);

        let (_, mut visual) = page(restarted, cx);
        let shown_state = state(&mut visual);
        assert!(shown_state.toggle_on);
        assert_only_what_stays(&mut visual, "restarted");
        host.set_toggle(false);
    }
}
