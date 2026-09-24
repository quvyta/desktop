//! The Desktop folder on the floor, and the places icons are put in: its entries stand after the
//! applications, a folder opens in a window, a new folder is made from the floor's menu, and every
//! icon stays in the cell it was put in, across a restart.
//!
//! Every test starts where a person starts — a drag, two clicks, a right click, a name typed — and
//! every folder it shows or writes is one it made for itself in the temporary folder and takes
//! away again. The desktop is given a Desktop folder, a home, a `PATH` and a desktop file of the
//! test's own, so nothing of the person's is read or written, and no program of theirs is found.
//!
//! The desktop of a screen test follows its Desktop folder as the running desktop does, each wait
//! for a change bounded by its patience, so a file another program puts there is seen by stepping
//! the desktop.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::Desk;
use qdesk::apps::Environment;
use qdesk::desktop::Desktop;
use qframe::color::ColorDepth;
use qframe::env::{AssetDirs, Env};
use qframe::event::{MouseButton, MouseKind};
use qframe::icons::GlyphMode;
use qframe::prelude::*;

use support::{BUDGET, HARMLESS, ICONS, MACHINE, MOMENT, OFFSET, PATIENCE, decoration};

/// A folder of one test in the temporary folder, taken away with everything in it when the test
/// ends, whether it passed or not.
struct Scratch(PathBuf);

impl Scratch {
    /// A new folder no other test shares, with a Desktop folder in it holding a folder, a file
    /// and a hidden file.
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-desktop-folder-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        let scratch = Self(path);
        fs::create_dir_all(scratch.desktop().join("Projeler")).expect("a folder is made");
        fs::write(scratch.desktop().join("notlar.txt"), "bir iki üç\n").expect("a file is written");
        fs::write(scratch.desktop().join(".gizli"), "").expect("a hidden file is written");
        fs::create_dir_all(scratch.0.join("bin")).expect("the PATH folder is made");
        scratch
    }

    /// The Desktop folder of this test.
    fn desktop(&self) -> PathBuf {
        self.0.join("Masaüstü")
    }

    /// Where this test's desktop file is written.
    fn config(&self) -> PathBuf {
        self.0.join("config").join("desktop.toml")
    }

    /// The folder this test's settings file, `desktop.conf`, is read from and written to.
    fn settings(&self) -> PathBuf {
        self.0.join("config")
    }

    /// The environment of this test's desktop: its Desktop folder, its home, a `PATH` of one
    /// folder of its own, and a shell that ends at once.
    fn environment(&self) -> Environment {
        Environment {
            home: Some(self.0.clone()),
            data_home: Some(self.0.join("veri")),
            path: Some(self.0.join("bin").into_os_string()),
            shell: Some(PathBuf::from(HARMLESS)),
            desktop: Some(self.desktop()),
            ..Environment::default()
        }
    }

    /// Puts a program called `qexp` on this test's `PATH` that writes the folder it was given into
    /// the file it returns. It is the only `qexp` the desktop can find: the `PATH` is the test's.
    fn explorer(&self) -> PathBuf {
        let said = self.0.join("qexp-said");
        let program = self.0.join("bin").join("qexp");
        fs::write(&program, format!("#!/bin/sh\nprintf '%s' \"$1\" > '{}'\n", said.display()))
            .expect("the explorer is written");
        make_executable(&program);
        said
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn make_executable(program: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(program, fs::Permissions::from_mode(0o755)).expect("the explorer can be run");
}

fn env() -> Env {
    let dirs = AssetDirs {
        locale_sources: qdesk::locales().iter().map(|(file, text)| ((*file).to_owned(), (*text).to_owned())).collect(),
        keymap_source: Some({
            let (file, text) = qdesk::keymap();
            (file.to_owned(), text.to_owned())
        }),
        ..AssetDirs::default()
    };
    Env::load(&dirs).expect("the built-in files load")
}

/// The desktop of `scratch` with `desktop` as its file says it, writing that file where the
/// scratch keeps it, and reading and writing its settings there too.
fn desk_from(scratch: &Scratch, apps: Environment, desktop: Desktop, width: u16, height: u16) -> Harness<Desk> {
    let clock = Box::new(|| MOMENT * 1_000);
    let loaded = qdesk::settings::load_in(&scratch.settings());
    assert!(loaded.diagnostics.is_empty(), "the settings read cleanly: {:?}", loaded.diagnostics);
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), clock)
        .apps(apps)
        .catalog(support::catalog())
        .desktop(desktop)
        .settings(loaded.settings, loaded.prefs, loaded.diagnostics)
        .config(Some(scratch.config()))
        .watch_within(PATIENCE);
    let mut harness = Harness::with_env(app, env(), width, height);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);
    harness
}

