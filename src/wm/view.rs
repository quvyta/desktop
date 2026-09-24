//! Drawing the window manager on the screen, and reading the pointer on it.
//!
//! [`super`] holds the truth about the windows and changes nothing by itself; this module puts
//! that truth on the screen with the framework's [`Window`](qframe::widgets::Window) surface and
//! turns what the pointer does into an [`Action`] the desktop applies. Nothing here decides
//! anything: an [`Action`] says what the person did, not what the window becomes.
//!
//! The arithmetic is kept out of the painting. Where a ghost starts, where it goes, which window
//! a key picks and what the layer over the windows shows are functions with tests of their own;
//! [`view`] only draws.

use std::path::Path;

use qframe::geometry::{Rect, Size};
use qframe::icons::Glyph;
use qframe::prelude::*;
use qframe::widgets::{Ghost, Window as Surface, WindowEdge, WindowEvent};

use super::layout::clamp_into;
use super::{Grip, Window, WindowId, Windows, snap};

/// Columns and rows the window surface keeps for itself around its body: the pillar column and the
/// cell after it, the column of the right edge handle, the title strip above and the row of the
/// bottom edge handle below. It is the framework surface's own arithmetic, kept here because the
/// first size of a program's screen has to be known before the surface has ever been drawn.
const FRAME: (u16, u16) = (3, 2);

/// How much of the accent the ghost of a dragged window mixes into the floor.
const GHOST_MIX: f32 = 0.25;

/// How much of the accent a snap preview mixes into what it covers. It is lighter than a ghost:
/// it says where a window would land, not where it is.
const SNAP_MIX: f32 = 0.20;

/// What the person did to a window with the pointer.
///
/// The deltas are the ones the framework reports: cells since the last message of the same drag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// A window that did not have the focus was pressed: bring it forward.
    Focus(WindowId),
    /// A window was dragged by this many cells.
    Move {
        /// The window.
        id: WindowId,
        /// Columns to the right; negative is to the left.
        dx: i32,
        /// Rows down; negative is up.
        dy: i32,
    },
    /// An edge or a corner of a window was dragged.
    Resize {
        /// The window.
        id: WindowId,
        /// The edge or corner that moves.
        grip: Grip,
        /// Columns the held side moves to the right.
        dx: i32,
        /// Rows the held side moves down.
        dy: i32,
    },
    /// The minimize mark was pressed.
    Minimize(WindowId),
    /// The maximize mark was pressed, or the title double-clicked.
    ToggleMaximize(WindowId),
    /// The close mark was pressed.
    Close(WindowId),
    /// A drag ended: the button came up after the window or one of its edges had moved.
    Dropped(WindowId),
}

/// The drag of a window that is running now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dragging {
    /// The window itself follows the pointer, and may snap to an edge when it is let go.
    Moving(WindowId),
    /// The window stays where it is and only a ghost follows the pointer; the window lands in one
    /// frame when the button comes up. This is the `ghost` drag style, the default everywhere and
    /// not only over a remote link (`drag-style`).
    Ghosting {
        /// The window.
        id: WindowId,
        /// The rectangle the ghost started at, so the landing is one movement from there.
        from: Rect,
        /// Where the ghost is now.
        rect: Rect,
    },
    /// An edge or a corner of the window follows the pointer. A resize never snaps: a window
    /// whose edge is pulled to the screen's edge is being sized, not thrown against it.
    ///
    /// The window is sized from where it began by everything the pointer has gone since, not a
    /// step at a time from where the last step left it: an edge stopped by the smallest size or by
    /// the screen stays under the pointer's way back instead of starting back as soon as the
    /// pointer turns, however far past the stop the pointer went.
    Sizing {
        /// The window.
        id: WindowId,
        /// The edge or corner held.
        grip: Grip,
        /// The rectangle the window had when the drag began.
        from: Rect,
        /// Columns and rows the pointer has gone since then.
        by: (i32, i32),
    },
}

impl Dragging {
    /// The window being dragged.
    #[must_use]
    pub fn id(self) -> WindowId {
        match self {
            Self::Moving(id) | Self::Ghosting { id, .. } | Self::Sizing { id, .. } => id,
        }
    }
}

