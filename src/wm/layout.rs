//! Where a window may sit: the smallest size it may take, the size it opens at, and the rules
//! that keep every rectangle inside the desktop area while it is moved or resized.
//!
//! Every function here is pure geometry on [`Rect`]; none of them knows about a window.

use qframe::geometry::{Rect, Size};

/// The smallest a window may be, in columns and rows.
pub const MIN_SIZE: Size = Size::new(20, 5);

/// The smallest size a window may take inside `area`.
///
/// Normally [`MIN_SIZE`], but on a terminal smaller than that the whole area: a window that
/// cannot be small enough fills what there is instead of hanging over the edge.
#[must_use]
pub fn min_size(area: Rect) -> Size {
    MIN_SIZE.min(area.size())
}

/// The size a window opens at in `area` when its entry asks for `wanted`, or two thirds of the
/// area when it asks for nothing. Never below [`min_size`], never larger than the area.
#[must_use]
pub fn opening_size(area: Rect, wanted: Option<(u16, u16)>) -> Size {
    let size = match wanted {
        Some((width, height)) => Size::new(width, height),
        None => Size::new(area.width / 3 * 2, area.height / 3 * 2),
    };
    fit_size(size, area)
}

/// `size` brought into the range a window in `area` may have.
#[must_use]
pub fn fit_size(size: Size, area: Rect) -> Size {
    let min = min_size(area);
    Size::new(
        size.width.clamp(min.width, area.width.max(min.width)),
        size.height.clamp(min.height, area.height.max(min.height)),
    )
}

/// `rect` brought fully inside `area`: first its size, then its position.
///
/// The desktop has no off-screen area, so a window is pushed in rather than clipped: after this
/// the whole rectangle is visible, which is what the invariants of the window list promise.
#[must_use]
pub fn clamp_into(rect: Rect, area: Rect) -> Rect {
    let size = fit_size(rect.size(), area);
    let last_x = area.right().saturating_sub(i32::from(size.width));
    let last_y = area.bottom().saturating_sub(i32::from(size.height));
    Rect::new(
        rect.x.clamp(area.x, last_x.max(area.x)),
        rect.y.clamp(area.y, last_y.max(area.y)),
        size.width,
        size.height,
    )
}

/// `rect` moved by `dx` columns and `dy` rows, kept inside `area`.
#[must_use]
pub fn moved(rect: Rect, dx: i32, dy: i32, area: Rect) -> Rect {
    let moved = Rect::new(rect.x.saturating_add(dx), rect.y.saturating_add(dy), rect.width, rect.height);
    clamp_into(moved, area)
}

/// The edge or corner of a window a drag holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Grip {
    /// The left edge: the window grows to the left.
    Left,
    /// The right edge.
    Right,
    /// The top edge: the window grows upwards.
    Top,
    /// The bottom edge.
    Bottom,
    /// The top left corner.
    TopLeft,
    /// The top right corner.
    TopRight,
    /// The bottom left corner.
    BottomLeft,
    /// The bottom right corner.
    BottomRight,
}