/// The usual floor of these tests: Terminal, Settings and Midnight Commander, and the Desktop
/// folder of `scratch` after them.
fn desk(scratch: &Scratch) -> Harness<Desk> {
    let desktop = Desktop { icons: ICONS.map(str::to_owned).to_vec(), welcome_seen: true, ..Desktop::default() };
    desk_from(scratch, scratch.environment(), desktop, 80, 24)
}

/// The desktop as the file of `scratch` says it, read as qdesk reads it when it starts.
fn restarted(scratch: &Scratch) -> Harness<Desk> {
    let (desktop, problems) = Desktop::load(&scratch.config());
    assert!(problems.is_empty(), "the file reads back cleanly: {problems:?}");
    desk_from(scratch, scratch.environment(), desktop, 80, 24)
}

/// Two clicks on the icon of the floor named `name`, as a person opens it.
fn open_icon(harness: &mut Harness<Desk>, name: &str) {
    let (x, y) = harness.find(name).unwrap_or_else(|| panic!("no {name} on the floor:\n{}", harness.screen()));
    harness.click(x, y);
    harness.click(x, y);
}

/// Renders until `ready` is happy, or gives up after [`BUDGET`] and says what was on the screen.
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

/// A right click on the bare floor and a click on `row` of its menu.
fn floor_menu(harness: &mut Harness<Desk>, row: &str) {
    harness.mouse(MouseKind::Down(MouseButton::Right), 45, 2);
    assert!(harness.screen().contains(row), "the floor's menu offers {row}:\n{}", harness.screen());
    harness.click_text(row);
}

#[test]
fn the_entries_of_the_desktop_folder_stand_after_the_applications_folders_first() {
    let scratch = Scratch::new();
    let harness = desk(&scratch);
    assert_eq!(harness.find("Midnight"), Some((1, 7)), "the applications come first:\n{}", harness.screen());
    assert_eq!(harness.find("Projeler"), Some((1, 10)), "then the folder:\n{}", harness.screen());
    assert_eq!(harness.find("notlar.t"), Some((1, 13)), "then the file, its name cut to the cell");
    assert!(!harness.screen().contains("gizli"), "a hidden entry is not shown:\n{}", harness.screen());
    let icons = harness.env().icons();
    let folder = icons.glyph("folder").into_owned();
    // A text file is drawn with the icon of its kind, as a Files window draws it.
    let text = icons.glyph(qframe::icons::file_kind("notlar.txt", false, false).icon()).into_owned();
    let screen: Vec<String> = harness.screen().lines().map(str::to_owned).collect();
    assert!(screen[9].contains(&folder), "a folder is drawn with the folder icon: {:?}", screen[9]);
    assert!(screen[12].contains(&text), "a file with the icon of its kind: {:?}", screen[12]);
    assert_eq!(harness.app().floor_ids()[3..], ["file:Projeler", "file:notlar.txt"]);
}

#[test]
fn without_a_desktop_folder_the_floor_holds_the_applications_alone() {
    let scratch = Scratch::new();
    let apps = Environment { desktop: None, ..scratch.environment() };
    let desktop = Desktop { icons: ICONS.map(str::to_owned).to_vec(), welcome_seen: true, ..Desktop::default() };
    let mut harness = desk_from(&scratch, apps, desktop, 80, 24);
    assert_eq!(harness.find("Projeler"), None);
    harness.mouse(MouseKind::Down(MouseButton::Right), 45, 2);
    assert!(!harness.screen().contains("New folder"), "no folder to make one in:\n{}", harness.screen());
}

