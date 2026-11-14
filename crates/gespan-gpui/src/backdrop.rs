//! The Gradient backdrop (Appearance › Backdrop): the design's desk, four
//! soft radial glows on a dark base, painted behind the glass window.
//!
//! GPUI has no radial gradient, and blurred shadows on rounded boxes show
//! their outline. So the glows are computed exactly as the design's CSS
//! (`radial-gradient(rx ry at x% y%, colour α, transparent 70%)`) into a
//! small texture, one texel per [`SCALE`] points, that the GPU stretches
//! over the window. It is rebuilt only when the colours or the window's
//! size bucket change, so painting it costs one textured quad a frame.
use gpui::{App, ObjectFit, Pixels, RenderImage, Size, div, img, prelude::*};
use std::{cell::RefCell, sync::Arc};

/// The desk's base colour beneath the glows.
pub(crate) const BASE: u32 = 0x17151c;
/// The design desk's four glow colours, in its order: violet, blue, rose, sand.
const DESK: [u32; 4] = [0x8f64f6, 0x459bf7, 0xf2a6ba, 0xf0d696];
/// (strength, centre x, centre y as a share of the window, radii in points),
/// from `.desk` in the design's glass.css, topmost first.
const GLOWS: [(f32, f32, f32, f32, f32); 4] = [
    (0.55, 0.12, 0.18, 620., 460.),
    (0.40, 0.88, 0.12, 560., 420.),
    (0.30, 0.70, 0.92, 700., 520.),
    (0.22, 0.22, 0.88, 520., 380.),
];
/// The window's wash over the desk, `--glass-shell` in the design: lighter
/// than the shell's usual fill, so the glows read across the window rather
/// than only at their cores.
pub(crate) fn wash() -> gpui::Hsla {
    gpui::rgba(0x0d0d0d9e).into()
}
/// The design's `backdrop-filter: saturate(1.7)` on the window, baked into
/// the texture since the desk is all the window would saturate.
const SATURATE: f32 = 1.7;
/// Where each glow reaches transparent, as a share of its radius.
const FADE: f32 = 0.7;
/// Points per texel. The glows are wide and smooth, so a quarter-resolution
/// texture stretched by the GPU looks the same and costs a sixteenth.
const SCALE: f32 = 4.;
/// Texture sizes round up to this many texels, so live resizing reuses
/// one texture across many window sizes instead of building one per frame.
const BUCKET: u32 = 16;

/// The four glow colours for a gradient whose swatch runs `a` → `b`: its
/// two colours lead, and the desk's other colours follow in the desk's order,
/// so Aurora is the design's desk exactly.
pub(crate) fn palette(a: u32, b: u32) -> [u32; 4] {
    let mut rest = DESK.into_iter().filter(|c| *c != a && *c != b);
    let third = rest.next().unwrap_or(a);
    let fourth = rest.next().unwrap_or(b);
    [a, b, third, fourth]
}

/// A glow's opacity at `t`, the distance from its centre as a share of its
/// radius: CSS's straight fall from `strength` at the centre to nothing at
/// [`FADE`]. There is no solid core and no edge.
pub(crate) fn glow_alpha(strength: f32, t: f32) -> f32 {
    (strength * (1. - t / FADE)).max(0.)
}

fn channels(hex: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| ((hex >> shift) & 0xff) as f32 / 255.)
}

/// The desk at `width` × `height` points as RGBA texels, one per [`SCALE`]
/// points, opaque. Colours mix in sRGB as the browser's gradients do, then
/// take the window's saturation.
pub(crate) fn pixels(colors: [u32; 4], width: f32, height: f32) -> image::RgbaImage {
    let (cols, rows) = texels(width, height);
    // The texture covers its whole bucket; positions follow the texture so
    // stretching it over the window keeps each glow where the design puts it.
    let (width, height) = (cols as f32 * SCALE, rows as f32 * SCALE);
    let base = channels(BASE);
    let glows = GLOWS.map(|(strength, x, y, rx, ry)| (strength, x * width, y * height, rx, ry));
    let tints = colors.map(channels);
    image::RgbaImage::from_fn(cols, rows, |col, row| {
        let (px, py) = ((col as f32 + 0.5) * SCALE, (row as f32 + 0.5) * SCALE);
        let mut color = base;
        // Bottom layer first: CSS paints the first gradient on top.
        for ((strength, cx, cy, rx, ry), tint) in glows.iter().zip(&tints).rev() {
            let t = (((px - cx) / rx).powi(2) + ((py - cy) / ry).powi(2)).sqrt();
            let alpha = glow_alpha(*strength, t);
            for (c, tint) in color.iter_mut().zip(tint) {
                *c += (tint - *c) * alpha;
            }
        }
        let [r, g, b] = saturate(color).map(|c| (c * 255.).round().clamp(0., 255.) as u8);
        image::Rgba([r, g, b, 255])
    })
}

/// CSS's `saturate()` matrix, in sRGB as browsers apply it.
fn saturate([r, g, b]: [f32; 3]) -> [f32; 3] {
    let s = SATURATE;
    [
        (0.213 + 0.787 * s) * r + (0.715 - 0.715 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 + 0.285 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 - 0.715 * s) * g + (0.072 + 0.928 * s) * b,
    ]
    .map(|c| c.clamp(0., 1.))
}

/// How many texels cover a `width` × `height` window, rounded up to the
/// size bucket.
fn texels(width: f32, height: f32) -> (u32, u32) {
    let side = |points: f32| {
        let n = (points.max(1.) / SCALE).ceil() as u32;
        n.div_ceil(BUCKET) * BUCKET
    };
    (side(width), side(height))
}

