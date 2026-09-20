//! Snapping a dragged window to an edge of the desktop.
//!
//! A window dragged against the left or the right edge takes that half of the desktop when it is
//! let go; against the top edge it takes the whole desktop. What it would take is worked out here
//! so the screen can show it before the drag ends, and so that letting go and pressing a key give
//! the same rectangle.

use qframe::geometry::Rect;

use super::layout::clamp_into;

/// An edge of the desktop a window snaps to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Edge {
    /// The left edge: the left half of the desktop.
    Left,
    /// The right edge: the right half of the desktop.
    Right,
    /// The top edge: the whole desktop.
    Top,
}

/// The rectangle a window takes when it snaps to `edge` of `area`.
///
/// The two halves together cover the whole area with no gap; an odd column goes to the left half,
/// which is also the half the one-shot tiling makes larger. On a desktop too narrow for two
/// windows the halves are widened to the smallest window size and so overlap: a window is never
/// smaller than that, and snapping stays usable instead of being switched off at some width.
#[must_use]
pub fn rect_of(edge: Edge, area: Rect) -> Rect {
    let right_width = area.width / 2;
    let left_width = area.width - right_width;
    let rect = match edge {
        Edge::Left => Rect::new(area.x, area.y, left_width, area.height),
        Edge::Right => Rect::new(area.x.saturating_add(i32::from(left_width)), area.y, right_width, area.height),
        Edge::Top => area,
    };
    clamp_into(rect, area)
}

/// The edge a window at `rect` is being dragged against, if any.
///
/// The window has to touch the edge: because a dragged window is kept inside the desktop, pushing
/// it further than the edge leaves it resting against it, which is the gesture. The top edge wins
/// over the side ones, so dragging into a top corner asks for the whole desktop rather than half
/// of it — the larger, more deliberate target.
#[must_use]
pub fn edge_at(rect: Rect, area: Rect) -> Option<Edge> {
    if rect.y <= area.y {
        return Some(Edge::Top);
    }
    if rect.x <= area.x {
        return Some(Edge::Left);
    }
    if rect.right() >= area.right() {
        return Some(Edge::Right);
    }
    None
}

/// What a dragged window would become if it were let go now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Snap {
    /// The edge the window rests against.
    pub edge: Edge,
    /// The rectangle it would take.
    pub rect: Rect,
}

/// The snap a window at `rect` is asking for inside `area`, or `None` when it touches no edge.
#[must_use]
pub fn target(rect: Rect, area: Rect) -> Option<Snap> {
    let edge = edge_at(rect, area)?;
    Some(Snap { edge, rect: rect_of(edge, area) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The desktop area of an 80x24 terminal.
    fn area() -> Rect {
        Rect::new(0, 0, 80, 23)
    }

    #[test]
    fn the_halves_cover_the_desktop_without_a_gap() {
        let left = rect_of(Edge::Left, area());
        let right = rect_of(Edge::Right, area());
        assert_eq!(left, Rect::new(0, 0, 40, 23));
        assert_eq!(right, Rect::new(40, 0, 40, 23));
        assert_eq!(left.right(), right.x);
    }

    #[test]
    fn an_odd_column_goes_to_the_left_half() {
        let narrow = Rect::new(0, 0, 81, 23);
        assert_eq!(rect_of(Edge::Left, narrow), Rect::new(0, 0, 41, 23));
        assert_eq!(rect_of(Edge::Right, narrow), Rect::new(41, 0, 40, 23));
    }

    #[test]
    fn the_top_edge_takes_the_whole_desktop() {
        assert_eq!(rect_of(Edge::Top, area()), area());
    }

    #[test]
    fn a_half_is_never_below_the_smallest_window() {
        let tiny = Rect::new(0, 0, 30, 8);
        assert_eq!(rect_of(Edge::Left, tiny), Rect::new(0, 0, 20, 8));
        assert_eq!(rect_of(Edge::Right, tiny), Rect::new(10, 0, 20, 8));
    }

    #[test]
    fn a_window_in_the_middle_asks_for_nothing() {
        assert_eq!(target(Rect::new(10, 5, 30, 10), area()), None);
    }

    #[test]
    fn resting_against_an_edge_asks_for_that_half() {
        let left = target(Rect::new(0, 5, 30, 10), area()).expect("left edge");
        assert_eq!(left.edge, Edge::Left);
        assert_eq!(left.rect, Rect::new(0, 0, 40, 23));
        let right = target(Rect::new(50, 5, 30, 10), area()).expect("right edge");
        assert_eq!(right.edge, Edge::Right);
        assert_eq!(right.rect, Rect::new(40, 0, 40, 23));
    }

    #[test]
    fn a_top_corner_asks_for_the_whole_desktop() {
        let corner = target(Rect::new(0, 0, 30, 10), area()).expect("top edge");
        assert_eq!(corner.edge, Edge::Top);
        assert_eq!(corner.rect, area());
    }
}
