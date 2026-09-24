//! The Files window: Quvyta's shared file manager in a window of the desktop.
//!
//! Every test starts where a person starts — two clicks on the Files icon of the floor, a click on
//! a row, a right click on the window's item on the dock — and every folder it shows is one the
//! test made for itself in the temporary folder and takes away again. No test looks at the
//! person's home folder, trash or programs: the desktop is given a home, a data folder and a shell
//! of the test's own.
//!
//! The desktop of a screen test follows the folders it shows, as the running desktop does, each
//! wait for a change bounded by its patience: a test runs the wait where it stands, so a file
//! another program makes in the folder is seen by stepping the desktop.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use qdesk::app::Desk;
use qdesk::apps::{Catalog, Declared, Entry, Environment, Folders, Source, load, parse_entry};
use qdesk::desktop::Desktop;
use qdesk::wm::{Window, WindowId};
use qframe::color::ColorDepth;
use qframe::env::{AssetDirs, Env};
use qframe::event::{MouseButton, MouseKind};
use qframe::icons::GlyphMode;
use qframe::prelude::*;
use qframe::widgets::FileView;

use support::{BUDGET, HARMLESS, MACHINE, MOMENT, OFFSET, PATIENCE, decoration};

/// A moment of the fake clock with no input in it: what the list waits for before it reads the
/// details of the rows it shows.
const STILL: Duration = Duration::from_millis(1);

/// A folder of one test in the temporary folder, taken away with everything in it when the test
/// ends, whether it passed or not.
struct Scratch(PathBuf);

impl Scratch {
    /// A new, empty folder no other test shares: tests of one binary are threads of one process,
    /// so the process id alone would be shared.
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-files-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the scratch folder is made");
        Self(path)
    }

    /// The home folder of the desktop of this test, with two folders and a file in it.
    fn home(&self) -> PathBuf {
        self.0.join("ev")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Makes the home folder of `scratch`: two folders, one with a file in it, and a file.
fn furnish(scratch: &Scratch) {
    let home = scratch.home();
    fs::create_dir_all(home.join("belgeler")).expect("a folder is made");
    fs::create_dir_all(home.join("projeler")).expect("a folder is made");
    fs::write(home.join("belgeler").join("mektup.txt"), "merhaba\n").expect("a file is written");
    fs::write(home.join("notlar.txt"), "bir iki üç\n").expect("a file is written");
}

/// The entries qdesk carries inside it, with `extra` beside them.
fn catalog(extra: Vec<Entry>) -> Catalog {
    let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
    let mut entries = load(&folders, None).entries;
    entries.extend(extra);
    Catalog::new(entries, |_| true)
}

/// An entry of this test, written as a person writes one.
fn entry(id: &str, text: &str) -> Entry {
    let file = format!("/nowhere/{id}.toml");
    let (declared, diagnostics) = parse_entry(id, Path::new(&file), text.as_bytes(), Source::User, None);
    assert!(diagnostics.is_empty(), "{id}: {diagnostics:?}");
    match declared {
        Some(Declared::Entry(entry)) => *entry,
        other => panic!("{id} declares no entry: {other:?}"),
    }
}

/// The desktop's environment in this test: the scratch home, a data folder beside it for the
/// trash, and a shell that ends at once.
fn environment(scratch: &Scratch) -> Environment {
    Environment {
        home: Some(scratch.home()),
        data_home: Some(scratch.0.join("veri")),
        shell: Some(PathBuf::from(HARMLESS)),
        ..Environment::default()
    }
}

/// A desktop of `width` by `height` with `icons` on its floor, in `environment`.
fn desk_in(environment: Environment, extra: Vec<Entry>, icons: &[&str], width: u16, height: u16) -> Harness<Desk> {
    let dirs = AssetDirs {
        locale_sources: qdesk::locales().iter().map(|(file, text)| ((*file).to_owned(), (*text).to_owned())).collect(),
        keymap_source: Some({
            let (file, text) = qdesk::keymap();
            (file.to_owned(), text.to_owned())
        }),
        ..AssetDirs::default()
    };
    let env = Env::load(&dirs).expect("the built-in files load");
    let desktop = Desktop {
        icons: icons.iter().map(|id| (*id).to_owned()).collect(),
        recents: Vec::new(),
        welcome_seen: true,
        ..Desktop::default()
    };
    let clock = Box::new(|| MOMENT * 1_000);
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), clock)
        .apps(environment)
        .catalog(catalog(extra))
        .desktop(desktop)
        .watch_within(PATIENCE);
    let mut harness = Harness::with_env(app, env, width, height);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);
    harness
}

