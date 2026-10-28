//! React's Pac-man and Snake engines, painted with GPUI rather than a web view,
//! plus a native-only colour Tetris.
mod tetris;
use crate::{popover::AnchorSlot, theme::*};
use gpui::{
    App, Context, FocusHandle, Focusable, Render, Role, Window, canvas, div, fill, point,
    prelude::*, px, size,
};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tetris::{Input, Tetris};
const GAMES: [&str; 3] = ["Pac-man", "Snake", "Tetris"];
const TETRIS: usize = 2;
fn seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| d.as_nanos() as u64)
}
/// The space the well leaves at the pane's (top, bottom, sides): in play the
/// HUD's bar above and its padding round the rest; idle the banner runs edge
/// to edge like Pac-man and Snake.
fn insets(playing: bool) -> (f32, f32, f32) {
    if playing { (36., 8., 8.) } else { (8., 8., 0.) }
}
/// Columns the next-piece preview and its gap take beside the well in play.
fn preview_cols(playing: bool) -> usize {
    if playing { 6 } else { 0 }
}
/// The well fills the pane: cells near 26px in play (8px idle, like the other
/// attract loops), leaving room for the HUD and, in play, the preview beside
/// it, so a wide pane plays wide rather than in a phone-shaped column.
fn tetris_size(width: f32, height: f32, playing: bool) -> (usize, usize) {
    let (top, bottom, side) = insets(playing);
    let (cell, max_rows) = if playing { (26., 30.) } else { (8., 22.) };
    let room = height - top - bottom;
    let rows = (room / cell).floor().clamp(8., max_rows);
    // Whole pixels, as `well` paints them, so the columns fill the width.
    let cell = (room / rows).floor().max(4.);
    let cols = ((width - 2. * side) / cell).floor() - preview_cols(playing) as f32;
    (cols.clamp(10., 400.) as usize, rows as usize)
}
/// Emitted when a game takes or releases the keyboard; the empty session
/// lifts the arcade over its composer only while one is in play.
pub(crate) struct PlayingChanged;
impl gpui::EventEmitter<PlayingChanged> for Arcade {}
fn tetris_input(key: &str) -> Option<Input> {
    Some(match key {
        "left" | "a" => Input::Left,
        "right" | "d" => Input::Right,
        "up" | "w" => Input::Rotate,
        "down" | "s" => Input::SoftDrop,
        "space" => Input::HardDrop,
        _ => return None,
    })
}
#[derive(Clone, Default, Deserialize)]
struct Frame {
    cells: Vec<(usize, f32)>,
    sprites: Vec<Sprite>,
    pickup: Option<Pickup>,
    bubble: Option<Bubble>,
    score: u32,
    lives: u32,
    dim: f32,
    fade: f32,
}
#[derive(Clone, Deserialize)]
struct Sprite {
    kind: String,
    cx: f32,
    cy: f32,
    size: f32,
    dx: f32,
    dy: f32,
    alpha: f32,
    mouth: f32,
    rows: Option<Vec<String>>,
    eyes: Option<bool>,
}
#[derive(Clone, Deserialize)]
struct Pickup {
    harness: String,
    x: f32,
    y: f32,
    cells: f32,
    alpha: f32,
}
#[derive(Clone, Deserialize)]
struct Bubble {
    text: String,
    x: f32,
    y: f32,
    alpha: f32,
}
pub(crate) struct Arcade {
    context: rquickjs::Context,
    _runtime: rquickjs::Runtime,
    frame: Frame,
    tetris: Tetris,
    anchor: AnchorSlot,
    focus: FocusHandle,
    previous: Option<FocusHandle>,
    playing: bool,
    index: usize,
    mode: &'static str,
    heading: (i32, i32),
    last: Instant,
    switched: Instant,
}
impl Arcade {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let runtime = rquickjs::Runtime::new().expect("arcade runtime");
        runtime.set_memory_limit(8 * 1024 * 1024);
        let context = rquickjs::Context::full(&runtime).expect("arcade context");
        context
            .with(|cx| cx.eval::<(), _>(include_str!("../assets/settings-arcade.js")))
            .expect("arcade engines");
        Self {
            context,
            _runtime: runtime,
            frame: Frame::default(),
            tetris: Tetris::new(seed(), 10, 20),
            anchor: Default::default(),
            focus: cx.focus_handle(),
            previous: None,
            playing: false,
            index: 0,
            mode: "mid",
            heading: (0, 0),
            last: Instant::now(),
            switched: Instant::now(),
        }
    }
    fn tick(&mut self, reduced: bool) {
        let now = Instant::now();
        if now.duration_since(self.last) < Duration::from_millis(33)
            && (self.index == TETRIS || !self.frame.cells.is_empty())
        {
            return;
        }
        if !self.playing && now.duration_since(self.switched) > Duration::from_secs(16) {
            self.show((self.index + 1) % GAMES.len());
        }
        let bounds = self.anchor.get().unwrap_or_default();
        let cols = (bounds.size.width.as_f32() / 7.).ceil().clamp(7., 400.) as usize;
        let rows = (bounds.size.height.as_f32() / 7.).ceil().clamp(5., 250.) as usize;
        let dt = if reduced && !self.playing {
            0.
        } else {
            now.duration_since(self.last).as_secs_f32().min(0.1) * 1000.
        };
        self.last = now;
        if self.index == TETRIS {
            self.fit_tetris(bounds.size.width.as_f32(), bounds.size.height.as_f32());
            let interval = match (self.playing, self.mode) {
                (false, _) => 90.,
                (true, "low") => 700.,
                (true, "hard") => 220.,
                _ => 450.,
            };
            self.tetris.update(dt, interval, !self.playing);
            return;
        }
        let (dx, dy) = std::mem::take(&mut self.heading);
        let result = self.context.with(|cx| {
            let object: rquickjs::Object = cx.globals().get("NativeArcade")?;
            let f: rquickjs::Function = object.get("frame")?;
            f.call::<_, String>((
                cols,
                rows,
                dt,
                self.playing,
                self.index,
                self.mode,
                vec![dx, dy],
            ))
        });
        if let Ok(json) = result
            && let Ok(frame) = serde_json::from_str(&json)
        {
            self.frame = frame;
        }
    }
    /// Resize the well to the pane. The idle loop always follows; a game in
    /// play only until its first piece lands, which covers the jump from the
    /// banner to the full pane on take-control without wiping a real game.
    fn fit_tetris(&mut self, width: f32, height: f32) {
        let size = tetris_size(width, height, self.playing);
        if size != (self.tetris.cols, self.tetris.rows)
            && (!self.playing || self.tetris.untouched())
        {
            self.tetris = if self.playing {
                Tetris::new(seed(), size.0, size.1)
            } else {
                Tetris::preview(seed(), size.0, size.1)
            };
        }
    }
    /// Shows a game in the idle rotation. Tetris opens on a freshly shuffled
    /// stack each time it comes round, so the preview never repeats itself.
    fn show(&mut self, index: usize) {
        self.index = index;
        self.switched = Instant::now();
        if index == TETRIS && !self.playing {
            self.tetris = Tetris::preview(seed(), self.tetris.cols, self.tetris.rows);
        }
    }
    /// Play starts on a clean well, not the preview's stack.
    fn clear_well(&mut self) {
        self.tetris = Tetris::new(seed(), self.tetris.cols, self.tetris.rows);
    }
    pub(crate) fn playing(&self) -> bool {
        self.playing
    }
    pub(crate) fn set_playing(&mut self, playing: bool, cx: &mut Context<Self>) {
        if self.playing != playing {
            self.playing = playing;
            cx.emit(PlayingChanged);
        }
        cx.notify();
    }
    /// The JS engines' pickup and speech bubble; their last frame lingers
    /// while Tetris plays, so it must not paint over the well.
    fn overlays(&self) -> (Option<&Pickup>, Option<&Bubble>) {
        if self.index == TETRIS {
            return (None, None);
        }
        (self.frame.pickup.as_ref(), self.frame.bubble.as_ref())
    }
    fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.switched = Instant::now();
        self.set_playing(false, cx);
        if let Some(focus) = self.previous.take() {
            window.focus(&focus, cx);
        }
    }
}
impl Focusable for Arcade {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for Arcade {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.tick(cx.reduce_motion());
        // The engine steps at most every 33ms, so repaint on the shared 30fps
        // clock. A game in play keeps running under reduced motion.
        if !crate::pulse::request(crate::pulse::Rate::Smooth, window, cx) && self.playing {
            window.request_animation_frame();
        }
        let frame = self.frame.clone();
        let tetris = (self.index == TETRIS).then(|| {
            (
                self.tetris.blocks(),
                self.tetris.next,
                (self.tetris.cols, self.tetris.rows),
            )
        });
        let playing = self.playing;
        let index = self.index;
        let mode = self.mode;
        let mut root = div()
            .id("empty-session-arcade")
            .debug_selector(|| "empty-session-arcade".into())
            .group("empty-session-arcade")
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .overflow_hidden()
            .track_focus(&self.focus)
            .when(playing, |d| d.h_full().bg(shell_fill()).occlude())
            .when(!playing, |d| d.h(px(192.)))
            .child(self.anchor.layout_probe())
            .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                if !this.playing
                    || event.keystroke.modifiers.platform
                    || event.keystroke.modifiers.control
                    || event.keystroke.modifiers.alt
                {
                    return;
                }
                let key = event.keystroke.key.as_str();
                if this.index == TETRIS && key != "escape" {
                    let Some(input) = tetris_input(key) else {
                        return;
                    };
                    this.tetris.input(input);
                    cx.stop_propagation();
                    cx.notify();
                    return;
                }
                let heading = match key {
                    "up" | "w" => (0, -1),
                    "down" | "s" => (0, 1),
                    "left" | "a" => (-1, 0),
                    "right" | "d" => (1, 0),
                    "escape" => {
                        this.release(window, cx);
                        cx.stop_propagation();
                        return;
                    }
                    _ => return,
                };
                this.heading = heading;
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        if let Some((blocks, next, size)) = &tetris {
                            paint_tetris(bounds, window, blocks, next, *size, playing);
                            return;
                        }
                        let cols = (bounds.size.width.as_f32() / 7.).ceil().max(7.) as usize;
                        let h = bounds.size.height.as_f32();
                        let opacity = |y: f32| {
                            if playing {
                                1.
                            } else {
                                (1. - ((y / h - 0.65) / 0.35).clamp(0., 1.)).max(0.)
                            }
                        };
                        for y in 0..(h / 7.).ceil() as usize {
                            for x in 0..cols {
                                let a = 0.06 * frame.fade * opacity(y as f32 * 7.);
                                if a > 0.005 {
                                    window.paint_quad(fill(
                                        gpui::Bounds::new(
                                            bounds.origin
                                                + point(px(x as f32 * 7.), px(y as f32 * 7.)),
                                            size(px(6.), px(6.)),
                                        ),
                                        content(a),
                                    ));
                                }
                            }
                        }
                        for (index, alpha) in &frame.cells {
                            let x = index % cols;
                            let y = index / cols;
                            window.paint_quad(fill(
                                gpui::Bounds::new(
                                    bounds.origin + point(px(x as f32 * 7.), px(y as f32 * 7.)),
                                    size(px(6.), px(6.)),
                                ),
                                content(alpha * 0.72 * frame.dim * opacity(y as f32 * 7.)),
                            ));
                        }
                        for sprite in &frame.sprites {
                            let side = sprite.size * 7. * 0.8;
                            let x = sprite.cx * 7. + 3.;
                            let y = sprite.cy * 7. + 3.;
                            let alpha = sprite.alpha * frame.dim * opacity(y);
                            if sprite.kind == "pacman" {
                                let angle = sprite.dy.atan2(sprite.dx);
                                let mouth = sprite.mouth * std::f32::consts::PI;
                                let mut path = gpui::PathBuilder::fill();
                                let center = bounds.origin + point(px(x), px(y));
                                path.move_to(center);
                                for i in 0..=32 {
                                    let a = angle
                                        + mouth
                                        + (std::f32::consts::TAU - 2. * mouth) * i as f32 / 32.;
                                    path.line_to(
                                        center
                                            + point(
                                                px(a.cos() * side / 2.),
                                                px(a.sin() * side / 2.),
                                            ),
                                    );
                                }
                                path.close();
                                if let Ok(path) = path.build() {
                                    window.paint_path(path, content(alpha * 0.92));
                                }
                            } else if let Some(rows) = &sprite.rows {
                                let unit = side / 8.;
                                for (iy, row) in rows.iter().enumerate() {
                                    for (ix, pixel) in row.bytes().enumerate() {
                                        if pixel == b'#' && !sprite.eyes.unwrap_or(false) {
                                            window.paint_quad(fill(
                                                gpui::Bounds::new(
                                                    bounds.origin
                                                        + point(
                                                            px(x - side / 2. + ix as f32 * unit),
                                                            px(y - side / 2. + iy as f32 * unit),
                                                        ),
                                                    size(px(unit), px(unit)),
                                                ),
                                                content(alpha * 0.8),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    },
                )
                .absolute()
                .size_full(),
            );
        let (pickup, bubble) = self.overlays();
        if let Some(pickup) = pickup {
            root = root.child(
                div()
                    .absolute()
                    .left(px(pickup.x * 7.))
                    .top(px(pickup.y * 7.))
                    .opacity(pickup.alpha * 0.7 * self.frame.dim)
                    .child(provider_icon(&pickup.harness, pickup.cells * 7. - 1.)),
            );
        }
        if let Some(bubble) = bubble {
            root = root.child(
                div()
                    .absolute()
                    .left(px((bubble.x * 7.).max(4.)))
                    .top(px((bubble.y * 7. - 24.).max(4.)))
                    .text_size(px(9.))
                    .font_family(crate::fonts::monospace())
                    .text_color(content(bubble.alpha * 0.9 * self.frame.dim))
                    .child(bubble.text.clone()),
            );
        }
        if playing {
            let difficulties = div()
                .flex()
                .gap_1()
                .children(["low", "mid", "hard"].map(|m| {
                    div()
                        .id(m)
                        .role(Role::Button)
                        .aria_label(format!("Difficulty {m}"))
                        .px_2()
                        .py_1()
                        .border_1()
                        .border_color(content(if mode == m { 0.4 } else { 0.1 }))
                        .child(m)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.mode = m;
                            cx.notify();
                        }))
                }));
            let release = div()
                .id("release-arcade")
                .role(Role::Button)
                .aria_label("Release game")
                .cursor_pointer()
                .child("esc · release")
                .on_click(cx.listener(|this, _, window, cx| this.release(window, cx)));
            root = root.child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .right_0()
                    .p_2()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(11.))
                    .font_family(crate::fonts::monospace())
                    .child(match index {
                        0 => format!(
                            "score {} {}",
                            self.frame.score,
                            "•".repeat(self.frame.lives as usize)
                        ),
                        TETRIS if self.tetris.game_over() => {
                            format!("game over · score {}", self.tetris.score)
                        }
                        TETRIS => format!("score {}", self.tetris.score),
                        _ => format!("score {}", self.frame.score),
                    })
                    .child(difficulties)
                    .child(release),
            );
        } else {
            let play = div()
                .id("play-arcade")
                .role(Role::Button)
                .aria_label(format!("Take control of {}", GAMES[index]))
                .opacity(0.)
                .group_hover("empty-session-arcade", |s| s.opacity(1.))
                .cursor_pointer()
                .font_family(crate::fonts::monospace())
                .text_size(px(11.))
                .child(format!(
                    "[ take control · {} ]",
                    GAMES[index].to_lowercase()
                ))
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.index == TETRIS {
                        this.clear_well();
                    }
                    this.previous = window.focused(cx);
                    this.set_playing(true, cx);
                    window.focus(&this.focus, cx);
                }));
            let games = div()
                .absolute()
                .bottom(px(6.))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .gap_1()
                .children((0..GAMES.len()).map(|i| {
                    div()
                        .id(("game", i))
                        .role(Role::Button)
                        .aria_label(GAMES[i].to_lowercase())
                        .size_4()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .size(px(6.))
                                .bg(content(if index == i { 0.45 } else { 0.15 })),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.show(i);
                            cx.notify();
                        }))
                }));
            root = root
                .child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(play),
                )
                .child(games);
        }
        root
    }
}

