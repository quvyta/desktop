//! A screen attached to the background, drawn: a window's program outlives the screen, and the
//! next screen opens the window where it was, with the program's screen and the program still
//! answering. Detaching leaves the programs; quitting ends them.
//!
//! Each test runs a background of its own in this process, on a socket in a folder of its own;
//! the programs are `/bin/sh` scripts the test writes. A screen that goes is a harness dropped,
//! which closes its connection as a killed screen's does.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use qdesk::app::{Desk, Msg, Target};
use qdesk::apps::{Catalog, Declared, Entry, Environment, Folders, Launch, Source, load, parse_entry};
use qdesk::desktop::Desktop;
use qdesk::serve::link::Link;
use qdesk::serve::wire::{Choice, Listed};
use qdesk::serve::{self, Listening, socket};
use qdesk::wm::{Run, Window};
use qframe::prelude::*;

use support::{BUDGET, HARMLESS, HOME, MACHINE, MOMENT, OFFSET, PATIENCE};

static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A folder of the test's own, with a background serving on a socket in it.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("qdesk-reattach-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the scratch folder is made");
        let scratch = Self(path);
        let listening = Listening::on(&scratch.socket()).expect("listening");
        thread::spawn(move || serve::run(&listening, || None));
        scratch
    }

    fn socket(&self) -> PathBuf {
        socket::place(Some(self.0.join("tmp").as_os_str()), socket::own_uid())
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    /// The desktops the background holds, as a screen that only asks hears them.
    fn listed(&self) -> Vec<Listed> {
        Link::connect(&self.socket()).ok().and_then(|link| link.list().ok()).unwrap_or_default()
    }

    /// A screen of 100 by 30 attached to the desktop no screen shows, its floor holding `entries`.
    fn screen(&self, entries: &[Entry]) -> Harness<Desk> {
        let link = Link::connect(&self.socket()).expect("the background answers");
        let attachment = link.attach(Choice::Free, Duration::ZERO).expect("attached");
        let mut all = builtins();
        all.extend(entries.iter().cloned());
        let catalog = Catalog::new(support::sealed(all), |entry| !matches!(entry.launch, Launch::Open(_)));
        let desktop = Desktop {
            icons: entries.iter().map(|entry| entry.id.clone()).collect(),
            welcome_seen: true,
            recommended_seen: true,
            resize_hint_seen: true,
            ..Desktop::default()
        };
        let apps = Environment { shell: Some(PathBuf::from(HARMLESS)), ..Environment::default() };
        let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), Box::new(|| MOMENT * 1_000))
            .apps(apps)
            .catalog(catalog)
            .desktop(desktop)
            .attach_to(link, attachment)
            .watch_within(PATIENCE);
        support::draw(app, 100, 30)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn builtins() -> Vec<Entry> {
    let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
    load(&folders, Some(Path::new(HOME))).entries
}

/// An entry of this test that runs `script` in `/bin/sh`.
fn entry(id: &str, name: &str, script: &str) -> Entry {
    assert!(!script.contains('"') && !script.contains('\\'), "a test script is written plainly");
    let text = format!("name = \"{name}\"\ncommand = [\"/bin/sh\", \"-c\", \"{script}\"]\ncategory = \"system\"\n");
    let file = format!("{HOME}/.local/share/quvyta/desktop/apps/{id}.toml");
    let (declared, diagnostics) =
        parse_entry(id, Path::new(&file), text.as_bytes(), Source::User, Some(Path::new(HOME)));
    assert!(diagnostics.is_empty(), "{id}: {diagnostics:?}");
    match declared {
        Some(Declared::Entry(entry)) => *entry,
        other => panic!("{id} declares no entry: {other:?}"),
    }
}

/// A program that says `word` and answers every line typed to it.
fn answering(id: &str, word: &str) -> Entry {
    entry(id, id, &format!("echo {word}; while read line; do echo got-$line; done"))
}

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

fn wait_for(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < BUDGET, "{what} never happened");
        thread::sleep(Duration::from_millis(5));
    }
}

fn front(harness: &Harness<Desk>) -> &Window {
    harness.app().windows().front().expect("a window on the desktop")
}

fn alive(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.rsplit_once(") ").is_some_and(|(_, rest)| !rest.starts_with('Z')))
}

/// The process id of the program in the window in front, as the background said it.
fn pid_in_front(harness: &Harness<Desk>) -> u32 {
    let id = front(harness).id();
    harness.app().program_pid(id).expect("the background gave a process id")
}

/// Clicks the button `label` on the lowest row that holds it and is not a question: a dialog's
/// buttons stand below its title.
fn press_button(harness: &mut Harness<Desk>, label: &str) {
    let screen = harness.screen();
    let rows: Vec<&str> = screen.lines().collect();
    let (y, x) = rows
        .iter()
        .enumerate()
        .rev()
        .find_map(|(y, row)| (!row.contains('?')).then(|| row.rfind(label).map(|x| (y, x))).flatten())
        .unwrap_or_else(|| panic!("no {label} to press:\n{screen}"));
    let x = rows[y][..x].chars().count();
    harness.click(i32::try_from(x).unwrap_or(0) + 1, i32::try_from(y).unwrap_or(0));
}

/// Waits until the background sees no screen attached, so the next one finds this desktop free.
fn until_left(scratch: &Scratch) {
    wait_for("the background to see the screen go", || scratch.listed().iter().all(|desktop| !desktop.attached));
}

