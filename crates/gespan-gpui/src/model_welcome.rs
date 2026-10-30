//! OpusWelcome.tsx and AstraWelcome.tsx: picking Opus 5.5 or Astra plays a
//! short decorative scene over the session. The scene is computed as plain
//! shapes for a moment in time, so its timing and layout are testable, and a
//! canvas paints them each frame without taking any input.
use gpui::{
    Bounds, Hsla, IntoElement, PathBuilder, Pixels, Point, Styled, Window, canvas, point, px, size,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Opus,
    Astra,
}

impl Kind {
    /// The scene for a picked model, matched on its ID or name.
    pub(crate) fn for_model<'a>(names: impl IntoIterator<Item = &'a str>) -> Option<Self> {
        let names: Vec<&str> = names.into_iter().collect();
        if names.iter().any(|name| is_astra(name)) {
            Some(Self::Astra)
        } else if names.iter().any(|name| is_opus_5_5(name)) {
            Some(Self::Opus)
        } else {
            None
        }
    }
    pub(crate) fn duration(self) -> Duration {
        match self {
            Self::Opus => Duration::from_millis(7000),
            Self::Astra => Duration::from_millis(7600),
        }
    }
}

/// `opus[\s-]?5[.-]5` after a non-alphanumeric, not followed by a digit or dot.
fn is_opus_5_5(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    lower.match_indices("opus").any(|(at, _)| {
        if at > 0 && bytes[at - 1].is_ascii_alphanumeric() {
            return false;
        }
        let mut rest = &lower[at + 4..];
        if let Some(stripped) = rest.strip_prefix([' ', '-']) {
            rest = stripped;
        }
        let Some(rest) = rest.strip_prefix('5') else {
            return false;
        };
        let Some(rest) = rest.strip_prefix(['.', '-']) else {
            return false;
        };
        let Some(rest) = rest.strip_prefix('5') else {
            return false;
        };
        !rest.starts_with(|ch: char| ch.is_ascii_digit() || ch == '.')
    })
}

/// "astra" as a whole word.
fn is_astra(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    lower.match_indices("astra").any(|(at, _)| {
        let before = at == 0 || !bytes[at - 1].is_ascii_alphanumeric();
        let after = bytes
            .get(at + 5)
            .is_none_or(|ch| !ch.is_ascii_alphanumeric());
        before && after
    })
}

const MIN_STAGE_BELOW: f32 = 140.;
const MAX_STAGE_ABOVE: f32 = 240.;

/// Pane-relative band for the Opus scene: the free space under a centered
/// composer, or a band just above a docked one that leaves no room below.
pub(crate) fn opus_stage(pane_height: f32, composer: Option<(f32, f32)>) -> (f32, f32) {
    let Some((top, bottom)) = composer else {
        let top = (pane_height * 0.55).round();
        return (top, pane_height - top);
    };
    let below = pane_height - bottom;
    if below >= MIN_STAGE_BELOW {
        return (bottom, below);
    }
    let height = MAX_STAGE_ABOVE.min(top).max(0.);
    (top - height, height)
}

/// One painted element, in layer coordinates; opacity is in its color.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Shape {
    Disc {
        center: Point<f32>,
        radius: f32,
        color: Hsla,
    },
    Ring {
        center: Point<f32>,
        radius: f32,
        width: f32,
        color: Hsla,
    },
    Line {
        points: Vec<Point<f32>>,
        width: f32,
        color: Hsla,
    },
    Polygon {
        points: Vec<Point<f32>>,
        color: Hsla,
    },
    Rect {
        origin: Point<f32>,
        size: (f32, f32),
        radius: f32,
        color: Hsla,
    },
}