/// The usual desktop of these tests: Terminal, Files and Settings on the floor of a wide screen.
fn desk(scratch: &Scratch) -> Harness<Desk> {
    desk_in(environment(scratch), Vec::new(), &["terminal", "files", "settings"], 120, 32)
}

/// The window in front.
fn front(harness: &Harness<Desk>) -> &Window {
    harness.app().windows().front().expect("a window on the desktop")
}

fn front_id(harness: &Harness<Desk>) -> WindowId {
    front(harness).id()
}

/// Two clicks on the icon of the floor named `name`, as a person opens an application.
fn open_icon(harness: &mut Harness<Desk>, name: &str) {
    let (x, y) = harness.find(name).unwrap_or_else(|| panic!("no {name} on the floor:\n{}", harness.screen()));
    harness.click(x, y);
    harness.click(x, y);
}

/// Where the row of the folder window that says `name` is: the first place inside the window in
/// front that shows it.
fn row_of(harness: &Harness<Desk>, name: &str) -> (i32, i32) {
    let rect = front(harness).rect();
    let screen = harness.screen();
    for (y, line) in screen.lines().enumerate() {
        let y = i32::try_from(y).expect("a row on screen");
        if y <= rect.y || y >= rect.y + i32::from(rect.height) {
            continue;
        }
        if let Some(at) = line.find(name) {
            let x = i32::from(qframe::text::width(&line[..at]));
            if x > rect.x && x < rect.x + i32::from(rect.width) {
                return (x, y);
            }
        }
    }
    panic!("no row says {name} in the window:\n{screen}");
}

/// A click on the close mark of the window in front, on its own title strip.
fn close_front(harness: &mut Harness<Desk>) {
    let rect = front(harness).rect();
    let screen = harness.screen();
    let strip = screen.lines().nth(usize::try_from(rect.y).expect("a row")).expect("the strip is on screen");
    let at = strip.rfind('×').unwrap_or_else(|| panic!("no close mark on the strip: {strip}"));
    let x = i32::from(qframe::text::width(&strip[..at]));
    harness.click(x, rect.y);
}

/// The window item of the dock that says `name`: its place on the dock's row.
fn dock_cell(harness: &Harness<Desk>, name: &str) -> (i32, i32) {
    let screen = harness.screen();
    let rows: Vec<&str> = screen.lines().collect();
    let row = rows.len() - 1;
    let at = rows[row].find(name).unwrap_or_else(|| panic!("no {name} on the dock: {}", rows[row]));
    let x = i32::from(qframe::text::width(&rows[row][..at]));
    (x, i32::try_from(row).expect("a row"))
}

/// Opens the menu of the window item `name` on the dock and chooses `row` in it.
fn window_menu(harness: &mut Harness<Desk>, name: &str, row: &str) {
    let (x, y) = dock_cell(harness, name);
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    assert!(harness.screen().contains(row), "the window's menu offers {row}:\n{}", harness.screen());
    harness.click_text(row);
}

#[test]
fn two_clicks_on_the_files_icon_show_the_home_folder_as_a_list() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let mut harness = desk(&scratch);
    open_icon(&mut harness, "Files");
    assert_eq!(harness.app().windows().len(), 1, "a window opened:\n{}", harness.screen());
    let id = front_id(&harness);
    let files = harness.app().files(id).expect("the window holds a file manager");
    assert_eq!(files.manager.root(), scratch.home(), "it shows the desktop's home folder");
    assert_eq!(files.view, FileView::List);
    let screen = harness.screen();
    for name in ["belgeler", "projeler", "notlar.txt"] {
        assert!(screen.contains(name), "{name} is listed:\n{screen}");
    }
    // A list asks for sizes, dates and permissions once the screen is still, a page of rows at a
    // time, as a person's screen is a moment after the window opened.
    harness.advance(STILL);
    let screen = harness.screen();
    // A list says how big a file is; neither a tree nor the icons do.
    assert!(screen.contains("13 B"), "the list shows the file's size:\n{screen}");
}

#[test]
fn a_double_click_on_a_folder_row_goes_into_it() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let mut harness = desk(&scratch);
    open_icon(&mut harness, "Files");
    let (x, y) = row_of(&harness, "belgeler");
    harness.click(x, y).click(x, y);
    let screen = harness.screen();
    assert!(screen.contains("mektup.txt"), "the folder's own file is shown:\n{screen}");
    assert!(!screen.contains("notlar.txt"), "the home folder's file is not:\n{screen}");
    // The strip says where the window is, as a Terminal window's strip says where its shell is.
    assert!(screen.contains("belgeler"), "the strip names the folder:\n{screen}");
}

