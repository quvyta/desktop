//! What must be true of the window list no matter what happened to it.
//!
//! The unit tests next to each rule say what one operation does. These say what every sequence of
//! them does together: whatever is opened, closed, dragged, resized, snapped, tiled, minimized,
//! maximized and whatever the terminal does meanwhile, a window is never outside the desktop,
//! never too small, the order and the ids stay sound, and the focus is where it belongs.
//!
//! The sequences come from a seeded generator, so a failure is always the same failure and can be
//! replayed by its seed.

use std::collections::HashSet;

use qframe::geometry::{Rect, Size};

use super::tests::entry;
use super::{Edge, Grip, Placement, Window, WindowId, Windows};

/// A small deterministic generator: xorshift64 with a final multiply. It only has to spread the
/// choices of the sequences evenly, which it does, and it adds no dependency.
struct Rng(u64);

impl Rng {
    /// A generator for `seed`. Zero would keep giving zero, so it is nudged off it.
    fn new(seed: u64) -> Self {
        Self(seed | 1)
    }

    /// The next number.
    fn next(&mut self) -> u64 {
        let mut state = self.0;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        self.0 = state;
        state.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    /// A number below `bound`.
    fn below(&mut self, bound: u64) -> usize {
        usize::try_from(self.next() % bound).unwrap_or(0)
    }

    /// A number in `low..=high`.
    fn between(&mut self, low: i32, high: i32) -> i32 {
        let span = u64::try_from(high - low + 1).unwrap_or(1);
        low + i32::try_from(self.below(span)).unwrap_or(0)
    }
}

/// The grips a resize may hold.
const GRIPS: [Grip; 8] = [
    Grip::Left,
    Grip::Right,
    Grip::Top,
    Grip::Bottom,
    Grip::TopLeft,
    Grip::TopRight,
    Grip::BottomLeft,
    Grip::BottomRight,
];

/// The edges a window may snap to.
const EDGES: [Edge; 3] = [Edge::Left, Edge::Right, Edge::Top];

/// How many kinds of operation a sequence draws from.
const OPERATIONS: u64 = 18;

/// Whether `rect` lies whole inside `area`. The edges are compared rather than the overlap,
/// because a rectangle of no cells overlaps nothing and would look as if it were outside.
fn inside(rect: Rect, area: Rect) -> bool {
    rect.x >= area.x && rect.right() <= area.right() && rect.y >= area.y && rect.bottom() <= area.bottom()
}

/// Checks everything the window list promises, and gives back the ids it holds.
fn check(desk: &Windows, seed: u64, step: usize) -> Vec<WindowId> {
    let area = desk.area();
    let min = desk.min_size();
    let mut ids = Vec::new();
    let mut seen = HashSet::new();
    for window in desk.iter() {
        let rect = window.rect();
        let place = format!("seed {seed}, step {step}, window {}", window.id().number());
        assert!(seen.insert(window.id()), "{place}: in the order twice");
        assert!(rect.width >= min.width, "{place}: {rect:?} is narrower than {min:?}");
        assert!(rect.height >= min.height, "{place}: {rect:?} is shorter than {min:?}");
        assert!(inside(rect, area), "{place}: {rect:?} is not inside {area:?}");
        if window.is_maximized() {
            assert_eq!(rect, area, "{place}: maximized but not filling {area:?}");
        }
        if let Some(restore) = window.placement().restore() {
            assert!(inside(restore, area), "{place}: restores to {restore:?}, outside {area:?}");
            assert!(restore.width >= min.width && restore.height >= min.height, "{place}: restores below {min:?}");
        }
        ids.push(window.id());
    }
    assert_eq!(ids.len(), desk.len(), "seed {seed}, step {step}: the order and the count disagree");
    match desk.focus() {
        Some(focus) => {
            let window = desk.get(focus).expect("the focused window is open");
            assert!(!window.is_minimized(), "seed {seed}, step {step}: the focus is on a minimized window");
        }
        None => assert_eq!(desk.visible().count(), 0, "seed {seed}, step {step}: a drawn window has no focus"),
    }
    ids
}

/// Does one random thing to `desk`, and records every id it ever handed out in `handed`.
fn step(desk: &mut Windows, rng: &mut Rng, ids: &[WindowId], handed: &mut HashSet<WindowId>) {
    let pick = |rng: &mut Rng| ids.get(rng.below(u64::try_from(ids.len()).unwrap_or(1).max(1))).copied();
    let area = desk.area();
    let far_x = i32::from(area.width) + 10;
    let far_y = i32::from(area.height) + 10;
    match rng.below(OPERATIONS) {
        0 | 1 => {
            let mut opening = entry("program");
            if rng.below(4) == 0 {
                let width = u16::try_from(rng.between(1, 120)).unwrap_or(1);
                let height = u16::try_from(rng.between(1, 40)).unwrap_or(1);
                opening.window.size = Some((width, height));
            }
            opening.window.maximized = rng.below(4) == 0;
            let id = desk.open(&opening);
            assert!(handed.insert(id), "the id {} was handed out twice", id.number());
        }
        2 => {
            if let Some(id) = pick(rng) {
                desk.close(id);
            }
        }
        3 | 4 => {
            if let Some(id) = pick(rng) {
                desk.raise(id);
            }
        }
        5 => {
            if let Some(id) = pick(rng) {
                desk.minimize(id);
            }
        }
        6 => {
            if let Some(id) = pick(rng) {
                desk.restore(id);
            }
        }
        7 => {
            if let Some(id) = pick(rng) {
                desk.maximize(id);
            }
        }
        8 => {
            if let Some(id) = pick(rng) {
                desk.unmaximize(id);
            }
        }
        9 => {
            if let Some(id) = pick(rng) {
                desk.toggle_maximized(id);
            }
        }
        10 | 11 => {
            if let Some(id) = pick(rng) {
                let (dx, dy) = (rng.between(-40, 40), rng.between(-20, 20));
                desk.move_by(id, dx, dy);
            }
        }
        12 => {
            if let Some(id) = pick(rng) {
                let (x, y) = (rng.between(-10, far_x), rng.between(-10, far_y));
                desk.move_to(id, x, y);
            }
        }
        13 | 14 => {
            if let Some(id) = pick(rng) {
                let grip = GRIPS[rng.below(u64::try_from(GRIPS.len()).unwrap_or(1))];
                let (dx, dy) = (rng.between(-40, 40), rng.between(-20, 20));
                desk.resize_by(id, grip, dx, dy);
            }
        }
        15 => {
            if let Some(id) = pick(rng) {
                if rng.below(2) == 0 {
                    desk.snap(id, EDGES[rng.below(u64::try_from(EDGES.len()).unwrap_or(1))]);
                } else {
                    desk.drop_dragged(id);
                }
            }
        }
        16 => {
            let _ = desk.tile();
        }
        _ => {
            let width = u16::try_from(rng.between(1, 200)).unwrap_or(1);
            let height = u16::try_from(rng.between(1, 60)).unwrap_or(1);
            desk.resize(Size::new(width, height));
        }
    }
}

/// Runs one sequence of `steps` operations from `seed`, checking the promises after every one.
fn sequence(seed: u64, steps: usize) -> Windows {
    let mut rng = Rng::new(seed);
    let mut desk = Windows::new(Size::new(80, 24));
    let mut handed = HashSet::new();
    let mut ids = check(&desk, seed, 0);
    for at in 1..=steps {
        step(&mut desk, &mut rng, &ids, &mut handed);
        ids = check(&desk, seed, at);
    }
    desk
}

#[test]
fn every_sequence_of_operations_keeps_the_promises() {
    for seed in 1..=60 {
        sequence(seed, 200);
    }
}

#[test]
fn the_sequences_do_reach_every_state_a_window_can_be_in() {
    let mut counts = (0_u32, 0_u32, 0_u32, 0_u32);
    for seed in 1..=60 {
        for window in sequence(seed, 200).iter() {
            match window.placement() {
                Placement::Floating => counts.0 += 1,
                Placement::Maximized { .. } => counts.1 += 1,
                Placement::Snapped { .. } => counts.2 += 1,
            }
            if window.is_minimized() {
                counts.3 += 1;
            }
        }
    }
    assert!(counts.0 > 0 && counts.1 > 0 && counts.2 > 0 && counts.3 > 0, "some state was never reached: {counts:?}");
}

#[test]
fn maximizing_and_restoring_gives_back_the_same_rectangle_whatever_came_before() {
    for seed in 1..=60 {
        let mut desk = sequence(seed, 120);
        let ids: Vec<WindowId> = desk.iter().map(Window::id).collect();
        for id in ids {
            desk.unmaximize(id);
            let before = desk.get(id).map(Window::rect).expect("open");
            assert!(desk.maximize(id), "seed {seed}: window {} would not maximize", id.number());
            assert_eq!(desk.get(id).map(Window::rect), Some(desk.area()));
            assert!(desk.unmaximize(id), "seed {seed}: window {} would not restore", id.number());
            assert_eq!(
                desk.get(id).map(Window::rect),
                Some(before),
                "seed {seed}: window {} came back elsewhere",
                id.number()
            );
        }
    }
}

#[test]
fn tiling_after_any_sequence_leaves_the_windows_apart_or_says_why_not() {
    for seed in 1..=60 {
        let mut desk = sequence(seed, 120);
        let before: Vec<_> = desk.iter().map(Window::rect).collect();
        match desk.tile() {
            Ok(()) => {
                let tiled: Vec<_> = desk.visible().map(Window::rect).collect();
                for (index, one) in tiled.iter().enumerate() {
                    for other in &tiled[index + 1..] {
                        assert!(one.intersect(*other).is_empty(), "seed {seed}: {one:?} and {other:?} overlap");
                    }
                }
                check(&desk, seed, usize::MAX);
            }
            Err(_) => assert_eq!(
                desk.iter().map(Window::rect).collect::<Vec<_>>(),
                before,
                "seed {seed}: a refused tiling moved a window"
            ),
        }
    }
}