/// What the layer over the windows shows while a drag runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preview {
    /// The rectangle the window would take if it were let go now.
    Snap(Rect),
    /// Where the window itself would be: the ghost of a drag the window does not follow.
    Ghost(Rect),
}

impl Preview {
    /// The rectangle the layer covers.
    #[must_use]
    pub fn rect(self) -> Rect {
        match self {
            Self::Snap(rect) | Self::Ghost(rect) => rect,
        }
    }

    /// How much accent it mixes into what lies under it.
    #[must_use]
    pub fn mix(self) -> f32 {
        match self {
            Self::Snap(_) => SNAP_MIX,
            Self::Ghost(_) => GHOST_MIX,
        }
    }
}

/// What the windows look like besides where they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Look {
    /// Whether the focused window casts a shadow.
    pub shadow: bool,
    /// What the layer over the windows shows, when it shows anything.
    pub preview: Option<Preview>,
}

/// The [`Action`] a framework [`WindowEvent`] on the window `id` means.
#[must_use]
pub fn action(id: WindowId, event: WindowEvent) -> Action {
    match event {
        WindowEvent::Focus => Action::Focus(id),
        WindowEvent::Move { dx, dy } => Action::Move { id, dx, dy },
        WindowEvent::Resize { edge, dx, dy } => Action::Resize { id, grip: grip_of(edge), dx, dy },
        WindowEvent::Minimize => Action::Minimize(id),
        WindowEvent::ToggleMaximize => Action::ToggleMaximize(id),
        WindowEvent::Close => Action::Close(id),
        WindowEvent::Dropped => Action::Dropped(id),
    }
}

/// The window manager's grip for the framework's edge. The two lists are the same eight sides;
/// they are apart because the window manager is a module of its own and knows no widgets.
#[must_use]
fn grip_of(edge: WindowEdge) -> Grip {
    match edge {
        WindowEdge::Left => Grip::Left,
        WindowEdge::Right => Grip::Right,
        WindowEdge::Top => Grip::Top,
        WindowEdge::Bottom => Grip::Bottom,
        WindowEdge::TopLeft => Grip::TopLeft,
        WindowEdge::TopRight => Grip::TopRight,
        WindowEdge::BottomLeft => Grip::BottomLeft,
        WindowEdge::BottomRight => Grip::BottomRight,
    }
}

/// The rectangle a ghost drag of `window` starts from inside `area`.
///
/// It is not always the rectangle the window shows: dragging a maximized or snapped window frees
/// it at the size it had before, so the ghost has to show that size from its very first frame.
/// Otherwise the window would land smaller than the shape the person was aiming with.
#[must_use]
pub fn ghost_start(window: &Window, area: Rect) -> Rect {
    clamp_into(window.placement().restore().unwrap_or_else(|| window.rect()), area)
}

/// What the layer over the windows shows for the drag `dragging`, if anything.
///
/// A window that follows the pointer asks the window manager itself; a ghost asks for the ghost's
/// rectangle, because the window has not moved yet. Either way an edge under the window wins: the
/// preview says where it would land, which is what the person is about to decide.
#[must_use]
pub fn preview(windows: &Windows, dragging: Option<Dragging>) -> Option<Preview> {
    match dragging? {
        Dragging::Moving(id) => windows.snap_target(id).map(|snap| Preview::Snap(snap.rect)),
        Dragging::Ghosting { rect, .. } => {
            Some(snap::target(rect, windows.area()).map_or(Preview::Ghost(rect), |snap| Preview::Snap(snap.rect)))
        }
        Dragging::Sizing { .. } => None,
    }
}

