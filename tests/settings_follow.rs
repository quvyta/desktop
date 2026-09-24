//! A running desktop follows its settings file: what another program writes into `desktop.conf`
//! (`qdesk wallpaper`, the explorer's "Set as desktop background") or a person saves by hand is
//! drawn without a restart, and a file broken by hand is said.
//!
//! The file is written here the way the command writes it, through qdesk's own settings code, into
//! a folder of the test's own. The desktop watches the folder as the running one does, each wait
//! bounded by its patience, and the test draws frames until the change is on screen or its
//! budget is spent.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::Desk;
use qdesk::settings::{self, FloorColor, FloorStyle};
use qframe::prelude::*;

use support::BUDGET;

/// The terminal these tests draw on.
const SIZE: (u16, u16) = (140, 44);

/// A cell of floor no icon stands on: the far corner above the dock.
const FLOOR_CELL: (u16, u16) = (SIZE.0 - 1, SIZE.1 - 2);

/// The configuration folder of one test in the temporary folder, taken away when the test ends.
struct Config(PathBuf);

impl Config {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-settings-follow-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the folder is made");
        Self(path)
    }

    fn file(&self) -> PathBuf {
        self.0.join("desktop.conf")
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Writes the floor into the settings file in `config` as `qdesk wallpaper` does.
fn wallpaper(config: &Path, color: Option<FloorColor>, pattern: Option<FloorStyle>) {
    let mut loaded = settings::load_in(config);
    settings::set_floor(&mut loaded.settings, color, pattern);
    loaded.settings.save().expect("the settings file is written");
}

/// Draws frames until `ready` is happy, or gives up after [`BUDGET`] and says what was on screen.
fn until(harness: &mut Harness<Desk>, what: &str, ready: impl Fn(&Harness<Desk>) -> bool) {
    let deadline = Instant::now() + BUDGET;
    loop {
        if ready(harness) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} never came:\n{}", harness.screen());
        harness.render();
    }
}

#[test]
fn a_floor_written_into_the_settings_file_while_the_desktop_runs_is_drawn_at_once() {
    let config = Config::new();
    let mut harness = support::desk_in(&config.0, SIZE.0, SIZE.1);
    let theme = harness.env().theme();
    let canvas = theme.color("canvas");
    let deep = FloorColor::Deep.in_theme(theme);
    assert_ne!(deep, canvas, "the tone is not the theme's own");
    assert_eq!(harness.bg(FLOOR_CELL.0, FLOOR_CELL.1), canvas, "the floor starts plain");

    wallpaper(&config.0, Some(FloorColor::Deep), None);
    until(&mut harness, "the deep floor", |harness| harness.bg(FLOOR_CELL.0, FLOOR_CELL.1) == deep);
    assert_eq!(harness.app().prefs().floor, FloorColor::Deep);

    // The pattern alone, the colour kept.
    wallpaper(&config.0, None, Some(FloorStyle::Dots));
    until(&mut harness, "the dotted floor", |harness| harness.app().prefs().floor_style == FloorStyle::Dots);
    assert_eq!(harness.app().prefs().floor, FloorColor::Deep);
    assert!(harness.screen().contains('\u{b7}'), "the dots are drawn:\n{}", harness.screen());
}

#[test]
fn a_file_broken_by_hand_is_said_and_the_floor_follows_it_again_once_it_is_mended() {
    let config = Config::new();
    let mut harness = support::desk_in(&config.0, SIZE.0, SIZE.1);
    let canvas = harness.env().theme().color("canvas");
    let mist = FloorColor::Mist.in_theme(harness.env().theme());

    fs::write(config.file(), "floor-color = \"tartan\"\n").expect("file");
    until(&mut harness, "the problem in the corner", |harness| {
        harness.screen().contains("The settings file could not be read")
    });
    assert!(harness.screen().contains("tartan"), "the value is named:\n{}", harness.screen());
    assert_eq!(harness.app().prefs().floor, FloorColor::Theme, "what cannot be used keeps the default");
    assert_eq!(harness.bg(FLOOR_CELL.0, FLOOR_CELL.1), canvas);
    assert_eq!(fs::read_to_string(config.file()).expect("file"), "floor-color = \"tartan\"\n", "never repaired");

    fs::write(config.file(), "floor-color = \"mist\"\n").expect("file");
    until(&mut harness, "the mended floor", |harness| harness.bg(FLOOR_CELL.0, FLOOR_CELL.1) == mist);
}

#[test]
fn the_theme_in_the_settings_file_is_applied_at_start_and_when_the_file_changes() {
    let config = Config::new();
    fs::write(config.file(), "theme = \"amber\"\n").expect("file");
    let mut harness = support::desk_in(&config.0, SIZE.0, SIZE.1);
    assert_eq!(harness.env().theme().id(), "amber", "the saved theme is the first frame's");

    fs::write(config.file(), "theme = \"nordic\"\n").expect("file");
    until(&mut harness, "the new theme", |harness| harness.env().theme().id() == "nordic");
}

#[test]
fn a_choice_made_on_the_settings_screen_is_not_undone_by_its_own_write() {
    let config = Config::new();
    let mut harness = support::desk_in(&config.0, SIZE.0, SIZE.1);
    let (x, y) = harness.find("Settings").expect("the Settings icon is on the floor");
    harness.click(x, y);
    harness.click(x, y);
    harness.click_text("Deep");
    assert_eq!(harness.app().prefs().floor, FloorColor::Deep);
    let written = fs::read_to_string(config.file()).expect("the choice is written");
    for _ in 0..20 {
        harness.render();
    }
    assert_eq!(harness.app().prefs().floor, FloorColor::Deep, "the desktop's own write changes nothing");
    assert_eq!(fs::read_to_string(config.file()).expect("file"), written, "reading the file never writes it");
}
