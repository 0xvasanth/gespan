//! The thread's browser tabs: the globe beside the composer's pills while
//! an agent has tabs open for the thread and the browser switch is on,
//! listing them with "Show in Gespan", and the news of each tab an agent opens, which the shell uses
//! to show the first one beside the thread on its own.
use super::*;
use gespan_core::browser::{
    BrowserState, TabState,
    view::{self as browser_view},
};

/// Tailwind's teal-400: the browser feature's colour.
const TEAL: u32 = 0x2dd4bf;

fn teal(alpha: f32) -> gpui::Hsla {
    let mut color: gpui::Hsla = gpui::rgb(TEAL).into();
    color.a = alpha;
    color
}

/// An agent opened `0`, a tab of this thread, since the thread was shown.
pub(crate) struct BrowserTabOpened(pub(crate) String);

/// One of the thread's tabs, as the menu lists it.
#[derive(Clone, Debug, PartialEq)]
struct Tab {
    id: String,
    title: String,
    host: String,
    status: &'static str,
    open: bool,
    /// The user opened it from a link, so it already shows.
    user: bool,
}

/// The line above the composer while a link opens in the in-app browser.
const LINK_OPENING: &str = "Opening in the in-app browser…";

#[derive(Default)]
pub(super) struct State {
    tabs: Vec<Tab>,
    /// The browser switch is on. Off, the globe hides: nothing it lists
    /// could be shown.
    on: bool,
    menu: bool,
    anchor: AnchorSlot,
    /// A link the in-app browser could not open went to the system browser;
    /// the reason shows above the composer for a moment. While a link is
    /// still opening, [`LINK_OPENING`] shows there.
    link_notice: Option<String>,
    link_notice_clear: Option<gpui::Task<()>>,
}

impl State {
    pub(super) fn new(session: i64, cx: &gpui::App) -> Self {
        let state = crate::browser::get(cx).map(|b| &b.read(cx).state);
        State {
            tabs: tabs_of(session, state),
            on: state.is_some_and(|state| state.toggle_on),
            ..State::default()
        }
    }
}

/// The thread's tabs that have not closed.
fn tabs_of(session: i64, state: Option<&BrowserState>) -> Vec<Tab> {
    let Some(state) = state else {
        return Vec::new();
    };
    let key = crate::browser::session_key(session);
    browser_view::session_tabs(state, &key)
        .filter(|tab| tab.tab.state != TabState::Closed)
        .map(|tab| Tab {
            id: tab.tab.tab.0.clone(),
            title: if tab.tab.title.is_empty() {
                browser_view::tab_host(&tab.tab.url)
            } else {
                tab.tab.title.clone()
            },
            host: browser_view::tab_host(&tab.tab.url),
            status: browser_view::tab_status(tab),
            open: tab.tab.state == TabState::Open,
            user: tab.user_tab,
        })
        .collect()
}

impl EventEmitter<BrowserTabOpened> for Conversation {}

impl Conversation {
    /// Follow the browser module's state for this thread.
    pub(super) fn observe_browser(cx: &mut Context<Self>) -> Option<Subscription> {
        let browser = crate::browser::get(cx)?;
        Some(cx.observe(&browser, |this, browser, cx| {
            let state = &browser.read(cx).state;
            let on = state.toggle_on;
            let tabs = tabs_of(this.summary.id, Some(state));
            if tabs == this.browser.tabs && on == this.browser.on {
                return;
            }
            this.browser.on = on;
            // A tab that is open now and was not before: the agent opened
            // it (or it finished opening).
            let opened: Vec<String> = tabs
                .iter()
                .filter(|tab| {
                    tab.open
                        && !tab.user
                        && !this
                            .browser
                            .tabs
                            .iter()
                            .any(|old| old.id == tab.id && old.open)
                })
                .map(|tab| tab.id.clone())
                .collect();
            if tabs.is_empty() || !on {
                this.browser.menu = false;
            }
            this.browser.tabs = tabs;
            for tab in opened {
                cx.emit(BrowserTabOpened(tab));
            }
            cx.notify();
        }))
    }

    /// "Show in Gespan": the tab live beside the thread.
    pub(crate) fn show_browser_tab(&mut self, tab: String, cx: &mut Context<Self>) {
        cx.emit(OpenSessionChanges(crate::review::ReviewTab::Browser {
            session: self.summary.id,
            tab,
            own: false,
        }));
    }

