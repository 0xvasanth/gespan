use super::*;
use gpui::{AnyElement, Asset, Image, ImageFormat, Window, div, img, prelude::*, px};
use std::sync::Arc;

struct Avatar;
impl Asset for Avatar {
    type Source = String;
    type Output = Result<Arc<Image>, String>;
    #[expect(
        clippy::manual_async_fn,
        reason = "An async method captures the non-Send App reference."
    )]
    fn load(
        url: String,
        _: &mut App,
    ) -> impl std::future::Future<Output = Self::Output> + Send + 'static {
        async move {
            #[cfg(test)]
            let fetched = test_avatars::get(&url);
            #[cfg(not(test))]
            let fetched = model::avatar_png(&url);
            fetched.map(|bytes| Arc::new(Image::from_bytes(ImageFormat::Png, bytes)))
        }
    }
}

/// Tests never reach the network for a picture: they load from here.
#[cfg(test)]
pub(crate) mod test_avatars {
    use std::sync::Mutex;
    static AVATARS: Mutex<Vec<(String, Vec<u8>)>> = Mutex::new(Vec::new());
    /// A 1×1 PNG.
    pub(crate) const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F,
        0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00,
        0x01, 0x00, 0x00, 0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49,
        0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
    ];
    pub(crate) fn serve(url: &str) {
        AVATARS.lock().unwrap().push((url.to_owned(), PNG.to_vec()));
    }
    pub(crate) fn get(url: &str) -> Result<Vec<u8>, String> {
        AVATARS
            .lock()
            .unwrap()
            .iter()
            .find(|(u, _)| u == url)
            .map(|(_, bytes)| bytes.clone())
            .ok_or_else(|| "Avatar unavailable".into())
    }
}

/// The picture to load for an account: the host's own URL, or on GitHub
/// the account's picture by its login (an app's by its name).
pub(super) fn source(provider: &str, name: &str, url: &str) -> Option<String> {
    let url = url.trim();
    if !url.is_empty() {
        return Some(url.to_owned());
    }
    // The avatars host serves users and apps alike by login; github.com's
    // `<login>.png` redirect fails for apps such as review bots.
    let login = name
        .trim()
        .trim_start_matches("app/")
        .trim_end_matches("[bot]");
    let safe = !login.is_empty()
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    (provider == "github" && safe)
        .then(|| format!("https://avatars.githubusercontent.com/{login}?s=64"))
}

/// The account's picture once it has loaded; None while it loads, or when
/// there is none.
pub(super) fn loaded(
    provider: &str,
    name: &str,
    url: &str,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<Image>> {
    let url = source(provider, name, url)?;
    window.use_asset::<Avatar>(&url, cx).and_then(Result::ok)
}

pub(crate) fn image(
    provider: &str,
    name: &str,
    url: &str,
    side: f32,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let image = loaded(provider, name, url, window, cx);
    let initial = name
        .trim()
        .chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string();
    if let Some(image) = image {
        img(image)
            .size(px(side))
            .rounded_full()
            .object_fit(gpui::ObjectFit::Cover)
            .into_any_element()
    } else {
        div()
            .size(px(side))
            .rounded_full()
            .bg(content(0.12))
            .flex()
            .items_center()
            .justify_center()
            .text_size(px((side * 0.45).max(9.)))
            .text_color(content(0.55))
            .child(initial)
            .into_any_element()
    }
}

pub(super) fn person(
    provider: &str,
    name: &str,
    url: &str,
    side: f32,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let avatar = image(provider, name, url, side, window, cx);
    div()
        .min_w_0()
        .flex()
        .items_center()
        .gap(px(6.))
        .child(avatar)
        .child(div().min_w_0().truncate().child(name.to_string()))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_picture_comes_from_the_host_or_on_github_from_the_login() {
        use super::source;
        assert_eq!(
            source("gitlab", "dana", " https://gitlab.com/uploads/dana.png "),
            Some("https://gitlab.com/uploads/dana.png".into())
        );
        assert_eq!(
            source("github", " dana ", ""),
            Some("https://avatars.githubusercontent.com/dana?s=64".into())
        );
        assert_eq!(
            source("github", "app/dependabot", ""),
            Some("https://avatars.githubusercontent.com/dependabot?s=64".into()),
            "an app by its name"
        );
        assert_eq!(
            source("github", "cubic-dev-ai[bot]", ""),
            Some("https://avatars.githubusercontent.com/cubic-dev-ai?s=64".into()),
            "a bot by its app's name"
        );
        assert_eq!(
            source("github", "../x?y", ""),
            None,
            "nothing that isn't a login goes in the URL"
        );
        assert_eq!(source("github", "  ", ""), None);
        assert_eq!(source("gitlab", "dana", ""), None);
        assert_eq!(source("azuredevops", "dana@acme.com", ""), None);
    }
}
