//! The programs of the windows on the screen: a window opens for an entry and runs what it names,
//! its strip follows the program, the window of a program that ended says how it ended and starts
//! it again, closing a running one asks first, leaving qdesk asks, and the dock carries the windows
//! of a full desktop.
//!
//! Every program here is one this file writes itself (`/bin/sh -c ...`): a screen test never runs
//! a program of the person's, any more than it writes in their folders.
//!
//! A live program can be driven here. The harness runs the wait for what a program says where it
//! stands, so the desktop is given a bound for it (`support::PATIENCE`, the rule is
//! `Desk::watch_within`): a window holding a program that has said its piece and is waiting to be
//! typed into lets the test draw the next frame, and the watch starts again there. The running
//! desktop waits with no bound on a thread of its own and is not touched by this.
//!
//! Nothing here is timed. A bound is a bound and not a sleep: a program that speaks is heard the
//! moment it does, what is waited for is waited for in a loop bounded by frames, and where the
//! **order** of two things matters the program is told when to speak by a file. What the programs
//! do below the screen — the environment they get, the lines they remember, how they end — is
//! proved against a real shell in `src/session`.

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::{Ask, Desk, Msg, Target};
use qdesk::apps::{Catalog, Declared, Entry, Environment, Folders, Launch, Source, load, parse_entry};
use qdesk::desktop::Desktop;
use qdesk::wm::{Run, Window, WindowId};
use qframe::color::Rgb;
use qframe::env::{AssetDirs, Env};
use qframe::event::{MouseButton, MouseKind};
use qframe::geometry::Rect;
use qframe::icons::GlyphMode;
use qframe::prelude::*;

use support::{BUDGET, HARMLESS, HOME, MACHINE, MOMENT, OFFSET, PATIENCE, decoration, harness_with};

/// What a program that must still be running does: waits to be typed into, and answers what it is
/// told, for as long as qdesk lets it.
///
/// It is written this way on purpose. A program that sleeps instead would be alive only for as
/// long as the sleep, and whether the test caught it alive would depend on how fast the machine
/// was that minute — under load such a test fails while nothing is wrong. One waiting on its input
/// is alive until its window ends it, however loaded the machine is, and it says nothing while it
/// waits, so a test that drives it proves the quiet case as well as the talkative one.
///
/// It answers rather than only waits, so a test can show a key really reaching the program: what
/// comes back is the program's own answer to what was typed.
const ANSWERS: &str = "while read satir; do printf 'yanit: %s\\n' $satir; done";

/// The most frames a test draws while waiting for something a program said to reach the screen.
///
/// It is a bound, not a sleep: every frame runs one waiting perform, so a desktop that works gets
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

/// Shell words for a program that says `first`, keeps talking until the file `signal` appears,
/// then says `then` and goes on talking.
///
/// A program told what to do by a file is the only way to be sure of the **order** of two things
/// here: that the bell rings after the window has lost the keys, and not a moment before. Waiting
/// for a number of seconds instead would only make the order likely, and on a loaded machine the
/// wrong order is what the test would catch.
fn on_signal(first: &str, signal: &Path, then: &str) -> String {
    let signal = signal.display();
    format!("printf '{first}'; while [ ! -e {signal} ]; do sleep 0.02; done; printf '{then}'; {ANSWERS}")
}

/// Renders until `ready` is happy with the dock row, or gives up and says what it kept seeing.
///
/// The program is told to speak by a file, so what is waited for here is only the moment its word
/// travels through the pty and the desktop draws again: a bounded wait that can be slow but never
/// wrong. Without a bound a broken desktop would hang the test instead of failing it.
fn until_dock(harness: &mut Harness<Desk>, what: &str, ready: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + BUDGET;
    loop {
        let row = dock(harness);
        if ready(&row) {
            return row;
        }
        assert!(Instant::now() < deadline, "{what} never came; the dock row stayed: {row}");
        harness.render();
    }
}

/// Renders until `ready` is happy with the desktop, or gives up and says what was on the screen.
///
/// Every render runs one waiting perform, so this is how a test steps a program's words through
/// the desktop. The giving up is bounded by [`BUDGET`], a length of time, and not by a number of
/// frames: a frame costs whatever the machine makes it cost, so counting frames measures the
/// machine's mood rather than the desktop's patience.
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

/// Renders until `text` is somewhere on the screen.
fn until_screen(harness: &mut Harness<Desk>, text: &str) {
    until(harness, text, |harness| harness.screen().contains(text));
}

/// Renders until the window in front has heard its program end.
fn until_ended(harness: &mut Harness<Desk>) {
    until(harness, "the end of the program", |harness| matches!(front(harness).run(), Some(Run::Ended { .. })));
}

