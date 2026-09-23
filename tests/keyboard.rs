//! One scenario from end to end, with the keyboard and nothing else: the measure of step D7.
//!
//! Not one call in this file touches the mouse. The desktop is opened, an application is found in
//! the launcher and opened in a window, the window is moved and sized, the help is read, a
//! notification arrives and its list is opened and answered, a window is closed and qdesk is left —
//! all of it with keys. If the desktop needed a pointer for any of that, this test would say where.
//!
//! The programs are ones this file writes itself (`/bin/sh -c ...`), never one of the person's, and
//! nothing here is timed: the one thing whose **order** matters — the program ending after its
//! window has lost the keys — is told to happen by a file, and what is waited for is waited for in
//! a loop bounded by frames.

mod support;

use std::path::{Path, PathBuf};
use std::time::Duration;

use qdesk::app::{Desk, Keys};
use qdesk::apps::{Catalog, Declared, Entry, Launch, Source, parse_entry};
use qdesk::desktop::Desktop;
use qframe::geometry::Rect;
use qframe::prelude::*;

use support::{HOME, harness_with};

/// The most frames the test draws while waiting for something a program said to reach the screen.
const FRAMES: usize = 400;

/// A program that stays alive waiting to be typed into, for as long as its window lets it.
const WAITS: &str = "while read satir; do :; done";

/// An entry of this test that runs `command`.
fn entry(id: &str, name: &str, command: &[&str]) -> Entry {
    let words = command
        .iter()
        .map(|word| {
            assert!(!word.contains('"'), "a test command is written without double quotes");
            format!("\"{}\"", word.replace('\\', "\\\\"))
        })
        .collect::<Vec<String>>()
        .join(", ");
    let text = format!("name = \"{name}\"\ncommand = [{words}]\ncategory = \"system\"\n");
    let file = format!("{HOME}/.local/share/quvyta/desktop/apps/{id}.toml");
    let (declared, diagnostics) =
        parse_entry(id, Path::new(&file), text.as_bytes(), Source::User, Some(Path::new(HOME)));
    assert!(diagnostics.is_empty(), "{id}: {diagnostics:?}");
    match declared {
        Some(Declared::Entry(entry)) => *entry,
        other => panic!("{id} declares no entry: {other:?}"),
    }
}

/// A path of this test's own to tell a program with; the caller removes it.
fn signal_path(what: &str) -> PathBuf {
    std::env::temp_dir().join(format!("qdesk-test-{what}-{}", std::process::id()))
}

/// The last row of the screen.
fn dock(harness: &Harness<Desk>) -> String {
    harness.screen().lines().last().expect("the dock row").to_owned()
}

/// Draws until the dock's row carries `text`, or gives up and says what it kept showing.
fn until_dock(harness: &mut Harness<Desk>, text: &str) {
    for _ in 0..FRAMES {
        if dock(harness).contains(text) {
            return;
        }
        harness.render();
    }
    panic!("{text} never reached the dock; the row stayed: {}", dock(harness));
}

/// The rectangle of the window in front.
fn rect(harness: &Harness<Desk>) -> Rect {
    harness.app().windows().front().expect("a window on the desktop").rect()
}

/// The id of the entry the window in front was opened from.
fn front_entry(harness: &Harness<Desk>) -> String {
    harness.app().windows().front().expect("a window on the desktop").entry().id.clone()
}

