//! The floor of the desktop and the icons standing on it.
//!
//! [`grid`] says where an icon goes and [`order`] what the person's own order is; both are pure.
//! [`Floor`] draws the cells and reads the mouse and the keys, and [`IconCell`] draws one icon.
//! The floor keeps nothing of its own: the application owns the selection, the cursor and the
//! order, and hears about every change as an [`Action`].
//!
//! The floor is bare (VISION §4): no wallpaper, no frame, no line anywhere. A selected icon is
//! the raised surface tone with the accent pillar down its left edge, and the band drawn from
//! empty floor is the accent mixed into the floor, never an outline.

pub mod grid;
pub mod order;

use std::time::Duration;

use qframe::event::{Event, KeyEvent, MouseButton, MouseEvent, MouseKind};
use qframe::geometry::{Rect, Size};
use qframe::icons::Icons;
use qframe::keymap::Key;
use qframe::style::CellStyle;
use qframe::text;
use qframe::widget::{Container, EventCx, MeasureCx, Node, PaintCx, Widget};

use crate::apps::{Category, Entry};

pub use grid::{CELL_HEIGHT, CELL_WIDTH, Grid, Step};
pub use order::Desktop;

/// The icon of the family, on the launcher button. The framework's own icon set holds it; the
/// name is looked up every frame rather than resolved once, so a set that does not (an older
/// framework, or a set of the person's own) shows [`FAMILY_FALLBACK`] instead of an empty cell.
pub const FAMILY_ICON: &str = "family";

/// What stands in for [`FAMILY_ICON`] in an icon set without that key.
pub const FAMILY_FALLBACK: &str = "project";

/// Two clicks on the same icon closer together than this open it.
pub const DOUBLE_CLICK: Duration = Duration::from_millis(400);

/// How much of the accent the band mixes into the floor.
const BAND_MIX: f32 = 0.20;

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
    /// A band was drawn from empty floor over these icons.
    Band(Vec<usize>),
    /// Empty floor was clicked: nothing is selected any more.
    Clear,
    /// The keyboard cursor moved to this icon.
    Cursor(usize),
    /// An icon was opened: a double click, or Enter on the cursor.
    Open(usize),
    /// An icon was dragged onto another cell of the grid.
    Drop {
        /// Where it came from.
        from: usize,
        /// Where it was dropped.
        to: usize,
    },
}

/// The key of the icon a button shows for the family: the one the icon set holds.
#[must_use]
pub fn family_icon(icons: &Icons) -> &'static str {
    if icons.contains(FAMILY_ICON) { FAMILY_ICON } else { FAMILY_FALLBACK }
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

/// The icon of a category, by name.
fn category_icon(category: Category, icons: &Icons) -> &'static str {
    match category {
        Category::System => "settings",
        Category::Development => "project",
        Category::Files => "folder",
        Category::Network => "dot-outline",
        Category::Office => "file",
        Category::Media => "dot",
        Category::Family => family_icon(icons),
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
    /// The icon the left button went down on, and whether the mouse has left its cell since.
    pressed: Option<usize>,
    dragging: Option<usize>,
    /// Where a band started, and the band it has grown to.
    band_from: Option<(i32, i32)>,
    band: Option<Rect>,
    /// The last click, for telling a double click from two single ones.
    clicked: Option<(usize, Duration)>,
    /// The icon the last typed letter found, so the same letter walks on.
    typed: Option<usize>,
}