/// An entry of this test that runs `command`.
fn entry(id: &str, name: &str, command: &[&str]) -> Entry {
    // The words go into a TOML string, so their backslashes are doubled; none of them holds a
    // double quote of its own.
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

/// A program that says a word and then goes on running until it is ended.
fn waiting(id: &str, name: &str) -> Entry {
    entry(id, name, &["/bin/sh", "-c", &format!("printf hazir; {ANSWERS}")])
}

/// The entries qdesk carries inside it.
fn builtins() -> Vec<Entry> {
    let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
    load(&folders, Some(Path::new(HOME))).entries
}

/// One of them, by id.
fn builtin(id: &str) -> Entry {
    builtins().into_iter().find(|entry| entry.id == id).unwrap_or_else(|| panic!("{id} is built in"))
}

/// A desktop whose floor holds `entries`, every one of them installed.
fn desk(entries: Vec<Entry>, width: u16, height: u16) -> Harness<Desk> {
    let icons: Vec<String> = entries.iter().map(|entry| entry.id.clone()).collect();
    let mut all = builtins();
    all.extend(entries);
    // Every program of these tests is one of the machine's own, so every entry is installed.
    let catalog = Catalog::new(all, |entry| !matches!(entry.launch, Launch::Open(_)));
    let desktop = Desktop { icons, recents: Vec::new(), welcome_seen: true };
    harness_with(catalog, desktop, width, height)
}

/// The same desktop as [`desk`], reached over a network instead of locally.
///
/// The question about closing a running program does not read whether the connection is remote —
/// nothing in the close paths does — so a test that wants to be sure has to force the answer
/// rather than read it off the machine it runs on, exactly as `support::desk_over_ssh` does for
/// its own fixed catalog. This one carries `entries` instead, since the running program this file
/// needs to ask about is never one of the built-in applications.
fn desk_over_ssh(entries: Vec<Entry>, width: u16, height: u16) -> Harness<Desk> {
    let icons: Vec<String> = entries.iter().map(|entry| entry.id.clone()).collect();
    let mut all = builtins();
    all.extend(entries);
    let catalog = Catalog::new(all, |entry| !matches!(entry.launch, Launch::Open(_)));
    let desktop = Desktop { icons, recents: Vec::new(), welcome_seen: true };
    let dirs = AssetDirs {
        locale_sources: qdesk::locales().iter().map(|(file, text)| ((*file).to_owned(), (*text).to_owned())).collect(),
        keymap_source: Some({
            let (file, text) = qdesk::keymap();
            (file.to_owned(), text.to_owned())
        }),
        ..AssetDirs::default()
    };
    let env = Env::load(&dirs).expect("the built-in files load");
    let clock = Box::new(|| MOMENT * 1_000);
    let apps = Environment { shell: Some(PathBuf::from(HARMLESS)), ..Environment::default() };
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), clock)
        .apps(apps)
        .catalog(catalog)
        .desktop(desktop)
        .remote(true)
        .watch_within(PATIENCE);
    let mut harness = Harness::with_env(app, env, width, height);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);
    harness
}

/// Opens `entry`, the way the launcher and the floor's menu open one.
fn open(harness: &mut Harness<Desk>, entry: &Entry) {
    harness.send(Msg::Open(Target::of(entry, "en")));
}

/// The window in front.
fn front(harness: &Harness<Desk>) -> &Window {
    harness.app().windows().front().expect("a window on the desktop")
}

/// The rows of the screen.
fn rows(harness: &Harness<Desk>) -> Vec<String> {
    harness.screen().lines().map(str::to_owned).collect()
}

/// The row a window's title strip is drawn on.
fn strip(harness: &Harness<Desk>, rect: Rect) -> String {
    let row = usize::try_from(rect.y).expect("a row on screen");
    rows(harness)[row].clone()
}

/// The last row of the screen.
fn dock(harness: &Harness<Desk>) -> String {
    rows(harness).last().expect("the dock row").clone()
}

/// Where `text` starts on the dock's row, for a right click on a window's item there.
fn dock_cell(harness: &Harness<Desk>, text: &str) -> (i32, i32) {
    let row = dock(harness);
    let x = row.find(text).unwrap_or_else(|| panic!("{text} is not on the dock: {row}"));
    (x as i32, (rows(harness).len() - 1) as i32)
}

#[test]
fn opening_an_icon_of_a_program_opens_a_window_running_it_with_its_own_title_in_the_strip() {
    // A program that names itself and then ends: the name it gave reaches the strip.
    let said = entry("said", "Deneme", &["/bin/sh", "-c", "printf '\\033]0;derleniyor\\007'; exit 0"]);
    let mut harness = desk(vec![said], 80, 24);
    // The first icon of the floor; two clicks open it, as on any desktop.
    harness.click(3, 1);
    harness.click(3, 1);
    assert_eq!(harness.app().windows().len(), 1, "the icon opened a window:\n{}", harness.screen());
    // The program's word travels through the pty first: it is waited for, bounded, not assumed
    // to have come within the first frame.
    until(&mut harness, "the title the program gave itself", |harness| {
        strip(harness, front(harness).rect()).contains("derleniyor")
    });
    let title = strip(&harness, front(&harness).rect());
    assert!(title.contains("Deneme"), "the strip names the application: {title}");
    assert_eq!(decoration(&harness.screen()), None, "no lines, no boxes:\n{}", harness.screen());
}

#[test]
fn a_terminal_window_runs_the_shell_of_the_environment_and_says_where_it_is() {
    let mut harness = desk(Vec::new(), 80, 24);
    open(&mut harness, &builtin("terminal"));
    assert_eq!(harness.app().windows().len(), 1, "{}", harness.screen());
    let window = front(&harness);
    assert!(matches!(window.run(), Some(Run::Running | Run::Ended { .. })), "a program was started: {window:?}");
    let title = strip(&harness, window.rect());
    assert!(title.contains("Terminal"), "{title}");
    // With no title of its own the strip says the folder the program is in.
    let folder = std::env::current_dir().expect("a folder of its own");
    let tail = folder.file_name().expect("a name").to_string_lossy().into_owned();
    assert!(title.contains(&tail) || title.contains('…'), "the folder stands after the name: {title}");
}

#[test]
fn a_window_whose_program_ended_says_how_and_starts_it_again() {
    let ended = done("bitti", "Bitti", 3);
    // A window wide enough for the whole line: on a narrower one the sentence wraps and the two
    // buttons keep their places, which is what the row is measured for.
    let mut harness = desk(vec![ended.clone()], 120, 30);
    open(&mut harness, &ended);
    // How many frames pass before the desktop hears the program go is the machine's business, so
    // it is waited for rather than assumed.
    until_ended(&mut harness);
    let screen = harness.screen();
    let line = screen
        .lines()
        .find(|line| line.contains("Restart"))
        .unwrap_or_else(|| panic!("the line of an ended program:\n{screen}"))
        .to_owned();
    assert!(line.contains("exit code 3"), "the exit code the program really gave: {line}");
    assert!(line.contains("Close"), "both ways on are there: {line}");
    assert!(matches!(front(&harness).run(), Some(Run::Ended { code: Some(3) })));
    assert_eq!(harness.app().windows().len(), 1, "it does not close by itself");
    // Restart runs the same entry again in the same window, and it ends the same way.
    harness.click_text("Restart");
    assert_eq!(harness.app().windows().len(), 1, "the window stayed");
    until_ended(&mut harness);
    assert!(matches!(front(&harness).run(), Some(Run::Ended { code: Some(3) })), "it ran again:\n{}", harness.screen());
}

