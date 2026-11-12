//! The document's ⋯ menu: the pull request's own actions, a review by an
//! agent, and the project's pull request automations to run on it now. A
//! run started here shows as a pill in the document until it finishes.
use super::*;
use crate::glass;
use crate::popover::{Align, Anchor, popover};
use gespan_core::automations::{AutomationRun, DueAutomationRun};
use gpui::{AnyElement, FontWeight, Role, SharedString, div, prelude::*, px, rgb};

/// A pull request automation the ⋯ menu offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ItemAutomation {
    pub id: String,
    pub name: String,
    /// The trigger as its card names it, "When a pull request is opened".
    pub trigger: String,
    /// The agent it runs, "Claude Code".
    pub agent: String,
}

/// A run started from an item's ⋯ menu, as its run record last said.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ItemRun {
    pub automation: String,
    pub name: String,
    pub run: String,
    pub status: String,
    pub started_at: i64,
}

impl ItemRun {
    pub(crate) fn of(due: &DueAutomationRun) -> Self {
        Self {
            automation: due.automation.id.clone(),
            name: due.automation.name.clone(),
            run: due.run.id.clone(),
            status: due.run.status.clone(),
            started_at: due.run.started_at.unwrap_or(due.run.created_at),
        }
    }
}

/// The pill a run shows: the automation's name and how its run stands.
pub(super) fn run_pill(run: &ItemRun, now: i64) -> (String, glass::Tone) {
    let (word, tone) = match run.status.as_str() {
        "pending" => ("queued".to_string(), glass::Tone::Running),
        "running" => {
            let minutes = (now - run.started_at).max(0) / 60_000;
            let word = match minutes {
                0 => "running".to_string(),
                m if m < 60 => format!("running {m}m"),
                m => format!("running {}h", m / 60),
            };
            (word, glass::Tone::Running)
        }
        "succeeded" => ("succeeded".to_string(), glass::Tone::Ok),
        "failed" => ("failed".to_string(), glass::Tone::Bad),
        other => (other.replace('_', " "), glass::Tone::Muted),
    };
    (format!("{} · {word}", run.name), tone)
}

/// One line of the ⋯ menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Entry {
    Review,
    Open(String),
    Copy,
    /// A pull request action, confirmed before it runs: its action name,
    /// label and icon.
    Pr(&'static str, &'static str, &'static str),
    Divider,
    /// "Run an automation on #476".
    Automations(String),
    /// The `n`th offered automation.
    Automation(usize),
    NewAutomation,
}

/// The pull request event a new automation from this item starts with.
pub(crate) fn new_automation_event(provider: &str) -> Option<&'static str> {
    match provider {
        "github" => Some("pull_request_opened"),
        "gitlab" => Some("merge_request_opened"),
        "azuredevops" => Some("pull_request_appeared"),
        _ => None,
    }
}

/// What the ⋯ menu lists for `item`, given `automations` offered for it.
pub(super) fn entries(item: &Item, automations: usize) -> Vec<Entry> {
    let pr = item.kind == "pr";
    let github_pr = pr && item.provider == "github";
    let mut entries = vec![];
    if github_pr {
        entries.push(Entry::Review);
    }
    if !item.url.is_empty() {
        entries.push(Entry::Open(format!(
            "Open on {}",
            model::provider_label(&item.provider)
        )));
        entries.push(Entry::Copy);
    }
    if github_pr {
        let open = item.state.eq_ignore_ascii_case("open");
        let actions = [
            (
                open && !item.draft,
                "merge",
                "Merge pull request",
                "git-merge",
            ),
            (
                open && !item.draft,
                "squash",
                "Squash and merge",
                "git-merge",
            ),
            (
                open && !item.draft,
                "rebase",
                "Rebase and merge",
                "git-merge",
            ),
            (
                open && item.draft,
                "ready",
                "Ready for review",
                "git-pull-request",
            ),
            (
                open && !item.draft,
                "draft",
                "Convert to draft",
                "git-pull-request-draft",
            ),
            (
                open,
                "close",
                "Close pull request",
                "git-pull-request-closed",
            ),
            (
                item.state.eq_ignore_ascii_case("closed"),
                "reopen",
                "Reopen pull request",
                "git-pull-request",
            ),
        ];
        let shown = actions
            .iter()
            .filter(|(show, ..)| *show)
            .collect::<Vec<_>>();
        if !shown.is_empty() {
            entries.push(Entry::Divider);
            entries.extend(
                shown
                    .into_iter()
                    .map(|(_, name, label, icon)| Entry::Pr(name, label, icon)),
            );
        }
    }
    if pr && new_automation_event(&item.provider).is_some() {
        entries.push(Entry::Divider);
        entries.push(Entry::Automations(format!(
            "Run an automation on {}",
            item.reference()
        )));
        entries.extend((0..automations).map(Entry::Automation));
        entries.push(Entry::NewAutomation);
    }
    while entries.first() == Some(&Entry::Divider) {
        entries.remove(0);
    }
    entries
}