#[test]
fn two_clicks_on_a_folder_open_it_in_a_files_window_when_there_is_no_explorer() {
    let scratch = Scratch::new();
    let mut harness = desk(&scratch);
    open_icon(&mut harness, "Projeler");
    assert_eq!(harness.app().windows().len(), 1, "a window opened:\n{}", harness.screen());
    let id = harness.app().windows().front().expect("a window").id();
    let files = harness.app().files(id).expect("the window is a Files window");
    assert_eq!(files.manager.root(), scratch.desktop().join("Projeler"));
}

#[test]
fn two_clicks_on_a_folder_open_it_in_the_explorer_when_it_is_on_the_path() {
    let scratch = Scratch::new();
    let said = scratch.explorer();
    let mut harness = desk(&scratch);
    open_icon(&mut harness, "Projeler");
    let window = harness.app().windows().front().expect("a window");
    assert!(harness.app().files(window.id()).is_none(), "the window is the explorer's, not Files");
    assert_eq!(window.entry().name.get("en"), "Projeler", "the window is named after the folder");
    until(&mut harness, "the explorer's word", |_| said.is_file());
    let given = fs::read_to_string(&said).expect("the explorer wrote what it was given");
    assert_eq!(Path::new(&given), scratch.desktop().join("Projeler"), "it was given the folder");
}

/// The usual floor of these tests on a screen tall enough for the Settings window to show its
/// Desktop section whole.
fn tall_desk(scratch: &Scratch) -> Harness<Desk> {
    let desktop = Desktop { icons: ICONS.map(str::to_owned).to_vec(), welcome_seen: true, ..Desktop::default() };
    desk_from(scratch, scratch.environment(), desktop, 140, 44)
}

/// Opens Settings from its icon on the floor, clicks the switch of "Open folders with qexp" where
/// it is drawn, at the right edge of its row, and closes the window again from its title.
fn switch_explorer_in_settings(harness: &mut Harness<Desk>) {
    open_icon(harness, "Settings");
    let (_, row) = harness.find("Open folders with qexp").unwrap_or_else(|| panic!("the row:\n{}", harness.screen()));
    // A switch is colour alone; it stands at the right edge of its row, where the arrow of the
    // language drop-down stands too.
    let (edge, _) = harness.find("▾").expect("the language drop-down");
    harness.click(edge - 1, row);
    let (x, y) = harness.find("×").unwrap_or_else(|| panic!("the window's close mark:\n{}", harness.screen()));
    harness.click(x, y);
    assert!(harness.app().windows().is_empty(), "the Settings window closed:\n{}", harness.screen());
}

#[test]
fn folders_open_in_files_once_the_explorer_is_switched_off_in_settings() {
    let scratch = Scratch::new();
    let said = scratch.explorer();
    let mut harness = tall_desk(&scratch);
    assert!(harness.app().prefs().folders_in_explorer, "on until someone turns it off");
    switch_explorer_in_settings(&mut harness);
    open_icon(&mut harness, "Projeler");
    let screen = harness.screen();
    let id = harness.app().windows().front().unwrap_or_else(|| panic!("a window opened:\n{screen}")).id();
    let files = harness.app().files(id).unwrap_or_else(|| panic!("a Files window, not the explorer's:\n{screen}"));
    assert_eq!(files.manager.root(), scratch.desktop().join("Projeler"), "showing the folder");
    assert!(!said.exists(), "the explorer was not started");
}

#[test]
fn the_explorer_switched_off_stays_off_after_a_restart() {
    let scratch = Scratch::new();
    let said = scratch.explorer();
    let mut harness = tall_desk(&scratch);
    switch_explorer_in_settings(&mut harness);
    drop(harness);
    let written = fs::read_to_string(scratch.settings().join("desktop.conf")).expect("the settings file is written");
    assert!(written.contains("folders-in-explorer = false"), "{written}");

    let mut again = tall_desk(&scratch);
    open_icon(&mut again, "Projeler");
    let id = again.app().windows().front().unwrap_or_else(|| panic!("a window opened:\n{}", again.screen())).id();
    assert!(again.app().files(id).is_some(), "still a Files window after the restart:\n{}", again.screen());
    assert!(!said.exists(), "the explorer was not started");

    // Switched on again, the key leaves the file.
    let (x, y) = again.find("×").unwrap_or_else(|| panic!("the Files window's close mark:\n{}", again.screen()));
    again.click(x, y);
    switch_explorer_in_settings(&mut again);
    let written = fs::read_to_string(scratch.settings().join("desktop.conf")).expect("the settings file");
    assert!(!written.contains("folders-in-explorer"), "the default is not written:\n{written}");
}

