//! The launcher: opening it, searching, the shelves, the Installable section, and the welcome
//! line that is shown once.

mod support;

use qframe::color::ColorDepth;
use qframe::event::{MouseButton, MouseKind};
use qframe::icons::GlyphMode;
use qframe::prelude::*;
use qframe::theme::State;
use support::{decoration, desk, screen, untouched};

/// The cell of the launcher button at the left end of the dock of a screen `height` rows tall.
fn button(height: u16) -> (i32, i32) {
    (2, i32::from(height) - 1)
}

#[test]
fn the_button_at_the_left_of_the_dock_opens_the_launcher_with_the_search_ready() {
    let mut harness = desk(80, 24);
    assert!(harness.app().launcher().is_none());
    harness.click(button(24).0, button(24).1);
    assert!(harness.app().launcher().is_some());
    assert!(harness.is_focused("launcher-search"), "the keys are in the search field at once");
    let screen = harness.screen();
    assert!(screen.contains("Search"), "the search field:\n{screen}");
    // The shelves on the left, the applications on the right.
    for shelf in ["All", "System", "Development", "Files", "Quvyta", "Installable"] {
        assert!(screen.contains(shelf), "the {shelf} shelf is missing:\n{screen}");
    }
    for app in ["Terminal", "Settings", "Vim", "qcode"] {
        assert!(screen.contains(app), "{app} is missing:\n{screen}");
    }
    assert!(screen.contains("open") && screen.contains("close"), "the key hints:\n{screen}");
    assert_eq!(decoration(&screen), None, "{screen}");
}

#[test]
fn space_opens_the_launcher_and_esc_closes_it() {
    let mut harness = desk(80, 24);
    harness.press("space");
    assert!(harness.app().launcher().is_some(), "{}", harness.screen());
    harness.press("esc");
    assert!(harness.app().launcher().is_none());
    assert!(!harness.screen().contains("Installable"), "{}", harness.screen());
    assert!(harness.is_focused("floor"), "the keys go back to the floor");
}

#[test]
fn the_close_mark_and_a_press_on_the_floor_both_put_the_launcher_away() {
    let mut harness = desk(80, 24);
    harness.press("space");
    harness.click(45, 2);
    assert!(harness.app().launcher().is_none(), "a press outside closes it");
    harness.press("space");
    assert!(harness.app().launcher().is_some());
    // The close mark sits at the right end of the search row.
    let (_, y) = harness.find("Search").expect("the search field is drawn");
    let row = &screen(&harness)[usize::try_from(y).unwrap_or(0)];
    let column = row.chars().position(|c| c == '×').expect("the close mark is on the search row");
    harness.click(i32::try_from(column).unwrap_or(0), y);
    assert!(harness.app().launcher().is_none(), "the close mark closes it");
}

#[test]
fn typing_filters_over_the_whole_catalog_and_enter_opens_the_first_hit() {
    let mut harness = desk(80, 24);
    harness.press("space");
    harness.type_text("vi");
    let screen = harness.screen();
    assert!(screen.contains("Vim"), "{screen}");
    assert!(!screen.contains("qcode"), "what does not match is gone:\n{screen}");
    harness.press("enter");
    assert!(harness.app().launcher().is_none(), "opening puts the launcher away");
    assert_eq!(harness.app().order().recents, ["vim"]);
    assert_eq!(harness.app().windows().len(), 1, "the first hit opened in a window:\n{}", harness.screen());
    let rows: Vec<String> = harness.screen().lines().map(str::to_owned).collect();
    assert!(rows.iter().any(|row| row.contains("Vim")), "the window's strip names it:\n{}", harness.screen());
}

#[test]
fn a_search_that_finds_nothing_says_so() {
    let mut harness = desk(80, 24);
    harness.press("space");
    harness.type_text("zzz");
    let screen = harness.screen();
    assert!(screen.contains("Nothing matches zzz"), "{screen}");
    assert!(screen.contains("The search looks at"), "{screen}");
    assert_eq!(decoration(&screen), None, "{screen}");
    // Enter on nothing does nothing, and the launcher stays open.
    harness.press("enter");
    assert!(harness.app().launcher().is_some());
    assert!(harness.app().order().recents.is_empty());
}

#[test]
fn the_installable_shelf_shows_what_is_missing_faded_with_how_it_arrives() {
    let mut harness = desk(80, 24);
    harness.press("space");
    harness.click_text("Installable");
    let screen = harness.screen();
    assert!(screen.contains("btop"), "a package qpac installs:\n{screen}");
    assert!(screen.contains("qfocus"), "a member quvyta installs:\n{screen}");
    assert!(screen.contains("install with qpac") && screen.contains("install with quvyta"), "{screen}");
    let (x, y) = harness.find("btop").expect("btop is on the shelf");
    let (x, y) = (u16::try_from(x).unwrap_or(0), u16::try_from(y).unwrap_or(0));
    let theme = harness.env().theme();
    assert_eq!(harness.fg(x, y), theme.color("muted"), "an application that is not there is faint");
}