#[test]
fn the_whole_desktop_is_driven_from_the_keyboard_from_opening_an_application_to_leaving_qdesk() {
    let signal = signal_path("senaryo");
    let _ = std::fs::remove_file(&signal);
    let ending = entry(
        "biten",
        "Biten",
        &["/bin/sh", "-c", &format!("while [ ! -e {} ]; do sleep 0.02; done; exit 4", signal.display())],
    );
    let waiting = entry("bekleyen", "Bekleyen", &["/bin/sh", "-c", WAITS]);
    let catalog = Catalog::new(vec![ending, waiting], |entry| !matches!(entry.launch, Launch::Open(_)));
    // A floor with no icons on it: everything below goes through the launcher, which is the way a
    // person who knows what they want reaches an application.
    let desktop = Desktop { icons: Vec::new(), recents: Vec::new(), welcome_seen: true, ..Desktop::default() };
    let mut harness = harness_with(catalog, desktop, 100, 30);

    // The launcher, from the floor: space, a few letters, Enter.
    harness.press("space");
    assert!(harness.app().launcher().is_some(), "space opened the launcher:\n{}", harness.screen());
    harness.type_text("bit");
    assert!(harness.screen().contains("Biten"), "the search found it:\n{}", harness.screen());
    harness.press("enter");
    assert_eq!(harness.app().windows().len(), 1, "Enter opened it in a window:\n{}", harness.screen());
    assert_eq!(front_entry(&harness), "biten");
    assert!(harness.app().launcher().is_none(), "and the launcher stepped out of the way");

    // A second application. The keys are the program's now, so the way to the launcher is through
    // desktop mode — the door the design puts on every window (3.3).
    harness.press("ctrl+alt+space");
    assert_eq!(harness.app().keys(), Some(Keys::Pick));
    harness.press("space");
    harness.type_text("bek");
    harness.press("enter");
    assert_eq!(harness.app().windows().len(), 2, "the second window opened:\n{}", harness.screen());
    assert_eq!(front_entry(&harness), "bekleyen");

    // Desktop mode again: pick, move and size the window in front.
    harness.press("ctrl+alt+space");
    let picked = rect(&harness);
    harness.press("m").press("right").press("right").press("down");
    assert_eq!(rect(&harness), Rect { x: picked.x + 2, y: picked.y + 1, ..picked }, "the arrows moved it");
    harness.press("enter");
    assert_eq!(harness.app().keys(), Some(Keys::Pick), "Enter let go of it");
    harness.press("r").press("left").press("up");
    let sized = rect(&harness);
    assert_eq!(
        (sized.width, sized.height),
        (picked.width - 1, picked.height - 1),
        "the arrows sized it from its right and bottom edge"
    );
    harness.press("enter");

    // The help, and back out of it.
    harness.press("f1");
    let help = harness.screen();
    assert!(help.contains("Keyboard shortcuts"), "f1 listed the keys:\n{help}");
    assert!(help.contains("Desktop mode: close the window"), "and they are the desktop's own:\n{help}");
    harness.press("esc");
    assert!(!harness.screen().contains("Keyboard shortcuts"), "esc closed the help");
    assert_eq!(harness.app().keys(), Some(Keys::Pick), "and left desktop mode as it was");

    // A notification, while the person is looking at the other window: the first program ends.
    harness.press("esc");
    assert_eq!(harness.app().keys(), None, "esc gave the keys back to the window");
    std::fs::write(&signal, b"").expect("the signal file is written");
    until_dock(&mut harness, "●1");
    let _ = std::fs::remove_file(&signal);

    // Its list, opened and answered from the keyboard: the window it came from comes forward.
    // The corner's own word goes by itself; what is left is the list.
    harness.advance(Duration::from_secs(30));
    harness.press("ctrl+alt+space").press("b");
    let listed = harness.screen();
    assert!(listed.contains("Notifications"), "the list opened:\n{listed}");
    assert!(
        listed.contains("Biten — The program ended, exit code 4."),
        "and it names the window it came from:\n{listed}"
    );
    harness.press("enter");
    assert_eq!(front_entry(&harness), "biten", "the window came forward:\n{}", harness.screen());
    assert!(!harness.screen().contains("Notifications"), "and the list stepped out of the way");
    assert_eq!(harness.app().keys(), Some(Keys::Pick), "answering the list left desktop mode standing");

    // Closing that window: its program has ended, so nothing is asked.
    harness.press("x");
    assert_eq!(harness.app().windows().len(), 1, "x closed it:\n{}", harness.screen());
    assert_eq!(harness.app().windows().front().map(|window| window.entry().id.clone()), Some("bekleyen".to_owned()));
    harness.press("esc");
    assert_eq!(harness.app().keys(), None, "esc gave the keys back to the window that is left");
    assert!(!dock(&harness).contains('●'), "the list was read: {}", dock(&harness));

    // And leaving qdesk, with a program still running: the question is answered with keys too.
    harness.press("ctrl+q");
    assert!(!harness.quit_requested(), "it waits for an answer:\n{}", harness.screen());
    assert!(harness.screen().contains("1 program is still running"), "counted:\n{}", harness.screen());
    harness.press("tab").press("enter");
    assert!(harness.quit_requested(), "answered, qdesk leaves:\n{}", harness.screen());
}
