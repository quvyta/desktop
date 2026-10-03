//! The panel of the first start: the welcome line with the recommended applications this machine
//! does not have, each installed by the launcher's own install, and Settings opening it again.
//!
//! Every test starts where a person starts: the panel's buttons and the Settings row are clicked
//! where they are drawn. No real cargo, quvyta or qpac ever runs. The desktop is given a `PATH` of
//! the test's own, holding a `cargo`, a `quvyta` and a `qpac` the test wrote, each of which writes
//! down the words it was given. The catalog decides which applications count as installed: the
//! pretend machine of `support` has qcode and not qexp, w3m, qfocus or qtools.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::Desk;
use qdesk::apps::{Catalog, Environment, Folders, Launch, load};
use qdesk::desktop::Desktop;
use qframe::color::ColorDepth;
use qframe::icons::GlyphMode;
use qframe::prelude::*;

use support::{BUDGET, HARMLESS, HOME, ICONS, MACHINE, MOMENT, OFFSET, PATIENCE, decoration};

/// A folder of one test in the temporary folder, taken away with everything in it when the test
/// ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-recommended-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("bin")).expect("the PATH folder is made");
        Self(path)
    }

    /// Puts a `program` on the test's `PATH` that writes the words it was given, one to a line,
    /// into the file [`asked`](Self::asked) names.
    fn program(&self, program: &str) {
        use std::os::unix::fs::PermissionsExt;
        let path = self.0.join("bin").join(program);
        let body = format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\n", self.asked(program).display());
        fs::write(&path, body).expect("the program is written");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("the program can be run");
    }

    /// Where the stand-in `program` writes what it was asked.
    fn asked(&self, program: &str) -> PathBuf {
        self.0.join(format!("{program}-asked"))
    }

    /// The desktop file of this test.
    fn file(&self) -> PathBuf {
        self.0.join("desktop.toml")
    }

    fn environment(&self) -> Environment {
        Environment {
            home: Some(self.0.clone()),
            path: Some(self.0.join("bin").into_os_string()),
            shell: Some(PathBuf::from(HARMLESS)),
            ..Environment::default()
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// What the stand-in `program` was asked, empty when it never ran.
fn asked(scratch: &Scratch, program: &str) -> String {
    fs::read_to_string(scratch.asked(program)).unwrap_or_default()
}

/// A desktop as its file in `scratch` says, or an untouched one when there is no file yet, with
/// `catalog`, at `width` × `height`: what one run writes, the next run starts from.
fn desk_with(scratch: &Scratch, catalog: Catalog, width: u16, height: u16) -> Harness<Desk> {
    let (mut desktop, problems) = Desktop::load(&scratch.file());
    assert!(problems.is_empty(), "{problems:?}");
    desktop.icons = ICONS.map(str::to_owned).to_vec();
    // The note on resizing that comes with the first window is not what these tests look at.
    desktop.resize_hint_seen = true;
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), Box::new(|| MOMENT * 1_000))
        .apps(scratch.environment())
        .catalog(catalog)
        .desktop(desktop)
        .config(Some(scratch.file()))
        .watch_within(PATIENCE);
    support::draw(app, width, height)
}

