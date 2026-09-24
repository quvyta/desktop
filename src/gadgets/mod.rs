//! The small things that stand on the floor beside the icons: a clock, a calendar, the machine's
//! readings and a note (design 3.12). The code calls them gadgets, because the framework's own
//! `widgets` and `Widget` are everywhere here already; the person reads "widget".
//!
//! This module is pure: what a gadget is, which of its options differ from the defaults, how
//! many cells of the icon grid it takes, where it stands on a floor of a given size, and how it
//! is written in and read back from the desktop file. [`face`] draws them.
//!
//! A gadget takes a rectangle of the icon grid's cells. Gadgets never overlap: the first in the
//! order keeps its place, one whose place is taken or off the screen stands in the nearest spot
//! that is free, and one for which no spot is left is not drawn. Icons flow around the cells the
//! gadgets take.

pub mod face;

use std::path::Path;

use qframe::date::Weekday;
use qframe::prelude::*;
use toml::de::{DeTable, DeValue};

use crate::apps::{Diagnostic, DiagnosticKind, Expected, Position};
use crate::desktop::grid::Cell;

/// What a gadget shows, without its options: what the floor's menu offers to add.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// The time, large, and the date under it.
    Clock,
    /// This month, with today in the accent.
    Calendar,
    /// The processor, the memory, the battery and the network.
    System,
    /// A plain text file, written in place.
    Note,
}

impl Kind {
    /// Every kind, in the order the menu offers them.
    pub const ALL: [Self; 4] = [Self::Clock, Self::Calendar, Self::System, Self::Note];

    /// The name written in the desktop file.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Clock => "clock",
            Self::Calendar => "calendar",
            Self::System => "system",
            Self::Note => "note",
        }
    }

    /// The kind a name in the file means.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// Its name on screen.
    #[must_use]
    pub fn label(self) -> String {
        t!(&format!("widget.{}", self.name()))
    }
}

/// The colour of the clock's digits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tint {
    /// The theme's accent.
    #[default]
    Accent,
    /// The theme's text colour: a quiet clock.
    Text,
    /// The accent blending down into the theme's second accent.
    Gradient,
}

impl Tint {
    /// Every colour, in the order the menu lists them.
    pub const ALL: [Self; 3] = [Self::Accent, Self::Text, Self::Gradient];

    /// The name written in the desktop file, also the key of its label.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Accent => "accent",
            Self::Text => "text",
            Self::Gradient => "gradient",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tint| tint.name() == name)
    }
}

/// The clock's options.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Clock {
    /// Whether the hours run to twelve, with a morning or afternoon mark.
    pub twelve: bool,
    /// Whether the seconds are shown, which draws the clock every second instead of every minute.
    pub seconds: bool,
    /// The colour of the digits.
    pub tint: Tint,
}

/// The day a calendar's weeks start on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum WeekStart {
    /// The language's (or the region's) own first day.
    #[default]
    Language,
    /// Monday.
    Monday,
    /// Sunday.
    Sunday,
}

impl WeekStart {
    /// Every choice, in the order the menu lists them.
    pub const ALL: [Self; 3] = [Self::Language, Self::Monday, Self::Sunday];

    /// The name written in the desktop file, also the key of its label.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Language => "language",
            Self::Monday => "monday",
            Self::Sunday => "sunday",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|start| start.name() == name)
    }

    /// The weekday the weeks start on, `language` being the language's own.
    #[must_use]
    pub fn day(self, language: Weekday) -> Weekday {
        match self {
            Self::Language => language,
            Self::Monday => Weekday::Monday,
            Self::Sunday => Weekday::Sunday,
        }
    }
}

/// One reading of the system gadget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reading {
    /// How busy the processor is.
    Cpu,
    /// How full the memory is.
    Memory,
    /// How full the battery is, on a machine that has one.
    Battery,
    /// The network rate.
    Network,
}

impl Reading {
    /// Every reading, top to bottom.
    pub const ALL: [Self; 4] = [Self::Cpu, Self::Memory, Self::Battery, Self::Network];

    /// The name written in the desktop file.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Battery => "battery",
            Self::Network => "network",
        }
    }

    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|reading| reading.name() == name)
    }

    /// Its name on screen: the status strip's own.
    #[must_use]
    pub fn label(self) -> String {
        t!(&format!("status.{}", self.name()))
    }
}

/// Which readings the system gadget shows, top to bottom. Never empty: taking the last one away
/// is refused, since an empty gadget would be a blank surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readings(Vec<Reading>);