impl Inbox {
    /// Opens or closes the ⋯ menu; opening it on a pull request asks the
    /// Shell for the automations it can run.
    pub(super) fn toggle_menu(&mut self, cx: &mut Context<Self>) {
        self.action_menu = !self.action_menu;
        self.action_notice = None;
        if self.action_menu
            && let Some(item) = self.item().filter(|item| item.kind == "pr")
        {
            if self.automations_for.as_deref() != Some(&item.key()) {
                self.item_automations.clear();
            }
            self.automations_for = Some(item.key());
            cx.emit(Event::AutomationsWanted(Box::new(item)));
        }
        cx.notify();
    }
    /// The Shell's answer to [`Event::AutomationsWanted`].
    pub(crate) fn set_item_automations(
        &mut self,
        key: &str,
        automations: Vec<ItemAutomation>,
        cx: &mut Context<Self>,
    ) {
        if self.automations_for.as_deref() == Some(key) {
            self.item_automations = automations;
            cx.notify();
        }
    }
    /// A run the ⋯ menu started on the item with `key`.
    pub(crate) fn track_item_run(&mut self, key: String, run: ItemRun, cx: &mut Context<Self>) {
        self.item_runs.insert(key, run);
        cx.notify();
    }
    /// A run record was saved: a run started here follows it.
    pub(crate) fn run_updated(&mut self, run: &AutomationRun, cx: &mut Context<Self>) {
        let mut changed = false;
        for tracked in self.item_runs.values_mut().filter(|t| t.run == run.id) {
            tracked.status = run.status.clone();
            tracked.started_at = run.started_at.unwrap_or(run.created_at);
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }
    /// A menu action could not start.
    pub(crate) fn menu_failed(&mut self, error: String, cx: &mut Context<Self>) {
        self.action_notice = Some(error);
        cx.notify();
    }
    pub(super) fn item_run(&self, item: &Item) -> Option<&ItemRun> {
        self.item_runs.get(&item.key())
    }

    fn menu_pick(&mut self, entry: Entry, cx: &mut Context<Self>) {
        let Some(item) = self.item() else {
            return;
        };
        match entry {
            Entry::Review => {
                self.action_menu = false;
                cx.emit(Event::ReviewWithAgent(Box::new(item)));
            }
            Entry::Open(_) => {
                self.action_menu = false;
                cx.open_url(&item.url);
            }
            // Stays open so the tick can say it worked.
            Entry::Copy => self.copy_link(item.url.clone(), cx),
            Entry::Pr(name, ..) => {
                self.action_menu = false;
                if !self.acting {
                    self.confirm_action = Some(name.into());
                    self.action_error = None;
                    self.action_notice = None;
                }
            }
            Entry::Automation(ix) => {
                self.action_menu = false;
                if let Some(automation) = self.item_automations.get(ix) {
                    cx.emit(Event::RunAutomation {
                        item: Box::new(item),
                        automation: automation.id.clone(),
                    });
                }
            }
            Entry::NewAutomation => {
                self.action_menu = false;
                cx.emit(Event::NewPullRequestAutomation(Box::new(item)));
            }
            Entry::Divider | Entry::Automations(_) => {}
        }
        cx.notify();
    }

    /// The ⋯ menu, under its button.
    pub(super) fn action_menu_view(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(item) = self.item() else {
            return div().into_any_element();
        };
        let copied = self
            .copied_link
            .as_ref()
            .is_some_and(|(url, _)| *url == item.url);
        let weak = cx.entity().downgrade();
        let row = |id: SharedString, label: SharedString| {
            div()
                .id(id.clone())
                .debug_selector(move || id.to_string())
                .role(Role::MenuItem)
                .aria_label(label)
                .h(px(34.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(10.))
                .rounded(px(9.))
                .text_size(px(13.))
                .text_color(content(0.92))
                .cursor_pointer()
                .hover(|s| s.bg(content(0.07)))
        };
        let glyph = |name: &'static str, ink: gpui::Hsla| icon(name, 15.).text_color(ink);
        let entries = entries(&item, self.item_automations.len());
        popover(
            "inbox-action-menu",
            Anchor::Slot(self.action_anchor.clone()),
        )
        .glass_menu()
        .align(Align::End)
        .width(340.)
        .padding(6.)
        .on_dismiss(move |_, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.action_menu = false;
                cx.notify();
            });
        })
        .children(entries.into_iter().map(|entry| {
            let pick = entry.clone();
            let on_click = cx.listener(move |this, _, _, cx| this.menu_pick(pick.clone(), cx));
            match entry {
                Entry::Review => row("inbox-menu-review".into(), "Review with an agent".into())
                    .bg(content(0.07))
                    .child(glyph("message-square", rgb(0xc0affc).into()))
                    .child(div().flex_1().child("Review with an agent"))
                    .child(
                        div()
                            .px(px(5.))
                            .h(px(18.))
                            .flex()
                            .items_center()
                            .rounded(px(5.))
                            .border_1()
                            .border_color(glass::edge())
                            .text_size(px(11.))
                            .text_color(content(0.55))
                            .child("R"),
                    )
                    .on_click(on_click)
                    .into_any_element(),
                Entry::Open(label) => row("inbox-menu-open".into(), label.clone().into())
                    .child(glyph("external-link", content(0.7)))
                    .child(label)
                    .on_click(on_click)
                    .into_any_element(),
                Entry::Copy => row(
                    "inbox-menu-copy".into(),
                    if copied { "Copied link" } else { "Copy link" }.into(),
                )
                .child(if copied {
                    glyph("check", diff_foreground(true))
                } else {
                    glyph("response-copy", content(0.7))
                })
                .child(if copied { "Copied link" } else { "Copy link" })
                .on_click(on_click)
                .into_any_element(),
                Entry::Pr(name, label, icon_name) => {
                    row(format!("inbox-menu-pr-{name}").into(), label.into())
                        .when(self.acting, |d| d.opacity(0.4))
                        .child(glyph(icon_name, content(0.7)))
                        .child(label)
                        .on_click(on_click)
                        .into_any_element()
                }
                Entry::Divider => div()
                    .h(px(1.))
                    .mx(px(4.))
                    .my(px(6.))
                    .bg(content(0.07))
                    .into_any_element(),
                Entry::Automations(label) => div()
                    .px(px(10.))
                    .pt(px(4.))
                    .pb(px(6.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .child(glyph("zap", rgb(0xfbbf24).into()).size(px(12.)))
                    .child(glass::eyebrow(label))
                    .into_any_element(),
                Entry::Automation(ix) => {
                    let automation = self.item_automations[ix].clone();
                    row(
                        format!("inbox-menu-automation-{ix}").into(),
                        format!("Run {}", automation.name).into(),
                    )
                    .h(px(44.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(div().truncate().child(automation.name))
                            .child(
                                div()
                                    .truncate()
                                    .text_size(px(11.5))
                                    .text_color(content(0.5))
                                    .child(format!(
                                        "{} · {}",
                                        automation.trigger, automation.agent
                                    )),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(12.))
                            .text_color(content(0.5))
                            .child("Run"),
                    )
                    .on_click(on_click)
                    .into_any_element()
                }
                Entry::NewAutomation => row(
                    "inbox-menu-new-automation".into(),
                    "New pull request automation".into(),
                )
                .h(px(30.))
                .text_size(px(12.))
                .text_color(content(0.62))
                .child(glyph("plus", content(0.62)).size(px(13.)))
                .child("New pull request automation")
                .on_click(on_click)
                .into_any_element(),
            }
        }))
        .into_any_element()
    }

    /// The pill of the run the ⋯ menu started on `item`: it opens Runs on
    /// that automation.
    pub(super) fn run_pill_view(&self, item: &Item, cx: &mut Context<Self>) -> Option<AnyElement> {
        let run = self.item_run(item)?.clone();
        let (label, tone) = run_pill(&run, gespan_core::time::now_ms() as i64);
        Some(
            div()
                .id("inbox-run-pill")
                .debug_selector(|| "inbox-run-pill".into())
                .role(Role::Button)
                .aria_label(format!("Open {} in Runs", run.name))
                .cursor_pointer()
                .child(glass::status(label, tone))
                .on_click(cx.listener(move |_, _, _, cx| {
                    cx.emit(Event::OpenRuns {
                        automation: run.automation.clone(),
                        name: run.name.clone(),
                    })
                }))
                .into_any_element(),
        )
    }
}

/// Two letters for a login: "vasanth" is "VA", "dependabot[bot]" is "DE".
pub(super) fn initials(login: &str) -> String {
    login
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// A 22px initials disc in a colour picked from the login, the same each time.
pub(super) fn initials_avatar(login: &str) -> AnyElement {
    const FILLS: [u32; 6] = [0xc0affc, 0x6ee7b7, 0xfda4af, 0xfbbf24, 0x7dd3fc, 0xebebeb];
    let hash = login.bytes().fold(0u32, |hash, byte| {
        hash.wrapping_mul(31).wrapping_add(u32::from(byte))
    });
    div()
        .flex_none()
        .size(px(22.))
        .rounded_full()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(FILLS[hash as usize % FILLS.len()]))
        .text_size(px(10.))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(0x111111))
        .child(initials(login))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{
        Bounds, TestAppContext, VisualTestContext, WindowBounds, WindowHandle, WindowOptions,
        point, size,
    };
    use std::{cell::RefCell, rc::Rc};

    fn pr(provider: &str) -> Item {
        Item {
            provider: provider.into(),
            kind: "pr".into(),
            repo: "acme/web".into(),
            number: 476,
            state: "open".into(),
            title: "Change".into(),
            url: "https://example.test/pr/476".into(),
            project_path: "/repo".into(),
            ..Default::default()
        }
    }
    fn names(entries: &[Entry]) -> Vec<String> {
        entries
            .iter()
            .map(|entry| match entry {
                Entry::Review => "review".into(),
                Entry::Open(label) => label.clone(),
                Entry::Copy => "copy".into(),
                Entry::Pr(name, ..) => format!("pr:{name}"),
                Entry::Divider => "-".into(),
                Entry::Automations(label) => label.clone(),
                Entry::Automation(ix) => format!("automation:{ix}"),
                Entry::NewAutomation => "new".into(),
            })
            .collect()
    }

    #[test]
    fn the_menu_offers_what_each_provider_and_state_can_do() {
        // An open GitHub pull request: review, links, its actions, then the
        // automations to run on it.
        assert_eq!(
            names(&entries(&pr("github"), 2)),
            [
                "review",
                "Open on GitHub",
                "copy",
                "-",
                "pr:merge",
                "pr:squash",
                "pr:rebase",
                "pr:draft",
                "pr:close",
                "-",
                "Run an automation on #476",
                "automation:0",
                "automation:1",
                "new",
            ]
        );
        // A draft can only be marked ready or closed; a closed one reopened.
        let draft = Item {
            draft: true,
            ..pr("github")
        };
        let actions = |item: &Item| {
            names(&entries(item, 0))
                .into_iter()
                .filter(|name| name.starts_with("pr:"))
                .collect::<Vec<_>>()
        };
        assert_eq!(actions(&draft), ["pr:ready", "pr:close"]);
        let closed = Item {
            state: "closed".into(),
            ..pr("github")
        };
        assert_eq!(actions(&closed), ["pr:reopen"]);
        // GitLab: no agent review or merge actions, but its automations.
        assert_eq!(
            names(&entries(&pr("gitlab"), 0)),
            [
                "Open on GitLab",
                "copy",
                "-",
                "Run an automation on #476",
                "new"
            ]
        );
        // An issue has only its links; without an address, nothing leads.
        let issue = Item {
            kind: "issue".into(),
            ..pr("github")
        };
        assert_eq!(names(&entries(&issue, 3)), ["Open on GitHub", "copy"]);
        let linkless = Item {
            url: String::new(),
            ..pr("github")
        };
        assert_eq!(
            names(&entries(&linkless, 0))[..3],
            ["review", "-", "pr:merge"]
        );
        // A tracker's issue cannot start a pull request automation.
        assert_eq!(new_automation_event("linear"), None);
        assert_eq!(new_automation_event("github"), Some("pull_request_opened"));
    }

    #[test]
    fn the_run_pill_reads_each_state_of_its_run() {
        let run = |status: &str, started_at: i64| ItemRun {
            automation: "a".into(),
            name: "Review pull requests".into(),
            run: "r".into(),
            status: status.into(),
            started_at,
        };
        let now = 10 * 60 * 60_000;
        let pill = |status: &str, ago_ms: i64| run_pill(&run(status, now - ago_ms), now);
        assert_eq!(
            pill("pending", 0),
            ("Review pull requests · queued".into(), glass::Tone::Running)
        );
        assert_eq!(
            pill("running", 30_000),
            (
                "Review pull requests · running".into(),
                glass::Tone::Running
            )
        );
        assert_eq!(
            pill("running", 60_000),
            (
                "Review pull requests · running 1m".into(),
                glass::Tone::Running
            )
        );
        assert_eq!(
            pill("running", 125 * 60_000).0,
            "Review pull requests · running 2h"
        );
        assert_eq!(
            pill("succeeded", 0),
            ("Review pull requests · succeeded".into(), glass::Tone::Ok)
        );
        assert_eq!(
            pill("failed", 0),
            ("Review pull requests · failed".into(), glass::Tone::Bad)
        );
        assert_eq!(
            pill("skipped", 0),
            ("Review pull requests · skipped".into(), glass::Tone::Muted)
        );
        // A clock behind the run's start never reads negative.
        assert_eq!(
            pill("running", -5 * 60_000).0,
            "Review pull requests · running"
        );
    }

    #[test]
    fn initials_come_from_the_logins_first_two_letters_or_digits() {
        assert_eq!(initials("vasanth"), "VA");
        assert_eq!(initials("dependabot[bot]"), "DE");
        assert_eq!(initials("-x"), "X");
        assert_eq!(initials("9lives"), "9L");
        assert_eq!(initials(""), "");
    }

    fn open(cx: &mut TestAppContext, item: Item) -> WindowHandle<Inbox> {
        cx.update(|cx| cx.set_global(crate::settings::Preferences::default()));
        let store = gespan_core::store::Store::open(std::path::Path::new(":memory:")).unwrap();
        let store = std::sync::Arc::new(std::sync::Mutex::new(store));
        let window = cx.update(|cx| {
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                        point(px(0.), px(0.)),
                        size(px(1300.), px(900.)),
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|cx| {
                        let mut inbox =
                            Inbox::new_inner(store, vec![], "/missing/inbox".into(), false, cx);
                        inbox.selected = Some(item.key());
                        inbox.snapshot.items = vec![item];
                        inbox.details = Some(Details::default());
                        inbox
                    })
                },
            )
            .unwrap()
        });
        cx.run_until_parked();
        window
    }
    fn settle(visual: &mut VisualTestContext) {
        visual.run_until_parked();
        visual.refresh().unwrap();
    }
    fn click(visual: &mut VisualTestContext, selector: &'static str) {
        settle(visual);
        let bounds = visual
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is not on screen"));
        visual.simulate_click(bounds.center(), Default::default());
        settle(visual);
    }
    /// What the Inbox asked the Shell for, as words.
    fn record(window: WindowHandle<Inbox>, cx: &mut TestAppContext) -> Rc<RefCell<Vec<String>>> {
        let seen = Rc::new(RefCell::new(vec![]));
        let inbox = window.root(cx).unwrap();
        let sink = seen.clone();
        cx.update(|cx| {
            cx.subscribe(&inbox, move |_, event: &Event, _| {
                let said = match event {
                    Event::AutomationsWanted(item) => format!("wanted {}", item.key()),
                    Event::RunAutomation { item, automation } => {
                        format!("run {automation} on {}", item.key())
                    }
                    Event::ReviewWithAgent(item) => format!("review {}", item.key()),
                    Event::NewPullRequestAutomation(item) => format!("new for {}", item.key()),
                    Event::OpenRuns { automation, name } => format!("runs {automation} {name}"),
                    _ => return,
                };
                sink.borrow_mut().push(said);
            })
            .detach()
        });
        seen
    }

    #[gpui::test]
    fn the_menu_runs_the_chosen_automation_on_this_pull_request(cx: &mut TestAppContext) {
        let item = pr("github");
        let key = item.key();
        let window = open(cx, item);
        let seen = record(window, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        click(visual, "inbox-more");
        // Opening asks for this pull request's automations.
        assert_eq!(*seen.borrow(), [format!("wanted {key}")]);
        window
            .update(visual, |inbox, _, cx| {
                let offered = vec![
                    ItemAutomation {
                        id: "triage".into(),
                        name: "Triage".into(),
                        trigger: "When a pull request is opened".into(),
                        agent: "Codex".into(),
                    },
                    ItemAutomation {
                        id: "fix".into(),
                        name: "Fix checks".into(),
                        trigger: "When a check fails".into(),
                        agent: "Claude Code".into(),
                    },
                ];
                // An answer for another item is ignored.
                inbox.set_item_automations("github:other", offered.clone(), cx);
                assert!(inbox.item_automations.is_empty());
                inbox.set_item_automations(&key, offered, cx);
            })
            .unwrap();
        click(visual, "inbox-menu-automation-1");
        assert_eq!(seen.borrow().last().unwrap(), &format!("run fix on {key}"));
        // Choosing closes the menu.
        settle(visual);
        assert!(visual.debug_bounds("inbox-menu-automation-0").is_none());

        click(visual, "inbox-more");
        click(visual, "inbox-menu-review");
        assert_eq!(seen.borrow().last().unwrap(), &format!("review {key}"));
        click(visual, "inbox-more");
        click(visual, "inbox-menu-new-automation");
        assert_eq!(seen.borrow().last().unwrap(), &format!("new for {key}"));
        // A pull request action asks for confirmation first.
        click(visual, "inbox-more");
        click(visual, "inbox-menu-pr-close");
        let confirming = window
            .update(visual, |inbox, _, _| inbox.confirm_action.clone())
            .unwrap();
        assert_eq!(confirming.as_deref(), Some("close"));
    }

    #[gpui::test]
    fn a_started_run_shows_its_record_and_opens_runs_on_its_automation(cx: &mut TestAppContext) {
        let item = pr("github");
        let key = item.key();
        let window = open(cx, item);
        let seen = record(window, cx);
        let visual = &mut VisualTestContext::from_window(window.into(), cx);
        settle(visual);
        assert!(visual.debug_bounds("inbox-run-pill").is_none());
        let mut record = AutomationRun {
            id: "run-1".into(),
            automation_id: "review".into(),
            trigger: "event".into(),
            scheduled_for: 0,
            created_at: gespan_core::time::now_ms() as i64,
            started_at: None,
            completed_at: None,
            status: "pending".into(),
            session_id: None,
            error: None,
            event_key: None,
            event_kind: None,
            event: None,
            event_title: None,
            prompt: None,
            pilot_message_id: None,
        };
        window
            .update(visual, |inbox, _, cx| {
                inbox.track_item_run(
                    key.clone(),
                    ItemRun {
                        automation: "review".into(),
                        name: "Review pull requests".into(),
                        run: "run-1".into(),
                        status: "pending".into(),
                        started_at: record.created_at,
                    },
                    cx,
                );
            })
            .unwrap();
        settle(visual);
        assert!(visual.debug_bounds("inbox-run-pill").is_some());
        // The record moves on; another run's record changes nothing.
        record.status = "succeeded".into();
        let other = AutomationRun {
            id: "run-2".into(),
            status: "failed".into(),
            ..record.clone()
        };
        let status = window
            .update(visual, |inbox, _, cx| {
                inbox.run_updated(&other, cx);
                let before = inbox.item_runs[&key].status.clone();
                inbox.run_updated(&record, cx);
                (before, inbox.item_runs[&key].status.clone())
            })
            .unwrap();
        assert_eq!(status, ("pending".to_string(), "succeeded".to_string()));
        click(visual, "inbox-run-pill");
        assert_eq!(
            seen.borrow().last().unwrap(),
            "runs review Review pull requests"
        );
    }
}