/// Where a `cols` × `rows` well paints in a pane: its cell size and top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Well {
    cell: f32,
    left: f32,
    top: f32,
}
fn well(width: f32, height: f32, (cols, rows): (usize, usize), playing: bool) -> Well {
    let (top, bottom, side) = insets(playing);
    // In play the next-piece preview sits a column right of the well, so
    // centre and size the pair together to keep it inside narrow panes.
    let span = (cols + preview_cols(playing)) as f32;
    let (room_x, room_y) = (width - 2. * side, height - top - bottom);
    let cell = (room_y / rows as f32).min(room_x / span).floor().max(4.);
    Well {
        cell,
        left: side + ((room_x - cell * span) / 2.).floor(),
        top: top + ((room_y - cell * rows as f32) / 2.).floor(),
    }
}

/// The well sits centred under the HUD; idle it paints at half strength
/// beneath the composer like the other attract loops.
fn paint_tetris(
    bounds: gpui::Bounds<gpui::Pixels>,
    window: &mut Window,
    blocks: &[(i32, i32, u8)],
    next: &tetris::Piece,
    (cols, rows): (usize, usize),
    playing: bool,
) {
    let Well { cell, left, top } = well(
        bounds.size.width.as_f32(),
        bounds.size.height.as_f32(),
        (cols, rows),
        playing,
    );
    let strength = if playing { 0.92 } else { 0.45 };
    let mut square = |x: f32, y: f32, color: gpui::Hsla| {
        window.paint_quad(fill(
            gpui::Bounds::new(
                bounds.origin + point(px(left + x * cell), px(top + y * cell)),
                size(px(cell - 1.), px(cell - 1.)),
            ),
            color,
        ));
    };
    for y in 0..rows {
        for x in 0..cols {
            square(
                x as f32,
                y as f32,
                content(if playing { 0.06 } else { 0.03 }),
            );
        }
    }
    for &(x, y, color) in blocks {
        square(x as f32, y as f32, block_color(color, strength));
    }
    if playing {
        let cells = next.cells();
        let min_x = cells.iter().map(|c| c.0).min().unwrap_or(0);
        let min_y = cells.iter().map(|c| c.1).min().unwrap_or(0);
        for (x, y) in cells {
            square(
                (cols as i32 + 1 + x - min_x) as f32,
                (y - min_y) as f32,
                block_color(next.color, strength),
            );
        }
    }
}
fn block_color(color: u8, alpha: f32) -> gpui::Hsla {
    const HUES: [f32; tetris::COLORS as usize] = [0.98, 0.12, 0.52];
    let lightness = if is_light() { 0.5 } else { 0.62 };
    gpui::hsla(HUES[color as usize % HUES.len()], 0.72, lightness, alpha)
}

