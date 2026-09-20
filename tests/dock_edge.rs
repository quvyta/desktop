//! The dock at the edge the settings name: the row it is drawn on, the room the windows get, and
//! everything that hangs off it — the launcher, the notification list, the list of the windows
//! that do not fit, and the hint row of desktop mode.
//!
//! Every state is checked at both edges, because the setting is only real if it is real
//! everywhere: an empty desktop, a maximized window, a snapped window, tiling, the narrow screen
//! that has no room for icons, and the screen that is too small for a desktop at all.

mod support;

use qdesk::app::{Desk, Msg};
use qdesk::settings::{self, DockPosition};
use qframe::event::{MouseButton, MouseKind};
use qframe::geometry::Rect;
use qframe::prelude::*;

use support::{MACHINE, desk, screen, untouched};

/// The desktop with the dock moved to `side`, the way the Settings screen moves it: a message,
/// not a restart.
fn moved(harness: &mut Harness<Desk>, side: DockPosition) {
    harness.send(Msg::Settings(settings::Msg::Dock(side)));
    harness.render();
    assert_eq!(harness.app().prefs().dock, side);
}

/// A desktop of `width` by `height` whose dock sits on `side`.
fn desk_with_dock(side: DockPosition, width: u16, height: u16) -> Harness<Desk> {
    let mut harness = desk(width, height);
    moved(&mut harness, side);
    harness
}

/// The row of the screen the dock is drawn on: the first one at the top, the last at the bottom.
fn dock_row(side: DockPosition, height: u16) -> usize {
    match side {
        DockPosition::Top => 0,
        DockPosition::Bottom => usize::from(height) - 1,
    }
}

/// The row the dock is on, as text.
fn dock(harness: &Harness<Desk>, side: DockPosition, height: u16) -> String {
    screen(harness)[dock_row(side, height)].clone()
}

/// Every row of the screen but the dock's: what is left for the desktop.
fn floor(harness: &Harness<Desk>, side: DockPosition, height: u16) -> Vec<String> {
    let mut rows = screen(harness);
    rows.remove(dock_row(side, height));
    rows
}

/// How many rows stand between the dock and the nearest row of the surface `marks` are found on.
///
/// It is the one number that says the same thing at both edges: a surface that hangs off the dock
/// touches its row the same way whichever end the dock took.
fn near_dock(rows: &[String], side: DockPosition, marks: &[&str]) -> usize {
    let found: Vec<usize> = rows
        .iter()
        .enumerate()
        .filter(|(_, line)| marks.iter().any(|mark| line.contains(mark)))
        .map(|(index, _)| index)
        .collect();
    assert!(!found.is_empty(), "{side:?}: none of {marks:?} is on screen");
    match side {
        DockPosition::Top => found.into_iter().min().expect("a row"),
        DockPosition::Bottom => rows.len() - 1 - found.into_iter().max().expect("a row"),
    }
}

/// Opens the Settings window from its icon on the floor, wherever the floor put it.
fn with_settings(harness: &mut Harness<Desk>) {
    let (x, y) = harness.find("Settings").expect("the Settings icon is on the floor");
    harness.click(x, y);
    harness.click(x, y);
    assert_eq!(harness.app().windows().len(), 1, "the window opened:\n{}", harness.screen());
}

/// The rectangle of the window in front, in the desktop's own coordinates.
fn front(harness: &Harness<Desk>) -> Rect {
    harness.app().windows().front().expect("a window on the desktop").rect()
}

#[test]
fn the_dock_is_drawn_on_the_row_the_setting_names() {
    for side in DockPosition::ALL {
        let harness = desk_with_dock(side, 80, 24);
        let row = dock(&harness, side, 24);
        assert!(row.contains(MACHINE), "{side:?}: the dock is not on its row: {row}");
        assert!(row.contains("14:32"), "{side:?}: {row}");
        let rest = floor(&harness, side, 24);
        assert!(
            rest.iter().all(|line| !line.contains(MACHINE)),
            "{side:?}: the dock is drawn twice:\n{}",
            harness.screen()
        );
    }
}