#[test]
fn a_window_whose_program_ended_closes_without_asking() {
    let ended = done("bitti", "Bitti", 0);
    let mut harness = desk(vec![ended.clone()], 80, 24);
    open(&mut harness, &ended);
    until_ended(&mut harness);
    harness.click_text("Close");
    assert_eq!(harness.app().windows().len(), 0, "nothing was asked:\n{}", harness.screen());
}

#[test]
fn an_entry_that_says_so_closes_its_window_when_its_program_ends() {
    let mut going = done("gecici", "Geçici", 0);
    going.close_on_exit = true;
    let mut harness = desk(vec![going.clone()], 80, 24);
    open(&mut harness, &going);
    until(&mut harness, "the window closing with its program", |harness| harness.app().windows().is_empty());
}

#[test]
fn a_program_that_is_not_there_leaves_the_window_open_and_names_the_program_and_the_reason() {
    let missing = entry("yok", "Yok", &["/nonexistent/qdesk-deneme"]);
    let mut harness = desk(vec![missing.clone()], 80, 24);
    open(&mut harness, &missing);
    assert_eq!(harness.app().windows().len(), 1, "the window stays open");
    let screen = harness.screen();
    assert!(screen.contains("qdesk-deneme"), "the program is named:\n{screen}");
    assert!(screen.contains("could not be started"), "{screen}");
    assert!(screen.contains("Close"), "the only way on is to close it:\n{screen}");
    assert!(!screen.contains("Restart"), "there is nothing to start again:\n{screen}");
    harness.set_locale("tr");
    assert!(harness.screen().contains("başlatılamadı"), "it reads Turkish too:\n{}", harness.screen());
}

#[test]
fn closing_a_window_whose_program_still_runs_asks_first_and_names_the_program() {
    let program = waiting("bekleyen", "Bekleyen");
    let mut harness = desk(vec![program.clone()], 80, 24);
    open(&mut harness, &program);
    let id = front(&harness).id();
    assert!(front(&harness).is_running(), "the program is still going:\n{}", harness.screen());
    harness.send(Msg::Ask(Ask::Close, id));
    let screen = harness.screen();
    assert!(screen.contains("still running"), "the question is on screen:\n{screen}");
    assert!(screen.contains("sh"), "it names the program that would be ended:\n{screen}");
    assert_eq!(harness.app().windows().len(), 1, "nothing closed before it was answered");
    assert_eq!(decoration(&screen), None, "the question is drawn in tones:\n{screen}");
}

#[test]
fn the_question_about_closing_keeps_the_window_on_esc_and_closes_it_when_answered() {
    let program = waiting("bekleyen", "Bekleyen");
    let mut harness = desk(vec![program.clone()], 80, 24);
    open(&mut harness, &program);
    let id = front(&harness).id();
    harness.send(Msg::Ask(Ask::Close, id));
    harness.press("esc");
    assert_eq!(harness.app().windows().len(), 1, "esc is the safe answer:\n{}", harness.screen());
    assert!(!harness.screen().contains("still running"), "and the question is gone:\n{}", harness.screen());

    let program = waiting("bekleyen", "Bekleyen");
    let mut harness = desk(vec![program.clone()], 80, 24);
    open(&mut harness, &program);
    let id = front(&harness).id();
    harness.send(Msg::Ask(Ask::Close, id));
    // Cancel has the keys; the next one along is the answer that closes.
    harness.press("tab").press("enter");
    assert_eq!(harness.app().windows().len(), 0, "answered, the window closes:\n{}", harness.screen());
}

#[test]
fn leaving_qdesk_with_a_program_running_asks_counting_them_and_esc_keeps_it_running() {
    let program = waiting("bekleyen", "Bekleyen");
    let mut harness = desk(vec![program.clone()], 80, 24);
    open(&mut harness, &program);
    harness.press("ctrl+q");
    let screen = harness.screen();
    assert!(!harness.quit_requested(), "it waits for an answer:\n{screen}");
    assert!(screen.contains("1 program is still running"), "the question counts them:\n{screen}");
    assert_eq!(decoration(&screen), None, "the question is drawn in tones:\n{screen}");
    harness.press("esc");
    assert!(!harness.quit_requested(), "esc keeps qdesk running:\n{}", harness.screen());
}

#[test]
fn answering_the_question_about_leaving_ends_qdesk() {
    let program = waiting("bekleyen", "Bekleyen");
    let mut harness = desk(vec![program.clone()], 80, 24);
    open(&mut harness, &program);
    harness.press("ctrl+q");
    assert!(!harness.quit_requested());
    harness.press("tab").press("enter");
    assert!(harness.quit_requested(), "answered, it leaves:\n{}", harness.screen());
}

#[test]
fn leaving_qdesk_with_nothing_running_quits_at_once() {
    let ended = done("bitti", "Bitti", 0);
    let mut harness = desk(vec![ended.clone()], 80, 24);
    open(&mut harness, &ended);
    harness.press("ctrl+q");
    assert!(harness.quit_requested(), "an ended program holds nobody up:\n{}", harness.screen());
}