#[cfg(test)]
mod tests {
    use gpui::AppContext;
    #[test]
    fn tetris_keys_cover_arrows_wasd_and_space() {
        use super::{Input, tetris_input};
        for (keys, input) in [
            (["left", "a"], Input::Left),
            (["right", "d"], Input::Right),
            (["up", "w"], Input::Rotate),
            (["down", "s"], Input::SoftDrop),
        ] {
            for key in keys {
                assert_eq!(tetris_input(key), Some(input), "{key}");
            }
        }
        assert_eq!(tetris_input("space"), Some(Input::HardDrop));
        assert_eq!(tetris_input("escape"), None);
        assert_eq!(tetris_input("enter"), None);
    }

    #[gpui::test]
    fn idle_attract_loop_rotates_through_all_three_games(cx: &mut gpui::TestAppContext) {
        let view = cx.new(super::Arcade::new);
        view.update(cx, |a, _| {
            let mut seen = Vec::new();
            for _ in 0..4 {
                a.switched -= std::time::Duration::from_secs(17);
                a.last -= std::time::Duration::from_millis(40);
                a.tick(false);
                seen.push(a.index);
            }
            assert_eq!(seen, [1, super::TETRIS, 0, 1]);
            // Nobody is playing, so the attract loop never stops on one game.
            a.playing = true;
            a.switched -= std::time::Duration::from_secs(17);
            a.last -= std::time::Duration::from_millis(40);
            a.tick(false);
            assert_eq!(a.index, 1);
        });
    }

