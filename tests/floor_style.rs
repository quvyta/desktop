//! The floor's pattern: chosen on the Settings screen with the mouse, drawn at once on the floor
//! around the window, kept through a restart, and drawn as each colour depth and glyph mode can.

mod support;

use std::path::PathBuf;

use qdesk::settings::{FloorColor, FloorStyle, Palette, Shades};
use qframe::color::ColorDepth;
use qframe::icons::GlyphMode;
use qframe::prelude::*;

/// The terminal these tests draw on: wide and tall enough that the Settings window leaves floor
/// on every side of it.
const SIZE: (u16, u16) = (140, 44);

/// A column of floor right of the Settings window, on a corner of the icon grid.
const COLUMN: u16 = 130;

/// A fresh folder of the test's own for the settings file.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qdesk-test-floor-style-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("folder");
    dir
}

/// Opens the Settings window from its icon on the floor.
fn open_settings(harness: &mut Harness<qdesk::app::Desk>) {
    let (x, y) = harness.find("Settings").expect("the Settings icon is on the floor");
    harness.click(x, y);
    harness.click(x, y);
    assert_eq!(harness.app().windows().len(), 1, "the window opened:\n{}", harness.screen());
    let window = harness.app().windows().front().expect("the window").rect();
    assert!(window.right() <= i32::from(COLUMN), "the column is floor: {window:?}");
}

/// The shades a floor of `tone` and `style` has in the harness's theme, in full colour.
fn shades(harness: &Harness<qdesk::app::Desk>, tone: FloorColor, style: FloorStyle) -> Shades {
    let palette = Palette::of(harness.env().theme()).expect("the theme has a canvas");
    Shades::new(tone, style, palette, ColorDepth::TrueColor)
}

/// The rows of floor: the screen without the dock's row.
fn floor_rows() -> u16 {
    SIZE.1 - 1
}

/// The symbol in the cell `x`, `y`.
fn symbol(harness: &Harness<qdesk::app::Desk>, x: u16, y: u16) -> String {
    harness.buffer()[(x, y)].symbol().to_owned()
}

#[test]
fn the_gradient_chosen_on_the_settings_screen_warms_the_floor_down_to_the_dock_and_survives_a_restart() {
    let config = folder("gradient");
    let mut harness = support::desk_in(&config, SIZE.0, SIZE.1);
    open_settings(&mut harness);
    let canvas = harness.env().theme().color("canvas");
    let bottom = floor_rows() - 1;
    assert_eq!(harness.bg(COLUMN, 0), canvas, "the floor starts plain");
    assert_eq!(harness.bg(COLUMN, bottom), canvas);

    harness.click_text("Gradient");
    assert_eq!(harness.app().prefs().floor_style, FloorStyle::Gradient);
    let expected = shades(&harness, FloorColor::Theme, FloorStyle::Gradient);
    assert_eq!(harness.bg(COLUMN, 0), Some(expected.row(0, floor_rows())), "the top row is the tone itself");
    assert_eq!(harness.bg(COLUMN, bottom), Some(expected.row(bottom, floor_rows())), "the bottom row carries the glow");
    assert_ne!(harness.bg(COLUMN, 0), harness.bg(COLUMN, bottom), "the floor changes from top to bottom");
    assert_eq!(harness.bg(0, bottom), harness.bg(COLUMN, bottom), "one colour across a row");
    assert_eq!(support::decoration(&harness.screen()), None, "{}", harness.screen());
    let written = std::fs::read_to_string(config.join("desktop.conf")).expect("the settings file is written");
    assert!(written.contains("floor-style = \"gradient\""), "{written}");

    // The next run reads it back from the file.
    let again = support::desk_in(&config, SIZE.0, SIZE.1);
    assert_eq!(again.app().prefs().floor_style, FloorStyle::Gradient);
    assert_eq!(
        again.bg(COLUMN, bottom),
        Some(expected.row(bottom, floor_rows())),
        "after a restart:\n{}",
        again.screen()
    );
    let _ = std::fs::remove_dir_all(&config);
}