/// Linear between `(at, value)` stops, held before the first and after the last.
fn ramp(t: f32, stops: &[(f32, f32)]) -> f32 {
    let Some(first) = stops.first() else {
        return 0.;
    };
    if t <= first.0 {
        return first.1;
    }
    for pair in stops.windows(2) {
        let ((a, va), (b, vb)) = (pair[0], pair[1]);
        if t <= b {
            let span = (b - a).max(f32::EPSILON);
            return va + (vb - va) * ((t - a) / span);
        }
    }
    stops[stops.len() - 1].1
}
fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}
fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    if t < 0.5 {
        4. * t * t * t
    } else {
        1. - (-2. * t + 2.).powi(3) / 2.
    }
}
/// Progress of an animation that starts at `delay` and lasts `duration`.
fn progress(elapsed: f32, delay: f32, duration: f32) -> f32 {
    ((elapsed - delay) / duration).clamp(0., 1.)
}
fn rgba(hex: u32, alpha: f32) -> Hsla {
    let mut color: Hsla = gpui::rgb(hex).into();
    color.a = alpha.clamp(0., 1.);
    color
}
/// A radial glow: soft concentric discs fading out from the center.
fn glow(out: &mut Vec<Shape>, center: Point<f32>, radius: f32, hex: u32, alpha: f32) {
    const RINGS: usize = 8;
    for ring in 0..RINGS {
        let share = 1. - ring as f32 / RINGS as f32;
        out.push(Shape::Disc {
            center,
            radius: radius * share,
            color: rgba(hex, alpha / RINGS as f32 * 1.6),
        });
    }
}
fn faded(shape: Shape, opacity: f32) -> Shape {
    let fade = |mut color: Hsla| {
        color.a *= opacity;
        color
    };
    match shape {
        Shape::Disc {
            center,
            radius,
            color,
        } => Shape::Disc {
            center,
            radius,
            color: fade(color),
        },
        Shape::Ring {
            center,
            radius,
            width,
            color,
        } => Shape::Ring {
            center,
            radius,
            width,
            color: fade(color),
        },
        Shape::Line {
            points,
            width,
            color,
        } => Shape::Line {
            points,
            width,
            color: fade(color),
        },
        Shape::Polygon { points, color } => Shape::Polygon {
            points,
            color: fade(color),
        },
        Shape::Rect {
            origin,
            size,
            radius,
            color,
        } => Shape::Rect {
            origin,
            size,
            radius,
            color: fade(color),
        },
    }
}
fn visible(shape: &Shape) -> bool {
    let color = match shape {
        Shape::Disc { color, .. }
        | Shape::Ring { color, .. }
        | Shape::Line { color, .. }
        | Shape::Polygon { color, .. }
        | Shape::Rect { color, .. } => color,
    };
    color.a > 0.002
}

/// The shapes for `kind` at `elapsed` seconds in a `width` × `height` layer.
/// `composer` is the composer box's top and bottom in the layer, if known.
pub(crate) fn scene(
    kind: Kind,
    elapsed: f32,
    width: f32,
    height: f32,
    composer: Option<(f32, f32)>,
    light: bool,
) -> Vec<Shape> {
    let total = kind.duration().as_secs_f32();
    let p = (elapsed / total).clamp(0., 1.);
    let mut out = Vec::new();
    let layer = match kind {
        Kind::Opus => {
            opus(&mut out, elapsed, p, width, height, composer, light);
            ramp(p, &[(0., 0.), (0.06, 1.), (0.78, 1.), (1., 0.)])
        }
        Kind::Astra => {
            astra(&mut out, elapsed, p, width, height, light);
            ramp(p, &[(0., 0.), (0.08, 1.), (0.72, 1.), (1., 0.)])
        }
    };
    out.into_iter()
        .map(|shape| faded(shape, layer))
        .filter(visible)
        .collect()
}

const MELODY: [(f32, f32); 10] = [
    (0.06, -3.),
    (0.13, -1.),
    (0.2, 1.),
    (0.28, 0.),
    (0.36, 2.),
    (0.44, 4.),
    (0.52, 3.),
    (0.6, 1.),
    (0.68, 2.),
    (0.77, 0.),
];