    #[gpui::test]
    fn tetris_ticks_the_native_engine_not_the_js_one(cx: &mut gpui::TestAppContext) {
        let view = cx.new(super::Arcade::new);
        view.update(cx, |a, _| {
            a.index = super::TETRIS;
            a.playing = true;
            let before = a.tetris.blocks();
            // 100ms of "hard" gravity is enough to drop the piece a row.
            a.mode = "hard";
            for _ in 0..4 {
                a.last -= std::time::Duration::from_millis(100);
                a.tick(false);
            }
            assert_ne!(a.tetris.blocks(), before);
            assert!(a.frame.cells.is_empty(), "the JS engine never ran");
        });
    }

    #[gpui::test]
    fn js_overlays_left_over_from_snake_stay_hidden_under_tetris(cx: &mut gpui::TestAppContext) {
        let view = cx.new(super::Arcade::new);
        view.update(cx, |a, _| {
            a.frame.pickup = Some(super::Pickup {
                harness: "claude".into(),
                x: 1.,
                y: 1.,
                cells: 2.,
                alpha: 1.,
            });
            a.frame.bubble = Some(super::Bubble {
                text: "hi".into(),
                x: 1.,
                y: 1.,
                alpha: 1.,
            });
            a.index = 1;
            let (pickup, bubble) = a.overlays();
            assert!(pickup.is_some() && bubble.is_some());
            a.index = super::TETRIS;
            let (pickup, bubble) = a.overlays();
            assert!(pickup.is_none() && bubble.is_none());
        });
    }

