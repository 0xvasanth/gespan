use gpui::{
    AnyElement, Bounds, CursorStyle, MouseButton, Pixels, ResizeEdge, Size, Window, div, point,
    prelude::*, px, size,
};

// Keep the resize border narrow around controls, with larger corner targets.
fn resize_regions(viewport: Size<Pixels>) -> [(ResizeEdge, CursorStyle, Bounds<Pixels>); 8] {
    use CursorStyle::*;
    use ResizeEdge::*;
    let w = f32::from(viewport.width);
    let h = f32::from(viewport.height);
    let edge = 6.;
    let corner = 12.;
    [
        (Top, ResizeUpDown, corner, 0., w - 2. * corner, edge),
        (
            Bottom,
            ResizeUpDown,
            corner,
            h - edge,
            w - 2. * corner,
            edge,
        ),
        (Left, ResizeLeftRight, 0., corner, edge, h - 2. * corner),
        (
            Right,
            ResizeLeftRight,
            w - edge,
            corner,
            edge,
            h - 2. * corner,
        ),
        (TopLeft, ResizeUpLeftDownRight, 0., 0., corner, corner),
        (
            TopRight,
            ResizeUpRightDownLeft,
            w - corner,
            0.,
            corner,
            corner,
        ),
        (
            BottomLeft,
            ResizeUpRightDownLeft,
            0.,
            h - corner,
            corner,
            corner,
        ),
        (
            BottomRight,
            ResizeUpLeftDownRight,
            w - corner,
            h - corner,
            corner,
            corner,
        ),
    ]
    .map(|(edge, cursor, x, y, w, h)| {
        (
            edge,
            cursor,
            Bounds::new(point(px(x), px(y)), size(px(w), px(h))),
        )
    })
}

pub(super) fn resize_handles(window: &Window) -> Vec<AnyElement> {
    if window.is_fullscreen()
        || window.is_maximized()
        || !matches!(
            window.window_decorations(),
            gpui::Decorations::Client { .. }
        )
    {
        return Vec::new();
    }
    resize_regions(window.viewport_size())
        .into_iter()
        .map(|(edge, cursor, bounds)| {
            div()
                .absolute()
                .occlude()
                .left(bounds.left())
                .top(bounds.top())
                .w(bounds.size.width)
                .h(bounds.size.height)
                .cursor(cursor)
                .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                    cx.stop_propagation();
                    window.start_window_resize(edge);
                })
                .into_any_element()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_regions_cover_edges_and_corners_without_covering_content() {
        let viewport = size(px(820.), px(520.));
        let regions = resize_regions(viewport);
        for (position, expected) in [
            ((1., 1.), ResizeEdge::TopLeft),
            ((410., 1.), ResizeEdge::Top),
            ((819., 1.), ResizeEdge::TopRight),
            ((819., 260.), ResizeEdge::Right),
            ((819., 519.), ResizeEdge::BottomRight),
            ((410., 519.), ResizeEdge::Bottom),
            ((1., 519.), ResizeEdge::BottomLeft),
            ((1., 260.), ResizeEdge::Left),
        ] {
            let hits: Vec<_> = regions
                .iter()
                .filter(|(_, _, bounds)| bounds.contains(&point(px(position.0), px(position.1))))
                .map(|(edge, _, _)| *edge)
                .collect();
            assert_eq!(hits, vec![expected]);
        }
        for position in [(410., 260.), (20., 20.), (800., 500.)] {
            assert!(regions.iter().all(|(_, _, bounds)| {
                !bounds.contains(&point(px(position.0), px(position.1)))
            }));
        }
    }
}
