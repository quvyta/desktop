//! The icons on the floor: how they are drawn, selected, moved and opened, with the mouse and
//! with the keys, at several sizes, glyph modes and colour depths.

mod support;

use qdesk::desktop::{CELL_HEIGHT, CELL_WIDTH};
use qframe::color::ColorDepth;
use qframe::event::{MouseButton, MouseKind};
use qframe::icons::GlyphMode;
use qframe::prelude::*;
use support::{desk, desk_with, screen};

/// The middle of the cell of the icon at `index`: the row its glyph is on.
fn glyph_row(index: u16) -> (i32, i32) {
    (5, i32::from(index * CELL_HEIGHT))
}

/// A cell of empty floor, to the right of every icon column.
const FLOOR: (i32, i32) = (45, 2);

#[test]
fn the_floor_shows_the_icons_down_its_left_edge_with_their_names_under_them() {
    let harness = desk(80, 24);
    let rows = screen(&harness);
    // The first column of cells: Terminal, Settings, Midnight Commander, three rows apart.
    assert_eq!(harness.find("Terminal"), Some((1, 1)));
    assert_eq!(harness.find("Settings"), Some((1, 4)));
    assert!(rows[2].is_empty(), "a free row stands between two icons: {:?}", rows[2]);
    // The long name is cut to the cell and keeps its beginning.
    let (x, y) = harness.find("Midnight").expect("the third icon is drawn");
    assert_eq!((x, y), (1, 7));
    assert!(rows[7].contains('…'), "the name is cut with an ellipsis: {:?}", rows[7]);
    assert!(qframe::text::width(rows[7].trim_end()) <= CELL_WIDTH, "the name stays inside its cell");
    // Every icon has its glyph above its name.
    for index in 0..3 {
        let (x, y) = glyph_row(index);
        let glyph = harness.buffer()[(u16::try_from(x).unwrap_or(0), u16::try_from(y).unwrap_or(0))].symbol();
        assert!(!glyph.trim().is_empty(), "the icon at {index} has a glyph, not {glyph:?}");
    }
}

#[test]
fn a_click_selects_one_icon_and_raises_its_cell_with_the_accent_pillar() {
    let mut harness = desk(80, 24);
    assert!(harness.app().selection().is_empty());
    harness.click(3, 1);
    assert_eq!(harness.app().selection(), ["terminal"]);
    // The pointer leaves the cell, so what is left is the selection's own tone and not the lift
    // the pointer gives whatever it rests on.
    harness.hover(60, 20);
    let theme = harness.env().theme();
    assert_eq!(harness.bg(3, 0), theme.color("active"), "the cell is a raised surface");
    assert_eq!(harness.bg(3, 1), theme.color("active"));
    assert_eq!(harness.bg(3, 2), theme.color("active"), "the whole cell, the free row too");
    assert_eq!(harness.fg(0, 0), theme.color("accent"), "the pillar takes the accent");
    assert_eq!(harness.buffer()[(0, 1)].symbol(), "▌", "the pillar runs down the left edge");
    // The icon beside it stays on the floor's own tone.
    assert_eq!(harness.bg(3, 3), theme.color("canvas"));
}

#[test]
fn ctrl_click_gathers_several_icons_and_a_click_on_the_floor_lets_them_go() {
    let mut harness = desk(80, 24);
    harness.click(3, 1);
    harness.mouse(MouseKind::Down(MouseButton::Left), 3, 4);
    harness.events(&[Event::Mouse(qframe::event::MouseEvent {
        kind: MouseKind::Up(MouseButton::Left),
        x: 3,
        y: 4,
        mods: qframe::keymap::Modifiers { ctrl: true, alt: false, shift: false },
    })]);
    assert_eq!(harness.app().selection(), ["terminal", "settings"]);
    harness.click(FLOOR.0, FLOOR.1);
    assert!(harness.app().selection().is_empty(), "the floor clears the selection");
}

#[test]
fn a_band_drawn_from_empty_floor_takes_the_icons_it_touches() {
    let mut harness = desk(80, 24);
    harness.drag((40, 1), (2, 5));
    assert_eq!(harness.app().selection(), ["terminal", "settings"]);
    // And a band over bare floor takes nothing.
    harness.drag((40, 1), (60, 8));
    assert!(harness.app().selection().is_empty());
}

