//! "Show in Gespan": one of a thread's browser tabs, live beside the
//! thread. The pictures the browser makes of the tab are drawn as images,
//! with no web view of our own; clicks, scrolls and keys go back to the tab.
//! Only one pane shows a tab at a time: a second view of it would stop the
//! first, so the review pane keeps one pane per tab.
use crate::{popover::AnchorSlot, theme::*};
use gespan_core::browser::{
    Ended, LivePane, TabId,
    live::{KeyEvent, LiveInput, LiveNav, MouseButton as LiveButton, MouseEvent, PanelSize},
};
use gpui::{
    AnyElement, App, Bounds, Context, FocusHandle, Focusable, ImageSource, KeyDownEvent,
    KeyUpEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels,
    Point, RenderImage, Role, ScrollDelta, ScrollWheelEvent, SharedString, Subscription, Task,
    Window, div, img, prelude::*, px,
};
use std::sync::Arc;
use std::time::{Duration, Instant};

gpui::actions!(
    browser_pane,
    [
        /// Tab, Shift+Tab and Escape in the pane go to the page, not to
        /// Gespan's focus keys.
        PageTab,
        PageShiftTab,
        PageEscape,
        /// F6 (and Cmd+Escape on macOS): the keys leave the page for
        /// the thread's composer.
        LeaveBrowser,
    ]
);

/// The pane's keys: the page's Tab and Escape, and the way out.
pub(crate) fn bind(cx: &mut App) {
    let context = Some("BrowserPane");
    cx.bind_keys([
        gpui::KeyBinding::new("tab", PageTab, context),
        gpui::KeyBinding::new("shift-tab", PageShiftTab, context),
        gpui::KeyBinding::new("escape", PageEscape, context),
    ]);
    cx.bind_keys(
        leave_keys(cfg!(target_os = "macos"))
            .iter()
            .map(|key| gpui::KeyBinding::new(key, LeaveBrowser, context)),
    );
}

/// The keys that take the keys out of the page: F6 everywhere, as in a
/// browser, and Cmd+Escape too on macOS. Ctrl+Escape belongs to the
/// system off macOS (Windows' Start menu, KDE's System Activity).
fn leave_keys(macos: bool) -> &'static [&'static str] {
    if macos {
        &["f6", "cmd-escape"]
    } else {
        &["f6"]
    }
}

/// The hint the pane shows while it has the keys.
fn leave_hint(macos: bool) -> &'static str {
    if macos {
        "F6 or ⌘Esc to leave"
    } else {
        "F6 to leave"
    }
}

/// The modifier Gespan's own shortcuts use: Cmd on macOS, Ctrl elsewhere.
fn app_modifier(modifiers: &Modifiers, macos: bool) -> bool {
    if macos {
        modifiers.platform
    } else {
        modifiers.control
    }
}

/// The app-modifier shortcuts the page keeps: select all, copy, cut,
/// paste and undo (Shift too for redo). Every other one is Gespan's.
fn page_shortcut(c: char, shift: bool) -> bool {
    match c.to_ascii_lowercase() {
        'a' | 'c' | 'v' | 'x' => !shift,
        'z' => true,
        _ => false,
    }
}

/// The editing command a page shortcut runs on macOS, where Chromium
/// leaves a Cmd key's edit undone unless the key names it. Paste is not
/// here: the pane types the clipboard's text itself.
fn edit_command(c: char, shift: bool) -> Option<&'static str> {
    Some(match (c.to_ascii_lowercase(), shift) {
        ('a', false) => "selectAll",
        ('c', false) => "copy",
        ('x', false) => "cut",
        ('z', false) => "undo",
        ('z', true) => "redo",
        _ => return None,
    })
}

/// AltGr off macOS: Windows reports it as Ctrl+Alt, so `@`, `{` or `€`
/// typed with it come with both held. The character it typed (not the
/// key's own) is text for the page, not a shortcut.
fn alt_gr_text(keystroke: &gpui::Keystroke, macos: bool) -> Option<String> {
    let modifiers = &keystroke.modifiers;
    if macos || !(modifiers.control && modifiers.alt) || modifiers.platform {
        return None;
    }
    keystroke.key_char.clone().filter(|text| {
        !text.is_empty()
            && !text.chars().any(char::is_control)
            && !text.eq_ignore_ascii_case(&keystroke.key)
    })
}

/// A picture on screen: its image and the page viewport it shows, in CSS
/// pixels.
pub(crate) struct Shown {
    image: Arc<RenderImage>,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

/// What one pull of the view brought: a picture, how it ended, and how
/// the last input sent went.
type Pulled = (
    Option<Result<gespan_core::browser::LivePicture, String>>,
    Option<Ended>,
    Option<Result<(), String>>,
);

/// How long the error line stays at least, so a moving page's next picture
/// or the pointer's next move never wipes it before it can be read.
const ERROR_SHOWN_AT_LEAST: Duration = Duration::from_secs(3);

/// The red line under the toolbar: what went wrong, and since when.
#[derive(Clone, Debug)]
pub(crate) struct PaneError {
    pub(crate) text: String,
    since: Instant,
}

impl PaneError {
    fn new(text: String) -> Self {
        PaneError {
            text,
            since: Instant::now(),
        }
    }