#[test]
fn two_clicks_on_a_file_open_it_in_a_window_of_its_own() {
    let scratch = Scratch::new();
    let said = scratch.0.join("opened");
    let script = scratch.0.join("editor.sh");
    fs::write(&script, format!("printf '%s' \"$1\" > '{}'\n", said.display())).expect("the editor is written");
    let apps = Environment { editor: Some(format!("/bin/sh {}", script.display())), ..scratch.environment() };
    let desktop = Desktop { icons: ICONS.map(str::to_owned).to_vec(), welcome_seen: true, ..Desktop::default() };
    let mut harness = desk_from(&scratch, apps, desktop, 80, 24);
    open_icon(&mut harness, "notlar.t");
    until(&mut harness, "the editor's word", |_| said.is_file());
    let given = fs::read_to_string(&said).expect("the editor wrote what it was given");
    assert_eq!(Path::new(&given), scratch.desktop().join("notlar.txt"));
}

#[test]
fn a_new_folder_from_the_floor_s_menu_is_made_in_the_desktop_folder_and_stands_on_the_floor() {
    let scratch = Scratch::new();
    let mut harness = desk(&scratch);
    floor_menu(&mut harness, "New folder");
    assert!(harness.screen().contains("New folder"), "the dialog asks for its name:\n{}", harness.screen());
    // A name already taken is pointed out as it is typed, and nothing is made.
    harness.type_text("Projeler");
    assert!(harness.screen().contains("already"), "the name is taken:\n{}", harness.screen());
    harness.press("enter");
    for _ in 0.."Projeler".len() {
        harness.press("backspace");
    }
    harness.type_text("Arşiv");
    harness.press("enter");
    assert!(scratch.desktop().join("Arşiv").is_dir(), "the folder is on the disk:\n{}", harness.screen());
    // The folder is read again once the new one is made: work that follows other work comes a
    // step later.
    until(&mut harness, "the new folder on the floor", |harness| harness.find("Arşiv").is_some());
    assert_eq!(
        harness.find("Arşiv"),
        Some((3, 10)),
        "and on the floor, among the folders by name:\n{}",
        harness.screen()
    );
    // The keys are the floor's again: the arrows walk the icons.
    harness.press("down");
    assert_eq!(harness.buffer()[(0, 0)].symbol(), "▌", "{}", harness.screen());
}

#[test]
fn an_entry_renamed_from_its_menu_keeps_its_place() {
    let scratch = Scratch::new();
    let mut harness = desk(&scratch);
    harness.drag((3, 13), (45, 7));
    assert_eq!(harness.find("notlar.t"), Some((41, 7)), "{}", harness.screen());
    harness.mouse(MouseKind::Down(MouseButton::Right), 43, 7);
    harness.click_text("Rename");
    for _ in 0.."notlar.txt".chars().count() {
        harness.press("backspace");
    }
    harness.type_text("liste");
    harness.press("enter");
    assert!(scratch.desktop().join("liste").is_file(), "the file is renamed on the disk:\n{}", harness.screen());
    until(&mut harness, "the new name on the floor", |harness| harness.find("liste").is_some());
    assert_eq!(harness.find("liste"), Some((43, 7)), "and stands where the old name stood:\n{}", harness.screen());
    assert_eq!(harness.app().order().places.get("file:liste"), Some(&(4, 2)));
    assert!(!harness.app().order().places.contains_key("file:notlar.txt"));
}