#[test]
fn a_band_shows_as_a_tone_mixed_into_the_floor_with_no_line_around_it() {
    let mut harness = desk(80, 24);
    harness.mouse(MouseKind::Down(MouseButton::Left), 40, 1);
    harness.mouse(MouseKind::Drag(MouseButton::Left), 20, 6);
    let canvas = harness.env().theme().color("canvas").expect("the theme has a canvas");
    let accent = harness.env().theme().color("accent").expect("the theme has an accent");
    let inside = harness.bg(30, 3).expect("the band is drawn");
    assert_ne!(inside, canvas, "the band lifts the floor's tone");
    assert_ne!(inside, accent, "but it is a mix, not the accent itself");
    assert_eq!(harness.bg(60, 3), Some(canvas), "outside the band the floor is untouched");
    let screen = harness.screen();
    assert_eq!(support::decoration(&screen), None, "a band is a tone, never a line:\n{screen}");
    harness.mouse(MouseKind::Up(MouseButton::Left), 20, 6);
}

#[test]
fn arrows_walk_the_icons_and_enter_opens_the_one_they_are_on() {
    let mut harness = desk(80, 24);
    // The first arrow key lands on the first icon; from there the keys walk.
    harness.press("down");
    assert_eq!(harness.buffer()[(0, 0)].symbol(), "▌", "the cursor is on the first icon");
    harness.press("down");
    assert_eq!(harness.buffer()[(0, 3)].symbol(), "▌", "and then on the second");
    assert_eq!(harness.buffer()[(0, 0)].symbol(), " ", "only one cursor at a time");
    harness.press("enter");
    assert_eq!(harness.app().order().recents, ["settings"], "opening remembers it");
    assert_eq!(harness.app().windows().len(), 1, "a window opened:\n{}", harness.screen());
    assert!(harness.screen().contains("Appearance"), "{}", harness.screen());
}

#[test]
fn the_mouse_and_the_keys_reach_the_same_result() {
    let mut keys = desk(80, 24);
    keys.press("down").press("down").press("enter");
    let mut mouse = desk(80, 24);
    // Two clicks on the same icon open it.
    mouse.click(3, 4);
    mouse.click(3, 4);
    assert_eq!(keys.app().order().recents, ["settings"]);
    assert_eq!(keys.app().order().recents, mouse.app().order().recents);
    assert_eq!(keys.app().windows().len(), 1, "{}", keys.screen());
    assert_eq!(mouse.app().windows().len(), 1, "{}", mouse.screen());
    // The cursor stands on the same icon whichever way it got there.
    assert_eq!(keys.buffer()[(0, 3)].symbol(), mouse.buffer()[(0, 3)].symbol());
}

#[test]
fn one_click_selects_and_does_not_open() {
    let mut harness = desk(80, 24);
    harness.click(3, 4);
    assert_eq!(harness.app().selection(), ["settings"]);
    assert!(harness.app().order().recents.is_empty(), "one click opens nothing");
    assert!(!harness.screen().contains("not opened yet"));
}

#[test]
fn typing_a_letter_jumps_to_the_icon_whose_name_starts_with_it() {
    let mut harness = desk(80, 24);
    harness.press("m");
    assert_eq!(harness.buffer()[(0, 6)].symbol(), "▌", "the cursor went to Midnight Commander");
    harness.press("t");
    assert_eq!(harness.buffer()[(0, 0)].symbol(), "▌", "and back up to Terminal");
}

#[test]
fn dragging_an_icon_onto_another_cell_moves_it_inside_the_grid() {
    let mut harness = desk(80, 24);
    harness.drag((3, 1), (3, 7));
    assert_eq!(harness.app().order().icons, ["settings", "mc", "terminal"]);
    assert_eq!(harness.find("Settings"), Some((1, 1)), "the others moved up");
    assert_eq!(harness.find("Terminal"), Some((1, 7)), "and it stands where it was dropped");
    assert_eq!(harness.buffer()[(0, 6)].symbol(), "▌", "the cursor followed it");
}

#[test]
fn the_menu_of_an_icon_names_it_and_the_menu_of_the_floor_offers_a_terminal() {
    let mut harness = desk(80, 24);
    harness.mouse(MouseKind::Down(MouseButton::Right), 3, 1);
    let screen = harness.screen();
    assert!(screen.contains("Open"), "the icon's menu:\n{screen}");
    assert!(screen.contains("Remove from the desktop"), "{screen}");
    assert!(screen.contains("Properties"), "{screen}");
    harness.press("esc");
    let mut floor = desk(80, 24);
    floor.mouse(MouseKind::Down(MouseButton::Right), FLOOR.0, FLOOR.1);
    let screen = floor.screen();
    assert!(screen.contains("New terminal"), "the floor's menu:\n{screen}");
    assert!(screen.contains("Add an application"), "{screen}");
    assert!(screen.contains("Arrange icons"), "{screen}");
    assert!(screen.contains("Settings"), "{screen}");
}

#[test]
fn taking_an_icon_off_the_desktop_leaves_the_others() {
    let mut harness = desk(80, 24);
    harness.mouse(MouseKind::Down(MouseButton::Right), 3, 1);
    harness.click_text("Remove from the desktop");
    assert_eq!(harness.app().order().icons, ["settings", "mc"]);
    assert_eq!(harness.find("Terminal"), None);
}

