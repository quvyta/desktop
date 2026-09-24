//! Which program a file opens with: the one the desktop's own databases choose for its kind, when
//! it is a terminal program, else the person's editor; and "Open with" on a file's menu, which
//! offers the terminal programs of its kind and the editor.
//!
//! Every test starts where a person starts — two clicks on the Files icon, a click on a row, a
//! right click on it — and every database it reads is one it wrote in the temporary folder: its
//! own kinds (`mime/globs2`), its own programs (`applications/*.desktop`) and its own choices
//! (`mimeapps.list`). The machine's are never read: the desktop is given no system folder. Each
//! program is a script read by the system's shell that writes down what it was given, so nothing
//! of the person's is started, and a graphical program's script proves it never ran.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::Desk;
use qdesk::apps::{Catalog, Environment, Folders, load};
use qdesk::desktop::Desktop;
use qframe::env::{AssetDirs, Env};
use qframe::event::{MouseButton, MouseKind};
use qframe::icons::GlyphMode;
use qframe::prelude::*;

use support::{BUDGET, HARMLESS, MACHINE, MOMENT, OFFSET, PATIENCE};

/// A folder of one test in the temporary folder, taken away with everything in it when the test
/// ends, whether it passed or not.
struct Scratch(PathBuf);

impl Scratch {
    /// A home with a Rust file and a text file, the databases that say what they are and which
    /// programs open them, and the scripts those programs are.
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-open-with-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        let scratch = Self(path);
        scratch.write("ev/main.rs", "fn main() {}\n");
        scratch.write("ev/notlar.txt", "bir iki üç\n");
        scratch.write("veri/mime/globs2", "50:text/x-rust:*.rs\n50:text/plain:*.txt\n");
        scratch.program("kedi", "Kedi", true, "text/x-rust;text/plain;");
        scratch.program("tavsan", "Tavşan", true, "text/x-rust;");
        scratch.program("pencereli", "Pencereli", false, "text/x-rust;text/plain;");
        // The person chose a terminal program for Rust and a graphical one for text.
        scratch.write(
            "ayar/mimeapps.list",
            "[Default Applications]\ntext/x-rust=tavsan.desktop\ntext/plain=pencereli.desktop\n",
        );
        scratch.write("editor.sh", &format!("printf '%s' \"$1\" > '{}'\n", scratch.said("editor").display()));
        scratch
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().expect("a folder above")).expect("the folder is made");
        fs::write(&path, text).expect("the file is written");
        path
    }

    /// A program called `name` whose desktop entry says it opens `kinds`: a script that writes
    /// down the file it was given.
    fn program(&self, id: &str, name: &str, terminal: bool, kinds: &str) {
        let script = self.write(&format!("{id}.sh"), &format!("printf '%s' \"$1\" > '{}'\n", self.said(id).display()));
        self.write(
            &format!("veri/applications/{id}.desktop"),
            &format!(
                "[Desktop Entry]\nType=Application\nName={name}\nExec=/bin/sh {} %f\nTerminal={terminal}\nMimeType={kinds}\n",
                script.display()
            ),
        );
    }

    /// Where the program `id` writes down the file it was given.
    fn said(&self, id: &str) -> PathBuf {
        self.0.join(format!("{id}-said"))
    }

    fn home(&self) -> PathBuf {
        self.0.join("ev")
    }

    /// The environment of this test's desktop: its home, its data and configuration folders and
    /// its editor; no system folder, so nothing of the machine is read.
    fn environment(&self) -> Environment {
        Environment {
            home: Some(self.home()),
            data_home: Some(self.0.join("veri")),
            config_home: Some(self.0.join("ayar")),
            path: Some(self.0.join("bin").into_os_string()),
            shell: Some(PathBuf::from(HARMLESS)),
            editor: Some(format!("/bin/sh {}", self.0.join("editor.sh").display())),
            ..Environment::default()
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn desk(scratch: &Scratch) -> Harness<Desk> {
    let dirs = AssetDirs {
        locale_sources: qdesk::locales().iter().map(|(file, text)| ((*file).to_owned(), (*text).to_owned())).collect(),
        keymap_source: Some({
            let (file, text) = qdesk::keymap();
            (file.to_owned(), text.to_owned())
        }),
        ..AssetDirs::default()
    };
    let env = Env::load(&dirs).expect("the built-in files load");
    let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
    let catalog = Catalog::new(load(&folders, None).entries, |_| true);
    let desktop = Desktop { icons: vec!["files".to_owned()], welcome_seen: true, ..Desktop::default() };
    let clock = Box::new(|| MOMENT * 1_000);
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), clock)
        .apps(scratch.environment())
        .catalog(catalog)
        .desktop(desktop)
        .watch_within(PATIENCE);
    let mut harness = Harness::with_env(app, env, 120, 32);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);
    harness
}

