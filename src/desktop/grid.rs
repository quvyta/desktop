//! Where the desktop icons sit: a pure grid, without a screen.
//!
//! An icon is a cell of [`CELL_WIDTH`] columns and [`CELL_HEIGHT`] rows: the icon itself, its
//! name below it, and a row of floor under both so two icons never touch. Cells fill a column
//! from the top before the next column starts, because the floor is read down its left edge and
//! the dock is at the bottom: a new icon should appear under the last one, not beside it.
//!
//! Nothing here draws or remembers anything. The grid is built again for every frame from the
//! area it has and the number of icons, so a resized terminal needs no state to be kept.

use qframe::geometry::{Rect, Size};
use qframe::text;

/// Columns one icon takes.
pub const CELL_WIDTH: u16 = 10;
/// Rows one icon takes: the icon, its name, and a free row.
pub const CELL_HEIGHT: u16 = 3;
/// The column of a cell kept for the selection pillar, so the icon and the name stand in the same
/// place whether the icon is selected or not.
pub const PILLAR_WIDTH: u16 = 1;

/// Which way a key moves the selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    /// One cell up, inside the column.
    Up,
    /// One cell down, inside the column.
    Down,
    /// One column left, keeping the row.
    Left,
    /// One column right, keeping the row.
    Right,
}

/// The cells of the desktop icons in the area they have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Grid {
    columns: u16,
    rows: u16,
    shown: usize,
    count: usize,
}

impl Grid {
    /// The grid `count` icons make in `area`.
    ///
    /// The icons fill the first column, then the next one. An area too small for a whole cell
    /// holds no icons at all: half an icon is worse than none, and the narrow screen of the
    /// design (3.7) hides them anyway.
    #[must_use]
    pub fn new(area: Size, count: usize) -> Self {
        let rows = area.height / CELL_HEIGHT;
        let fit = area.width / CELL_WIDTH;
        if rows == 0 || fit == 0 || count == 0 {
            return Self { columns: 0, rows, shown: 0, count };
        }
        let per_column = usize::from(rows);
        let needed = count.div_ceil(per_column);
        let columns = u16::try_from(needed).unwrap_or(u16::MAX).min(fit);
        let shown = count.min(usize::from(columns) * per_column);
        Self { columns, rows, shown, count }
    }

    /// How many icons the area holds: the rest have nowhere to stand this frame.
    #[must_use]
    pub fn shown(&self) -> usize {
        self.shown
    }

    /// Whether an icon was left out for want of room.
    #[must_use]
    pub fn hidden(&self) -> usize {
        self.count - self.shown
    }

    /// The columns of cells in use.
    #[must_use]
    pub fn columns(&self) -> u16 {
        self.columns
    }

    /// The cells one column holds.
    #[must_use]
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// The column and row of the icon at `index`.
    #[must_use]
    pub fn cell(&self, index: usize) -> Option<(u16, u16)> {
        if index >= self.shown {
            return None;
        }
        let per_column = usize::from(self.rows);
        let column = u16::try_from(index / per_column).unwrap_or(u16::MAX);
        let row = u16::try_from(index % per_column).unwrap_or(u16::MAX);
        Some((column, row))
    }

    /// Where the icon at `index` is drawn, in cells from the top left of the area.
    #[must_use]
    pub fn rect(&self, index: usize) -> Option<Rect> {
        let (column, row) = self.cell(index)?;
        Some(Rect::new(
            i32::from(column) * i32::from(CELL_WIDTH),
            i32::from(row) * i32::from(CELL_HEIGHT),
            CELL_WIDTH,
            CELL_HEIGHT,
        ))
    }

    /// The icon at `x`, `y` in the area, if the cell there holds one.
    #[must_use]
    pub fn at(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 {
            return None;
        }
        let column = u16::try_from(x / i32::from(CELL_WIDTH)).ok()?;
        let row = u16::try_from(y / i32::from(CELL_HEIGHT)).ok()?;
        if column >= self.columns || row >= self.rows {
            return None;
        }
        let index = usize::from(column) * usize::from(self.rows) + usize::from(row);
        (index < self.shown).then_some(index)
    }

    /// The icon a key reaches from `index`, or `index` itself at the edge of the grid.
    ///
    /// Left and right keep the row when the next column has one that far down; the last column
    /// of a grid whose icons do not fill it is shorter, so the row moves up to its last icon
    /// instead of the key doing nothing.
    #[must_use]
    pub fn step(&self, index: usize, step: Step) -> usize {
        let Some((column, row)) = self.cell(index) else { return index };
        let moved = match step {
            Step::Up => (column, row.saturating_sub(1)),
            Step::Down => (column, (row + 1).min(self.rows.saturating_sub(1))),
            Step::Left => (column.saturating_sub(1), row),
            Step::Right => ((column + 1).min(self.columns.saturating_sub(1)), row),
        };
        let wanted = usize::from(moved.0) * usize::from(self.rows) + usize::from(moved.1);
        if wanted < self.shown {
            return wanted;
        }
        match step {
            // Down at the bottom of a column and right into a shorter column both aim past the
            // last icon; the nearest icon there is the last one, since only the last column of
            // the grid is short.
            Step::Down | Step::Right => self.shown - 1,
            Step::Up | Step::Left => index,
        }
    }

