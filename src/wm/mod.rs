//! The window manager: every window of the desktop, its place and its state.
//!
//! Nothing here draws, reads a key or starts a program. [`Windows`] holds the truth the screen
//! only shows: which windows exist, in which order they lie over each other, which one has the
//! focus, where each one sits and what it holds. The screen asks it questions and tells it what
//! the person did; it answers with rectangles.
//!
//! It is kept this way on purpose. A later version keeps the programs in a background process and
//! lets a screen connect to it, and then this state moves into that process unchanged: the screen
//! only swaps a direct call for a message.
//!
//! Two promises hold after every operation, and the tests check them over long random sequences:
//! every window lies whole inside the desktop and is no smaller than [`MIN_SIZE`] (or the whole
//! desktop, when the terminal is smaller than that), and exactly one window has the focus as long
//! as one is not minimized.

pub mod layout;
pub mod snap;
pub mod tile;
pub mod view;
mod window;

#[cfg(test)]
mod property;

use qframe::geometry::{Rect, Size};

use crate::apps::{Entry, Launch, Screen};
use crate::dock;

pub use layout::{Grip, MIN_SIZE};
pub use snap::{Edge, Snap};
pub use tile::TooSmall;
pub use view::{Action, Dragging, Look, Preview};
pub use window::{Body, Placement, Run, Window, WindowId};

/// How far a new window is offset from the one before it, in columns and rows, and how many
/// offsets there are before they start again.
const CASCADE: (i32, i32, u32) = (3, 2, 6);

/// Every window of one desktop, from the floor up.
///
/// The list is in stacking order: the first window is the furthest back, the last one is in front.
/// A minimized window keeps its place in it.
#[derive(Debug, Clone)]
pub struct Windows {
    screen: Size,
    windows: Vec<Window>,
    focus: Option<WindowId>,
    next: WindowId,
    cascade: u32,
}

impl Windows {
    /// An empty desktop on a terminal of `screen` cells.
    #[must_use]
    pub fn new(screen: Size) -> Self {
        Self { screen, windows: Vec::new(), focus: None, next: WindowId::first(), cascade: 0 }
    }

    /// The part of the terminal windows may use, in the desktop's own coordinates.
    ///
    /// The dock is always there and always one row, at whichever edge the settings name, so the
    /// desktop is one row shorter than the terminal and nothing is ever hidden behind the dock.
    /// Which edge that is does not reach here: the screen draws the dock as a bar of the
    /// application frame and the windows inside what is left, so the desktop's first row is the
    /// first row the dock left, wherever that lies on the terminal. A rectangle from here is
    /// therefore read against the desktop, not against the terminal.
    #[must_use]
    pub fn area(&self) -> Rect {
        Rect::new(0, 0, self.screen.width, self.screen.height.saturating_sub(dock::HEIGHT))
    }

    /// The smallest size a window may have on this terminal.
    #[must_use]
    pub fn min_size(&self) -> Size {
        layout::min_size(self.area())
    }

    /// Takes the desktop to a terminal of `screen` cells.
    ///
    /// Windows are pushed into the new desktop, not scaled: a terminal program's text does not
    /// stretch, so a window that keeps its size keeps its content readable. A window that no
    /// longer fits takes what there is. Maximized and snapped windows follow the new desktop, and
    /// the rectangle they will be restored to is brought inside it as well, so restoring later
    /// cannot put a window outside.
    pub fn resize(&mut self, screen: Size) {
        self.screen = screen;
        let area = self.area();
        for window in &mut self.windows {
            match window.placement {
                Placement::Floating => window.rect = layout::clamp_into(window.rect, area),
                Placement::Maximized { restore } => {
                    window.placement = Placement::Maximized { restore: layout::clamp_into(restore, area) };
                    window.rect = area;
                }
                Placement::Snapped { edge, restore } => {
                    window.placement = Placement::Snapped { edge, restore: layout::clamp_into(restore, area) };
                    window.rect = snap::rect_of(edge, area);
                }
            }
        }
    }

    /// How many windows are open, minimized ones included.
    #[must_use]
    pub fn len(&self) -> usize {
        self.windows.len()
    }

    /// Whether no window is open.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }

    /// Every window from the furthest back to the one in front, minimized ones included.
    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &Window> {
        self.windows.iter()
    }