#[test]
fn several_windows_run_at_once_each_with_a_program_of_its_own() {
    let entries = vec![done("bir", "Bir", 0), done("iki", "İki", 1), done("uc", "Üç", 2)];
    let mut harness = desk(entries.clone(), 100, 30);
    for entry in &entries {
        open(&mut harness, entry);
    }
    assert_eq!(harness.app().windows().len(), 3, "three windows:\n{}", harness.screen());
    let ids: Vec<WindowId> = harness.app().windows().iter().map(Window::id).collect();
    assert_eq!(ids.len(), 3, "each window is its own");
    // How many frames the three ends take to arrive is the machine's business, so they are waited
    // for; reading the codes straight after opening would be believing they had already come.
    until(&mut harness, "all three programs ending", |harness| {
        harness.app().windows().iter().all(|window| matches!(window.run(), Some(Run::Ended { .. })))
    });
    // Each window heard its own program: the codes are the three the programs gave.
    let codes: Vec<Option<i32>> = harness
        .app()
        .windows()
        .iter()
        .map(|window| match window.run() {
            Some(Run::Ended { code }) => code,
            other => panic!("every program ended: {other:?}"),
        })
        .collect();
    assert_eq!(codes, [Some(0), Some(1), Some(2)]);
    let dock = dock(&harness);
    for name in ["Bir", "İki", "Üç"] {
        assert!(dock.contains(name), "{name} is on the dock: {dock}");
    }
}

#[test]
fn the_windows_that_do_not_fit_on_the_dock_go_behind_a_control_that_opens_them() {
    // The windows hold programs that stay running. This test is about the dock's overflow
    // control, and programs that ended would fill the corner with notices and move the keys about
    // while the control is being driven — churn that has nothing to do with what is being proved.
    let entries: Vec<Entry> =
        (0..8).map(|index| waiting(&format!("app{index}"), &format!("Uygulama {index}"))).collect();
    let mut harness = desk(entries.clone(), 60, 24);
    for entry in &entries {
        open(&mut harness, entry);
    }
    assert_eq!(harness.app().windows().len(), 8);
    let row = dock(&harness);
    let control =
        row.split_whitespace().find(|word| word.starts_with('+')).unwrap_or_else(|| panic!("a +n control: {row}"));
    let hidden: usize = control[1..].parse().expect("a number after the plus");
    assert!(hidden > 0 && hidden < 8, "{control} stands for some of the eight windows");
    // Pressing it lists exactly the windows the row left out, and Esc puts the list away.
    let control = control.to_owned();
    harness.click_text(&control);
    let listed = harness.screen();
    assert!(listed.contains("More windows"), "the list has a name:\n{listed}");
    assert!(listed.contains(&format!("Uygulama {}", 8 - hidden)), "the first window it hides is in it:\n{listed}");
    assert_eq!(decoration(&listed), None, "the list is drawn in tones:\n{listed}");
    harness.press("esc");
    assert!(!harness.screen().contains("More windows"), "esc puts it away:\n{}", harness.screen());
}

#[test]
fn a_bell_marks_the_item_of_a_window_that_does_not_have_the_keys_and_the_mark_goes_when_it_does() {
    // The bell rings once the window has lost the keys to another one, which is when the design
    // asks for the mark.
    let signal = signal_path("zil");
    let _ = std::fs::remove_file(&signal);
    let ringing = entry("zil", "Zil", &["/bin/sh", "-c", &on_signal("hazir", &signal, "\\a")]);
    let other = done("iki", "İki", 0);
    let mut harness = desk(vec![ringing.clone(), other.clone()], 100, 30);
    open(&mut harness, &ringing);
    open(&mut harness, &other);
    assert!(!dock(&harness).contains('●'), "nothing has rung yet: {}", dock(&harness));
    // Now that the other window has the keys, tell it to ring.
    std::fs::write(&signal, b"").expect("the signal file is written");
    let row = until_dock(&mut harness, "the bell", |row| row.contains('●'));
    let _ = std::fs::remove_file(&signal);
    let marked = row.find("Zil").map(|at| row[at..].starts_with("Zil ●"));
    assert_eq!(marked, Some(true), "the ringing window carries the mark: {row}");
    assert!(!row.contains("İki ●"), "the window with the keys carries none: {row}");
    // Bringing it forward reads what it had to say, so the mark goes.
    let id =
        harness.app().windows().iter().find(|window| window.entry().id == "zil").map(Window::id).expect("its window");
    harness.send(Msg::Bring(id));
    let row = dock(&harness);
    // The dot at the right end is the count of unread notifications, which the bell also added to;
    // what has to be gone is the mark on the window's own item.
    assert!(!row.contains("Zil ●"), "the mark is gone: {row}");
}

#[test]
fn a_notification_of_a_program_is_said_in_the_corner_and_a_press_on_it_brings_the_window_forward() {
    let notifying = entry(
        "bildiren",
        "Bildiren",
        &["/bin/sh", "-c", &format!("printf 'hazir\\033]777;notify;Yedek;tamam\\007'; {ANSWERS}")],
    );
    let other = done("iki", "İki", 0);
    let mut harness = desk(vec![notifying.clone(), other.clone()], 100, 30);
    open(&mut harness, &notifying);
    open(&mut harness, &other);
    let screen = harness.screen();
    assert!(screen.contains("Yedek"), "the title the program sent:\n{screen}");
    assert!(screen.contains("tamam"), "and its message:\n{screen}");
    assert_eq!(decoration(&screen), None, "the notification is drawn in tones:\n{screen}");
    // A press on it brings the window of the program that sent it forward.
    harness.click_text("Yedek");
    assert_eq!(front(&harness).entry().id, "bildiren", "the window came forward:\n{}", harness.screen());
}

#[test]
fn an_entry_that_opens_once_comes_forward_again_unless_a_window_of_its_own_is_asked_for() {
    let mut harness = desk(Vec::new(), 100, 30);
    let settings = builtin("settings");
    assert!(settings.single, "Settings is the entry that opens once");
    open(&mut harness, &settings);
    open(&mut harness, &settings);
    assert_eq!(harness.app().windows().len(), 1, "the same window came forward:\n{}", harness.screen());
    harness.send(Msg::OpenNew(Target::of(&settings, "en")));
    assert_eq!(harness.app().windows().len(), 2, "a window of its own opened:\n{}", harness.screen());
}

