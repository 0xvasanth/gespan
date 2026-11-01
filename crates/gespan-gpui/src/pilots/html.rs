//! Generated HTML lives in an opaque sandbox with no app bridge or network.
use crate::{right_panel::RED_400, settings::widgets::secondary_button, theme::*};
use gpui::{AnyElement, Context, Window, canvas, div, prelude::*, px};
#[cfg(target_os = "macos")]
use objc2::{
    DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, rc::Retained,
    runtime::ProtocolObject,
};
#[cfg(target_os = "macos")]
use objc2_app_kit::{NSView, NSWindowOrderingMode};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSObject, NSObjectProtocol};
#[cfg(target_os = "macos")]
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
#[cfg(target_os = "macos")]
use objc2_web_kit::{
    WKNavigationAction, WKNavigationActionPolicy, WKNavigationDelegate, WKNavigationType,
    WKWebView, WKWebViewConfiguration, WKWebsiteDataStore,
};
#[cfg(target_os = "macos")]
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::sync::Arc;
#[cfg(target_os = "macos")]
use std::{cell::RefCell, rc::Rc};
#[cfg(target_os = "macos")]
define_class!(
    #[unsafe(super(NSView))]
    #[name = "GespanArtifactClip"]
    #[thread_kind = MainThreadOnly]
    #[ivars = std::cell::Cell<bool>]
    struct ArtifactClip;
    unsafe impl NSObjectProtocol for ArtifactClip {}
    impl ArtifactClip {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }

        #[unsafe(method_id(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> Option<Retained<NSView>> {
            if !self.ivars().get() {
                None
            } else {
                unsafe { msg_send![super(self), hitTest: point] }
            }
        }
    }
);
#[cfg(target_os = "macos")]
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "GespanArtifactNavigation"]
    #[thread_kind=MainThreadOnly]
    #[ivars=gpui::AsyncApp]
    struct NavigationGuard;
    unsafe impl NSObjectProtocol for NavigationGuard {}
    unsafe impl WKNavigationDelegate for NavigationGuard {
        #[unsafe(method(webView:decidePolicyForNavigationAction:decisionHandler:))]
        unsafe fn decide(
            &self,
            _: &WKWebView,
            action: &WKNavigationAction,
            reply: &block2::DynBlock<dyn Fn(WKNavigationActionPolicy)>,
        ) {
            let url = unsafe { action.request().URL() }.and_then(|url| url.absoluteString());
            if unsafe { action.navigationType() } == WKNavigationType::LinkActivated
                && let Some(url) = url.as_ref()
                && (url.to_string().starts_with("https://")
                    || url.to_string().starts_with("http://"))
            {
                let url = url.to_string();
                self.ivars().clone().update(|cx| cx.open_url(&url));
            }
            let allowed = url.is_none_or(|url| {
                let s = url.to_string();
                s.starts_with("about:blank") || s.starts_with("about:srcdoc")
            });
            reply.call((if allowed {
                WKNavigationActionPolicy::Allow
            } else {
                WKNavigationActionPolicy::Cancel
            },));
        }
    }
);
#[cfg(target_os = "macos")]
struct Hosted {
    clip: Retained<ArtifactClip>,
    web: Retained<WKWebView>,
    _guard: Retained<NavigationGuard>,
    markup: Arc<str>,
}
struct Markup {
    ink: gpui::Hsla,
    accent: gpui::Hsla,
    html: Arc<str>,
}
pub(super) struct Html {
    body: String,
    interactive: bool,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    open_error: Option<String>,
    markup: Option<Markup>,
    #[cfg(target_os = "macos")]
    view: Rc<RefCell<Option<Hosted>>>,
    #[cfg(not(target_os = "macos"))]
    inline: super::html_inline::Inline,
}
fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
/// Only the page itself may load in the outer web view. Links and scripted
/// redirects go nowhere, and the opaque iframe keeps the page from the app.
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub(super) fn allows_navigation(url: &str) -> bool {
    url.starts_with("about:blank")
        || url.starts_with("about:srcdoc")
        || url.starts_with("data:text/html")
}
/// The page every system web view hosts. Where none can be embedded it opens
/// in the browser, and the same opaque iframe and policy apply there.
fn sandbox(body: &str) -> String {
    let color = |value: gpui::Hsla| {
        let value: gpui::Rgba = value.into();
        format!(
            "#{:02x}{:02x}{:02x}",
            (value.r * 255.) as u8,
            (value.g * 255.) as u8,
            (value.b * 255.) as u8
        )
    };
    let ink: gpui::Rgba = crate::theme::content(1.).into();
    let foreground = color(crate::theme::content(1.));
    let accent = color(crate::theme::accent_color());
    let light = ink.r * 0.2126 + ink.g * 0.7152 + ink.b * 0.0722 < 0.5;
    let background = if light { "#fafafa" } else { "#141414" };
    let muted = if light { "#737373" } else { "#969696" };
    let surface = if light { "#eeeeee" } else { "#222222" };
    let border = if light { "#d4d4d4" } else { "#383838" };
    let source = format!(
        "<!doctype html><meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; script-src 'unsafe-inline' https:; style-src 'unsafe-inline' https:; img-src data: blob: https:; font-src data: https:; media-src data: blob: https:; worker-src blob:; connect-src 'none'; frame-src 'none'; base-uri 'none'; form-action 'none'\"><meta name=viewport content=\"width=device-width,initial-scale=1\"><style>:root{{--background:{background};--foreground:{foreground};--muted:{muted};--surface:{surface};--border:{border};--accent:{accent};--link:{accent};--positive:#10b981;--negative:#f43f5e;--chart-1:{accent};--chart-2:#10b981;--chart-3:#f59e0b;--chart-4:#f43f5e;--chart-5:#a78bfa;--chart-6:#22d3ee;--font-sans:system-ui;--font-mono:ui-monospace,monospace;--radius:10px;color-scheme:{scheme}}}body{{font:14px var(--font-sans);margin:20px;color:var(--foreground);background:var(--background)}}a{{color:var(--link)}}</style>{body}",
        scheme = if light { "light" } else { "dark" }
    );
    format!(
        "<!doctype html><style>html,body,iframe{{margin:0;border:0;width:100%;height:100%;overflow:hidden;background:{background}}}</style><iframe sandbox=\"allow-scripts\" referrerpolicy=\"no-referrer\" srcdoc=\"{}\"></iframe>",
        escape(&source)
    )
}
impl Html {
    pub(super) fn new(body: String) -> Self {
        Self {
            body,
            interactive: true,
            open_error: None,
            markup: None,
            #[cfg(target_os = "macos")]
            view: Rc::new(RefCell::new(None)),
            #[cfg(not(target_os = "macos"))]
            inline: Default::default(),
        }
    }
    fn markup(&mut self) -> Arc<str> {
        let ink = crate::theme::content(1.);
        let accent = crate::theme::accent_color();
        if self
            .markup
            .as_ref()
            .is_none_or(|m| m.ink != ink || m.accent != accent)
        {
            self.markup = Some(Markup {
                ink,
                accent,
                html: sandbox(&self.body).into(),
            });
        }
        self.markup.as_ref().unwrap().html.clone()
    }
    /// Saves the sandboxed page and hands it to the user's browser.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn open_in_browser(&mut self, cx: &mut Context<Self>) {
        self.open_error = None;
        let page = sandbox(&self.body);
        let saved = cx
            .background_executor()
            .spawn(async move { gespan_core::files::save_browser_page(&page) });
        cx.spawn(async move |this, cx| {
            let saved = saved.await;
            let _ = this.update(cx, |this, cx| {
                match saved {
                    Ok(url) => cx.open_url(&url),
                    Err(error) => this.open_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    /// Without a hosted web view the panel opens in the browser, and its
    /// source stays readable here.
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn browser_fallback(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .size_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_4()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        secondary_button("html-open-browser", "Open in browser", false, false)
                            .debug_selector(|| "html-open-browser".into())
                            .child("Open in browser")
                            .on_click(cx.listener(|this, _, _, cx| this.open_in_browser(cx))),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(content(0.45))
                            .child("Panels can't be shown inside the app on this system."),
                    ),
            )
            .when_some(self.open_error.clone(), |d, error| {
                d.child(
                    div()
                        .text_size(px(12.))
                        .text_color(gpui::rgb(RED_400))
                        .child(format!("Could not open this panel: {error}")),
                )
            })
            .child(
                div()
                    .id("html-source")
                    .min_h_0()
                    .flex_1()
                    .overflow_y_scroll()
                    .text_size(px(12.))
                    .text_color(content(0.6))
                    .child(self.body.clone()),
            )
            .into_any_element()
    }
    pub(super) fn set_interactive(&mut self, interactive: bool) {
        if self.interactive != interactive {
            self.interactive = interactive;
        }
    }
    pub(super) fn hide(&self, cx: &gpui::App) {
        #[cfg(target_os = "macos")]
        {
            let _ = cx;
            if let Some(view) = self.view.borrow().as_ref() {
                view.clip.setHidden(true);
            }
        }
        #[cfg(not(target_os = "macos"))]
        self.inline.hide(cx);
    }
}
impl gpui::Render for Html {
    #[cfg_attr(target_os = "macos", allow(unused_variables))]
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        #[cfg(target_os = "macos")]
        {
            let view = self.view.clone();
            let html = self.markup();
            let app = cx.to_async();
            let interactive = self.interactive;
            canvas(
                |_, _, _| {},
                move |bounds, _, window, _| {
                    let Ok(handle) = HasWindowHandle::window_handle(window) else {
                        return;
                    };
                    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
                        return;
                    };
                    // GPUI keeps its AppKit content view alive throughout paint.
                    let parent = unsafe { &*handle.ns_view.as_ptr().cast::<NSView>() };
                    // AppKit frames are in window points; GPUI's are divided
                    // by the interface zoom.
                    let zoom = window.content_zoom();
                    let clipped = bounds
                        .intersect(&window.content_mask().bounds)
                        .map(|v| v * zoom);
                    let bounds = bounds.map(|v| v * zoom);
                    let mut view = view.borrow_mut();
                    if clipped.size.width <= gpui::px(0.) || clipped.size.height <= gpui::px(0.) {
                        if let Some(view) = view.as_ref() {
                            view.clip.setHidden(true);
                        }
                        return;
                    }
                    let x = f64::from(clipped.origin.x);
                    let y = f64::from(clipped.origin.y);
                    let clip_width = f64::from(clipped.size.width);
                    let clip_height = f64::from(clipped.size.height);
                    let width = f64::from(bounds.size.width);
                    let height = f64::from(bounds.size.height);
                    let y = if parent.isFlipped() {
                        y
                    } else {
                        parent.bounds().size.height - y - clip_height
                    };
                    let rect =
                        NSRect::new(NSPoint::new(x, y), NSSize::new(clip_width, clip_height));
                    // Keep the document at its full layout size while its
                    // native host follows GPUI's moving content mask.
                    let web_rect = NSRect::new(
                        NSPoint::new(
                            f64::from(bounds.left() - clipped.left()),
                            f64::from(bounds.top() - clipped.top()),
                        ),
                        NSSize::new(width, height),
                    );
                    if view.is_none() {
                        let Some(mtm) = MainThreadMarker::new() else {
                            return;
                        };
                        // No message handlers, file base URL,
                        // app credentials or persistent website data are installed.
                        let hosted = unsafe {
                            let clip = ArtifactClip::alloc(mtm)
                                .set_ivars(std::cell::Cell::new(interactive));
                            let clip: Retained<ArtifactClip> =
                                msg_send![super(clip), initWithFrame: rect];
                            clip.setWantsLayer(true);
                            clip.setClipsToBounds(true);
                            let config = WKWebViewConfiguration::new(mtm);
                            config
                                .preferences()
                                .setJavaScriptCanOpenWindowsAutomatically(false);
                            config.setWebsiteDataStore(
                                &WKWebsiteDataStore::nonPersistentDataStore(mtm),
                            );
                            let web = WKWebView::initWithFrame_configuration(
                                WKWebView::alloc(mtm),
                                web_rect,
                                &config,
                            );
                            let guard = NavigationGuard::alloc(mtm).set_ivars(app.clone());
                            let guard: Retained<NavigationGuard> = msg_send![super(guard), init];
                            web.setNavigationDelegate(Some(ProtocolObject::from_ref(&*guard)));
                            web.loadHTMLString_baseURL(&NSString::from_str(&html), None);
                            clip.addSubview(&web);
                            Hosted {
                                clip,
                                web,
                                _guard: guard,
                                markup: html.clone(),
                            }
                        };
                        *view = Some(hosted);
                    }
                    if let Some(view) = view.as_mut() {
                        if !Arc::ptr_eq(&view.markup, &html) {
                            unsafe {
                                view.web
                                    .loadHTMLString_baseURL(&NSString::from_str(&html), None);
                            }
                            view.markup = html.clone();
                        }
                        parent.addSubview_positioned_relativeTo(
                            &view.clip,
                            NSWindowOrderingMode::Above,
                            None,
                        );
                        view.clip.ivars().set(interactive);
                        view.clip.setFrame(rect);
                        view.web.setFrame(web_rect);
                        unsafe { view.web.setPageZoom(f64::from(zoom)) };
                        view.clip.setHidden(false);
                    }
                },
            )
            .size_full()
            .into_any_element()
        }
        #[cfg(not(target_os = "macos"))]
        {
            if self.inline.failed() || !super::html_inline::can_host(window) {
                return self.browser_fallback(cx);
            }
            super::html_inline::start_events(cx);
            let inline = self.inline.clone();
            let html = self.markup();
            let panel = cx.weak_entity();
            let interactive = self.interactive;
            canvas(
                |_, _, _| {},
                move |bounds, _, window, cx| {
                    // Web view frames are in window points; GPUI's are divided
                    // by the interface zoom.
                    let zoom = window.content_zoom();
                    let clipped = bounds
                        .intersect(&window.content_mask().bounds)
                        .map(|v| v * zoom);
                    if interactive {
                        inline.show(clipped, html.clone(), window, panel.clone(), cx);
                    } else {
                        inline.hide(cx);
                    }
                },
            )
            .size_full()
            .into_any_element()
        }
    }
}
#[cfg(target_os = "macos")]
impl Drop for Html {
    fn drop(&mut self) {
        if let Some(view) = self.view.borrow_mut().take() {
            view.clip.removeFromSuperview();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn generated_markup_cannot_escape_the_opaque_iframe() {
        let html = sandbox("\"><script>window.top.postMessage('x','*')</script>");
        assert!(html.contains("sandbox=\"allow-scripts\""));
        assert!(!html.contains("allow-same-origin"));
        assert!(html.contains("connect-src 'none'"));
        assert!(html.contains("&lt;script&gt;"));
    }

    #[test]
    fn only_the_page_itself_loads_in_the_outer_web_view() {
        for url in [
            "about:blank",
            "about:srcdoc",
            "data:text/html;charset=utf-8;base64,PGh0bWw+",
        ] {
            assert!(allows_navigation(url), "{url}");
        }
        for url in [
            "https://example.com/",
            "http://localhost:3000/",
            "file:///etc/passwd",
            "javascript:alert(1)",
            "data:image/svg+xml,<svg/>",
            "",
        ] {
            assert!(!allows_navigation(url), "{url}");
        }
    }

    struct Fallback(gpui::Entity<Html>);
    impl gpui::Render for Fallback {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            self.0.update(cx, |html, cx| html.browser_fallback(cx))
        }
    }

    #[gpui::test]
    fn opening_in_the_browser_keeps_the_page_in_its_sandbox(cx: &mut gpui::TestAppContext) {
        let window = cx.add_window(|_, cx| {
            let html = cx.new(|_| Html::new("<script>top.location='x'</script>".into()));
            Fallback(html)
        });
        let mut visual = gpui::VisualTestContext::from_window(window.into(), cx);
        visual.run_until_parked();
        let open = visual.debug_bounds("html-open-browser").unwrap().center();
        visual.simulate_click(open, Default::default());
        visual.run_until_parked();

        let opened = cx.opened_url().expect("the browser was asked to open it");
        let file = url::Url::parse(&opened).unwrap().to_file_path().unwrap();
        let page = std::fs::read_to_string(&file).unwrap();
        std::fs::remove_file(&file).unwrap();
        assert!(page.contains("sandbox=\"allow-scripts\""));
        assert!(!page.contains("allow-same-origin"));
        assert!(page.contains("connect-src 'none'"));
        assert!(page.contains("&lt;script&gt;"));
        assert!(!page.contains("<script>"));
    }
}
