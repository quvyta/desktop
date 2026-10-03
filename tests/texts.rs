//! The text viewer: a text, code or Markdown file opens in qdesk's own window, drawn
//! with the framework's code and Markdown views, and `e` or the Edit button hands it to the
//! person's editor in that same window.
//!
//! Every test starts where a person starts — two clicks on the Files icon, two on a file's row, a
//! key pressed while the viewer has the keys, a click on the drawn button — and every file is one
//! the test wrote in a folder of its own under the temporary folder. Nothing of the person running
//! the tests is read: the desktop is given a home, a data folder, an editor and a shell of the
//! test's own.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::Desk;
use qdesk::apps::{Catalog, Declared, Entry, Environment, Folders, Source, load, parse_entry};
use qdesk::desktop::Desktop;
use qdesk::wm::Window;
use qframe::env::{AssetDirs, Env};
use qframe::icons::GlyphMode;
use qframe::prelude::*;

use support::{BUDGET, HARMLESS, MACHINE, MOMENT, OFFSET, PATIENCE};

/// What the test's editor says when it starts, before it waits for a line.
const EDITOR_SAYS: &str = "düzenleyici açıldı";

/// A folder of one test in the temporary folder, taken away with everything in it when the test
/// ends, whether it passed or not.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-texts-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("ev")).expect("the home folder is made");
        // The editor writes down the file it was given, says it is open and waits for a line, as
        // an editor holds its window until the person is done.
        let script = format!(
            "printf '%s' \"$1\" > '{}'\nprintf '%s\\n' '{EDITOR_SAYS}'\nread satir\n",
            path.join("editor-said").display()
        );
        fs::write(path.join("editor.sh"), script).expect("the editor is written");
        Self(path)
    }

    fn home(&self) -> PathBuf {
        self.0.join("ev")
    }

    fn said(&self) -> PathBuf {
        self.0.join("editor-said")
    }

    /// The desktop's environment: this folder's home and data folder, the editor above, and a
    /// shell that ends at once.
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

