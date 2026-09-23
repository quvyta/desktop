//! The notifications on the dock: the count of what has not been read at the row's right end, and
//! the list it opens (design 3.4 and 3.6).
//!
//! Every program here is one this file writes itself (`/bin/sh -c ...`): a screen test never runs
//! a program of the person's. Nothing here is timed; what a program says is waited for in a loop
//! bounded by frames.

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use qdesk::app::{Ask, Desk, Msg, Target};
use qdesk::apps::{Catalog, Declared, Entry, Launch, Source, parse_entry};
use qdesk::desktop::Desktop;
use qdesk::wm::Window;
use qframe::prelude::*;

use support::{BUDGET, HOME, decoration, harness_with};

/// What a program that must still be running does: waits to be typed into, for as long as qdesk
/// lets it. A program that slept instead would be alive only for as long as the sleep, and on a
/// loaded machine the test would fail while nothing was wrong.
const WAITS: &str = "while read satir; do :; done";

/// A path of this test's own to tell a program with: the name, this process, and a number no
/// other call gets.
///
/// The number matters. Tests of one binary are threads of one process, so a path built from the
/// process alone is shared by every test that asks for the same name — and each of them clears
/// the file before it starts, which takes the signal away from whichever test is already waiting
/// on it. That is a race between tests, and it shows up as a program that never does what it was
/// told, on a machine busy enough to interleave them.
fn signal_path(what: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let once = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("qdesk-test-{what}-{}-{once}", std::process::id()))
}

/// Shell words for a program that waits until the file `signal` appears and then does `then`.
///
/// A program told what to do by a file is the only way to be sure of the **order** of two things:
/// that the program ends, or rings, after its window has lost the keys and not a moment before.
fn on_signal(signal: &Path, then: &str) -> String {
    format!("while [ ! -e {} ]; do sleep 0.02; done; {then}", signal.display())
}

