//! The floor of the desktop and the icons standing on it.
//!
//! [`grid`] says where an icon goes and [`order`] what the person's own order and places are; both
//! are pure.
//! [`Floor`] draws the cells and reads the mouse and the keys, and [`IconCell`] draws one icon.
//! The floor keeps nothing of its own: the application owns the selection, the cursor and the
//! order, and hears about every change as an [`Action`].
//!
//! The floor is bare (VISION §4): no wallpaper, no frame, no line anywhere. A selected icon is
//! the raised surface tone with the accent pillar down its left edge, and the band drawn from
//! empty floor is the accent mixed into the floor, never an outline. So is the cell a dragged icon
//! would land in, as the snap preview of a window is.

pub mod grid;
pub mod order;

use std::time::Duration;

use qframe::event::{Event, KeyEvent, KeyKind, MouseButton, MouseEvent, MouseKind};
use qframe::geometry::{Rect, Size};
use qframe::icons::Icons;
use qframe::keymap::{Key, Modifiers};
use qframe::style::CellStyle;
use qframe::text;
use qframe::widget::{Container, EventCx, MeasureCx, Node, PaintCx, Widget};

use crate::apps::{Category, Entry};
use crate::gadgets::{self, Spot, face};

pub use grid::{CELL_HEIGHT, CELL_WIDTH, Cell, Grid, Step};
pub use order::Desktop;

/// The Quvyta icon, on the launcher button. The framework's own icon set holds it; the
/// name is looked up every frame rather than resolved once, so a set that does not (an older
/// framework, or a set of the person's own) shows [`QUVYTA_FALLBACK`] instead of an empty cell.
/// The framework still names the key after the ecosystem's old name.
pub const QUVYTA_ICON: &str = "family";

/// What stands in for [`QUVYTA_ICON`] in an icon set without that key.
pub const QUVYTA_FALLBACK: &str = "project";

/// Two clicks on the same icon closer together than this open it.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// How much of the accent the band mixes into the floor.
const BAND_MIX: f32 = 0.20;

/// How much of the accent the cell a dragged icon would land in mixes into the floor: the band's
/// and the window snap preview's share, so every "this is where it goes" on the desktop is one tone.
const DROP_MIX: f32 = 0.20;

/// How much of the text colour a hovered icon lifts its cell by, as a pressable panel does.
const HOVER_MIX: f32 = 0.08;

/// What the person did on the floor. The application decides what each one means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// An icon was clicked: it alone is selected, or with ctrl held it joins the selection.
    Select {
        /// The icon.
        index: usize,
        /// Whether it joins the selection instead of replacing it.
        add: bool,
    },
    /// An icon was clicked with shift held: every icon from the cursor to it, in the floor's
    /// order, is selected.
    Extend(usize),
    /// A band was drawn from empty floor over these icons.
    Band(Vec<usize>),
    /// Empty floor was clicked: nothing is selected any more.
    Clear,
    /// The keyboard cursor moved to this icon.
    Cursor(usize),
    /// An icon was opened: a double click, or Enter on the cursor.
    Open(usize),
    /// Icons were put into other cells: one dropped there with the mouse or moved with shift and
    /// an arrow key, or a selection dragged together by one of its icons.
    Place {
        /// The icon that was held: the one dragged, or the one under the cursor.
        index: usize,
        /// Every icon that moves, with the cell it is put into, which may hold another icon. The
        /// held one is among them.
        moves: Vec<(usize, Cell)>,
        /// The cell every icon was drawn in when it happened, in their order: where the others
        /// stand, which is what the move is made against.
        layout: Vec<Option<Cell>>,
    },
    /// A gadget was dragged to another spot of the floor: `at` is its new top left cell. A drop
    /// that would take it off the floor or over another gadget is never reported.
    PlaceGadget {
        /// The gadget, in the order given to [`Floor::gadgets`].
        index: usize,
        /// The cell its top left corner goes to.
        at: Cell,
    },
}

/// The key of the icon a button shows for Quvyta: the one the icon set holds.
#[must_use]
pub fn quvyta_icon(icons: &Icons) -> &'static str {
    if icons.contains(QUVYTA_ICON) { QUVYTA_ICON } else { QUVYTA_FALLBACK }
}

