//! A picture over the floor (design 3.9): chosen where a person chooses one — the Settings
//! screen and its file picker, a picture's menu in a Files window, a picture on the floor from the
//! Desktop folder — drawn in the picture's own colours behind the icons, kept through a restart,
//! and left out where it cannot be drawn or decoded.
//!
//! Every picture here is made by the test, in a folder of its own under the temporary folder, and
//! every folder the desktop is given — home, data, Desktop, settings — is one of those. Nothing of
//! the person running the tests is read or written.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::{Desk, Shown};
use qdesk::apps::Environment;
use qdesk::wallpapers::{MOST, OURS, Picture};
use qframe::color::{ColorDepth, Rgb};
use qframe::event::{MouseButton, MouseKind};
use qframe::graphics::Graphics;
use qframe::icons::GlyphMode;
use qframe::prelude::*;

use support::{BUDGET, HARMLESS};

/// The terminal these tests draw on: wide enough that a window leaves floor beside it.
const SIZE: (u16, u16) = (140, 44);

/// A cell of floor near the top that no icon and no window stands on.
const HIGH: (u16, u16) = (136, 4);

/// A cell of floor near the bottom, above the dock, that no icon and no window stands on.
const LOW: (u16, u16) = (136, SIZE.1 - 3);

/// A cell of floor in the middle, where no icon, window or notification stands.
const MIDDLE: (u16, u16) = (100, 20);

/// The colour of the upper half of the two-tone picture.
const TOP: [u8; 3] = [200, 40, 60];

/// The colour of the lower half of the two-tone picture.
const BOTTOM: [u8; 3] = [30, 90, 200];