impl Default for Readings {
    fn default() -> Self {
        Self(Reading::ALL.to_vec())
    }
}

impl Readings {
    /// Whether `reading` is shown.
    #[must_use]
    pub fn shows(&self, reading: Reading) -> bool {
        self.0.contains(&reading)
    }

    /// The readings shown, in their order.
    #[must_use]
    pub fn shown(&self) -> &[Reading] {
        &self.0
    }

    /// Shows or hides `reading`, keeping the order of [`Reading::ALL`]. Hiding the last one is
    /// refused. Returns whether anything changed.
    pub fn set(&mut self, reading: Reading, on: bool) -> bool {
        if on == self.shows(reading) || (!on && self.0.len() == 1) {
            return false;
        }
        if on {
            self.0.push(reading);
        } else {
            self.0.retain(|kept| *kept != reading);
        }
        self.0.sort_by_key(|kept| Reading::ALL.iter().position(|each| each == kept));
        true
    }
}

/// What a gadget shows, with its options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shows {
    /// The clock.
    Clock(Clock),
    /// The calendar, starting its weeks on this day.
    Calendar(WeekStart),
    /// The system readings.
    System(Readings),
    /// The note kept in this file of the notes folder.
    Note(String),
}

impl Shows {
    /// Its kind.
    #[must_use]
    pub fn kind(&self) -> Kind {
        match self {
            Self::Clock(_) => Kind::Clock,
            Self::Calendar(_) => Kind::Calendar,
            Self::System(_) => Kind::System,
            Self::Note(_) => Kind::Note,
        }
    }
}

/// The first note's file; the next ones are `notes-2.txt` and on.
pub const FIRST_NOTE: &str = "notes.txt";

/// One gadget on the floor: what it shows and the cell of its top left corner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gadget {
    /// What it shows.
    pub shows: Shows,
    /// Where its top left corner wants to be, in the icon grid's cells.
    pub place: Cell,
}

impl Gadget {
    /// A new gadget of `kind` at `place`, with the default options. A note takes `file`.
    #[must_use]
    pub fn new(kind: Kind, place: Cell, file: &str) -> Self {
        let shows = match kind {
            Kind::Clock => Shows::Clock(Clock::default()),
            Kind::Calendar => Shows::Calendar(WeekStart::default()),
            Kind::System => Shows::System(Readings::default()),
            Kind::Note => Shows::Note(file.to_owned()),
        };
        Self { shows, place }
    }

    /// The columns and rows of icon cells it takes.
    ///
    /// Each is the smallest that holds its content in every glyph mode with a free column and row
    /// on its far sides, as an icon's cell keeps a free row under its name: the clock's digits are
    /// three rows with Unicode glyphs and five in ASCII, a month is nine rows, and the system
    /// gadget takes a row for each reading shown.
    #[must_use]
    pub fn size(&self) -> (u16, u16) {
        match &self.shows {
            Shows::Clock(clock) => (if clock.seconds { 4 } else { 3 }, 3),
            Shows::Calendar(_) => (3, 4),
            Shows::System(readings) => {
                // A row for each reading, a row of room above and below them, and the free row.
                let rows = u16::try_from(readings.shown().len()).unwrap_or(u16::MAX).saturating_add(3);
                (3, rows.div_ceil(crate::desktop::grid::CELL_HEIGHT))
            }
            Shows::Note(_) => (3, 3),
        }
    }

    /// Its spot at its own place.
    #[must_use]
    pub fn spot(&self) -> Spot {
        Spot { at: self.place, size: self.size() }
    }
}

/// A change to one option of a gadget, chosen from its menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Change {
    /// The clock's hours run to twelve (`true`) or to twenty-four.
    Twelve(bool),
    /// The clock shows its seconds.
    Seconds(bool),
    /// The colour of the clock's digits.
    Tint(Tint),
    /// The day the calendar's weeks start on.
    WeekStart(WeekStart),
    /// A reading of the system gadget is shown or hidden.
    Reading(Reading, bool),
}

impl Gadget {
    /// Applies `change`, when it is one of this gadget's options. Returns whether anything
    /// changed.
    pub fn apply(&mut self, change: Change) -> bool {
        match (&mut self.shows, change) {
            (Shows::Clock(clock), Change::Twelve(on)) => std::mem::replace(&mut clock.twelve, on) != on,
            (Shows::Clock(clock), Change::Seconds(on)) => std::mem::replace(&mut clock.seconds, on) != on,
            (Shows::Clock(clock), Change::Tint(tint)) => std::mem::replace(&mut clock.tint, tint) != tint,
            (Shows::Calendar(start), Change::WeekStart(day)) => std::mem::replace(start, day) != day,
            (Shows::System(readings), Change::Reading(reading, on)) => readings.set(reading, on),
            _ => false,
        }
    }
}