/// The glyph an entry is drawn with: the framework icon it names, the single character it gives,
/// or the icon of its category.
///
/// An entry may name an icon a newer framework has and this one does not; the category's icon
/// stands in for it rather than an empty cell.
#[must_use]
pub fn glyph_of(entry: &Entry, icons: &Icons) -> String {
    if let Some(named) = &entry.icon {
        if icons.contains(named) {
            return icons.glyph(named).into_owned();
        }
        // A single cell of a character is an icon of its own, as the entry format allows.
        if named.chars().count() == 1 && text::width(named) == 1 {
            return named.clone();
        }
    }
    icons.glyph(category_icon(entry.category, icons)).into_owned()
}

/// The name of the icon set's icon an entry is drawn with, for a place that takes a name rather
/// than a glyph: the icon it names when the set has it, else its category's. An entry drawn with a
/// character of its own has no name, and its category's stands in.
#[must_use]
pub fn icon_name<'a>(entry: &'a Entry, icons: &Icons) -> &'a str {
    match &entry.icon {
        Some(named) if icons.contains(named) => named,
        _ => category_icon(entry.category, icons),
    }
}

/// The name of the icon an entry of a folder is drawn with, by its kind: the Rust logo on a Rust
/// file, a zipper on an archive, a program's icon on a file that may be run whose name says
/// nothing else. The kind is told from the name alone by the framework, as its file manager tells
/// it, so the floor and a Files window draw a file alike.
#[must_use]
pub fn kind_icon(name: &str, folder: bool, executable: bool) -> &'static str {
    qframe::icons::file_kind(name, folder, executable).icon()
}

/// The icon of a category, by name.
fn category_icon(category: Category, icons: &Icons) -> &'static str {
    match category {
        Category::System => "settings",
        Category::Development => "project",
        Category::Files => "folder",
        Category::Network => "dot-outline",
        Category::Office => "file",
        Category::Media => "dot",
        Category::Quvyta => quvyta_icon(icons),
        Category::Other => "bullet",
    }
}

/// One icon: the glyph, the name under it, and how it stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IconCell {
    glyph: String,
    name: String,
    selected: bool,
    cursor: bool,
}

impl IconCell {
    /// An icon drawn with `glyph` and called `name`.
    #[must_use]
    pub fn new(glyph: impl Into<String>, name: impl Into<String>) -> Self {
        Self { glyph: glyph.into(), name: name.into(), selected: false, cursor: false }
    }

    /// Whether the icon is one of the selected ones.
    #[must_use]
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Whether the keyboard cursor is on it.
    #[must_use]
    pub fn cursor(mut self, cursor: bool) -> Self {
        self.cursor = cursor;
        self
    }
}

impl<Msg: 'static> Widget<Msg> for IconCell {
    fn measure(&self, _cx: &mut MeasureCx<'_>, available: Size) -> Size {
        Size::new(CELL_WIDTH.min(available.width), CELL_HEIGHT.min(available.height))
    }

    fn paint(&self, cx: &mut PaintCx<'_>, area: Rect) {
        if area.is_empty() {
            return;
        }
        cx.register_hit(area);
        let selected = self.selected;
        if selected {
            cx.clear(area, cx.color("active"));
        }
        if cx.is_hovered() {
            cx.tint(area, cx.color("text"), HOVER_MIX);
        }
        if selected || self.cursor {
            let accent = cx.color("accent");
            for row in 0..area.height {
                cx.pillar(area.x, area.y + i32::from(row), accent);
            }
        }
        let inner = Rect::new(
            area.x + i32::from(grid::PILLAR_WIDTH),
            area.y,
            area.width.saturating_sub(grid::PILLAR_WIDTH),
            area.height,
        );
        let tone = |token: &str, cx: &PaintCx<'_>| {
            let mut style = CellStyle::fg(cx.color(token));
            if selected {
                style = style.on(cx.color("active"));
            }
            style
        };
        let centred = |text: &str| {
            let width = text::width(text);
            inner.x + i32::from(inner.width.saturating_sub(width) / 2)
        };
        let glyph = self.glyph.clone();
        let style = tone("text", cx);
        cx.text(centred(&glyph), inner.y, &glyph, style, inner.width);
        if inner.height > 1 {
            let name = grid::shown_name(&self.name);
            let role = if selected { "text" } else { "dim" };
            let style = tone(role, cx);
            cx.text(centred(&name), inner.y + 1, &name, style, inner.width);
        }
    }
}

