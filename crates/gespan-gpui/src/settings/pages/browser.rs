//! Settings → Browser, ported from upstream's page: the switch, which
//! browser agents use and how, and the folders they may upload from.
//! Phase 1 offers "Always on" access only; the Allow bar that "Ask" needs
//! comes later, with the threads-and-tabs list and the live pane.
use crate::{
    settings::{
        SettingsView,
        widgets::{note, red, secondary_button},
    },
    theme::*,
};
use gespan_core::browser::{
    AccessMode, BrowserChoice, BrowserKind, BrowserMode, BrowserSettings, BrowserState, view,
};
use gpui::{AnyElement, Context, FontWeight, Hsla, SharedString, div, prelude::*, px, rgb};

/// Shown whenever "Your own browser" is the mode, so also when the switch
/// goes on in it.
pub(super) const WARNING: &str = "Agents will use your own browser with your signed-in accounts. A web page can try to trick an agent; only turn this on for work you'd trust it with.";

/// The modes in the picker's order: the default first.
pub(super) const MODES: [(BrowserMode, &str, &str); 3] = [
    (BrowserMode::Attach, "attach", "Your own browser"),
    (BrowserMode::Managed, "managed", "Gespan browser"),
    (BrowserMode::Private, "private", "Private"),
];

/// Thread access this build offers. "Ask in the thread" joins it with the
/// Allow bar; until then a turn whose access would be Ask gets no browser.
pub(super) const ACCESS: [(AccessMode, &str, &str); 1] =
    [(AccessMode::Always, "always", "Always on")];

/// Tailwind's amber-400.
const AMBER: u32 = 0xfbbf24;

fn amber(alpha: f32) -> Hsla {
    let mut color: Hsla = rgb(AMBER).into();
    color.a = alpha;
    color
}

pub(super) fn mode_value(mode: BrowserMode) -> &'static str {
    MODES
        .iter()
        .find(|(candidate, _, _)| *candidate == mode)
        .map(|(_, value, _)| *value)
        .unwrap_or("attach")
}

fn mode_from(value: &str) -> BrowserMode {
    MODES
        .iter()
        .find(|(_, candidate, _)| *candidate == value)
        .map(|(mode, _, _)| *mode)
        .unwrap_or_default()
}

/// Where the user turns remote debugging on for the found or chosen browser.
fn inspect_pages(state: &BrowserState) -> Vec<&'static str> {
    const CHROME: &str = "chrome://inspect/#remote-debugging";
    const EDGE: &str = "edge://inspect";
    const BRAVE: &str = "brave://inspect/#remote-debugging";
    let page = |kind: BrowserKind| match kind {
        BrowserKind::Edge => EDGE,
        BrowserKind::Brave => BRAVE,
        _ => CHROME,
    };
    if let Some(browser) = &state.connection.browser {
        return vec![page(browser.kind)];
    }
    match &state.settings.browser {
        BrowserChoice::Channel { channel } => vec![page(view::channel_kind(channel))],
        _ => vec![CHROME, EDGE],
    }
}

fn code_chip(text: &'static str) -> AnyElement {
    div()
        .rounded(px(4.))
        .bg(content(0.08))
        .px(px(6.))
        .py(px(2.))
        .font_family(crate::fonts::monospace())
        .text_size(px(11.))
        .text_color(content(1.))
        .child(text)
        .into_any_element()
}

/// A numbered list of steps under a group's rows, then notes (`true`:
/// muted).
fn steps(
    selector: &'static str,
    intro: &'static str,
    items: Vec<AnyElement>,
    notes: Vec<(&'static str, bool)>,
) -> AnyElement {
    div()
        .debug_selector(move || selector.into())
        .border_t_1()
        .border_color(content(0.05))
        .px_4()
        .py(px(14.))
        .text_size(px(12.))
        .line_height(px(19.5))
        .text_color(content(0.65))
        .child(div().mb_2().text_color(content(0.85)).child(intro))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .children(items.into_iter().enumerate().map(|(ix, item)| {
                    div()
                        .flex()
                        .gap_2()
                        .child(div().flex_shrink_0().child(format!("{}.", ix + 1)))
                        .child(div().min_w_0().flex_1().child(item))
                })),
        )
        .children(notes.into_iter().map(|(text, muted)| {
            div()
                .mt_2()
                .when(muted, |d| d.text_color(content(0.45)))
                .child(text)
        }))
        .into_any_element()
}