/// The pretend machine at 80 × 24, the smallest usual terminal.
fn desk(scratch: &Scratch) -> Harness<Desk> {
    desk_with(scratch, support::catalog(), 80, 24)
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

/// Where the Install button on the row of `name` is drawn: the row that names it as a word and
/// ends in Install.
fn install_button(harness: &Harness<Desk>, name: &str) -> Option<(i32, i32)> {
    let screen = harness.screen();
    let (y, row) = screen
        .lines()
        .enumerate()
        .find(|(_, row)| row.split_whitespace().any(|word| word == name) && row.trim_end().ends_with("Install"))?;
    let at = row.rfind("Install")?;
    let x = row[..at].chars().count();
    Some((i32::try_from(x).ok()?, i32::try_from(y).ok()?))
}

/// Clicks Install on the row of `name`.
fn install(harness: &mut Harness<Desk>, name: &str) {
    let (x, y) = install_button(harness, name)
        .unwrap_or_else(|| panic!("no Install on the row of {name}:\n{}", harness.screen()));
    harness.click(x + 1, y);
}

/// Clicks `label` on the lowest row it stands on that is not a question: a dialog's buttons are
/// below its title and its message.
fn press(harness: &mut Harness<Desk>, label: &str) {
    let screen = harness.screen();
    let rows: Vec<&str> = screen.lines().collect();
    let (y, row) = rows
        .iter()
        .enumerate()
        .rev()
        .find(|(_, row)| row.contains(label) && !row.contains('?'))
        .unwrap_or_else(|| panic!("no {label} to press:\n{screen}"));
    let x = row[..row.find(label).unwrap_or(0)].chars().count();
    harness.click(i32::try_from(x).unwrap_or(0) + 1, i32::try_from(y).unwrap_or(0));
}

/// The screen with its rows joined and runs of spaces made one, so a sentence wrapped over two
/// rows is found whole.
fn flat(harness: &Harness<Desk>) -> String {
    let screen = harness.screen();
    let rows: Vec<&str> = screen.lines().map(|row| row.rsplit('▌').next().unwrap_or(row)).collect();
    rows.join(" ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The words the only open window runs.
fn window_words(harness: &Harness<Desk>) -> Vec<String> {
    let windows = harness.app().windows();
    assert_eq!(windows.len(), 1, "one window:\n{}", harness.screen());
    match &windows.iter().next().expect("a window").entry().launch {
        Launch::Command(words) => words.clone(),
        other => panic!("the window runs no command: {other:?}"),
    }
}

const HEADING: &str = "Recommended applications";

#[test]
fn the_first_start_offers_the_missing_recommended_applications_and_the_second_does_not() {
    let scratch = Scratch::new();
    let mut harness = desk(&scratch);
    let screen = harness.screen();
    assert!(screen.contains("The applications are behind"), "the welcome line is in the panel:\n{screen}");
    assert!(screen.contains(HEADING), "{screen}");
    for name in ["qexp", "w3m", "qfocus", "qtools"] {
        assert!(install_button(&harness, name).is_some(), "{name} has a row with Install:\n{screen}");
    }
    assert!(flat(&harness).contains("A text web browser"), "a row says what the application is:\n{screen}");
    assert!(flat(&harness).contains("Settings can show this again."), "{screen}");
    assert_eq!(decoration(&screen), None, "{screen}");

    harness.click_text("Got it");
    assert!(!harness.screen().contains(HEADING), "{}", harness.screen());
    assert!(harness.is_focused("floor"), "the floor has the keys again");
    let written = fs::read_to_string(scratch.file()).expect("the desktop file is written");
    assert!(written.contains("recommended_seen = true"), "{written}");

    let second = desk(&scratch);
    let screen = second.screen();
    assert!(!screen.contains(HEADING) && !screen.contains("The applications are behind"), "{screen}");
}

#[test]
fn an_installed_recommended_application_has_no_row() {
    let scratch = Scratch::new();
    let harness = desk(&scratch);
    // qcode is on the pretend machine; the other four are not.
    assert!(harness.screen().contains(HEADING), "{}", harness.screen());
    assert!(install_button(&harness, "qcode").is_none(), "an installed program is not offered:\n{}", harness.screen());
    assert!(install_button(&harness, "qfocus").is_some(), "{}", harness.screen());
}

#[test]
fn with_every_recommended_application_installed_only_the_welcome_line_is_shown() {
    let scratch = Scratch::new();
    let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
    let entries = load(&folders, Some(Path::new(HOME))).entries;
    let catalog = Catalog::new(entries, |_| true);
    let harness = desk_with(&scratch, catalog, 80, 24);
    let screen = harness.screen();
    assert!(screen.contains("The applications are behind"), "{screen}");
    assert!(!screen.contains(HEADING), "nothing to offer, no offer:\n{screen}");
}

#[test]
fn install_on_a_row_asks_the_launchers_question_and_installs_the_member_in_a_window() {
    let scratch = Scratch::new();
    scratch.program("cargo");
    scratch.program("quvyta");
    let mut harness = desk(&scratch);
    install(&mut harness, "qfocus");
    let question = flat(&harness);
    assert!(!harness.screen().contains(HEADING), "the panel makes way:\n{}", harness.screen());
    assert!(question.contains("Install quvyta here?"), "the installer is missing, so it is asked for:\n{question}");
    assert!(question.contains("cargo install --locked quvyta && quvyta show quvyta-focus"), "{question}");
    assert!(harness.app().windows().is_empty(), "nothing starts before the answer");
    press(&mut harness, "Install here");
    let words = window_words(&harness);
    let cargo = scratch.0.join("bin/cargo").display().to_string();
    assert_eq!(&words[4..], [cargo.as_str(), "quvyta", "quvyta", "show", "quvyta-focus"], "{words:?}");
    until(&mut harness, "quvyta being started", |_| !asked(&scratch, "quvyta").is_empty());
    assert_eq!(asked(&scratch, "cargo"), "install\n--locked\nquvyta\n");
    assert_eq!(asked(&scratch, "quvyta"), "show\nquvyta-focus\n");
    let written = fs::read_to_string(scratch.file()).expect("the desktop file is written");
    assert!(written.contains("recommended_seen = true"), "an install counts as seen:\n{written}");
}

#[test]
fn install_on_a_row_with_the_installer_there_opens_qpac_on_the_package() {
    let scratch = Scratch::new();
    scratch.program("cargo");
    scratch.program("qpac");
    let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
    let entries = load(&folders, Some(Path::new(HOME))).entries;
    let catalog = Catalog::new(entries, |entry| {
        matches!(entry.launch, Launch::Screen(_)) || ["qpac", "qcode"].contains(&&*entry.id)
    });
    let mut harness = desk_with(&scratch, catalog, 80, 24);
    install(&mut harness, "w3m");
    assert!(!flat(&harness).contains("Install here"), "qpac is there, nothing to ask:\n{}", harness.screen());
    until(&mut harness, "qpac being started", |_| scratch.asked("qpac").exists());
    assert!(asked(&scratch, "cargo").is_empty(), "nothing is installed with cargo");
    assert_eq!(harness.app().windows().len(), 1, "qpac has its window:\n{}", harness.screen());
}

#[test]
fn the_settings_row_opens_the_panel_again() {
    let scratch = Scratch::new();
    let mut harness = desk(&scratch);
    harness.click_text("Got it");
    let (x, y) = harness.find("Settings").expect("the Settings icon is on the floor");
    harness.click(x, y);
    harness.click(x, y);
    assert_eq!(harness.app().windows().len(), 1, "the window opened:\n{}", harness.screen());
    assert!(support::scroll_to(&mut harness, "Recommended applications"), "{}", harness.screen());
    support::click_on_row(&mut harness, "Recommended applications", "Show");
    let screen = harness.screen();
    assert!(install_button(&harness, "qexp").is_some(), "the panel is back:\n{screen}");
    assert!(!screen.contains("The applications are behind"), "without the welcome line:\n{screen}");
    press(&mut harness, "Close");
    assert!(install_button(&harness, "qexp").is_none(), "{}", harness.screen());
}

#[test]
fn the_panel_keeps_its_shape_on_a_narrow_screen_in_ascii_and_in_sixteen_colours() {
    let scratch = Scratch::new();
    for (width, height) in [(80, 24), (59, 20), (80, 15), (45, 12)] {
        let mut harness = desk_with(&scratch, support::catalog(), width, height);
        harness.set_glyph_mode(GlyphMode::Ascii).set_depth(ColorDepth::Ansi16).render();
        let screen = harness.screen();
        assert_eq!(decoration(&screen), None, "{width}x{height}:\n{screen}");
        for name in ["qexp", "w3m", "qfocus", "qtools"] {
            assert!(install_button(&harness, name).is_some(), "{width}x{height}: {name}:\n{screen}");
        }
        assert!(screen.contains("Got it"), "{width}x{height}: the way out is on screen:\n{screen}");
        // Centred: as much floor on the left of the heading as on the right of the buttons.
        let (left, _) = harness.find(HEADING).expect("the heading is on screen");
        let (install, _) = install_button(&harness, "qexp").expect("a row");
        let right = i32::from(width) - (install + 7);
        assert!((left - right).abs() <= 4, "{width}x{height}: {left} on the left, {right} on the right:\n{screen}");
        let narrow = width < 60 || height < 16;
        assert_eq!(
            flat(&harness).contains("A text web browser"),
            !narrow,
            "{width}x{height}: a narrow screen drops the descriptions:\n{screen}"
        );
    }
}