/// What the floor remembers between frames: where its cells were, and what the mouse is doing.
#[derive(Debug, Default)]
struct FloorMemory {
    /// The grid and area painted last, which the mouse is read against.
    grid: Option<Grid>,
    area: Rect,
    /// The icon the left button went down on, and the cell of the floor the mouse is over while
    /// it is held: where the icon would land.
    pressed: Option<usize>,
    target: Option<Cell>,
    /// Where a band started, and the band it has grown to.
    band_from: Option<(i32, i32)>,
    band: Option<Rect>,
    /// The last click, for telling a double click from two single ones.
    clicked: Option<(usize, Duration)>,
    /// The icon the last typed letter found, so the same letter walks on.
    typed: Option<usize>,
    /// Where every gadget was drawn last, in their order.
    spots: Vec<Option<Spot>>,
    /// The gadget the left button went down on, with the cell of it that was held, counted from
    /// its top left cell, and the top left cell it would land in while the button is held.
    gadget: Option<(usize, (u16, u16))>,
    gadget_target: Option<Cell>,
}

/// The floor: the icon cells laid out on the grid, the gadgets standing among them, the band, and
/// the keys of the desktop.
///
/// Add the cells with [`View::add_with`](qframe::widget::View::add_with), one per icon in the
/// order they stand in; each may be wrapped in its own [`ContextMenu`](qframe::widgets::ContextMenu)
/// and [`Tooltip`](qframe::widgets::Tooltip). After them come the gadgets, one node each in the
/// order of [`Floor::gadgets`], each drawn over the cells it takes.
pub struct Floor<Msg> {
    names: Vec<String>,
    places: Vec<Option<Cell>>,
    gadgets: Vec<Spot>,
    selected: Vec<usize>,
    cursor: Option<usize>,
    keys: bool,
    on: Box<dyn Fn(Action) -> Msg>,
    cells: Vec<Node<Msg>>,
}

