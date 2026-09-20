//! The one-shot tiling: one large window on the left, the rest stacked on the right.
//!
//! This is a placement, not a mode. It works out rectangles for a number of windows and hands
//! them back; nothing here remembers that it happened, so a window dragged afterwards simply
//! floats again.

use qframe::geometry::{Rect, Size};

use super::layout::min_size;

/// Rows and columns of desktop floor left between two tiled windows. Windows are drawn without
/// borders, so this gap is the only thing that keeps two of them apart.
pub const GAP: u16 = 1;

/// Why the windows could not be tiled.
///
/// The desktop says so instead of tiling: broken rectangles, windows below the smallest size or
/// windows over each other would all be worse than leaving the layout alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TooSmall {
    /// The desktop is not wide enough for two columns of windows.
    Narrow,
    /// The desktop is not tall enough for this many windows; `fits` is how many it would hold.
    Short {
        /// The largest number of windows this desktop can tile.
        fits: usize,
    },
}

/// The rectangles for `count` tiled windows in `area`, the large one first and the stacked ones
/// after it from top to bottom.
///
/// A single window takes the whole desktop. From two on, the desktop is split into a left column
/// that is half of it (an odd column going to the left, the large one) and a right column holding
/// the rest, one above the other with [`GAP`] between them. Rows left over by the division go to
/// the topmost windows of the stack, so the sizes differ by at most one row and the result is the
/// same every time for the same desktop and count.
///
/// # Errors
///
/// Returns [`TooSmall`] when the desktop cannot hold this many windows at the smallest window
/// size.
pub fn rects(area: Rect, count: usize) -> Result<Vec<Rect>, TooSmall> {
    let min = min_size(area);
    if count == 0 {
        return Ok(Vec::new());
    }
    if count == 1 {
        return Ok(vec![area]);
    }
    let room = area.width.saturating_sub(GAP);
    let right_width = room / 2;
    let left_width = room - right_width;
    if right_width < min.width {
        return Err(TooSmall::Narrow);
    }
    let stacked = count - 1;
    let gaps = u16::try_from(stacked - 1).unwrap_or(u16::MAX).saturating_mul(GAP);
    let rows = area.height.saturating_sub(gaps);
    let wanted = u16::try_from(stacked).unwrap_or(u16::MAX).saturating_mul(min.height);
    if rows < wanted || gaps >= area.height {
        return Err(TooSmall::Short { fits: fits(area, min) });
    }
    let mut rects = Vec::with_capacity(count);
    rects.push(Rect::new(area.x, area.y, left_width, area.height));
    let stack_x = area.x.saturating_add(i32::from(left_width)).saturating_add(i32::from(GAP));
    let each = rows / u16::try_from(stacked).unwrap_or(u16::MAX);
    let extra = usize::from(rows % u16::try_from(stacked).unwrap_or(u16::MAX));
    let mut y = area.y;
    for index in 0..stacked {
        let height = each + u16::from(index < extra);
        rects.push(Rect::new(stack_x, y, right_width, height));
        y = y.saturating_add(i32::from(height)).saturating_add(i32::from(GAP));
    }
    Ok(rects)
}

/// How many windows `area` can tile at the smallest size `min`: the large one plus the stack the
/// height allows.
fn fits(area: Rect, min: Size) -> usize {
    let step = min.height.saturating_add(GAP);
    let stacked = area.height.saturating_add(GAP).checked_div(step).unwrap_or(0);
    1 + usize::from(stacked)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The desktop area of an 80x24 terminal.
    fn area() -> Rect {
        Rect::new(0, 0, 80, 23)
    }

    /// Whether the rectangles cover no cell twice.
    fn disjoint(rects: &[Rect]) -> bool {
        rects
            .iter()
            .enumerate()
            .all(|(index, one)| rects[index + 1..].iter().all(|other| one.intersect(*other).is_empty()))
    }

    #[test]
    fn nothing_to_tile_is_no_layout() {
        assert_eq!(rects(area(), 0), Ok(Vec::new()));
    }

    #[test]
    fn one_window_takes_the_whole_desktop() {
        assert_eq!(rects(area(), 1), Ok(vec![area()]));
    }

    #[test]
    fn two_windows_share_the_desktop_with_a_gap_between_them() {
        let two = rects(area(), 2).expect("fits");
        assert_eq!(two, vec![Rect::new(0, 0, 40, 23), Rect::new(41, 0, 39, 23)]);
        assert_eq!(two[1].x - two[0].right(), i32::from(GAP));
    }

    #[test]
    fn three_windows_stack_on_the_right() {
        let three = rects(area(), 3).expect("fits");
        assert_eq!(three, vec![Rect::new(0, 0, 40, 23), Rect::new(41, 0, 39, 11), Rect::new(41, 12, 39, 11)]);
        assert!(disjoint(&three));
    }

    #[test]
    fn spare_rows_go_to_the_top_of_the_stack() {
        let four = rects(area(), 4).expect("fits");
        let heights: Vec<u16> = four[1..].iter().map(|rect| rect.height).collect();
        assert_eq!(heights, vec![7, 7, 7]);
        let five = rects(area(), 5).expect("fits");
        let heights: Vec<u16> = five[1..].iter().map(|rect| rect.height).collect();
        assert_eq!(heights, vec![5, 5, 5, 5]);
        assert!(disjoint(&five));
    }

    #[test]
    fn every_count_that_fits_stays_inside_and_apart() {
        for count in 1..=5 {
            let tiled = rects(area(), count).expect("fits");
            assert_eq!(tiled.len(), count);
            assert!(disjoint(&tiled));
            for rect in tiled {
                assert_eq!(rect.intersect(area()), rect);
                assert!(rect.width >= min_size(area()).width && rect.height >= min_size(area()).height);
            }
        }
    }

    #[test]
    fn a_narrow_desktop_says_so_instead_of_tiling() {
        let narrow = Rect::new(0, 0, 40, 23);
        assert_eq!(rects(narrow, 2), Err(TooSmall::Narrow));
        assert_eq!(rects(narrow, 1), Ok(vec![narrow]));
        assert_eq!(rects(Rect::new(0, 0, 41, 23), 2).map(|rects| rects.len()), Ok(2));
    }

    #[test]
    fn a_short_desktop_says_how_many_it_holds() {
        assert_eq!(rects(area(), 6), Err(TooSmall::Short { fits: 5 }));
        assert_eq!(rects(area(), 20), Err(TooSmall::Short { fits: 5 }));
        assert!(rects(area(), 5).is_ok());
    }

    #[test]
    fn the_count_a_desktop_holds_is_the_count_it_tiles() {
        for height in 5..40 {
            let area = Rect::new(0, 0, 80, height);
            let holds = fits(area, min_size(area));
            assert!(rects(area, holds).is_ok(), "{height} rows should tile {holds} windows");
            assert!(rects(area, holds + 1).is_err(), "{height} rows should not tile {} windows", holds + 1);
        }
    }
}