/// A rectangle of the icon grid's cells: a corner and a size in cells.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spot {
    /// The top left cell.
    pub at: Cell,
    /// Columns and rows of cells.
    pub size: (u16, u16),
}

impl Spot {
    /// The same size with its corner at `at`.
    #[must_use]
    pub fn moved(self, at: Cell) -> Self {
        Self { at, ..self }
    }

    /// Whether it lies wholly inside a floor of `columns` and `rows` cells.
    #[must_use]
    pub fn inside(self, columns: u16, rows: u16) -> bool {
        u32::from(self.at.0) + u32::from(self.size.0) <= u32::from(columns)
            && u32::from(self.at.1) + u32::from(self.size.1) <= u32::from(rows)
    }

    /// Whether it holds `cell`.
    #[must_use]
    pub fn holds(self, (column, row): Cell) -> bool {
        let (left, top) = (u32::from(self.at.0), u32::from(self.at.1));
        let (column, row) = (u32::from(column), u32::from(row));
        column >= left && column < left + u32::from(self.size.0) && row >= top && row < top + u32::from(self.size.1)
    }

    /// Whether it shares a cell with `other`.
    #[must_use]
    pub fn overlaps(self, other: Self) -> bool {
        let span = |at: u16, size: u16| (u32::from(at), u32::from(at) + u32::from(size));
        let (a_left, a_right) = span(self.at.0, self.size.0);
        let (b_left, b_right) = span(other.at.0, other.size.0);
        let (a_top, a_bottom) = span(self.at.1, self.size.1);
        let (b_top, b_bottom) = span(other.at.1, other.size.1);
        a_left < b_right && b_left < a_right && a_top < b_bottom && b_top < a_bottom
    }

    /// Every cell it holds.
    #[must_use]
    pub fn cells(self) -> Vec<Cell> {
        let (left, top) = self.at;
        (0..self.size.0).flat_map(|column| (0..self.size.1).map(move |row| (left + column, top + row))).collect()
    }
}

/// Where every gadget of `wanted` stands on a floor of `columns` and `rows` cells, in their order;
/// `None` for one that has no room.
///
/// A gadget whose own spot is inside the floor and free of the ones before it stands there. One
/// whose spot is off the floor — a terminal made smaller — or taken stands in the free spot
/// nearest to it, counted in cells, and keeps its own place in the file, so it comes back when
/// the room does.
#[must_use]
pub fn arrange(columns: u16, rows: u16, wanted: &[Spot]) -> Vec<Option<Spot>> {
    let mut placed: Vec<Spot> = Vec::new();
    let mut drawn = Vec::with_capacity(wanted.len());
    for spot in wanted {
        let free = |candidate: Spot| candidate.inside(columns, rows) && !placed.iter().any(|at| at.overlaps(candidate));
        let chosen = if free(*spot) {
            Some(*spot)
        } else {
            corners(columns, rows, spot.size).map(|at| spot.moved(at)).filter(|candidate| free(*candidate)).min_by_key(
                |candidate| {
                    let (column, row) = candidate.at;
                    (column.abs_diff(spot.at.0) + row.abs_diff(spot.at.1), column.abs_diff(spot.at.0), column, row)
                },
            )
        };
        if let Some(chosen) = chosen {
            placed.push(chosen);
        }
        drawn.push(chosen);
    }
    drawn
}

/// Every corner a spot of `size` can have on a floor of `columns` and `rows` cells.
fn corners(columns: u16, rows: u16, size: (u16, u16)) -> impl Iterator<Item = Cell> {
    let last_column = columns.checked_sub(size.0).map(|last| last + 1).unwrap_or(0);
    let last_row = rows.checked_sub(size.1).map(|last| last + 1).unwrap_or(0);
    (0..last_column).flat_map(move |column| (0..last_row).map(move |row| (column, row)))
}

