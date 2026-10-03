//! The picture viewer: a picture opens in qdesk's own window, fitted whole, and the
//! keys and clicks of that window walk the pictures of its folder.
//!
//! Every test starts where a person starts — two clicks on the Files icon, two on a picture's row,
//! the keys pressed while the viewer has them, a click on the drawn picture — and every picture is
//! one the test wrote in a folder of its own under the temporary folder. Nothing of the person
//! running the tests is read: the desktop is given a home, a data folder, an editor and a shell of
//! the test's own.

mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::Desk;
use qdesk::apps::{Catalog, Declared, Entry, Environment, Folders, Source, load, parse_entry};
use qdesk::desktop::Desktop;
use qdesk::wm::Window;
use qframe::color::{ColorDepth, Rgb};
use qframe::env::{AssetDirs, Env};
use qframe::event::{MouseButton, MouseKind};
use qframe::icons::GlyphMode;
use qframe::prelude::*;

use support::{BUDGET, HARMLESS, MACHINE, MOMENT, OFFSET, PATIENCE};

const RED: [u8; 3] = [210, 30, 40];
const BLUE: [u8; 3] = [20, 60, 220];
const GREEN: [u8; 3] = [30, 180, 60];
const YELLOW: [u8; 3] = [230, 200, 20];

/// A folder of one test in the temporary folder, taken away with everything in it when the test
/// ends, whether it passed or not.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-pictures-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("ev")).expect("the home folder is made");
        fs::write(path.join("editor.sh"), format!("printf '%s' \"$1\" > '{}'\n", path.join("editor-said").display()))
            .expect("the editor is written");
        Self(path)
    }

    fn home(&self) -> PathBuf {
        self.0.join("ev")
    }

    /// The desktop's environment: this folder's home and data folder, an editor that writes down
    /// the file it was given, and a shell that ends at once.
    fn environment(&self) -> Environment {
        Environment {
            home: Some(self.home()),
            data_home: Some(self.0.join("veri")),
            config_home: Some(self.0.join("ayar")),
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

/// Writes a PNG of `width` × `height` whose pixel at `x`, `y` is `pixel(x, y)`.
fn picture(path: &Path, width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) {
    let image = image::RgbImage::from_fn(width, height, |x, y| image::Rgb(pixel(x, y)));
    image.save_with_format(path, image::ImageFormat::Png).expect("the picture is written");
}

/// A picture of one colour.
fn plain(path: &Path, colour: [u8; 3]) {
    picture(path, 64, 32, |_, _| colour);
}

/// A wide picture of four upright stripes, [`RED`], [`BLUE`], [`GREEN`] and [`YELLOW`], each 100
/// pixels wide: far wider than any window of these tests.
fn stripes(path: &Path) {
    picture(path, 400, 100, |x, _| [RED, BLUE, GREEN, YELLOW][(x / 100) as usize]);
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

/// Where the row that says `name` is in the window in front.
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

/// A Files window on the home folder, and two clicks on the row of `name` in it.
fn open_from_files(harness: &mut Harness<Desk>, name: &str) {
    open_icon(harness, "Files");
    until(harness, "the home folder", |harness| harness.screen().contains(name));
    let (x, y) = row_of(harness, name);
    harness.click(x, y).click(x, y);
}

/// The colours of the half blocks drawn inside the window in front, both halves.
fn colours(harness: &Harness<Desk>) -> BTreeSet<[u8; 3]> {
    let rect = front(harness).rect();
    let mut seen = BTreeSet::new();
    for y in rect.y + 1..rect.y + i32::from(rect.height) {
        for x in rect.x..rect.x + i32::from(rect.width) {
            let (Ok(x), Ok(y)) = (u16::try_from(x), u16::try_from(y)) else { continue };
            if !matches!(harness.buffer()[(x, y)].symbol(), "▀" | "▄") {
                continue;
            }
            for colour in [harness.fg(x, y), harness.bg(x, y)].into_iter().flatten() {
                let Rgb { r, g, b, .. } = colour;
                seen.insert([r, g, b]);
            }
        }
    }
    seen
}

/// Renders until the window in front shows `colour` in its half blocks.
fn shows(harness: &mut Harness<Desk>, colour: [u8; 3]) {
    until(harness, &format!("the colour {colour:?}"), |harness| colours(harness).contains(&colour));
}

/// A desktop over a home with three plain pictures and a note, with the first picture opened from
/// a Files window.
fn three_pictures(scratch: &Scratch) -> Harness<Desk> {
    plain(&scratch.home().join("a-kirmizi.png"), RED);
    plain(&scratch.home().join("b-mavi.png"), BLUE);
    plain(&scratch.home().join("c-yesil.png"), GREEN);
    fs::write(scratch.home().join("notlar.txt"), "bir iki\n").expect("a note is written");
    let mut harness = desk(scratch, Vec::new());
    open_from_files(&mut harness, "a-kirmizi.png");
    shows(&mut harness, RED);
    harness
}

#[test]
fn a_double_click_on_a_picture_in_a_files_window_opens_it_in_qdesk_s_own_viewer() {
    let scratch = Scratch::new();
    let harness = three_pictures(&scratch);
    assert_eq!(harness.app().windows().len(), 2, "a window of its own:\n{}", harness.screen());
    assert_eq!(title(&harness), "a-kirmizi.png", "named after the picture");
    assert_eq!(front(&harness).entry().icon.as_deref(), Some(qdesk::desktop::kind_icon("a-kirmizi.png", false, false)));
    assert!(front(&harness).run().is_none(), "no program runs in it:\n{}", harness.screen());
    assert!(harness.app().picture(front(&harness).id()).is_some(), "it is a picture viewer");
    assert!(!scratch.0.join("editor-said").exists(), "the editor was never started");
    let screen = harness.screen();
    assert!(screen.contains("64\u{d7}32") && screen.contains("1 / 3"), "the strip names its size and place:\n{screen}");
}

#[test]
fn the_arrows_and_the_letters_walk_the_folder_s_pictures_and_wrap_around() {
    let scratch = Scratch::new();
    let mut harness = three_pictures(&scratch);
    harness.press("right");
    shows(&mut harness, BLUE);
    assert_eq!(title(&harness), "b-mavi.png", "the title follows the picture");
    assert!(!colours(&harness).contains(&RED), "the first picture is gone");
    assert!(harness.screen().contains("2 / 3"), "{}", harness.screen());
    harness.press("n");
    shows(&mut harness, GREEN);
    assert_eq!(title(&harness), "c-yesil.png");
    harness.press("space");
    shows(&mut harness, RED);
    assert_eq!(title(&harness), "a-kirmizi.png", "after the last comes the first");
    harness.press("left");
    shows(&mut harness, GREEN);
    assert_eq!(title(&harness), "c-yesil.png", "before the first comes the last");
    harness.press("p");
    shows(&mut harness, BLUE);
    harness.press("home");
    shows(&mut harness, RED);
    harness.press("end");
    shows(&mut harness, GREEN);
    harness.press("backspace");
    shows(&mut harness, BLUE);
    assert_eq!(title(&harness), "b-mavi.png");
}

#[test]
fn a_click_on_the_right_third_of_the_picture_shows_the_next_and_on_the_left_third_the_one_before() {
    let scratch = Scratch::new();
    let mut harness = three_pictures(&scratch);
    let rect = front(&harness).rect();
    let middle = rect.y + i32::from(rect.height) / 2;
    harness.click(rect.x + i32::from(rect.width) - 3, middle);
    shows(&mut harness, BLUE);
    assert_eq!(title(&harness), "b-mavi.png");
    harness.click(rect.x + 2, middle);
    shows(&mut harness, RED);
    assert_eq!(title(&harness), "a-kirmizi.png");
    harness.click(rect.x + i32::from(rect.width) / 2, middle);
    harness.render();
    assert_eq!(title(&harness), "a-kirmizi.png", "the middle third changes nothing");
}

#[test]
fn f_shows_the_picture_at_its_own_size_cut_to_the_window_and_again_whole() {
    let scratch = Scratch::new();
    stripes(&scratch.home().join("seritler.png"));
    let mut harness = desk(&scratch, Vec::new());
    open_from_files(&mut harness, "seritler.png");
    shows(&mut harness, YELLOW);
    let whole = colours(&harness);
    assert!([RED, BLUE, GREEN, YELLOW].iter().all(|colour| whole.contains(colour)), "fitted, all four: {whole:?}");

    harness.press("f").render();
    let actual = colours(&harness);
    // A pixel a half cell: the window is far narrower than 400 pixels, so only the middle two
    // stripes reach it.
    assert!(actual.contains(&BLUE) && actual.contains(&GREEN), "the middle of it: {actual:?}");
    assert!(!actual.contains(&RED) && !actual.contains(&YELLOW), "its edges are cut: {actual:?}");

    harness.press("f").render();
    assert!(colours(&harness).contains(&RED), "whole again:\n{}", harness.screen());
}

#[test]
fn a_broken_picture_says_so_with_its_name_and_the_reason() {
    let scratch = Scratch::new();
    let broken = scratch.home().join("bozuk.png");
    plain(&broken, RED);
    let bytes = fs::read(&broken).expect("the picture");
    fs::write(&broken, &bytes[..bytes.len() / 2]).expect("cut in half");
    let mut harness = desk(&scratch, Vec::new());
    open_from_files(&mut harness, "bozuk.png");
    until(&mut harness, "the empty state", |harness| harness.screen().contains("bozuk.png cannot be shown"));
    assert!(harness.screen().contains("damaged"), "the framework's reason:\n{}", harness.screen());
    assert!(colours(&harness).is_empty(), "nothing drawn of it");
}

#[test]
fn a_terminal_that_cannot_draw_pictures_says_so_and_draws_no_letters_for_pixels() {
    let scratch = Scratch::new();
    let mut harness = three_pictures(&scratch);
    for mode in ["ascii", "sixteen"] {
        if mode == "ascii" {
            harness.set_glyph_mode(GlyphMode::Ascii).render();
        } else {
            harness.set_glyph_mode(GlyphMode::Unicode).set_depth(ColorDepth::Ansi16).render();
        }
        let screen = harness.screen();
        assert!(screen.contains("This terminal cannot show pictures"), "{mode}:\n{screen}");
        assert!(screen.contains("kitty or foot"), "{mode}: the hint:\n{screen}");
        assert!(screen.contains("64\u{d7}32") || screen.contains("64x32"), "{mode}: its size:\n{screen}");
        assert!(colours(&harness).is_empty(), "{mode}: no pixel drawn");
    }
}

#[test]
fn an_entry_that_opens_a_picture_opens_the_viewer_and_one_that_opens_a_book_still_has_no_viewer() {
    let scratch = Scratch::new();
    let file = scratch.home().join("deniz.png");
    plain(&file, BLUE);
    // A note opens in the text viewer (tests/texts.rs); a PDF has no viewer of qdesk's.
    let note = scratch.home().join("kitap.pdf");
    fs::write(&note, "%PDF-1.4\n").expect("a book is written");
    let sea = entry("deniz", &format!("name = \"Sea\"\nopen = \"{}\"\n", file.display()));
    let words = entry("not", &format!("name = \"Note\"\nopen = \"{}\"\n", note.display()));
    let mut harness = desk(&scratch, vec![sea, words]);
    open_icon(&mut harness, "Sea");
    shows(&mut harness, BLUE);
    assert_eq!(title(&harness), "deniz.png");
    assert!(harness.app().picture(front(&harness).id()).is_some());

    let (x, y) = harness.find("Note").expect("the note's icon");
    // The viewer covers part of the floor; the icon is reached where it shows.
    harness.click(x, y).click(x, y);
    until(&mut harness, "the corner's word", |harness| harness.screen().contains("no viewer"));
    assert_eq!(harness.app().windows().len(), 1, "no window opens onto the note:\n{}", harness.screen());
}

#[test]
fn open_with_on_a_picture_still_offers_and_starts_a_terminal_program() {
    let scratch = Scratch::new();
    plain(&scratch.home().join("deniz.png"), BLUE);
    let mut harness = desk(&scratch, Vec::new());
    open_icon(&mut harness, "Files");
    until(&mut harness, "the home folder", |harness| harness.screen().contains("deniz.png"));
    let (x, y) = row_of(&harness, "deniz.png");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    harness.click_text("Open with");
    assert!(harness.screen().contains("your editor"), "the editor is offered:\n{}", harness.screen());
    harness.click_text("your editor");
    let said = scratch.0.join("editor-said");
    until(&mut harness, "the editor's word", |_| said.is_file());
    assert_eq!(PathBuf::from(fs::read_to_string(&said).expect("said")), scratch.home().join("deniz.png"));
    assert!(harness.app().picture(front(&harness).id()).is_none(), "a terminal window, not the viewer");
}

#[test]
fn the_help_layer_lists_the_viewer_s_keys_while_a_viewer_is_in_front() {
    let scratch = Scratch::new();
    let mut harness = three_pictures(&scratch);
    harness.press("f1");
    let screen = harness.screen();
    assert!(screen.contains("Next picture in the folder"), "{screen}");
    assert!(screen.contains("Whole picture or actual size"), "{screen}");
}
