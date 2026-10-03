//! Installing an application whose installer is not on the machine: qdesk offers to install the
//! installer with cargo in a window of its own, and starts nothing before the person says so.
//!
//! Every test starts where a person starts: a card of the launcher's Installable shelf or the item
//! of its menu, then the dialog's buttons by click or by key. No real cargo, quvyta or qpac ever
//! runs. The desktop is given a `PATH` of the test's own, holding a `cargo`, a `quvyta` and a
//! `qpac` the test wrote, each of which writes down the words it was given. The catalog still
//! decides which applications count as installed, so quvyta and qpac are missing from it while
//! their stand-ins wait on the `PATH`.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::Desk;
use qdesk::apps::{Catalog, Environment, Folders, Launch, load};
use qdesk::desktop::Desktop;
use qframe::event::{MouseButton, MouseKind};
use qframe::prelude::*;

use support::{BUDGET, HARMLESS, HOME, ICONS, MACHINE, MOMENT, OFFSET, PATIENCE};

/// A folder of one test in the temporary folder, taken away with everything in it when the test
/// ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-install-here-{}-{once}", std::process::id()));
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

/// The usual floor at 100 × 30 in the environment of `scratch`, with `catalog`.
fn desk_with(scratch: &Scratch, catalog: Catalog) -> Harness<Desk> {
    let desktop = Desktop {
        icons: ICONS.map(str::to_owned).to_vec(),
        welcome_seen: true,
        recommended_seen: true,
        resize_hint_seen: true,
        ..Desktop::default()
    };
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), Box::new(|| MOMENT * 1_000))
        .apps(scratch.environment())
        .catalog(catalog)
        .desktop(desktop)
        .watch_within(PATIENCE);
    support::draw(app, 100, 30)
}