impl<Msg: 'static> Floor<Msg> {
    /// A floor for icons called `names`, in the order they stand in, reporting what happens on it
    /// through `on`.
    #[must_use]
    pub fn new(names: Vec<String>, on: impl Fn(Action) -> Msg + 'static) -> Self {
        Self {
            names,
            places: Vec::new(),
            gadgets: Vec::new(),
            selected: Vec::new(),
            cursor: None,
            keys: true,
            on: Box::new(on),
            cells: Vec::new(),
        }
    }

    /// The icons that are selected, by their index: dragging any of them carries them all.
    #[must_use]
    pub fn selected(mut self, selected: Vec<usize>) -> Self {
        self.selected = selected;
        self
    }

    /// Where the icons land when the one at `index` is dragged into `cell`: the whole selection
    /// carried as far as that icon went, when it is one of several selected, else that icon alone.
    fn landing(&self, grid: &Grid, index: usize, cell: Cell) -> Vec<(usize, Cell)> {
        let Some(from) = grid.cell(index) else { return Vec::new() };
        let carried = self.selected.iter().filter(|selected| grid.cell(**selected).is_some()).count();
        if carried < 2 || !self.selected.contains(&index) {
            return vec![(index, cell)];
        }
        let by = (i32::from(cell.0) - i32::from(from.0), i32::from(cell.1) - i32::from(from.1));
        grid.shifted(&self.selected, by)
    }

    /// The cells the icons want, in their order; an icon with no place of its own, or past the
    /// end of `places`, flows into the first free cell.
    #[must_use]
    pub fn places(mut self, places: Vec<Option<Cell>>) -> Self {
        self.places = places;
        self
    }

    /// The spots the gadgets want, in the order their nodes come after the icons' cells.
    #[must_use]
    pub fn gadgets(mut self, gadgets: Vec<Spot>) -> Self {
        self.gadgets = gadgets;
        self
    }

    /// How many of the children are icon cells: the rest are gadgets.
    fn icon_count(&self) -> usize {
        self.cells.len().saturating_sub(self.gadgets.len())
    }

    /// The grid the icons make in `area`, around the gadgets, and where each gadget stands.
    fn layout(&self, area: Size) -> (Grid, Vec<Option<Spot>>) {
        let spots = gadgets::arrange(area.width / CELL_WIDTH, area.height / CELL_HEIGHT, &self.gadgets);
        let blocked: Vec<Cell> = spots.iter().flatten().flat_map(|spot| spot.cells()).collect();
        let mut wanted = self.places.clone();
        wanted.resize(self.icon_count(), None);
        (Grid::placed_around(area, &wanted, &blocked), spots)
    }

    /// Where the icons land when the one at `index` is dropped into `cell`, or `None` when any of
    /// them would land on a gadget: a group lands whole or not at all.
    fn lands(&self, grid: &Grid, index: usize, cell: Cell) -> Option<Vec<(usize, Cell)>> {
        let moves = self.landing(grid, index, cell);
        (!moves.iter().any(|(_, to)| grid.is_blocked(*to))).then_some(moves)
    }

    /// The screen rectangle of the surface of a gadget standing in `spot`.
    fn surface(spot: Spot, area: Rect) -> Rect {
        let x = area.x + i32::from(spot.at.0) * i32::from(CELL_WIDTH);
        let y = area.y + i32::from(spot.at.1) * i32::from(CELL_HEIGHT);
        face::surface_rect(x, y, spot.size)
    }

    /// The gadget whose surface holds the screen cell `x`, `y`.
    fn gadget_at(spots: &[Option<Spot>], area: Rect, x: i32, y: i32) -> Option<usize> {
        spots.iter().position(|spot| {
            spot.is_some_and(|spot| {
                let rect = Self::surface(spot, area);
                x >= rect.x && x < rect.x + i32::from(rect.width) && y >= rect.y && y < rect.y + i32::from(rect.height)
            })
        })
    }

    /// Where the gadget at `index` would land with the pointer at `(x, y)`, holding it by `grab`:
    /// its top left cell, kept on the floor the way a window stops at the screen's edge. `None`
    /// when that spot is its own or overlaps another gadget.
    fn gadget_landing(
        &self,
        grid: &Grid,
        spots: &[Option<Spot>],
        area: Rect,
        index: usize,
        grab: (u16, u16),
        (x, y): (i32, i32),
    ) -> Option<Cell> {
        let spot = spots.get(index).copied().flatten()?;
        let column = (x - area.x).div_euclid(i32::from(CELL_WIDTH)) - i32::from(grab.0);
        let row = (y - area.y).div_euclid(i32::from(CELL_HEIGHT)) - i32::from(grab.1);
        let last_column = i32::from(grid.columns()) - i32::from(spot.size.0);
        let last_row = i32::from(grid.rows()) - i32::from(spot.size.1);
        if last_column < 0 || last_row < 0 {
            return None;
        }
        let at = (u16::try_from(column.clamp(0, last_column)).ok()?, u16::try_from(row.clamp(0, last_row)).ok()?);
        let moved = spot.moved(at);
        let clear = spots
            .iter()
            .enumerate()
            .filter(|(other, _)| *other != index)
            .all(|(_, other)| other.is_none_or(|other| !other.overlaps(moved)));
        (at != spot.at && clear).then_some(at)
    }

    /// Where the keyboard cursor is.
    #[must_use]
    pub fn cursor(mut self, cursor: Option<usize>) -> Self {
        self.cursor = cursor;
        self
    }

    /// Whether the floor takes the keys. It does not while another surface, such as the launcher,
    /// has them.
    #[must_use]
    pub fn keys(mut self, keys: bool) -> Self {
        self.keys = keys;
        self
    }

    /// The grid of the last frame and the area it was drawn in.
    fn last(cx: &mut EventCx<'_, Msg>) -> Option<(Grid, Rect)> {
        let memory = cx.memory::<FloorMemory>();
        memory.grid.clone().map(|grid| (grid, memory.area))
    }

    /// Where the gadgets stood in the last frame.
    fn last_spots(cx: &mut EventCx<'_, Msg>) -> Vec<Option<Spot>> {
        cx.memory::<FloorMemory>().spots.clone()
    }

    /// The icon at a screen cell.
    fn icon_at(grid: &Grid, area: Rect, x: i32, y: i32) -> Option<usize> {
        grid.at(x - area.x, y - area.y)
    }

    fn on_key(&self, cx: &mut EventCx<'_, Msg>, key: &KeyEvent) -> bool {
        let Some((grid, _)) = Self::last(cx) else { return false };
        if grid.shown() == 0 {
            return false;
        }
        let steps = [(Key::Up, Step::Up), (Key::Down, Step::Down), (Key::Left, Step::Left), (Key::Right, Step::Right)];
        // Shift with an arrow carries the icon under the cursor one cell that way, as a drag
        // would; into a cell that holds an icon, the two change places.
        let shifted = Modifiers { shift: true, ..Modifiers::default() };
        if key.kind != KeyKind::Release
            && key.chord.mods == shifted
            && let Some((_, step)) = steps.into_iter().find(|(chord, _)| key.chord.key == *chord)
        {
            let Some(index) = self.cursor else { return false };
            let Some(cell) = grid.cell(index).and_then(|from| grid.beside(from, step)) else { return false };
            if grid.is_blocked(cell) {
                // A gadget stands there: the icon stays, as it does against the floor's edge.
                return true;
            }
            cx.emit((self.on)(Action::Place { index, moves: vec![(index, cell)], layout: grid.cells().to_vec() }));
            return true;
        }
        if let Some((_, step)) = steps.into_iter().find(|(chord, _)| key.is_plain(*chord)) {
            let from = self.cursor.filter(|index| grid.cell(*index).is_some());
            // The first key lands on the first icon drawn.
            let first = (0..grid.cells().len()).find(|index| grid.cell(*index).is_some()).unwrap_or(0);
            let target = from.map_or(first, |index| grid.step(index, step));
            cx.memory::<FloorMemory>().typed = None;
            cx.emit((self.on)(Action::Cursor(target)));
            return true;
        }
        if key.is_plain(Key::Enter) {
            let Some(index) = self.cursor.filter(|index| grid.cell(*index).is_some()) else { return false };
            cx.emit((self.on)(Action::Open(index)));
            return true;
        }
        if key.is_plain(Key::Esc) {
            cx.emit((self.on)(Action::Clear));
            return true;
        }
        // Typing a letter jumps to the icon whose name starts with it; the same letter again walks
        // on to the next one.
        let Some(letter) = key.text.filter(|letter| !letter.is_whitespace() && !letter.is_control()) else {
            return false;
        };
        if key.chord.mods.ctrl || key.chord.mods.alt {
            return false;
        }
        let from = cx.memory::<FloorMemory>().typed.or(self.cursor);
        let Some(found) = grid.jump(&self.names, from, letter) else { return false };
        cx.memory::<FloorMemory>().typed = Some(found);
        cx.emit((self.on)(Action::Cursor(found)));
        true
    }

    fn on_mouse(&self, cx: &mut EventCx<'_, Msg>, mouse: &MouseEvent) -> bool {
        let Some((grid, area)) = Self::last(cx) else { return false };
        let under = Self::icon_at(&grid, area, mouse.x, mouse.y);
        let spots = Self::last_spots(cx);
        match mouse.kind {
            MouseKind::Down(MouseButton::Right) => {
                // An icon's own menu belongs to its cell; the floor's menu is the one wrapping
                // the floor, so a right click on bare floor is left to bubble up to it.
                let Some(index) = under else { return false };
                let Some((node, rect)) = self.cell_of(index, &grid, area) else { return false };
                cx.forward(node, rect, &Event::Mouse(*mouse))
            }
            MouseKind::Down(MouseButton::Left) => {
                cx.capture_pointer();
                // A press on a gadget that its content did not take holds the gadget, to carry it.
                let held = under.is_none().then(|| Self::gadget_at(&spots, area, mouse.x, mouse.y)).flatten();
                let memory = cx.memory::<FloorMemory>();
                memory.gadget = held.and_then(|index| {
                    let spot = spots.get(index).copied().flatten()?;
                    let column = (mouse.x - area.x).div_euclid(i32::from(CELL_WIDTH)) - i32::from(spot.at.0);
                    let row = (mouse.y - area.y).div_euclid(i32::from(CELL_HEIGHT)) - i32::from(spot.at.1);
                    Some((index, (u16::try_from(column).ok()?, u16::try_from(row).ok()?)))
                });
                memory.gadget_target = None;
                if memory.gadget.is_some() {
                    memory.pressed = None;
                    memory.band_from = None;
                    memory.band = None;
                    return true;
                }
                memory.pressed = under;
                memory.target = None;
                memory.band_from = under.is_none().then_some((mouse.x, mouse.y));
                memory.band = None;
                true
            }
            MouseKind::Drag(MouseButton::Left) => {
                if let Some((index, grab)) = cx.memory::<FloorMemory>().gadget {
                    let target = self.gadget_landing(&grid, &spots, area, index, grab, (mouse.x, mouse.y));
                    cx.memory::<FloorMemory>().gadget_target = target;
                    return true;
                }
                let memory = cx.memory::<FloorMemory>();
                if let Some(from) = memory.band_from {
                    memory.band = Some(band(from, (mouse.x, mouse.y)));
                    return true;
                }
                if memory.pressed.is_some() {
                    memory.target = grid.cell_at(mouse.x - area.x, mouse.y - area.y);
                    return true;
                }
                false
            }
            MouseKind::Up(MouseButton::Left) => {
                let (gadget, landing) = {
                    let memory = cx.memory::<FloorMemory>();
                    (memory.gadget.take(), memory.gadget_target.take())
                };
                if let Some((index, _)) = gadget {
                    if let Some(at) = landing {
                        cx.emit((self.on)(Action::PlaceGadget { index, at }));
                    }
                    return true;
                }
                let now = cx.now();
                let (pressed, target, from, drawn, clicked) = {
                    let memory = cx.memory::<FloorMemory>();
                    let taken = (memory.pressed, memory.target, memory.band_from, memory.band, memory.clicked);
                    memory.pressed = None;
                    memory.target = None;
                    memory.band_from = None;
                    memory.band = None;
                    memory.clicked = pressed_click(taken.0, now);
                    taken
                };
                if let Some(index) = pressed {
                    let action = match target.filter(|cell| grid.cell(index) != Some(*cell)) {
                        Some(cell) => {
                            // A drop on a gadget, or of a group part of which would land on one,
                            // is refused: no icon ever stands under a gadget.
                            let Some(moves) = self.lands(&grid, index, cell) else { return true };
                            // A group already against the edge it was pulled towards goes
                            // nowhere; the drop is not a click either, so the selection stays.
                            if moves.iter().all(|(moved, to)| grid.cell(*moved) == Some(*to)) {
                                return true;
                            }
                            Action::Place { index, moves, layout: grid.cells().to_vec() }
                        }
                        None if clicked
                            .is_some_and(|(last, at)| last == index && now.saturating_sub(at) <= DOUBLE_CLICK) =>
                        {
                            Action::Open(index)
                        }
                        None if mouse.mods.shift => Action::Extend(index),
                        None => Action::Select { index, add: mouse.mods.ctrl },
                    };
                    cx.emit((self.on)(action));
                    return true;
                }
                if from.is_some() {
                    let inside = drawn.map(|band| grid.inside(shift(band, area))).unwrap_or_default();
                    let action = if inside.is_empty() { Action::Clear } else { Action::Band(inside) };
                    cx.emit((self.on)(action));
                    return true;
                }
                false
            }
            _ => false,
        }
    }

    /// The node and screen rectangle of the icon at `index`.
    fn cell_of(&self, index: usize, grid: &Grid, area: Rect) -> Option<(&Node<Msg>, Rect)> {
        let rect = grid.rect(index)?;
        let node = self.cells.get(index).filter(|_| index < self.icon_count())?;
        Some((node, Rect::new(area.x + rect.x, area.y + rect.y, rect.width, rect.height)))
    }
}

