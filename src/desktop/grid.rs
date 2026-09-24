//! Where the desktop icons sit: a pure grid, without a screen.
//!
//! An icon is a cell of [`CELL_WIDTH`] columns and [`CELL_HEIGHT`] rows: the icon itself, its
//! name below it, and a row of floor under both so two icons never touch. The floor is a grid of
//! such cells, and an icon may stand in any of them (design 3.2): one the person put somewhere
//! stays there, and one that has no place of its own flows into the first free cell, filling a
//! column from the top before the next column starts, because the floor is read down its left
//! edge and the dock is at the bottom.
//!
//! Nothing here draws or remembers anything. The grid is built again for every frame from the
//! area it has and the places the icons want, so a resized terminal needs no state to be kept: a
//! place that does not fit this frame is kept in the file and comes back when the room does.

use qframe::geometry::{Rect, Size};
use qframe::text;

/// Columns one icon takes.
pub const CELL_WIDTH: u16 = 10;
/// Rows one icon takes: the icon, its name, and a free row.
pub const CELL_HEIGHT: u16 = 3;
/// The column of a cell kept for the selection pillar, so the icon and the name stand in the same
/// place whether the icon is selected or not.
pub const PILLAR_WIDTH: u16 = 1;

/// A cell of the floor: its column and its row, from the top left.
pub type Cell = (u16, u16);

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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grid {
    columns: u16,
    rows: u16,
    cells: Vec<Option<Cell>>,
    /// The cells the gadgets take: no icon stands or lands in them.
    blocked: Vec<Cell>,
}

impl Grid {
    /// The grid `count` icons with no place of their own make in `area`: they flow from the top
    /// left, down the first column and then the next.
    #[must_use]
    pub fn new(area: Size, count: usize) -> Self {
        Self::placed(area, &vec![None; count])
    }

    /// The grid of icons that want the cells `wanted`, one for each icon in its order; `None` is
    /// an icon with no place of its own.
    ///
    /// An icon whose cell is free and inside the area stands there. One whose cell is outside the
    /// area — a terminal made smaller than when it was put there — or already taken stands in the
    /// free cell nearest to it, and its own place is not forgotten, because the grid remembers
    /// nothing. The icons with no place fill the cells left, in order. An area too small for a
    /// whole cell holds no icons at all: half an icon is worse than none, and the narrow screen of
    /// the design (3.7) hides them anyway. Icons for which no cell is left are not drawn.
    #[must_use]
    pub fn placed(area: Size, wanted: &[Option<Cell>]) -> Self {
        Self::placed_around(area, wanted, &[])
    }

    /// The grid of icons that want the cells `wanted`, around the cells `blocked` that the
    /// gadgets take (design 3.12).
    ///
    /// A blocked cell is as good as an icon's for the icons: one whose own place is blocked stands
    /// in the nearest free cell and keeps its place, and the icons with no place flow past the
    /// blocked cells. A blocked cell outside the area blocks nothing.
    #[must_use]
    pub fn placed_around(area: Size, wanted: &[Option<Cell>], blocked: &[Cell]) -> Self {
        let columns = area.width / CELL_WIDTH;
        let rows = area.height / CELL_HEIGHT;
        let blocked: Vec<Cell> =
            blocked.iter().copied().filter(|(column, row)| *column < columns && *row < rows).collect();
        let mut grid = Self { columns, rows, cells: vec![None; wanted.len()], blocked };
        if columns == 0 || rows == 0 {
            return grid;
        }
        let mut taken = vec![false; usize::from(columns) * usize::from(rows)];
        let slot = |(column, row): Cell| usize::from(column) * usize::from(rows) + usize::from(row);
        for cell in &grid.blocked {
            taken[slot(*cell)] = true;
        }
        // Places that fit and are free, first come first served in the order of the icons.
        for (index, place) in wanted.iter().enumerate() {
            if let Some(cell) = place.filter(|(column, row)| *column < columns && *row < rows)
                && !taken[slot(cell)]
            {
                taken[slot(cell)] = true;
                grid.cells[index] = Some(cell);
            }
        }
        // Places that do not fit this frame, or that another icon already holds: the nearest free
        // cell to where the icon wants to be, counted in cells, its own column first on a tie.
        for (index, place) in wanted.iter().enumerate() {
            let Some((column, row)) = *place else { continue };
            if grid.cells[index].is_some() {
                continue;
            }
            let aim = (column.min(columns - 1), row.min(rows - 1));
            let nearest = (0..columns)
                .flat_map(|column| (0..rows).map(move |row| (column, row)))
                .filter(|cell| !taken[slot(*cell)])
                .min_by_key(|(column, row)| {
                    (column.abs_diff(aim.0) + row.abs_diff(aim.1), column.abs_diff(aim.0), *column, *row)
                });
            if let Some(cell) = nearest {
                taken[slot(cell)] = true;
                grid.cells[index] = Some(cell);
            }
        }
        // The icons with no place of their own flow into the cells left.
        let mut free = (0..columns).flat_map(|column| (0..rows).map(move |row| (column, row)));
        for (index, place) in wanted.iter().enumerate() {
            if place.is_some() {
                continue;
            }
            if let Some(cell) = free.by_ref().find(|cell| !taken[slot(*cell)]) {
                taken[slot(cell)] = true;
                grid.cells[index] = Some(cell);
            }
        }
        grid
    }