/// An entry of this test that runs `command`.
fn entry(id: &str, name: &str, command: &[&str]) -> Entry {
    // The words go into a TOML string, so their backslashes are doubled.
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

/// A program that ends at once with the code `code`.
fn done(id: &str, name: &str, code: u8) -> Entry {
    entry(id, name, &["/bin/sh", "-c", &format!("exit {code}")])
}

/// A desktop whose catalog holds `entries`, all of them installed, and nothing on its floor.
fn desk(entries: Vec<Entry>, width: u16, height: u16) -> Harness<Desk> {
    let catalog = Catalog::new(entries, |entry| !matches!(entry.launch, Launch::Open(_)));
    let desktop = Desktop { icons: Vec::new(), recents: Vec::new(), welcome_seen: true, ..Desktop::default() };
    harness_with(catalog, desktop, width, height)
}

fn open(harness: &mut Harness<Desk>, entry: &Entry) {
    harness.send(Msg::Open(Target::of(entry, "en")));
}

/// The last row of the screen.
fn dock(harness: &Harness<Desk>) -> String {
    harness.screen().lines().last().expect("the dock row").to_owned()
}

/// Renders until the dock's row carries `text`, or gives up and says what it kept showing.
fn until_dock(harness: &mut Harness<Desk>, text: &str) {
    // Bounded by a length of time, not by a number of frames: a frame costs whatever the machine
    // and the programs in the windows make it cost, so counting frames would measure how busy the
    // machine is rather than how long the desktop is being given.
    let deadline = Instant::now() + BUDGET;
    loop {
        if dock(harness).contains(text) {
            return;
        }
        assert!(Instant::now() < deadline, "{text} never reached the dock; the row stayed: {}", dock(harness));
        harness.render();
    }
}

/// A desktop where one window's program ended while another had the keys: one notification, unread.
///
/// The program is told to end by a file, after the window beside it has taken the keys, so the end
/// really is heard while the person is looking elsewhere — the case the design notifies about.
fn one_notification() -> Harness<Desk> {
    let signal = signal_path("biten");
    let _ = std::fs::remove_file(&signal);
    let ending = entry("biten", "Biten", &["/bin/sh", "-c", &on_signal(&signal, "exit 4")]);
    let other = entry("iki", "İki", &["/bin/sh", "-c", WAITS]);
    let mut harness = desk(vec![ending.clone(), other.clone()], 100, 30);
    open(&mut harness, &ending);
    open(&mut harness, &other);
    std::fs::write(&signal, b"").expect("the signal file is written");
    until_dock(&mut harness, "●1");
    let _ = std::fs::remove_file(&signal);
    harness
}

#[test]
fn a_quiet_desktop_counts_nothing_on_its_dock() {
    let harness = desk(vec![done("bir", "Bir", 0)], 100, 30);
    let row = dock(&harness);
    assert!(!row.contains('●'), "nothing has been said: {row}");
    assert!(row.contains("sunucu-1") && row.contains("14:32"), "{row}");
}

#[test]
fn a_program_that_ended_elsewhere_is_counted_and_the_list_names_its_window() {
    let mut harness = one_notification();
    let row = dock(&harness);
    assert!(row.contains("●1"), "one unread notification: {row}");
    // The corner's own word goes by itself; what is left is the list, which is the whole point of
    // writing it down. The harness's clock is moved on, never slept on.
    harness.advance(Duration::from_secs(30));
    harness.click_text("●1");
    let screen = harness.screen();
    assert!(screen.contains("Notifications"), "the list opened:\n{screen}");
    assert!(
        screen.contains("Biten — The program ended, exit code 4."),
        "the whole row is there, its window named:\n{screen}"
    );
    assert_eq!(decoration(&screen), None, "the list is drawn in tones:\n{screen}");
    // Reading it takes the count to zero, and the row keeps its other parts.
    harness.press("esc");
    let row = dock(&harness);
    assert!(!row.contains('●'), "nothing is unread any more: {row}");
    assert!(row.contains("sunucu-1"), "{row}");
}

#[test]
fn choosing_a_notification_brings_its_window_forward_and_closes_the_list() {
    let mut harness = one_notification();
    assert_eq!(harness.app().windows().front().map(|window| window.entry().id.clone()), Some("iki".to_owned()));
    harness.click_text("●1");
    // The keys land on the first notice that leads somewhere, so Enter alone answers it.
    harness.press("enter");
    let front = harness.app().windows().front().map(|window| window.entry().id.clone());
    assert_eq!(front, Some("biten".to_owned()), "the window came forward:\n{}", harness.screen());
    assert!(!harness.screen().contains("Notifications"), "and the list stepped out of the way");
}

#[test]
fn the_keys_open_the_list_in_desktop_mode_and_esc_puts_it_away() {
    let mut harness = one_notification();
    harness.press("ctrl+alt+space").press("b");
    let screen = harness.screen();
    assert!(screen.contains("Notifications") && screen.contains("exit code 4"), "b opened the list:\n{screen}");
    harness.press("esc");
    // The toast of the program that ended may still be in the corner; what has to be gone is the
    // list itself.
    assert!(!harness.screen().contains("Notifications"), "esc put it away:\n{}", harness.screen());
    assert!(harness.app().keys().is_some(), "and left desktop mode alone");
}

#[test]
fn the_hint_line_of_desktop_mode_names_the_key_that_opens_the_list() {
    let mut harness = one_notification();
    // The hints drop from the end of a row that is too narrow for them, and the notifications are
    // the last of them: a wide row says them all.
    harness.resize(160, 30);
    harness.press("ctrl+alt+space");
    let hints = dock(&harness);
    assert!(hints.contains("notices"), "the hint line says what b does: {hints}");
    harness.set_locale("tr");
    assert!(dock(&harness).contains("bildirim"), "and it says it in Turkish: {}", dock(&harness));
}

#[test]
fn a_desktop_with_nothing_to_say_still_opens_a_list_and_says_it_is_empty() {
    let mut harness = desk(vec![done("bir", "Bir", 0)], 100, 30);
    harness.press("ctrl+alt+space").press("b");
    let screen = harness.screen();
    assert!(screen.contains("Nothing has been said yet"), "the empty list is drawn:\n{screen}");
    assert_eq!(decoration(&screen), None, "{screen}");
}

#[test]
fn a_bell_is_counted_and_written_down_without_a_word_in_the_corner() {
    // A bell has no words of its own, so the list says what happened; it stays out of the corner
    // because a shell rings for every completion it cannot finish.
    let signal = signal_path("zil");
    let _ = std::fs::remove_file(&signal);
    let ringing = entry("zil", "Zil", &["/bin/sh", "-c", &on_signal(&signal, "printf \\\\a; exit 0")]);
    let other = entry("iki", "İki", &["/bin/sh", "-c", WAITS]);
    let mut harness = desk(vec![ringing.clone(), other.clone()], 100, 30);
    open(&mut harness, &ringing);
    open(&mut harness, &other);
    std::fs::write(&signal, b"").expect("the signal file is written");
    until_dock(&mut harness, "●");
    let _ = std::fs::remove_file(&signal);
    harness.press("ctrl+alt+space").press("b");
    let screen = harness.screen();
    assert!(screen.contains("rang the bell"), "the list says what the bell was:\n{screen}");
    assert!(screen.contains("Zil"), "and which window rang it:\n{screen}");
}

#[test]
fn a_notification_of_a_program_reaches_both_the_corner_and_the_list() {
    let notifying = entry(
        "bildiren",
        "Bildiren",
        &["/bin/sh", "-c", &format!("printf 'hazir\\033]777;notify;Yedek;tamam\\007'; {WAITS}")],
    );
    let other = done("iki", "İki", 0);
    let mut harness = desk(vec![notifying.clone(), other.clone()], 100, 30);
    open(&mut harness, &notifying);
    open(&mut harness, &other);
    until_dock(&mut harness, "●");
    assert!(harness.screen().contains("Yedek"), "the corner said it:\n{}", harness.screen());
    harness.press("ctrl+alt+space").press("b");
    let screen = harness.screen();
    assert!(screen.contains("Yedek") && screen.contains("tamam"), "and the list kept it:\n{screen}");
}

#[test]
fn a_notification_whose_window_is_closed_keeps_its_words_and_offers_no_way_back() {
    let mut harness = one_notification();
    let id = harness
        .app()
        .windows()
        .iter()
        .find(|window| window.entry().id == "biten")
        .map(Window::id)
        .expect("the window that ended");
    harness.send(Msg::Ask(Ask::Close, id));
    assert!(harness.app().windows().iter().all(|window| window.entry().id != "biten"), "the window closed");
    harness.press("ctrl+alt+space").press("b");
    let screen = harness.screen();
    assert!(screen.contains("exit code 4"), "what it said is still readable:\n{screen}");
    // A row that led to a window that is gone would be a promise nothing can keep, so it is a line
    // and not a button: pressing where it stands raises nothing and the list stays.
    harness.click_text("exit code 4");
    assert!(harness.screen().contains("exit code 4"), "the row did nothing:\n{}", harness.screen());
}

#[test]
fn the_count_gives_way_before_the_machine_name_on_a_narrow_row() {
    let mut harness = one_notification();
    assert!(dock(&harness).contains("●1"));
    harness.resize(40, 20);
    let row = dock(&harness);
    assert!(row.contains("sunucu-1"), "the machine is the last thing to go (3.4): {row}");
    harness.resize(100, 30);
    assert!(dock(&harness).contains("●1"), "a wide row shows it again: {}", dock(&harness));
}
