//! GeneratedImage.tsx: an image a provider made, drawn from its saved file.
use super::*;
use gespan_core::generated_image::Image;
use gpui::{AnyElement, ObjectFit, StyledImage};

impl TranscriptView {
    pub(super) fn generated_image(&self, image: &Image, key: usize) -> AnyElement {
        let path = std::path::PathBuf::from(&image.path);
        let alt: SharedString = image
            .alt
            .clone()
            .unwrap_or_else(|| image.name.clone())
            .into();
        let name: SharedString = if image.name.is_empty() {
            "generated-image".into()
        } else {
            image.name.clone().into()
        };
        let open = crate::image_lightbox::Open {
            source: crate::image_lightbox::Source::Path(path.clone()),
            alt: alt.clone(),
        };
        div()
            .w_full()
            .min_w_0()
            .py_3()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .id(("generated-image", key))
                    .debug_selector(|| "generated-image".into())
                    .max_w_full()
                    .overflow_hidden()
                    .rounded(px(12.))
                    .border_1()
                    .border_color(content(0.1))
                    .bg(content(0.05))
                    .role(Role::Button)
                    .aria_label(format!("Open {name} full screen"))
                    .cursor(gpui::CursorStyle::PointingHand)
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(Box::new(open.clone()), cx)
                    })
                    .child(
                        crate::image_memory::image(path)
                            .max_w_full()
                            .max_h(px(640.))
                            .object_fit(ObjectFit::Contain)
                            .with_loading(|| notice("Loading generated image…"))
                            .with_fallback(|| notice("Could not open generated image.")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .min_w_0()
                    .gap_2()
                    .text_size(px(11.))
                    .text_color(content(0.45))
                    .child(div().truncate().child(name))
                    .when(image.size > 0, |row| {
                        row.child(crate::file_editor::media::file_size(image.size))
                    }),
            )
            .into_any_element()
    }
}

fn notice(text: &'static str) -> AnyElement {
    div()
        .px_4()
        .py_3()
        .text_size(px(12.))
        .text_color(content(0.5))
        .child(text)
        .into_any_element()
}