    #[test]
    fn the_well_fills_wide_panes_instead_of_a_phone_column() {
        use super::tetris_size;
        // A wide desktop pane plays landscape.
        let (cols, rows) = tetris_size(1400., 800., true);
        assert!(cols > rows * 3 / 2, "{cols}x{rows}");
        assert!((26..=30).contains(&rows), "{rows}");
        // The preview still fits: well + gap + preview within the width.
        let cell = (800. - 44.) / rows as f32;
        assert!((cols + 6) as f32 * cell <= 1400. - 16.);
        // A narrow pane never drops below a classic 10-wide well.
        assert_eq!(tetris_size(260., 800., true).0, 10);
        // Idle, the banner spreads across the width like Pac-man and Snake,
        // even past the 160 columns that once left wide panes gapped.
        assert_eq!(tetris_size(1000., 192., false), (125, 22));
        assert_eq!(tetris_size(2000., 192., false), (250, 22));
    }

    #[test]
    fn the_idle_banner_spans_the_full_width_like_pacman_and_snake() {
        use super::{tetris_size, well};
        for width in (400..=3200).step_by(25) {
            let width = width as f32;
            let (cols, rows) = tetris_size(width, 192., false);
            let w = well(width, 192., (cols, rows), false);
            let right = width - w.left - w.cell * cols as f32;
            assert!(w.left >= 0. && right >= 0., "{width}: {w:?}");
            // Under a cell of slack in all, split evenly between the sides.
            assert!(w.left + right < w.cell, "{width}: {} + {right}px", w.left);
            assert!(
                (w.left - right).abs() <= 1.,
                "{width}: {} vs {right}",
                w.left
            );
            // Every width keeps the banner's 8px cells and full height.
            assert_eq!((w.cell, rows), (8., 22), "{width}");
        }
    }

