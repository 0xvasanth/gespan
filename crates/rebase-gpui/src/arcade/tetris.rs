//! Colour Tetris: classic falling tetrominoes, except every piece carries one
//! of a few colours and a full row only clears when all of it is one colour.
//! Mixed full rows stay put, so each drop weighs colour as well as shape.
//! The well takes whatever size the pane offers, so wide panes play wide.
pub(crate) const COLORS: u8 = 3;
/// Colours arrive in runs: the next piece repeats the previous colour this
/// often (out of 3), so the preview gives something to plan a row around.
const REPEAT_IN_3: u64 = 2;
/// How long a topped-out board and its final score stay up before restarting.
const GAME_OVER_MS: f32 = 2500.;
/// The idle loop lands each piece within this many columns of where it
/// appeared, so pieces fall mostly straight down instead of sliding across.
const AUTO_REACH: i32 = 3;
/// A preview's opening stack peaks somewhere between these shares of the
/// well's height, leaving the spawn rows clear.
const STACK_PEAK: (f32, f32) = (0.25, 0.45);
/// Each opening piece settles within this many columns of a random one, so
/// the stack builds up all across the well rather than from one side.
const STACK_REACH: i32 = 6;

/// Box size and rotation-zero cells for I, O, T, S, Z, J, L.
const SHAPES: [(i32, [(i32, i32); 4]); 7] = [
    (4, [(0, 1), (1, 1), (2, 1), (3, 1)]),
    (2, [(0, 0), (1, 0), (0, 1), (1, 1)]),
    (3, [(1, 0), (0, 1), (1, 1), (2, 1)]),
    (3, [(1, 0), (2, 0), (0, 1), (1, 1)]),
    (3, [(0, 0), (1, 0), (1, 1), (2, 1)]),
    (3, [(0, 0), (0, 1), (1, 1), (2, 1)]),
    (3, [(2, 0), (0, 1), (1, 1), (2, 1)]),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    kind: usize,
    rotation: usize,
    x: i32,
    y: i32,
    pub color: u8,
}