#[test]
fn a_window_comes_back_where_it_was_with_its_programs_screen_and_the_program_answering() {
    let scratch = Scratch::new("back");
    let program = answering("talker", "ready-to-talk");
    let mut first = scratch.screen(std::slice::from_ref(&program));
    first.send(Msg::Open(Target::of(&program, "en")));
    until(&mut first, "the program's first word", |harness| harness.screen().contains("ready-to-talk"));
    // Moved by its title strip, as a person moves it.
    let rect = front(&first).rect();
    first.drag((rect.x + 12, rect.y), (rect.x + 20, rect.y + 3));
    let moved = front(&first).rect();
    assert_ne!(moved, rect, "the window moved:\n{}", first.screen());
    let pid = pid_in_front(&first);
    drop(first);
    until_left(&scratch);
    assert!(alive(pid), "the program outlived its screen");

    let mut second = scratch.screen(std::slice::from_ref(&program));
    until(&mut second, "the window back with its program's screen", |harness| {
        harness.screen().contains("ready-to-talk")
    });
    assert_eq!(second.app().windows().len(), 1);
    assert_eq!(front(&second).rect(), moved, "where it was left:\n{}", second.screen());
    assert_eq!(front(&second).run(), Some(Run::Running));
    assert_eq!(pid_in_front(&second), pid, "the same program, not a new one");
    second.type_text("again").press("enter");
    until(&mut second, "the program answering the new screen", |harness| harness.screen().contains("got-again"));
}

#[test]
fn detaching_leaves_the_programs_running_and_quitting_from_the_next_screen_ends_them() {
    let scratch = Scratch::new("detach");
    let program = answering("keeper", "kept-alive");
    let mut first = scratch.screen(std::slice::from_ref(&program));
    first.send(Msg::Open(Target::of(&program, "en")));
    until(&mut first, "the program's first word", |harness| harness.screen().contains("kept-alive"));
    let pid = pid_in_front(&first);
    first.press("ctrl+q");
    let question = first.screen();
    assert!(question.contains("detaching leaves it running"), "{question}");
    press_button(&mut first, "Detach");
    assert!(first.quit_requested(), "{}", first.screen());
    drop(first);
    until_left(&scratch);
    assert!(alive(pid), "detaching ends nothing");
    assert_eq!(scratch.listed().first().map(|desktop| desktop.running), Some(1));

    let mut second = scratch.screen(std::slice::from_ref(&program));
    until(&mut second, "the kept program", |harness| harness.screen().contains("kept-alive"));
    second.press("ctrl+q");
    let question = second.screen();
    assert!(question.contains("Quit qdesk?"), "{question}");
    press_button(&mut second, "Quit");
    assert!(second.quit_requested(), "{}", second.screen());
    wait_for("quitting to end the program", || !alive(pid));
    drop(second);
    wait_for("the desktop to be gone", || scratch.listed().is_empty());
}

#[test]
fn a_program_that_ended_while_nobody_was_attached_says_so_in_its_window_when_one_comes() {
    let scratch = Scratch::new("ended");
    let go = scratch.file("go");
    let program = entry(
        "brief",
        "brief",
        &format!("echo last-words; while [ ! -e {} ]; do sleep 0.02; done; exit 4", go.display()),
    );
    let mut first = scratch.screen(std::slice::from_ref(&program));
    first.send(Msg::Open(Target::of(&program, "en")));
    until(&mut first, "the program's words", |harness| harness.screen().contains("last-words"));
    let pid = pid_in_front(&first);
    drop(first);
    until_left(&scratch);
    fs::write(&go, "").expect("told to end");
    wait_for("the program to end", || !alive(pid));

    let mut second = scratch.screen(std::slice::from_ref(&program));
    until(&mut second, "the window saying how its program ended", |harness| {
        harness.app().windows().front().is_some_and(|window| window.run() == Some(Run::Ended { code: Some(4) }))
    });
    let screen = second.screen();
    assert!(screen.contains("last-words"), "the last screen is kept:\n{screen}");
}

#[test]
fn closing_a_window_on_an_attached_screen_ends_its_program_in_the_background() {
    let scratch = Scratch::new("close");
    let program = answering("closer", "closing-soon");
    let mut screen = scratch.screen(std::slice::from_ref(&program));
    screen.send(Msg::Open(Target::of(&program, "en")));
    until(&mut screen, "the program's first word", |harness| harness.screen().contains("closing-soon"));
    let pid = pid_in_front(&screen);
    let id = front(&screen).id();
    screen.send(Msg::CloseAnyway(id));
    wait_for("the closed window's program to end", || !alive(pid));
    assert!(screen.app().windows().is_empty());
    wait_for("the background to forget it", || scratch.listed().first().is_some_and(|desktop| desktop.programs == 0));
}

#[test]
fn a_new_screen_beside_a_shown_desktop_opens_one_of_its_own() {
    let scratch = Scratch::new("beside");
    let program = answering("first-one", "on-the-first");
    let mut first = scratch.screen(std::slice::from_ref(&program));
    first.send(Msg::Open(Target::of(&program, "en")));
    until(&mut first, "the program's first word", |harness| harness.screen().contains("on-the-first"));
    let second = scratch.screen(std::slice::from_ref(&program));
    assert!(second.app().windows().is_empty(), "the shown desktop stays with its screen:\n{}", second.screen());
    assert_eq!(scratch.listed().len(), 2);
    drop(first);
}

#[test]
fn the_program_gets_the_screens_environment() {
    let scratch = Scratch::new("env");
    // The screen and this test share a process, so what the screen's environment holds is the
    // test's: Cargo's own variable stands for anything the person's session sets.
    let package = std::env::var("CARGO_PKG_NAME").expect("cargo names the package");
    let program = entry("env", "env", "echo pkg-$CARGO_PKG_NAME-$QDESK; cat");
    let mut screen = scratch.screen(std::slice::from_ref(&program));
    screen.send(Msg::Open(Target::of(&program, "en")));
    let expected = format!("pkg-{package}-1");
    until(&mut screen, "the program saying what it was given", |harness| harness.screen().contains(&expected));
}