fn opus(
    out: &mut Vec<Shape>,
    elapsed: f32,
    p: f32,
    width: f32,
    height: f32,
    composer: Option<(f32, f32)>,
    light: bool,
) {
    let (clay, note) = if light {
        (0xbd5d3a, 0x8a4128)
    } else {
        (0xd97757, 0xf4ede4)
    };
    let breathe = ramp(p, &[(0., 0.), (0.22, 1.), (0.58, 1.), (1., 0.)]);
    let reach = width.max(height);
    glow(
        out,
        point(width * 0.82, height * 0.14),
        reach * 0.46,
        clay,
        0.14 * breathe,
    );
    glow(
        out,
        point(width * 0.4, height * 0.18),
        reach * 0.6,
        clay,
        0.07 * breathe,
    );
    glow(
        out,
        point(width * 0.1, height * 0.85),
        reach * 0.55,
        note,
        0.05 * breathe,
    );

    let (top, band) = opus_stage(height, composer);
    if band <= 0. {
        return;
    }
    // Rising motes: small thoughts drifting up while the phrase is written.
    for i in 0..26 {
        let x = width * (4. + ((i * 41) % 92) as f32) / 100.;
        let y = top + band * (20. + ((i * 29) % 72) as f32) / 100.;
        let size = 2. + (i % 4) as f32;
        let delay = 0.5 + ((i * 11) % 30) as f32 * 0.12;
        let duration = 2.6 + (i % 5) as f32 * 0.4;
        let drift = if i % 2 == 1 { 1. } else { -1. } * (6. + (i % 7) as f32 * 3.);
        if elapsed < delay {
            continue;
        }
        let q = ease_out(progress(elapsed, delay, duration));
        let center = point(
            x + ramp(q, &[(0., 0.), (0.25, drift * 0.3), (1., drift)]),
            y + ramp(q, &[(0., 0.), (0.25, -10.), (1., -46.)]),
        );
        let scale = ramp(q, &[(0., 0.6), (0.25, 1.), (1., 0.8)]);
        let alpha = ramp(q, &[(0., 0.), (0.25, 0.55), (1., 0.)]);
        out.push(if i % 5 == 0 {
            Shape::Ring {
                center,
                radius: size * 1.1 * scale,
                width: 1.,
                color: rgba(clay, 0.6 * alpha),
            }
        } else {
            Shape::Disc {
                center,
                radius: size / 2. * scale,
                color: rgba(clay, alpha),
            }
        });
    }

    // Five lines rise as a loose, handwritten wave and gather into one thread.
    let end_y = band * 0.42;
    let rise = band * 0.86 - end_y;
    let staff_y = |t: f32, step: f32| {
        top + end_y
            + rise * (1. - t).powf(1.7)
            + (t * std::f32::consts::PI * 2.4 + 0.3).sin() * 20f32.min(band * 0.08) * (1. - t)
            - (step / 2.) * 11. * (1. - 0.84 * t)
    };
    let settle = ramp(p, &[(0., 1.), (0.6, 1.), (1., 0.)]);
    for (index, step) in [-4., -2., 0., 2., 4.].into_iter().enumerate() {
        // A pen edge sweeps left to right, so each line writes itself.
        let written = ease_in_out(progress(elapsed, 0.15 + index as f32 * 0.09, 2.3));
        let points: Vec<_> = (0..=120)
            .map(|n| n as f32 / 120.)
            .take_while(|t| *t <= written * 1.2)
            .map(|t| point(t * width, staff_y(t, step)))
            .collect();
        if points.len() > 1 {
            let alpha = if index == 2 { 0.8 } else { 0.55 };
            out.push(Shape::Line {
                points,
                width: 1.,
                color: rgba(clay, alpha * settle),
            });
        }
    }
    for (index, (t, step)) in MELODY.into_iter().enumerate() {
        let delay = 1.05 + index as f32 * 0.23;
        if elapsed < delay {
            continue;
        }
        let q = progress(elapsed, delay, 3.6);
        let size = 12.6 * (1. - 0.84 * t);
        let (w, h) = (size * 1.45, size);
        let scale = ramp(q, &[(0., 0.2), (0.12, 1.)]);
        let alpha = ramp(q, &[(0., 0.), (0.12, 1.), (0.6, 0.9), (1., 0.)]);
        let center = point(
            t * width + ramp(q, &[(0.6, 0.), (1., w * 0.18)]),
            staff_y(t, step) + ramp(q, &[(0., -0.4 * h), (0.12, 0.)]),
        );
        let (w, h) = (w * scale, h * scale);
        out.push(Shape::Rect {
            origin: point(center.x - w / 2., center.y - h / 2.),
            size: (w, h),
            radius: h / 2.,
            color: rgba(note, alpha),
        });
        let stem = (1f32.max(size * 0.12) * scale, size * 3.1 * scale);
        let stem_origin = if step >= 1. {
            point(center.x - w / 2., center.y - h / 2. + h * 0.45)
        } else {
            point(
                center.x + w / 2. - stem.0,
                center.y + h / 2. - h * 0.45 - stem.1,
            )
        };
        out.push(Shape::Rect {
            origin: stem_origin,
            size: stem,
            radius: 1.,
            color: rgba(note, 0.85 * alpha),
        });
        // Each note rings once as it lands, like a key being struck.
        let ring = progress(elapsed, delay, 1.1);
        if ring < 1. {
            out.push(Shape::Ring {
                center,
                radius: w * 0.9 * ramp(ease_out(ring), &[(0., 0.4), (1., 2.6)]),
                width: 1.,
                color: rgba(clay, ramp(ring, &[(0., 0.), (0.2, 0.6), (1., 0.)])),
            });
        }
    }
}