    #[test]
    fn a_game_in_play_fills_the_pane_under_the_hud() {
        use super::{tetris_size, well};
        for width in (700..=2600).step_by(50) {
            for height in (500..=1500).step_by(50) {
                let (width, height) = (width as f32, height as f32);
                let (cols, rows) = tetris_size(width, height, true);
                let w = well(width, height, (cols, rows), true);
                let at = format!("{width}x{height}: {cols}x{rows} {w:?}");
                // The well, a column's gap and the next-piece preview.
                let right = width - w.left - (cols + 6) as f32 * w.cell;
                let bottom = height - w.top - rows as f32 * w.cell;
                // Inside the HUD's 8px padding and below its 36px bar, centred.
                assert!(w.left >= 8. && right >= 8., "{at}");
                assert!(w.top >= 36. && bottom >= 8., "{at}");
                assert!((w.left - right).abs() <= 1., "{at}: {} vs {right}", w.left);
                assert!((w.top - 36. - (bottom - 8.)).abs() <= 1., "{at}: {bottom}");
                // Less than a cell of slack across and under a pixel per row
                // down, unless a narrow pane holds the well at its 10 columns.
                if cols > 10 {
                    assert!(w.left + right - 16. < w.cell, "{at}: {} + {right}", w.left);
                    assert!(w.top - 36. + bottom - 8. < rows as f32, "{at}: {bottom}");
                }
            }
        }
    }

    #[test]
    fn a_game_kept_through_a_resize_stays_inside_the_pane() {
        use super::{tetris_size, well};
        let board = tetris_size(1400., 900., true);
        for (width, height) in [(700., 900.), (1400., 500.), (900., 600.), (2400., 1400.)] {
            let w = well(width, height, board, true);
            let at = format!("{width}x{height}: {w:?}");
            assert!(w.left >= 0., "{at}");
            assert!(w.left + (board.0 + 6) as f32 * w.cell <= width, "{at}");
            assert!(w.top + board.1 as f32 * w.cell <= height, "{at}");
        }
    }