#[test]
fn an_icon_put_on_bare_floor_is_there_again_after_a_restart() {
    let scratch = Scratch::new();
    let mut harness = desk(&scratch);
    harness.drag((3, 1), (45, 7));
    harness.drag((3, 10), (65, 16));
    assert_eq!(harness.find("Terminal"), Some((41, 7)));
    assert_eq!(harness.find("Projeler"), Some((61, 16)), "{}", harness.screen());
    drop(harness);
    let written = fs::read_to_string(scratch.config()).expect("the desktop file was written");
    assert!(written.contains("[places]"), "{written}");
    let again = restarted(&scratch);
    assert_eq!(again.find("Terminal"), Some((41, 7)), "{}", again.screen());
    assert_eq!(again.find("Projeler"), Some((61, 16)), "{}", again.screen());
    assert_eq!(again.find("Settings"), Some((1, 4)), "an icon that was not moved stays too");
}

#[test]
fn a_desktop_file_of_0_1_with_only_the_order_still_opens_and_its_icons_flow() {
    let scratch = Scratch::new();
    fs::create_dir_all(scratch.config().parent().expect("a folder")).expect("the folder is made");
    fs::write(
        scratch.config(),
        "# The desktop of qdesk. Written when something changes.\n\nicons = [\"mc\", \"terminal\"]\n\nrecents = []\n\nwelcome_seen = true\n",
    )
    .expect("an old file is written");
    let harness = restarted(&scratch);
    assert_eq!(harness.find("Midnight"), Some((1, 1)), "{}", harness.screen());
    assert_eq!(harness.find("Terminal"), Some((1, 4)));
    assert_eq!(harness.find("Projeler"), Some((1, 7)), "the Desktop folder flows after them");
}

#[test]
fn the_desktop_folder_draws_without_decoration_in_every_glyph_mode_and_colour_depth() {
    let scratch = Scratch::new();
    for mode in [GlyphMode::Nerd, GlyphMode::Unicode, GlyphMode::Ascii] {
        for depth in [ColorDepth::Ansi16, ColorDepth::Ansi256, ColorDepth::TrueColor] {
            let mut harness = desk(&scratch);
            harness.set_glyph_mode(mode).set_depth(depth);
            harness.mouse(MouseKind::Down(MouseButton::Left), 3, 10);
            harness.mouse(MouseKind::Drag(MouseButton::Left), 45, 7);
            let screen = harness.screen();
            assert!(screen.contains("Projeler"), "{mode:?} {depth:?}:\n{screen}");
            assert_eq!(decoration(&screen), None, "{mode:?} {depth:?}:\n{screen}");
            if mode == GlyphMode::Ascii {
                assert!(screen.replace('…', "...").is_ascii(), "{depth:?}:\n{screen}");
            }
            harness.mouse(MouseKind::Up(MouseButton::Left), 45, 7);
        }
    }
}

#[test]
fn a_file_another_program_puts_in_the_desktop_folder_comes_to_the_floor() {
    let scratch = Scratch::new();
    let mut harness = desk(&scratch);
    assert!(!harness.screen().contains("yeni.md"), "not there yet:\n{}", harness.screen());
    fs::write(scratch.desktop().join("yeni.md"), "# yeni\n").expect("a file is written");
    until(&mut harness, "the new file on the floor", |harness| harness.screen().contains("yeni.md"));
}

#[test]
fn an_entry_on_the_floor_carries_the_icon_of_its_kind() {
    let scratch = Scratch::new();
    fs::write(scratch.desktop().join("main.rs"), "fn main() {}\n").expect("a file is written");
    let mut harness = desk(&scratch);
    harness.set_glyph_mode(GlyphMode::Nerd).render();
    let icons = harness.env().icons();
    let (rust, plain) = (icons.glyph("file-rust").into_owned(), icons.glyph("file").into_owned());
    assert_ne!(rust, plain, "the kinds are told apart in a Nerd Font");
    let screen = harness.screen();
    let (_, y) = harness.find("main.rs").unwrap_or_else(|| panic!("the file is on the floor:\n{screen}"));
    // The glyph stands on the row above the name, in the icon's cell.
    let above = screen.lines().nth(usize::try_from(y - 1).expect("a row")).expect("the glyph's row");
    assert!(above.contains(&rust), "the Rust file carries the Rust icon:\n{screen}");
}