type Key = ([u32; 4], (u32, u32));
thread_local! {
    static CACHE: RefCell<Option<(Key, Arc<RenderImage>)>> = const { RefCell::new(None) };
}

/// The desk texture for these colours at this window size, built once and
/// reused until either changes; the replaced texture leaves the GPU atlas.
fn texture(colors: [u32; 4], size: Size<Pixels>, cx: &mut App) -> Arc<RenderImage> {
    let (width, height) = (f32::from(size.width), f32::from(size.height));
    let key = (colors, texels(width, height));
    if let Some(image) = CACHE.with_borrow(|cache| {
        cache
            .as_ref()
            .filter(|(cached, _)| *cached == key)
            .map(|(_, image)| image.clone())
    }) {
        return image;
    }
    let mut pixels = pixels(colors, width, height);
    // GPUI's textures are BGRA.
    for pixel in pixels.pixels_mut() {
        pixel.0.swap(0, 2);
    }
    let image = Arc::new(RenderImage::new(vec![image::Frame::new(pixels)]));
    if let Some((_, old)) = CACHE.replace(Some((key, image.clone()))) {
        cx.drop_image(old, None);
    }
    image
}

/// The backdrop layer for a gradient `a` → `b`, filling its parent.
pub(crate) fn layer(a: u32, b: u32, size: Size<Pixels>, cx: &mut App) -> gpui::Div {
    div()
        .debug_selector(|| "shell-backdrop".into())
        .absolute()
        .inset_0()
        .overflow_hidden()
        .child(
            img(texture(palette(a, b), size, cx))
                .size_full()
                .object_fit(ObjectFit::Fill),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_glow_fades_smoothly_to_nothing_with_no_opaque_core() {
        for (strength, ..) in GLOWS {
            let samples: Vec<f32> = (0..=100)
                .map(|i| glow_alpha(strength, i as f32 / 100.))
                .collect();
            assert_eq!(
                samples[0], strength,
                "the centre is the glow's own strength"
            );
            assert!(strength < 1., "no glow is opaque");
            assert!(
                samples.windows(2).all(|w| w[1] <= w[0]),
                "it only ever fades outwards"
            );
            assert_eq!(glow_alpha(strength, FADE), 0., "gone at 70% of its radius");
            assert_eq!(glow_alpha(strength, 1.), 0.);
            // The largest step between neighbouring samples is the even
            // fall, not a jump at an edge.
            let step = samples.windows(2).map(|w| w[0] - w[1]).fold(0., f32::max);
            assert!(step <= strength / (FADE * 100.) + 1e-6, "step {step}");
        }
    }

    #[test]
    fn the_desk_has_no_visible_edge_at_any_window_size() {
        for (width, height) in [(1440., 900.), (2560., 1440.), (900., 640.)] {
            let image = pixels(palette(0x8f64f6, 0x459bf7), width, height);
            let mut worst = 0u8;
            for (x, y, pixel) in image.enumerate_pixels() {
                for (dx, dy) in [(1, 0), (0, 1)] {
                    if let Some(next) = image.get_pixel_checked(x + dx, y + dy) {
                        for c in 0..3 {
                            worst = worst.max(pixel.0[c].abs_diff(next.0[c]));
                        }
                    }
                }
            }
            // Four points apart, neighbours differ by a couple of levels at
            // most: a gradient, never an outline.
            assert!(worst <= 3, "{width}×{height}: a step of {worst}");
        }
    }

    #[test]
    fn glows_sit_where_the_design_puts_them_and_fade_to_the_base() {
        let image = pixels(palette(0x8f64f6, 0x459bf7), 1440., 900.);
        let at = |x: f32, y: f32| {
            let (cols, rows) = image.dimensions();
            image
                .get_pixel(
                    ((x * cols as f32) as u32).min(cols - 1),
                    ((y * rows as f32) as u32).min(rows - 1),
                )
                .0
        };
        let base = saturate(channels(BASE)).map(|c| (c * 255.).round() as u8);
        // Top left glows violet: more blue and red than the base.
        let violet = at(0.12, 0.18);
        assert!(
            violet[2] > base[2] + 60 && violet[0] > base[0] + 40,
            "{violet:?}"
        );
        // Top right glows blue: blue well above red.
        let blue = at(0.88, 0.12);
        assert!(blue[2] > blue[0] + 40, "{blue:?}");
        // The middle of the left edge is between glows: near the base.
        let between = at(0.0, 0.5);
        for c in 0..3 {
            assert!(between[c].abs_diff(base[c]) < 40, "{between:?}");
        }
        assert!(image.pixels().all(|p| p.0[3] == 255), "the desk is opaque");
    }

    #[test]
    fn aurora_is_the_design_desk_and_every_gradient_keeps_four_distinct_glows() {
        assert_eq!(palette(0x8f64f6, 0x459bf7), DESK);
        for (a, b) in [(0xf2a6ba, 0x8f64f6), (0xf0d696, 0xf2a6ba)] {
            let colors = palette(a, b);
            assert_eq!(&colors[..2], &[a, b], "its swatch colours lead");
            let mut sorted = colors;
            sorted.sort();
            sorted.windows(2).for_each(|w| assert_ne!(w[0], w[1]));
        }
    }

    #[test]
    fn resizing_within_a_bucket_reuses_the_texture_size() {
        assert_eq!(texels(1440., 900.), texels(1450., 905.));
        assert_ne!(texels(1440., 900.), texels(1600., 900.));
        let (cols, rows) = texels(1440., 900.);
        assert!(cols as f32 * SCALE >= 1440. && rows as f32 * SCALE >= 900.);
    }
}