#[test]
fn installing_tells_the_person_the_command_that_does_it() {
    let mut harness = desk(80, 24);
    harness.press("space");
    harness.click_text("Installable");
    harness.click_text("btop");
    let (x, y) = harness.find("btop").expect("btop is on the shelf");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    assert!(harness.screen().contains("Install btop with qpac"), "{}", harness.screen());
    harness.click_text("Install btop with qpac");
    let screen = harness.screen();
    assert!(screen.contains("Installing btop"), "{screen}");
    assert!(screen.contains("qpac") && screen.contains("btop"), "the notice says what to run:\n{screen}");
}

#[test]
fn a_quvyta_application_is_installed_through_quvyta_with_the_command_that_shows_it() {
    let mut harness = desk(80, 24);
    harness.press("space");
    harness.click_text("Installable");
    harness.click_text("qfocus");
    let (x, y) = harness.find("qfocus").expect("qfocus is on the shelf");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    let menu = harness.screen();
    assert!(menu.contains("Install qfocus with quvyta"), "the menu of the card:\n{menu}");
    harness.click_text("Install qfocus with quvyta");
    let screen = harness.screen();
    assert!(screen.contains("Installing qfocus"), "{screen}");
    // The notice wraps in a narrow toast, so the command is looked for in its parts.
    assert!(screen.contains("quvyta show") && screen.contains("quvyta-focus"), "{screen}");
}

#[test]
fn an_application_can_be_put_on_the_desktop_from_the_launcher_and_taken_off_again() {
    let mut harness = desk(80, 24);
    harness.press("space");
    harness.click_text("Vim");
    let (x, y) = harness.find("Vim").expect("Vim is on a card");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    harness.click_text("Put Vim on the desktop");
    assert_eq!(harness.app().order().icons, ["terminal", "settings", "mc", "vim"]);
    harness.press("esc");
    assert_eq!(harness.find("Vim"), Some((4, 10)), "it stands on the floor now");
    harness.press("space");
    harness.click_text("Vim");
    let (x, y) = harness.find("Vim").expect("Vim is on a card");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    harness.click_text("Take Vim off the desktop");
    assert_eq!(harness.app().order().icons, ["terminal", "settings", "mc"]);
}

#[test]
fn what_was_opened_comes_back_on_the_recent_shelf() {
    let mut harness = desk(80, 24);
    harness.press("space");
    harness.type_text("vi");
    harness.press("enter");
    // The keys are the program's now, and space types a space in it: the launcher is reached the
    // way the welcome line says, through desktop mode.
    harness.press("ctrl+alt+space").press("space");
    let screen = harness.screen();
    assert!(screen.contains("Recent"), "the shelf appears once something was opened:\n{screen}");
    harness.click_text("Recent");
    let screen = harness.screen();
    assert!(screen.contains("Vim"), "{screen}");
}

#[test]
fn the_launcher_stays_inside_every_screen_it_opens_on() {
    for (width, height) in [(80, 24), (200, 50), (60, 16), (40, 10)] {
        let mut harness = desk(width, height);
        harness.press("space");
        let screen = harness.screen();
        assert!(screen.contains("Search"), "{width}x{height}:\n{screen}");
        for line in screen.lines() {
            assert!(qframe::text::width(line) <= width, "{width}x{height} has a long row: {line:?}");
        }
        assert_eq!(screen.lines().count(), usize::from(height), "{width}x{height}");
        assert_eq!(decoration(&screen), None, "{width}x{height}:\n{screen}");
    }
}

#[test]
fn every_glyph_mode_and_colour_depth_draws_the_launcher_without_decoration() {
    for mode in [GlyphMode::Nerd, GlyphMode::Unicode, GlyphMode::Ascii] {
        for depth in [ColorDepth::Ansi16, ColorDepth::Ansi256, ColorDepth::TrueColor] {
            let mut harness = desk(80, 24);
            harness.set_glyph_mode(mode).set_depth(depth);
            harness.press("space");
            let screen = harness.screen();
            assert!(screen.contains("Search") && screen.contains("Terminal"), "{mode:?} {depth:?}:\n{screen}");
            assert_eq!(decoration(&screen), None, "{mode:?} {depth:?}:\n{screen}");
            // An ASCII screen is not all ASCII here: an entry may give a character of its own as
            // its icon (Quvyta's own entries do), and the framework cuts text with `…`.
        }
    }
}

#[test]
fn the_launcher_speaks_the_language_of_the_screen() {
    let mut harness = desk(80, 24);
    harness.set_locale("tr");
    harness.press("space");
    let screen = harness.screen();
    assert!(screen.contains("Ara"), "{screen}");
    assert!(screen.contains("Tümü") && screen.contains("Kurulabilir"), "{screen}");
    assert!(screen.contains("Ayarlar"), "the names come in Turkish too:\n{screen}");
}