#[test]
fn arranging_the_icons_puts_them_in_the_order_of_their_names() {
    let mut harness = desk_with(&["mc", "terminal", "settings"], 80, 24);
    harness.mouse(MouseKind::Down(MouseButton::Right), FLOOR.0, FLOOR.1);
    harness.click_text("Arrange icons");
    assert_eq!(harness.app().order().icons, ["mc", "settings", "terminal"]);
}

#[test]
fn the_icons_are_hidden_on_a_screen_too_narrow_for_them() {
    let harness = desk(59, 20);
    assert_eq!(harness.find("Terminal"), None, "no icons:\n{}", harness.screen());
    assert!(harness.screen().contains("sunucu-1"), "the dock stays");
    let short = desk(80, 15);
    assert_eq!(short.find("Terminal"), None);
    // Sixty by sixteen is the smallest screen that keeps them.
    let kept = desk(60, 16);
    assert_eq!(kept.find("Terminal"), Some((1, 1)));
}

/// The rows of `harness`'s screen that say where the applications went.
fn hint_rows(harness: &Harness<qdesk::app::Desk>) -> Vec<String> {
    screen(harness).into_iter().filter(|row| row.contains("Applications:")).collect()
}

#[test]
fn a_screen_too_narrow_for_icons_says_in_one_line_where_the_applications_are() {
    let harness = desk(59, 20);
    let said = hint_rows(&harness);
    assert_eq!(said.len(), 1, "one line, no more:\n{}", harness.screen());
    assert!(said[0].contains("ctrl alt space"), "the keys are named where they fit: {:?}", said[0]);
    // The line stands in the middle of the floor, not against an edge.
    let (x, y) = harness.find("Applications:").expect("the line is drawn");
    assert!(x > 0 && y > 2 && y < 17, "centred, not at an edge: ({x}, {y})");

    // The smallest desktop has no room for the keys, so the line keeps only the button.
    let smallest = desk(40, 10);
    let said = hint_rows(&smallest);
    assert_eq!(said.len(), 1, "the short form is still one line:\n{}", smallest.screen());
    assert!(!said[0].contains("ctrl"), "nothing is cut half way: {:?}", said[0]);
    assert!(!said[0].contains('…'), "the short form fits whole: {:?}", said[0]);

    // A screen with room for the icons shows the icons, not the line.
    assert!(hint_rows(&desk(60, 16)).is_empty());
}

#[test]
fn the_welcome_line_on_a_narrow_screen_is_not_said_twice() {
    let harness = support::untouched(59, 20);
    assert!(harness.screen().contains("The applications are behind"), "the welcome is there:\n{}", harness.screen());
    assert!(hint_rows(&harness).is_empty(), "the welcome stands over the line:\n{}", harness.screen());
}

#[test]
fn icons_fill_one_column_before_another_one_starts() {
    let floor = ["terminal", "settings", "mc", "vim", "lf", "qcode"];
    let mut harness = desk_with(&floor, 80, 16);
    // Fifteen rows of floor hold five cells; the sixth icon starts the second column.
    assert_eq!(harness.find("Terminal"), Some((1, 1)));
    assert_eq!(harness.find("qcode"), Some((13, 1)), "the second column starts ten cells along");
    harness.resize(80, 24);
    assert_eq!(harness.find("qcode"), Some((3, 16)), "a taller screen puts them all in one column");
}

#[test]
fn every_glyph_mode_and_colour_depth_draws_the_icons_without_decoration() {
    for mode in [GlyphMode::Nerd, GlyphMode::Unicode, GlyphMode::Ascii] {
        for depth in [ColorDepth::Ansi16, ColorDepth::Ansi256, ColorDepth::TrueColor] {
            for (width, height) in [(80, 24), (200, 50), (60, 16)] {
                let mut harness = desk(width, height);
                harness.set_glyph_mode(mode).set_depth(depth);
                harness.click(3, 1);
                let screen = harness.screen();
                assert!(screen.contains("Terminal"), "{mode:?} {depth:?} {width}x{height}:\n{screen}");
                assert_eq!(support::decoration(&screen), None, "{mode:?} {depth:?}:\n{screen}");
                if mode == GlyphMode::Ascii {
                    // The framework cuts text with `…` in every mode, so that one character is
                    // what an ASCII screen may still hold.
                    assert!(screen.replace('…', "...").is_ascii(), "{depth:?}:\n{screen}");
                }
                assert_eq!(harness.app().selection(), ["terminal"], "{mode:?} {depth:?} {width}x{height}");
            }
        }
    }
}