    /// Shown long enough for the next picture or input to clear it.
    fn read_by(&self, now: Instant) -> bool {
        now.saturating_duration_since(self.since) >= ERROR_SHOWN_AT_LEAST
    }
}

pub(crate) struct BrowserPane {
    pub(crate) session: i64,
    pub(crate) tab: TabId,
    focus: FocusHandle,
    live: Option<Arc<LivePane>>,
    pub(crate) picture: Option<Shown>,
    pub(crate) ended: Option<Ended>,
    /// Cleared by the next picture or input that goes through, once it has
    /// been up for [`ERROR_SHOWN_AT_LEAST`].
    pub(crate) error: Option<PaneError>,
    stage: AnchorSlot,
    size: Option<PanelSize>,
    /// Buttons held down over the picture, CDP's bit mask.
    buttons: u32,
    /// Where the button went down, until the pointer leaves the click's
    /// slop ([`CLICK_SLOP`]).
    press: Option<Press>,
    title: String,
    url: String,
    /// Out of sight (another tab of the review pane shows): the view stops,
    /// and starts again by itself when the pane shows, from the last picture.
    paused: bool,
    /// Not on screen (the review pane shows another tab, is hidden or
    /// closing, or the thread is not shown): no view until shown again.
    hidden: bool,
    _pull: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for BrowserPane {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// CDP's modifier bits: Alt 1, Ctrl 2, Meta 4, Shift 8.
fn modifier_bits(modifiers: &Modifiers) -> u32 {
    u32::from(modifiers.alt)
        | (u32::from(modifiers.control) << 1)
        | (u32::from(modifiers.platform) << 2)
        | (u32::from(modifiers.shift) << 3)
}

fn button_of(button: MouseButton) -> Option<(LiveButton, u32)> {
    match button {
        MouseButton::Left => Some((LiveButton::Left, 1)),
        MouseButton::Right => Some((LiveButton::Right, 2)),
        MouseButton::Middle => Some((LiveButton::Middle, 4)),
        MouseButton::Navigate(_) => None,
    }
}

/// Keys that edit or move rather than type: the DOM's key and code, and
/// the virtual key code.
fn raw_key(key: &str) -> Option<(&'static str, &'static str, u32)> {
    Some(match key {
        "backspace" => ("Backspace", "Backspace", 8),
        "tab" => ("Tab", "Tab", 9),
        "enter" => ("Enter", "Enter", 13),
        "escape" => ("Escape", "Escape", 27),
        "pageup" => ("PageUp", "PageUp", 33),
        "pagedown" => ("PageDown", "PageDown", 34),
        "end" => ("End", "End", 35),
        "home" => ("Home", "Home", 36),
        "left" => ("ArrowLeft", "ArrowLeft", 37),
        "up" => ("ArrowUp", "ArrowUp", 38),
        "right" => ("ArrowRight", "ArrowRight", 39),
        "down" => ("ArrowDown", "ArrowDown", 40),
        "delete" => ("Delete", "Delete", 46),
        _ => return None,
    })
}

/// The tab's input for a key the user pressed in the pane: editing keys,
/// shortcuts such as select all, and the text the key typed.
pub(crate) fn key_input(keystroke: &gpui::Keystroke, down: bool) -> Option<LiveInput> {
    key_input_on(keystroke, down, cfg!(target_os = "macos"))
}

/// [`key_input`] as it is on macOS (`macos`) or elsewhere.
fn key_input_on(keystroke: &gpui::Keystroke, down: bool, macos: bool) -> Option<LiveInput> {
    let modifiers = modifier_bits(&keystroke.modifiers);
    if let Some((key, code, key_code)) = raw_key(&keystroke.key) {
        return Some(LiveInput::Key {
            event: match (down, key_code) {
                (false, _) => KeyEvent::KeyUp,
                (true, 13) => KeyEvent::KeyDown,
                (true, _) => KeyEvent::RawKeyDown,
            },
            key: key.into(),
            code: code.into(),
            text: (down && key_code == 13).then(|| "\r".into()),
            key_code,
            modifiers,
            commands: Vec::new(),
        });
    }
    if let Some(text) = alt_gr_text(keystroke, macos) {
        return down.then_some(LiveInput::Text { text });
    }
    let command = keystroke.modifiers.control || keystroke.modifiers.platform;
    let mut chars = keystroke.key.chars();
    if let (Some(c), None) = (chars.next(), chars.next())
        && command
        && !keystroke.modifiers.alt
        && c.is_ascii_alphanumeric()
    {
        let shift = keystroke.modifiers.shift;
        let app = app_modifier(&keystroke.modifiers, macos);
        // Gespan's shortcuts (Cmd+W, Cmd+T, Cmd+1...) are not the page's.
        if app && !page_shortcut(c, shift) {
            return None;
        }
        let upper = c.to_ascii_uppercase();
        return Some(LiveInput::Key {
            event: if down {
                KeyEvent::RawKeyDown
            } else {
                KeyEvent::KeyUp
            },
            key: keystroke.key.clone(),
            code: if c.is_ascii_digit() {
                format!("Digit{c}")
            } else {
                format!("Key{upper}")
            },
            text: None,
            key_code: u32::from(upper),
            modifiers,
            commands: (macos && app && down)
                .then(|| edit_command(c, shift))
                .flatten()
                .map(|command| vec![command.to_owned()])
                .unwrap_or_default(),
        });
    }
    if !down || command {
        return None;
    }
    if keystroke.key == "space" {
        return Some(LiveInput::Text { text: " ".into() });
    }
    keystroke
        .key_char
        .clone()
        .filter(|text| !text.is_empty())
        .map(|text| LiveInput::Text { text })
}

/// How far the pointer may move while pressed and still click. Chromium
/// starts dragging a link from 4 pixels, which a hand easily moves while
/// clicking in the pane (Edge on Windows then sends the link no click); a
/// drag that goes further is a drag.
const CLICK_SLOP: f32 = 8.;

/// A button that went down over the picture.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Press {
    /// In the window.
    at: Point<Pixels>,
    /// In the page.
    page: (f64, f64),
}

/// What the pane sends for a mouse event while a button may be held.
#[derive(Debug, PartialEq)]
enum Pointer {
    /// Send it at this point of the page.
    Send((f64, f64)),
    /// Hold it back: a hand that shakes while clicking.
    Hold,
}

/// Keep a shaking click a click: moves within [`CLICK_SLOP`] of the press
/// stay with Gespan and the release lands where the button went down, so
/// the page sees a click and never the start of a drag. `press` is updated
/// for the next event.
fn through_slop(
    press: &mut Option<Press>,
    event: MouseEvent,
    at: Point<Pixels>,
    page: (f64, f64),
) -> Pointer {
    if event == MouseEvent::Pressed {
        *press = Some(Press { at, page });
        return Pointer::Send(page);
    }
    let Some(down) = *press else {
        return Pointer::Send(page);
    };
    let moved = (at.x - down.at.x)
        .as_f32()
        .hypot((at.y - down.at.y).as_f32());
    let within = moved < CLICK_SLOP;
    match event {
        MouseEvent::Moved if within => Pointer::Hold,
        MouseEvent::Moved => {
            *press = None;
            Pointer::Send(page)
        }
        _ => {
            *press = None;
            Pointer::Send(if within { down.page } else { page })
        }
    }
}

/// A point on the shown picture, in the page's CSS pixels.
fn page_point(at: Point<Pixels>, rect: Bounds<Pixels>, width: f64, height: f64) -> (f64, f64) {
    let fraction = |offset: Pixels, side: Pixels| {
        (f64::from(offset.as_f32()) / f64::from(side.as_f32().max(1.))).clamp(0., 1.)
    };
    let round = |v: f64| (v * 10.).round() / 10.;
    (
        round(fraction(at.x - rect.left(), rect.size.width) * width),
        round(fraction(at.y - rect.top(), rect.size.height) * height),
    )
}

impl BrowserPane {
    pub(crate) fn new(session: i64, tab: TabId, cx: &mut Context<Self>) -> Self {
        let mut subscriptions = Vec::new();
        if let Some(browser) = crate::browser::get(cx) {
            subscriptions.push(cx.observe(&browser, |this, _, cx| this.follow_tab(cx)));
        }
        cx.on_release(|this, cx| {
            if let Some(shown) = this.picture.take() {
                cx.drop_image(shown.image, None);
            }
        })
        .detach();
        let mut this = Self {
            session,
            tab,
            focus: cx.focus_handle(),
            live: None,
            picture: None,
            ended: None,
            error: None,
            stage: AnchorSlot::default(),
            size: None,
            buttons: 0,
            press: None,
            title: String::new(),
            url: String::new(),
            paused: false,
            hidden: false,
            _pull: None,
            _subscriptions: subscriptions,
        };
        this.follow_tab(cx);
        this
    }