fn astra(out: &mut Vec<Shape>, elapsed: f32, p: f32, width: f32, height: f32, light: bool) {
    const STAR: u32 = 0xd8c9a2;
    let (bright, dust) = if light {
        (0x806c3b, 0x806c3b)
    } else {
        (0xf5f0e4, STAR)
    };
    let nebula = ramp(p, &[(0., 0.), (0.25, 1.), (0.55, 1.), (1., 0.)]);
    let reach = width.max(height);
    glow(
        out,
        point(width * 0.82, height * 0.12),
        reach * 0.48,
        STAR,
        0.15 * nebula,
    );
    glow(out, point(width, 0.), reach * 0.65, STAR, 0.07 * nebula);
    glow(
        out,
        point(width * 0.15, height * 0.75),
        reach * 0.6,
        STAR,
        0.08 * nebula,
    );

    // A small solar focal point above the composer, even in splits.
    let center = point(width * 0.82, (height * 0.15).clamp(58., 120.));
    let arrival = progress(elapsed, 0., 6.);
    let scale = ramp(arrival, &[(0., 0.7), (0.16, 1.), (1., 1.06)]);
    let solar = ramp(arrival, &[(0., 0.), (0.16, 0.85), (0.62, 0.6), (1., 0.)]);
    let side = (width * 0.1).clamp(46., 86.) * scale;
    let mut sun = Vec::new();
    let turn = (45. * p).to_radians();
    let corona = side * 1.4;
    for ray in 0..24 {
        let angle = turn + ray as f32 * std::f32::consts::TAU / 24.;
        let (sin, cos) = angle.sin_cos();
        sun.push(Shape::Line {
            points: vec![
                point(
                    center.x + cos * corona * 0.22,
                    center.y + sin * corona * 0.22,
                ),
                point(center.x + cos * corona * 0.6, center.y + sin * corona * 0.6),
            ],
            width: 1.,
            color: rgba(STAR, 0.22),
        });
    }
    for delay in [0.2, 0.85] {
        let wave = progress(elapsed, delay, 3.2);
        if elapsed >= delay && wave < 1. {
            sun.push(Shape::Ring {
                center,
                radius: side / 2. * ramp(ease_out(wave), &[(0., 0.5), (1., 4.5)]),
                width: 1.,
                color: rgba(STAR, ramp(wave, &[(0., 0.), (0.15, 0.25), (1., 0.)])),
            });
        }
    }
    let spin = ramp(p, &[(0., 15.), (1., 235.)]);
    for (across, tilt, squash, alpha, direction) in
        [(1.9, -28f32, 0.38, 0.21, 1.), (2.4, 48f32, 0.32, 0.19, -1.)]
    {
        let (rx, ry) = (side * across / 2., side * across / 2. * squash);
        let (tilt_sin, tilt_cos) = tilt.to_radians().sin_cos();
        let at = |angle: f32| {
            let (sin, cos) = angle.sin_cos();
            let (x, y) = (cos * rx, sin * ry);
            point(
                center.x + x * tilt_cos - y * tilt_sin,
                center.y + x * tilt_sin + y * tilt_cos,
            )
        };
        sun.push(Shape::Line {
            points: (0..=48)
                .map(|n| at(n as f32 * std::f32::consts::TAU / 48.))
                .collect(),
            width: 1.,
            color: rgba(STAR, alpha),
        });
        let planet = (spin * direction - 90.).to_radians();
        sun.push(Shape::Disc {
            center: at(planet),
            radius: 2.,
            color: rgba(STAR, 1.),
        });
    }
    glow(&mut sun, center, side * 0.6, STAR, 0.25);
    sun.push(Shape::Disc {
        center,
        radius: side * 0.22,
        color: rgba(STAR, 1.),
    });
    sun.push(Shape::Disc {
        center: point(center.x - side * 0.04, center.y - side * 0.05),
        radius: side * 0.1,
        color: rgba(0xf5f0e4, 0.8),
    });
    out.extend(sun.into_iter().map(|shape| faded(shape, solar)));

    for i in 0..36 {
        let delay = 0.2 + ((i * 7) % 24) as f32 * 0.12;
        let duration = 2.2 + (i % 4) as f32 * 0.45;
        if elapsed < delay {
            continue;
        }
        let q = ease_in_out(progress(elapsed, delay, duration));
        let cross = i % 6 == 0;
        let size = if cross { 9. } else { 1. + (i % 3) as f32 };
        let scale = ramp(q, &[(0., 0.6), (0.3, 1.), (1., 0.7)]);
        let alpha = ramp(q, &[(0., 0.), (0.3, 0.7), (0.65, 0.3), (1., 0.)]);
        let c = point(
            width * (3. + ((i * 37) % 94) as f32) / 100.,
            height * (4. + ((i * 23) % 86) as f32) / 100.
                + ramp(q, &[(0., 0.), (0.3, 4.), (1., 16.)]),
        );
        let r = size / 2. * scale;
        out.push(if cross {
            let (long, short) = (r, r * 0.22);
            Shape::Polygon {
                points: vec![
                    point(c.x, c.y - long),
                    point(c.x + short, c.y - short),
                    point(c.x + long, c.y),
                    point(c.x + short, c.y + short),
                    point(c.x, c.y + long),
                    point(c.x - short, c.y + short),
                    point(c.x - long, c.y),
                    point(c.x - short, c.y - short),
                ],
                color: rgba(bright, alpha),
            }
        } else {
            Shape::Disc {
                center: c,
                radius: r,
                color: rgba(dust, alpha),
            }
        });
    }

    // Shooting stars fall down and to the left, their tails up and right.
    let (tail_sin, tail_cos) = (-55f32).to_radians().sin_cos();
    for i in 0..38 {
        let delay = 0.35 + i as f32 * 0.12;
        let duration = 1.4 + (i % 5) as f32 * 0.22;
        if elapsed < delay {
            continue;
        }
        let q = progress(elapsed, delay, duration);
        let long = i % 9 == 0;
        let length = if long {
            190.
        } else {
            45. + ((i * 23) % 95) as f32
        };
        let brightness = if i % 3 == 0 { 0.95 } else { 0.5 };
        let alpha = ramp(
            q,
            &[(0., 0.), (0.12, brightness), (0.7, brightness), (1., 0.)],
        );
        let head = point(
            width * (8. + ((i * 37) % 125) as f32) / 100. - 0.7 * height * q,
            height * (-12. + ((i * 19) % 48) as f32) / 100. + height * q,
        );
        const SEGMENTS: usize = 6;
        for segment in 0..SEGMENTS {
            let (a, b) = (
                segment as f32 / SEGMENTS as f32,
                (segment + 1) as f32 / SEGMENTS as f32,
            );
            out.push(Shape::Line {
                points: vec![
                    point(
                        head.x + tail_cos * length * a,
                        head.y + tail_sin * length * a,
                    ),
                    point(
                        head.x + tail_cos * length * b,
                        head.y + tail_sin * length * b,
                    ),
                ],
                width: if long { 2. } else { 1. },
                color: rgba(if segment == 0 { bright } else { STAR }, alpha * (1. - a)),
            });
        }
        out.push(Shape::Disc {
            center: head,
            radius: 1.5,
            color: rgba(bright, alpha),
        });
    }
}

