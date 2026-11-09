//! The thread's browser tabs: the globe beside the composer's pills while
//! an agent has tabs open for the thread, listing them with "Show in
//! Gespan", and the news of each tab an agent opens, which the shell uses
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
}

#[derive(Default)]
pub(super) struct State {
    tabs: Vec<Tab>,
    menu: bool,
    anchor: AnchorSlot,
}

impl State {
    pub(super) fn new(session: i64, cx: &gpui::App) -> Self {
        State {
            tabs: tabs_of(session, crate::browser::get(cx).map(|b| &b.read(cx).state)),
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
        })
        .collect()
}

impl EventEmitter<BrowserTabOpened> for Conversation {}

impl Conversation {
    /// Follow the browser module's state for this thread.
    pub(super) fn observe_browser(cx: &mut Context<Self>) -> Option<Subscription> {
        let browser = crate::browser::get(cx)?;
        Some(cx.observe(&browser, |this, browser, cx| {
            let tabs = tabs_of(this.summary.id, Some(&browser.read(cx).state));
            if tabs == this.browser.tabs {
                return;
            }
            // A tab that is open now and was not before: the agent opened
            // it (or it finished opening).
            let opened: Vec<String> = tabs
                .iter()
                .filter(|tab| {
                    tab.open
                        && !this
                            .browser
                            .tabs
                            .iter()
                            .any(|old| old.id == tab.id && old.open)
                })
                .map(|tab| tab.id.clone())
                .collect();
            if tabs.is_empty() {
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
        }));
    }

    /// The globe beside the composer's pills, while the thread has tabs.
    pub(super) fn browser_pill(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.browser.tabs.is_empty() {
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
        if !self.browser.menu || self.browser.tabs.is_empty() {
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