/// A desktop of 120 by 32 over `scratch`, with Files and the `extra` entries on its floor.
fn desk(scratch: &Scratch, extra: Vec<Entry>) -> Harness<Desk> {
    let dirs = AssetDirs {
        locale_sources: qdesk::locales().iter().map(|(file, text)| ((*file).to_owned(), (*text).to_owned())).collect(),
        keymap_source: Some({
            let (file, text) = qdesk::keymap();
            (file.to_owned(), text.to_owned())
        }),
        ..AssetDirs::default()
    };
    let env = Env::load_with(&dirs, support::terminal).expect("the built-in files load");
    let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
    let mut icons = vec!["files".to_owned()];
    icons.extend(extra.iter().map(|entry| entry.id.clone()));
    let mut entries = load(&folders, None).entries;
    entries.extend(extra);
    let catalog = Catalog::new(support::sealed(entries), |_| true);
    let desktop = Desktop { icons, welcome_seen: true, recommended_seen: true, ..Desktop::default() };
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

/// Two clicks on the icon of the floor named `name`, as a person opens an application.
fn open_icon(harness: &mut Harness<Desk>, name: &str) {
    let (x, y) = harness.find(name).unwrap_or_else(|| panic!("no {name} on the floor:\n{}", harness.screen()));
    harness.click(x, y);
    harness.click(x, y);
}

/// The window in front.
fn front(harness: &Harness<Desk>) -> &Window {
    harness.app().windows().front().expect("a window on the desktop")
}

/// The name the window in front is called by.
fn title(harness: &Harness<Desk>) -> String {
    front(harness).entry().name.get("en").to_owned()
}

/// Where `text` is drawn inside the window in front, past its title row.
fn in_window(harness: &Harness<Desk>, text: &str) -> Option<(i32, i32)> {
    let rect = front(harness).rect();
    let screen = harness.screen();
    for (y, line) in screen.lines().enumerate() {
        let y = i32::try_from(y).expect("a row on screen");
        if y <= rect.y || y >= rect.y + i32::from(rect.height) {
            continue;
        }
        if let Some(at) = line.find(text) {
            let x = i32::from(qframe::text::width(&line[..at]));
            if x >= rect.x && x < rect.x + i32::from(rect.width) {
                return Some((x, y));
            }
        }
    }
    None
}

/// A Files window on the home folder, two clicks on the row of `name` in it, and the viewer that
/// opens, once it shows `shows`.
fn open_from_files(harness: &mut Harness<Desk>, name: &str, shows: &str) {
    open_icon(harness, "Files");
    until(harness, "the home folder", |harness| harness.screen().contains(name));
    let (x, y) = in_window(harness, name).unwrap_or_else(|| panic!("no row says {name}:\n{}", harness.screen()));
    harness.click(x, y).click(x, y);
    until(harness, &format!("{shows} in the viewer"), |harness| {
        harness.app().windows().len() == 2 && in_window(harness, shows).is_some()
    });
}

#[test]
fn a_double_click_on_a_note_in_a_files_window_opens_it_in_qdesk_s_own_viewer() {
    let scratch = Scratch::new();
    fs::write(scratch.home().join("notlar.txt"), "ilk satır burada\nikinci satır\n").expect("a note");
    let mut harness = desk(&scratch, Vec::new());
    open_from_files(&mut harness, "notlar.txt", "ilk satır burada");
    assert_eq!(title(&harness), "notlar.txt", "named after the file");
    assert_eq!(front(&harness).entry().icon.as_deref(), Some(qdesk::desktop::kind_icon("notlar.txt", false, false)));
    assert!(front(&harness).run().is_none(), "no program runs in it:\n{}", harness.screen());
    assert!(harness.app().text(front(&harness).id()).is_some(), "it is a text viewer");
    assert!(!scratch.said().exists(), "the editor was never started");
    assert!(in_window(&harness, "2 lines").is_some(), "the strip counts the lines:\n{}", harness.screen());
    assert!(in_window(&harness, "Edit").is_some(), "the strip offers Edit:\n{}", harness.screen());
}

#[test]
fn a_rust_file_shows_its_code_with_a_number_in_the_code_colour_for_numbers() {
    let scratch = Scratch::new();
    fs::write(scratch.home().join("main.rs"), "fn main() {\n    let sayi = 42;\n}\n").expect("a source file");
    let mut harness = desk(&scratch, Vec::new());
    open_from_files(&mut harness, "main.rs", "let sayi = 42;");
    let at = |text: &str| {
        let (x, y) = in_window(&harness, text).unwrap_or_else(|| panic!("{text} is drawn"));
        (u16::try_from(x).expect("on screen"), u16::try_from(y).expect("on screen"))
    };
    // The keywords of the built-in theme are drawn in its accent, which is the colour of the text
    // itself; a number has a colour of its own, so it is what tells code drawn as code.
    let number = harness.env().theme().style("code-token", Some("number"), &[]).paint("fg").map(|paint| paint.at(0.0));
    let ((nx, ny), (sx, sy)) = (at("42;"), at("sayi"));
    assert!(number.is_some(), "the theme colours numbers");
    assert_eq!(harness.fg(nx, ny), number, "42 is drawn as a number:\n{}", harness.screen());
    assert_ne!(harness.fg(sx, sy), number, "a name is not");
}

#[test]
fn a_markdown_file_is_drawn_as_a_document_not_as_its_source() {
    let scratch = Scratch::new();
    fs::write(scratch.home().join("README.md"), "# Başlık\n\nBir **kalın** söz.\n").expect("a readme");
    let mut harness = desk(&scratch, Vec::new());
    open_from_files(&mut harness, "README.md", "Başlık");
    assert!(in_window(&harness, "# Başlık").is_none(), "the heading's mark is not shown:\n{}", harness.screen());
    assert!(in_window(&harness, "Bir kalın söz.").is_some(), "the bold is drawn, not marked:\n{}", harness.screen());
}

#[test]
fn a_file_longer_than_four_mib_shows_its_start_and_says_so() {
    let scratch = Scratch::new();
    let mut long = String::with_capacity(5 * 1024 * 1024 + 64);
    let mut line = 0;
    while long.len() < 5 * 1024 * 1024 {
        line += 1;
        long.push_str(&format!("kayıt {line:07} tamam\n"));
    }
    fs::write(scratch.home().join("sunucu.log"), &long).expect("a long log");
    let mut harness = desk(&scratch, Vec::new());
    open_from_files(&mut harness, "sunucu.log", "kayıt 0000001 tamam");
    let screen = harness.screen();
    assert!(screen.contains("Showing the first 4 MiB of 5.0 MiB"), "the notice:\n{screen}");
    let window = harness.app().text(front(&harness).id()).expect("the viewer");
    let qdesk::texts::Reading::Ready(read) = window.reading() else { panic!("read") };
    assert!(read.text.len() <= 4 * 1024 * 1024, "no more than four MiB is held");
}

#[test]
fn a_file_with_bytes_that_are_not_utf8_says_some_characters_could_not_be_shown() {
    let scratch = Scratch::new();
    fs::write(scratch.home().join("eski.txt"), b"caf\xe9 au lait\n").expect("a latin-1 note");
    let mut harness = desk(&scratch, Vec::new());
    open_from_files(&mut harness, "eski.txt", "au lait");
    assert!(harness.screen().contains("Some characters could not be shown"), "{}", harness.screen());
}

#[test]
fn e_replaces_the_viewer_with_the_editor_in_the_same_window() {
    let scratch = Scratch::new();
    fs::write(scratch.home().join("notlar.txt"), "ilk satır burada\n").expect("a note");
    let mut harness = desk(&scratch, Vec::new());
    open_from_files(&mut harness, "notlar.txt", "ilk satır burada");
    let (id, rect) = (front(&harness).id(), front(&harness).rect());
    harness.press("e");
    until(&mut harness, "the editor's word", |harness| in_window(harness, EDITOR_SAYS).is_some());
    assert_eq!(harness.app().windows().len(), 2, "no new window:\n{}", harness.screen());
    assert_eq!((front(&harness).id(), front(&harness).rect()), (id, rect), "the same window, in the same place");
    assert_eq!(title(&harness), "notlar.txt", "keeps its name");
    assert!(front(&harness).run().is_some(), "it holds a program now");
    assert!(harness.app().text(id).is_none(), "the viewer is gone");
    let given = fs::read_to_string(scratch.said()).expect("the editor wrote what it was given");
    assert_eq!(Path::new(&given), scratch.home().join("notlar.txt"));
    assert!(in_window(&harness, "ilk satır burada").is_none(), "the text is the editor's to show now");
}

#[test]
fn a_click_on_the_edit_button_replaces_the_viewer_with_the_editor() {
    let scratch = Scratch::new();
    fs::write(scratch.home().join("notlar.txt"), "ilk satır burada\n").expect("a note");
    let mut harness = desk(&scratch, Vec::new());
    open_from_files(&mut harness, "notlar.txt", "ilk satır burada");
    let id = front(&harness).id();
    let (x, y) = in_window(&harness, "Edit").expect("the Edit button is drawn");
    harness.click(x, y);
    until(&mut harness, "the editor's word", |harness| in_window(harness, EDITOR_SAYS).is_some());
    assert_eq!(front(&harness).id(), id, "the same window");
    assert_eq!(title(&harness), "notlar.txt");
    assert!(harness.app().text(id).is_none(), "the viewer is gone");
}

#[test]
fn an_entry_that_opens_a_note_opens_the_viewer_and_a_missing_file_says_so() {
    let scratch = Scratch::new();
    let note = scratch.home().join("notlar.txt");
    fs::write(&note, "bir iki\n").expect("a note is written");
    let words = entry("not", &format!("name = \"Note\"\nopen = \"{}\"\n", note.display()));
    let gone = entry("kayip", &format!("name = \"Lost\"\nopen = \"{}\"\n", scratch.home().join("kayip.txt").display()));
    let mut harness = desk(&scratch, vec![words, gone]);
    open_icon(&mut harness, "Note");
    until(&mut harness, "the note", |harness| in_window(harness, "bir iki").is_some());
    assert!(harness.app().text(front(&harness).id()).is_some());

    let (x, y) = harness.find("Lost").expect("the lost file's icon");
    harness.click(x, y).click(x, y);
    until(&mut harness, "the empty state", |harness| harness.screen().contains("kayip.txt cannot be shown"));
    assert!(harness.screen().contains("It is no longer there"), "the reason:\n{}", harness.screen());
}

#[test]
fn the_help_layer_lists_edit_while_a_text_viewer_is_in_front() {
    let scratch = Scratch::new();
    fs::write(scratch.home().join("notlar.txt"), "bir iki\n").expect("a note");
    let mut harness = desk(&scratch, Vec::new());
    open_from_files(&mut harness, "notlar.txt", "bir iki");
    harness.press("f1");
    assert!(harness.screen().contains("Edit the file in this window"), "{}", harness.screen());
}