impl Grip {
    /// Whether this grip moves the left edge.
    #[must_use]
    pub fn holds_left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }

    /// Whether this grip moves the right edge.
    #[must_use]
    pub fn holds_right(self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }

    /// Whether this grip moves the top edge.
    #[must_use]
    pub fn holds_top(self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }

    /// Whether this grip moves the bottom edge.
    #[must_use]
    pub fn holds_bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::BottomLeft | Self::BottomRight)
    }

    /// The grip a drag started at the cell `(x, y)` of `rect` holds.
    ///
    /// The rectangle is read as nine zones: a point in the outer third of an axis holds that
    /// edge, one in the middle third holds neither, and the two answers together name an edge or
    /// a corner. A point in the middle of both axes holds the corner it is nearest, so a drag
    /// from anywhere in the window resizes it — the body has no free spot that does nothing.
    #[must_use]
    pub fn nearest(rect: Rect, x: i32, y: i32) -> Self {
        let horizontal = third(x, rect.x, rect.width);
        let vertical = third(y, rect.y, rect.height);
        match (horizontal, vertical) {
            (Third::Low, Third::Low) => Self::TopLeft,
            (Third::Low, Third::High) => Self::BottomLeft,
            (Third::High, Third::Low) => Self::TopRight,
            (Third::High, Third::High) => Self::BottomRight,
            (Third::Low, Third::Middle) => Self::Left,
            (Third::High, Third::Middle) => Self::Right,
            (Third::Middle, Third::Low) => Self::Top,
            (Third::Middle, Third::High) => Self::Bottom,
            (Third::Middle, Third::Middle) => Self::corner_of(rect, x, y),
        }
    }

    /// The corner of `rect` nearest `(x, y)`.
    fn corner_of(rect: Rect, x: i32, y: i32) -> Self {
        let left = x - rect.x <= rect.right() - 1 - x;
        let top = y - rect.y <= rect.bottom() - 1 - y;
        match (left, top) {
            (true, true) => Self::TopLeft,
            (true, false) => Self::BottomLeft,
            (false, true) => Self::TopRight,
            (false, false) => Self::BottomRight,
        }
    }
}

/// Which third of a span a coordinate falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Third {
    /// The near third.
    Low,
    /// The middle third.
    Middle,
    /// The far third.
    High,
}

/// Which third of the span that starts at `start` and is `length` cells long holds `value`.
///
/// A span of one or two cells has no middle: every cell is an edge, the nearer one.
fn third(value: i32, start: i32, length: u16) -> Third {
    let length = i32::from(length);
    let offset = (value - start).clamp(0, (length - 1).max(0));
    let edge = (length / 3).max(1);
    if offset < edge {
        Third::Low
    } else if offset >= length - edge {
        Third::High
    } else {
        Third::Middle
    }
}