/// The desktop with a Files window open on the home folder.
fn files_open(scratch: &Scratch) -> Harness<Desk> {
    let mut harness = desk(scratch);
    let (x, y) = harness.find("Files").unwrap_or_else(|| panic!("no Files on the floor:\n{}", harness.screen()));
    harness.click(x, y);
    harness.click(x, y);
    assert!(harness.screen().contains("main.rs"), "the home is shown:\n{}", harness.screen());
    harness
}

/// Where the row that says `name` is in the window in front.
fn row_of(harness: &Harness<Desk>, name: &str) -> (i32, i32) {
    let rect = harness.app().windows().front().expect("a window").rect();
    let screen = harness.screen();
    for (y, line) in screen.lines().enumerate() {
        let y = i32::try_from(y).expect("a row on screen");
        if y <= rect.y || y >= rect.y + i32::from(rect.height) {
            continue;
        }
        if let Some(at) = line.find(name) {
            return (i32::from(qframe::text::width(&line[..at])), y);
        }
    }
    panic!("no row says {name} in the window:\n{screen}");
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

/// A right click on the row `name` and a click on "Open with", which opens the list beside it.
fn open_with(harness: &mut Harness<Desk>, name: &str) -> String {
    let (x, y) = row_of(harness, name);
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    assert!(harness.screen().contains("Open with"), "the file's menu offers Open with:\n{}", harness.screen());
    harness.click_text("Open with");
    harness.screen()
}

/// What the program `id` was given, once it has run.
fn given(harness: &mut Harness<Desk>, scratch: &Scratch, id: &str) -> PathBuf {
    let said = scratch.said(id);
    until(harness, &format!("{id}'s word"), |_| said.is_file());
    PathBuf::from(fs::read_to_string(&said).expect("the program wrote what it was given"))
}

#[test]
fn a_double_click_on_a_file_opens_it_with_the_terminal_program_chosen_for_its_kind() {
    let scratch = Scratch::new();
    let mut harness = files_open(&scratch);
    let (x, y) = row_of(&harness, "main.rs");
    harness.click(x, y).click(x, y);
    assert_eq!(given(&mut harness, &scratch, "tavsan"), scratch.home().join("main.rs"));
    assert!(!scratch.said("kedi").exists() && !scratch.said("editor").exists(), "only the chosen program ran");
    let front = harness.app().windows().front().expect("the file's window");
    assert_eq!(front.entry().name.get("en"), "main.rs", "the window is named after the file");
    assert_eq!(front.entry().icon.as_deref(), Some("file-rust"), "and drawn with its kind's icon");
}

#[test]
fn a_file_whose_chosen_program_is_graphical_opens_in_the_editor_and_nothing_graphical_starts() {
    let scratch = Scratch::new();
    let mut harness = files_open(&scratch);
    let (x, y) = row_of(&harness, "notlar.txt");
    harness.click(x, y).click(x, y);
    assert_eq!(given(&mut harness, &scratch, "editor"), scratch.home().join("notlar.txt"));
    assert!(!scratch.said("pencereli").exists(), "the graphical program never ran");
    assert!(harness.opens().is_empty(), "nothing was opened beside the desktop: {:?}", harness.opens());
    assert_eq!(harness.app().windows().len(), 2, "the file has a window of its own:\n{}", harness.screen());
}

#[test]
fn open_with_lists_the_terminal_programs_of_the_kind_and_the_editor_but_no_graphical_one() {
    let scratch = Scratch::new();
    let mut harness = files_open(&scratch);
    let screen = open_with(&mut harness, "main.rs");
    for offered in ["Tavşan", "Kedi", "sh, your editor"] {
        assert!(screen.contains(offered), "{offered} is offered:\n{screen}");
    }
    assert!(!screen.contains("Pencereli"), "a graphical program is not offered:\n{screen}");
    let (tavsan, kedi) = (harness.find("Tavşan").expect("offered"), harness.find("Kedi").expect("offered"));
    assert!(tavsan.1 < kedi.1, "the chosen program comes first:\n{screen}");
}

#[test]
fn a_program_chosen_from_open_with_opens_the_file() {
    let scratch = Scratch::new();
    let mut harness = files_open(&scratch);
    open_with(&mut harness, "main.rs");
    harness.click_text("Kedi");
    assert_eq!(given(&mut harness, &scratch, "kedi"), scratch.home().join("main.rs"));
    assert!(!scratch.said("tavsan").exists(), "the default was not what ran");
}

#[test]
fn the_editor_chosen_from_open_with_opens_the_file() {
    let scratch = Scratch::new();
    let mut harness = files_open(&scratch);
    open_with(&mut harness, "main.rs");
    harness.click_text("sh, your editor");
    assert_eq!(given(&mut harness, &scratch, "editor"), scratch.home().join("main.rs"));
    assert!(!Path::new(&scratch.said("tavsan")).exists(), "the default was not what ran");
}