    /// Every drawn window, from the furthest back to the one in front: the order to draw them in.
    pub fn visible(&self) -> impl DoubleEndedIterator<Item = &Window> {
        self.windows.iter().filter(|window| !window.minimized)
    }

    /// The window `id`, when it is open.
    #[must_use]
    pub fn get(&self, id: WindowId) -> Option<&Window> {
        self.windows.iter().find(|window| window.id == id)
    }

    /// The window in front of all the others, when one is drawn.
    #[must_use]
    pub fn front(&self) -> Option<&Window> {
        self.visible().next_back()
    }

    /// Which window has the focus: the one every key goes to.
    #[must_use]
    pub fn focus(&self) -> Option<WindowId> {
        self.focus
    }

    /// The window that has the focus.
    #[must_use]
    pub fn focused(&self) -> Option<&Window> {
        self.focus.and_then(|id| self.get(id))
    }

    /// The first open window that came from the entry `entry`, from the front.
    ///
    /// An entry marked `single` opens once: the launcher brings this window forward instead.
    #[must_use]
    pub fn of_entry(&self, entry: &str) -> Option<WindowId> {
        self.visible()
            .rev()
            .chain(self.windows.iter().filter(|window| window.minimized))
            .find(|window| window.entry.id == entry)
            .map(|window| window.id)
    }

    /// Opens a window for `entry` and gives it the focus.
    ///
    /// It takes the size the entry asks for, or two thirds of the desktop, and sits a few cells
    /// down and to the right of the window before it, so a second window of the same program does
    /// not hide the first one exactly. An entry that asks to open maximized does.
    pub fn open(&mut self, entry: &Entry) -> WindowId {
        let area = self.area();
        let floating = self.cascaded(layout::opening_size(area, entry.window.size));
        let (rect, placement) = if entry.window.maximized {
            (area, Placement::Maximized { restore: floating })
        } else {
            (floating, Placement::Floating)
        };
        let body = match entry.launch {
            // A Terminal window runs a program as much as a command entry does: the shell. Only
            // the screens qdesk draws itself and the files it shows in a viewer hold none.
            Launch::Command(_) | Launch::Screen(Screen::Terminal) => Body::Program(Run::Waiting),
            Launch::Screen(Screen::Settings | Screen::Files) | Launch::Open(_) => Body::Screen,
        };
        let id = self.next;
        self.next = id.next();
        self.cascade = self.cascade.wrapping_add(1);
        self.windows.push(Window { id, entry: entry.clone(), body, title: None, rect, placement, minimized: false });
        self.focus = Some(id);
        id
    }

    /// Where a window of `size` opens: a few cells on from the last one, back to the middle once
    /// the offsets are used up, and never exactly over a window that is already drawn.
    fn cascaded(&self, size: Size) -> Rect {
        let area = self.area();
        let base = area.centered(size);
        let (dx, dy, steps) = CASCADE;
        for step in 0..steps {
            let away = i32::try_from(self.cascade.wrapping_add(step) % steps).unwrap_or(0);
            let shifted = Rect::new(base.x + dx * away, base.y + dy * away, size.width, size.height);
            let rect = layout::clamp_into(shifted, area);
            if self.visible().all(|window| window.rect != rect) {
                return rect;
            }
        }
        layout::clamp_into(base, area)
    }

    /// Closes the window `id` and gives the focus to the one below it; `false` when there is no
    /// such window.
    ///
    /// Whether a running program may be killed is asked before this: a window with a program of
    /// the person's in it is their data, and the window manager does not decide about it.
    pub fn close(&mut self, id: WindowId) -> bool {
        let Some(index) = self.index_of(id) else { return false };
        self.windows.remove(index);
        if self.focus == Some(id) {
            self.focus = self.below(index);
        }
        true
    }

    /// Brings the window `id` in front of the others and gives it the focus.
    ///
    /// A minimized window is not brought forward: it is not on the desktop to be clicked, and the
    /// focus never lands on one. [`Windows::restore`] brings it back first.
    pub fn raise(&mut self, id: WindowId) -> bool {
        let Some(index) = self.index_of(id) else { return false };
        if self.windows[index].minimized {
            return false;
        }
        let window = self.windows.remove(index);
        self.windows.push(window);
        self.focus = Some(id);
        true
    }