/// A folder of one test in the temporary folder, taken away with everything in it when the test
/// ends: the home, the data folder and the settings of one desktop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-wallpaper-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("ev/Pictures")).expect("the home folder is made");
        fs::create_dir_all(path.join("ayarlar")).expect("the settings folder is made");
        Self(path)
    }

    fn home(&self) -> PathBuf {
        self.0.join("ev")
    }

    fn pictures(&self) -> PathBuf {
        self.home().join("Pictures")
    }

    fn config(&self) -> PathBuf {
        self.0.join("ayarlar")
    }

    fn data(&self) -> PathBuf {
        self.0.join("veri")
    }

    /// The desktop's environment: this scratch folder's home and data folder, a shell that ends at
    /// once, and `desktop` as the Desktop folder when one is given.
    fn environment(&self, desktop: Option<PathBuf>) -> Environment {
        Environment {
            home: Some(self.home()),
            data_home: Some(self.data()),
            config_home: Some(self.0.join("xdg")),
            shell: Some(PathBuf::from(HARMLESS)),
            desktop,
            ..Environment::default()
        }
    }

    /// A desktop over this scratch folder with `icons` on its floor.
    fn desk(&self, icons: &[&str]) -> Harness<Desk> {
        support::desk_at(&self.config(), self.environment(None), icons, SIZE.0, SIZE.1)
    }

    /// What the settings file says, or nothing when there is none.
    fn settings(&self) -> String {
        fs::read_to_string(self.config().join("desktop.conf")).unwrap_or_default()
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

/// A picture of two halves, [`TOP`] over [`BOTTOM`].
fn two_tone(path: &Path) {
    picture(path, 64, 32, |_, y| if y < 16 { TOP } else { BOTTOM });
}

/// A picture as busy as one gets: every pixel a colour of its own, dark and light side by side, so
/// whatever lies under an icon's name, the name must still read.
fn busy(path: &Path) {
    let mut seed = 0x2545_f491_u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let pixels: Vec<[u8; 3]> = (0..160 * 90)
        .map(|_| {
            let value = next().to_le_bytes();
            [value[0], value[1], value[2]]
        })
        .collect();
    picture(path, 160, 90, |x, y| pixels[(y * 160 + x) as usize]);
}

fn rgb([r, g, b]: [u8; 3]) -> Rgb {
    Rgb::new(r, g, b)
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

/// Whether the cell `cell` is a half block of the picture: the upper pixel `top`, the lower one
/// `bottom`.
fn half_block(harness: &Harness<Desk>, (x, y): (u16, u16), top: [u8; 3], bottom: [u8; 3]) -> bool {
    harness.buffer()[(x, y)].symbol() == "▀"
        && harness.fg(x, y) == Some(rgb(top))
        && harness.bg(x, y) == Some(rgb(bottom))
}

/// Whether the cell `cell` is plain floor of the theme's canvas, nothing drawn on it.
fn plain_floor(harness: &Harness<Desk>, (x, y): (u16, u16)) -> bool {
    harness.buffer()[(x, y)].symbol() == " " && harness.bg(x, y) == harness.env().theme().color("canvas")
}

/// Two clicks on the icon of the floor named `name`, as a person opens an application.
fn open_icon(harness: &mut Harness<Desk>, name: &str) {
    let (x, y) = harness.find(name).unwrap_or_else(|| panic!("no {name} on the floor:\n{}", harness.screen()));
    harness.click(x, y);
    harness.click(x, y);
}

/// A right click on the first place `text` is shown, and a click on `row` of the menu it opens.
fn menu_of(harness: &mut Harness<Desk>, text: &str, row: &str) {
    let (x, y) = harness.find(text).unwrap_or_else(|| panic!("no {text} on screen:\n{}", harness.screen()));
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    harness.mouse(MouseKind::Up(MouseButton::Right), x, y);
    assert!(harness.screen().contains(row), "the menu of {text} offers {row}:\n{}", harness.screen());
    harness.click_text(row);
}

#[test]
fn a_picture_chosen_with_the_settings_screens_file_picker_lies_over_the_floor_and_survives_a_restart() {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("deniz.png");
    two_tone(&file);
    fs::write(scratch.pictures().join("notlar.txt"), "not a picture").expect("a file");
    let mut harness = scratch.desk(&support::ICONS);
    assert!(plain_floor(&harness, HIGH), "the floor starts plain");
    open_icon(&mut harness, "Settings");
    let shown = harness.screen();
    assert!(shown.contains("Wallpaper") && shown.contains("None"), "the row says there is none:\n{shown}");

    harness.click_text("Choose…");
    until(&mut harness, "the picker on the Pictures folder", |harness| harness.screen().contains("deniz.png"));
    assert!(harness.app().wallpaper().picking());
    assert!(!harness.screen().contains("notlar.txt"), "only pictures are offered:\n{}", harness.screen());
    // The only picture is the one the picker stands on; its button chooses it.
    harness.click_text("Choose file");
    until(&mut harness, "the picture on the floor", |harness| half_block(harness, HIGH, TOP, TOP));
    assert!(!harness.app().wallpaper().picking(), "the picker closed");
    assert!(half_block(&harness, LOW, BOTTOM, BOTTOM), "the lower half lies low:\n{}", harness.screen());
    assert_eq!(harness.app().wallpaper().path(), Some(file.as_path()));
    assert!(harness.screen().contains("deniz.png"), "the row names it:\n{}", harness.screen());
    assert!(harness.screen().contains("Remove"));
    let line = format!("wallpaper = \"{}\"", file.display());
    until(&mut harness, "the file written", |_| scratch.settings().contains(&line));

    // The next run reads it back from the file and decodes it off the first frame.
    let mut again = scratch.desk(&support::ICONS);
    until(&mut again, "the picture after a restart", |harness| half_block(harness, HIGH, TOP, TOP));
}

#[test]
fn remove_on_the_settings_screen_gives_the_floor_its_colour_back_and_takes_the_line_out() {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("deniz.png");
    two_tone(&file);
    fs::write(scratch.config().join("desktop.conf"), format!("wallpaper = \"{}\"\n", file.display())).expect("file");
    let mut harness = scratch.desk(&support::ICONS);
    until(&mut harness, "the picture", |harness| half_block(harness, HIGH, TOP, TOP));
    open_icon(&mut harness, "Settings");
    harness.click_text("Remove");
    assert!(plain_floor(&harness, HIGH), "the floor is plain again:\n{}", harness.screen());
    assert!(matches!(harness.app().wallpaper().shown(), Shown::Nothing));
    until(&mut harness, "the file written", |_| !scratch.settings().contains("wallpaper"));
}

#[test]
fn an_included_picture_is_decoded_from_the_program_and_the_settings_name_it_builtin() {
    let scratch = Scratch::new();
    let mut harness = scratch.desk(&support::ICONS);
    open_icon(&mut harness, "Settings");
    harness.click_text("Pick one");
    harness.click_text("Tide");
    let tide = Picture::Ours(OURS[2]);
    until(&mut harness, "the included picture", |harness| harness.app().wallpaper().picture() == Some(&tide));
    until(&mut harness, "the file written", |_| scratch.settings().contains("wallpaper = \"builtin:tide\""));
    assert!(!scratch.data().exists(), "nothing is written into the data folder");
    assert_eq!(harness.buffer()[(HIGH.0, HIGH.1)].symbol(), "▀", "it is drawn:\n{}", harness.screen());
    assert_eq!(harness.app().settings_screen().wallpaper().ours, Some(2), "the drop-down shows which");
    assert_eq!(harness.app().settings_screen().wallpaper().name.as_deref(), Some("Tide"));

    // The next run decodes it from the program again.
    let mut again = scratch.desk(&support::ICONS);
    until(&mut again, "the picture after a restart", |harness| harness.buffer()[(HIGH.0, HIGH.1)].symbol() == "▀");
    assert!(!scratch.data().exists());
}

#[test]
fn a_path_version_0_1_7_wrote_one_of_ours_into_is_still_that_picture_when_its_file_is_gone() {
    let scratch = Scratch::new();
    let old = scratch.data().join("quvyta/desktop/wallpapers/ember.png");
    fs::write(scratch.config().join("desktop.conf"), format!("wallpaper = \"{}\"\n", old.display())).expect("file");
    let mut harness = scratch.desk(&support::ICONS);
    until(&mut harness, "the picture", |harness| harness.buffer()[(HIGH.0, HIGH.1)].symbol() == "▀");
    assert!(!old.exists(), "it was never read from there");
    assert_eq!(harness.app().settings_screen().wallpaper().ours, Some(0));
    assert_eq!(harness.app().settings_screen().wallpaper().name.as_deref(), Some("Ember"));
    assert!(scratch.settings().contains(&old.display().to_string()), "reading writes nothing");
}

/// A picture far larger than any floor, of two halves side by side, so the size it is decoded at
/// is the size it was asked for.
fn wide(path: &Path) {
    picture(path, 2400, 1200, |x, _| if x < 1200 { TOP } else { BOTTOM });
}

/// The width and height the picture on the floor was decoded at, once it is.
fn decoded(harness: &Harness<Desk>) -> Option<(u32, u32)> {
    match harness.app().wallpaper().shown() {
        Shown::Ready(data) => Some((data.width(), data.height())),
        _ => None,
    }
}

#[test]
fn the_picture_is_decoded_at_the_size_the_terminal_shows_and_again_when_that_changes() {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("genis.png");
    wide(&file);
    fs::write(scratch.config().join("desktop.conf"), format!("wallpaper = \"{}\"\n", file.display())).expect("file");
    let mut harness = scratch.desk(&[]);
    until(&mut harness, "the half blocks' picture", |harness| decoded(harness) == Some((960, 480)));
    assert_eq!(harness.app().wallpaper().asked(), Some(MOST));

    // The floor is 140 by 43 cells: ten by twenty pixels a cell.
    harness.set_graphics(Graphics::Kitty).render();
    assert_eq!(harness.app().wallpaper().asked(), Some((1400, 860)));
    until(&mut harness, "the kitty picture", |harness| decoded(harness) == Some((1400, 700)));

    harness.resize(100, 30);
    assert_eq!(harness.app().wallpaper().asked(), Some((1000, 580)), "a smaller floor asks for less");
    until(&mut harness, "the smaller picture", |harness| decoded(harness) == Some((1000, 500)));

    harness.set_graphics(Graphics::HalfBlock).render();
    assert_eq!(harness.app().wallpaper().asked(), Some(MOST));
    until(&mut harness, "the half blocks' picture again", |harness| decoded(harness) == Some((960, 480)));
}

#[test]
fn a_kitty_terminal_over_ssh_is_sent_twice_the_half_blocks_pixels() {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("genis.png");
    wide(&file);
    fs::write(scratch.config().join("desktop.conf"), format!("wallpaper = \"{}\"\n", file.display())).expect("file");
    let mut harness = support::desk_over(&scratch.config(), scratch.environment(None), &[], SIZE, true);
    until(&mut harness, "the half blocks' picture", |harness| decoded(harness) == Some((960, 480)));
    harness.set_graphics(Graphics::Kitty).render();
    assert_eq!(harness.app().wallpaper().asked(), Some((280, 172)));
    until(&mut harness, "the kitty picture", |harness| decoded(harness) == Some((280, 140)));
}

#[test]
fn a_sixel_terminal_is_sent_a_cells_worth_of_pixels_here_and_twice_the_half_blocks_over_ssh() {
    for (remote, asked, got) in [(false, (1400, 860), (1400, 700)), (true, (280, 172), (280, 140))] {
        let scratch = Scratch::new();
        let file = scratch.pictures().join("genis.png");
        wide(&file);
        fs::write(scratch.config().join("desktop.conf"), format!("wallpaper = \"{}\"\n", file.display()))
            .expect("file");
        let mut harness = support::desk_over(&scratch.config(), scratch.environment(None), &[], SIZE, remote);
        until(&mut harness, "the half blocks' picture", |harness| decoded(harness) == Some((960, 480)));
        harness.set_graphics(Graphics::Sixel).render();
        assert_eq!(harness.app().wallpaper().asked(), Some(asked), "over ssh: {remote}");
        until(&mut harness, "the sixel picture", |harness| decoded(harness) == Some(got));
    }
}

#[test]
fn set_as_wallpaper_on_a_pictures_menu_in_a_files_window_lays_it_over_the_floor() {
    let scratch = Scratch::new();
    let file = scratch.home().join("deniz.png");
    two_tone(&file);
    fs::write(scratch.home().join("notlar.txt"), "not a picture").expect("a file");
    let mut harness = scratch.desk(&["terminal", "files", "settings"]);
    open_icon(&mut harness, "Files");
    until(&mut harness, "the home folder in the window", |harness| harness.screen().contains("deniz.png"));
    let window = harness.app().windows().front().expect("the Files window").rect();
    assert!(window.right() < i32::from(HIGH.0), "the cell looked at is floor: {window:?}");

    // A file that is not a picture is not offered as one.
    let (x, y) = harness.find("notlar.txt").expect("the text file's row");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    harness.mouse(MouseKind::Up(MouseButton::Right), x, y);
    assert!(harness.screen().contains("Open with"), "its menu is open:\n{}", harness.screen());
    assert!(!harness.screen().contains("Set as wallpaper"), "{}", harness.screen());
    harness.press("esc");

    menu_of(&mut harness, "deniz.png", "Set as wallpaper");
    until(&mut harness, "the picture on the floor", |harness| half_block(harness, HIGH, TOP, TOP));
    until(&mut harness, "the file written", |_| scratch.settings().contains("deniz.png"));
}

#[test]
fn a_picture_among_the_desktop_folders_icons_is_set_from_its_own_menu() {
    let scratch = Scratch::new();
    let desktop = scratch.home().join("Desktop");
    fs::create_dir_all(&desktop).expect("the Desktop folder");
    let file = desktop.join("deniz.png");
    two_tone(&file);
    fs::write(desktop.join("not.txt"), "not a picture").expect("a file");
    let mut harness =
        support::desk_at(&scratch.config(), scratch.environment(Some(desktop)), &support::ICONS, SIZE.0, SIZE.1);
    until(&mut harness, "the Desktop folder's icons", |harness| harness.screen().contains("deniz.png"));
    let (x, y) = harness.find("not.txt").expect("the text file's icon");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    harness.mouse(MouseKind::Up(MouseButton::Right), x, y);
    assert!(harness.screen().contains("Rename"), "its menu is open:\n{}", harness.screen());
    assert!(!harness.screen().contains("Set as wallpaper"), "{}", harness.screen());
    harness.press("esc");

    menu_of(&mut harness, "deniz.png", "Set as wallpaper");
    until(&mut harness, "the picture on the floor", |harness| half_block(harness, HIGH, TOP, TOP));
    assert_eq!(harness.app().wallpaper().path(), Some(file.as_path()));
}

#[test]
fn every_icon_name_reads_on_the_busiest_picture_and_the_picture_shows_between_the_tiles() {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("gurultu.png");
    busy(&file);
    fs::write(scratch.config().join("desktop.conf"), format!("wallpaper = \"{}\"\n", file.display())).expect("file");
    for theme in ["amber", "iris", "nordic", "monochrome"] {
        let mut harness = scratch.desk(&support::ICONS);
        harness.set_theme(theme).render();
        until(&mut harness, "the picture", |harness| harness.buffer()[(HIGH.0, HIGH.1)].symbol() == "▀");
        let surface = harness.env().theme().color("surface").expect("a card tone");
        for name in ["Terminal", "Settings", "Midnight"] {
            let (x, y) = harness.find(name).unwrap_or_else(|| panic!("{name}:\n{}", harness.screen()));
            let (x, y) = (u16::try_from(x).expect("x"), u16::try_from(y).expect("y"));
            for column in x..x + u16::try_from(name.len()).expect("width") {
                let (fg, bg) = (harness.fg(column, y).expect("fg"), harness.bg(column, y).expect("bg"));
                assert_eq!(bg, surface, "{theme}: {name} stands on the card tone");
                let ratio = fg.contrast_ratio(bg);
                assert!(ratio >= 4.5, "{theme}: {name} reads at {ratio:.2}:1");
            }
            let glyph = y - 1;
            let tile: Vec<u16> = (0..9).map(|step| (x / 10) * 10 + 1 + step).collect();
            assert!(
                tile.iter().all(|column| harness.bg(*column, glyph) == Some(surface)),
                "{theme}: the icon's row is tiled"
            );
            let beside = (x / 10) * 10;
            assert_eq!(harness.buffer()[(beside, y)].symbol(), "▀", "{theme}: the pillar's column is picture");
            assert_eq!(harness.buffer()[(x, y + 1)].symbol(), "▀", "{theme}: the row under the name is picture");
        }
    }
}

#[test]
fn icons_on_a_floor_without_a_picture_stand_bare_as_before() {
    let scratch = Scratch::new();
    let harness = scratch.desk(&support::ICONS);
    let canvas = harness.env().theme().color("canvas");
    let (x, y) = harness.find("Terminal").expect("the icon");
    let (x, y) = (u16::try_from(x).expect("x"), u16::try_from(y).expect("y"));
    assert_eq!(harness.bg(x, y), canvas, "no tile under a name on the bare floor");
}

#[test]
fn sixteen_colours_and_ascii_show_the_floors_colour_and_pattern_instead_of_the_picture() {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("deniz.png");
    two_tone(&file);
    let conf = format!("wallpaper = \"{}\"\nfloor-style = \"dots\"\n", file.display());
    fs::write(scratch.config().join("desktop.conf"), conf).expect("file");
    let mut harness = scratch.desk(&support::ICONS);
    until(&mut harness, "the picture", |harness| half_block(harness, HIGH, TOP, TOP));
    // The dots are the floor's; over a picture there are none.
    assert!(!harness.screen().contains('\u{b7}'), "{}", harness.screen());

    harness.set_depth(ColorDepth::Ansi16).render();
    assert!(!harness.screen().contains('▀'), "no pixel in sixteen colours:\n{}", harness.screen());
    assert!(plain_floor(&harness, HIGH) || harness.buffer()[(HIGH.0, HIGH.1)].symbol() == " ", "{}", harness.screen());
    assert!(!harness.screen().contains("cannot show"), "no sentence across the floor:\n{}", harness.screen());
    let (x, y) = harness.find("Terminal").expect("the icon");
    let (x, y) = (u16::try_from(x).expect("x"), u16::try_from(y).expect("y"));
    assert_ne!(harness.bg(x, y), harness.env().theme().color("surface"), "no tile without a picture");

    harness.set_depth(ColorDepth::TrueColor).set_glyph_mode(GlyphMode::Ascii).render();
    assert!(!harness.screen().contains('▀'), "no pixel in ASCII:\n{}", harness.screen());
    assert_eq!(harness.buffer()[(130, 2)].symbol(), ".", "the floor's dots in ASCII:\n{}", harness.screen());

    harness.set_glyph_mode(GlyphMode::Unicode).set_depth(ColorDepth::Ansi256).render();
    assert_eq!(harness.buffer()[(HIGH.0, HIGH.1)].symbol(), "▀", "256 colours draw it again");
}

#[test]
fn a_missing_or_damaged_picture_is_said_and_the_floor_keeps_its_colour() {
    let scratch = Scratch::new();
    let damaged = scratch.pictures().join("yarim.png");
    two_tone(&damaged);
    let bytes = fs::read(&damaged).expect("file");
    fs::write(&damaged, &bytes[..bytes.len() / 2]).expect("cut in half");
    let missing = scratch.pictures().join("yok.png");
    for (file, reason) in [(&missing, "does not exist"), (&damaged, "damaged")] {
        let conf = format!("wallpaper = \"{}\"\nfloor-color = \"deep\"\n", file.display());
        fs::write(scratch.config().join("desktop.conf"), conf).expect("file");
        let mut harness = scratch.desk(&support::ICONS);
        until(&mut harness, "the notice", |harness| harness.screen().contains("The wallpaper could not be shown"));
        assert!(matches!(harness.app().wallpaper().shown(), Shown::Failed(_)));
        let deep = qdesk::settings::FloorColor::Deep.in_theme(harness.env().theme());
        assert_eq!(harness.bg(MIDDLE.0, MIDDLE.1), deep, "the floor keeps its colour:\n{}", harness.screen());
        assert_ne!(harness.buffer()[(MIDDLE.0, MIDDLE.1)].symbol(), "▀");
        open_icon(&mut harness, "Settings");
        assert!(harness.screen().contains(reason), "the row says why:\n{}", harness.screen());
        // The file is the person's: it is left as it was.
        assert!(scratch.settings().contains("wallpaper = "), "{}", scratch.settings());
    }
}

#[test]
fn a_file_chosen_that_is_not_a_picture_is_refused_and_the_floor_is_left_as_it_was() {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("sahte.png");
    fs::write(&file, "only text in a picture's name").expect("file");
    let mut harness = scratch.desk(&support::ICONS);
    open_icon(&mut harness, "Settings");
    harness.click_text("Choose…");
    until(&mut harness, "the picker", |harness| harness.screen().contains("sahte.png"));
    // A double click on the file chooses it, as Enter does; a single click only stands on it.
    harness.click_text("sahte.png").click_text("sahte.png");
    until(&mut harness, "the refusal", |harness| harness.screen().contains("sahte.png is not the wallpaper"));
    assert!(harness.screen().contains("not a PNG, JPEG, GIF or WebP"), "{}", harness.screen());
    assert!(plain_floor(&harness, HIGH));
    assert_eq!(harness.app().wallpaper().path(), None);
    assert!(!scratch.settings().contains("wallpaper"), "nothing is written: {}", scratch.settings());
}

#[test]
fn a_picture_written_into_the_settings_file_by_the_command_is_drawn_by_the_running_desktop() {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("deniz.png");
    two_tone(&file);
    let mut harness = scratch.desk(&support::ICONS);
    assert!(plain_floor(&harness, HIGH));
    let args = ["wallpaper".to_owned(), file.display().to_string()];
    let (mut out, mut err) = (Vec::new(), Vec::new());
    let exit = qdesk::cli::execute_in(&args, Some("en"), Some(&scratch.config()), &mut out, &mut err);
    assert_eq!(exit, Some(qdesk::cli::EXIT_OK), "{}", String::from_utf8_lossy(&err));
    until(&mut harness, "the picture", |harness| half_block(harness, HIGH, TOP, TOP));

    let args = ["wallpaper".to_owned(), "--no-picture".to_owned()];
    let exit = qdesk::cli::execute_in(&args, Some("en"), Some(&scratch.config()), &mut out, &mut err);
    assert_eq!(exit, Some(qdesk::cli::EXIT_OK));
    until(&mut harness, "the plain floor", |harness| plain_floor(harness, HIGH));
}

#[test]
fn escape_closes_the_picker_and_nothing_is_chosen() {
    let scratch = Scratch::new();
    two_tone(&scratch.pictures().join("deniz.png"));
    let mut harness = scratch.desk(&support::ICONS);
    open_icon(&mut harness, "Settings");
    harness.click_text("Choose…");
    until(&mut harness, "the picker", |harness| harness.screen().contains("Choose a wallpaper"));
    harness.press("esc");
    assert!(!harness.app().wallpaper().picking(), "{}", harness.screen());
    assert!(!harness.screen().contains("Choose a wallpaper"));
    assert_eq!(harness.app().wallpaper().path(), None);
    assert!(plain_floor(&harness, HIGH));
}

#[test]
fn a_relative_path_in_the_settings_file_is_said_with_its_line_and_left_out() {
    let scratch = Scratch::new();
    fs::write(scratch.config().join("desktop.conf"), "floor-color = \"deep\"\nwallpaper = \"deniz.png\"\n")
        .expect("file");
    let loaded = qdesk::settings::load_in(&scratch.config());
    assert_eq!(loaded.wallpaper, None);
    assert_eq!(loaded.diagnostics.len(), 1, "{:?}", loaded.diagnostics);
    assert!(loaded.diagnostics[0].location().contains("desktop.conf:2"), "{:?}", loaded.diagnostics);
    let harness = scratch.desk(&support::ICONS);
    assert_eq!(harness.app().wallpaper().path(), None);
}

/// The cells that show the picture in half blocks: `▀` in two of its colours.
fn picture_cells(harness: &Harness<Desk>) -> Vec<(u16, u16)> {
    let mut cells = Vec::new();
    for y in 0..SIZE.1 {
        for x in 0..SIZE.0 {
            if harness.buffer()[(x, y)].symbol() == "▀" {
                cells.push((x, y));
            }
        }
    }
    cells
}

#[test]
fn a_kitty_terminal_draws_the_picture_itself_on_a_floor_nothing_stands_on() {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("deniz.png");
    two_tone(&file);
    fs::write(scratch.config().join("desktop.conf"), format!("wallpaper = \"{}\"\n", file.display())).expect("file");
    let mut harness = scratch.desk(&[]);
    until(&mut harness, "the picture", |harness| half_block(harness, HIGH, TOP, TOP));
    harness.set_graphics(Graphics::Kitty).render();
    assert!(picture_cells(&harness).is_empty(), "no half block is left:\n{}", harness.screen());
    for cell in [HIGH, MIDDLE, LOW, (0, 0)] {
        assert!(plain_floor(&harness, cell), "{cell:?} is ground for the terminal's pixels:\n{}", harness.screen());
    }
    assert!(harness.screen().contains("14:32"), "the dock is drawn over it:\n{}", harness.screen());

    harness.set_graphics(Graphics::HalfBlock).render();
    assert!(half_block(&harness, HIGH, TOP, TOP), "half blocks again where the terminal cannot");
}

/// The text colour the framework marks a picture's cells with on `ground`, where the terminal is
/// to show its pixels: a space in a colour far from the ground's in every channel. A cell still
/// holding it after the frame is painted is one the terminal puts pixels in.
fn marker(ground: Rgb) -> Rgb {
    let far = |channel: u8, offset: u8| if channel < 128 { 255 - offset } else { offset };
    Rgb::new(far(ground.r, 3), far(ground.g, 7), far(ground.b, 4))
}

/// The cells the terminal shows the picture's own pixels in: nothing was painted over their mark.
fn pixel_cells(harness: &Harness<Desk>) -> Vec<(u16, u16)> {
    let canvas = harness.env().theme().color("canvas").expect("a canvas colour");
    let mark = marker(canvas);
    let mut cells = Vec::new();
    for y in 0..SIZE.1 {
        for x in 0..SIZE.0 {
            if harness.buffer()[(x, y)].symbol() == " "
                && harness.bg(x, y) == Some(canvas)
                && harness.fg(x, y) == Some(mark)
            {
                cells.push((x, y));
            }
        }
    }
    cells
}

/// The screen as it stands drawn with half blocks and with `graphics`: whatever hides the picture
/// in half blocks hides it there too, and the picture shows, as pixels or half blocks, in every
/// cell it shows in with half blocks. Answers how many of those cells are pixels.
fn same_cover(harness: &mut Harness<Desk>, graphics: Graphics, what: &str) -> usize {
    harness.set_graphics(Graphics::HalfBlock).render();
    let mut shown = picture_cells(harness);
    shown.sort_unstable();
    harness.set_graphics(graphics).render();
    let pixels = pixel_cells(harness);
    let mut there: Vec<(u16, u16)> = pixels.iter().copied().chain(picture_cells(harness)).collect();
    there.sort_unstable();
    let through: Vec<&(u16, u16)> = there.iter().filter(|cell| shown.binary_search(cell).is_err()).collect();
    assert!(
        through.is_empty(),
        "{graphics:?}, {what}: the picture shows through at {through:?}:\n{}",
        harness.screen()
    );
    let holes: Vec<&(u16, u16)> = shown.iter().filter(|cell| there.binary_search(cell).is_err()).collect();
    assert!(holes.is_empty(), "{graphics:?}, {what}: the picture is missing at {holes:?}:\n{}", harness.screen());
    pixels.len()
}

/// Walks a desktop with icons through what stands on a picture: the icons' tiles, the floor's
/// menu, a clock widget, the launcher, the help and a Settings window, asking at each that
/// `graphics` covers the picture exactly where half blocks do. Answers the pixel cells of each.
fn walk_the_covers(graphics: Graphics) -> Vec<(&'static str, usize)> {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("deniz.png");
    two_tone(&file);
    fs::write(scratch.config().join("desktop.conf"), format!("wallpaper = \"{}\"\n", file.display())).expect("file");
    let mut harness = scratch.desk(&support::ICONS);
    until(&mut harness, "the picture", |harness| half_block(harness, HIGH, TOP, TOP));
    let mut pixels = Vec::new();
    pixels.push(("the icons", same_cover(&mut harness, graphics, "the icons")));
    harness.mouse(MouseKind::Down(MouseButton::Right), i32::from(MIDDLE.0), i32::from(MIDDLE.1));
    harness.mouse(MouseKind::Up(MouseButton::Right), i32::from(MIDDLE.0), i32::from(MIDDLE.1));
    assert!(harness.screen().contains("Add a widget"), "the floor's menu is open:\n{}", harness.screen());
    pixels.push(("the floor's menu", same_cover(&mut harness, graphics, "the floor's menu")));
    harness.click_text("Add a widget");
    let (x, y) = harness.find("Clock").unwrap_or_else(|| panic!("a clock is offered:\n{}", harness.screen()));
    harness.click(x, y);
    assert_eq!(harness.app().gadgets().len(), 1, "a clock stands on the floor:\n{}", harness.screen());
    pixels.push(("a clock", same_cover(&mut harness, graphics, "a clock")));
    harness.press("space");
    pixels.push(("the launcher", same_cover(&mut harness, graphics, "the launcher")));
    harness.press("esc");
    harness.press("f1");
    pixels.push(("the help", same_cover(&mut harness, graphics, "the help")));
    harness.press("esc");
    harness.press("space");
    harness.type_text("Settings");
    harness.press("enter");
    assert_eq!(harness.app().windows().len(), 1, "a Settings window is open:\n{}", harness.screen());
    pixels.push(("a Settings window", same_cover(&mut harness, graphics, "a Settings window")));
    pixels
}

/// The cells marked as a picture's are the framework's; if its mark changed, this file would
/// look for the wrong colour and every covering test would pass with no pixels at all.
#[test]
fn the_mark_looked_for_is_the_one_the_picture_leaves() {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("deniz.png");
    two_tone(&file);
    fs::write(scratch.config().join("desktop.conf"), format!("wallpaper = \"{}\"\n", file.display())).expect("file");
    let mut harness = scratch.desk(&[]);
    until(&mut harness, "the picture", |harness| half_block(harness, HIGH, TOP, TOP));
    harness.set_graphics(Graphics::Kitty).render();
    for cell in [HIGH, MIDDLE, LOW] {
        assert!(pixel_cells(&harness).contains(&cell), "{cell:?} is marked for pixels:\n{}", harness.screen());
    }
}

/// Whatever hides the picture where it is drawn with half blocks hides it where the terminal
/// draws it with the kitty protocol, and the picture still shows as pixels between the icons, the
/// widget and the windows: the terminal places it once in every free rectangle. The help lays a
/// backdrop over the whole screen, and a dimmed picture is drawn with half blocks.
#[test]
fn whatever_covers_the_picture_in_half_blocks_covers_it_on_a_kitty_terminal_too() {
    for (what, pixels) in walk_the_covers(Graphics::Kitty) {
        if what == "the help" {
            assert_eq!(pixels, 0, "under the help's backdrop the picture is dimmed half blocks");
        } else {
            assert!(pixels > 1000, "with {what} most of the floor is still the terminal's pixels: {pixels} cells");
        }
    }
}

/// The same on a sixel terminal, which paints the picture into the cells: it is written only where
/// nothing at all stands on it, so with icons on the floor it is drawn with half blocks, over none
/// of what stands on it.
#[test]
fn whatever_covers_the_picture_in_half_blocks_covers_it_on_a_sixel_terminal_too() {
    for (what, pixels) in walk_the_covers(Graphics::Sixel) {
        assert_eq!(pixels, 0, "with {what} the picture is half blocks");
    }
}

/// A sixel terminal is written the picture's pixels on a floor nothing stands on, and half
/// blocks again as soon as a window opens over it.
#[test]
fn a_sixel_terminal_draws_the_picture_itself_on_a_floor_nothing_stands_on() {
    let scratch = Scratch::new();
    let file = scratch.pictures().join("deniz.png");
    two_tone(&file);
    fs::write(scratch.config().join("desktop.conf"), format!("wallpaper = \"{}\"\n", file.display())).expect("file");
    let mut harness = scratch.desk(&[]);
    until(&mut harness, "the picture", |harness| half_block(harness, HIGH, TOP, TOP));
    let pixels = same_cover(&mut harness, Graphics::Sixel, "an empty floor");
    assert!(picture_cells(&harness).is_empty(), "no half block is left:\n{}", harness.screen());
    assert_eq!(usize::from(SIZE.0) * usize::from(SIZE.1 - 1), pixels, "the whole floor is the terminal's pixels");
    harness.press("space");
    harness.type_text("Settings");
    harness.press("enter");
    assert_eq!(same_cover(&mut harness, Graphics::Sixel, "a Settings window"), 0, "a window turns it to half blocks");
}