#[test]
fn the_setting_moves_the_dock_at_once_and_back_again() {
    let mut harness = desk(80, 24);
    assert!(dock(&harness, DockPosition::Bottom, 24).contains(MACHINE), "it starts at the bottom");
    moved(&mut harness, DockPosition::Top);
    assert!(dock(&harness, DockPosition::Top, 24).contains(MACHINE), "and moves without a restart");
    assert!(!dock(&harness, DockPosition::Bottom, 24).contains(MACHINE), "the bottom row is the desktop's again");
    moved(&mut harness, DockPosition::Bottom);
    assert!(dock(&harness, DockPosition::Bottom, 24).contains(MACHINE), "and moves back");
    assert!(!dock(&harness, DockPosition::Top, 24).contains(MACHINE));
}

#[test]
fn the_dock_keeps_a_tone_of_its_own_at_either_edge() {
    for side in DockPosition::ALL {
        let harness = desk_with_dock(side, 80, 24);
        let theme = harness.env().theme();
        let canvas = theme.color("canvas").expect("the theme has a canvas");
        let surface = theme.color("surface").expect("the theme has a surface");
        let row = u16::try_from(dock_row(side, 24)).expect("a row on screen");
        for x in [0, 40, 79] {
            assert_eq!(harness.bg(x, row), Some(surface), "{side:?}: the dock at column {x}");
        }
        let elsewhere = if side == DockPosition::Top { 23 } else { 0 };
        assert_eq!(harness.bg(79, elsewhere), Some(canvas), "{side:?}: the floor keeps the canvas");
    }
}

#[test]
fn a_maximized_window_takes_the_desktop_and_never_the_dock_s_row() {
    for side in DockPosition::ALL {
        let mut harness = desk_with_dock(side, 80, 24);
        with_settings(&mut harness);
        harness.press("ctrl+alt+space").press("z").press("esc");
        assert_eq!(front(&harness), Rect::new(0, 0, 80, 23), "{side:?}: it fills the desktop");
        // The dock is whole on its row: a window over it would have taken the machine name away.
        assert!(dock(&harness, side, 24).contains(MACHINE), "{side:?}:\n{}", harness.screen());
        // And the window itself starts on the first row the dock left it.
        let rows = screen(&harness);
        let strip = if side == DockPosition::Top { &rows[1] } else { &rows[0] };
        assert!(strip.contains("Settings"), "{side:?}: the window's strip is not against the dock: {strip}");
    }
}

#[test]
fn a_snapped_window_takes_half_the_desktop_at_either_edge() {
    for side in DockPosition::ALL {
        let mut harness = desk_with_dock(side, 80, 24);
        with_settings(&mut harness);
        let rect = front(&harness);
        let title = (rect.x + 4, rect.y + i32::from(u16::from(side == DockPosition::Top)));
        harness.mouse(MouseKind::Down(MouseButton::Left), title.0, title.1);
        harness.mouse(MouseKind::Drag(MouseButton::Left), title.0 - 40, title.1);
        harness.mouse(MouseKind::Up(MouseButton::Left), title.0 - 40, title.1);
        assert_eq!(front(&harness), Rect::new(0, 0, 40, 23), "{side:?}: it takes the left half");
        assert!(dock(&harness, side, 24).contains(MACHINE), "{side:?}:\n{}", harness.screen());
    }
}

#[test]
fn tiled_windows_share_the_desktop_without_the_dock_s_row() {
    for side in DockPosition::ALL {
        let mut harness = desk_with_dock(side, 80, 24);
        with_settings(&mut harness);
        harness.press("ctrl+alt+space").press("t");
        let rects: Vec<Rect> = harness.app().windows().iter().map(qdesk::wm::Window::rect).collect();
        assert!(!rects.is_empty(), "{side:?}: a window is on the desktop");
        for rect in rects {
            assert!(rect.y >= 0 && rect.bottom() <= 23, "{side:?}: {rect:?} reaches the dock's row");
        }
    }
}