    /// Takes the window `id` off the desktop, down to the dock, and gives the focus to the window
    /// below it. Its program keeps running.
    pub fn minimize(&mut self, id: WindowId) -> bool {
        let Some(index) = self.index_of(id) else { return false };
        if self.windows[index].minimized {
            return false;
        }
        self.windows[index].minimized = true;
        if self.focus == Some(id) {
            self.focus = self.below(index);
        }
        true
    }

    /// Brings the minimized window `id` back to the desktop, in front and with the focus.
    pub fn restore(&mut self, id: WindowId) -> bool {
        let Some(index) = self.index_of(id) else { return false };
        if !self.windows[index].minimized {
            return false;
        }
        self.windows[index].minimized = false;
        self.raise(id)
    }

    /// Fills the desktop with the window `id`, remembering the rectangle it had.
    ///
    /// What it remembers is always the rectangle it has right now, even when it is already snapped
    /// to an edge: a window goes back to where it was, one step at a time, and never to a size the
    /// person has since replaced.
    pub fn maximize(&mut self, id: WindowId) -> bool {
        let area = self.area();
        let Some(window) = self.window_mut(id) else { return false };
        if window.is_maximized() {
            return false;
        }
        let restore = window.rect;
        window.placement = Placement::Maximized { restore };
        window.rect = area;
        true
    }

    /// Gives the maximized window `id` back exactly the rectangle it had before.
    pub fn unmaximize(&mut self, id: WindowId) -> bool {
        let area = self.area();
        let Some(window) = self.window_mut(id) else { return false };
        let Placement::Maximized { restore } = window.placement else { return false };
        window.placement = Placement::Floating;
        window.rect = layout::clamp_into(restore, area);
        true
    }

    /// Maximizes the window `id`, or restores it when it already fills the desktop: what a double
    /// click on its title does.
    pub fn toggle_maximized(&mut self, id: WindowId) -> bool {
        if self.unmaximize(id) { true } else { self.maximize(id) }
    }

    /// Moves the window `id` by `dx` columns and `dy` rows, keeping it inside the desktop.
    ///
    /// A maximized or snapped window comes loose here: it goes back to the rectangle it had before
    /// and that rectangle follows the drag. Dragging a window that fills the desktop is how a
    /// person says they want it floating again, and the size they had is the size they mean.
    ///
    /// A move of no cells changes nothing, not even the placement: a drag reports the cell the
    /// pointer is on every time it moves, and pressing on the title of a maximized window without
    /// dragging it must leave it maximized.
    pub fn move_by(&mut self, id: WindowId, dx: i32, dy: i32) -> bool {
        let area = self.area();
        let Some(window) = self.window_mut(id) else { return false };
        if dx == 0 && dy == 0 {
            return true;
        }
        if let Some(restore) = window.placement.restore() {
            window.placement = Placement::Floating;
            window.rect = layout::clamp_into(restore, area);
        }
        window.rect = layout::moved(window.rect, dx, dy, area);
        true
    }

    /// Moves the window `id` so its top left corner is at `(x, y)`, as far as the desktop allows.
    pub fn move_to(&mut self, id: WindowId, x: i32, y: i32) -> bool {
        let Some(window) = self.get(id) else { return false };
        let (dx, dy) = (x - window.rect.x, y - window.rect.y);
        self.move_by(id, dx, dy)
    }

    /// Moves the edge or corner `grip` of the window `id` by `dx` columns and `dy` rows.
    ///
    /// A maximized or snapped window floats again at the size it is now, not at the one it came
    /// from: the edge the person is holding has to be the edge that moves.
    pub fn resize_by(&mut self, id: WindowId, grip: Grip, dx: i32, dy: i32) -> bool {
        let area = self.area();
        let Some(window) = self.window_mut(id) else { return false };
        window.placement = Placement::Floating;
        window.rect = layout::resized(window.rect, grip, dx, dy, area);
        true
    }

    /// What the window `id` would snap to if the drag ended now, for the screen to show before it
    /// does. Only a floating window that rests against an edge asks for anything.
    #[must_use]
    pub fn snap_target(&self, id: WindowId) -> Option<Snap> {
        let window = self.get(id)?;
        if window.minimized || window.placement != Placement::Floating {
            return None;
        }
        snap::target(window.rect, self.area())
    }

