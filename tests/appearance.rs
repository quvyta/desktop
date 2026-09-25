//! The desktop's look is the Quvyta ecosystem's: the language, the theme, the icons and reduced
//! motion are shared with every Quvyta application unless the person chose them for the desktop
//! alone, on the framework's own appearance section of the Settings screen.
//!
//! Each test starts the desktop the way `qdesk` starts, over a folder of the test's own standing
//! for the ecosystem's (`~/.config/quvyta`): the settings an older qdesk wrote are settled, and the
//! runtime starts the desktop as a member of the ecosystem. What a person does is done where they
//! do it, on the Settings window opened from its icon; what another application does is a file
//! written in the folder, read again as the runtime reads it when its watch hears the change.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use qdesk::app::Desk;
use qdesk::settings::FloorColor;
use qframe::color::Rgb;
use qframe::prelude::*;
use qframe::storage::{Ecosystem, Scope, Shared, Source};
use qframe::theme::ThemeRegistry;

/// A terminal tall enough for the whole appearance section in the Settings window.
const SIZE: (u16, u16) = (140, 60);

/// A cell of floor no icon and no window stands on: the far corner above the dock.
const FLOOR_CELL: (u16, u16) = (SIZE.0 - 1, SIZE.1 - 2);

/// The label of the box under each shared row.
const EVERYWHERE: &str = "In every Quvyta application";

/// The ecosystem's folder of one test in the temporary folder, taken away when the test ends.
struct Config(PathBuf);

impl Config {
    /// A folder whose shared file says `theme`, in English, with Unicode icons and reduced motion,
    /// as another Quvyta application left it; `desktop` is what `desktop.conf` says, if anything.
    fn new(theme: &str, desktop: Option<&str>) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-appearance-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the folder is made");
        let shared = format!("language = \"en\"\ntheme = \"{theme}\"\nicons = \"unicode\"\nreduced-motion = true\n");
        fs::write(path.join("quvyta.conf"), shared).expect("the shared file");
        if let Some(text) = desktop {
            fs::write(path.join("desktop.conf"), text).expect("the desktop's file");
        }
        Self(path)
    }

    fn read(&self, name: &str) -> String {
        fs::read_to_string(self.0.join(name)).unwrap_or_default()
    }
}