    /// A link went to the system browser because the in-app one could not
    /// open it: say why, for a few seconds.
    pub(crate) fn show_link_notice(&mut self, why: String, cx: &mut Context<Self>) {
        self.browser.link_notice = Some(format!(
            "Opened in your system browser: the in-app browser couldn't open the link ({why})."
        ));
        self.browser.link_notice_clear = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_secs(8))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.browser.link_notice = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// A link of the thread is opening in the in-app browser, which may
    /// first have to start: say so until its pane opens
    /// ([`Self::clear_link_opening`]) or the system browser takes it
    /// ([`Self::show_link_notice`]).
    pub(crate) fn show_link_opening(&mut self, cx: &mut Context<Self>) {
        self.browser.link_notice = Some(LINK_OPENING.into());
        self.browser.link_notice_clear = None;
        cx.notify();
    }

    /// The thread's links have opened: the opening line goes. Any other
    /// notice stays for its moment.
    pub(crate) fn clear_link_opening(&mut self, cx: &mut Context<Self>) {
        if self.browser.link_notice.as_deref() == Some(LINK_OPENING) {
            self.browser.link_notice = None;
            cx.notify();
        }
    }

    /// The text of the link notice, while it shows.
    #[cfg(test)]
    pub(crate) fn link_notice(&self) -> Option<&str> {
        self.browser.link_notice.as_deref()
    }

    /// A thin line above the composer while a link notice shows.
    pub(super) fn link_notice_line(&self) -> Option<AnyElement> {
        let text = self.browser.link_notice.clone()?;
        Some(
            div()
                .debug_selector(|| "browser-link-notice".into())
                .mb(px(6.))
                .px(px(12.))
                .py(px(6.))
                .rounded(px(10.))
                .bg(content(0.05))
                .flex()
                .items_center()
                .gap(px(8.))
                .text_size(px(12.))
                .text_color(content(0.6))
                .child(icon("globe", 13.).text_color(teal(0.9)))
                .child(div().min_w_0().flex_1().child(text))
                .into_any_element(),
        )
    }

    /// The globe beside the composer's pills, while the thread has tabs
    /// and the browser switch is on.
    pub(super) fn browser_pill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.browser.tabs.is_empty() || !self.browser.on {
            return None;
        }
        Some(
            tool_pill("composer-browser", "Browser tabs")
                .debug_selector(|| "composer-browser".into())
                .aria_expanded(self.browser.menu)
                .when(self.browser.menu, |d| d.bg(content(0.2)))
                .w(px(26.))
                .justify_center()
                .child(self.browser.anchor.probe())
                .child(icon("globe", 14.).text_color(teal(0.9)))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.browser.menu = !this.browser.menu;
                    cx.notify();
                }))
                .into_any_element(),
        )
    }

    /// The globe's menu: the thread's tabs, each to show beside the thread.
    pub(super) fn browser_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.browser.menu || self.browser.tabs.is_empty() || !self.browser.on {
            return None;
        }
        let rows: Vec<AnyElement> = self
            .browser
            .tabs
            .iter()
            .map(|tab| {
                let show = tab.id.clone();
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .rounded_lg()
                    .bg(content(0.03))
                    .px(px(10.))
                    .py(px(6.))
                    .text_size(px(12.))
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .child(
                                div()
                                    .truncate()
                                    .text_color(content(1.))
                                    .child(tab.title.clone()),
                            )
                            .child(
                                div()
                                    .truncate()
                                    .text_color(content(0.45))
                                    .child(format!("{} · {}", tab.host, tab.status)),
                            ),
                    )
                    .when(tab.open, |d| {
                        let id = format!("browser-show-{}", tab.id);
                        d.child(
                            div()
                                .id(SharedString::from(id.clone()))
                                .debug_selector(move || id.clone())
                                .role(Role::Button)
                                .aria_label("Show in Gespan")
                                .flex_shrink_0()
                                .h(px(24.))
                                .px(px(6.))
                                .flex()
                                .items_center()
                                .gap_1()
                                .rounded_md()
                                .cursor_pointer()
                                .text_color(content(0.7))
                                .hover(|s| s.bg(content(0.1)).text_color(content(1.)))
                                .child(icon("app-window", 14.).text_color(content(0.6)))
                                .child("Show in Gespan")
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.browser.menu = false;
                                    this.show_browser_tab(show.clone(), cx);
                                    cx.notify();
                                })),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        let weak = cx.entity().downgrade();
        Some(
            popover(
                "composer-browser-menu",
                Anchor::Slot(self.browser.anchor.clone()),
            )
            .side(Side::Top)
            .align(Align::End)
            .width(320.)
            .on_dismiss(move |_, cx| {
                let _ = weak.update(cx, |this, cx| {
                    this.browser.menu = false;
                    cx.notify();
                });
            })
            .child(
                div()
                    .debug_selector(|| "composer-browser-menu".into())
                    .p_2()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .child(
                        div()
                            .px_1()
                            .pb_1()
                            .text_size(px(12.))
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(content(1.))
                            .child("This thread's browser tabs"),
                    )
                    .children(rows),
            )
            .into_any_element(),
        )
    }
}
