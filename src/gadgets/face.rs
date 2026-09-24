//! How a gadget is drawn: a surface one tone above the floor with its content in the middle, and
//! the two pieces of content the framework has no widget for, a month and a named reading.
//!
//! No line, frame or title strip anywhere (VISION §4): the surface's tone is the gadget's shape,
//! and a gadget keeps generous room between its edge and what it shows.

use qframe::date::{Date, Weekday};
use qframe::geometry::{Rect, Size};
use qframe::style::CellStyle;
use qframe::text;
use qframe::widget::{Container, MeasureCx, Node, PaintCx, Widget};

use crate::desktop::grid::{CELL_HEIGHT, CELL_WIDTH};

/// Rows between the surface's edge and its content, above and below.
pub const PAD_ROWS: u16 = 1;
/// Columns between the surface's edge and its content, left and right.
pub const PAD_COLUMNS: u16 = 2;

/// The rectangle of the screen a gadget taking `columns` by `rows` icon cells, with its top left
/// cell at `x`, `y`, draws its surface in: its cells without their last column and last row, so
/// two gadgets side by side, or a gadget beside an icon, never touch.
#[must_use]
pub fn surface_rect(x: i32, y: i32, (columns, rows): (u16, u16)) -> Rect {
    let width = columns.saturating_mul(CELL_WIDTH).saturating_sub(1);
    let height = rows.saturating_mul(CELL_HEIGHT).saturating_sub(1);
    Rect::new(x, y, width, height)
}

/// The surface of a gadget, with its content stacked in the middle.
///
/// Each child takes the rows it measures; the stack is centred up and down, and each child left
/// and right, so a clock's digits and its date sit on one axis whatever their widths. A surface
/// made with [`Surface::filled`] gives its one child all the room inside its padding instead: a
/// note's text area.
pub struct Surface<Msg> {
    gap: u16,
    filled: bool,
    children: Vec<Node<Msg>>,
}

impl<Msg: 'static> Surface<Msg> {
    /// A surface whose children stand `gap` rows apart in its middle.
    #[must_use]
    pub fn new(gap: u16) -> Self {
        Self { gap, filled: false, children: Vec::new() }
    }

    /// A surface whose one child takes all the room inside its padding.
    #[must_use]
    pub fn filled() -> Self {
        Self { gap: 0, filled: true, children: Vec::new() }
    }

    fn inner(area: Rect) -> Rect {
        Rect::new(
            area.x + i32::from(PAD_COLUMNS),
            area.y + i32::from(PAD_ROWS),
            area.width.saturating_sub(PAD_COLUMNS * 2),
            area.height.saturating_sub(PAD_ROWS * 2),
        )
    }
}

impl<Msg: 'static> Widget<Msg> for Surface<Msg> {
    fn measure(&self, _cx: &mut MeasureCx<'_>, available: Size) -> Size {
        available
    }

    fn paint(&self, cx: &mut PaintCx<'_>, area: Rect) {
        if area.is_empty() {
            return;
        }
        // The panel's own tone, so a gadget is the same surface a panel is in every theme.
        let tone = cx.style("panel", None, &[]).text().bg.unwrap_or_else(|| cx.color("surface"));
        cx.clear(area, tone);
        cx.register_hit(area);
        let inner = Self::inner(area);
        if inner.is_empty() {
            return;
        }
        if self.filled {
            if let Some(child) = self.children.first() {
                cx.paint_child(child, inner);
            }
            return;
        }
        let sizes: Vec<Size> =
            self.children.iter().map(|child| cx.measure_child(child, inner.size()).min(inner.size())).collect();
        let gaps = self.gap.saturating_mul(u16::try_from(sizes.len().saturating_sub(1)).unwrap_or(u16::MAX));
        let tall = sizes.iter().map(|size| size.height).fold(gaps, u16::saturating_add).min(inner.height);
        let mut y = inner.y + i32::from((inner.height - tall) / 2);
        for (child, size) in self.children.iter().zip(sizes) {
            let bottom = inner.y + i32::from(inner.height);
            if y >= bottom {
                break;
            }
            let height = size.height.min(u16::try_from(bottom - y).unwrap_or(0));
            let x = inner.x + i32::from(inner.width.saturating_sub(size.width) / 2);
            cx.paint_child(child, Rect::new(x, y, size.width, height));
            y += i32::from(size.height) + i32::from(self.gap);
        }
    }

    fn children(&self) -> &[Node<Msg>] {
        &self.children
    }

    fn children_mut(&mut self) -> &mut [Node<Msg>] {
        &mut self.children
    }
}

impl<Msg: 'static> Container<Msg> for Surface<Msg> {
    fn set_children(&mut self, children: Vec<Node<Msg>>) {
        self.children = children;
    }
}

/// Columns one day takes in a month: two digits and a space.
const DAY_WIDTH: u16 = 3;
/// Rows a month takes: its name, a free row, the weekdays and six weeks.
pub const MONTH_ROWS: u16 = 9;
/// How much of the accent today's cell mixes into the surface: the tone of a selected day, one
/// step above the surface, under the accent-coloured number.
const TODAY_MIX: f32 = 0.22;

/// The month of `today`, as a calendar shows it: its name and year, the weekdays starting on
/// `first`, and its days, today's in the accent on a raised cell.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Month {
    today: Date,
    first: Weekday,
}

impl Month {
    /// The month holding `today`, its weeks starting on `first`.
    #[must_use]
    pub fn new(today: Date, first: Weekday) -> Self {
        Self { today, first }
    }