    /// Whether a gadget takes `cell`, so no icon may be put there.
    #[must_use]
    pub fn is_blocked(&self, cell: Cell) -> bool {
        self.blocked.contains(&cell)
    }

    /// How many icons the area holds this frame.
    #[must_use]
    pub fn shown(&self) -> usize {
        self.cells.iter().filter(|cell| cell.is_some()).count()
    }

    /// How many icons were left out for want of room.
    #[must_use]
    pub fn hidden(&self) -> usize {
        self.cells.len() - self.shown()
    }

    /// The columns of cells the area holds.
    #[must_use]
    pub fn columns(&self) -> u16 {
        self.columns
    }

    /// The cells one column holds.
    #[must_use]
    pub fn rows(&self) -> u16 {
        self.rows
    }

    /// The cell of every icon, in their order; `None` for one that is not drawn.
    #[must_use]
    pub fn cells(&self) -> &[Option<Cell>] {
        &self.cells
    }

    /// The column and row of the icon at `index`.
    #[must_use]
    pub fn cell(&self, index: usize) -> Option<Cell> {
        self.cells.get(index).copied().flatten()
    }

    /// Where the icon at `index` is drawn, in cells from the top left of the area.
    #[must_use]
    pub fn rect(&self, index: usize) -> Option<Rect> {
        self.cell(index).map(Self::cell_rect)
    }

    /// Where the cell `cell` is, in cells from the top left of the area.
    #[must_use]
    pub fn cell_rect((column, row): Cell) -> Rect {
        Rect::new(
            i32::from(column) * i32::from(CELL_WIDTH),
            i32::from(row) * i32::from(CELL_HEIGHT),
            CELL_WIDTH,
            CELL_HEIGHT,
        )
    }

    /// The cell of the area at `x`, `y`, whether an icon stands in it or not.
    #[must_use]
    pub fn cell_at(&self, x: i32, y: i32) -> Option<Cell> {
        if x < 0 || y < 0 {
            return None;
        }
        let column = u16::try_from(x / i32::from(CELL_WIDTH)).ok()?;
        let row = u16::try_from(y / i32::from(CELL_HEIGHT)).ok()?;
        (column < self.columns && row < self.rows).then_some((column, row))
    }

    /// The icon at `x`, `y` in the area, if the cell there holds one.
    #[must_use]
    pub fn at(&self, x: i32, y: i32) -> Option<usize> {
        let cell = self.cell_at(x, y)?;
        self.holding(cell)
    }

    /// The icon standing in `cell`.
    #[must_use]
    pub fn holding(&self, cell: Cell) -> Option<usize> {
        self.cells.iter().position(|held| *held == Some(cell))
    }

    /// The cell one step from `cell`, while it is still inside the area.
    #[must_use]
    pub fn beside(&self, (column, row): Cell, step: Step) -> Option<Cell> {
        let cell = match step {
            Step::Up => (column, row.checked_sub(1)?),
            Step::Down => (column, row + 1),
            Step::Left => (column.checked_sub(1)?, row),
            Step::Right => (column + 1, row),
        };
        (cell.0 < self.columns && cell.1 < self.rows).then_some(cell)
    }