    /// The tab's title and address from the module's state.
    fn follow_tab(&mut self, cx: &mut Context<Self>) {
        let Some(browser) = crate::browser::get(cx) else {
            return;
        };
        let found = browser
            .read(cx)
            .state
            .tabs
            .iter()
            .find(|t| t.tab.tab == self.tab)
            .map(|t| (t.tab.title.clone(), t.tab.url.clone()));
        let Some((title, url)) = found else {
            return;
        };
        if url != self.url {
            self.url = url;
            cx.notify();
        }
        if title != self.title {
            self.title = title;
            cx.notify();
        }
    }

    pub(crate) fn title(&self) -> String {
        if self.title.is_empty() {
            gespan_core::browser::view::tab_host(&self.url)
        } else {
            self.title.clone()
        }
    }

    /// The pane went out of sight: stop its view until it shows again.
    fn pause(&mut self) {
        if self.live.take().is_some() {
            self._pull = None;
            self.paused = true;
        }
    }

    /// The review pane shows this tab, or hides it (another tab is in
    /// front, the review pane is hidden or closing, or its thread is not
    /// on screen): hidden, the view stops; shown again, the next frame
    /// starts it again.
    pub(crate) fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        if shown != self.hidden {
            return;
        }
        if !shown {
            self.pause();
        }
        self.hidden = !shown;
        cx.notify();
    }

    /// Start the view once the pane has a size; follow later sizes.
    fn sync_size(&mut self, window: &Window, cx: &mut Context<Self>) {
        let Some(bounds) = self.stage.get().filter(|_| !self.hidden) else {
            return;
        };
        let size = PanelSize::new(
            bounds.size.width.as_f32().round() as u32,
            bounds.size.height.as_f32().round() as u32,
            f64::from(window.scale_factor()),
        );
        if std::mem::take(&mut self.paused) && self.ended.is_none() {
            self.size = Some(size);
            self.start(size, cx);
            return;
        }
        if self.size == Some(size) {
            return;
        }
        self.size = Some(size);
        match &self.live {
            Some(live) => {
                let live = live.clone();
                cx.background_executor()
                    .spawn(async move { live.resize(size) })
                    .detach();
            }
            None if self.ended.is_none() => self.start(size, cx),
            None => {}
        }
    }