/// Where a new gadget of `size` goes on a floor of `columns` and `rows` cells, among the gadgets
/// drawn at `gadgets` and the icons drawn in `icons`.
///
/// Gadgets gather on the right and icons flow down the left (design 3.12), so the floor is looked
/// at from its top right corner: columns right to left, each from the top down. The first spot
/// that touches neither an icon nor a gadget is taken; failing that the first that touches no
/// gadget, and the icons make way. `None` when even that is not there.
#[must_use]
pub fn spot_for(columns: u16, rows: u16, size: (u16, u16), gadgets: &[Spot], icons: &[Cell]) -> Option<Cell> {
    let mut order: Vec<Cell> = corners(columns, rows, size).collect();
    order.sort_by_key(|(column, row)| (std::cmp::Reverse(*column), *row));
    let spot = |at: Cell| Spot { at, size };
    let clear_of_gadgets = |at: &Cell| !gadgets.iter().any(|gadget| gadget.overlaps(spot(*at)));
    order
        .iter()
        .copied()
        .filter(clear_of_gadgets)
        .find(|at| !icons.iter().any(|icon| spot(*at).holds(*icon)))
        .or_else(|| order.iter().copied().find(clear_of_gadgets))
}

/// The file the next note takes: the first of `notes.txt`, `notes-2.txt` and on that no gadget
/// of `gadgets` already shows.
#[must_use]
pub fn next_note(gadgets: &[Gadget]) -> String {
    let taken = |file: &str| gadgets.iter().any(|gadget| gadget.shows == Shows::Note(file.to_owned()));
    if !taken(FIRST_NOTE) {
        return FIRST_NOTE.to_owned();
    }
    (2..).map(|number| format!("notes-{number}.txt")).find(|file| !taken(file)).unwrap_or_default()
}

/// Whether `file` is a name a note may have: a plain name inside the notes folder, never a path
/// that leads out of it.
#[must_use]
pub fn note_name_is_plain(file: &str) -> bool {
    !file.is_empty()
        && file != "."
        && file != ".."
        && !file.contains(['/', '\\', '\0'])
        && Path::new(file).file_name().is_some_and(|name| name == file)
}

/// The gadgets as the desktop file writes them: one `[[widgets]]` table each, with only the
/// options that differ from the defaults. Empty when there are none.
#[must_use]
pub fn to_toml(gadgets: &[Gadget]) -> String {
    let mut text = String::new();
    if gadgets.is_empty() {
        return text;
    }
    text.push_str("\n# The widgets on the floor, each with the cell of its top left corner as [column, row]\n");
    text.push_str("# and the options that differ from its defaults.\n");
    for gadget in gadgets {
        let (column, row) = gadget.place;
        text.push_str(&format!("[[widgets]]\nkind = \"{}\"\nplace = [{column}, {row}]\n", gadget.shows.kind().name()));
        match &gadget.shows {
            Shows::Clock(clock) => {
                if clock.twelve {
                    text.push_str("hours = 12\n");
                }
                if clock.seconds {
                    text.push_str("seconds = true\n");
                }
                if clock.tint != Tint::default() {
                    text.push_str(&format!("color = \"{}\"\n", clock.tint.name()));
                }
            }
            Shows::Calendar(start) => {
                if *start != WeekStart::default() {
                    text.push_str(&format!("week-start = \"{}\"\n", start.name()));
                }
            }
            Shows::System(readings) => {
                if *readings != Readings::default() {
                    let names: Vec<String> =
                        readings.shown().iter().map(|reading| format!("\"{}\"", reading.name())).collect();
                    text.push_str(&format!("readings = [{}]\n", names.join(", ")));
                }
            }
            Shows::Note(file) => {
                if file != FIRST_NOTE {
                    text.push_str(&format!("file = \"{}\"\n", file.replace('\\', "\\\\").replace('"', "\\\"")));
                }
            }
        }
    }
    text
}

/// Reads the `widgets` array of the desktop file `file`, whose text is `text`, into gadgets.
///
/// A table that says nothing usable — no kind, a kind this qdesk does not know, no place — is
/// left out with a diagnostic at its line; a bad option is a diagnostic and keeps its default; an
/// unknown field is a warning, since a newer qdesk may know it.
#[must_use]
pub fn parse(file: &Path, text: &str, value: &toml::Spanned<DeValue<'_>>) -> (Vec<Gadget>, Vec<Diagnostic>) {
    let mut gadgets = Vec::new();
    let mut diagnostics = Vec::new();
    let at = |offset: usize| Some(Position::of_offset(text, offset));
    let DeValue::Array(tables) = value.get_ref() else {
        diagnostics.push(Diagnostic::at(
            file,
            at(value.span().start),
            DiagnosticKind::WrongType { key: "widgets".to_owned(), expected: Expected::Table },
        ));
        return (gadgets, diagnostics);
    };
    for table in tables {
        let DeValue::Table(fields) = table.get_ref() else {
            diagnostics.push(Diagnostic::at(
                file,
                at(table.span().start),
                DiagnosticKind::WrongType { key: "widgets".to_owned(), expected: Expected::Table },
            ));
            continue;
        };
        if let Some(gadget) = one(file, text, table.span().start, fields, &mut diagnostics) {
            gadgets.push(gadget);
        }
    }
    (gadgets, diagnostics)
}