impl Drop for Config {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The desktop started over `config`, with its Settings window opened from the icon on the floor.
fn settings_open(config: &Path) -> Harness<Desk> {
    let mut harness = support::member_in(config, SIZE.0, SIZE.1);
    let (x, y) = harness.find("Settings").expect("the Settings icon is on the floor");
    harness.click(x, y);
    harness.click(x, y);
    assert_eq!(harness.app().windows().len(), 1, "the window opened:\n{}", harness.screen());
    assert!(harness.screen().contains(EVERYWHERE), "the appearance section is shown:\n{}", harness.screen());
    harness
}

/// The row the Theme choice is drawn on.
fn theme_row(harness: &Harness<Desk>) -> String {
    let screen = harness.screen();
    screen.lines().find(|line| line.contains("Theme")).map(str::to_owned).expect("a Theme row")
}

/// The first cell of the box under the `nth` shared row (0 language, 1 theme, 2 icons): the first
/// cell right of its label whose ground is not the window's.
fn shared_box(harness: &Harness<Desk>, nth: usize) -> (u16, u16) {
    let screen = harness.screen();
    let y = screen.lines().enumerate().filter(|(_, line)| line.contains(EVERYWHERE)).nth(nth).expect("the box's row").0;
    let y = u16::try_from(y).expect("on screen");
    let (label, _) = harness.find(EVERYWHERE).expect("the label");
    let after = u16::try_from(label).expect("on screen") + u16::try_from(EVERYWHERE.chars().count()).expect("short");
    let ground = harness.bg(after, y);
    let x = (after..SIZE.0).find(|x| harness.bg(*x, y) != ground).expect("a box right of the label");
    (x, y)
}

/// Whether the box under the theme row is drawn checked: filled as the box under the language
/// row is, which follows the ecosystem in every test here since only the shared file names it.
fn theme_box_checked(harness: &Harness<Desk>) -> bool {
    let (x, y) = shared_box(harness, 1);
    let (lx, ly) = shared_box(harness, 0);
    harness.bg(x, y) == harness.bg(lx, ly)
}

/// The floor's colour as the built-in theme `id` draws it.
fn floor_in(id: &str) -> Option<Rgb> {
    let (theme, _) = ThemeRegistry::builtin().resolve_or_default(id);
    assert_eq!(theme.id(), id, "a built-in theme");
    FloorColor::Theme.in_theme(&theme)
}

/// Opens the theme's drop-down where it shows `shown` and picks `name` in its list, as a person
/// does with the mouse.
fn choose_theme(harness: &mut Harness<Desk>, shown: &str, name: &str) {
    let row = theme_row(harness);
    let y = harness.screen().lines().position(|line| line == row).expect("the Theme row");
    let x = row.find(shown).map(|at| row[..at].chars().count()).expect("the theme in force on its row");
    harness.click(i32::try_from(x).expect("on screen"), i32::try_from(y).expect("on screen"));
    harness.click_text(name);
}

#[test]
fn a_theme_an_older_qdesk_wrote_that_the_ecosystem_shares_follows_the_ecosystem_from_now_on() {
    // qdesk 0.1.9 wrote the theme chosen on its Settings screen into its own file; the ecosystem's
    // shared file says the same theme.
    let config = Config::new("iris", Some("theme = \"iris\"\n"));
    let harness = settings_open(&config.0);
    assert_eq!(harness.app().appearance().preferences().source(Shared::Theme), Source::Ecosystem);
    assert!(theme_box_checked(&harness), "the box under the theme is checked:\n{}", harness.screen());
    assert!(config.read("desktop.conf").contains("theme = \"quvyta\""), "{}", config.read("desktop.conf"));
    assert_eq!(harness.env().theme().id(), "iris");
    assert!(theme_row(&harness).contains("Iris"), "{}", harness.screen());
    assert_eq!(harness.bg(FLOOR_CELL.0, FLOOR_CELL.1), floor_in("iris"), "the floor is drawn in iris");
}

#[test]
fn a_theme_an_older_qdesk_wrote_that_differs_from_the_ecosystems_stays_the_desktops_own() {
    let config = Config::new("nordic", Some("theme = \"iris\"\n"));
    let harness = settings_open(&config.0);
    assert_eq!(harness.app().appearance().preferences().source(Shared::Theme), Source::App);
    assert!(!theme_box_checked(&harness), "the box under the theme is clear:\n{}", harness.screen());
    assert!(config.read("desktop.conf").contains("theme = \"iris\""), "{}", config.read("desktop.conf"));
    assert_eq!(harness.env().theme().id(), "iris", "the desktop keeps its own theme");
    assert!(theme_row(&harness).contains("Iris"), "{}", harness.screen());
    assert_ne!(floor_in("iris"), floor_in("nordic"));
    assert_eq!(harness.bg(FLOOR_CELL.0, FLOOR_CELL.1), floor_in("iris"), "the floor is drawn in iris");
}

#[test]
fn a_theme_chosen_with_the_box_checked_goes_to_every_quvyta_application() {
    let config = Config::new("iris", None);
    let mut harness = settings_open(&config.0);
    assert!(theme_box_checked(&harness), "{}", harness.screen());
    choose_theme(&mut harness, "Iris", "Amber");
    assert_eq!(harness.env().theme().id(), "amber", "the theme changes at once:\n{}", harness.screen());
    assert!(config.read("quvyta.conf").contains("theme = \"amber\""), "{}", config.read("quvyta.conf"));
    assert!(!config.read("desktop.conf").contains("amber"), "{}", config.read("desktop.conf"));
    assert_eq!(harness.bg(FLOOR_CELL.0, FLOOR_CELL.1), floor_in("amber"));
}

#[test]
fn a_theme_chosen_with_the_box_cleared_stays_on_the_desktop() {
    let config = Config::new("iris", None);
    let mut harness = settings_open(&config.0);
    let (x, y) = shared_box(&harness, 1);
    harness.click(i32::from(x), i32::from(y));
    assert!(!theme_box_checked(&harness), "the box is cleared:\n{}", harness.screen());
    choose_theme(&mut harness, "Iris", "Amber");
    assert_eq!(harness.env().theme().id(), "amber", "{}", harness.screen());
    assert!(config.read("desktop.conf").contains("theme = \"amber\""), "{}", config.read("desktop.conf"));
    assert!(config.read("quvyta.conf").contains("theme = \"iris\""), "{}", config.read("quvyta.conf"));

    // The next start keeps it.
    let again = settings_open(&config.0);
    assert_eq!(again.env().theme().id(), "amber");
    assert!(!theme_box_checked(&again), "{}", again.screen());
}

#[test]
fn a_theme_another_application_shares_is_shown_by_the_open_desktop_and_taken_as_it_is() {
    let config = Config::new("iris", None);
    let mut harness = settings_open(&config.0);
    Ecosystem::QUVYTA
        .set_in(&config.0, "code", Shared::Theme, "nordic", Scope::Ecosystem)
        .expect("another application");
    harness.poll_preferences();
    assert_eq!(harness.env().theme().id(), "nordic");
    assert!(theme_row(&harness).contains("Nordic"), "{}", harness.screen());
    assert_eq!(harness.bg(FLOOR_CELL.0, FLOOR_CELL.1), floor_in("nordic"));

    // The section heard it too: keeping the theme for the desktop alone keeps the one on screen.
    let (x, y) = shared_box(&harness, 1);
    harness.click(i32::from(x), i32::from(y));
    assert!(config.read("desktop.conf").contains("theme = \"nordic\""), "{}", config.read("desktop.conf"));
    harness.poll_preferences();
    assert_eq!(harness.env().theme().id(), "nordic", "{}", harness.screen());
    assert!(!theme_box_checked(&harness), "{}", harness.screen());
}