#[test]
fn a_window_whose_program_ended_takes_no_keys_and_the_two_ways_on_are_the_way_on() {
    // A program that leaves a word on its screen and ends: what it left behind is there to be
    // read, not to be typed into.
    let ended = entry("bitti", "Bitti", &["/bin/sh", "-c", "printf 'son soz\\n'; exit 7"]);
    let mut harness = desk(vec![ended.clone()], 120, 30);
    open(&mut harness, &ended);
    until_ended(&mut harness);
    assert!(matches!(front(&harness).run(), Some(Run::Ended { code: Some(7) })), "{}", harness.screen());
    let id = front(&harness).id();
    let body = qdesk::wm::view::body_id(id);
    // What it left behind is still there: that is the whole reason the window did not close.
    assert!(harness.screen().contains("son soz"), "the last word it said is kept:\n{}", harness.screen());
    assert!(!harness.is_focused(&body), "the keys left the dead body with the program:\n{}", harness.screen());
    // Typing reaches the desktop, never the dead program: nothing typed lands in its screen, and
    // what it left there is untouched.
    let rect = front(&harness).rect();
    harness.type_text("abc");
    let line = strip(&harness, Rect { y: rect.y + 1, ..rect });
    assert!(line.contains("son soz"), "what it left is untouched: {line}");
    assert!(!harness.screen().contains("abc"), "a dead program took a key:\n{}", harness.screen());
    // Tab walks the window and never stops in the dead screen: a thing that only shows is not a
    // place the keys can land.
    for _ in 0..8 {
        harness.press("tab");
        assert!(!harness.is_focused(&body), "Tab stopped in a dead screen:\n{}", harness.screen());
    }
    // Neither does a click straight on the words the program left there.
    harness.click_text("son soz");
    assert!(!harness.is_focused(&body), "a click put the keys in a dead screen:\n{}", harness.screen());
    // And the two ways on are there to be reached.
    let screen = harness.screen();
    assert!(screen.contains("Restart") && screen.contains("Close"), "the two ways on:\n{screen}");
}

/// How far apart two colours are, channel by channel: enough to tell a faded colour from the one
/// it was mixed out of, without asking for the exact mixture the theme chose.
fn apart(one: Rgb, other: Rgb) -> u32 {
    let channel = |a: u8, b: u8| u32::from(a.abs_diff(b));
    channel(one.r, other.r) + channel(one.g, other.g) + channel(one.b, other.b)
}

/// The colour of the word `text` on screen and the colour behind it.
fn colors_of(harness: &Harness<Desk>, text: &str) -> (Rgb, Rgb) {
    let (x, y) = harness.find(text).unwrap_or_else(|| panic!("{text} is on screen:\n{}", harness.screen()));
    let x = u16::try_from(x).expect("a column on screen");
    let y = u16::try_from(y).expect("a row on screen");
    let fg = harness.fg(x, y).unwrap_or_else(|| panic!("{text} is drawn in a colour"));
    let bg = harness.bg(x, y).unwrap_or_else(|| panic!("{text} has something behind it"));
    (fg, bg)
}

#[test]
fn the_last_screen_of_an_ended_program_is_drawn_faint() {
    // The same word, said by a program that is still going and by one that has ended. The design
    // asks for the second to read as faded, so a person can see at a glance which window is alive.
    let word = "solgun";
    let live = entry("canli", "Canlı", &["/bin/sh", "-c", &format!("printf '{word}\\n'; {ANSWERS}")]);
    let mut running = desk(vec![live.clone()], 120, 30);
    open(&mut running, &live);
    until_screen(&mut running, word);
    let (live_fg, live_bg) = colors_of(&running, word);

    let gone = entry("bitti", "Bitti", &["/bin/sh", "-c", &format!("printf '{word}\\n'; exit 0")]);
    let mut ended = desk(vec![gone.clone()], 120, 30);
    open(&mut ended, &gone);
    until_ended(&mut ended);
    until_screen(&mut ended, word);
    let (dead_fg, dead_bg) = colors_of(&ended, word);

    assert_eq!(live_bg, dead_bg, "the same screen is behind both words");
    assert_ne!(dead_fg, live_fg, "the dead screen is not drawn as brightly as a living one");
    assert!(
        apart(dead_fg, dead_bg) < apart(live_fg, live_bg),
        "the dead screen is mixed into its background: {dead_fg:?} against {live_fg:?} on {dead_bg:?}"
    );
}

/// Whether a row of the screen ends with `text`: a line of a program's output, told apart from a
/// longer line that merely starts the same way.
fn shows_line(harness: &Harness<Desk>, text: &str) -> bool {
    rows(harness).iter().any(|row| row.trim_end().ends_with(text))
}

#[test]
fn the_screen_of_an_ended_program_is_still_scrolled_back_through() {
    // More lines than the window has rows, so the first one is off the top when the program ends.
    let ended = entry(
        "uzun",
        "Uzun",
        &["/bin/sh", "-c", "i=1; while [ $i -le 120 ]; do printf 'satir %s\\n' $i; i=$((i+1)); done; exit 0"],
    );
    let mut harness = desk(vec![ended.clone()], 120, 30);
    open(&mut harness, &ended);
    until_ended(&mut harness);
    // The window is far shorter than 120 lines, so the first ones are off the top when it ends.
    assert!(!shows_line(&harness, "satir 3"), "the first lines are off the top:\n{}", harness.screen());
    // The wheel over the dead screen walks back through what the program said, which is the other
    // half of why the window stayed open.
    // The wheel goes over a line the program itself wrote, which is inside the screen and not in
    // the space between it and the line of buttons.
    let (x, y) = harness.find("satir 1").expect("a line of the program on screen");
    for _ in 0..60 {
        harness.mouse(MouseKind::ScrollUp, x, y);
        if shows_line(&harness, "satir 3") {
            return;
        }
    }
    panic!("scrolling back never reached the start:\n{}", harness.screen());
}