impl Piece {
    fn spawn(kind: usize, color: u8, cols: usize) -> Self {
        Self {
            kind,
            rotation: 0,
            x: (cols as i32 - SHAPES[kind].0) / 2,
            y: 0,
            color,
        }
    }
    pub fn cells(&self) -> [(i32, i32); 4] {
        let (n, base) = SHAPES[self.kind];
        base.map(|(mut x, mut y)| {
            for _ in 0..self.rotation % 4 {
                (x, y) = (n - 1 - y, x);
            }
            (self.x + x, self.y + y)
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Input {
    Left,
    Right,
    Rotate,
    SoftDrop,
    HardDrop,
}

pub(crate) struct Tetris {
    pub cols: usize,
    pub rows: usize,
    board: Vec<Vec<Option<u8>>>,
    piece: Piece,
    pub next: Piece,
    pub score: u32,
    seed: u64,
    fall: f32,
    target: Option<(usize, i32)>,
    over: Option<f32>,
    /// The idle loop is steering, so new pieces drop in anywhere across the width.
    auto: bool,
    /// The first piece hasn't been placed for the idle loop yet.
    fresh: bool,
    /// The idle loop already turned or stepped the piece on this row.
    moved_this_row: bool,
}

impl Tetris {
    pub fn new(seed: u64, cols: usize, rows: usize) -> Self {
        let (cols, rows) = (cols.max(4), rows.max(4));
        let mut game = Self {
            cols,
            rows,
            board: vec![vec![None; cols]; rows],
            piece: Piece::spawn(0, 0, cols),
            next: Piece::spawn(0, 0, cols),
            score: 0,
            seed: seed.max(1),
            fall: 0.,
            target: None,
            over: None,
            auto: false,
            fresh: true,
            moved_this_row: false,
        };
        game.piece = game.random_piece(None);
        game.next = game.random_piece(Some(game.piece.color));
        game
    }
    /// A well for the idle preview, opening on a random stack of pieces
    /// already settled along the bottom, so each showing looks different.
    pub fn preview(seed: u64, cols: usize, rows: usize) -> Self {
        let mut game = Self::new(seed, cols, rows);
        game.stack();
        game
    }
    /// Settles random pieces under a random skyline that rises and falls
    /// across the well, each where a fair player would put it near a random
    /// column: deep, with few holes, and the odd gap left in.
    fn stack(&mut self) {
        let (low, high) = STACK_PEAK;
        let share = low + (self.random() % 1_000) as f32 / 1_000. * (high - low);
        // At least two rows everywhere, so every shape can settle lying flat.
        let peak = ((self.rows as f32 * share) as i32).max(3);
        let mut height = 2 + (self.random() % (peak - 1) as u64) as i32;
        let skyline: Vec<i32> = (0..self.cols)
            .map(|_| {
                height = (height + (self.random() % 3) as i32 - 1).clamp(2, peak);
                self.rows as i32 - height
            })
            .collect();
        // Pieces only fall straight down, so each lands on the highest filled
        // cell of its columns; tracking those keeps a wide well quick to fill.
        let mut tops = vec![self.rows as i32; self.cols];
        let mut color = (self.random() % COLORS as u64) as u8;
        let (mut misses, mut tries) = (0, 0);
        while misses < 3 * self.cols && tries < self.cols * self.rows {
            tries += 1;
            let kind = (self.random() % SHAPES.len() as u64) as usize;
            if self.random() % 3 >= REPEAT_IN_3 {
                color = (self.random() % COLORS as u64) as u8;
            }
            let near = (self.random() % self.cols as u64) as i32;
            let mut best: Option<(i32, [(i32, i32); 4])> = None;
            for rotation in 0..4 {
                for x in near - STACK_REACH..=near + STACK_REACH {
                    let cells = Piece {
                        kind,
                        rotation,
                        x,
                        y: 0,
                        color,
                    }
                    .cells();
                    if cells.iter().any(|c| !(0..self.cols as i32).contains(&c.0)) {
                        continue;
                    }
                    let drop = cells
                        .iter()
                        .map(|&(x, y)| tops[x as usize] - 1 - y)
                        .min()
                        .unwrap_or(-1);
                    let cells = cells.map(|(x, y)| (x, y + drop));
                    if drop < 0 || cells.iter().any(|&(x, y)| y < skyline[x as usize]) {
                        continue;
                    }
                    let holes = cells
                        .iter()
                        .filter(|&&(x, y)| {
                            y + 1 < self.rows as i32
                                && !cells.contains(&(x, y + 1))
                                && self.board[y as usize + 1][x as usize].is_none()
                        })
                        .count() as i32;
                    let depth: i32 = cells.iter().map(|c| c.1).sum();
                    let score = depth * 2 - holes * 8 + (self.random() % 4) as i32;
                    if best.is_none_or(|(top, _)| score > top) {
                        best = Some((score, cells));
                    }
                }
            }
            let Some((_, cells)) = best else {
                misses += 1;
                continue;
            };
            misses = 0;
            for (x, y) in cells {
                self.board[y as usize][x as usize] = Some(color);
                tops[x as usize] = tops[x as usize].min(y);
            }
        }
    }
    fn random(&mut self) -> u64 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.seed = x;
        x
    }
    fn random_piece(&mut self, previous: Option<u8>) -> Piece {
        let kind = (self.random() % SHAPES.len() as u64) as usize;
        let color = match previous {
            Some(color) if self.random() % 3 < REPEAT_IN_3 => color,
            _ => (self.random() % COLORS as u64) as u8,
        };
        Piece::spawn(kind, color, self.cols)
    }
    /// Nothing has landed yet, so the well can still be resized for free.
    pub fn untouched(&self) -> bool {
        self.score == 0 && self.board.iter().flatten().all(Option::is_none)
    }
    /// The board stays frozen on its final score until the pause runs out.
    pub fn game_over(&self) -> bool {
        self.over.is_some()
    }
    fn fits(&self, piece: &Piece) -> bool {
        piece.cells().iter().all(|&(x, y)| {
            (0..self.cols as i32).contains(&x)
                && (0..self.rows as i32).contains(&y)
                && self.board[y as usize][x as usize].is_none()
        })
    }
    fn shift(&mut self, dx: i32, dy: i32) -> bool {
        let moved = Piece {
            x: self.piece.x + dx,
            y: self.piece.y + dy,
            ..self.piece
        };
        let fits = self.fits(&moved);
        if fits {
            self.piece = moved;
        }
        fits
    }
    fn rotate(&mut self) -> bool {
        let turned = Piece {
            rotation: (self.piece.rotation + 1) % 4,
            ..self.piece
        };
        // Nudge sideways off a wall or stack rather than refusing the turn.
        for kick in [0, -1, 1, -2, 2] {
            let kicked = Piece {
                x: turned.x + kick,
                ..turned
            };
            if self.fits(&kicked) {
                self.piece = kicked;
                return true;
            }
        }
        false
    }
    pub fn input(&mut self, input: Input) {
        if self.game_over() {
            return;
        }
        // A piece someone has played stays where they left it if the idle
        // loop takes over again.
        self.fresh = false;
        match input {
            Input::Left => _ = self.shift(-1, 0),
            Input::Right => _ = self.shift(1, 0),
            Input::Rotate => _ = self.rotate(),
            Input::SoftDrop => {
                if !self.shift(0, 1) {
                    self.lock();
                }
                self.fall = 0.;
            }
            Input::HardDrop => {
                while self.shift(0, 1) {}
                self.lock();
                self.fall = 0.;
            }
        }
    }
    /// Advance by `dt` ms with one gravity step every `interval` ms. `auto`
    /// steers the piece itself for the idle attract loop.
    pub fn update(&mut self, dt: f32, interval: f32, auto: bool) {
        if let Some(left) = self.over {
            if left > dt {
                self.over = Some(left - dt);
            } else {
                let seed = self.random();
                *self = if auto {
                    Self::preview(seed, self.cols, self.rows)
                } else {
                    Self::new(seed, self.cols, self.rows)
                };
            }
            return;
        }
        self.auto = auto;
        if auto {
            self.steer();
        }
        self.fall += dt;
        while self.fall >= interval {
            self.fall -= interval;
            if self.shift(0, 1) {
                self.moved_this_row = false;
            } else {
                self.lock();
            }
        }
    }
    fn lock(&mut self) {
        for (x, y) in self.piece.cells() {
            self.board[y as usize][x as usize] = Some(self.piece.color);
        }
        self.clear_rows();
        self.target = None;
        self.moved_this_row = false;
        if self.fits(&self.next) {
            self.piece = self.next;
            self.next = self.random_piece(Some(self.piece.color));
            if self.auto {
                self.scatter();
            }
        } else {
            self.over = Some(GAME_OVER_MS);
        }
    }
    fn clear_rows(&mut self) {
        let kept: Vec<_> = std::mem::take(&mut self.board)
            .into_iter()
            .filter(|row| {
                let first = row[0];
                !(first.is_some() && row.iter().all(|cell| *cell == first))
            })
            .collect();
        let cleared = self.rows - kept.len();
        self.score += 100 * cleared as u32 * cleared as u32;
        self.board = vec![vec![None; self.cols]; cleared];
        self.board.extend(kept);
    }
    /// Drops the idle loop's new piece in at a random column, the way a banner
    /// of falling blocks would, rather than always at the centre.
    fn scatter(&mut self) {
        self.fresh = false;
        let span = (self.cols as i32 - SHAPES[self.piece.kind].0).max(0) as u64 + 1;
        let moved = Piece {
            x: (self.random() % span) as i32,
            ..self.piece
        };
        if self.fits(&moved) {
            self.piece = moved;
        }
    }
    /// Deepest landing spot within reach, favouring cells that touch the same
    /// colour and, on a tie, the spot nearest where the piece is now.
    fn plan(&self) -> (usize, i32) {
        let mut best = (i32::MIN, i32::MIN, self.piece.rotation, self.piece.x);
        for rotation in 0..4 {
            for x in self.piece.x - AUTO_REACH..=self.piece.x + AUTO_REACH {
                let mut p = Piece {
                    rotation,
                    x,
                    ..self.piece
                };
                if !self.fits(&p) {
                    continue;
                }
                while self.fits(&Piece { y: p.y + 1, ..p }) {
                    p.y += 1;
                }
                let score: i32 = p
                    .cells()
                    .iter()
                    .map(|&(cx, cy)| {
                        let same = [(-1, 0), (1, 0), (0, 1)]
                            .iter()
                            .filter(|(dx, dy)| self.color_at(cx + dx, cy + dy) == Some(p.color))
                            .count() as i32;
                        cy * 2 + same * 3
                    })
                    .sum();
                let near = -(x - self.piece.x).abs();
                if (score, near) > (best.0, best.1) {
                    best = (score, near, rotation, x);
                }
            }
        }
        (best.2, best.3)
    }
    fn color_at(&self, x: i32, y: i32) -> Option<u8> {
        if (0..self.cols as i32).contains(&x) && (0..self.rows as i32).contains(&y) {
            self.board[y as usize][x as usize]
        } else {
            None
        }
    }
    /// Plays like a person: one turn or one step per row of fall, so a piece
    /// drifts a little as it drops instead of skidding sideways.
    fn steer(&mut self) {
        if self.fresh {
            self.scatter();
        }
        let (rotation, x) = match self.target {
            Some(target) => target,
            None => *self.target.insert(self.plan()),
        };
        if self.moved_this_row {
            return;
        }
        let moved = if self.piece.rotation != rotation {
            self.rotate()
        } else if self.piece.x != x {
            self.shift((x - self.piece.x).signum(), 0)
        } else {
            false
        };
        self.moved_this_row = moved;
    }
    /// Every settled cell plus the falling piece, as `(x, y, colour)`.
    pub fn blocks(&self) -> Vec<(i32, i32, u8)> {
        let mut blocks: Vec<_> = (0..self.rows)
            .flat_map(|y| (0..self.cols).map(move |x| (x, y)))
            .filter_map(|(x, y)| self.board[y][x].map(|c| (x as i32, y as i32, c)))
            .collect();
        if self.game_over() {
            return blocks;
        }
        blocks.extend(
            self.piece
                .cells()
                .iter()
                .map(|&(x, y)| (x, y, self.piece.color)),
        );
        blocks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const COLS: usize = 10;
    const ROWS: usize = 20;

    fn game() -> Tetris {
        Tetris::new(42, COLS, ROWS)
    }
    /// Fill row `y` except column `gap`, colouring each cell with `color(x)`.
    fn fill_row(t: &mut Tetris, y: usize, gap: Option<usize>, color: impl Fn(usize) -> u8) {
        for x in 0..COLS {
            t.board[y][x] = (Some(x) != gap).then(|| color(x));
        }
    }
    /// A vertical I piece of `color` dropped into column `x`.
    fn vertical_i(x: i32, color: u8) -> Piece {
        Piece {
            kind: 0,
            rotation: 1,
            x: x - 2,
            y: 0,
            color,
        }
    }

    #[test]
    fn full_row_of_one_colour_clears_and_drops_rows_above() {
        let mut t = game();
        fill_row(&mut t, ROWS - 1, Some(4), |_| 1);
        t.board[ROWS - 2][0] = Some(2);
        t.board[ROWS - 5][0] = Some(0);
        t.piece = vertical_i(4, 1);
        t.input(Input::HardDrop);
        assert_eq!(t.score, 100);
        // The cleared row is gone; the I piece's other three cells and the
        // marker above all fell one row.
        assert_eq!(t.board[ROWS - 1][0], Some(2));
        assert_eq!(t.board[ROWS - 4][0], Some(0));
        assert_eq!(t.board[ROWS - 1][4], Some(1));
        assert_eq!(t.board[ROWS - 3][4], Some(1));
        assert_eq!(t.board[ROWS - 4][4], None);
    }

    #[test]
    fn full_row_with_mixed_colours_stays() {
        let mut t = game();
        fill_row(&mut t, ROWS - 1, Some(4), |_| 1);
        t.piece = vertical_i(4, 2);
        t.input(Input::HardDrop);
        assert_eq!(t.score, 0);
        assert!(t.board[ROWS - 1].iter().all(Option::is_some));
        assert_eq!(t.board[ROWS - 1][4], Some(2));
    }

    #[test]
    fn one_colour_row_with_a_gap_stays() {
        let mut t = game();
        fill_row(&mut t, ROWS - 1, Some(9), |_| 0);
        t.clear_rows();
        assert_eq!(t.score, 0);
        assert_eq!(t.board[ROWS - 1][0], Some(0));
        assert_eq!(t.board[ROWS - 1][9], None);
    }

    #[test]
    fn only_single_colour_rows_clear_when_several_fill_at_once() {
        let mut t = game();
        fill_row(&mut t, ROWS - 1, None, |_| 0);
        fill_row(&mut t, ROWS - 2, None, |x| (x % 2) as u8);
        fill_row(&mut t, ROWS - 3, None, |_| 2);
        t.clear_rows();
        // Two rows of 100 · 2² and the mixed row sinks to the floor.
        assert_eq!(t.score, 400);
        assert_eq!(t.board[ROWS - 1][0], Some(0));
        assert_eq!(t.board[ROWS - 1][1], Some(1));
        assert!(t.board[ROWS - 2].iter().all(Option::is_none));
    }

    #[test]
    fn walls_and_stack_block_sideways_moves() {
        let mut t = game();
        t.piece = Piece::spawn(1, 0, COLS);
        for _ in 0..COLS {
            t.input(Input::Left);
        }
        assert_eq!(t.piece.cells().iter().map(|c| c.0).min(), Some(0));
        for _ in 0..COLS {
            t.input(Input::Right);
        }
        assert_eq!(
            t.piece.cells().iter().map(|c| c.0).max(),
            Some(COLS as i32 - 1)
        );
        t.board[0][COLS - 3] = Some(0);
        t.piece = Piece::spawn(1, 0, COLS);
        for _ in 0..COLS {
            t.input(Input::Right);
        }
        assert_eq!(
            t.piece.cells().iter().map(|c| c.0).max(),
            Some(COLS as i32 - 4)
        );
    }

    #[test]
    fn rotation_against_a_wall_kicks_back_inside() {
        let mut t = game();
        t.piece = vertical_i(0, 0);
        t.piece.y = 5;
        t.input(Input::Rotate);
        assert_eq!(t.piece.rotation, 2);
        assert!(t.piece.cells().iter().all(|&(x, _)| x >= 0));
        assert!(t.fits(&t.piece));
    }

    #[test]
    fn rotation_with_no_room_is_refused() {
        let mut t = game();
        t.piece = vertical_i(4, 0);
        t.piece.y = 5;
        for y in 5..9 {
            for x in 0..COLS {
                if x != 4 {
                    t.board[y][x] = Some(1);
                }
            }
        }
        t.input(Input::Rotate);
        assert_eq!(t.piece.rotation, 1);
    }

    #[test]
    fn gravity_steps_once_per_interval_and_locks_on_landing() {
        let mut t = game();
        t.piece = Piece::spawn(1, 2, COLS);
        t.update(250., 500., false);
        assert_eq!(t.piece.y, 0);
        t.update(250., 500., false);
        assert_eq!(t.piece.y, 1);
        for _ in 0..ROWS * 2 {
            t.update(500., 500., false);
        }
        // The O piece has settled on the floor and a new piece is falling.
        assert_eq!(t.board[ROWS - 1][4], Some(2));
        assert_eq!(t.board[ROWS - 2][5], Some(2));
    }

    #[test]
    fn soft_drop_on_the_floor_locks_immediately() {
        let mut t = game();
        t.piece = Piece::spawn(1, 1, COLS);
        t.piece.y = ROWS as i32 - 2;
        let next = t.next;
        t.input(Input::SoftDrop);
        assert!(!t.game_over());
        assert_eq!(t.board[ROWS - 1][4], Some(1));
        assert_eq!(t.piece, next);
    }

    #[test]
    fn topping_out_starts_a_fresh_game() {
        let mut t = game();
        t.score = 900;
        for y in 1..ROWS {
            fill_row(&mut t, y, Some(0), |x| (x % 2) as u8);
        }
        // The I piece fills the gap but the rows are mixed, so nothing clears
        // and the next piece has nowhere to spawn.
        t.piece = vertical_i(0, 0);
        t.next = Piece::spawn(1, 0, COLS);
        t.input(Input::HardDrop);
        // The full board and final score hold, ignoring input, for the pause.
        assert!(t.game_over());
        assert_eq!(t.score, 900);
        let frozen = t.blocks();
        assert_eq!(frozen.len(), (ROWS - 1) * (COLS - 1) + 4);
        t.input(Input::Left);
        t.update(GAME_OVER_MS - 1., 10., false);
        assert!(t.game_over());
        assert_eq!(t.blocks(), frozen);
        t.update(1., 10., false);
        assert!(!t.game_over());
        assert_eq!(t.score, 0);
        assert!(t.board.iter().flatten().all(Option::is_none));
    }

    #[test]
    fn colours_come_in_runs_but_all_of_them_appear() {
        let mut t = game();
        let mut previous = t.piece.color;
        let (mut repeats, mut seen) = (0, [0; COLORS as usize]);
        for _ in 0..3_000 {
            let piece = t.random_piece(Some(previous));
            repeats += (piece.color == previous) as u32;
            seen[piece.color as usize] += 1;
            previous = piece.color;
        }
        // Uniform picks would repeat ~1/3 of the time; runs repeat ~7/9.
        assert!((2_100..2_550).contains(&repeats), "{repeats} repeats");
        assert!(seen.iter().all(|&n| n > 500), "{seen:?}");
    }

    #[test]
    fn attract_loop_keeps_playing_without_input() {
        let mut t = game();
        for _ in 0..5_000 {
            t.update(33., 90., true);
            assert!(t.game_over() || t.fits(&t.piece));
            assert!(t.blocks().iter().all(|&(x, y, c)| {
                (0..COLS as i32).contains(&x) && (0..ROWS as i32).contains(&y) && c < COLORS
            }));
        }
    }

    /// Runs the idle attract loop and calls `seen` with each piece's states,
    /// in order, from its first frame to its last before it locks.
    fn watch_idle(t: &mut Tetris, frames: usize, mut seen: impl FnMut(&[Piece])) {
        let mut states: Vec<Piece> = Vec::new();
        for _ in 0..frames {
            t.update(33., 90., true);
            let piece = t.piece;
            let same = states.last().is_some_and(|last: &Piece| {
                last.kind == piece.kind && last.color == piece.color && piece.y >= last.y
            });
            if !same && !states.is_empty() {
                seen(&states);
                states.clear();
            }
            if !t.game_over() {
                states.push(piece);
            }
        }
    }

    #[test]
    fn idle_pieces_make_at_most_one_move_per_row() {
        let mut t = Tetris::new(11, 60, 22);
        let mut pieces = 0;
        watch_idle(&mut t, 4_000, |states| {
            pieces += 1;
            for row in states.chunk_by(|a, b| a.y == b.y) {
                let moves = row
                    .windows(2)
                    .filter(|w| w[0].rotation != w[1].rotation || w[0].x != w[1].x)
                    .count();
                assert!(moves <= 1, "{moves} moves in one row: {row:?}");
                for w in row.windows(2) {
                    if w[0].rotation == w[1].rotation {
                        assert!((w[0].x - w[1].x).abs() <= 1, "jumped sideways: {w:?}");
                    }
                }
            }
        });
        assert!(pieces > 20, "only {pieces} pieces fell");
    }

    #[test]
    fn idle_pieces_land_near_where_they_appear() {
        let mut t = Tetris::new(5, 60, 22);
        watch_idle(&mut t, 4_000, |states| {
            let (first, last) = (states[0], states[states.len() - 1]);
            // The reach, plus a rotation's wall kick.
            assert!(
                (last.x - first.x).abs() <= AUTO_REACH + 2,
                "slid from {} to {}",
                first.x,
                last.x
            );
        });
    }

    #[test]
    fn idle_pieces_spread_across_a_wide_well() {
        let mut t = Tetris::new(9, 60, 22);
        let mut landed = Vec::new();
        watch_idle(&mut t, 6_000, |states| {
            landed.push(states[states.len() - 1].x)
        });
        assert!(landed.len() > 30);
        let left = landed.iter().filter(|&&x| x < 15).count();
        let right = landed.iter().filter(|&&x| x >= 45).count();
        assert!(left > 0 && right > 0, "landings {landed:?}");
        // Not everything heads for the left wall.
        assert!(left * 2 < landed.len(), "landings {landed:?}");
    }

    #[test]
    fn handing_a_played_piece_back_to_the_idle_loop_leaves_it_in_place() {
        let mut t = Tetris::new(6, 60, 22);
        t.input(Input::Left);
        t.input(Input::SoftDrop);
        let played = t.piece;
        t.update(1., 90., true);
        assert_eq!(t.piece.x, played.x);
        assert_eq!(t.piece.y, played.y);
    }

    #[test]
    fn played_pieces_still_spawn_centred() {
        let mut t = Tetris::new(4, 60, 22);
        for _ in 0..3 {
            t.input(Input::HardDrop);
            let width = SHAPES[t.piece.kind].0;
            assert_eq!(t.piece.x, (60 - width) / 2);
            assert_eq!(t.piece.y, 0);
        }
    }

    #[test]
    fn a_wide_well_spawns_centred_and_clears_its_long_rows() {
        let mut t = Tetris::new(7, 32, 14);
        assert_eq!(t.board.len(), 14);
        assert!(t.board.iter().all(|row| row.len() == 32));
        let xs = t.piece.cells().map(|c| c.0);
        let mid = (xs.iter().min().unwrap() + xs.iter().max().unwrap()) / 2;
        assert!((14..=17).contains(&mid), "{xs:?}");
        for x in 0..32 {
            t.board[13][x] = (x != 20).then_some(2);
        }
        t.piece = Piece {
            kind: 0,
            rotation: 1,
            x: 18,
            y: 0,
            color: 2,
        };
        t.input(Input::HardDrop);
        assert_eq!(t.score, 100);
        assert_eq!(t.board.len(), 14);
        assert!(t.board.iter().all(|row| row.len() == 32));
        assert_eq!(t.board[13][20], Some(2));
        assert_eq!(t.board[13][0], None);
    }

    /// Settled cells, ignoring the falling piece.
    fn settled(t: &Tetris) -> Vec<(usize, usize, u8)> {
        (0..t.rows)
            .flat_map(|y| (0..t.cols).map(move |x| (x, y)))
            .filter_map(|(x, y)| t.board[y][x].map(|c| (x, y, c)))
            .collect()
    }

    #[test]
    fn a_preview_opens_on_a_colourful_stack_of_whole_pieces() {
        for seed in 1..40 {
            let t = Tetris::preview(seed, 60, 22);
            let cells = settled(&t);
            // Whole tetrominoes, nothing cleared yet.
            assert!(cells.len() >= 60, "seed {seed}: only {} cells", cells.len());
            assert_eq!(cells.len() % 4, 0, "seed {seed}");
            // Along the bottom, below the spawn rows, so play carries on.
            assert!(cells.iter().all(|&(_, y, _)| y >= 22 / 2), "seed {seed}");
            assert!(t.fits(&t.piece), "seed {seed}");
            // Every colour shows, and the floor is mostly covered end to end.
            for color in 0..COLORS {
                assert!(cells.iter().any(|c| c.2 == color), "seed {seed}");
            }
            let floor: Vec<_> = cells.iter().filter(|c| c.1 == 21).map(|c| c.0).collect();
            assert!(floor.len() > 60 / 2, "seed {seed}: floor {floor:?}");
            assert!(floor.iter().any(|&x| x < 10) && floor.iter().any(|&x| x >= 50));
        }
    }

    #[test]
    fn each_preview_has_its_own_skyline() {
        let skyline = |t: &Tetris| -> Vec<usize> {
            (0..t.cols)
                .map(|x| {
                    (0..t.rows)
                        .find(|&y| t.board[y][x].is_some())
                        .unwrap_or(t.rows)
                })
                .collect()
        };
        let skylines: Vec<_> = (1..20)
            .map(|s| skyline(&Tetris::preview(s, 40, 22)))
            .collect();
        for (i, a) in skylines.iter().enumerate() {
            for b in &skylines[i + 1..] {
                assert_ne!(a, b, "two previews built the same stack");
            }
        }
        // Not a flat slab: each stack rises and falls across the well.
        for s in &skylines {
            let (lo, hi) = (s.iter().min().unwrap(), s.iter().max().unwrap());
            assert!(hi - lo >= 2, "flat skyline {s:?}");
        }
        // And how high they reach varies from one preview to the next.
        let peaks: std::collections::HashSet<_> =
            skylines.iter().map(|s| *s.iter().min().unwrap()).collect();
        assert!(peaks.len() >= 3, "peaks {peaks:?}");
    }

    #[test]
    fn previews_fill_narrow_and_very_wide_wells() {
        for (cols, rows) in [(10, 20), (125, 22), (400, 22)] {
            let t = Tetris::preview(3, cols, rows);
            let cells = settled(&t);
            assert!(!cells.is_empty(), "{cols}x{rows}");
            assert!(cells.iter().all(|&(x, y, _)| x < cols && y >= rows / 2));
            assert!(t.fits(&t.piece), "{cols}x{rows}");
        }
    }

    #[test]
    fn the_idle_loop_plays_on_from_a_preview_stack() {
        let mut t = Tetris::preview(8, 30, 22);
        for _ in 0..5_000 {
            t.update(33., 90., true);
            assert!(t.game_over() || t.fits(&t.piece));
        }
    }

    #[test]
    fn an_idle_game_that_tops_out_restarts_on_a_new_stack() {
        let mut t = Tetris::preview(2, 24, 12);
        let before = settled(&t);
        t.over = Some(1.);
        t.update(5., 100., true);
        let after = settled(&t);
        assert!(!after.is_empty() && after != before);
        // A played game still restarts on a clean well.
        t.over = Some(1.);
        t.update(5., 100., false);
        assert!(t.untouched());
    }

    #[test]
    fn a_game_restarted_after_topping_out_keeps_its_well_size() {
        let mut t = Tetris::new(3, 24, 12);
        t.over = Some(1.);
        t.update(5., 100., false);
        assert!(!t.game_over());
        assert_eq!((t.cols, t.rows), (24, 12));
        assert!(t.untouched());
    }
}
