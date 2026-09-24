//! Windows on the desktop: opening one, moving and sizing it with the mouse and with the keys,
//! snapping it to an edge, the marks, the dock's window items and desktop mode.
//!
//! Every window action is done twice where it can be: once with the pointer and once from the
//! keyboard, and the two have to end at the same desktop.

mod support;

use qdesk::app::{Ask, Desk, Keys, Msg};
use qdesk::settings::{self, DragStyle};
use qdesk::wm::{Window, WindowId};
use qframe::color::ColorDepth;
use qframe::event::{Event, MouseButton, MouseEvent, MouseKind};
use qframe::geometry::Rect;
use qframe::icons::GlyphMode;
use qframe::keymap::Modifiers;
use qframe::prelude::*;

use support::{decoration, desk, desk_over_ssh};

/// The rectangle the first window opens at on an 80x24 terminal: two thirds of the desktop, in
/// the middle of it.
const FIRST: Rect = Rect { x: 14, y: 4, width: 52, height: 14 };

/// Opens the Settings window by opening its icon on the floor, as a person does.
fn with_settings(width: u16, height: u16) -> Harness<Desk> {
    let mut harness = desk(width, height);
    // The second icon of the floor is Settings; two clicks open it.
    harness.click(3, 4);
    harness.click(3, 4);
    assert_eq!(harness.app().windows().len(), 1, "the window opened:\n{}", harness.screen());
    harness
}

/// The window in front.
fn front(harness: &Harness<Desk>) -> &Window {
    harness.app().windows().front().expect("a window on the desktop")
}

fn rect(harness: &Harness<Desk>) -> Rect {
    front(harness).rect()
}

fn id(harness: &Harness<Desk>) -> WindowId {
    front(harness).id()
}

/// Clicks the window item `label` on the dock's row, wherever the row put it.
fn click_item(harness: &mut Harness<Desk>, label: &str) {
    let rows: Vec<String> = harness.screen().lines().map(str::to_owned).collect();
    let row = rows.len() - 1;
    let dock = &rows[row];
    let at = dock.find(label).unwrap_or_else(|| panic!("no {label} on the dock: {dock}"));
    let x = i32::from(qframe::text::width(&dock[..at]));
    let y = i32::try_from(row).expect("a row on screen");
    harness.click(x + 1, y);
}

/// An alt drag, which the harness has no helper for.
fn alt_drag(harness: &mut Harness<Desk>, button: MouseButton, at: (i32, i32), by: (i32, i32)) {
    let mods = Modifiers { alt: true, ..Modifiers::default() };
    let event = |kind, (x, y)| Event::Mouse(MouseEvent { kind, x, y, mods });
    let to = (at.0 + by.0, at.1 + by.1);
    harness.events(&[
        event(MouseKind::Down(button), at),
        event(MouseKind::Drag(button), to),
        event(MouseKind::Up(button), to),
    ]);
}

/// Drags from `(x, y)` to `(x + dx, y + dy)` and lets go.
fn drag(harness: &mut Harness<Desk>, at: (i32, i32), by: (i32, i32)) {
    let to = (at.0 + by.0, at.1 + by.1);
    harness.mouse(MouseKind::Down(MouseButton::Left), at.0, at.1);
    harness.mouse(MouseKind::Drag(MouseButton::Left), to.0, to.1);
    harness.mouse(MouseKind::Up(MouseButton::Left), to.0, to.1);
}

#[test]
fn a_settings_window_opens_over_the_floor_with_its_name_and_its_marks() {
    let harness = with_settings(80, 24);
    assert_eq!(rect(&harness), FIRST);
    let rows: Vec<String> = harness.screen().lines().map(str::to_owned).collect();
    let title = &rows[usize::try_from(FIRST.y).expect("a row on screen")];
    assert!(title.contains("Settings"), "{title}");
    assert!(title.contains('×'), "the close mark is at the right end: {title}");
    assert!(harness.screen().contains("Appearance"), "the screen itself is in the window:\n{}", harness.screen());
    assert_eq!(decoration(&harness.screen()), None, "no lines, no boxes:\n{}", harness.screen());
}

#[test]
fn the_window_carries_the_accent_pillar_down_its_left_edge_while_it_has_the_focus() {
    let harness = with_settings(80, 24);
    let left = u16::try_from(FIRST.x).expect("a column on screen");
    let row = u16::try_from(FIRST.y + 2).expect("a row on screen");
    assert_eq!(harness.buffer()[(left, row)].symbol(), "▌", "the focused window shows the pillar");
}