/// The floor: the icon cells laid out on the grid, the band, and the keys of the desktop.
///
/// Add the cells with [`View::add_with`](qframe::widget::View::add_with), one per icon in the
/// order they stand in; each may be wrapped in its own [`ContextMenu`](qframe::widgets::ContextMenu)
/// and [`Tooltip`](qframe::widgets::Tooltip).
pub struct Floor<Msg> {
    names: Vec<String>,
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
        Self { names, cursor: None, keys: true, on: Box::new(on), cells: Vec::new() }
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
        memory.grid.map(|grid| (grid, memory.area))
    }

    /// The icon at a screen cell.
    fn icon_at(grid: Grid, area: Rect, x: i32, y: i32) -> Option<usize> {
        grid.at(x - area.x, y - area.y)
    }

    fn on_key(&self, cx: &mut EventCx<'_, Msg>, key: &KeyEvent) -> bool {
        let Some((grid, _)) = Self::last(cx) else { return false };
        if grid.shown() == 0 {
            return false;
        }
        let steps = [(Key::Up, Step::Up), (Key::Down, Step::Down), (Key::Left, Step::Left), (Key::Right, Step::Right)];
        if let Some((_, step)) = steps.into_iter().find(|(chord, _)| key.is_plain(*chord)) {
            let from = self.cursor.filter(|index| *index < grid.shown());
            let target = from.map_or(0, |index| grid.step(index, step));
            cx.memory::<FloorMemory>().typed = None;
            cx.emit((self.on)(Action::Cursor(target)));
            return true;
        }
        if key.is_plain(Key::Enter) {
            let Some(index) = self.cursor.filter(|index| *index < grid.shown()) else { return false };
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
        let under = Self::icon_at(grid, area, mouse.x, mouse.y);
        match mouse.kind {
            MouseKind::Down(MouseButton::Right) => {
                // An icon's own menu belongs to its cell; the floor's menu is the one wrapping
                // the floor, so a right click on bare floor is left to bubble up to it.
                let Some(index) = under else { return false };
                let Some((node, rect)) = self.cell_of(index, grid, area) else { return false };
                cx.forward(node, rect, &Event::Mouse(*mouse))
            }
            MouseKind::Down(MouseButton::Left) => {
                cx.capture_pointer();
                let memory = cx.memory::<FloorMemory>();
                memory.pressed = under;
                memory.dragging = None;
                memory.band_from = under.is_none().then_some((mouse.x, mouse.y));
                memory.band = None;
                true
            }
            MouseKind::Drag(MouseButton::Left) => {
                let memory = cx.memory::<FloorMemory>();
                if let Some(from) = memory.band_from {
                    memory.band = Some(band(from, (mouse.x, mouse.y)));
                    return true;
                }
                if memory.pressed.is_some() {
                    memory.dragging = under;
                    return true;
                }
                false
            }
            MouseKind::Up(MouseButton::Left) => {
                let now = cx.now();
                let (pressed, dragging, from, drawn, clicked) = {
                    let memory = cx.memory::<FloorMemory>();
                    let taken = (memory.pressed, memory.dragging, memory.band_from, memory.band, memory.clicked);
                    memory.pressed = None;
                    memory.dragging = None;
                    memory.band_from = None;
                    memory.band = None;
                    memory.clicked = pressed_click(taken.0, now);
                    taken
                };
                if let Some(index) = pressed {
                    let action = match dragging.filter(|to| *to != index) {
                        Some(to) => Action::Drop { from: index, to },
                        None if clicked
                            .is_some_and(|(last, at)| last == index && now.saturating_sub(at) <= DOUBLE_CLICK) =>
                        {
                            Action::Open(index)
                        }
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
    fn cell_of(&self, index: usize, grid: Grid, area: Rect) -> Option<(&Node<Msg>, Rect)> {
        let rect = grid.rect(index)?;
        let node = self.cells.get(index)?;
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
        let grid = Grid::new(area.size(), self.cells.len());
        {
            let memory = cx.memory::<FloorMemory>();
            memory.grid = Some(grid);
            memory.area = area;
        }
        if area.is_empty() {
            return;
        }
        cx.register_hit(area);
        if self.keys {
            cx.register_focusable();
        }
        for index in 0..grid.shown() {
            if let Some((node, rect)) = self.cell_of(index, grid, area) {
                cx.paint_child(node, rect);
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