    fn start(&mut self, size: PanelSize, cx: &mut Context<Self>) {
        let started = crate::browser::host(cx)
            .ok_or_else(|| Ended::Failed("Browser access is not available.".to_owned()))
            .and_then(|host| LivePane::start_with(host, self.session, self.tab.clone(), size));
        let live = match started {
            Ok(live) => Arc::new(live),
            Err(ended) => {
                self.ended = Some(ended);
                cx.notify();
                return;
            }
        };
        self.live = Some(live.clone());
        self.ended = None;
        // Tests take each picture themselves (`pull_for_tests`): a loop that
        // never parks would hold the test scheduler forever.
        if !cfg!(test) {
            self._pull = Some(cx.spawn(async move |this, cx| {
                loop {
                    let pane = live.clone();
                    let pulled = cx
                        .background_executor()
                        .spawn(async move { pull(&pane) })
                        .await;
                    let keep = this.update(cx, |this, cx| this.accept(pulled, cx));
                    if !keep.unwrap_or(false) {
                        break;
                    }
                }
            }));
        }
        cx.notify();
    }

    /// Show what a pull brought; `false` once the view is over.
    fn accept(&mut self, (picture, ended, input): Pulled, cx: &mut Context<Self>) -> bool {
        let now = Instant::now();
        let went_through = matches!(picture, Some(Ok(_))) || matches!(input, Some(Ok(())));
        if went_through && self.error.as_ref().is_some_and(|error| error.read_by(now)) {
            self.error = None;
            cx.notify();
        }
        if let Some(Err(error)) = input {
            self.error = Some(PaneError::new(error));
            cx.notify();
        }
        match picture {
            Some(Ok(picture)) => {
                let image = Arc::new(RenderImage::new(vec![image::Frame::new(picture.pixels)]));
                let old = self.picture.replace(Shown {
                    image,
                    width: picture.width,
                    height: picture.height,
                });
                if let Some(old) = old {
                    cx.drop_image(old.image, None);
                }
                cx.notify();
                true
            }
            Some(Err(error)) => {
                self.error = Some(PaneError::new(error));
                cx.notify();
                true
            }
            None => match ended {
                Some(ended) => {
                    self.ended = Some(ended);
                    self.live = None;
                    cx.notify();
                    false
                }
                None => true,
            },
        }
    }

    /// Tests: show `pulled` as if the view had sent it.
    #[cfg(test)]
    pub(crate) fn accept_for_tests(&mut self, pulled: Pulled, cx: &mut Context<Self>) {
        self.accept(pulled, cx);
    }

    /// Tests: an error that has been up for `shown`.
    #[cfg(test)]
    pub(crate) fn set_error_for_tests(&mut self, text: &str, shown: Duration) {
        self.error = Some(PaneError {
            text: text.into(),
            since: Instant::now() - shown,
        });
    }

    /// Tests: whether the view runs.
    #[cfg(test)]
    pub(crate) fn is_live(&self) -> bool {
        self.live.is_some()
    }

    /// Tests: take what the view sent, as the pull loop would.
    #[cfg(test)]
    /// `true` when a picture came.
    pub(crate) fn pull_for_tests(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(live) = self.live.clone() else {
            return false;
        };
        let pulled = pull(&live);
        let picture = matches!(pulled.0, Some(Ok(_)));
        self.accept(pulled, cx);
        picture
    }

    fn retry(&mut self, cx: &mut Context<Self>) {
        self.ended = None;
        self.error = None;
        if let Some(size) = self.size {
            self.start(size, cx);
        }
    }

    fn send(&mut self, input: LiveInput) {
        if let Some(live) = &self.live {
            live.input(input);
        }
    }