    /// The icon a key reaches from `index`, or `index` itself when there is none that way.
    ///
    /// Up and down walk the icons of the column. Left and right go to the nearest column that has
    /// an icon and, in it, to the icon nearest the row, the upper one on a tie: icons may stand
    /// anywhere, so the next column is often shorter than this one, or has a gap at this row, and
    /// a key that did nothing there would feel broken.
    #[must_use]
    pub fn step(&self, index: usize, step: Step) -> usize {
        let Some((column, row)) = self.cell(index) else { return index };
        let placed = || self.cells.iter().enumerate().filter_map(|(index, cell)| cell.map(|cell| (index, cell)));
        let found = match step {
            Step::Up => placed().filter(|(_, (c, r))| *c == column && *r < row).max_by_key(|(_, (_, r))| *r),
            Step::Down => placed().filter(|(_, (c, r))| *c == column && *r > row).min_by_key(|(_, (_, r))| *r),
            Step::Left | Step::Right => placed()
                .filter(|(_, (c, _))| if step == Step::Left { *c < column } else { *c > column })
                .min_by_key(|(_, (c, r))| (c.abs_diff(column), r.abs_diff(row), *r)),
        };
        found.map_or(index, |(found, _)| found)
    }

    /// Where the icons at `indices` land when every one of them is carried `by` columns and rows,
    /// each with its index; the ones not drawn are left out.
    ///
    /// The group keeps its shape. A carry that would take any of them off the grid is cut short
    /// at the edge, the way a window stops against the side of the screen, so the group always
    /// lands whole: it stops rather than refuses.
    #[must_use]
    pub fn shifted(&self, indices: &[usize], (dx, dy): (i32, i32)) -> Vec<(usize, Cell)> {
        let drawn: Vec<(usize, Cell)> =
            indices.iter().filter_map(|index| self.cell(*index).map(|cell| (*index, cell))).collect();
        let columns = drawn.iter().map(|(_, (column, _))| i32::from(*column));
        let rows = drawn.iter().map(|(_, (_, row))| i32::from(*row));
        let (Some(left), Some(right), Some(top), Some(bottom)) =
            (columns.clone().min(), columns.max(), rows.clone().min(), rows.max())
        else {
            return Vec::new();
        };
        let dx = dx.clamp(-left, i32::from(self.columns) - 1 - right);
        let dy = dy.clamp(-top, i32::from(self.rows) - 1 - bottom);
        let carried = |at: u16, by: i32| u16::try_from(i32::from(at) + by).unwrap_or(at);
        drawn.into_iter().map(|(index, (column, row))| (index, (carried(column, dx), carried(row, dy)))).collect()
    }

    /// The icons whose cells the rectangle `band` touches: the rubber-band selection.
    #[must_use]
    pub fn inside(&self, band: Rect) -> Vec<usize> {
        (0..self.cells.len())
            .filter(|index| self.rect(*index).is_some_and(|cell| !cell.intersect(band).is_empty()))
            .collect()
    }

    /// The icon whose name starts with `letter`, after `from`, so pressing the same letter again
    /// walks through the icons that share it. `names` are the names as they are shown.
    #[must_use]
    pub fn jump(&self, names: &[String], from: Option<usize>, letter: char) -> Option<usize> {
        let wanted = lower(letter);
        let count = self.cells.len();
        let start = from.map_or(0, |index| index + 1).min(count);
        let starts_with = |index: &usize| {
            self.cell(*index).is_some()
                && names.get(*index).and_then(|name| name.chars().next()).map(lower) == Some(wanted)
        };
        (start..count).find(starts_with).or_else(|| (0..start).find(starts_with))
    }
}