/// `rect` with the edge or corner `grip` moved by `dx` columns and `dy` rows.
///
/// The edges that are not held stay where they are, the window never goes below the smallest size
/// of `area`, and the result stays inside `area`.
#[must_use]
pub fn resized(rect: Rect, grip: Grip, dx: i32, dy: i32, area: Rect) -> Rect {
    let min = min_size(area);
    let (mut left, mut top) = (rect.x, rect.y);
    let (mut right, mut bottom) = (rect.right(), rect.bottom());
    let min_width = i32::from(min.width);
    let min_height = i32::from(min.height);
    if grip.holds_left() {
        left = left.saturating_add(dx).clamp(area.x, (right - min_width).max(area.x));
    }
    if grip.holds_right() {
        right = right.saturating_add(dx).clamp((left + min_width).min(area.right()), area.right());
    }
    if grip.holds_top() {
        top = top.saturating_add(dy).clamp(area.y, (bottom - min_height).max(area.y));
    }
    if grip.holds_bottom() {
        bottom = bottom.saturating_add(dy).clamp((top + min_height).min(area.bottom()), area.bottom());
    }
    let width = u16::try_from(right - left).unwrap_or(u16::MAX);
    let height = u16::try_from(bottom - top).unwrap_or(u16::MAX);
    clamp_into(Rect::new(left, top, width, height), area)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A desktop area of a plain 80x24 terminal: the dock takes the last row.
    fn area() -> Rect {
        Rect::new(0, 0, 80, 23)
    }

    #[test]
    fn smallest_size_gives_way_to_a_tiny_terminal() {
        assert_eq!(min_size(area()), MIN_SIZE);
        assert_eq!(min_size(Rect::new(0, 0, 12, 3)), Size::new(12, 3));
    }

    #[test]
    fn a_window_without_a_wish_opens_at_two_thirds() {
        assert_eq!(opening_size(area(), None), Size::new(52, 14));
        assert_eq!(opening_size(area(), Some((30, 10))), Size::new(30, 10));
    }

    #[test]
    fn an_opening_size_stays_between_the_smallest_and_the_area() {
        assert_eq!(opening_size(area(), Some((4, 2))), MIN_SIZE);
        assert_eq!(opening_size(area(), Some((500, 500))), Size::new(80, 23));
        // Two thirds of a narrow area would be below the smallest size.
        assert_eq!(opening_size(Rect::new(0, 0, 24, 6), None), MIN_SIZE);
    }

    #[test]
    fn clamping_pushes_a_rectangle_in_instead_of_cutting_it() {
        assert_eq!(clamp_into(Rect::new(-5, -3, 30, 10), area()), Rect::new(0, 0, 30, 10));
        assert_eq!(clamp_into(Rect::new(70, 20, 30, 10), area()), Rect::new(50, 13, 30, 10));
    }

    #[test]
    fn a_move_keeps_the_size_and_stops_at_the_edge() {
        let rect = Rect::new(10, 5, 30, 10);
        assert_eq!(moved(rect, 5, 2, area()), Rect::new(15, 7, 30, 10));
        assert_eq!(moved(rect, -100, -100, area()), Rect::new(0, 0, 30, 10));
        assert_eq!(moved(rect, 100, 100, area()), Rect::new(50, 13, 30, 10));
    }

    #[test]
    fn a_resize_moves_only_the_held_edges() {
        let rect = Rect::new(10, 5, 30, 10);
        assert_eq!(resized(rect, Grip::Right, 6, 3, area()), Rect::new(10, 5, 36, 10));
        assert_eq!(resized(rect, Grip::Bottom, 6, 3, area()), Rect::new(10, 5, 30, 13));
        assert_eq!(resized(rect, Grip::TopLeft, -4, -2, area()), Rect::new(6, 3, 34, 12));
        assert_eq!(resized(rect, Grip::BottomRight, 4, 2, area()), Rect::new(10, 5, 34, 12));
    }

    #[test]
    fn a_resize_stops_at_the_smallest_size() {
        let rect = Rect::new(10, 5, 30, 10);
        assert_eq!(resized(rect, Grip::Left, 100, 0, area()), Rect::new(20, 5, 20, 10));
        assert_eq!(resized(rect, Grip::Right, -100, 0, area()), Rect::new(10, 5, 20, 10));
        assert_eq!(resized(rect, Grip::Top, 100, 100, area()), Rect::new(10, 10, 30, 5));
        assert_eq!(resized(rect, Grip::Bottom, 0, -100, area()), Rect::new(10, 5, 30, 5));
    }

    #[test]
    fn a_resize_stops_at_the_edge_of_the_area() {
        let rect = Rect::new(10, 5, 30, 10);
        assert_eq!(resized(rect, Grip::Left, -100, 0, area()), Rect::new(0, 5, 40, 10));
        assert_eq!(resized(rect, Grip::BottomRight, 100, 100, area()), Rect::new(10, 5, 70, 18));
    }

    #[test]
    fn a_grip_is_the_edge_or_corner_the_drag_started_nearest() {
        let rect = Rect::new(10, 5, 30, 12);
        assert_eq!(Grip::nearest(rect, 11, 6), Grip::TopLeft);
        assert_eq!(Grip::nearest(rect, 38, 15), Grip::BottomRight);
        assert_eq!(Grip::nearest(rect, 25, 6), Grip::Top);
        assert_eq!(Grip::nearest(rect, 11, 10), Grip::Left);
        assert_eq!(Grip::nearest(rect, 38, 10), Grip::Right);
        assert_eq!(Grip::nearest(rect, 25, 15), Grip::Bottom);
    }

    #[test]
    fn the_middle_of_a_window_holds_the_nearest_corner() {
        let rect = Rect::new(0, 0, 30, 12);
        assert_eq!(Grip::nearest(rect, 14, 5), Grip::TopLeft);
        assert_eq!(Grip::nearest(rect, 16, 7), Grip::BottomRight);
    }

    #[test]
    fn the_smallest_window_has_no_middle_to_hold() {
        let rect = Rect::new(0, 0, 2, 2);
        assert_eq!(Grip::nearest(rect, 0, 0), Grip::TopLeft);
        assert_eq!(Grip::nearest(rect, 1, 1), Grip::BottomRight);
    }
}