    fn navigate(&mut self, nav: LiveNav, cx: &mut Context<Self>) {
        let Some(live) = self.live.clone() else {
            return;
        };
        let task = cx
            .background_executor()
            .spawn(async move { live.navigate(&nav) });
        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.error = result.err().map(PaneError::new);
                cx.notify();
            });
        })
        .detach();
    }

    /// Where the picture is drawn in the pane: the page's shape, as large as
    /// fits.
    fn picture_rect(&self) -> Option<(Bounds<Pixels>, f64, f64)> {
        let stage = self.stage.get()?;
        let shown = self.picture.as_ref()?;
        let (w, h) = (shown.width.max(1.), shown.height.max(1.));
        let scale = (f64::from(stage.size.width.as_f32()) / w)
            .min(f64::from(stage.size.height.as_f32()) / h);
        let size = gpui::size(px((w * scale) as f32), px((h * scale) as f32));
        let origin = gpui::point(
            stage.left() + (stage.size.width - size.width) / 2.,
            stage.top() + (stage.size.height - size.height) / 2.,
        );
        Some((Bounds { origin, size }, w, h))
    }

    fn mouse(
        &mut self,
        event: MouseEvent,
        at: Point<Pixels>,
        button: LiveButton,
        click_count: usize,
        modifiers: &Modifiers,
    ) {
        let Some((rect, width, height)) = self.picture_rect() else {
            return;
        };
        let page = page_point(at, rect, width, height);
        let Pointer::Send((x, y)) = through_slop(&mut self.press, event, at, page) else {
            return;
        };
        self.send(LiveInput::Mouse {
            event,
            x,
            y,
            button,
            buttons: self.buttons,
            click_count: click_count as u32,
            modifiers: modifier_bits(modifiers),
        });
    }

    /// What an ended pane says, and whether it offers to show the tab again.
    pub(crate) fn ended_message(&self) -> Option<(String, bool)> {
        Some(match self.ended.as_ref()? {
            Ended::TabGone => ("This tab closed or left the thread.".to_owned(), false),
            Ended::Stopped => ("The tab is shown somewhere else now.".to_owned(), true),
            Ended::Failed(error) => (error.clone(), true),
        })
    }

    fn toolbar_button(
        id: &'static str,
        label: &'static str,
        path: &'static str,
        cx: &mut Context<Self>,
        click: impl Fn(&mut Self, &mut Context<Self>) + 'static,
    ) -> AnyElement {
        div()
            .id(id)
            .debug_selector(move || id.into())
            .role(Role::Button)
            .aria_label(label)
            .size(px(24.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .justify_center()
            .rounded_md()
            .cursor_pointer()
            .hover(|s| s.bg(content(0.1)))
            .child(icon(path, 14.).text_color(content(0.6)))
            .on_click(cx.listener(move |this, _, _, cx| click(this, cx)))
            .into_any_element()
    }

    fn body(&mut self, cx: &mut Context<Self>) -> AnyElement {
        if let Some((text, retry)) = self.ended_message() {
            return div()
                .debug_selector(|| "browser-pane-ended".into())
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_3()
                .p_6()
                .text_center()
                .text_size(px(13.))
                .text_color(content(0.5))
                .child(text)
                .when(retry, |d| {
                    d.child(
                        crate::settings::widgets::secondary_button(
                            "browser-pane-retry",
                            "Show it here",
                            false,
                            false,
                        )
                        .debug_selector(|| "browser-pane-retry".into())
                        .child("Show it here")
                        .on_click(cx.listener(|this, _, _, cx| this.retry(cx))),
                    )
                })
                .into_any_element();
        }
        let Some(shown) = &self.picture else {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child(crate::right_panel::spinner(
                    "browser-pane-loading",
                    "loader",
                    16.,
                ))
                .into_any_element();
        };
        div()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(
                img(ImageSource::Render(shown.image.clone()))
                    .debug_selector(|| "browser-pane-picture".into())
                    .max_w_full()
                    .max_h_full()
                    .w_full()
                    .h_full()
                    .object_fit(gpui::ObjectFit::Contain),
            )
            .into_any_element()
    }
}

/// Wait up to a second for what the view sends. Blocks: off the UI thread.
fn pull(live: &LivePane) -> Pulled {
    (live.take(), live.ended(), live.take_input_outcome())
}

impl Render for BrowserPane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync_size(window, cx);
        let toolbar = div()
            .h(px(36.))
            .flex_shrink_0()
            .flex()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(content(0.08))
            .child(Self::toolbar_button(
                "browser-pane-back",
                "Back",
                "arrow-left",
                cx,
                |this, cx| this.navigate(LiveNav::Back, cx),
            ))
            .child(Self::toolbar_button(
                "browser-pane-forward",
                "Forward",
                "chevron-right",
                cx,
                |this, cx| this.navigate(LiveNav::Forward, cx),
            ))
            .child(Self::toolbar_button(
                "browser-pane-reload",
                "Reload",
                "rotate-ccw",
                cx,
                |this, cx| this.navigate(LiveNav::Reload, cx),
            ))
            // The tab's address, to read: the agent moves the tab, and the
            // user only clicks and types in the page itself.
            .child(
                div()
                    .id("browser-pane-address")
                    .debug_selector(|| "browser-pane-address".into())
                    .role(Role::Label)
                    .aria_label(SharedString::from(format!("Address {}", self.url)))
                    .min_w_0()
                    .flex_1()
                    .h(px(26.))
                    .flex()
                    .items_center()
                    .rounded_md()
                    .bg(content(0.05))
                    .px_2()
                    .text_size(px(12.))
                    .text_color(content(0.6))
                    .child(div().truncate().child(self.url.clone())),
            );
        let error = self.error.as_ref().map(|error| error.text.clone());
        div()
            .id("browser-pane")
            .debug_selector(|| "browser-pane".into())
            .role(Role::Group)
            .aria_label(SharedString::from(format!("Browser tab {}", self.title())))
            .size_full()
            .min_h_0()
            .flex()
            .flex_col()
            .child(toolbar)
            .when_some(error, |d, error| {
                d.child(
                    div()
                        .debug_selector(|| "browser-pane-error".into())
                        .px_3()
                        .py_1()
                        .text_size(px(12.))
                        .text_color(gpui::rgb(0xf87171))
                        .child(error),
                )
            })
            .child(
                div()
                    .id("browser-pane-stage")
                    .debug_selector(|| "browser-pane-stage".into())
                    .key_context("BrowserPane")
                    .track_focus(&self.focus)
                    .on_action(cx.listener(|this, _: &PageTab, _, _| this.tap("tab", false)))
                    .on_action(cx.listener(|this, _: &PageShiftTab, _, _| this.tap("tab", true)))
                    .on_action(cx.listener(|this, _: &PageEscape, _, _| this.tap("escape", false)))
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .child(self.stage.layout_probe())
                    .child(self.body(cx))
                    .when(self.focus.is_focused(window), |d| {
                        d.child(
                            div()
                                .debug_selector(|| "browser-pane-leave-hint".into())
                                .absolute()
                                .bottom(px(8.))
                                .right(px(8.))
                                .px(px(6.))
                                .py(px(2.))
                                .rounded_md()
                                .bg(gpui::black().opacity(0.55))
                                .text_size(px(11.))
                                .text_color(gpui::white().opacity(0.85))
                                .child(leave_hint(cfg!(target_os = "macos"))),
                        )
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseDownEvent, window, cx| {
                            window.focus(&this.focus, cx);
                            this.press(event, cx);
                        }),
                    )
                    .on_mouse_down(
                        MouseButton::Right,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| this.press(event, cx)),
                    )
                    .on_mouse_down(
                        MouseButton::Middle,
                        cx.listener(|this, event: &MouseDownEvent, _, cx| this.press(event, cx)),
                    )
                    .on_mouse_up(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, _, cx| this.release(event, cx)),
                    )
                    .on_mouse_up(
                        MouseButton::Right,
                        cx.listener(|this, event: &MouseUpEvent, _, cx| this.release(event, cx)),
                    )
                    .on_mouse_up(
                        MouseButton::Middle,
                        cx.listener(|this, event: &MouseUpEvent, _, cx| this.release(event, cx)),
                    )
                    // A button pressed over the page and let go anywhere
                    // else still lets go in the page, or it stays in a drag.
                    .on_mouse_up_out(
                        MouseButton::Left,
                        cx.listener(|this, event: &MouseUpEvent, _, _| this.release_out(event)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Right,
                        cx.listener(|this, event: &MouseUpEvent, _, _| this.release_out(event)),
                    )
                    .on_mouse_up_out(
                        MouseButton::Middle,
                        cx.listener(|this, event: &MouseUpEvent, _, _| this.release_out(event)),
                    )
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, _| {
                        this.mouse(
                            MouseEvent::Moved,
                            event.position,
                            LiveButton::None,
                            0,
                            &event.modifiers,
                        );
                    }))
                    .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                        this.wheel(event);
                        cx.stop_propagation();
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        if this.key(&event.keystroke, true, cx) {
                            cx.stop_propagation();
                        }
                    }))
                    .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                        if this.key(&event.keystroke, false, cx) {
                            cx.stop_propagation();
                        }
                    })),
            )
    }
}