/// The name as a cell shows it: whole when it fits, else cut with an ellipsis. The whole name is
/// what the tooltip says.
#[must_use]
pub fn shown_name(name: &str) -> String {
    text::truncate(name, CELL_WIDTH - PILLAR_WIDTH).into_owned()
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

    #[test]
    fn icons_without_a_place_fill_a_column_before_the_next_one_starts() {
        let grid = Grid::new(floor(), 9);
        assert_eq!((grid.columns(), grid.rows(), grid.shown()), (8, 7, 9));
        assert_eq!(grid.cell(0), Some((0, 0)));
        assert_eq!(grid.cell(6), Some((0, 6)));
        assert_eq!(grid.cell(7), Some((1, 0)));
        assert_eq!(grid.cell(8), Some((1, 1)));
        assert_eq!(grid.cell(9), None);
    }

    #[test]
    fn an_icon_with_a_place_stands_there_and_the_others_flow_around_it() {
        let grid = Grid::placed(floor(), &[None, Some((0, 0)), None, Some((5, 3))]);
        assert_eq!(grid.cell(1), Some((0, 0)), "the placed icon keeps its cell");
        assert_eq!(grid.cell(0), Some((0, 1)), "the first free cell goes to the first icon without one");
        assert_eq!(grid.cell(2), Some((0, 2)));
        assert_eq!(grid.cell(3), Some((5, 3)));
        assert_eq!(grid.at(52, 10), Some(3));
    }

    #[test]
    fn icons_flow_past_the_cells_a_gadget_takes_and_one_placed_there_stands_nearest() {
        let blocked = [(0, 0), (0, 1), (1, 0), (1, 1)];
        let grid = Grid::placed_around(floor(), &[None, None, Some((1, 1))], &blocked);
        assert_eq!(grid.cell(0), Some((0, 2)), "the first free cell of the first column");
        assert_eq!(grid.cell(1), Some((0, 3)));
        assert_eq!(grid.cell(2), Some((1, 2)), "the nearest free cell to its own place");
        assert!(grid.is_blocked((1, 1)) && !grid.is_blocked((2, 2)));
        // A blocked cell off the area blocks nothing.
        assert!(!Grid::placed_around(floor(), &[], &[(40, 40)]).is_blocked((40, 40)));
    }

    #[test]
    fn a_place_the_screen_is_too_small_for_goes_to_the_nearest_free_cell() {
        // Two columns and three rows: (7, 5) is far outside, (1, 2) is its nearest cell.
        let grid = Grid::placed(size(25, 11), &[Some((7, 5)), None]);
        assert_eq!(grid.cell(0), Some((1, 2)));
        assert_eq!(grid.cell(1), Some((0, 0)));
        // When the nearest cell is taken by an icon that fits there, the next nearest is used.
        let grid = Grid::placed(size(25, 11), &[Some((7, 5)), Some((1, 2))]);
        assert_eq!(grid.cell(1), Some((1, 2)), "the icon that fits keeps its own place");
        assert_eq!(grid.cell(0), Some((1, 1)));
    }

    #[test]
    fn two_icons_that_want_one_cell_do_not_stand_on_each_other() {
        let grid = Grid::placed(floor(), &[Some((2, 2)), Some((2, 2))]);
        assert_eq!(grid.cell(0), Some((2, 2)), "the first in the order keeps it");
        assert_ne!(grid.cell(1), Some((2, 2)));
        assert!(grid.cell(1).is_some());
    }

    #[test]
    fn a_cell_is_ten_by_three_from_the_top_left() {
        let grid = Grid::new(floor(), 9);
        assert_eq!(grid.rect(0), Some(Rect::new(0, 0, 10, 3)));
        assert_eq!(grid.rect(1), Some(Rect::new(0, 3, 10, 3)));
        assert_eq!(grid.rect(7), Some(Rect::new(10, 0, 10, 3)));
    }

    #[test]
    fn a_click_finds_the_icon_under_it_and_the_cell_of_bare_floor() {
        let grid = Grid::new(floor(), 9);
        assert_eq!(grid.at(0, 0), Some(0));
        assert_eq!(grid.at(9, 2), Some(0), "the whole cell is the icon's");
        assert_eq!(grid.at(3, 4), Some(1));
        assert_eq!(grid.at(12, 1), Some(7));
        assert_eq!(grid.at(12, 6), None, "the third cell of the second column is empty floor");
        assert_eq!(grid.cell_at(12, 6), Some((1, 2)), "but it is a cell an icon can be put in");
        assert_eq!(grid.at(70, 1), None);
        assert_eq!(grid.cell_at(79, 20), Some((7, 6)));
        assert_eq!(grid.cell_at(80, 1), None, "past the last whole column");
        assert_eq!(grid.cell_at(1, 21), None, "below the last whole row");
        assert_eq!(grid.at(-2, 1), None);
    }

    #[test]
    fn a_step_beside_a_cell_stays_inside_the_area() {
        let grid = Grid::new(size(25, 11), 1);
        assert_eq!(grid.beside((0, 0), Step::Up), None);
        assert_eq!(grid.beside((0, 0), Step::Left), None);
        assert_eq!(grid.beside((0, 0), Step::Right), Some((1, 0)));
        assert_eq!(grid.beside((1, 0), Step::Right), None);
        assert_eq!(grid.beside((1, 1), Step::Down), Some((1, 2)));
        assert_eq!(grid.beside((1, 2), Step::Down), None);
    }

    #[test]
    fn an_area_too_small_for_a_cell_holds_no_icons() {
        for area in [size(9, 23), size(80, 2), size(0, 0), size(9, 2)] {
            let grid = Grid::placed(area, &[None, None, Some((0, 0)), None, None]);
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
    fn a_key_into_a_shorter_column_lands_on_its_nearest_icon() {
        let grid = Grid::new(floor(), 9);
        // Row 4 of the first column; the second column ends at row 1.
        assert_eq!(grid.step(4, Step::Right), 8);
        assert_eq!(grid.step(8, Step::Left), 1);
    }

    #[test]
    fn a_key_crosses_empty_columns_and_gaps_to_the_next_icon() {
        let grid = Grid::placed(floor(), &[Some((0, 0)), Some((0, 4)), Some((5, 3))]);
        assert_eq!(grid.step(0, Step::Down), 1, "a gap in the column is crossed");
        assert_eq!(grid.step(1, Step::Up), 0);
        assert_eq!(grid.step(0, Step::Right), 2, "empty columns are crossed");
        assert_eq!(grid.step(2, Step::Left), 1, "the nearest row of the column reached");
        assert_eq!(grid.step(2, Step::Up), 2, "nothing above it in its column");
    }

    #[test]
    fn every_area_and_count_keeps_the_cells_inside_the_area_and_the_indices_unique() {
        for width in [0, 9, 10, 11, 25, 40, 80, 200] {
            for height in [0, 2, 3, 5, 11, 23, 49] {
                for count in [0, 1, 2, 7, 9, 40, 300] {
                    let area = size(width, height);
                    // Every third icon wants a place, some of them far outside any area.
                    let wanted: Vec<Option<Cell>> = (0..count)
                        .map(|n| {
                            (n % 3 == 0)
                                .then(|| (u16::try_from(n % 23).unwrap_or(0), u16::try_from(n % 17).unwrap_or(0)))
                        })
                        .collect();
                    let grid = Grid::placed(area, &wanted);
                    assert!(grid.shown() <= count);
                    let mut seen = Vec::new();
                    for index in 0..count {
                        let Some(cell) = grid.rect(index) else { continue };
                        assert!(cell.right() <= i32::from(width), "{area:?} {count}: {cell:?}");
                        assert!(cell.bottom() <= i32::from(height), "{area:?} {count}: {cell:?}");
                        assert_eq!(grid.at(cell.x, cell.y), Some(index));
                        assert!(!seen.contains(&(cell.x, cell.y)), "two icons in one cell");
                        seen.push((cell.x, cell.y));
                        for step in [Step::Up, Step::Down, Step::Left, Step::Right] {
                            assert!(grid.cell(grid.step(index, step)).is_some());
                        }
                    }
                    let cells = usize::from(width / CELL_WIDTH) * usize::from(height / CELL_HEIGHT);
                    assert_eq!(grid.shown(), count.min(cells), "{area:?} {count}: every free cell is used");
                }
            }
        }
    }

    #[test]
    fn a_carried_group_keeps_its_shape_and_stops_at_the_edge() {
        // Nine icons on an 80x24 floor: three columns of seven, then two in the second column.
        let grid = Grid::new(floor(), 9);
        assert_eq!(grid.shifted(&[0, 1], (2, 1)), vec![(0, (2, 1)), (1, (2, 2))]);
        // Pulled past the top and the left, the group stops against them, whole.
        assert_eq!(grid.shifted(&[1, 2], (-3, -5)), vec![(1, (0, 0)), (2, (0, 1))]);
        // Pushed past the right and the bottom, the same.
        let (columns, rows) = (grid.columns(), grid.rows());
        assert_eq!(grid.shifted(&[0, 1], (99, 99)), vec![(0, (columns - 1, rows - 2)), (1, (columns - 1, rows - 1))]);
        assert!(grid.shifted(&[42], (1, 1)).is_empty(), "an icon that is not drawn is not carried");
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
}