#[test]
fn the_lines_set_in_settings_are_as_far_back_as_the_next_window_scrolls() {
    // Sixty numbered lines, one to a row, and then the end: the window's own screen holds the
    // last of them and whatever it remembers holds the ones above.
    let counted = entry(
        "sayan",
        "Sayan",
        &["/bin/sh", "-c", "i=1; while [ $i -le 60 ]; do printf 'satir %s\\n' $i; i=$((i+1)); done"],
    );
    let mut harness = desk(vec![counted.clone()], 120, 40);
    open(&mut harness, &builtin("settings"));
    // The field of the remembered lines, as the person reaches it: a click into the number it
    // shows, all of it chosen, and two typed over it.
    let (_, y) =
        harness.find("Remembered lines").unwrap_or_else(|| panic!("the setting is on screen:\n{}", harness.screen()));
    let row = rows(&harness)[usize::try_from(y).expect("a row on screen")].clone();
    let field = row.chars().position(|glyph| glyph == '❯').unwrap_or_else(|| panic!("the row holds a field: {row}"));
    harness.click(i32::try_from(field).expect("a column on screen") + 2, y).press("ctrl+a").type_text("2");
    assert_eq!(harness.app().prefs().scrollback, 2, "the field set the number:\n{}", harness.screen());
    open(&mut harness, &counted);
    until_ended(&mut harness);
    // The wheel goes over a line the program itself wrote.
    let (x, y) =
        harness.find("satir ").unwrap_or_else(|| panic!("a line of the program on screen:\n{}", harness.screen()));
    // The top row of the window's screen, as the number of the line on it.
    let top = |harness: &Harness<Desk>| -> u32 {
        let rows = rows(harness);
        let first = rows
            .iter()
            .find_map(|row| {
                row.split("satir ").nth(1).map(|rest| rest.split_whitespace().next().unwrap_or_default().to_owned())
            })
            .unwrap_or_else(|| panic!("a line of the program on screen:\n{}", harness.screen()));
        first.parse().unwrap_or_else(|_| panic!("a number, not {first:?}"))
    };
    let live = top(&harness);
    // Far more wheel steps than the window could ever give back.
    for _ in 0..40 {
        harness.mouse(MouseKind::ScrollUp, x, y);
    }
    assert_eq!(top(&harness), live - 2, "two lines remembered, two lines further back:\n{}", harness.screen());
}

#[test]
fn the_last_lines_of_an_ended_program_stay_in_sight_above_its_line() {
    // Sixty numbered lines and then the end. The last of them are where an error message would
    // be, the very reason the window stays open, so the line that says the program ended goes
    // under them and never over them.
    let counted = entry(
        "sayan",
        "Sayan",
        &["/bin/sh", "-c", "i=1; while [ $i -le 60 ]; do printf 'satir %s\\n' $i; i=$((i+1)); done"],
    );
    let mut harness = desk(vec![counted.clone()], 80, 24);
    open(&mut harness, &counted);
    until_ended(&mut harness);
    until_screen(&mut harness, "satir 60");
    let screen = harness.screen();
    let (_, last) = harness.find("satir 60").unwrap_or_else(|| panic!("the last line is in sight:\n{screen}"));
    let (_, ended) = harness.find("Restart").unwrap_or_else(|| panic!("the ended line is there:\n{screen}"));
    assert!(last < ended, "the last line stands above the ended line:\n{screen}");
}

#[test]
fn resizing_a_window_does_not_reflow_the_screen_of_an_ended_program() {
    // A line longer than the window will be made, on a program that has ended. Nothing may be
    // written to that session any more — not even the size — because a screen left to be read
    // would be reflowed out from under the person reading it.
    let line = "bir iki uc dort bes alti yedi sekiz dokuz on bir iki";
    let ended = entry("genis", "Geniş", &["/bin/sh", "-c", &format!("printf '{line}\\n'; exit 0")]);
    let mut harness = desk(vec![ended.clone()], 120, 30);
    open(&mut harness, &ended);
    until_ended(&mut harness);
    until_screen(&mut harness, line);
    let wide = front(&harness).rect().width;
    // The desktop shrinks, and every window is clamped into what is left of it.
    harness.resize(40, 20);
    harness.render();
    let narrow = front(&harness).rect().width;
    assert!(narrow < wide, "the window really was narrowed: {narrow} out of {wide}");
    // The desktop is itself again, and the window is opened out to it: a screen that was never
    // written to is all still there to be drawn.
    harness.resize(120, 30);
    harness.send(Msg::Ask(Ask::Maximize, front(&harness).id()));
    harness.render();
    assert!(
        harness.screen().contains(line),
        "the screen came back whole after the window changed size:\n{}",
        harness.screen()
    );
}

#[test]
fn a_paste_into_a_window_whose_program_has_ended_is_answered_instead_of_vanishing() {
    let ended = entry("bitti", "Bitti", &["/bin/sh", "-c", "printf 'son soz\\n'; exit 0"]);
    let mut harness = desk(vec![ended.clone()], 120, 30);
    open(&mut harness, &ended);
    until_ended(&mut harness);
    harness.paste("yapistirilan");
    harness.render();
    let screen = harness.screen();
    assert!(!screen.contains("yapistirilan"), "nothing took the text:\n{screen}");
    assert!(screen.contains("went nowhere"), "and the desktop said so:\n{screen}");
}

#[test]
fn a_paste_onto_the_desktop_itself_is_not_worth_a_word() {
    // Nobody expects pasting onto a desktop background to do anything, so qdesk says nothing.
    let mut harness = desk(Vec::new(), 100, 30);
    harness.render();
    harness.paste("yapistirilan");
    harness.render();
    let screen = harness.screen();
    assert!(!screen.contains("yapistirilan"), "the floor took no text:\n{screen}");
    assert!(!screen.contains("went nowhere"), "and qdesk kept quiet about it:\n{screen}");
}