impl BrowserPane {
    fn press(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let Some((button, bit)) = button_of(event.button) else {
            return;
        };
        self.buttons |= bit;
        self.mouse(
            MouseEvent::Pressed,
            event.position,
            button,
            event.click_count.max(1),
            &event.modifiers,
        );
        cx.stop_propagation();
    }

    fn release(&mut self, event: &MouseUpEvent, cx: &mut Context<Self>) {
        self.let_go(event);
        cx.stop_propagation();
    }

    /// A button let go outside the stage: the page lets go of it too, at
    /// the edge of the picture nearest the pointer, when it went down over
    /// the picture. Anywhere else's buttons are not the page's.
    fn release_out(&mut self, event: &MouseUpEvent) {
        if button_of(event.button).is_some_and(|(_, bit)| self.buttons & bit != 0) {
            self.let_go(event);
        }
    }

    fn let_go(&mut self, event: &MouseUpEvent) {
        let Some((button, bit)) = button_of(event.button) else {
            return;
        };
        self.buttons &= !bit;
        self.mouse(
            MouseEvent::Released,
            event.position,
            button,
            event.click_count.max(1),
            &event.modifiers,
        );
        if self.buttons == 0 {
            self.press = None;
        }
    }

    fn wheel(&mut self, event: &ScrollWheelEvent) {
        let Some((rect, width, height)) = self.picture_rect() else {
            return;
        };
        let (x, y) = page_point(event.position, rect, width, height);
        // GPUI's deltas move the content; the page's wheel moves the view.
        let (dx, dy) = match event.delta {
            ScrollDelta::Pixels(delta) => (delta.x.as_f32() as f64, delta.y.as_f32() as f64),
            ScrollDelta::Lines(delta) => (f64::from(delta.x) * 40., f64::from(delta.y) * 40.),
        };
        self.send(LiveInput::Wheel {
            x,
            y,
            delta_x: -dx,
            delta_y: -dy,
            modifiers: modifier_bits(&event.modifiers),
        });
    }

    /// A key a binding of the pane took, down and up, for the page.
    fn tap(&mut self, key: &str, shift: bool) {
        let keystroke = gpui::Keystroke {
            modifiers: Modifiers {
                shift,
                ..Default::default()
            },
            key: key.into(),
            key_char: None,
        };
        for down in [true, false] {
            if let Some(input) = key_input(&keystroke, down) {
                self.send(input);
            }
        }
    }