#[test]
fn the_launcher_hangs_off_the_dock_at_either_edge() {
    let reach = |side| {
        let mut harness = desk_with_dock(side, 80, 24);
        let row = i32::try_from(dock_row(side, 24)).expect("a row on screen");
        // The launcher button is the first thing on the dock's row, wherever that row is.
        harness.click(2, row);
        assert!(harness.screen().contains("Search"), "{side:?}: the launcher did not open:\n{}", harness.screen());
        assert!(dock(&harness, side, 24).contains(MACHINE), "{side:?}: the dock keeps its row");
        near_dock(&screen(&harness), side, &["Search", "close"])
    };
    // The design has it rise from the dock (3.5); a dock on the top row is above it, so it comes
    // down from there instead. Either way it stands the same way off the dock's own row.
    assert_eq!(reach(DockPosition::Top), reach(DockPosition::Bottom));
}

#[test]
fn the_notification_list_hangs_off_the_dock_at_either_edge() {
    let reach = |side| {
        let mut harness = desk_with_dock(side, 80, 24);
        harness.send(Msg::Notices);
        harness.render();
        assert!(harness.screen().contains("Notifications"), "{side:?}:\n{}", harness.screen());
        assert!(dock(&harness, side, 24).contains(MACHINE), "{side:?}: the dock keeps its row");
        near_dock(&screen(&harness), side, &["Notifications", "come here"])
    };
    assert_eq!(reach(DockPosition::Top), reach(DockPosition::Bottom));
}

#[test]
fn the_list_of_the_windows_that_do_not_fit_hangs_off_the_dock_at_either_edge() {
    let reach = |side| {
        let mut harness = desk_with_dock(side, 80, 24);
        for name in ["Settings", "Terminal", "Midnight"] {
            let (x, y) = harness.find(name).unwrap_or_else(|| panic!("{name} is on the floor"));
            harness.click(x, y);
            harness.click(x, y);
        }
        assert_eq!(harness.app().windows().len(), 3, "{side:?}:\n{}", harness.screen());
        // A row this narrow cannot hold three items, so the last of them go behind the control.
        harness.resize(40, 24);
        harness.send(Msg::MoreWindows);
        harness.render();
        assert!(harness.screen().contains("More windows"), "{side:?}:\n{}", harness.screen());
        assert!(dock(&harness, side, 24).contains("+"), "{side:?}: the control is on the dock");
        near_dock(&screen(&harness), side, &["More windows", "Midnight"])
    };
    assert_eq!(reach(DockPosition::Top), reach(DockPosition::Bottom));
}

#[test]
fn desktop_mode_replaces_the_dock_s_row_wherever_it_is() {
    for side in DockPosition::ALL {
        let mut harness = desk_with_dock(side, 80, 24);
        with_settings(&mut harness);
        harness.press("ctrl+alt+space");
        let row = dock(&harness, side, 24);
        assert!(!row.contains(MACHINE), "{side:?}: the hints take the dock's row: {row}");
        assert!(row.contains("move"), "{side:?}: {row}");
        harness.press("esc");
        assert!(dock(&harness, side, 24).contains(MACHINE), "{side:?}: and the dock comes back");
    }
}

#[test]
fn the_narrow_desktop_and_the_smallest_one_keep_the_dock_on_its_row() {
    for side in DockPosition::ALL {
        for (width, height) in [(80, 24), (60, 16), (40, 10)] {
            let harness = desk_with_dock(side, width, height);
            let row = dock(&harness, side, height);
            assert!(row.contains(MACHINE), "{side:?} at {width}x{height}: {row}");
        }
    }
}

#[test]
fn the_welcome_line_never_hides_the_dock_at_either_edge() {
    for side in DockPosition::ALL {
        let mut harness = untouched(80, 24);
        moved(&mut harness, side);
        assert!(harness.screen().contains('❖'), "{side:?}: the welcome line is there:\n{}", harness.screen());
        // It stands in the middle of the floor, so the dock's row is untouched at either edge.
        assert!(dock(&harness, side, 24).contains(MACHINE), "{side:?}:\n{}", harness.screen());
    }
}

#[test]
fn a_screen_too_small_for_a_desktop_has_no_dock_at_either_edge() {
    for side in DockPosition::ALL {
        let harness = desk_with_dock(side, 30, 8);
        assert!(!harness.screen().contains(MACHINE), "{side:?}: no dock on a screen this small:\n{}", harness.screen());
    }
}