/// The window a key picks after `from`, going `forward` or back, or `None` when no window is on
/// the desktop.
///
/// The order is the order the windows were opened, which is the order the dock shows them in, and
/// not the stacking order: picking a window brings it forward, and an order that changed with
/// every pick would send the next key somewhere the person could not have guessed. It wraps.
#[must_use]
pub fn pick(windows: &Windows, from: Option<WindowId>, forward: bool) -> Option<WindowId> {
    let mut order: Vec<WindowId> = windows.visible().map(Window::id).collect();
    order.sort_unstable();
    let at = from.and_then(|id| order.iter().position(|kept| *kept == id));
    let Some(at) = at else {
        return if forward { order.first().copied() } else { order.last().copied() };
    };
    let count = order.len();
    let next = if forward { (at + 1) % count } else { (at + count - 1) % count };
    order.get(next).copied()
}

/// Draws every window of `windows` from the back forwards, and the layer over them.
///
/// What the title strip of each window says, in the language on screen.
///
/// The three are asked per window while the windows are drawn, because the language and the icon
/// set live in the view the drawing borrows.
pub struct Strip<'a> {
    /// The name of the window's application.
    pub name: &'a dyn Fn(&Window) -> String,
    /// The glyph of its icon.
    pub glyph: &'a dyn Fn(&Window) -> String,
    /// What stands after the name: what the program says about itself, when it says anything.
    pub subtitle: &'a dyn Fn(&Window) -> Option<String>,
}

/// `ui` must already be inside a stack, so the windows lie over the floor and under the dock.
/// `strip` says what each window's title row writes and `body` fills it. Everything the pointer
/// does arrives as `on_action`.
pub fn view<Msg: 'static>(
    windows: &Windows,
    look: &Look,
    on_action: impl Fn(Action) -> Msg + Clone + 'static,
    strip: &Strip<'_>,
    body: &mut dyn FnMut(&Window, &mut View<'_, Msg>),
    ui: &mut View<'_, Msg>,
) {
    let focus = windows.focus();
    for window in windows.visible() {
        let id = window.id();
        let focused = focus == Some(id);
        let send = on_action.clone();
        let mut surface = Surface::new((strip.name)(window))
            .icon(Glyph::literal((strip.glyph)(window)))
            .focused(focused)
            .maximized(window.is_maximized())
            // Only the focused window casts a shadow (design 3.1): it is the one that is meant to
            // look lifted, and over a remote link every shadow is cells sent again.
            .shadow(look.shadow && focused)
            .on_event(move |event| send(action(id, event)));
        if let Some(said) = (strip.subtitle)(window) {
            surface = surface.subtitle(said);
        }
        ui.place(window.rect(), |ui| {
            ui.add_with(surface, |ui| body(window, ui));
        })
        // Named, so a window keeps its memory and its running drag when it comes to the front.
        .id(id_of(id));
    }
    if let Some(preview) = look.preview {
        ui.place(preview.rect(), |ui| {
            ui.add(Ghost::new().mix(preview.mix()));
        })
        .id("window-preview");
    }
}

/// The widget name of the window `id`.
#[must_use]
pub fn id_of(id: WindowId) -> String {
    format!("window-{}", id.number())
}

/// The widget name of what the window `id` holds, so the keys can be sent to it.
#[must_use]
pub fn body_id(id: WindowId) -> String {
    format!("window-body-{}", id.number())
}

/// The size the body of a window of `rect` is drawn at: the rectangle without the strip, the
/// pillar and the edges the surface keeps.
///
/// A program needs this before its first drawing, so its first screen is already the size of the
/// window it will appear in. From then on the widget tells the pseudo-terminal the size it was
/// really drawn at, and nothing here resizes anything.
#[must_use]
pub fn body_size(rect: Rect) -> Size {
    Size::new(rect.width.saturating_sub(FRAME.0), rect.height.saturating_sub(FRAME.1))
}