    /// Snaps the window `id` to `edge`, remembering the rectangle it had.
    pub fn snap(&mut self, id: WindowId, edge: Edge) -> bool {
        let area = self.area();
        let Some(window) = self.window_mut(id) else { return false };
        let restore = window.rect;
        window.placement = Placement::Snapped { edge, restore };
        window.rect = snap::rect_of(edge, area);
        true
    }

    /// Ends a drag of the window `id`: snaps it when it rests against an edge and says which, and
    /// leaves it where it is otherwise.
    pub fn drop_dragged(&mut self, id: WindowId) -> Option<Edge> {
        let edge = self.snap_target(id)?.edge;
        self.snap(id, edge);
        Some(edge)
    }

    /// Lays every drawn window out at once: the focused one large on the left, the others stacked
    /// on the right from the front backwards, with a row of desktop floor between them.
    ///
    /// This is a placement and not a mode; the windows keep floating, so dragging one afterwards
    /// simply moves it.
    ///
    /// # Errors
    ///
    /// Returns [`TooSmall`] and changes nothing when the desktop cannot hold this many windows at
    /// the smallest window size.
    pub fn tile(&mut self) -> Result<(), TooSmall> {
        let area = self.area();
        let order = self.tile_order();
        let rects = tile::rects(area, order.len())?;
        for (id, rect) in order.into_iter().zip(rects) {
            if let Some(window) = self.window_mut(id) {
                window.placement = Placement::Floating;
                window.rect = rect;
            }
        }
        Ok(())
    }

    /// The drawn windows in the order they take the tiled rectangles: the focused one first, so it
    /// gets the large place, then the others from the front backwards.
    fn tile_order(&self) -> Vec<WindowId> {
        let mut order: Vec<WindowId> = self.visible().rev().map(|window| window.id).collect();
        if let Some(focus) = self.focus
            && let Some(at) = order.iter().position(|&id| id == focus)
        {
            let id = order.remove(at);
            order.insert(0, id);
        }
        order
    }

    /// Says that the program of the window `id` has started; `false` when there is no such window
    /// or it holds no program.
    pub fn mark_running(&mut self, id: WindowId) -> bool {
        let Some(window) = self.window_mut(id) else { return false };
        if window.run().is_none() {
            return false;
        }
        window.body = Body::Program(Run::Running);
        true
    }

    /// Says that the program of the window `id` has ended with `code`, and tells what became of
    /// the window.
    ///
    /// A window whose entry says `close_on_exit` closes; every other one stays open and shows the
    /// exit code, so an error message is not lost before it is read.
    pub fn mark_ended(&mut self, id: WindowId, code: Option<i32>) -> Exit {
        let Some(window) = self.window_mut(id) else { return Exit::Unknown };
        if window.run().is_none() {
            return Exit::Unknown;
        }
        window.body = Body::Program(Run::Ended { code });
        if window.entry.close_on_exit {
            self.close(id);
            return Exit::Closed;
        }
        Exit::Kept
    }

    /// Sets the title the program of the window `id` gave itself, or clears it.
    pub fn set_title(&mut self, id: WindowId, title: Option<String>) -> bool {
        let Some(window) = self.window_mut(id) else { return false };
        window.title = title;
        true
    }

    /// Where the window `id` is in the stacking order.
    fn index_of(&self, id: WindowId) -> Option<usize> {
        self.windows.iter().position(|window| window.id == id)
    }

    /// The window `id`, to be changed.
    fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows.iter_mut().find(|window| window.id == id)
    }

    /// The window that takes the focus when the one at `index` gives it up: the nearest drawn one
    /// below it, or the nearest one above when it was at the bottom.
    fn below(&self, index: usize) -> Option<WindowId> {
        let index = index.min(self.windows.len());
        let (under, over) = self.windows.split_at(index);
        under.iter().rev().chain(over.iter()).find(|window| !window.minimized).map(|window| window.id)
    }
}