    /// The weeks of the month, each seven days from `first`; `None` for a day of the month before
    /// or after.
    #[must_use]
    pub fn weeks(&self) -> Vec<[Option<u8>; 7]> {
        let start = self.today.first_of_month();
        let lead = usize::from(start.weekday().days_since(self.first));
        let days = qframe::date::days_in_month(start.year(), start.month());
        let mut weeks = Vec::new();
        let mut week = [None; 7];
        for day in 1..=days {
            let at = (lead + usize::from(day) - 1) % 7;
            week[at] = Some(day);
            if at == 6 {
                weeks.push(week);
                week = [None; 7];
            }
        }
        if week.iter().any(Option::is_some) {
            weeks.push(week);
        }
        weeks
    }

    fn width() -> u16 {
        DAY_WIDTH * 7 - 1
    }
}

impl<Msg: 'static> Widget<Msg> for Month {
    fn measure(&self, _cx: &mut MeasureCx<'_>, available: Size) -> Size {
        Size::new(Self::width(), MONTH_ROWS).min(available)
    }

    fn paint(&self, cx: &mut PaintCx<'_>, area: Rect) {
        if area.is_empty() {
            return;
        }
        let surface = cx.style("panel", None, &[]).text().bg.unwrap_or_else(|| cx.color("surface"));
        let title = qframe::t!(
            "quvyta.date.title",
            month = qframe::t!(&format!("quvyta.date.month-{}", self.today.month())).as_str(),
            year = self.today.year()
        );
        let centred = |text: &str| area.x + i32::from(area.width.saturating_sub(text::width(text)) / 2);
        let heading = CellStyle::fg(cx.color("text")).with_bold(true);
        cx.text(centred(&title), area.y, &title, heading, area.width);
        let dim = CellStyle::fg(cx.color("dim"));
        for column in 0..7_u8 {
            let day = Weekday::from_number((self.first.number() - 1 + column) % 7 + 1).unwrap_or(Weekday::Monday);
            let name = qframe::t!(&format!("quvyta.date.weekday-{}", day.number()));
            let name = text::truncate(&name, 2).into_owned();
            let x = area.x + i32::from(u16::from(column) * DAY_WIDTH);
            cx.text(x + i32::from(2_u16.saturating_sub(text::width(&name))), area.y + 2, &name, dim, 2);
        }
        let plain = CellStyle::fg(cx.color("text"));
        for (row, week) in self.weeks().iter().enumerate() {
            let y = area.y + 3 + i32::try_from(row).unwrap_or(0);
            if y >= area.y + i32::from(area.height) {
                break;
            }
            for (column, day) in week.iter().enumerate() {
                let Some(day) = day else { continue };
                let x = area.x + i32::from(u16::try_from(column).unwrap_or(0) * DAY_WIDTH);
                let number = format!("{day:>2}");
                if *day == self.today.day() {
                    // Today is the accent on a raised cell, and bold: the day reads as today by
                    // its shape as well as by its colour (VISION 4.5).
                    let raised = surface.mix(cx.color("accent"), TODAY_MIX);
                    cx.clear(Rect::new(x, y, 2, 1), raised);
                    let style = CellStyle::fg(cx.color("accent")).on(raised).with_bold(true);
                    cx.text(x, y, &number, style, 2);
                } else {
                    cx.text(x, y, &number, plain, 2);
                }
            }
        }
    }
}

/// A reading written as its name and its value, the name faint on the left and the value on the
/// right: the network rate, which has no range to fill a meter with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    label: String,
    label_width: u16,
    value: String,
}

impl Line {
    /// `label`, padded to `label_width` cells so it lines up with the meters above it, and
    /// `value`.
    #[must_use]
    pub fn new(label: impl Into<String>, label_width: u16, value: impl Into<String>) -> Self {
        Self { label: label.into(), label_width, value: value.into() }
    }
}

impl<Msg: 'static> Widget<Msg> for Line {
    fn measure(&self, _cx: &mut MeasureCx<'_>, available: Size) -> Size {
        Size::new(available.width, 1.min(available.height))
    }

    fn paint(&self, cx: &mut PaintCx<'_>, area: Rect) {
        if area.is_empty() {
            return;
        }
        let label = text::truncate(&self.label, self.label_width.min(area.width)).into_owned();
        cx.text(area.x, area.y, &label, CellStyle::fg(cx.color("dim")), area.width);
        let width = text::width(&self.value);
        let x = area.x + i32::from(area.width.saturating_sub(width));
        cx.text(x, area.y, &self.value, CellStyle::fg(cx.color("text")).with_bold(true), area.width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(year: i32, month: u8, day: u8) -> Date {
        Date::new(year, month, day).expect("a real day")
    }

    #[test]
    fn a_month_starts_its_first_week_on_the_weekday_of_its_first_day() {
        // September 2026 starts on a Tuesday.
        let monday = Month::new(day(2026, 9, 19), Weekday::Monday).weeks();
        assert_eq!(monday[0], [None, Some(1), Some(2), Some(3), Some(4), Some(5), Some(6)]);
        assert_eq!(monday.len(), 5);
        assert_eq!(monday[4], [Some(28), Some(29), Some(30), None, None, None, None]);
        let sunday = Month::new(day(2026, 9, 19), Weekday::Sunday).weeks();
        assert_eq!(sunday[0], [None, None, Some(1), Some(2), Some(3), Some(4), Some(5)]);
    }

    #[test]
    fn a_month_never_takes_more_than_six_weeks() {
        // August 2026 starts on a Saturday and has 31 days: the longest a month can spread.
        let weeks = Month::new(day(2026, 8, 1), Weekday::Monday).weeks();
        assert_eq!(weeks.len(), 6);
        assert!(usize::from(MONTH_ROWS) >= 3 + weeks.len());
    }

    #[test]
    fn a_surface_leaves_its_last_column_and_row_to_the_floor() {
        assert_eq!(surface_rect(20, 3, (3, 2)), Rect::new(20, 3, 29, 5));
    }
}