#[test]
fn the_welcome_line_is_shown_once_and_never_comes_back() {
    let mut harness = untouched(80, 24);
    let screen = harness.screen();
    assert!(screen.contains("The applications are behind"), "{screen}");
    assert!(screen.contains("ctrl alt space"), "{screen}");
    assert_eq!(decoration(&screen), None, "{screen}");
    harness.click_text("Got it");
    assert!(harness.app().order().welcome_seen, "the desktop remembers that it was seen");
    let screen = harness.screen();
    assert!(!screen.contains("The applications are behind"), "{screen}");
    // It does not come back when the screen changes, and the floor has the keys again.
    harness.resize(100, 30);
    assert!(!harness.screen().contains("The applications are behind"));
    assert!(harness.is_focused("floor"));
}

#[test]
fn the_welcome_line_stands_over_the_floor_without_hiding_the_dock() {
    let harness = untouched(80, 24);
    let screen = harness.screen();
    assert!(screen.contains("Terminal"), "the icons are still there:\n{screen}");
    assert!(screen.lines().last().is_some_and(|dock| dock.contains("sunucu-1")), "{screen}");
}

/// Whether the launcher button at the left end of the bottom dock is drawn pressed: the chosen
/// state of a framework button, its steady pillar in the first cell and the chosen tone under it.
fn pressed(harness: &mut Harness<qdesk::app::Desk>, height: u16) -> bool {
    // The pointer is taken off the dock first: under it the button lights up as any button does.
    harness.hover(40, 1);
    let row = screen(harness)[usize::from(height) - 1].clone();
    let pillar = row.chars().nth(1) == Some('▌');
    let theme = harness.env().theme();
    let chosen = theme.style("button", None, &[State::Selected]).paint("bg").map(|paint| paint.at(0.0));
    pillar && chosen.is_some() && harness.bg(2, height - 1) == chosen
}

#[test]
fn the_dock_button_opens_and_closes_the_launcher_like_a_start_button() {
    let mut harness = desk(80, 24);
    assert!(!pressed(&mut harness, 24), "at rest the button is not pressed:\n{}", harness.screen());
    harness.click(button(24).0, button(24).1);
    assert!(harness.app().launcher().is_some(), "the first press opens it");
    assert!(pressed(&mut harness, 24), "open, the button stays pressed:\n{}", harness.screen());
    harness.click(button(24).0, button(24).1);
    assert!(harness.app().launcher().is_none(), "the second press closes it:\n{}", harness.screen());
    assert!(!pressed(&mut harness, 24), "and the button comes up again:\n{}", harness.screen());
    assert!(harness.is_focused("floor"), "the keys go back to the floor");
}

#[test]
fn a_second_space_closes_the_launcher_until_something_is_typed() {
    let mut harness = desk(80, 24);
    harness.press("space");
    assert!(harness.app().launcher().is_some());
    harness.press("space");
    assert!(harness.app().launcher().is_none(), "space again closes it:\n{}", harness.screen());

    // Once a word is typed a space belongs to the search.
    harness.press("space");
    harness.type_text("midnight");
    harness.press("space");
    assert!(harness.app().launcher().is_some(), "{}", harness.screen());
    assert_eq!(harness.app().launcher().map(|launcher| launcher.query.as_str()), Some("midnight "));
}

#[test]
fn in_desktop_mode_space_opens_the_launcher_and_space_closes_it() {
    let mut harness = desk(80, 24);
    harness.press("ctrl+alt+space");
    harness.press("space");
    assert!(harness.app().launcher().is_some(), "{}", harness.screen());
    harness.press("space");
    assert!(harness.app().launcher().is_none(), "{}", harness.screen());
}

#[test]
fn the_launcher_is_a_compact_menu_in_the_corner_of_every_screen() {
    for (width, height) in [(80_u16, 24_u16), (120, 40), (200, 50)] {
        let mut harness = desk(width, height);
        harness.press("space");
        let rows = screen(&harness);
        let top = rows.iter().position(|row| row.contains("Search")).expect("the search row");
        let hints = rows.iter().rposition(|row| row.contains("open") && row.contains("close")).expect("the hint row");
        let close = rows[top].chars().position(|c| c == '×').expect("the close mark");
        // A Start menu: narrower than the old strip of 72 columns, and as tall as what it holds
        // rather than stretched over the screen.
        assert!(close < 64, "{width}x{height}: the launcher reaches column {close}:\n{}", harness.screen());
        assert!(hints - top <= 12, "{width}x{height}: {} rows for seven shelves:\n{}", hints - top, harness.screen());
        // It stands on the dock: its last row is the one above the dock's.
        assert_eq!(hints + 2, usize::from(height) - 1, "{width}x{height}:\n{}", harness.screen());
        // Every application is on one row with what it does beside it.
        let terminal = rows.iter().find(|row| row.contains("Terminal") && row.contains("Your shell"));
        assert!(terminal.is_some(), "{width}x{height}: the description stands beside the name:\n{}", harness.screen());
    }
}