/// The folder as the title strip writes it: under the home folder it starts with `~`, so a long
/// path keeps the part that says where the program is.
#[must_use]
pub fn folder_text(folder: &Path, home: Option<&Path>) -> String {
    match home.and_then(|home| folder.strip_prefix(home).ok()) {
        Some(rest) if rest.as_os_str().is_empty() => "~".to_owned(),
        Some(rest) => format!("~/{}", rest.display()),
        None => folder.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use qframe::geometry::Size;

    use super::super::tests::entry;
    use super::*;
    use crate::wm::Edge;

    /// A desktop on an 80x24 terminal: 23 rows for windows and one for the dock.
    fn desk() -> Windows {
        Windows::new(Size::new(80, 24))
    }

    #[test]
    fn every_window_event_becomes_the_action_of_its_window() {
        let id = desk().open(&entry("one"));
        assert_eq!(action(id, WindowEvent::Focus), Action::Focus(id));
        assert_eq!(action(id, WindowEvent::Move { dx: 2, dy: -1 }), Action::Move { id, dx: 2, dy: -1 });
        assert_eq!(
            action(id, WindowEvent::Resize { edge: WindowEdge::BottomLeft, dx: 1, dy: 2 }),
            Action::Resize { id, grip: Grip::BottomLeft, dx: 1, dy: 2 }
        );
        assert_eq!(action(id, WindowEvent::Minimize), Action::Minimize(id));
        assert_eq!(action(id, WindowEvent::ToggleMaximize), Action::ToggleMaximize(id));
        assert_eq!(action(id, WindowEvent::Close), Action::Close(id));
        assert_eq!(action(id, WindowEvent::Dropped), Action::Dropped(id));
    }

    #[test]
    fn every_edge_of_the_framework_names_the_same_side_here() {
        let sides = [
            (WindowEdge::Left, Grip::Left),
            (WindowEdge::Right, Grip::Right),
            (WindowEdge::Top, Grip::Top),
            (WindowEdge::Bottom, Grip::Bottom),
            (WindowEdge::TopLeft, Grip::TopLeft),
            (WindowEdge::TopRight, Grip::TopRight),
            (WindowEdge::BottomLeft, Grip::BottomLeft),
            (WindowEdge::BottomRight, Grip::BottomRight),
        ];
        for (edge, grip) in sides {
            let mapped = grip_of(edge);
            assert_eq!(mapped.holds_left(), edge.left(), "{edge:?}");
            assert_eq!(mapped.holds_right(), edge.right(), "{edge:?}");
            assert_eq!(mapped.holds_top(), edge.top(), "{edge:?}");
            assert_eq!(mapped.holds_bottom(), edge.bottom(), "{edge:?}");
            assert_eq!(mapped, grip, "{edge:?}");
        }
    }

    #[test]
    fn a_ghost_starts_at_the_rectangle_the_window_shows() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.move_to(id, 5, 4);
        let window = desk.get(id).expect("open");
        assert_eq!(ghost_start(window, desk.area()), Rect::new(5, 4, 52, 14));
    }

    #[test]
    fn a_ghost_of_a_maximized_window_starts_at_the_size_it_will_land_with() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.move_to(id, 5, 4);
        desk.maximize(id);
        let window = desk.get(id).expect("open");
        assert_eq!(window.rect(), desk.area(), "it fills the desktop now");
        assert_eq!(ghost_start(window, desk.area()), Rect::new(5, 4, 52, 14), "the ghost shows the size it frees at");
    }

    #[test]
    fn nothing_is_previewed_without_a_drag_or_while_an_edge_is_pulled() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.move_by(id, -100, 5);
        assert_eq!(preview(&desk, None), None);
        let sizing = Dragging::Sizing { id, grip: Grip::Left, from: desk.area(), by: (0, 0) };
        assert_eq!(preview(&desk, Some(sizing)), None, "a resize never snaps");
    }

    #[test]
    fn a_dragged_window_against_an_edge_previews_the_half_it_would_take() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        assert_eq!(preview(&desk, Some(Dragging::Moving(id))), None, "in the middle there is nothing to show");
        desk.move_by(id, -100, 5);
        assert_eq!(preview(&desk, Some(Dragging::Moving(id))), Some(Preview::Snap(Rect::new(0, 0, 40, 23))));
    }

    #[test]
    fn a_ghost_shows_itself_until_it_rests_against_an_edge() {
        let desk = desk();
        let middle = Rect::new(20, 6, 30, 8);
        let ghosting = |rect| Dragging::Ghosting { id: WindowId::first(), from: middle, rect };
        assert_eq!(preview(&desk, Some(ghosting(middle))), Some(Preview::Ghost(middle)));
        let edge = Rect::new(0, 6, 30, 8);
        assert_eq!(preview(&desk, Some(ghosting(edge))), Some(Preview::Snap(Rect::new(0, 0, 40, 23))));
    }

    #[test]
    fn a_snap_preview_is_lighter_than_a_ghost() {
        let rect = Rect::new(0, 0, 10, 4);
        assert!(Preview::Snap(rect).mix() < Preview::Ghost(rect).mix());
        assert_eq!(Preview::Snap(rect).rect(), rect);
        assert_eq!(Preview::Ghost(rect).rect(), rect);
    }

    #[test]
    fn the_keys_pick_windows_in_the_order_they_were_opened_and_wrap() {
        let mut desk = desk();
        let first = desk.open(&entry("one"));
        let second = desk.open(&entry("two"));
        let third = desk.open(&entry("three"));
        // Bringing the first window forward does not change what the next key picks.
        desk.raise(first);
        assert_eq!(pick(&desk, Some(first), true), Some(second));
        assert_eq!(pick(&desk, Some(second), true), Some(third));
        assert_eq!(pick(&desk, Some(third), true), Some(first));
        assert_eq!(pick(&desk, Some(first), false), Some(third));
        assert_eq!(pick(&desk, Some(second), false), Some(first));
    }

    #[test]
    fn the_keys_leave_out_minimized_windows_and_an_empty_desktop_picks_nothing() {
        let mut desk = desk();
        assert_eq!(pick(&desk, None, true), None);
        let first = desk.open(&entry("one"));
        let second = desk.open(&entry("two"));
        desk.minimize(second);
        assert_eq!(pick(&desk, Some(first), true), Some(first), "the only window left is picked again");
        assert_eq!(pick(&desk, Some(second), true), Some(first), "from a minimized window it starts over");
        assert_eq!(pick(&desk, None, false), Some(first));
    }

    #[test]
    fn the_body_of_a_window_is_its_rectangle_without_the_strip_and_the_edges() {
        // The usual first window of an 80x24 terminal: 52 by 14 cells.
        assert_eq!(body_size(Rect::new(14, 4, 52, 14)), Size::new(49, 12));
        // The smallest window there can be, and a rectangle smaller than the frame itself.
        assert_eq!(body_size(Rect::new(0, 0, 20, 5)), Size::new(17, 3));
        assert_eq!(body_size(Rect::new(0, 0, 1, 1)), Size::new(0, 0));
    }

    #[test]
    fn the_strip_writes_a_folder_under_the_home_folder_with_a_tilde() {
        let home = Path::new("/home/kisi");
        assert_eq!(folder_text(Path::new("/home/kisi/isler"), Some(home)), "~/isler");
        assert_eq!(folder_text(Path::new("/home/kisi"), Some(home)), "~");
        assert_eq!(folder_text(Path::new("/etc"), Some(home)), "/etc");
        assert_eq!(folder_text(Path::new("/home/kisi/isler"), None), "/home/kisi/isler");
    }

    #[test]
    fn a_window_and_what_it_holds_have_names_of_their_own() {
        let id = WindowId::first();
        assert_ne!(id_of(id), body_id(id));
        assert!(body_id(id).ends_with(&id.number().to_string()));
    }

    #[test]
    fn a_dragging_names_its_window() {
        let id = WindowId::first();
        let rect = Rect::new(0, 0, 20, 5);
        assert_eq!(Dragging::Moving(id).id(), id);
        assert_eq!(Dragging::Sizing { id, grip: Grip::Right, from: rect, by: (1, 0) }.id(), id);
        assert_eq!(Dragging::Ghosting { id, from: rect, rect }.id(), id);
    }

    #[test]
    fn a_window_that_is_already_snapped_is_not_previewed_again() {
        let mut desk = desk();
        let id = desk.open(&entry("one"));
        desk.snap(id, Edge::Left);
        assert_eq!(desk.get(id).and_then(Window::snapped_to), Some(Edge::Left));
        assert_eq!(preview(&desk, Some(Dragging::Moving(id))), None);
    }
}