    #[gpui::test]
    fn the_well_follows_the_pane_until_a_game_has_something_to_lose(cx: &mut gpui::TestAppContext) {
        let view = cx.new(super::Arcade::new);
        view.update(cx, |a, _| {
            a.fit_tetris(1000., 192.);
            assert_eq!((a.tetris.cols, a.tetris.rows), (125, 22));
            // Taking control clears the preview's stack and grows the
            // arcade to the whole pane.
            a.clear_well();
            a.playing = true;
            a.fit_tetris(1400., 800.);
            let wide = (a.tetris.cols, a.tetris.rows);
            assert_eq!(wide, super::tetris_size(1400., 800., true));
            // Once a piece has landed, a resize keeps the game as it is.
            a.tetris.input(super::Input::HardDrop);
            assert!(!a.tetris.untouched());
            a.fit_tetris(700., 800.);
            assert_eq!((a.tetris.cols, a.tetris.rows), wide);
        });
    }

    #[gpui::test]
    fn the_preview_reshuffles_its_stack_each_time_tetris_comes_round(
        cx: &mut gpui::TestAppContext,
    ) {
        let view = cx.new(super::Arcade::new);
        view.update(cx, |a, _| {
            let mut stacks = Vec::new();
            for _ in 0..3 {
                // Rotate on until Tetris shows again.
                loop {
                    a.switched -= std::time::Duration::from_secs(17);
                    a.last -= std::time::Duration::from_millis(40);
                    a.tick(false);
                    if a.index == super::TETRIS {
                        break;
                    }
                }
                assert!(!a.tetris.untouched(), "the preview opened on an empty well");
                stacks.push(a.tetris.blocks());
            }
            assert_ne!(stacks[0], stacks[1]);
            assert_ne!(stacks[1], stacks[2]);
            // Picking Tetris from the dots shows a fresh stack too.
            a.show(1);
            a.show(super::TETRIS);
            assert!(!a.tetris.untouched());
            assert_ne!(a.tetris.blocks(), stacks[2]);
        });
    }

    #[gpui::test]
    fn taking_control_of_tetris_starts_on_a_clean_well(cx: &mut gpui::TestAppContext) {
        let view = cx.new(super::Arcade::new);
        view.update(cx, |a, _| {
            a.show(super::TETRIS);
            assert!(!a.tetris.untouched());
            let size = (a.tetris.cols, a.tetris.rows);
            a.clear_well();
            assert!(a.tetris.untouched());
            assert_eq!((a.tetris.cols, a.tetris.rows), size);
            // While playing, a resize before the first landing stays clean.
            a.playing = true;
            a.fit_tetris(1400., 800.);
            assert!(a.tetris.untouched());
        });
    }

    #[gpui::test]
    fn taking_and_releasing_control_tells_the_session_once(cx: &mut gpui::TestAppContext) {
        let view = cx.new(super::Arcade::new);
        let changes = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let seen = changes.clone();
        let _subscription = cx.update(|cx| {
            cx.subscribe(&view, move |arcade, _: &super::PlayingChanged, cx| {
                seen.borrow_mut().push(arcade.read(cx).playing());
            })
        });
        for playing in [true, true, false] {
            view.update(cx, |a, cx| a.set_playing(playing, cx));
            cx.run_until_parked();
        }
        // Repeating the same state is not a change.
        assert_eq!(*changes.borrow(), [true, false]);
    }

    #[gpui::test]
    fn original_engines_support_both_games(cx: &mut gpui::TestAppContext) {
        let view = cx.new(super::Arcade::new);
        view.update(cx, |a, _| {
            for index in [0usize, 1usize] {
                let result = a
                    .context
                    .with(|cx| {
                        let o: rquickjs::Object = cx.globals().get("NativeArcade")?;
                        let f: rquickjs::Function = o.get("frame")?;
                        f.call::<_, String>((
                            120usize,
                            50usize,
                            33f32,
                            true,
                            index,
                            "mid",
                            vec![0, 0],
                        ))
                    })
                    .unwrap();
                let frame: super::Frame = serde_json::from_str(&result).unwrap();
                assert_eq!(frame.dim, 1.);
                assert!(!frame.cells.is_empty());
            }
        });
    }
}