#[test]
fn a_single_click_on_a_row_only_chooses_it_as_a_file_explorer_does() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let mut harness = desk(&scratch);
    open_icon(&mut harness, "Files");
    let (x, y) = row_of(&harness, "belgeler");
    harness.click(x, y);
    let screen = harness.screen();
    assert!(screen.contains("notlar.txt"), "the window stays in the home folder:\n{screen}");
    assert!(!screen.contains("mektup.txt"), "the folder is not gone into:\n{screen}");
    let (x, y) = row_of(&harness, "notlar.txt");
    harness.click(x, y);
    assert_eq!(harness.app().windows().len(), 1, "no window opens for the file:\n{}", harness.screen());
}

#[test]
fn an_entry_that_names_a_folder_opens_a_files_window_on_it() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let folder = scratch.home().join("belgeler");
    let logs = entry("belgeler", &format!("name = \"Letters\"\nopen = \"{}\"\n", folder.display()));
    let mut harness = desk_in(environment(&scratch), vec![logs], &["belgeler"], 120, 32);
    open_icon(&mut harness, "Letters");
    let id = front_id(&harness);
    let files = harness.app().files(id).expect("the folder opened in a Files window");
    assert_eq!(files.manager.root(), folder);
    assert!(harness.screen().contains("mektup.txt"), "its file is listed:\n{}", harness.screen());
}

#[test]
fn an_entry_that_names_a_file_still_says_there_is_no_viewer() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let file = scratch.home().join("notlar.txt");
    let note = entry("not", &format!("name = \"Note\"\nopen = \"{}\"\n", file.display()));
    let mut harness = desk_in(environment(&scratch), vec![note], &["not"], 120, 32);
    open_icon(&mut harness, "Note");
    assert!(harness.app().windows().is_empty(), "no window opens onto a file:\n{}", harness.screen());
    assert!(harness.screen().contains("no viewer"), "the corner says why:\n{}", harness.screen());
}

#[test]
fn the_window_s_menu_draws_the_folder_as_a_tree_or_as_icons_and_marks_the_shape_in_use() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let mut harness = desk(&scratch);
    open_icon(&mut harness, "Files");
    let id = front_id(&harness);
    let (x, y) = dock_cell(&harness, "Files");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    let menu = harness.screen();
    let marked = menu.lines().find(|line| line.contains("Show as a list")).expect("the menu offers the list");
    assert!(marked.contains('✓'), "the shape in use carries a sign:\n{menu}");
    let other = menu.lines().find(|line| line.contains("Show as a tree")).expect("the menu offers the tree");
    assert!(!other.contains('✓'), "only the shape in use is marked:\n{menu}");
    harness.click_text("Show as a tree");
    assert_eq!(harness.app().files(id).map(|files| files.view), Some(FileView::Tree));
    // The tree shows the folder itself as its top row, named, with its entries under it; the size
    // column of the list is gone.
    let screen = harness.screen();
    assert!(screen.contains("ev"), "the root row names the home folder:\n{screen}");
    assert!(!screen.contains("13 B"), "a tree shows no sizes:\n{screen}");
    window_menu(&mut harness, "Files", "Show as icons");
    assert_eq!(harness.app().files(id).map(|files| files.view), Some(FileView::Icons));
    assert!(harness.screen().contains("notlar.txt"), "the icons name the entries:\n{}", harness.screen());
}

#[test]
fn each_files_window_keeps_its_own_folder_and_shape_and_lets_it_go_when_it_closes() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let mut harness = desk(&scratch);
    open_icon(&mut harness, "Files");
    let first = front_id(&harness);
    let (x, y) = row_of(&harness, "belgeler");
    harness.click(x, y).click(x, y);
    open_icon(&mut harness, "Files");
    let second = front_id(&harness);
    assert_ne!(first, second, "Files opens as many windows as are asked for");
    let folder = |harness: &Harness<Desk>, id| harness.app().files(id).map(|files| files.manager.folder().to_owned());
    assert_eq!(folder(&harness, first).as_deref(), Some("belgeler"));
    assert_eq!(folder(&harness, second).as_deref(), Some(""), "the new window starts at home");
    close_front(&mut harness);
    assert!(harness.app().files(second).is_none(), "the closed window's manager is gone");
    assert!(harness.app().files(first).is_some(), "the other window keeps its own");
}