/// A playing scene: what it is and when it started.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Run {
    pub(crate) kind: Kind,
    pub(crate) started: Instant,
}

impl Run {
    pub(crate) fn new(kind: Kind) -> Self {
        Self {
            kind,
            started: Instant::now(),
        }
    }
}

/// The layer for `run`, filling its parent. It never takes mouse input.
pub(crate) fn layer(run: Run, composer: Option<Bounds<Pixels>>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let elapsed = run.started.elapsed().as_secs_f32();
            let composer = composer.map(|box_| {
                (
                    (box_.top() - bounds.top()).as_f32(),
                    (box_.bottom() - bounds.top()).as_f32(),
                )
            });
            let shapes = scene(
                run.kind,
                elapsed,
                bounds.size.width.as_f32(),
                bounds.size.height.as_f32(),
                composer,
                crate::theme::is_light(),
            );
            window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                for shape in shapes {
                    paint(shape, bounds.origin, window);
                }
            });
            if elapsed < run.kind.duration().as_secs_f32() {
                window.request_animation_frame();
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn paint(shape: Shape, origin: Point<Pixels>, window: &mut Window) {
    let at = |p: Point<f32>| origin + point(px(p.x), px(p.y));
    match shape {
        Shape::Disc {
            center,
            radius,
            color,
        } => window.paint_quad(
            gpui::fill(
                Bounds::new(
                    at(point(center.x - radius, center.y - radius)),
                    size(px(radius * 2.), px(radius * 2.)),
                ),
                color,
            )
            .corner_radii(px(radius)),
        ),
        Shape::Ring {
            center,
            radius,
            width,
            color,
        } => window.paint_quad(
            gpui::fill(
                Bounds::new(
                    at(point(center.x - radius, center.y - radius)),
                    size(px(radius * 2.), px(radius * 2.)),
                ),
                gpui::transparent_black(),
            )
            .corner_radii(px(radius))
            .border_widths(px(width))
            .border_color(color),
        ),
        Shape::Rect {
            origin: corner,
            size: (w, h),
            radius,
            color,
        } => window.paint_quad(
            gpui::fill(Bounds::new(at(corner), size(px(w), px(h))), color).corner_radii(px(radius)),
        ),
        Shape::Line {
            points,
            width,
            color,
        } => {
            let mut builder = PathBuilder::stroke(px(width));
            for (index, p) in points.into_iter().enumerate() {
                if index == 0 {
                    builder.move_to(at(p));
                } else {
                    builder.line_to(at(p));
                }
            }
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
        Shape::Polygon { points, color } => {
            let mut builder = PathBuilder::fill();
            for (index, p) in points.into_iter().enumerate() {
                if index == 0 {
                    builder.move_to(at(p));
                } else {
                    builder.line_to(at(p));
                }
            }
            builder.close();
            if let Ok(path) = builder.build() {
                window.paint_path(path, color);
            }
        }
    }
}

#[cfg(test)]
mod tests;