/// What a release remembers as the last click: the icon it landed on, if it landed on one.
fn pressed_click(pressed: Option<usize>, now: Duration) -> Option<(usize, Duration)> {
    pressed.map(|index| (index, now))
}

/// The rectangle two corners make, however they were dragged.
fn band(from: (i32, i32), to: (i32, i32)) -> Rect {
    let (left, right) = (from.0.min(to.0), from.0.max(to.0));
    let (top, bottom) = (from.1.min(to.1), from.1.max(to.1));
    let width = u16::try_from(right - left + 1).unwrap_or(u16::MAX);
    let height = u16::try_from(bottom - top + 1).unwrap_or(u16::MAX);
    Rect::new(left, top, width, height)
}

/// A screen rectangle in the floor's own cells.
fn shift(rect: Rect, area: Rect) -> Rect {
    Rect::new(rect.x - area.x, rect.y - area.y, rect.width, rect.height)
}

impl<Msg: 'static> Widget<Msg> for Floor<Msg> {
    fn measure(&self, _cx: &mut MeasureCx<'_>, available: Size) -> Size {
        available
    }

    fn paint(&self, cx: &mut PaintCx<'_>, area: Rect) {
        let (grid, spots) = self.layout(area.size());
        let (pressed, target, gadget_target) = {
            let memory = cx.memory::<FloorMemory>();
            memory.grid = Some(grid.clone());
            memory.area = area;
            memory.spots.clone_from(&spots);
            (memory.pressed, memory.target, memory.gadget_target)
        };
        if area.is_empty() {
            return;
        }
        cx.register_hit(area);
        if self.keys {
            cx.register_focusable();
        }
        let icons = self.icon_count();
        for (index, spot) in spots.iter().enumerate() {
            if let (Some(spot), Some(node)) = (spot, self.cells.get(icons + index)) {
                cx.paint_child(node, Self::surface(*spot, area));
            }
        }
        for index in 0..icons {
            if let Some((node, rect)) = self.cell_of(index, &grid, area) {
                cx.paint_child(node, rect);
            }
        }
        // Where a dragged gadget would land: the same tone over the surface it would have there.
        let held = cx.memory::<FloorMemory>().gadget;
        if let (Some((index, _)), Some(at)) = (held, gadget_target)
            && let Some(spot) = spots.get(index).copied().flatten()
        {
            cx.tint(Self::surface(spot.moved(at), area).intersect(area), cx.color("accent"), DROP_MIX);
        }
        // Where a dragged icon would land, or every icon of the selection it carries: a tone
        // over each whole cell, over an icon standing there too, since it will make way. A drop a
        // gadget would refuse lights nothing.
        if let (Some(index), Some(cell)) = (pressed, target)
            && grid.cell(index) != Some(cell)
            && let Some(moves) = self.lands(&grid, index, cell)
        {
            for (_, cell) in moves {
                let rect = Grid::cell_rect(cell);
                let rect = Rect::new(area.x + rect.x, area.y + rect.y, rect.width, rect.height);
                cx.tint(rect.intersect(area), cx.color("accent"), DROP_MIX);
            }
        }
        let band = cx.memory::<FloorMemory>().band;
        if let Some(band) = band {
            // The band is a tone mixed into the floor, not an outline: a line would be the one
            // thing the desktop never draws.
            cx.tint(band.intersect(area), cx.color("accent"), BAND_MIX);
        }
    }

    fn event(&self, cx: &mut EventCx<'_, Msg>, event: &Event) -> bool {
        match event {
            Event::Key(key) if self.keys => self.on_key(cx, key),
            Event::Mouse(mouse) => self.on_mouse(cx, mouse),
            _ => false,
        }
    }

    fn focusable(&self) -> bool {
        self.keys
    }

    fn children(&self) -> &[Node<Msg>] {
        &self.cells
    }

    fn children_mut(&mut self) -> &mut [Node<Msg>] {
        &mut self.cells
    }
}

impl<Msg: 'static> Container<Msg> for Floor<Msg> {
    fn set_children(&mut self, children: Vec<Node<Msg>>) {
        self.cells = children;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_band_is_the_same_rectangle_whichever_way_it_was_dragged() {
        let wanted = Rect::new(2, 1, 4, 3);
        assert_eq!(band((2, 1), (5, 3)), wanted);
        assert_eq!(band((5, 3), (2, 1)), wanted);
        assert_eq!(band((5, 1), (2, 3)), wanted);
        assert_eq!(band((2, 1), (2, 1)), Rect::new(2, 1, 1, 1), "a band over one cell is one cell");
    }

    #[test]
    fn a_screen_rectangle_reads_in_the_floors_own_cells() {
        assert_eq!(shift(Rect::new(12, 5, 3, 2), Rect::new(10, 4, 20, 10)), Rect::new(2, 1, 3, 2));
    }
}