#[test]
fn moving_a_file_to_the_trash_puts_it_in_the_desktop_s_own_data_folder() {
    // The trash is the one under the data folder the desktop was given; a test's is inside its
    // scratch folder, so no test ever puts anything in the person's own trash.
    let scratch = Scratch::new();
    furnish(&scratch);
    let mut harness = desk(&scratch);
    open_icon(&mut harness, "Files");
    let (x, y) = row_of(&harness, "notlar.txt");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    assert!(harness.screen().contains("Move to the trash"), "the row's menu offers the trash:\n{}", harness.screen());
    harness.click_text("Move to the trash").advance(STILL);
    assert!(!scratch.home().join("notlar.txt").exists(), "the file left its folder:\n{}", harness.screen());
    assert!(scratch.0.join("veri/Trash/files/notlar.txt").is_file(), "it is in the desktop's trash");
}

/// A shell that says the folder it was started in and ends: a program every machine has, which
/// reads nothing and writes nothing, so a test learns where a terminal started from its screen.
const PWD: &str = "/bin/pwd";

/// Renders until `ready` is happy, or gives up after [`BUDGET`] and says what was on the screen.
/// Every render runs one bounded wait for a program's next word, so this is how a test steps a
/// program through the desktop.
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

/// A right click on the row `name` of the window in front, and a click on `row` of its menu.
fn row_menu(harness: &mut Harness<Desk>, name: &str, row: &str) {
    let (x, y) = row_of(harness, name);
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    assert!(harness.screen().contains(row), "the row's menu offers {row}:\n{}", harness.screen());
    harness.click_text(row);
}

/// The line of the window in front that shows the row `name`.
fn row_line(harness: &Harness<Desk>, name: &str) -> String {
    let (_, y) = row_of(harness, name);
    harness.screen().lines().nth(usize::try_from(y).expect("a row")).expect("the row is on screen").to_owned()
}

/// Brings the window whose dock item says `name` to the front with a click on that item.
fn bring(harness: &mut Harness<Desk>, name: &str) {
    let (x, y) = dock_cell(harness, name);
    harness.click(x, y);
}

#[test]
fn a_double_click_on_a_file_opens_it_in_the_person_s_editor_in_a_window_of_its_own() {
    let scratch = Scratch::new();
    furnish(&scratch);
    // The editor is the test's own: a script read by the system's shell, which writes down the
    // file it was given. It is read, never run as a program of its own, so nothing is executed
    // that this test has just written.
    let said = scratch.0.join("opened");
    let script = scratch.0.join("editor.sh");
    fs::write(&script, format!("printf '%s' \"$1\" > '{}'\n", said.display())).expect("the editor is written");
    let apps = Environment { editor: Some(format!("/bin/sh {}", script.display())), ..environment(&scratch) };
    let mut harness = desk_in(apps, Vec::new(), &["files"], 120, 32);
    open_icon(&mut harness, "Files");
    // A double click on a file's row is what opens it, as one on a folder's row goes into it.
    let (x, y) = row_of(&harness, "notlar.txt");
    harness.click(x, y).click(x, y);
    until(&mut harness, "the editor's word", |_| said.is_file());
    let opened = fs::read_to_string(&said).expect("the editor wrote what it was given");
    assert_eq!(Path::new(&opened), scratch.home().join("notlar.txt"), "the editor was given the file");
    assert_eq!(harness.app().windows().len(), 2, "the file has a window of its own:\n{}", harness.screen());
    assert_eq!(front(&harness).entry().name.get("en"), "notlar.txt", "the window is named after the file");
}

#[test]
fn open_a_terminal_here_starts_a_terminal_in_that_folder() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let apps = Environment { shell: Some(PathBuf::from(PWD)), ..environment(&scratch) };
    let mut harness = desk_in(apps, Vec::new(), &["files"], 120, 32);
    open_icon(&mut harness, "Files");
    row_menu(&mut harness, "belgeler", "Open a terminal here");
    assert_eq!(harness.app().windows().len(), 2, "a terminal opened:\n{}", harness.screen());
    let folder = scratch.home().join("belgeler");
    let shown = folder.display().to_string();
    until(&mut harness, "the shell's folder", |harness| harness.screen().contains(&shown));
    assert!(harness.app().files(front_id(&harness)).is_none(), "the new window is a terminal, not Files");
}