#[test]
fn the_title_drags_the_window_and_the_bottom_right_corner_sizes_it() {
    let mut harness = with_settings(80, 24);
    drag(&mut harness, (FIRST.x + 4, FIRST.y), (3, 2));
    assert_eq!(rect(&harness), Rect::new(17, 6, 52, 14));
    // The last column and the last row of the window are its handles.
    let corner = (rect(&harness).right() - 1, rect(&harness).bottom() - 1);
    drag(&mut harness, corner, (-6, -3));
    assert_eq!(rect(&harness), Rect::new(17, 6, 46, 11));
}

#[test]
fn alt_drags_move_and_size_the_window_from_anywhere_in_its_body() {
    let mut harness = with_settings(80, 24);
    let body = (FIRST.x + 10, FIRST.y + 5);
    alt_drag(&mut harness, MouseButton::Left, body, (-4, -1));
    assert_eq!(rect(&harness), Rect::new(10, 3, 52, 14), "alt with the left button moves it");
    // Alt with the right button sizes it from the edge nearest the press.
    let inside = (rect(&harness).right() - 4, rect(&harness).y + 7);
    alt_drag(&mut harness, MouseButton::Right, inside, (-5, 0));
    assert_eq!(rect(&harness).width, 47, "the right edge came in");
}

#[test]
fn alt_and_the_right_button_size_the_window_from_whichever_edge_or_corner_is_nearest() {
    // The window read as nine zones: a press in each outer third holds that edge, and the two
    // axes together name a corner. Each drag goes three columns left and two rows up.
    let (left, top) = (FIRST.x + 2, FIRST.y + 2);
    let (right, bottom) = (FIRST.right() - 3, FIRST.bottom() - 3);
    let (middle_x, middle_y) = (FIRST.x + i32::from(FIRST.width) / 2, FIRST.y + i32::from(FIRST.height) / 2 - 1);
    let (x, y, w, h) = (FIRST.x, FIRST.y, FIRST.width, FIRST.height);
    let cases = [
        ("left", (left, middle_y), Rect::new(x - 3, y, w + 3, h)),
        ("right", (right, middle_y), Rect::new(x, y, w - 3, h)),
        ("top", (middle_x, top), Rect::new(x, y - 2, w, h + 2)),
        ("bottom", (middle_x, bottom), Rect::new(x, y, w, h - 2)),
        ("top left", (left, top), Rect::new(x - 3, y - 2, w + 3, h + 2)),
        ("top right", (right, top), Rect::new(x, y - 2, w - 3, h + 2)),
        ("bottom left", (left, bottom), Rect::new(x - 3, y, w + 3, h - 2)),
        ("bottom right", (right, bottom), Rect::new(x, y, w - 3, h - 2)),
    ];
    for (zone, at, expected) in cases {
        let mut harness = with_settings(80, 24);
        assert_eq!(rect(&harness), FIRST);
        alt_drag(&mut harness, MouseButton::Right, at, (-3, -2));
        assert_eq!(rect(&harness), expected, "a press in the {zone} zone:\n{}", harness.screen());
    }
}

/// Presses at `at`, drags through every point of `path` and lets go at the last one.
fn drag_through(harness: &mut Harness<Desk>, at: (i32, i32), path: &[(i32, i32)]) {
    harness.mouse(MouseKind::Down(MouseButton::Left), at.0, at.1);
    for &(x, y) in path {
        harness.mouse(MouseKind::Drag(MouseButton::Left), x, y);
    }
    let (x, y) = path.last().copied().unwrap_or(at);
    harness.mouse(MouseKind::Up(MouseButton::Left), x, y);
}