#[test]
fn the_dots_chosen_on_the_settings_screen_stand_on_the_corners_of_the_icon_grid_and_nowhere_else() {
    let config = folder("dots");
    let mut harness = support::desk_in(&config, SIZE.0, SIZE.1);
    open_settings(&mut harness);
    assert_eq!(symbol(&harness, COLUMN, 2), " ", "no dot before one is chosen");
    harness.click_text("Dots");
    assert_eq!(harness.app().prefs().floor_style, FloorStyle::Dots);
    assert_eq!(symbol(&harness, COLUMN, 2), "\u{b7}", "a dot on a corner of the grid:\n{}", harness.screen());
    assert_eq!(symbol(&harness, COLUMN + 1, 2), " ", "and none beside it");
    assert_eq!(symbol(&harness, COLUMN, 3), " ", "nor under it");
    let ground = harness.bg(COLUMN, 2).expect("the floor is drawn");
    let dot = harness.fg(COLUMN, 2).expect("the dot has a colour");
    let quiet = dot.contrast_ratio(ground);
    assert!((1.15..2.5).contains(&quiet), "a dot is quiet: {quiet:.2}:1");
    // Terminal's cell is neither selected nor under the cursor, so the corner under it is bare
    // floor and carries its dot; its glyph and name rows never do.
    assert_eq!(symbol(&harness, 0, 2), "\u{b7}", "{}", harness.screen());
    for row in harness.screen().lines().take(2) {
        assert!(!row.contains('\u{b7}'), "no dot on an icon's own rows: {row:?}");
    }
    let dots = harness.screen().matches('\u{b7}').count();
    assert!(dots > 20, "the floor around the window is dotted: {dots}");

    let again = support::desk_in(&config, SIZE.0, SIZE.1);
    assert_eq!(symbol(&again, COLUMN, 2), "\u{b7}", "after a restart:\n{}", again.screen());
    let _ = std::fs::remove_dir_all(&config);
}

#[test]
fn both_are_the_gradient_with_the_dots_over_it() {
    let config = folder("both");
    let mut harness = support::desk_in(&config, SIZE.0, SIZE.1);
    open_settings(&mut harness);
    harness.click_text("Both");
    assert_eq!(harness.app().prefs().floor_style, FloorStyle::GradientDots);
    let expected = shades(&harness, FloorColor::Theme, FloorStyle::GradientDots);
    let bottom = floor_rows() - 1;
    assert_eq!(harness.bg(COLUMN, bottom), Some(expected.row(bottom, floor_rows())));
    assert_eq!(symbol(&harness, COLUMN, 2), "\u{b7}");
    let _ = std::fs::remove_dir_all(&config);
}

#[test]
fn ascii_draws_the_dots_as_full_stops_and_sixteen_colours_draw_a_plain_floor() {
    let config = folder("modes");
    std::fs::write(config.join("desktop.conf"), "floor-style = \"gradient-dots\"\nfloor-color = \"deep\"\n")
        .expect("file");
    let mut harness = support::desk_in(&config, SIZE.0, SIZE.1);
    harness.set_glyph_mode(GlyphMode::Ascii).render();
    assert_eq!(symbol(&harness, COLUMN, 2), ".", "{}", harness.screen());

    harness.set_glyph_mode(GlyphMode::Unicode).set_depth(ColorDepth::Ansi16).render();
    assert_eq!(symbol(&harness, COLUMN, 2), " ", "no dots in sixteen colours:\n{}", harness.screen());
    assert_eq!(harness.bg(COLUMN, 0), harness.bg(COLUMN, floor_rows() - 1), "and one colour top to bottom");
    assert_eq!(support::decoration(&harness.screen()), None);

    harness.set_depth(ColorDepth::Ansi256).render();
    assert_eq!(symbol(&harness, COLUMN, 2), "\u{b7}", "256 colours keep the dots");
    let _ = std::fs::remove_dir_all(&config);
}

#[test]
fn a_floor_pattern_the_file_does_not_know_is_said_and_the_floor_stays_plain() {
    let config = folder("unknown");
    std::fs::write(config.join("desktop.conf"), "floor-style = \"tartan\"\n").expect("file");
    let loaded = qdesk::settings::load_in(&config);
    assert_eq!(loaded.prefs.floor_style, FloorStyle::Plain);
    assert_eq!(loaded.diagnostics.len(), 1, "{:?}", loaded.diagnostics);
    assert!(loaded.diagnostics[0].location().contains("desktop.conf:1"), "{:?}", loaded.diagnostics);
    let harness = support::desk_in(&config, SIZE.0, SIZE.1);
    let canvas = harness.env().theme().color("canvas");
    assert_eq!(harness.bg(COLUMN, floor_rows() - 1), canvas);
    assert_eq!(symbol(&harness, COLUMN, 2), " ");
    let _ = std::fs::remove_dir_all(&config);
}

#[test]
fn the_floor_pattern_is_chosen_beside_the_colour_in_both_languages() {
    let config = folder("words");
    let mut harness = support::desk_in(&config, SIZE.0, SIZE.1);
    open_settings(&mut harness);
    let shown = harness.screen();
    for text in ["Floor pattern", "Plain", "Gradient", "Dots", "Both"] {
        assert!(shown.contains(text), "no `{text}`:\n{shown}");
    }
    harness.set_locale("tr").render();
    let shown = harness.screen();
    for text in ["Zemin deseni", "Düz", "Degrade", "Noktalı", "İkisi"] {
        assert!(shown.contains(text), "no `{text}`:\n{shown}");
    }
    let _ = std::fs::remove_dir_all(&config);
}