/// One `[[widgets]]` table, starting at `start` of `text`.
fn one(
    file: &Path,
    text: &str,
    start: usize,
    fields: &DeTable<'_>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Option<Gadget> {
    let at = |offset: usize| Some(Position::of_offset(text, offset));
    let invalid = |key: &str, offset: usize| {
        Diagnostic::at(file, at(offset), DiagnosticKind::InvalidValue { key: format!("widgets.{key}") })
    };
    let wrong = |key: &str, expected: Expected, offset: usize| {
        Diagnostic::at(file, at(offset), DiagnosticKind::WrongType { key: format!("widgets.{key}"), expected })
    };
    let field = |name: &str| fields.iter().find(|(key, _)| key.get_ref().as_ref() == name).map(|(_, value)| value);
    let kind = match field("kind").map(|value| (value.get_ref(), value.span().start)) {
        Some((DeValue::String(name), offset)) => match Kind::from_name(name) {
            Some(kind) => kind,
            None => {
                diagnostics.push(invalid("kind", offset));
                return None;
            }
        },
        Some((_, offset)) => {
            diagnostics.push(wrong("kind", Expected::String, offset));
            return None;
        }
        None => {
            diagnostics.push(invalid("kind", start));
            return None;
        }
    };
    let place = match field("place") {
        Some(value) => match crate::desktop::order::place_of(value.get_ref()) {
            Some(cell) => cell,
            None => {
                diagnostics.push(invalid("place", value.span().start));
                return None;
            }
        },
        None => {
            diagnostics.push(invalid("place", start));
            return None;
        }
    };
    let mut gadget = Gadget::new(kind, place, FIRST_NOTE);
    for (key, value) in fields {
        let name = key.get_ref().as_ref();
        let offset = value.span().start;
        match (&mut gadget.shows, name, value.get_ref()) {
            (_, "kind" | "place", _) => {}
            (Shows::Clock(clock), "hours", DeValue::Integer(number)) => match number.as_str() {
                "12" => clock.twelve = true,
                "24" => clock.twelve = false,
                _ => diagnostics.push(invalid(name, offset)),
            },
            (Shows::Clock(clock), "seconds", DeValue::Boolean(on)) => clock.seconds = *on,
            (Shows::Clock(clock), "color", DeValue::String(tint)) => match Tint::from_name(tint) {
                Some(tint) => clock.tint = tint,
                None => diagnostics.push(invalid(name, offset)),
            },
            (Shows::Calendar(start), "week-start", DeValue::String(day)) => match WeekStart::from_name(day) {
                Some(day) => *start = day,
                None => diagnostics.push(invalid(name, offset)),
            },
            (Shows::System(readings), "readings", DeValue::Array(items)) => {
                let named: Option<Vec<Reading>> = items
                    .iter()
                    .map(|item| match item.get_ref() {
                        DeValue::String(reading) => Reading::from_name(reading),
                        _ => None,
                    })
                    .collect();
                match named {
                    Some(mut named) if !named.is_empty() => {
                        named.sort_by_key(|kept| Reading::ALL.iter().position(|each| each == kept));
                        named.dedup();
                        *readings = Readings(named);
                    }
                    _ => diagnostics.push(invalid(name, offset)),
                }
            }
            (Shows::Note(note), "file", DeValue::String(file)) if note_name_is_plain(file) => {
                *note = file.to_string();
            }
            (Shows::Note(_), "file", DeValue::String(_)) => diagnostics.push(invalid(name, offset)),
            (Shows::Clock(_), "hours", _) => diagnostics.push(invalid(name, offset)),
            (Shows::Clock(_), "seconds", _) => diagnostics.push(wrong(name, Expected::Boolean, offset)),
            (Shows::Clock(_), "color", _) | (Shows::Calendar(_), "week-start", _) | (Shows::Note(_), "file", _) => {
                diagnostics.push(wrong(name, Expected::String, offset));
            }
            (Shows::System(_), "readings", _) => diagnostics.push(wrong(name, Expected::StringList, offset)),
            _ => diagnostics.push(Diagnostic::at(
                file,
                at(key.span().start),
                DiagnosticKind::UnknownField { key: format!("widgets.{name}") },
            )),
        }
    }
    Some(gadget)
}

#[cfg(test)]
mod tests;