    /// The icons whose cells the rectangle `band` touches: the rubber-band selection.
    #[must_use]
    pub fn inside(&self, band: Rect) -> Vec<usize> {
        (0..self.shown).filter(|index| self.rect(*index).is_some_and(|cell| !cell.intersect(band).is_empty())).collect()
    }

    /// The icon whose name starts with `letter`, after `from`, so pressing the same letter again
    /// walks through the icons that share it. `names` are the names as they are shown.
    #[must_use]
    pub fn jump(&self, names: &[String], from: Option<usize>, letter: char) -> Option<usize> {
        let wanted = lower(letter);
        let start = from.map_or(0, |index| index + 1);
        let starts_with =
            |index: &usize| names.get(*index).and_then(|name| name.chars().next()).map(lower) == Some(wanted);
        (start..self.shown).find(starts_with).or_else(|| (0..start.min(self.shown)).find(starts_with))
    }
}

/// The name as a cell shows it: whole when it fits, else cut with an ellipsis. The whole name is
/// what the tooltip says.
#[must_use]
pub fn shown_name(name: &str) -> String {
    text::truncate(name, CELL_WIDTH - PILLAR_WIDTH).into_owned()
}

/// The order `ids` take when the icon at `from` is dropped on `to`.
#[must_use]
pub fn moved<T: Clone>(ids: &[T], from: usize, to: usize) -> Vec<T> {
    let mut order = ids.to_vec();
    if from >= order.len() || to >= order.len() || from == to {
        return order;
    }
    let icon = order.remove(from);
    order.insert(to, icon);
    order
}