/// A program that says a word, then stays alive waiting to be typed into and answering what it is
/// told. The word says it has drawn; the waiting is the case a bounded watch made testable.
fn live(id: &str, name: &str) -> Entry {
    entry(id, name, &["/bin/sh", "-c", &format!("printf 'hazir\\n'; {ANSWERS}")])
}

#[test]
fn a_window_shows_what_its_live_program_drew_and_the_program_goes_on_running() {
    let program = live("canli", "Canlı");
    let mut harness = desk(vec![program.clone()], 100, 30);
    open(&mut harness, &program);
    until_screen(&mut harness, "hazir");
    let window = front(&harness);
    assert!(window.is_running(), "the program is still there after it drew:\n{}", harness.screen());
    assert!(matches!(window.run(), Some(Run::Running)), "{:?}", window.run());
    // It says nothing more, and the desktop goes on drawing: the window keeps what the program
    // left on its screen and its item stays on the dock.
    harness.render().render();
    assert!(harness.screen().contains("hazir"), "the screen is still the program's:\n{}", harness.screen());
    assert!(front(&harness).is_running(), "a silent program is a running program");
    assert!(dock(&harness).contains("Canlı"), "its window is on the dock: {}", dock(&harness));
}

#[test]
fn keys_typed_into_a_window_reach_its_live_program_and_its_answer_comes_back() {
    let program = live("canli", "Canlı");
    let mut harness = desk(vec![program.clone()], 100, 30);
    open(&mut harness, &program);
    until_screen(&mut harness, "hazir");
    // The keys are in the body of a window whose program runs, which is what makes the typing real.
    let body = qdesk::wm::view::body_id(front(&harness).id());
    assert!(harness.is_focused(&body), "the running program has the keys:\n{}", harness.screen());
    harness.type_text("merhaba");
    harness.press("enter");
    until_screen(&mut harness, "yanit: merhaba");
    let screen = harness.screen();
    assert!(screen.contains("merhaba"), "the program echoed what was typed:\n{screen}");
    assert!(front(&harness).is_running(), "and it is still running, waiting for more");
    // A second line goes the same way, so the window is a terminal and not a one-shot.
    harness.type_text("yine");
    harness.press("enter");
    until_screen(&mut harness, "yanit: yine");
}

#[test]
fn the_title_a_live_program_gives_itself_reaches_the_strip_while_it_is_still_running() {
    let titled = entry(
        "basliksiz",
        "Başlıksız",
        &["/bin/sh", "-c", &format!("printf 'hazir\\033]0;derleniyor\\007'; {ANSWERS}")],
    );
    let mut harness = desk(vec![titled.clone()], 100, 30);
    open(&mut harness, &titled);
    until(&mut harness, "the title the program gave itself", |harness| {
        strip(harness, front(harness).rect()).contains("derleniyor")
    });
    let line = strip(&harness, front(&harness).rect());
    assert!(line.contains("Başlıksız"), "the strip still names the application: {line}");
    assert!(front(&harness).is_running(), "the title came from a program that is still running");
}

#[test]
fn a_live_program_rings_the_bell_and_keeps_running_with_the_mark_on_its_window() {
    // The bell rings once the window has lost the keys, which is when the design asks for the
    // mark, and the program is alive on both sides of the ring.
    let signal = signal_path("canli-zil");
    let _ = std::fs::remove_file(&signal);
    let ringing = entry("zil", "Zil", &["/bin/sh", "-c", &on_signal("hazir", &signal, "\\a")]);
    let other = live("iki", "İki");
    let mut harness = desk(vec![ringing.clone(), other.clone()], 100, 30);
    open(&mut harness, &ringing);
    until_screen(&mut harness, "hazir");
    open(&mut harness, &other);
    assert!(!dock(&harness).contains('●'), "nothing has rung yet: {}", dock(&harness));
    std::fs::write(&signal, b"").expect("the signal file is written");
    let row = until_dock(&mut harness, "the bell", |row| row.contains('●'));
    let _ = std::fs::remove_file(&signal);
    assert_eq!(row.find("Zil").map(|at| row[at..].starts_with("Zil ●")), Some(true), "{row}");
    let rang = harness.app().windows().iter().find(|window| window.entry().id == "zil").expect("its window");
    assert!(rang.is_running(), "the program that rang is still running");
    assert!(front(&harness).is_running(), "and so is the one with the keys");
}

#[test]
fn the_close_mark_of_a_window_asks_about_a_running_program_as_every_other_way_does() {
    // The mark on the title strip is how a window is closed, so it is driven here as a person
    // drives it. Sending the message instead would prove the question works without proving the
    // mark ever asks it — which is exactly how the mark came to close outright while the dock's
    // menu, the exit line and the desktop mode's key all asked.
    let program = waiting("bekleyen", "Bekleyen");
    let mut harness = desk(vec![program.clone()], 80, 24);
    open(&mut harness, &program);
    assert!(front(&harness).is_running(), "the program is still going:\n{}", harness.screen());
    let rect = front(&harness).rect();
    // The three marks sit at the right end of the strip, three cells each; the last is the close.
    harness.click(rect.right() - 2, rect.y);
    let screen = harness.screen();
    assert!(screen.contains("still running"), "the mark asked about the program:\n{screen}");
    assert_eq!(harness.app().windows().len(), 1, "nothing closed before it was answered");
    // And the window whose program has ended closes from the same mark without a word.
    harness.press("esc");
    let ended = done("bitti", "Bitti", 0);
    let mut harness = desk(vec![ended.clone()], 80, 24);
    open(&mut harness, &ended);
    until_ended(&mut harness);
    let rect = front(&harness).rect();
    harness.click(rect.right() - 2, rect.y);
    assert_eq!(harness.app().windows().len(), 0, "nothing to ask about:\n{}", harness.screen());
}