/// The seven handles a plain drag holds on the first window, and the rectangle a drag of three
/// columns left and two rows up leaves it at. The left edge is the pillar's column; the top side
/// is the title strip, so only its two end cells size the window and the cells between move it.
fn handles() -> [(&'static str, (i32, i32), Rect); 7] {
    let (x, y, w, h) = (FIRST.x, FIRST.y, FIRST.width, FIRST.height);
    let (right, bottom) = (FIRST.right() - 1, FIRST.bottom() - 1);
    let (middle_x, middle_y) = (x + i32::from(w) / 2, y + i32::from(h) / 2);
    [
        ("left edge", (x, middle_y), Rect::new(x - 3, y, w + 3, h)),
        ("right edge", (right, middle_y), Rect::new(x, y, w - 3, h)),
        ("bottom edge", (middle_x, bottom), Rect::new(x, y, w, h - 2)),
        ("top left corner", (x, y), Rect::new(x - 3, y - 2, w + 3, h + 2)),
        ("top right corner", (right, y), Rect::new(x, y - 2, w - 3, h + 2)),
        ("bottom left corner", (x, bottom), Rect::new(x - 3, y, w + 3, h - 2)),
        ("bottom right corner", (right, bottom), Rect::new(x, y, w - 3, h - 2)),
    ]
}

/// A folder of a test's own, removed when dropped.
struct Folder(std::path::PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The Settings window opened as [`with_settings`] opens it, on a desktop whose settings file
/// chose `style` for dragging a window; `Ghost` is the untouched default and writes no file. The
/// file lives in the folder handed back, named by `name`, for as long as the test keeps it: the
/// desktop follows its settings file, and a file taken away would put the default back.
fn with_settings_dragging(style: DragStyle, name: &str) -> (Harness<Desk>, Option<Folder>) {
    if style == DragStyle::Ghost {
        return (with_settings(80, 24), None);
    }
    let folder = Folder(std::env::temp_dir().join(format!("qdesk-test-drag-{name}-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&folder.0);
    std::fs::create_dir_all(&folder.0).expect("a folder of the test's own");
    std::fs::write(folder.0.join("desktop.conf"), "drag-style = \"live\"\n").expect("a settings file");
    let mut harness = support::desk_in(&folder.0, 80, 24);
    assert_eq!(harness.app().prefs().drag, style, "the file chose it");
    harness.click(3, 4);
    harness.click(3, 4);
    assert_eq!(harness.app().windows().len(), 1, "the window opened:\n{}", harness.screen());
    (harness, Some(folder))
}

#[test]
fn a_plain_drag_on_every_edge_and_corner_sizes_the_window_in_either_drag_style() {
    for style in [DragStyle::Ghost, DragStyle::Live] {
        for (handle, at, expected) in handles() {
            let (mut harness, _folder) = with_settings_dragging(style, "edges");
            drag(&mut harness, at, (-3, -2));
            assert_eq!(rect(&harness), expected, "a drag on the {handle}, {style:?}:\n{}", harness.screen());
            assert_eq!(harness.app().prefs().drag, style, "still dragging {style:?}");
        }
        // The top side has no row of its own: between its corners the title moves the window.
        let (mut harness, _folder) = with_settings_dragging(style, "title");
        drag(&mut harness, (FIRST.x + 1, FIRST.y), (-3, -2));
        assert_eq!(rect(&harness), Rect::new(FIRST.x - 3, FIRST.y - 2, FIRST.width, FIRST.height), "{style:?}");
    }
}

#[test]
fn a_held_edge_stops_at_the_smallest_size_and_waits_there_for_the_pointer_to_come_back() {
    let mut harness = with_settings(80, 24);
    let middle = FIRST.y + 5;
    // The left edge pulled far past the right one leaves the smallest window; coming back by five
    // columns still leaves the pointer far right of the edge, which has to stay where it stopped.
    drag_through(&mut harness, (FIRST.x, middle), &[(FIRST.x + 60, middle), (FIRST.x + 55, middle)]);
    assert_eq!(rect(&harness), Rect::new(FIRST.right() - 20, FIRST.y, 20, FIRST.height), "{}", harness.screen());
    // The top left corner pulled down and right past the smallest size, then back up a little.
    let mut harness = with_settings(80, 24);
    drag_through(&mut harness, (FIRST.x, FIRST.y), &[(FIRST.x + 50, FIRST.y + 20), (FIRST.x + 48, FIRST.y + 18)]);
    assert_eq!(rect(&harness), Rect::new(FIRST.right() - 20, FIRST.bottom() - 5, 20, 5), "{}", harness.screen());
    // And back all the way: once the pointer is over where the edge started, the window is whole.
    let mut harness = with_settings(80, 24);
    drag_through(&mut harness, (FIRST.x, middle), &[(FIRST.x + 60, middle), (FIRST.x, middle)]);
    assert_eq!(rect(&harness), FIRST, "{}", harness.screen());
}

#[test]
fn a_held_edge_stops_at_the_screen_and_at_the_dock_s_row() {
    let mut harness = with_settings(80, 24);
    // The top left corner goes as far as the screen's corner and keeps the other corner.
    drag(&mut harness, (FIRST.x, FIRST.y), (-FIRST.x, -FIRST.y));
    assert_eq!(rect(&harness), Rect::new(0, 0, 66, 18), "{}", harness.screen());
    // The bottom edge goes down to the last row above the dock and no further. The pointer can
    // go on onto the dock's row; coming back from there to the row the edge stopped at leaves it
    // where it is, since that is the row the edge's handle is on.
    let mut harness = with_settings(80, 24);
    let bottom = FIRST.bottom() - 1;
    drag_through(&mut harness, (FIRST.x + 10, bottom), &[(FIRST.x + 10, 23), (FIRST.x + 10, 22)]);
    assert_eq!(rect(&harness).bottom(), 23, "the dock's row stays the dock's:\n{}", harness.screen());
    assert!(harness.screen().lines().last().is_some_and(|dock| dock.contains("sunucu-1")), "{}", harness.screen());
}

#[test]
fn a_filled_or_snapped_window_is_sized_from_its_edges_and_floats_at_that_size() {
    let mut harness = with_settings(80, 24);
    let title = (FIRST.x + 4, FIRST.y);
    harness.click(title.0, title.1);
    harness.click(title.0, title.1);
    assert_eq!(rect(&harness), Rect::new(0, 0, 80, 23), "the window fills the desktop");
    drag(&mut harness, (0, 10), (10, 0));
    assert_eq!(rect(&harness), Rect::new(10, 0, 70, 23), "its left edge came in:\n{}", harness.screen());
    assert!(!front(&harness).is_maximized(), "and it floats at that size");

    let mut harness = with_settings(80, 24);
    drag(&mut harness, title, (-40, 0));
    assert_eq!(rect(&harness), Rect::new(0, 0, 40, 23), "it took the left half");
    drag(&mut harness, (0, 0), (3, 2));
    assert_eq!(rect(&harness), Rect::new(3, 2, 37, 21), "its top left corner came in:\n{}", harness.screen());
    drag(&mut harness, (3 + 36, 10), (5, 0));
    assert_eq!(rect(&harness), Rect::new(3, 2, 42, 21), "and the right edge went out from there");
}

#[test]
fn the_pointer_asks_for_a_resize_arrow_over_every_edge_and_corner() {
    use qframe::widget::PointerShape;
    let mut harness = with_settings(80, 24);
    let (right, bottom) = (FIRST.right() - 1, FIRST.bottom() - 1);
    let middle = FIRST.y + 5;
    let cases = [
        ((FIRST.x, middle), PointerShape::EwResize),
        ((right, middle), PointerShape::EwResize),
        ((FIRST.x + 10, bottom), PointerShape::NsResize),
        ((FIRST.x, FIRST.y), PointerShape::NwseResize),
        ((right, bottom), PointerShape::NwseResize),
        ((right, FIRST.y), PointerShape::NeswResize),
        ((FIRST.x, bottom), PointerShape::NeswResize),
        ((FIRST.x + 10, FIRST.y), PointerShape::Default),
        ((FIRST.x + 10, middle), PointerShape::Default),
    ];
    for ((x, y), shape) in cases {
        harness.hover(x, y);
        assert_eq!(harness.pointer_shape(), shape, "over ({x}, {y})");
    }
}

#[test]
fn a_window_dragged_against_the_left_edge_shows_where_it_would_land_and_takes_that_half() {
    let mut harness = with_settings(80, 24);
    let title = (FIRST.x + 4, FIRST.y);
    harness.mouse(MouseKind::Down(MouseButton::Left), title.0, title.1);
    harness.mouse(MouseKind::Drag(MouseButton::Left), title.0 - 40, title.1);
    // The preview lies over the half the window would take, as a tone and not a line.
    let accent = harness.env().theme().color("accent").expect("an accent");
    let under = harness.bg(70, 1).expect("the bare floor");
    assert_eq!(harness.bg(2, 1), Some(under.mix(accent, 0.20)), "the left half is previewed");
    assert_ne!(harness.bg(70, 1), Some(under.mix(accent, 0.20)), "the right half is not");
    harness.mouse(MouseKind::Up(MouseButton::Left), title.0 - 40, title.1);
    assert_eq!(rect(&harness), Rect::new(0, 0, 40, 23), "it takes the left half");
}

#[test]
fn the_marks_maximize_minimize_and_close_the_window() {
    let mut harness = with_settings(80, 24);
    let marks = |harness: &Harness<Desk>| {
        let rect = rect(harness);
        (rect.right() - 9, rect.y)
    };
    let (x, y) = marks(&harness);
    harness.click(x + 4, y);
    assert_eq!(rect(&harness), Rect::new(0, 0, 80, 23), "the middle mark fills the desktop");
    let (x, y) = marks(&harness);
    harness.click(x + 4, y);
    assert_eq!(rect(&harness), FIRST, "and gives the rectangle back");
    let (x, y) = marks(&harness);
    harness.click(x + 1, y);
    assert!(harness.app().windows().front().is_none(), "the first mark takes it to the dock");
    assert_eq!(harness.app().windows().len(), 1, "a minimized window is still open");
    click_item(&mut harness, "▤ Settings");
    assert!(harness.app().windows().front().is_some(), "the dock item brings it back");
    let (x, y) = marks(&harness);
    harness.click(x + 7, y);
    assert_eq!(harness.app().windows().len(), 0, "the last mark closes it");
}

#[test]
fn a_double_click_on_the_title_fills_the_desktop_and_gives_the_rectangle_back() {
    let mut harness = with_settings(80, 24);
    let title = (FIRST.x + 4, FIRST.y);
    harness.click(title.0, title.1);
    harness.click(title.0, title.1);
    assert_eq!(rect(&harness), Rect::new(0, 0, 80, 23));
}

#[test]
fn the_dock_shows_the_open_window_raised_and_a_minimized_one_with_its_mark() {
    let mut harness = with_settings(80, 24);
    let dock = harness.screen().lines().last().expect("the dock row").to_owned();
    assert!(dock.contains("▤ Settings"), "{dock}");
    assert!(dock.contains("sunucu-1"), "the machine name keeps its place: {dock}");
    let window = id(&harness);
    harness.send(Msg::Ask(Ask::Minimize, window));
    let dock = harness.screen().lines().last().expect("the dock row").to_owned();
    assert!(dock.contains("▤ Settings −"), "a minimized window carries the mark: {dock}");
}

#[test]
fn a_press_on_the_focused_window_item_takes_it_to_the_dock_and_another_brings_it_back() {
    let mut harness = with_settings(80, 24);
    click_item(&mut harness, "▤ Settings");
    assert!(front_of(&harness).is_none(), "a press on the focused item minimizes it");
    click_item(&mut harness, "▤ Settings");
    assert!(front_of(&harness).is_some(), "and the next one brings it back");
}

/// The window in front, when one is drawn.
fn front_of(harness: &Harness<Desk>) -> Option<&Window> {
    harness.app().windows().front()
}

#[test]
fn desktop_mode_puts_the_keys_on_the_desktop_and_says_so_in_the_dock_s_row() {
    let mut harness = with_settings(80, 24);
    assert_eq!(harness.app().keys(), None);
    harness.press("ctrl+alt+space");
    assert_eq!(harness.app().keys(), Some(Keys::Pick));
    let hints = harness.screen().lines().last().expect("the dock row").to_owned();
    // A row of 80 columns holds the hints a person cannot guess; the rest is in the help layer.
    for said in ["window", "move", "size", "close", "esc"] {
        assert!(hints.contains(said), "{said} is missing from {hints}");
    }
    assert_eq!(decoration(&harness.screen()), None, "the hints are not bracketed:\n{}", harness.screen());
    harness.press("esc");
    assert_eq!(harness.app().keys(), None, "esc gives the keys back to the window");
    assert!(harness.screen().lines().last().expect("the dock row").contains("sunucu-1"), "the dock is back");
}

#[test]
fn the_keys_move_and_size_the_window_a_cell_at_a_time_and_five_with_shift() {
    let mut harness = with_settings(80, 24);
    harness.press("ctrl+alt+space").press("m");
    assert_eq!(harness.app().keys(), Some(Keys::Move));
    harness.press("right").press("down");
    assert_eq!(rect(&harness), Rect::new(FIRST.x + 1, FIRST.y + 1, 52, 14));
    harness.press("shift+left");
    assert_eq!(rect(&harness), Rect::new(FIRST.x - 4, FIRST.y + 1, 52, 14), "shift moves five at a time");
    harness.press("enter");
    assert_eq!(harness.app().keys(), Some(Keys::Pick), "enter lets go");
    harness.press("r").press("left");
    assert_eq!(rect(&harness).width, 51, "the right edge comes in");
    harness.press("shift+down");
    // Five rows down would reach past the floor, so the edge stops at the last row of it.
    assert_eq!(rect(&harness).bottom(), 23, "and the bottom edge goes down to the dock");
}

#[test]
fn the_mouse_and_the_keys_move_a_window_to_the_same_place() {
    let mut mouse = with_settings(80, 24);
    drag(&mut mouse, (FIRST.x + 4, FIRST.y), (3, 2));
    let mut keys = with_settings(80, 24);
    keys.press("ctrl+alt+space").press("m");
    keys.press("right").press("right").press("right").press("down").press("down");
    assert_eq!(rect(&mouse), rect(&keys));
}

#[test]
fn the_keys_fill_the_desktop_take_a_window_to_the_dock_and_close_it() {
    let mut harness = with_settings(80, 24);
    harness.press("ctrl+alt+space").press("z");
    assert_eq!(rect(&harness), Rect::new(0, 0, 80, 23), "z fills the desktop");
    harness.press("z");
    assert_eq!(rect(&harness), FIRST, "and gives the rectangle back");
    harness.press("n");
    assert!(front_of(&harness).is_none(), "n takes it to the dock");
    assert_eq!(harness.app().windows().len(), 1);
    // The dock's row is the hint line while the desktop has the keys; the item is there again as
    // soon as the keys go back.
    harness.press("ctrl+alt+space");
    click_item(&mut harness, "▤ Settings −");
    harness.press("ctrl+alt+space").press("x");
    assert_eq!(harness.app().windows().len(), 0, "x closes it");
}

#[test]
fn the_letters_of_desktop_mode_do_nothing_while_a_window_has_the_keys() {
    let mut harness = with_settings(80, 24);
    harness.press("x").press("n").press("z");
    assert_eq!(harness.app().windows().len(), 1, "the keys are the window's:\n{}", harness.screen());
    assert_eq!(rect(&harness), FIRST);
}

#[test]
fn tiling_lays_every_window_out_at_once_from_the_keys_and_from_the_menu() {
    let mut harness = with_settings(80, 24);
    // A second window: the launcher opens the Terminal entry, which has no window yet, so the
    // desktop is tiled with the one window it has.
    harness.press("ctrl+alt+space").press("t");
    assert_eq!(rect(&harness), Rect::new(0, 0, 80, 23), "one window takes the whole desktop");
    harness.send(Msg::Tile);
    assert_eq!(rect(&harness), Rect::new(0, 0, 80, 23));
}

#[test]
fn nobody_has_touched_the_setting_and_a_ghost_carries_the_drag() {
    // The default is `ghost`, untouched — the point being tested is that the default itself
    // drags a ghost, not merely that `Prefs::default().drag` reads as `Ghost`.
    let mut harness = with_settings(80, 24);
    let title = (FIRST.x + 4, FIRST.y);
    harness.mouse(MouseKind::Down(MouseButton::Left), title.0, title.1);
    harness.mouse(MouseKind::Drag(MouseButton::Left), title.0 + 6, title.1 + 2);
    assert_eq!(rect(&harness), FIRST, "the window itself has not moved");
    // The ghost lies over the floor as a tone: a cell it covers that the window does not.
    let accent = harness.env().theme().color("accent").expect("an accent");
    let floor = harness.bg(78, 22).expect("the bare floor");
    assert_eq!(harness.bg(68, 19), Some(floor.mix(accent, 0.25)), "the ghost's tone is on the floor");
    assert_ne!(harness.bg(78, 22), Some(floor.mix(accent, 0.25)));
    harness.mouse(MouseKind::Up(MouseButton::Left), title.0 + 6, title.1 + 2);
    assert_eq!(rect(&harness), Rect::new(FIRST.x + 6, FIRST.y + 2, 52, 14), "it lands where the ghost stood");
}

#[test]
fn live_is_chosen_in_settings_and_the_window_itself_follows_the_pointer() {
    let mut harness = with_settings(80, 24);
    harness.send(Msg::Settings(settings::Msg::Drag(DragStyle::Live)));
    assert_eq!(harness.app().prefs().drag, DragStyle::Live);
    let title = (FIRST.x + 4, FIRST.y);
    harness.mouse(MouseKind::Down(MouseButton::Left), title.0, title.1);
    harness.mouse(MouseKind::Drag(MouseButton::Left), title.0 + 6, title.1 + 2);
    assert_eq!(rect(&harness), Rect::new(FIRST.x + 6, FIRST.y + 2, 52, 14), "the window itself follows the pointer");
}

#[test]
fn the_untouched_default_drags_a_ghost_here_and_over_ssh_alike() {
    // Only the frame cap still follows the connection; the drag style does not, so the same
    // untouched default drags a ghost whichever way the terminal is reached.
    let mut here = with_settings(80, 24);
    assert_eq!(here.app().prefs().drag, DragStyle::Ghost);
    let title = (FIRST.x + 4, FIRST.y);
    here.mouse(MouseKind::Down(MouseButton::Left), title.0, title.1);
    here.mouse(MouseKind::Drag(MouseButton::Left), title.0 + 6, title.1 + 2);
    assert_eq!(rect(&here), FIRST, "the window stays where it is here too");
    here.mouse(MouseKind::Up(MouseButton::Left), title.0 + 6, title.1 + 2);
    assert_eq!(rect(&here), Rect::new(FIRST.x + 6, FIRST.y + 2, 52, 14), "it lands where the ghost stood");

    let mut over_ssh = desk_over_ssh(80, 24);
    over_ssh.click(3, 4);
    over_ssh.click(3, 4);
    assert_eq!(over_ssh.app().prefs().drag, DragStyle::Ghost, "the same, untouched default over the other connection");
    over_ssh.mouse(MouseKind::Down(MouseButton::Left), title.0, title.1);
    over_ssh.mouse(MouseKind::Drag(MouseButton::Left), title.0 + 6, title.1 + 2);
    assert_eq!(rect(&over_ssh), FIRST, "the window stays where it is over SSH");
    over_ssh.mouse(MouseKind::Up(MouseButton::Left), title.0 + 6, title.1 + 2);
    assert_eq!(rect(&over_ssh), Rect::new(FIRST.x + 6, FIRST.y + 2, 52, 14), "it lands where the ghost stood");
}

#[test]
fn a_smaller_terminal_pushes_the_window_in_and_a_larger_one_leaves_it_alone() {
    let mut harness = with_settings(80, 24);
    harness.resize(60, 16);
    let inside = rect(&harness);
    assert!(inside.right() <= 60 && inside.bottom() <= 15, "{inside:?} is inside the smaller desktop");
    harness.resize(120, 40);
    assert_eq!(rect(&harness), inside, "a bigger terminal leaves it where it was");
}

#[test]
fn the_hint_line_speaks_turkish_when_the_language_does() {
    let mut harness = with_settings(80, 24);
    harness.set_locale("tr");
    harness.press("ctrl+alt+space");
    let hints = harness.screen().lines().last().expect("the dock row").to_owned();
    for said in ["pencere", "taşı", "boyut", "kapat", "geri"] {
        assert!(hints.contains(said), "{said} is missing from {hints}");
    }
}

#[test]
fn a_window_keeps_its_place_in_every_glyph_mode_and_colour_depth() {
    for mode in [GlyphMode::Nerd, GlyphMode::Unicode, GlyphMode::Ascii] {
        for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256, ColorDepth::Ansi16] {
            let mut harness = with_settings(80, 24);
            harness.set_glyph_mode(mode).set_depth(depth);
            assert_eq!(rect(&harness), FIRST, "{mode:?} {depth:?}");
            let screen = harness.screen();
            assert_eq!(decoration(&screen), None, "{mode:?} {depth:?}:\n{screen}");
            assert!(screen.contains("Settings"), "{mode:?} {depth:?}:\n{screen}");
        }
    }
}

/// Right-clicks the window item `label` on the dock's row and picks `row` from its menu.
fn from_the_window_menu(harness: &mut Harness<Desk>, label: &str, row: &str) {
    let rows: Vec<String> = harness.screen().lines().map(str::to_owned).collect();
    let y = rows.len() - 1;
    let at = rows[y].find(label).unwrap_or_else(|| panic!("no {label} on the dock: {}", rows[y]));
    let x = i32::from(qframe::text::width(&rows[y][..at])) + 1;
    let y = i32::try_from(y).expect("a row on screen");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    harness.mouse(MouseKind::Up(MouseButton::Right), x, y);
    assert!(harness.screen().contains(row), "the menu has {row}:\n{}", harness.screen());
    harness.click_text(row);
}

#[test]
fn the_edges_that_size_a_window_light_up_under_the_pointer() {
    let mut harness = with_settings(80, 24);
    let window = rect(&harness);
    let (right, middle) = (window.right() - 1, window.y + i32::from(window.height) / 2);
    let (x, y) = (u16::try_from(right).expect("on screen"), u16::try_from(middle).expect("on screen"));
    harness.hover(70, 22);
    let resting = harness.bg(x, y);
    harness.hover(right, middle);
    assert_ne!(harness.bg(x, y), resting, "the right edge lights under the pointer:\n{}", harness.screen());
    let bottom = u16::try_from(window.bottom() - 1).expect("on screen");
    harness.hover(window.x + 10, window.bottom() - 1);
    assert_ne!(harness.bg(u16::try_from(window.x + 10).expect("on screen"), bottom), resting, "and so does the bottom");
    // Where the pointer does not size anything, nothing lights.
    harness.hover(window.x + 10, middle);
    assert_eq!(harness.bg(x, y), resting);
}

#[test]
fn resize_on_the_window_menu_lets_the_arrows_size_it_and_the_dock_says_the_mouse_does_too() {
    let mut harness = with_settings(80, 24);
    from_the_window_menu(&mut harness, "Settings", "Resize");
    assert_eq!(harness.app().keys(), Some(Keys::Resize), "the sizing step is open");
    let hints = harness.screen().lines().last().expect("the dock row").to_owned();
    for said in ["size", "enter", "esc", "any edge", "drag it"] {
        assert!(hints.contains(said), "{said} is missing from {hints}");
    }
    assert_eq!(decoration(&harness.screen()), None, "{}", harness.screen());
    harness.press("right").press("down");
    assert_eq!(rect(&harness), Rect::new(FIRST.x, FIRST.y, FIRST.width + 1, FIRST.height + 1));
    harness.press("enter");
    assert_eq!(harness.app().keys(), Some(Keys::Pick), "enter lets go, as in desktop mode");
}

#[test]
fn resize_on_the_menu_of_a_window_on_the_dock_brings_it_back_first() {
    let mut harness = with_settings(80, 24);
    harness.press("ctrl+alt+space").press("n");
    assert!(front_of(&harness).is_none(), "the window is on the dock");
    harness.press("esc");
    from_the_window_menu(&mut harness, "Settings", "Resize");
    assert!(front_of(&harness).is_some(), "it came back to be sized:\n{}", harness.screen());
    assert_eq!(harness.app().keys(), Some(Keys::Resize));
}

#[test]
fn the_first_window_says_once_how_windows_are_resized_and_a_restart_remembers() {
    let folder = std::env::temp_dir().join(format!("qdesk-test-resize-hint-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).expect("a folder of the test's own");
    let file = folder.join("desktop.toml");
    let first = qdesk::desktop::Desktop {
        icons: support::ICONS.map(str::to_owned).to_vec(),
        welcome_seen: true,
        ..qdesk::desktop::Desktop::default()
    };
    let mut harness = support::desk_writing(&file, first);
    assert!(!harness.screen().contains("resize from their edges"), "nothing is said before a window");
    harness.click(3, 4);
    harness.click(3, 4);
    assert_eq!(harness.app().windows().len(), 1, "the window opened:\n{}", harness.screen());
    let shown = harness.screen();
    assert!(shown.contains("Windows resize from their edges"), "the note is said:\n{shown}");
    assert!(shown.contains("Drag any edge or corner"), "every side sizes a window, not two:\n{shown}");
    assert_eq!(decoration(&shown), None, "{shown}");
    let written = std::fs::read_to_string(&file).expect("the desktop file is written");
    assert!(written.contains("resize_hint_seen = true"), "{written}");

    // The next start reads the file and says nothing more when a window opens.
    let (desktop, problems) = qdesk::desktop::Desktop::load(&file);
    assert!(problems.is_empty(), "{problems:?}");
    let mut again = support::desk_writing(&file, desktop);
    again.click(3, 4);
    again.click(3, 4);
    assert_eq!(again.app().windows().len(), 1);
    assert!(!again.screen().contains("resize from their edges"), "said once only:\n{}", again.screen());
    let _ = std::fs::remove_dir_all(&folder);
}