/// The usual floor, where neither quvyta nor qpac is installed.
fn desk(scratch: &Scratch) -> Harness<Desk> {
    desk_with(scratch, support::catalog())
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

/// Opens the launcher on its Installable shelf.
fn installable(harness: &mut Harness<Desk>) {
    harness.press("space");
    harness.click_text("Installable");
}

/// Clicks the card of `name` on the Installable shelf once, which asks for it to be installed.
fn click_card(harness: &mut Harness<Desk>, name: &str) {
    installable(harness);
    let (x, y) = harness.find(name).unwrap_or_else(|| panic!("{name} is on the shelf:\n{}", harness.screen()));
    harness.click(x, y);
}

/// Chooses "Install `name` with quvyta" in the menu of its card.
fn menu_install(harness: &mut Harness<Desk>, name: &str) {
    installable(harness);
    let (x, y) = harness.find(name).unwrap_or_else(|| panic!("{name} is on the shelf:\n{}", harness.screen()));
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    harness.click_text(&format!("Install {name} with quvyta"));
}

/// The screen with its rows joined and runs of spaces made one, so a sentence a dialog wrapped
/// over two rows is found whole. Of a row with a pillar only what stands right of it is kept: the
/// dialog's text, not the launcher's shelves beside it.
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

#[test]
fn a_missing_installer_is_offered_with_its_command_and_cancel_starts_nothing() {
    let scratch = Scratch::new();
    scratch.program("cargo");
    let mut harness = desk(&scratch);
    click_card(&mut harness, "qfocus");
    let question = flat(&harness);
    assert!(question.contains("Install quvyta here?"), "{question}");
    assert!(
        question.contains("cargo install --locked quvyta && quvyta show quvyta-focus"),
        "the dialog shows the exact command:\n{}",
        harness.screen()
    );
    assert!(question.contains("Install here") && question.contains("Cancel"), "{question}");
    assert!(harness.app().windows().is_empty(), "nothing starts before the answer");
    press(&mut harness, "Cancel");
    assert!(!harness.screen().contains("Install quvyta here?"), "{}", harness.screen());
    harness.render();
    assert!(harness.app().windows().is_empty(), "a cancelled question opens no window:\n{}", harness.screen());
    assert!(asked(&scratch, "cargo").is_empty(), "a cancelled question runs no cargo");
}

#[test]
fn escape_answers_the_install_question_as_cancel() {
    let scratch = Scratch::new();
    scratch.program("cargo");
    let mut harness = desk(&scratch);
    click_card(&mut harness, "btop");
    assert!(flat(&harness).contains("Install qpac here?"), "{}", harness.screen());
    harness.press("esc");
    assert!(!harness.screen().contains("Install qpac here?"), "{}", harness.screen());
    harness.render();
    assert!(harness.app().windows().is_empty(), "{}", harness.screen());
    assert!(asked(&scratch, "cargo").is_empty());
}

#[test]
fn install_here_installs_quvyta_with_cargo_in_a_window_and_opens_it_on_the_member() {
    let scratch = Scratch::new();
    scratch.program("cargo");
    scratch.program("quvyta");
    let mut harness = desk(&scratch);
    menu_install(&mut harness, "qfocus");
    press(&mut harness, "Install here");
    assert!(harness.app().launcher().is_none(), "the launcher makes way for the window");
    let words = window_words(&harness);
    let cargo = scratch.0.join("bin/cargo").display().to_string();
    assert_eq!(&words[4..], [cargo.as_str(), "quvyta", "quvyta", "show", "quvyta-focus"], "{words:?}");
    until(&mut harness, "quvyta being started", |_| !asked(&scratch, "quvyta").is_empty());
    assert_eq!(asked(&scratch, "cargo"), "install\n--locked\nquvyta\n");
    assert_eq!(asked(&scratch, "quvyta"), "show\nquvyta-focus\n");
    assert!(harness.screen().contains("Installing quvyta"), "the window says what it does:\n{}", harness.screen());
}

#[test]
fn install_here_by_keyboard_installs_qpac_and_opens_it() {
    let scratch = Scratch::new();
    scratch.program("cargo");
    scratch.program("qpac");
    let mut harness = desk(&scratch);
    click_card(&mut harness, "btop");
    let question = flat(&harness);
    assert!(question.contains("cargo install --locked quvyta-packages && qpac"), "{}", harness.screen());
    // Cancel has the focus when the question opens; Tab walks to Install here.
    harness.press("tab");
    harness.press("enter");
    until(&mut harness, "qpac being started", |_| scratch.asked("qpac").exists());
    assert_eq!(asked(&scratch, "cargo"), "install\n--locked\nquvyta-packages\n");
    // With no words the stand-in writes one empty line.
    assert_eq!(asked(&scratch, "qpac"), "\n", "qpac is opened as it is, with no words");
    assert_eq!(harness.app().windows().len(), 1);
}

#[test]
fn a_failed_cargo_install_does_not_start_the_installer() {
    let scratch = Scratch::new();
    scratch.program("quvyta");
    let cargo = scratch.0.join("bin/cargo");
    {
        use std::os::unix::fs::PermissionsExt;
        let body = format!("#!/bin/sh\nprintf '%s\\n' \"$@\" >> '{}'\nexit 101\n", scratch.asked("cargo").display());
        fs::write(&cargo, body).expect("cargo is written");
        fs::set_permissions(&cargo, fs::Permissions::from_mode(0o755)).expect("cargo can be run");
    }
    let mut harness = desk(&scratch);
    click_card(&mut harness, "qfocus");
    press(&mut harness, "Install here");
    until(&mut harness, "the window's program ending", |harness| {
        harness.app().windows().iter().all(|window| window.run() != Some(qdesk::wm::Run::Running))
    });
    assert_eq!(asked(&scratch, "cargo"), "install\n--locked\nquvyta\n");
    assert!(asked(&scratch, "quvyta").is_empty(), "no quvyta after a failed install");
    assert_eq!(harness.app().windows().len(), 1, "the window stays to show what went wrong");
}

#[test]
fn without_cargo_the_notice_says_what_is_missing_and_offers_no_button() {
    let scratch = Scratch::new();
    let mut harness = desk(&scratch);
    click_card(&mut harness, "qfocus");
    let screen = flat(&harness);
    assert!(!screen.contains("Install here"), "no button onto a missing cargo:\n{}", harness.screen());
    assert!(screen.contains("Installing qfocus"), "{screen}");
    assert!(screen.contains("cargo"), "the notice names what is missing:\n{}", harness.screen());
    assert!(harness.app().windows().is_empty());
}

#[test]
fn an_installed_quvyta_is_opened_on_the_member_without_a_question() {
    let scratch = Scratch::new();
    scratch.program("cargo");
    scratch.program("quvyta");
    let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
    let entries = load(&folders, Some(Path::new(HOME))).entries;
    let catalog = Catalog::new(entries, |entry| matches!(entry.launch, Launch::Screen(_)) || entry.id == "quvyta");
    let mut harness = desk_with(&scratch, catalog);
    menu_install(&mut harness, "qfocus");
    assert!(!flat(&harness).contains("Install here"), "{}", harness.screen());
    until(&mut harness, "quvyta being started", |_| !asked(&scratch, "quvyta").is_empty());
    assert_eq!(asked(&scratch, "quvyta"), "show\nquvyta-focus\n");
    assert!(asked(&scratch, "cargo").is_empty(), "nothing is installed");
}