#[test]
fn open_in_a_new_window_shows_the_folder_in_a_files_window_of_its_own() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let mut harness = desk(&scratch);
    open_icon(&mut harness, "Files");
    let first = front_id(&harness);
    row_menu(&mut harness, "belgeler", "Open in a new window");
    let second = front_id(&harness);
    assert_ne!(first, second, "a second window opened");
    let files = harness.app().files(second).expect("the new window is a Files window");
    assert_eq!(files.manager.root(), scratch.home().join("belgeler"));
    assert!(harness.screen().contains("mektup.txt"), "it lists the folder:\n{}", harness.screen());
    assert_eq!(harness.app().files(first).map(|files| files.manager.folder().to_owned()).as_deref(), Some(""));
}

/// A desktop whose Files window shows the home folder, with a terminal opened from it in `belgeler`
/// and the Files window brought back to the front.
fn terminal_in_belgeler(scratch: &Scratch, glyphs: GlyphMode) -> Harness<Desk> {
    let apps = Environment { shell: Some(PathBuf::from(PWD)), ..environment(scratch) };
    let mut harness = desk_in(apps, Vec::new(), &["files"], 120, 32);
    harness.set_glyph_mode(glyphs);
    open_icon(&mut harness, "Files");
    row_menu(&mut harness, "belgeler", "Open a terminal here");
    bring(&mut harness, "Files");
    assert!(harness.app().files(front_id(&harness)).is_some(), "Files is in front:\n{}", harness.screen());
    harness
}

#[test]
fn a_folder_a_terminal_stands_in_carries_the_terminal_s_icon_in_the_accent_until_it_closes() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let mut harness = terminal_in_belgeler(&scratch, GlyphMode::Unicode);
    let line = row_line(&harness, "belgeler");
    assert!(line.contains("❯ belgeler"), "the row carries the Terminal's icon: {line}");
    assert!(row_line(&harness, "projeler").contains("■ projeler"), "a folder no window is in is unmarked");
    let (x, y) = row_of(&harness, "belgeler");
    let accent = harness.env().theme().color("accent");
    assert_eq!(harness.fg(u16::try_from(x - 2).expect("a column"), u16::try_from(y).expect("a row")), accent);
    // The terminal closes from its own title strip, and the mark goes with it.
    bring(&mut harness, "Terminal");
    close_front(&mut harness);
    assert_eq!(harness.app().windows().len(), 1, "the terminal closed:\n{}", harness.screen());
    let line = row_line(&harness, "belgeler");
    assert!(line.contains("■ belgeler"), "the row is a plain folder again: {line}");
}

#[test]
fn the_mark_is_a_sign_of_its_own_in_ascii_and_in_sixteen_colours_without_brackets_or_lines() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let mut harness = terminal_in_belgeler(&scratch, GlyphMode::Ascii);
    harness.set_depth(ColorDepth::Ansi16);
    let marked = row_line(&harness, "belgeler");
    let plain = row_line(&harness, "projeler");
    assert!(marked.contains("> belgeler"), "the Terminal's ASCII sign stands before the name: {marked}");
    assert!(plain.contains("# projeler"), "a plain folder keeps its own sign: {plain}");
    let screen = harness.screen();
    assert_eq!(decoration(&screen), None, "no brackets and no box lines:\n{screen}");
}

#[test]
fn a_file_another_program_makes_in_the_folder_appears_in_the_window_by_itself() {
    let scratch = Scratch::new();
    furnish(&scratch);
    let mut harness = desk(&scratch);
    open_icon(&mut harness, "Files");
    assert!(!harness.screen().contains("yeni.txt"), "not there yet:\n{}", harness.screen());
    // Nothing is asked of the window: the file is simply made, as another program would make it.
    fs::write(scratch.home().join("yeni.txt"), "yeni\n").expect("a file is written");
    until(&mut harness, "the new file in the window", |harness| harness.screen().contains("yeni.txt"));
}

#[test]
fn a_row_s_icon_says_what_kind_of_file_it_is() {
    let scratch = Scratch::new();
    furnish(&scratch);
    fs::write(scratch.home().join("main.rs"), "fn main() {}\n").expect("a file is written");
    let mut harness = desk(&scratch);
    harness.set_glyph_mode(GlyphMode::Nerd);
    open_icon(&mut harness, "Files");
    let icons = harness.env().icons();
    let (rust, plain) = (icons.glyph("file-rust").into_owned(), icons.glyph("file").into_owned());
    assert_ne!(rust, plain, "the kinds are told apart in a Nerd Font");
    let line = row_line(&harness, "main.rs");
    assert!(line.contains(&rust), "the Rust file carries the Rust icon: {line}");
    assert!(!line.contains(&plain), "not the plain file icon: {line}");
}