    /// A key in the pane; `true` when the tab took it.
    fn key(&mut self, keystroke: &gpui::Keystroke, down: bool, cx: &mut Context<Self>) -> bool {
        if app_modifier(&keystroke.modifiers, cfg!(target_os = "macos"))
            && !keystroke.modifiers.alt
            && !keystroke.modifiers.shift
            && keystroke.key == "v"
        {
            if down
                && let Some(text) = cx.read_from_clipboard().and_then(|item| item.text())
                && !text.is_empty()
            {
                self.send(LiveInput::Text { text });
            }
            return true;
        }
        match key_input(keystroke, down) {
            Some(input) => {
                self.send(input);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
#[path = "browser_pane_tests.rs"]
mod live_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn stroke(key: &str, key_char: Option<&str>, control: bool) -> gpui::Keystroke {
        gpui::Keystroke {
            modifiers: Modifiers {
                control,
                ..Default::default()
            },
            key: key.into(),
            key_char: key_char.map(Into::into),
        }
    }

    #[test]
    fn keys_reach_the_tab_as_the_page_expects_them() {
        assert_eq!(
            key_input(&stroke("enter", None, false), true),
            Some(LiveInput::Key {
                event: KeyEvent::KeyDown,
                key: "Enter".into(),
                code: "Enter".into(),
                text: Some("\r".into()),
                key_code: 13,
                modifiers: 0,
                commands: vec![],
            })
        );
        assert_eq!(
            key_input(&stroke("a", None, true), true),
            Some(LiveInput::Key {
                event: KeyEvent::RawKeyDown,
                key: "a".into(),
                code: "KeyA".into(),
                text: None,
                key_code: 65,
                modifiers: 2,
                commands: vec![],
            })
        );
        assert_eq!(
            key_input(&stroke("a", Some("ä"), false), true),
            Some(LiveInput::Text { text: "ä".into() })
        );
        // A layout without ASCII keys types its own characters, whole.
        assert_eq!(
            key_input(&stroke("q", Some("ๆ"), false), true),
            Some(LiveInput::Text { text: "ๆ".into() })
        );
        assert_eq!(key_input(&stroke("a", Some("a"), false), false), None);
        assert_eq!(
            key_input(&stroke("space", Some(" "), false), true),
            Some(LiveInput::Text { text: " ".into() })
        );
        // A shortcut with a modifier the page has no key for stays with Gespan.
        assert_eq!(key_input(&stroke("é", Some("é"), true), true), None);
    }

    /// A key with Gespan's shortcut modifier (Cmd on macOS, Ctrl elsewhere).
    fn app_stroke(key: &str, shift: bool) -> gpui::Keystroke {
        gpui::Keystroke {
            modifiers: Modifiers {
                platform: cfg!(target_os = "macos"),
                control: !cfg!(target_os = "macos"),
                shift,
                ..Default::default()
            },
            key: key.into(),
            key_char: None,
        }
    }

    #[test]
    fn app_shortcuts_stay_with_gespan_and_editing_ones_go_to_the_page() {
        for key in ["w", "t", "n", "1", "9", "k", "b"] {
            assert_eq!(key_input(&app_stroke(key, false), true), None, "{key}");
            assert_eq!(key_input(&app_stroke(key, false), false), None, "{key}");
        }
        assert_eq!(key_input(&app_stroke(",", false), true), None);
        for (key, shift) in [
            ("a", false),
            ("c", false),
            ("v", false),
            ("x", false),
            ("z", false),
            ("z", true),
        ] {
            let Some(LiveInput::Key {
                event, code, text, ..
            }) = key_input(&app_stroke(key, shift), true)
            else {
                panic!("{key} shift={shift} goes to the page");
            };
            assert_eq!(event, KeyEvent::RawKeyDown);
            assert_eq!(code, format!("Key{}", key.to_ascii_uppercase()));
            assert_eq!(text, None);
        }
        // Shift+Cmd+A or C are Gespan's (or nobody's), not the page's.
        assert_eq!(key_input(&app_stroke("a", true), true), None);
        assert_eq!(key_input(&app_stroke("c", true), true), None);
    }

    /// `key` with Cmd on macOS (`macos`) or Ctrl elsewhere, Shift if `shift`.
    fn shortcut_on(key: &str, shift: bool, macos: bool) -> gpui::Keystroke {
        gpui::Keystroke {
            modifiers: Modifiers {
                platform: macos,
                control: !macos,
                shift,
                ..Default::default()
            },
            key: key.into(),
            key_char: None,
        }
    }

    fn commands_of(input: Option<LiveInput>) -> Vec<String> {
        match input {
            Some(LiveInput::Key { commands, .. }) => commands,
            other => panic!("not a key: {other:?}"),
        }
    }

    #[test]
    fn on_macos_the_editing_shortcuts_name_their_edit_for_chromium() {
        for (key, shift, edit) in [
            ("a", false, "selectAll"),
            ("c", false, "copy"),
            ("x", false, "cut"),
            ("z", false, "undo"),
            ("z", true, "redo"),
        ] {
            let stroke = shortcut_on(key, shift, true);
            let Some(LiveInput::Key {
                event,
                modifiers,
                commands,
                ..
            }) = key_input_on(&stroke, true, true)
            else {
                panic!("Cmd+{key} goes to the page");
            };
            assert_eq!(event, KeyEvent::RawKeyDown);
            assert_eq!(modifiers, 4 | if shift { 8 } else { 0 }, "Meta (and Shift)");
            assert_eq!(commands, [edit], "Cmd+{key} shift={shift}");
            // The edit runs once, with the key down.
            assert!(commands_of(key_input_on(&stroke, false, true)).is_empty());
            // Elsewhere Ctrl+key does the edit by itself.
            let ctrl = shortcut_on(key, shift, false);
            assert!(commands_of(key_input_on(&ctrl, true, false)).is_empty());
        }
        // Paste types the clipboard's text instead ([`BrowserPane::key`]).
        assert!(commands_of(key_input_on(&shortcut_on("v", false, true), true, true)).is_empty());
        // Ctrl+A on macOS is the line start in a field, not select all.
        let ctrl_a = shortcut_on("a", false, false);
        assert!(commands_of(key_input_on(&ctrl_a, true, true)).is_empty());
        // Gespan's own shortcuts still never reach the page.
        assert_eq!(
            key_input_on(&shortcut_on("w", false, true), true, true),
            None
        );
    }

    /// Ctrl+Alt with `key_char`, as Windows reports AltGr.
    fn alt_gr(key: &str, key_char: Option<&str>) -> gpui::Keystroke {
        gpui::Keystroke {
            modifiers: Modifiers {
                control: true,
                alt: true,
                ..Default::default()
            },
            key: key.into(),
            key_char: key_char.map(Into::into),
        }
    }

    #[test]
    fn alt_gr_characters_are_typed_off_macos() {
        // German and other layouts: AltGr+Q is @, +7 {, +8 [, +ß \, +E €.
        for (key, typed) in [
            ("q", "@"),
            ("7", "{"),
            ("0", "}"),
            ("8", "["),
            ("9", "]"),
            ("ß", "\\"),
            ("e", "€"),
        ] {
            let stroke = alt_gr(key, Some(typed));
            assert_eq!(
                key_input_on(&stroke, true, false),
                Some(LiveInput::Text { text: typed.into() }),
                "AltGr+{key}"
            );
            // Typed once, on the way down.
            assert_eq!(key_input_on(&stroke, false, false), None);
            // macOS has no AltGr: Ctrl+Option stays as it was.
            assert_eq!(key_input_on(&stroke, true, true), None, "macOS {key}");
        }
        // Ctrl+Alt that types nothing, or only the key itself, is a
        // shortcut, not text.
        assert_eq!(key_input_on(&alt_gr("t", None), true, false), None);
        assert_eq!(key_input_on(&alt_gr("t", Some("t")), true, false), None);
        assert_eq!(key_input_on(&alt_gr("t", Some("T")), true, false), None);
        assert_eq!(
            key_input_on(&alt_gr("t", Some("\u{14}")), true, false),
            None
        );
        // Ctrl alone keeps its shortcuts (Ctrl+A goes to the page as a key).
        assert!(matches!(
            key_input_on(&shortcut_on("a", false, false), true, false),
            Some(LiveInput::Key { .. })
        ));
    }

    #[test]
    fn f6_leaves_the_page_on_every_platform_and_cmd_escape_on_macos() {
        // The table each platform compiles, whichever this one is.
        assert_eq!(leave_keys(true), ["f6", "cmd-escape"]);
        assert_eq!(leave_keys(false), ["f6"]);
        for macos in [true, false] {
            assert!(
                !leave_keys(macos).iter().any(|key| key.contains("ctrl")),
                "Ctrl+Escape is the system's off macOS"
            );
            let bindings: Vec<_> = leave_keys(macos)
                .iter()
                .map(|key| gpui::KeyBinding::new(key, LeaveBrowser, Some("BrowserPane")))
                .collect();
            assert!(bindings.iter().any(|binding| {
                binding.keystrokes().len() == 1 && binding.keystrokes()[0].key() == "f6"
            }));
        }
        assert_eq!(leave_hint(true), "F6 or ⌘Esc to leave");
        assert_eq!(leave_hint(false), "F6 to leave");
        // F6 has nothing for the page even without its binding.
        let f6 = gpui::Keystroke {
            modifiers: Modifiers::default(),
            key: "f6".into(),
            key_char: None,
        };
        assert_eq!(key_input(&f6, true), None);
        assert_eq!(key_input(&f6, false), None);
    }

    #[test]
    fn a_point_on_the_picture_is_a_point_on_the_page() {
        let rect = Bounds {
            origin: gpui::point(px(10.), px(20.)),
            size: gpui::size(px(200.), px(100.)),
        };
        assert_eq!(
            page_point(gpui::point(px(110.), px(70.)), rect, 1000., 500.),
            (500., 250.)
        );
        // Outside the picture (the bars beside it) clamps to its edge.
        assert_eq!(
            page_point(gpui::point(px(0.), px(500.)), rect, 1000., 500.),
            (0., 500.)
        );
        // Rounded to a tenth of a pixel.
        assert_eq!(
            page_point(gpui::point(px(10.3), px(20.)), rect, 1000., 500.),
            (1.5, 0.)
        );
    }

    #[test]
    fn a_shaking_click_stays_a_click_and_a_longer_move_drags() {
        let at = |x: f32, y: f32| gpui::point(px(x), px(y));
        let mut press = None;
        let down = through_slop(&mut press, MouseEvent::Pressed, at(100., 100.), (50., 50.));
        assert_eq!(down, Pointer::Send((50., 50.)));
        // Chromium drags a link from 4 px: a hand moves that much clicking.
        let shake = through_slop(&mut press, MouseEvent::Moved, at(104., 103.), (52., 51.));
        assert_eq!(shake, Pointer::Hold);
        let up = through_slop(&mut press, MouseEvent::Released, at(105., 103.), (52., 51.));
        assert_eq!(up, Pointer::Send((50., 50.)), "released where it pressed");
        assert!(press.is_none());

        through_slop(&mut press, MouseEvent::Pressed, at(100., 100.), (50., 50.));
        let drag = through_slop(
            &mut press,
            MouseEvent::Moved,
            at(100. + CLICK_SLOP, 100.),
            (54., 50.),
        );
        assert_eq!(drag, Pointer::Send((54., 50.)));
        // Once a drag, the rest of it goes as it is, back near the start too.
        let back = through_slop(&mut press, MouseEvent::Moved, at(101., 100.), (50.5, 50.));
        assert_eq!(back, Pointer::Send((50.5, 50.)));
        let up = through_slop(
            &mut press,
            MouseEvent::Released,
            at(101., 100.),
            (50.5, 50.),
        );
        assert_eq!(up, Pointer::Send((50.5, 50.)));
        // A hover with no button is never held.
        let hover = through_slop(&mut press, MouseEvent::Moved, at(1., 1.), (0.5, 0.5));
        assert_eq!(hover, Pointer::Send((0.5, 0.5)));
    }
}
