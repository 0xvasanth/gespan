//! LocalMedia.tsx: a reply's local image or video, shown in place of its link.
use super::*;
use gpui::{AnyElement, ObjectFit, StyledImage};

impl TranscriptView {
    pub(super) fn local_media(&self, href: &str, label: &str, ix: usize) -> AnyElement {
        let href = href.strip_prefix("file://").unwrap_or(href);
        let (path, _) =
            crate::file_editor::file_location(href, self.file_link_base.as_deref().unwrap_or(""));
        let path = std::path::PathBuf::from(path);
        let name: SharedString = if label.is_empty() {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| href.to_owned())
                .into()
        } else {
            label.to_owned().into()
        };
        if !path.is_absolute() {
            return fallback(name);
        }
        let video = gespan_core::files::document::is_video_path(&path);
        // One fixed 16:9 box from loading to drawn, so loading never moves the chat.
        let frame = div()
            .id(("local-media", ix))
            .debug_selector(move || if video { "local-video" } else { "local-image" }.into())
            .my_2()
            .w(px(320.))
            .max_w_full()
            .h(px(180.))
            .overflow_hidden()
            .rounded(px(12.))
            .border_1()
            .border_color(content(0.1))
            .bg(content(0.05))
            .role(Role::Button)
            .cursor(gpui::CursorStyle::PointingHand);
        if video {
            // The file viewer plays video; the reply shows where it is.
            let target = path.to_string_lossy().into_owned();
            return frame
                .aria_label(format!("Open {name}"))
                .flex()
                .flex_col()
                .items_center()
                .justify_center()
                .gap_2()
                .child(icon("play", 24.).text_color(content(0.6)))
                .child(
                    div()
                        .max_w_full()
                        .px_3()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(content(0.5))
                        .child(name),
                )
                .on_click(move |_, window, cx| {
                    cx.stop_propagation();
                    window.dispatch_action(
                        Box::new(crate::file_editor::OpenPath(target.clone())),
                        cx,
                    );
                })
                .into_any_element();
        }
        let open = crate::image_lightbox::Open {
            source: crate::image_lightbox::Source::Path(path.clone()),
            alt: name.clone(),
        };
        let missing = name.clone();
        frame
            .aria_label(format!("Open {name} full screen"))
            .on_click(move |_, window, cx| {
                cx.stop_propagation();
                window.dispatch_action(Box::new(open.clone()), cx);
            })
            .child(
                crate::image_memory::image(path)
                    .size_full()
                    .object_fit(ObjectFit::Contain)
                    .with_fallback(move || fallback(missing.clone())),
            )
            .into_any_element()
    }
}

/// What an unreadable file falls back to: the link text it replaced.
fn fallback(name: SharedString) -> AnyElement {
    div()
        .py_1()
        .text_size(px(13.))
        .text_color(content(0.6))
        .child(name)
        .into_any_element()
}