/// One lower-case character for one, so a letter whose lower case is two characters (`İ`) still
/// answers its own key.
fn lower(letter: char) -> char {
    letter.to_lowercase().next().unwrap_or(letter)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: u16, height: u16) -> Size {
        Size { width, height }
    }

    /// The area of the floor on an 80x24 terminal: the dock takes the last row.
    fn floor() -> Size {
        size(80, 23)
    }

    fn names(count: usize) -> Vec<String> {
        (0..count).map(|n| format!("app-{n}")).collect()
    }

    #[test]
    fn icons_fill_a_column_before_the_next_one_starts() {
        let grid = Grid::new(floor(), 9);
        assert_eq!((grid.columns(), grid.rows(), grid.shown()), (2, 7, 9));
        assert_eq!(grid.cell(0), Some((0, 0)));
        assert_eq!(grid.cell(6), Some((0, 6)));
        assert_eq!(grid.cell(7), Some((1, 0)));
        assert_eq!(grid.cell(8), Some((1, 1)));
        assert_eq!(grid.cell(9), None);
    }

    #[test]
    fn a_cell_is_ten_by_three_from_the_top_left() {
        let grid = Grid::new(floor(), 9);
        assert_eq!(grid.rect(0), Some(Rect::new(0, 0, 10, 3)));
        assert_eq!(grid.rect(1), Some(Rect::new(0, 3, 10, 3)));
        assert_eq!(grid.rect(7), Some(Rect::new(10, 0, 10, 3)));
    }

    #[test]
    fn a_click_finds_the_icon_under_it_and_the_floor_between_columns() {
        let grid = Grid::new(floor(), 9);
        assert_eq!(grid.at(0, 0), Some(0));
        assert_eq!(grid.at(9, 2), Some(0), "the whole cell is the icon's");
        assert_eq!(grid.at(3, 4), Some(1));
        assert_eq!(grid.at(12, 1), Some(7));
        assert_eq!(grid.at(12, 6), None, "the third cell of the second column is empty floor");
        assert_eq!(grid.at(70, 1), None);
        assert_eq!(grid.at(-2, 1), None);
    }

    #[test]
    fn an_area_too_small_for_a_cell_holds_no_icons() {
        for area in [size(9, 23), size(80, 2), size(0, 0), size(9, 2)] {
            let grid = Grid::new(area, 5);
            assert_eq!(grid.shown(), 0, "{area:?}");
            assert_eq!(grid.hidden(), 5);
            assert_eq!(grid.at(0, 0), None);
            assert_eq!(grid.rect(0), None);
        }
    }

    #[test]
    fn icons_that_do_not_fit_are_left_out_and_counted() {
        // Three rows of cells, two columns of ten: six icons fit, four do not.
        let grid = Grid::new(size(25, 11), 10);
        assert_eq!((grid.columns(), grid.rows(), grid.shown(), grid.hidden()), (2, 3, 6, 4));
    }

    #[test]
    fn arrow_keys_stay_inside_the_grid() {
        let grid = Grid::new(floor(), 9);
        assert_eq!(grid.step(0, Step::Up), 0);
        assert_eq!(grid.step(0, Step::Left), 0);
        assert_eq!(grid.step(0, Step::Down), 1);
        assert_eq!(grid.step(0, Step::Right), 7);
        assert_eq!(grid.step(6, Step::Down), 6, "the bottom of a full column");
        assert_eq!(grid.step(8, Step::Right), 8, "the last column");
        assert_eq!(grid.step(7, Step::Left), 0);
    }

    #[test]
    fn a_key_into_a_shorter_column_lands_on_its_last_icon() {
        let grid = Grid::new(floor(), 9);
        // Row 4 of the first column; the second column ends at row 1.
        assert_eq!(grid.step(4, Step::Right), 8);
        assert_eq!(grid.step(8, Step::Left), 1);
    }

    #[test]
    fn every_area_and_count_keeps_the_cells_inside_the_area_and_the_indices_unique() {
        for width in [0, 9, 10, 11, 25, 40, 80, 200] {
            for height in [0, 2, 3, 5, 11, 23, 49] {
                for count in [0, 1, 2, 7, 9, 40, 300] {
                    let area = size(width, height);
                    let grid = Grid::new(area, count);
                    assert!(grid.shown() <= count);
                    let mut seen = Vec::new();
                    for index in 0..grid.shown() {
                        let cell = grid.rect(index).expect("a shown icon has a cell");
                        assert!(cell.right() <= i32::from(width), "{area:?} {count}: {cell:?}");
                        assert!(cell.bottom() <= i32::from(height), "{area:?} {count}: {cell:?}");
                        assert_eq!(grid.at(cell.x, cell.y), Some(index));
                        assert!(!seen.contains(&(cell.x, cell.y)), "two icons in one cell");
                        seen.push((cell.x, cell.y));
                        for step in [Step::Up, Step::Down, Step::Left, Step::Right] {
                            assert!(grid.step(index, step) < grid.shown().max(1));
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn a_band_takes_every_icon_it_touches() {
        let grid = Grid::new(floor(), 9);
        assert_eq!(grid.inside(Rect::new(0, 0, 1, 1)), vec![0]);
        assert_eq!(grid.inside(Rect::new(0, 0, 10, 7)), vec![0, 1, 2]);
        assert_eq!(grid.inside(Rect::new(5, 1, 10, 4)), vec![0, 1, 7, 8]);
        assert!(grid.inside(Rect::new(40, 0, 10, 10)).is_empty(), "empty floor selects nothing");
    }

    #[test]
    fn a_letter_jumps_to_the_next_icon_that_starts_with_it() {
        let grid = Grid::new(floor(), 4);
        let names = vec!["Terminal".to_owned(), "htop".to_owned(), "Home".to_owned(), "Settings".to_owned()];
        assert_eq!(grid.jump(&names, None, 'h'), Some(1));
        assert_eq!(grid.jump(&names, Some(1), 'h'), Some(2), "the same letter walks on");
        assert_eq!(grid.jump(&names, Some(2), 'h'), Some(1), "and comes round");
        assert_eq!(grid.jump(&names, None, 'T'), Some(0), "case is ignored");
        assert_eq!(grid.jump(&names, None, 'z'), None);
    }

    #[test]
    fn a_long_name_is_cut_with_an_ellipsis_and_a_short_one_is_not() {
        assert_eq!(shown_name("htop"), "htop");
        assert_eq!(shown_name("Midnight Commander"), format!("Midnight{}", text::ELLIPSIS));
        assert_eq!(text::width(&shown_name("Midnight Commander")), CELL_WIDTH - PILLAR_WIDTH);
        assert_eq!(shown_name("伺服器東京三號機"), format!("伺服器東{}", text::ELLIPSIS));
        assert!(text::width(&shown_name("伺服器東京三號機")) <= CELL_WIDTH - PILLAR_WIDTH);
    }

    #[test]
    fn dropping_an_icon_moves_it_inside_the_order() {
        let ids = names(4);
        assert_eq!(moved(&ids, 0, 2), vec!["app-1", "app-2", "app-0", "app-3"]);
        assert_eq!(moved(&ids, 3, 0), vec!["app-3", "app-0", "app-1", "app-2"]);
        assert_eq!(moved(&ids, 1, 1), ids, "a drop on itself changes nothing");
        assert_eq!(moved(&ids, 9, 0), ids, "an index that is not there changes nothing");
    }
}