/// "Turn on remote debugging once in chrome://inspect or edge://inspect."
fn inspect_step(state: &BrowserState) -> AnyElement {
    let mut line = div()
        .flex()
        .flex_wrap()
        .items_center()
        .gap_1()
        .child("Turn on remote debugging once in");
    for (ix, page) in inspect_pages(state).into_iter().enumerate() {
        if ix > 0 {
            line = line.child("or");
        }
        line = line.child(code_chip(page));
    }
    line.child(".").into_any_element()
}

fn text(value: &'static str) -> AnyElement {
    div().child(value).into_any_element()
}

/// The switch's label and its Beta badge.
fn switch_label() -> AnyElement {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child("Let agents use the browser")
        .child(
            div()
                .debug_selector(|| "settings-browser-beta".into())
                .rounded(px(4.))
                .border_1()
                .border_color(accent_color())
                .px(px(5.))
                .text_size(px(10.))
                .line_height(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(accent_color())
                .child("Beta"),
        )
        .into_any_element()
}

fn warning() -> AnyElement {
    div()
        .debug_selector(|| "settings-browser-warning".into())
        .border_t_1()
        .border_color(content(0.05))
        .px_4()
        .py(px(12.))
        .bg(amber(0.08))
        .flex()
        .items_start()
        .gap_2()
        .child(
            div()
                .flex_shrink_0()
                .pt(px(2.))
                .child(icon("shield-alert", 14.).text_color(amber(1.))),
        )
        .child(
            div()
                .min_w_0()
                .flex_1()
                .text_size(px(12.))
                .line_height(px(19.5))
                .text_color(content(0.85))
                .child(WARNING),
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
            settings.access = AccessMode::Always;
            host.set_settings(settings);
            Ok(())
        });
    }

    fn browser_access_group(&mut self, state: &BrowserState, cx: &mut Context<Self>) -> AnyElement {
        let settings = &state.settings;
        let managed = settings.mode.launches();
        let private = settings.mode == BrowserMode::Private;
        let mut rows = vec![self.row(
            Some("browser-toggle"),
            true,
            switch_label(),
            Some("Gespan remembers this switch, and nothing starts until an agent needs the browser. Turning it off stops every browser action and leaves the agents' tabs open.".into()),
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
        if !managed {
            rows.push(warning());
        }
        rows.push(self.row(
            Some("browser-mode"),
            false,
            "Browser",
            Some(if private {
                "Like the Gespan browser, with a fresh profile every time it starts. Nothing is kept when it closes: no sign-ins, cookies or history."
            } else if managed {
                "Gespan starts an installed browser with its own profile. Nothing to set up, and your own browser does not need to run."
            } else {
                "Gespan connects to the Chrome, Edge, Brave, Vivaldi, Opera or Arc you run, signed in as you."
            }
            .into()),
            vec![self.select(
                "browser-mode",
                "Browser",
                mode_value(settings.mode),
                MODES
                    .iter()
                    .map(|(_, value, label)| ((*value).to_owned(), (*label).into(), None))
                    .collect(),
                None,
                cx,
                |this, mode, cx| {
                    let mode = mode_from(&mode);
                    this.save_browser(cx, move |settings| settings.mode = mode);
                },
            )],
        ));
        if managed {
            rows.push(
                div()
                    .border_t_1()
                    .border_color(content(0.05))
                    .px_4()
                    .py(px(10.))
                    .text_size(px(12.))
                    .line_height(px(19.5))
                    .text_color(content(0.45))
                    .child(if private {
                        "Agents open their own tabs in a private Gespan browser that forgets everything when it closes. Your own browser and its tabs stay out of their reach."
                    } else {
                        "Agents open their own tabs in the Gespan browser, which keeps its own sign-ins. Your own browser and its tabs stay out of their reach."
                    })
                    .into_any_element(),
            );
        } else {
            rows.push(steps(
                "settings-browser-attach-steps",
                "Your own browser needs these steps before agents can use it:",
                vec![
                    text("Use Chrome, Edge, Brave, Vivaldi, Opera or Arc, version 144 or newer."),
                    inspect_step(state),
                    text("Keep the browser running. Gespan never starts it."),
                    text("Choose Allow when the browser asks. It asks again on every new connection, for example after a browser or Gespan restart."),
                    text("While Gespan is connected, the browser shows a bar saying it is controlled by automated software."),
                ],
                vec![
                    ("Agents open tabs of their own, marked ◆ Gespan, and never see your own tabs.", false),
                    ("The Gespan browser needs none of this.", true),
                ],
            ));
        }
        rows.push(self.row(
            Some("browser-access"),
            false,
            "Thread access",
            Some("While the switch is on, every thread you type in can use the browser. Turns an agent or automation starts, Pilot habit runs, orchestration and remote threads never can.".into()),
            vec![self.segmented(
                "browser-access",
                "Thread access",
                "always",
                &ACCESS.map(|(_, value, label)| (value, label)),
                cx,
                |this, _, cx| this.save_browser(cx, move |_| {}),
            )],
        ));
        rows.push(self.browser_choice_row(settings, cx));
        if managed {
            rows.push(
                self.row(
                    Some("browser-show-window"),
                    false,
                    "Show browser window",
                    Some(
                        "Off: the Gespan browser runs without a window. Agents work the same way."
                            .into(),
                    ),
                    vec![self.toggle(
                        "browser-show-window",
                        "Show browser window",
                        settings.show_window,
                        false,
                        cx,
                        |this, show, cx| {
                            this.save_browser(cx, move |settings| settings.show_window = show)
                        },
                    )],
                ),
            );
        }
        self.group(
            Some("browser"),
            true,
            "Browser access",
            Some(if managed {
                "Agents open their own tabs in the Gespan browser: your installed Chrome, Edge, Brave, Vivaldi, Opera or Arc with a profile of its own."
            } else {
                "Agents open their own tabs in your own browser, next to yours and signed in as you. They never see or touch your own tabs."
            }
            .into()),
            None,
            rows,
        )
    }

    /// Which browser the Gespan browser starts, or which one Gespan
    /// connects to.
    fn browser_choice_row(
        &mut self,
        settings: &BrowserSettings,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut options: Vec<(String, SharedString, Option<AnyElement>)> =
            vec![("auto".into(), "Automatic".into(), None)];
        if settings.mode.launches() {
            options.extend(
                view::managed_channels()
                    .map(|(channel, label)| (channel.to_owned(), label.into(), None)),
            );
            return self.row(
                Some("browser-choice"),
                false,
                "Which browser to start",
                Some("Automatic starts Chrome, or else the first of Edge, Brave, Vivaldi, Opera and Arc that is installed.".into()),
                vec![self.select(
                    "browser-choice",
                    "Which browser to start",
                    view::managed_choice(&settings.browser),
                    options,
                    None,
                    cx,
                    |this, channel, cx| {
                        let choice = if channel == "auto" {
                            BrowserChoice::Auto
                        } else {
                            BrowserChoice::Channel { channel }
                        };
                        this.save_browser(cx, move |settings| settings.browser = choice);
                    },
                )],
            );
        }
        let custom = matches!(settings.browser, BrowserChoice::UserDataDir { .. });
        let value = match &settings.browser {
            BrowserChoice::Auto => "auto".to_owned(),
            BrowserChoice::Channel { channel } => channel.clone(),
            BrowserChoice::UserDataDir { path } => path.clone(),
        };
        options.extend(
            view::CHANNELS
                .iter()
                .map(|(channel, label)| ((*channel).to_owned(), (*label).into(), None)),
        );
        if let BrowserChoice::UserDataDir { path } = &settings.browser {
            options.push((path.clone(), format!("Profile {path}").into(), None));
        }
        options.push(("custom".into(), "Profile folder…".into(), None));
        self.row(
            Some("browser-choice"),
            false,
            "Which browser to connect to",
            Some(if custom {
                "Gespan connects to the browser that uses this profile folder."
            } else {
                "Automatic tries Chrome, then Edge, their Beta, Dev and Canary builds, then Brave, Vivaldi, Opera and Arc. Pick a profile folder for Snap, Flatpak or portable installs."
            }
            .into()),
            vec![self.select(
                "browser-choice",
                "Which browser to connect to",
                &value,
                options,
                None,
                cx,
                move |this, choice, cx| match choice.as_str() {
                    "auto" => this.save_browser(cx, move |s| s.browser = BrowserChoice::Auto),
                    "custom" => this.pick_browser_profile(cx),
                    channel if view::CHANNELS.iter().any(|(c, _)| *c == channel) => {
                        let channel = channel.to_owned();
                        this.save_browser(cx, move |s| s.browser = BrowserChoice::Channel { channel })
                    }
                    _ => {}
                },
            )],
        )
    }

    /// "Profile folder…": the user data folder of a Snap, Flatpak or
    /// portable browser.
    fn pick_browser_profile(&mut self, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Choose the browser's user data folder".into()),
        });
        cx.spawn(async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let path = path.to_string_lossy().into_owned();
            let _ = this.update(cx, |this, cx| {
                this.save_browser(cx, move |s| s.browser = BrowserChoice::UserDataDir { path })
            });
        })
        .detach();
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
            vec![secondary_button("browser-add-roots", "Add folder…", false, false)
                .child("Add folder…")
                .on_click(cx.listener(|this, _, _, cx| this.pick_upload_roots(cx)))
                .into_any_element()],
        )];
        if !settings.upload_roots.is_empty() {
            rows.push(
                div()
                    .px_4()
                    .pb_3()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(settings.upload_roots.iter().map(|root| {
                        let remove = root.clone();
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
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
                                secondary_button(
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
    use gespan_core::browser::{BrowserHost, ConnectionState};
    use gpui::{
        Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions,
        point, size,
    };
    use std::{path::Path, sync::Arc};

    /// A browser module of the test's own on `dir`, as the app would open
    /// it there. "Your own browser" looks at an empty profile, so turning
    /// the switch on never reaches a real browser.
    fn host_on(dir: &Path) -> Arc<BrowserHost> {
        let host = gespan_core::browser::open(dir).unwrap();
        let nothing = dir.join("no-browser");
        std::fs::create_dir_all(&nothing).unwrap();
        host.set_settings(BrowserSettings {
            browser: BrowserChoice::UserDataDir {
                path: nothing.display().to_string(),
            },
            ..host.state().settings
        });
        host
    }

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

    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is shown"));
        visual.simulate_click(bounds.center(), Default::default());
        visual.run_until_parked();
        visual.refresh().unwrap();
    }

    /// Open the mode picker and choose its option `ix` (the order of
    /// [`MODES`]).
    fn choose_mode(visual: &mut VisualTestContext, mode: BrowserMode) {
        let ix = MODES.iter().position(|(m, _, _)| *m == mode).unwrap();
        click(visual, "settings-select-browser-mode");
        // Selectors are 'static; a test leaks three short strings at most.
        click(
            visual,
            Box::leak(format!("settings-option-browser-mode-{ix}").into_boxed_str()),
        );
    }

    fn shown(visual: &mut VisualTestContext, selector: &'static str) -> bool {
        visual.debug_bounds(selector).is_some()
    }

    fn state(visual: &mut VisualTestContext) -> gespan_core::browser::BrowserState {
        visual.update(|_, cx| crate::browser::state(cx).unwrap())
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

    #[gpui::test]
    fn switch_starts_off_and_mode_starts_as_your_own_browser(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let (_, mut visual) = page(gespan_core::browser::open(dir.path()).unwrap(), cx);
        let state = state(&mut visual);
        assert!(!state.toggle_on);
        assert_eq!(state.settings.mode, BrowserMode::Attach);
        assert_eq!(MODES[0].0, BrowserMode::Attach, "listed first");
        assert_eq!(MODES[0].2, "Your own browser");
        assert_eq!(mode_value(state.settings.mode), MODES[0].1);
        assert!(shown(&mut visual, "settings-toggle-browser-toggle"));
        assert!(shown(&mut visual, "settings-browser-beta"));
        // What "Your own browser" needs, and nothing of the Gespan browser.
        assert!(shown(&mut visual, "settings-browser-attach-steps"));
        assert!(shown(&mut visual, "settings-row-browser-choice"));
        assert!(!shown(&mut visual, "settings-row-browser-show-window"));
        assert!(shown(&mut visual, "settings-row-browser-uploads"));
        assert!(shown(&mut visual, "settings-row-browser-dotfiles"));
    }

    #[gpui::test]
    fn turning_on_in_own_browser_mode_shows_the_warning(cx: &mut TestAppContext) {
        assert_eq!(
            WARNING,
            "Agents will use your own browser with your signed-in accounts. A web page \
             can try to trick an agent; only turn this on for work you'd trust it with."
        );
        let dir = tempfile::tempdir().unwrap();
        let host = host_on(dir.path());
        let (_, mut visual) = page(host.clone(), cx);
        click(&mut visual, "settings-toggle-browser-toggle");
        assert!(host.state().toggle_on, "the switch reached the host");
        assert!(state(&mut visual).toggle_on, "and the page follows it");
        // Phase 1 has no Allow bar: on means every thread you type in.
        assert_eq!(host.state().settings.access, AccessMode::Always);
        assert!(shown(&mut visual, "settings-browser-warning"));

        // The Gespan browser is not signed in as you: no warning.
        choose_mode(&mut visual, BrowserMode::Managed);
        assert_eq!(host.state().settings.mode, BrowserMode::Managed);
        assert!(!shown(&mut visual, "settings-browser-warning"));
        assert!(!shown(&mut visual, "settings-browser-attach-steps"));
        assert!(shown(&mut visual, "settings-row-browser-show-window"));

        // Choosing your own browser again shows it again.
        choose_mode(&mut visual, BrowserMode::Attach);
        assert_eq!(host.state().settings.mode, BrowserMode::Attach);
        assert!(shown(&mut visual, "settings-browser-warning"));
        assert!(shown(&mut visual, "settings-browser-attach-steps"));
    }

    #[gpui::test]
    fn ask_is_not_offered_yet(cx: &mut TestAppContext) {
        assert_eq!(ACCESS, [(AccessMode::Always, "always", "Always on")]);
        let dir = tempfile::tempdir().unwrap();
        let host = host_on(dir.path());
        assert_eq!(
            host.state().settings.access,
            AccessMode::Ask,
            "stored default"
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
                view.save_browser(cx, |s| s.show_window = false);
            })
            .unwrap();
        visual.run_until_parked();
        let settings = host.state().settings;
        assert!(settings.allow_dotfiles);
        assert!(!settings.show_window);
    }

    #[gpui::test]
    fn settings_survive_a_restart(cx: &mut TestAppContext) {
        let dir = tempfile::tempdir().unwrap();
        let host = host_on(dir.path());
        let (_, mut visual) = page(host.clone(), cx);
        choose_mode(&mut visual, BrowserMode::Private);
        click(&mut visual, "settings-toggle-browser-toggle");
        click(&mut visual, "settings-toggle-browser-show-window");
        let before = host.state();
        assert_eq!(before.settings.mode, BrowserMode::Private);
        assert!(before.toggle_on);
        assert!(!before.settings.show_window);

        // The next launch opens the same data directory.
        let restarted = gespan_core::browser::open(dir.path()).unwrap();
        let after = restarted.state();
        assert!(after.toggle_on);
        assert_eq!(after.settings.mode, BrowserMode::Private);
        assert_eq!(after.settings.access, AccessMode::Always);
        assert!(!after.settings.show_window);
        assert_eq!(after.connection.state, ConnectionState::Idle);

        let (_, mut visual) = page(restarted, cx);
        let shown_state = state(&mut visual);
        assert!(shown_state.toggle_on);
        assert_eq!(shown_state.settings.mode, BrowserMode::Private);
        assert!(!shown(&mut visual, "settings-browser-warning"));
        assert!(shown(&mut visual, "settings-row-browser-show-window"));
    }
}