/// What became of a window when its program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exit {
    /// There is no such window, or it holds no program.
    Unknown,
    /// The window stays open and shows the exit code.
    Kept,
    /// The window closed itself, because its entry says `close_on_exit`.
    Closed,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::apps::{Category, Install, Localized, Source, WindowPrefs};

    /// An entry that starts a program, with everything else left at its default.
    pub(super) fn entry(id: &str) -> Entry {
        Entry {
            id: id.to_owned(),
            name: Localized::plain(id),
            comment: None,
            icon: None,
            launch: Launch::Command(vec![id.to_owned()]),
            folder: None,
            env: Vec::new(),
            category: Category::Other,
            keywords: Vec::new(),
            single: false,
            close_on_exit: false,
            window: WindowPrefs::default(),
            install: Install::default(),
            try_exec: None,
            source: Source::Builtin,
            file: None,
        }
    }

    /// A desktop on an 80x24 terminal: 23 rows for windows and one for the dock.
    fn desk() -> Windows {
        Windows::new(Size::new(80, 24))
    }

    #[test]
    fn the_dock_row_is_not_part_of_the_desktop() {
        assert_eq!(desk().area(), Rect::new(0, 0, 80, 23));
    }

    #[test]
    fn a_new_window_is_in_front_and_has_the_focus() {
        let mut desk = desk();
        let first = desk.open(&entry("one"));
        let second = desk.open(&entry("two"));
        assert_eq!(desk.focus(), Some(second));
        assert_eq!(desk.front().map(Window::id), Some(second));
        assert_eq!(desk.iter().map(Window::id).collect::<Vec<_>>(), vec![first, second]);
    }

    #[test]
    fn ids_are_never_given_out_twice() {
        let mut desk = desk();
        let first = desk.open(&entry("one"));
        desk.close(first);
        let second = desk.open(&entry("one"));
        assert_ne!(first, second);
    }

    #[test]
    fn a_window_opens_at_two_thirds_of_the_desktop_unless_its_entry_says_otherwise() {
        let mut desk = desk();
        let plain = desk.open(&entry("one"));
        assert_eq!(desk.get(plain).map(Window::rect).map(Rect::size), Some(Size::new(52, 14)));
        let mut asks = entry("two");
        asks.window = WindowPrefs { size: Some((30, 8)), maximized: false };
        let sized = desk.open(&asks);
        assert_eq!(desk.get(sized).map(Window::rect).map(Rect::size), Some(Size::new(30, 8)));
    }

    #[test]
    fn a_window_does_not_open_exactly_over_the_one_before_it() {
        let mut desk = desk();
        let mut rects = Vec::new();
        for _ in 0..6 {
            let id = desk.open(&entry("one"));
            rects.push(desk.get(id).map(Window::rect).expect("just opened"));
        }
        for (index, rect) in rects.iter().enumerate() {
            assert!(!rects[index + 1..].contains(rect), "{rect:?} opened twice");
        }
    }

    #[test]
    fn an_entry_may_open_maximized_and_still_know_where_to_go_back_to() {
        let mut desk = desk();
        let mut asks = entry("one");
        asks.window = WindowPrefs { size: Some((30, 8)), maximized: true };
        let id = desk.open(&asks);
        assert_eq!(desk.get(id).map(Window::rect), Some(desk.area()));
        assert!(desk.unmaximize(id));
        assert_eq!(desk.get(id).map(Window::rect).map(Rect::size), Some(Size::new(30, 8)));
    }

    #[test]
    fn a_screen_of_qdesk_has_no_program_to_wait_for() {
        let mut desk = desk();
        let mut screen = entry("settings");
        screen.launch = Launch::Screen(crate::apps::Screen::Settings);
        let id = desk.open(&screen);
        assert_eq!(desk.get(id).map(Window::body), Some(Body::Screen));
        assert_eq!(desk.get(id).and_then(Window::run), None);
        assert!(!desk.mark_running(id));
    }

    #[test]
    fn a_program_goes_from_waiting_to_running_to_ended() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        assert_eq!(desk.get(id).and_then(Window::run), Some(Run::Waiting));
        assert!(desk.mark_running(id));
        assert!(desk.get(id).expect("open").is_running());
        assert_eq!(desk.mark_ended(id, Some(1)), Exit::Kept);
        assert_eq!(desk.get(id).and_then(Window::run), Some(Run::Ended { code: Some(1) }));
        assert!(!desk.get(id).expect("open").is_running());
    }

    #[test]
    fn a_program_that_ended_can_be_started_again() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.mark_ended(id, None);
        assert!(desk.mark_running(id));
        assert_eq!(desk.get(id).and_then(Window::run), Some(Run::Running));
    }

    #[test]
    fn an_entry_that_asks_for_it_closes_when_its_program_ends() {
        let mut desk = desk();
        let mut asks = entry("one");
        asks.close_on_exit = true;
        let id = desk.open(&asks);
        assert_eq!(desk.mark_ended(id, Some(0)), Exit::Closed);
        assert!(desk.get(id).is_none());
        assert_eq!(desk.mark_ended(id, Some(0)), Exit::Unknown);
    }

    #[test]
    fn raising_a_window_brings_it_in_front_and_focuses_it() {
        let mut desk = desk();
        let first = desk.open(&entry("one"));
        let second = desk.open(&entry("two"));
        assert!(desk.raise(first));
        assert_eq!(desk.iter().map(Window::id).collect::<Vec<_>>(), vec![second, first]);
        assert_eq!(desk.focus(), Some(first));
    }

    #[test]
    fn closing_a_window_focuses_the_one_below_it() {
        let mut desk = desk();
        let first = desk.open(&entry("one"));
        let second = desk.open(&entry("two"));
        let third = desk.open(&entry("three"));
        assert!(desk.close(third));
        assert_eq!(desk.focus(), Some(second));
        assert!(desk.close(second));
        assert_eq!(desk.focus(), Some(first));
        assert!(desk.close(first));
        assert_eq!(desk.focus(), None);
    }

    #[test]
    fn closing_the_window_at_the_bottom_focuses_the_one_above_it() {
        let mut desk = desk();
        let first = desk.open(&entry("one"));
        let second = desk.open(&entry("two"));
        desk.raise(first);
        assert!(desk.close(first));
        assert_eq!(desk.focus(), Some(second));
    }

    #[test]
    fn a_minimized_window_keeps_its_place_but_is_not_drawn_or_focused() {
        let mut desk = desk();
        let first = desk.open(&entry("one"));
        let second = desk.open(&entry("two"));
        let third = desk.open(&entry("three"));
        assert!(desk.minimize(second));
        assert_eq!(desk.iter().map(Window::id).collect::<Vec<_>>(), vec![first, second, third]);
        assert_eq!(desk.visible().map(Window::id).collect::<Vec<_>>(), vec![first, third]);
        assert_eq!(desk.focus(), Some(third));
        assert!(!desk.raise(second));
        assert_eq!(desk.focus(), Some(third));
    }

    #[test]
    fn minimizing_the_focused_window_passes_the_focus_down() {
        let mut desk = desk();
        let first = desk.open(&entry("one"));
        let second = desk.open(&entry("two"));
        assert!(desk.minimize(second));
        assert_eq!(desk.focus(), Some(first));
        assert!(desk.minimize(first));
        assert_eq!(desk.focus(), None);
        assert!(desk.restore(second));
        assert_eq!(desk.focus(), Some(second));
        assert_eq!(desk.front().map(Window::id), Some(second));
    }

    #[test]
    fn a_window_that_opens_while_every_other_is_minimized_takes_the_focus() {
        let mut desk = desk();
        let first = desk.open(&entry("one"));
        desk.minimize(first);
        let second = desk.open(&entry("two"));
        assert_eq!(desk.focus(), Some(second));
    }

    #[test]
    fn maximizing_remembers_the_rectangle_and_restoring_gives_it_back() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.move_to(id, 4, 3);
        let before = desk.get(id).map(Window::rect).expect("open");
        assert!(desk.maximize(id));
        assert_eq!(desk.get(id).map(Window::rect), Some(desk.area()));
        assert!(!desk.maximize(id));
        assert!(desk.unmaximize(id));
        assert_eq!(desk.get(id).map(Window::rect), Some(before));
        assert!(!desk.unmaximize(id));
    }

    #[test]
    fn a_double_click_maximizes_and_restores_the_same_window() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        let before = desk.get(id).map(Window::rect).expect("open");
        assert!(desk.toggle_maximized(id));
        assert!(desk.get(id).expect("open").is_maximized());
        assert!(desk.toggle_maximized(id));
        assert_eq!(desk.get(id).map(Window::rect), Some(before));
    }

    #[test]
    fn dragging_a_maximized_window_floats_it_at_the_size_it_had() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.move_to(id, 10, 5);
        desk.maximize(id);
        assert!(desk.move_by(id, 2, 1));
        let window = desk.get(id).expect("open");
        assert_eq!(window.placement(), Placement::Floating);
        assert_eq!(window.rect(), Rect::new(12, 6, 52, 14));
    }

    #[test]
    fn resizing_a_maximized_window_floats_it_at_the_size_it_shows() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.maximize(id);
        assert!(desk.resize_by(id, Grip::Bottom, 0, -3));
        let window = desk.get(id).expect("open");
        assert_eq!(window.placement(), Placement::Floating);
        assert_eq!(window.rect(), Rect::new(0, 0, 80, 20));
    }

    #[test]
    fn a_window_dragged_against_an_edge_asks_to_snap_and_takes_that_half() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        assert_eq!(desk.snap_target(id), None);
        desk.move_by(id, -100, 5);
        assert_eq!(desk.snap_target(id).map(|snap| snap.edge), Some(Edge::Left));
        assert_eq!(desk.drop_dragged(id), Some(Edge::Left));
        let window = desk.get(id).expect("open");
        assert_eq!(window.rect(), Rect::new(0, 0, 40, 23));
        assert_eq!(window.snapped_to(), Some(Edge::Left));
        assert_eq!(desk.snap_target(id), None);
    }

    #[test]
    fn a_snapped_window_floats_again_at_the_rectangle_it_came_from() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.move_to(id, 5, 5);
        desk.snap(id, Edge::Right);
        assert_eq!(desk.get(id).map(Window::rect), Some(Rect::new(40, 0, 40, 23)));
        assert!(desk.move_by(id, 1, 0));
        assert_eq!(desk.get(id).map(Window::rect), Some(Rect::new(6, 5, 52, 14)));
        assert_eq!(desk.get(id).map(Window::placement), Some(Placement::Floating));
    }

    #[test]
    fn pressing_on_a_maximized_window_without_dragging_it_leaves_it_maximized() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.maximize(id);
        assert!(desk.move_by(id, 0, 0));
        assert!(desk.get(id).expect("open").is_maximized());
        assert!(desk.move_to(id, 0, 0));
        assert!(desk.get(id).expect("open").is_maximized());
    }

    #[test]
    fn dropping_a_window_in_the_middle_leaves_it_alone() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.move_to(id, 5, 5);
        assert_eq!(desk.drop_dragged(id), None);
        assert_eq!(desk.get(id).map(Window::rect), Some(Rect::new(5, 5, 52, 14)));
    }

    #[test]
    fn tiling_gives_the_focused_window_the_large_place() {
        let mut desk = desk();
        let first = desk.open(&entry("one"));
        let second = desk.open(&entry("two"));
        let third = desk.open(&entry("three"));
        desk.raise(first);
        assert_eq!(desk.tile(), Ok(()));
        assert_eq!(desk.get(first).map(Window::rect), Some(Rect::new(0, 0, 40, 23)));
        assert_eq!(desk.get(third).map(Window::rect), Some(Rect::new(41, 0, 39, 11)));
        assert_eq!(desk.get(second).map(Window::rect), Some(Rect::new(41, 12, 39, 11)));
    }

    #[test]
    fn tiling_leaves_out_minimized_windows() {
        let mut desk = desk();
        let first = desk.open(&entry("one"));
        let second = desk.open(&entry("two"));
        desk.minimize(second);
        let before = desk.get(second).map(Window::rect);
        assert_eq!(desk.tile(), Ok(()));
        assert_eq!(desk.get(first).map(Window::rect), Some(desk.area()));
        assert_eq!(desk.get(second).map(Window::rect), before);
    }

    #[test]
    fn tiling_an_empty_desktop_does_nothing() {
        assert_eq!(desk().tile(), Ok(()));
    }

    #[test]
    fn tiling_frees_a_maximized_window() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.maximize(id);
        assert_eq!(desk.tile(), Ok(()));
        assert_eq!(desk.get(id).map(Window::placement), Some(Placement::Floating));
    }

    #[test]
    fn a_desktop_too_small_to_tile_says_so_and_changes_nothing() {
        let mut desk = Windows::new(Size::new(80, 24));
        for name in ["one", "two", "three", "four", "five", "six"] {
            desk.open(&entry(name));
        }
        let before: Vec<Rect> = desk.iter().map(Window::rect).collect();
        assert_eq!(desk.tile(), Err(TooSmall::Short { fits: 5 }));
        assert_eq!(desk.iter().map(Window::rect).collect::<Vec<_>>(), before);
    }

    #[test]
    fn a_growing_terminal_leaves_windows_where_they_are() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.move_to(id, 5, 5);
        desk.resize(Size::new(200, 50));
        assert_eq!(desk.get(id).map(Window::rect), Some(Rect::new(5, 5, 52, 14)));
    }

    #[test]
    fn a_shrinking_terminal_pushes_windows_in() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.move_to(id, 20, 8);
        desk.resize(Size::new(60, 16));
        assert_eq!(desk.get(id).map(Window::rect), Some(Rect::new(8, 1, 52, 14)));
    }

    #[test]
    fn a_window_too_large_for_the_new_terminal_takes_what_there_is() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.resize(Size::new(24, 7));
        assert_eq!(desk.get(id).map(Window::rect), Some(Rect::new(0, 0, 24, 6)));
    }

    #[test]
    fn a_window_smaller_than_the_smallest_size_takes_the_whole_desktop() {
        let mut desk = Windows::new(Size::new(16, 4));
        let id = desk.open(&entry("one"));
        assert_eq!(desk.get(id).map(Window::rect), Some(Rect::new(0, 0, 16, 3)));
    }

    #[test]
    fn maximized_and_snapped_windows_follow_the_new_terminal() {
        let mut desk = desk();
        let big = desk.open(&entry("one"));
        let half = desk.open(&entry("two"));
        desk.maximize(big);
        desk.snap(half, Edge::Right);
        desk.resize(Size::new(100, 31));
        assert_eq!(desk.get(big).map(Window::rect), Some(Rect::new(0, 0, 100, 30)));
        assert_eq!(desk.get(half).map(Window::rect), Some(Rect::new(50, 0, 50, 30)));
    }

    #[test]
    fn a_restored_rectangle_is_brought_inside_the_new_terminal() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.move_to(id, 25, 8);
        desk.maximize(id);
        desk.resize(Size::new(60, 16));
        assert!(desk.unmaximize(id));
        let rect = desk.get(id).map(Window::rect).expect("open");
        assert_eq!(rect, Rect::new(8, 1, 52, 14));
    }

    #[test]
    fn a_single_entry_is_found_by_its_id() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.open(&entry("two"));
        assert_eq!(desk.of_entry("one"), Some(id));
        assert_eq!(desk.of_entry("three"), None);
    }

    #[test]
    fn a_minimized_window_is_still_found_by_its_entry() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.minimize(id);
        assert_eq!(desk.of_entry("one"), Some(id));
    }

    #[test]
    fn the_title_a_program_gives_itself_is_kept() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        assert_eq!(desk.get(id).and_then(Window::title), None);
        assert!(desk.set_title(id, Some("~/projects".to_owned())));
        assert_eq!(desk.get(id).and_then(Window::title), Some("~/projects"));
        assert!(desk.set_title(id, None));
        assert_eq!(desk.get(id).and_then(Window::title), None);
    }

    #[test]
    fn nothing_happens_to_a_window_that_is_not_there() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.close(id);
        assert!(!desk.close(id));
        assert!(!desk.raise(id));
        assert!(!desk.minimize(id));
        assert!(!desk.restore(id));
        assert!(!desk.maximize(id));
        assert!(!desk.unmaximize(id));
        assert!(!desk.move_by(id, 1, 1));
        assert!(!desk.move_to(id, 1, 1));
        assert!(!desk.resize_by(id, Grip::Right, 1, 1));
        assert!(!desk.snap(id, Edge::Left));
        assert_eq!(desk.snap_target(id), None);
        assert_eq!(desk.drop_dragged(id), None);
        assert!(!desk.set_title(id, None));
        assert!(!desk.mark_running(id));
        assert_eq!(desk.mark_ended(id, None), Exit::Unknown);
    }
}