#[test]
fn the_close_row_of_a_window_s_menu_on_the_dock_asks_about_a_running_program() {
    // The menu is opened the way a person opens it — a right click on the window's own item on
    // the dock — and its Close row is chosen the same way. Sending the message it carries would
    // prove the question works without proving the row on the real menu ever reaches it, which is
    // exactly the gap the close mark once fell through.
    let program = waiting("bekleyen", "Bekleyen");
    let mut harness = desk(vec![program.clone()], 100, 24);
    open(&mut harness, &program);
    assert!(front(&harness).is_running(), "the program is still going:\n{}", harness.screen());
    let (x, y) = dock_cell(&harness, "Bekleyen");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    let opened = harness.screen();
    assert!(opened.contains("Close"), "the menu opened with its Close row:\n{opened}");
    harness.click_text("Close");
    let screen = harness.screen();
    assert!(screen.contains("still running"), "the menu's Close row asked about the program:\n{screen}");
    assert_eq!(harness.app().windows().len(), 1, "nothing closed before it was answered");
}

#[test]
fn the_close_row_of_a_window_s_menu_on_the_dock_closes_one_whose_program_has_ended() {
    // Nothing else on screen says "Close" here: the window whose program has ended is narrow, so
    // its own line of text does not reach that word, and the only "Close" is the menu's own row.
    let ended = done("bitti", "Bitti", 0);
    let mut harness = desk(vec![ended.clone()], 100, 24);
    open(&mut harness, &ended);
    until_ended(&mut harness);
    let (x, y) = dock_cell(&harness, "Bitti");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    assert!(harness.screen().contains("Close"), "the menu opened:\n{}", harness.screen());
    harness.click_text("Close");
    assert_eq!(harness.app().windows().len(), 0, "nothing to ask about:\n{}", harness.screen());
}

#[test]
fn the_x_key_of_desktop_mode_asks_about_a_running_program_and_closes_one_that_has_ended() {
    // Desktop mode's own key is driven exactly as a person drives it: entered with its chord and
    // pressed by name, not sent as the message it maps to.
    let program = waiting("bekleyen", "Bekleyen");
    let mut harness = desk(vec![program.clone()], 80, 24);
    open(&mut harness, &program);
    assert!(front(&harness).is_running(), "the program is still going:\n{}", harness.screen());
    harness.press("ctrl+alt+space");
    harness.press("x");
    let screen = harness.screen();
    assert!(screen.contains("still running"), "the key asked about the program:\n{screen}");
    assert_eq!(harness.app().windows().len(), 1, "nothing closed before it was answered");

    let ended = done("bitti", "Bitti", 0);
    let mut harness = desk(vec![ended.clone()], 80, 24);
    open(&mut harness, &ended);
    until_ended(&mut harness);
    harness.press("ctrl+alt+space");
    harness.press("x");
    assert_eq!(harness.app().windows().len(), 0, "nothing to ask about:\n{}", harness.screen());
}

#[test]
fn the_close_row_of_a_window_s_menu_asks_about_a_running_program_over_ssh_too() {
    // The desktop over SSH is the main way qdesk is used (design), so the question has to hold
    // there as much as it does locally; the remote flag is forced rather than read off the
    // sandbox this test happens to run in.
    let program = waiting("bekleyen", "Bekleyen");
    let mut harness = desk_over_ssh(vec![program.clone()], 100, 24);
    open(&mut harness, &program);
    assert!(front(&harness).is_running(), "the program is still going:\n{}", harness.screen());
    let (x, y) = dock_cell(&harness, "Bekleyen");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    harness.click_text("Close");
    let screen = harness.screen();
    assert!(screen.contains("still running"), "the menu's Close row asked about the program over SSH:\n{screen}");
    assert_eq!(harness.app().windows().len(), 1, "nothing closed before it was answered");
}

#[test]
fn leaving_qdesk_over_ssh_still_asks_about_a_running_program() {
    let program = waiting("bekleyen", "Bekleyen");
    let mut harness = desk_over_ssh(vec![program.clone()], 80, 24);
    open(&mut harness, &program);
    harness.press("ctrl+q");
    let screen = harness.screen();
    assert!(!harness.quit_requested(), "it waits for an answer over SSH too:\n{screen}");
    assert!(screen.contains("1 program is still running"), "the question counts them over SSH too:\n{screen}");
}

#[test]
fn alt_and_the_right_button_size_the_window_of_a_program_that_reads_the_mouse() {
    // The program asks for every click, as `htop` or `vim` with the mouse on do. A plain right
    // drag in its body is the program's; with alt it is the window's, from the nearest corner.
    let reader = entry("okuyan", "Okuyan", &["/bin/sh", "-c", "printf '\\033[?1000h\\033[?1006hhazir\\n'; exec cat"]);
    let mut harness = desk(vec![reader.clone()], 100, 30);
    open(&mut harness, &reader);
    until_screen(&mut harness, "hazir");
    let before = front(&harness).rect();
    let near_top_left = (before.x + 2, before.y + 2);
    let mouse = |alt: bool, kind, (x, y): (i32, i32)| {
        Event::Mouse(qframe::event::MouseEvent {
            kind,
            x,
            y,
            mods: qframe::keymap::Modifiers { alt, ..qframe::keymap::Modifiers::default() },
        })
    };
    let to = (near_top_left.0 + 3, near_top_left.1 + 2);
    for alt in [false, true] {
        harness.events(&[
            mouse(alt, MouseKind::Down(MouseButton::Right), near_top_left),
            mouse(alt, MouseKind::Drag(MouseButton::Right), to),
            mouse(alt, MouseKind::Up(MouseButton::Right), to),
        ]);
        if !alt {
            assert_eq!(front(&harness).rect(), before, "a plain right drag is the program's");
        }
    }
    let after = front(&harness).rect();
    assert_eq!(
        after,
        Rect::new(before.x + 3, before.y + 2, before.width - 3, before.height - 2),
        "the top left corner followed the pointer:\n{}",
        harness.screen()
    );
}
