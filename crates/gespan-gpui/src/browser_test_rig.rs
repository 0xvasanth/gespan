//! Tests: a browser module of the test's own on a fake browser whose tabs
//! stream pictures, and agent tabs opened in it for a thread.
use gespan_browser::testing::{FakeCdp, Received, Response, default_response};
use gespan_core::browser::{
    AccessMode, BrowserChoice, BrowserHost, BrowserMode, BrowserSettings, ConnectionState, TabId,
    session_key,
};
use serde_json::json;
use std::sync::{Arc, Mutex, atomic::AtomicU64};
use std::time::{Duration, Instant};

/// The fake browser and the host attached to it.
pub(crate) struct Rig {
    pub(crate) server: FakeCdp,
    pub(crate) host: Arc<BrowserHost>,
    /// The page's size, as the pane last set it.
    viewport: Arc<Mutex<(i64, i64)>>,
}

/// A moving picture of the page at `size` (`n` tells it from the others).
fn frame_event(n: i64, size: (i64, i64), session: Option<&str>) -> gespan_browser::testing::Event {
    let (w, h) = size;
    gespan_browser::testing::Event::new(
        "Page.screencastFrame",
        json!({
            "data": picture(image::ImageFormat::Jpeg, w as u32 / 10, h as u32 / 10, [200, (n * 40 % 256) as u8, 20]),
            "sessionId": n,
            "metadata": {
                "deviceWidth": w, "deviceHeight": h,
                "offsetTop": 0, "pageScaleFactor": 1
            }
        }),
        session,
    )
}

/// One `width`x`height` picture of `rgb` in `format`, base64.
fn picture(format: image::ImageFormat, width: u32, height: u32, rgb: [u8; 3]) -> String {
    use base64::Engine as _;
    let mut bytes = Vec::new();
    image::RgbImage::from_pixel(width, height, image::Rgb(rgb))
        .write_to(&mut std::io::Cursor::new(&mut bytes), format)
        .unwrap();
    base64::engine::general_purpose::STANDARD.encode(&bytes)
}

/// A browser whose tabs answer a screencast with one picture at the size
/// the pane asked for, and a still picture after.
fn screencast_browser(viewport: Arc<Mutex<(i64, i64)>>) -> FakeCdp {
    let counter = Arc::new(AtomicU64::new(1));
    // Moving pictures come as JPEG, the still one as PNG, as Chrome sends;
    // a tenth of the page's size, its shape (the view drops misfits).
    FakeCdp::start_with(move |received: &Received| match received.method.as_str() {
        "Emulation.setDeviceMetricsOverride" => {
            *viewport.lock().unwrap() = (
                received.params["width"].as_i64().unwrap_or(0),
                received.params["height"].as_i64().unwrap_or(0),
            );
            Response::Result(json!({}))
        }
        "Page.getLayoutMetrics" => {
            let (w, h) = *viewport.lock().unwrap();
            Response::Result(json!({
                "cssContentSize": { "x": 0, "y": 0, "width": w, "height": h },
                "cssLayoutViewport": { "pageX": 0, "pageY": 0, "clientWidth": w, "clientHeight": h },
                "cssVisualViewport": { "pageX": 0, "pageY": 0, "clientWidth": w, "clientHeight": h, "scale": 1 },
            }))
        }
        "Page.startScreencast" => Response::ResultThen(
            json!({}),
            vec![frame_event(
                1,
                *viewport.lock().unwrap(),
                received.session.as_deref(),
            )],
        ),
        "Page.captureScreenshot" => {
            let (w, h) = *viewport.lock().unwrap();
            let still = picture(
                image::ImageFormat::Png,
                w as u32 / 10,
                h as u32 / 10,
                [1, 2, 3],
            );
            Response::Result(json!({ "data": still }))
        }
        _ => default_response(received, &counter),
    })
}

impl Rig {
    /// Connected, with the switch on and "Always on".
    pub(crate) fn start() -> Rig {
        let viewport = Arc::new(Mutex::new((800i64, 600i64)));
        let server = screencast_browser(viewport.clone());
        let host = BrowserHost::for_tests(BrowserSettings {
            browser: BrowserChoice::UserDataDir {
                path: server.profile_dir().display().to_string(),
            },
            mode: BrowserMode::Attach,
            access: AccessMode::Always,
            ..BrowserSettings::default()
        });
        host.set_toggle(true);
        let deadline = Instant::now() + Duration::from_secs(5);
        while host.connection().state != ConnectionState::Connected {
            assert!(Instant::now() < deadline, "{:?}", host.connection());
            std::thread::sleep(Duration::from_millis(10));
        }
        Rig {
            server,
            host,
            viewport,
        }
    }

    /// The Gespan browser (Managed) on the fake, with the switch off and
    /// nothing started: what a thread link meets in the app by default.
    pub(crate) fn start_managed() -> Rig {
        let viewport = Arc::new(Mutex::new((800i64, 600i64)));
        let server = screencast_browser(viewport.clone());
        let host = BrowserHost::for_tests(BrowserSettings {
            mode: BrowserMode::Managed,
            access: AccessMode::Always,
            ..BrowserSettings::default()
        });
        host.set_profile_dir(server.profile_dir());
        Rig {
            server,
            host,
            viewport,
        }
    }

    /// The page moved: the browser sends a new picture of the tab it last
    /// started a screencast in.
    pub(crate) fn push_frame(&self, n: i64) {
        let session = self
            .server
            .received()
            .iter()
            .rev()
            .find(|r| r.method == "Page.startScreencast")
            .and_then(|r| r.session.clone());
        let size = *self.viewport.lock().unwrap();
        let event = frame_event(n, size, session.as_deref());
        self.server
            .emit(&event.method, event.params, event.session.as_deref());
    }

    /// A tab of `session`, open and ready to show.
    pub(crate) fn open_tab(&self, session: i64) -> TabId {
        self.host
            .new_tab(&session_key(session), gespan_core::browser::WINDOW, None)
            .expect("the tab opens")
    }

    /// Whether the browser got `method` with params `matches` takes,
    /// within five seconds.
    pub(crate) fn received(
        &self,
        method: &'static str,
        matches: impl Fn(&serde_json::Value) -> bool,
    ) -> bool {
        self.server.wait_for(
            |all| all.iter().any(|r| r.method == method && matches(&r.params)),
            Duration::from_secs(5),
        )
    }
}
