//! The background and a screen's link to it, without a desktop drawn: a program started through
//! one screen lives on after it goes, and the next screen finds it as it was.
//!
//! Each test runs a background of its own on a socket in a folder of its own, in this process,
//! and its programs are `/bin/sh` scripts the test writes. Nothing is timed: a program is told
//! when to act by a file and says it has by another, and what a test waits for is waited for in a
//! bounded loop.

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use qdesk::serve::link::{Attachment, Link};
use qdesk::serve::wire::{Choice, Listed, Message, Spec};
use qdesk::serve::{self, Listening, socket};
use qframe::widgets::TerminalSession;

/// Generous: everything waited for comes in milliseconds, a loaded machine only makes it slower.
const BUDGET: Duration = Duration::from_secs(60);

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let n = NEXT.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("qdesk-background-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the scratch folder is made");
        Self(path)
    }

    fn file(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }

    fn socket(&self) -> PathBuf {
        socket::place(Some(self.0.join("tmp").as_os_str()), socket::own_uid())
    }

    /// A background serving on this folder's socket, on a thread of this process, that never ends
    /// by itself.
    fn background(&self) -> PathBuf {
        let path = self.socket();
        let listening = Listening::on(&path).expect("listening");
        thread::spawn(move || serve::run(&listening, || None));
        path
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn wait_for(what: &str, done: impl Fn() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < BUDGET, "{what} never happened");
        thread::sleep(Duration::from_millis(5));
    }
}

/// What a session's screen and scrollback hold, as the text in them.
fn shows(session: &TerminalSession, text: &str) -> bool {
    String::from_utf8_lossy(&session.snapshot()).contains(text)
}

/// A program that runs `script` in `/bin/sh` on a screen of 60 by 12, with only the environment
/// it is given.
fn script(script: &str, folder: &Path, env: &[(&str, &str)]) -> Spec {
    Spec {
        program: OsString::from("/bin/sh"),
        args: vec![OsString::from("-c"), OsString::from(script)],
        folder: folder.to_path_buf(),
        env: env.iter().map(|(name, value)| (OsString::from(name), OsString::from(value))).collect(),
        columns: 60,
        rows: 12,
        scrollback: 500,
    }
}

/// The desktops the background on `path` holds, as a screen that only asks hears them; `None`
/// when it cannot be asked.
fn listed(path: &Path) -> Option<Vec<Listed>> {
    Link::connect(path).ok()?.list().ok()
}

fn attach(path: &Path) -> (Link, Attachment) {
    let link = Link::connect(path).expect("the background answers");
    let attachment = link.attach(Choice::Free, Duration::ZERO).expect("attached");
    (link, attachment)
}

fn alive(pid: u32) -> bool {
    // A process that ended but was not yet waited for still has its folder; its state says so.
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.rsplit_once(") ").is_some_and(|(_, rest)| !rest.starts_with('Z')))
}

#[test]
fn a_program_lives_on_when_its_screen_goes_and_the_next_screen_finds_it_as_it_was() {
    let scratch = Scratch::new("lives");
    let path = scratch.background();
    let (link, first) = attach(&path);
    assert!(first.programs.is_empty(), "a new desktop holds nothing");
    let spec = script("echo first; while read line; do echo \"got $line\"; done", &scratch.0, &[]);
    let opened = link.open(&spec, b"the window".to_vec(), Duration::ZERO).expect("started");
    let pid = opened.pid.expect("a process id");
    wait_for("the first line on the first screen", || shows(&opened.session, "first"));

    // The screen goes as a killed one does: nothing said, the connection gone.
    drop(opened);
    drop(link);
    wait_for("the background to see the screen go", || {
        listed(&path).is_some_and(|desktops| desktops.first().is_some_and(|desktop| !desktop.attached))
    });
    assert!(alive(pid), "the program outlived its screen");

    let (link, second) = attach(&path);
    assert_eq!(second.desktop, first.desktop, "the desktop nobody shows is the one a screen finds");
    let [kept] = &second.programs[..] else { panic!("one program: {:?}", second.programs) };
    assert_eq!(kept.record.pid, Some(pid));
    assert_eq!(kept.record.window, b"the window");
    assert_eq!((kept.record.columns, kept.record.rows), (60, 12));
    wait_for("the first line on the second screen", || shows(&kept.session, "first"));
    kept.session.write(b"ping\r").expect("typed");
    wait_for("the program answering the second screen", || shows(&kept.session, "got ping"));
    drop(link);
}

#[test]
fn what_a_program_writes_while_no_screen_is_attached_is_there_for_the_next() {
    let scratch = Scratch::new("alone");
    let path = scratch.background();
    let go = scratch.file("go");
    let done = scratch.file("done");
    let (link, _) = attach(&path);
    let program = format!(
        "echo before; while [ ! -e '{go}' ]; do sleep 0.02; done; echo written-alone; : > '{done}'; cat",
        go = go.display(),
        done = done.display()
    );
    let opened = link.open(&script(&program, &scratch.0, &[]), Vec::new(), Duration::ZERO).expect("started");
    wait_for("the first line", || shows(&opened.session, "before"));
    drop(opened);
    drop(link);

    fs::write(&go, "").expect("told to write");
    wait_for("the program to have written", || done.exists());
    let (_link, again) = attach(&path);
    let [kept] = &again.programs[..] else { panic!("one program") };
    wait_for("what it wrote alone", || shows(&kept.session, "written-alone"));
    assert!(shows(&kept.session, "before"), "and what came before it, scrolled or not");
}

#[test]
fn what_the_screen_said_about_the_window_and_what_the_program_said_about_itself_come_back() {
    let scratch = Scratch::new("place");
    let path = scratch.background();
    let (link, _) = attach(&path);
    let opened = link
        .open(
            &script("printf '\\033]2;deploy\\007'; echo titled; cat", &scratch.0, &[]),
            b"opened".to_vec(),
            Duration::ZERO,
        )
        .expect("started");
    wait_for("the program to say its title", || shows(&opened.session, "titled"));
    link.send(&Message::Place { program: opened.program, window: b"moved".to_vec() }).expect("said");
    link.send(&Message::Lock { locked: true }).expect("said");
    drop(opened);
    drop(link);

    // The background hears the title on a thread of its own: a screen that comes too soon is let
    // go and another comes, until one hears it.
    let start = Instant::now();
    loop {
        let (link, again) = attach(&path);
        let [kept] = &again.programs[..] else { panic!("one program") };
        assert_eq!(kept.record.window, b"moved", "the last word on the window");
        assert!(again.locked, "left locked");
        if kept.record.title.as_deref() == Some("deploy") {
            break;
        }
        assert!(start.elapsed() < BUDGET, "the title never came back: {:?}", kept.record.title);
        drop(again);
        drop(link);
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_program_that_ended_with_no_screen_shows_its_last_screen_and_how_it_ended() {
    let scratch = Scratch::new("ended");
    let path = scratch.background();
    let go = scratch.file("go");
    let (link, _) = attach(&path);
    let program = format!("echo last words; while [ ! -e '{}' ]; do sleep 0.02; done; exit 3", go.display());
    let opened = link.open(&script(&program, &scratch.0, &[]), Vec::new(), Duration::ZERO).expect("started");
    let pid = opened.pid.expect("a process id");
    wait_for("the last words", || shows(&opened.session, "last words"));
    drop(opened);
    drop(link);
    fs::write(&go, "").expect("told to end");
    wait_for("the program to end", || !alive(pid));

    let start = Instant::now();
    let (link, kept) = loop {
        let (link, again) = attach(&path);
        let [kept] = &again.programs[..] else { panic!("the ended program is still there, for its window") };
        if kept.record.exit.is_some() {
            break (link, again);
        }
        assert!(start.elapsed() < BUDGET, "the end never reached the background");
        drop(again);
        drop(link);
        thread::sleep(Duration::from_millis(5));
    };
    let kept = &kept.programs[0];
    assert_eq!(kept.record.exit, Some(Some(3)));
    wait_for("the session to end as the program did", || kept.session.exit() == Some(Some(3)));
    assert!(shows(&kept.session, "last words"));
    drop(link);
}

#[test]
fn a_closed_window_ends_its_program_and_a_finished_desktop_ends_them_all() {
    let scratch = Scratch::new("close");
    let path = scratch.background();
    let (link, attached) = attach(&path);
    let one = link.open(&script("echo one; cat", &scratch.0, &[]), Vec::new(), Duration::ZERO).expect("started");
    let two = link.open(&script("echo two; cat", &scratch.0, &[]), Vec::new(), Duration::ZERO).expect("started");
    let three = link.open(&script("echo three; cat", &scratch.0, &[]), Vec::new(), Duration::ZERO).expect("started");
    let pids = [one.pid, two.pid, three.pid].map(|pid| pid.expect("a process id"));
    link.send(&Message::Close { program: one.program, grace_ms: 2_000 }).expect("said");
    wait_for("the closed window's program to end", || !alive(pids[0]));
    assert!(alive(pids[1]) && alive(pids[2]), "the others run on");
    let desktops = link.list().expect("listed");
    assert_eq!(desktops.len(), 1);
    assert_eq!((desktops[0].desktop, desktops[0].programs, desktops[0].running), (attached.desktop, 2, 2));

    link.send(&Message::Finish).expect("said");
    wait_for("every program of the finished desktop to end", || !alive(pids[1]) && !alive(pids[2]));
    drop([one, two, three]);
    drop(link);
    wait_for("the finished desktop to be gone", || listed(&path).is_some_and(|desktops| desktops.is_empty()));
    let (_link, next) = attach(&path);
    assert_ne!(next.desktop, attached.desktop, "a screen that comes after starts afresh");
    assert!(next.programs.is_empty());
}

#[test]
fn a_program_gets_the_screens_environment_and_none_of_the_backgrounds() {
    let scratch = Scratch::new("env");
    let path = scratch.background();
    let (link, _) = attach(&path);
    // Cargo gives a test this variable and the background runs in the test's process; the screen
    // gave none. (Not `PATH`: a shell started without one makes one up.)
    assert!(std::env::var_os("CARGO_PKG_NAME").is_some());
    let program = "echo \"given=$QDESK_GIVEN cargo=${CARGO_PKG_NAME-none}\"; cat";
    let opened = link
        .open(&script(program, &scratch.0, &[("QDESK_GIVEN", "by-the-screen")]), Vec::new(), Duration::ZERO)
        .expect("started");
    let start = Instant::now();
    while !shows(&opened.session, "given=by-the-screen cargo=none") {
        assert!(start.elapsed() < BUDGET, "the program said: {}", String::from_utf8_lossy(&opened.session.snapshot()));
        thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_program_that_cannot_start_is_refused_with_what_the_system_said() {
    let scratch = Scratch::new("refused");
    let path = scratch.background();
    let (link, _) = attach(&path);
    let mut spec = script("", &scratch.0, &[]);
    spec.program = OsString::from(scratch.file("no-such-program"));
    let error = link.open(&spec, Vec::new(), Duration::ZERO).expect_err("refused");
    // What the system said reaches the screen, which puts it in a sentence of the person's own
    // language as it does for a program it could not start itself.
    assert!(error.to_string().contains("no-such-program"), "{error}");
}

#[test]
fn a_new_screen_beside_an_attached_one_gets_a_desktop_of_its_own_unless_it_is_free() {
    let scratch = Scratch::new("two");
    let path = scratch.background();
    let (first_link, first) = attach(&path);
    let (second_link, second) = attach(&path);
    assert_ne!(first.desktop, second.desktop, "two connections, two desktops");
    let third_link = Link::connect(&path).expect("answers");
    let third = third_link.attach(Choice::New, Duration::ZERO).expect("attached");
    assert_ne!(third.desktop, first.desktop);
    assert_ne!(third.desktop, second.desktop);
    let listed = third_link.list().expect("listed");
    assert_eq!(listed.len(), 3);
    assert!(listed.iter().all(|desktop| desktop.attached));
    drop((first_link, first, second_link, second, third_link, third));
}

#[test]
fn the_background_stays_while_a_program_runs_and_ends_once_it_is_alone() {
    let scratch = Scratch::new("stays");
    let path = scratch.socket();
    let listening = Listening::on(&path).expect("listening");
    let asked = std::sync::Arc::new(AtomicUsize::new(0));
    let counted = std::sync::Arc::clone(&asked);
    // Never before the first screen; a short while every time after.
    let server = thread::spawn(move || {
        serve::run(&listening, || (counted.fetch_add(1, Ordering::SeqCst) > 0).then(|| Duration::from_millis(10)))
    });
    let go = scratch.file("go");
    let (link, _) = attach(&path);
    let program = format!("while [ ! -e '{}' ]; do sleep 0.02; done", go.display());
    let opened = link.open(&script(&program, &scratch.0, &[]), Vec::new(), Duration::ZERO).expect("started");
    drop(opened);
    drop(link);
    wait_for("the background to see the screen go", || {
        listed(&path)
            .is_some_and(|desktops| desktops.first().is_some_and(|desktop| !desktop.attached && desktop.running == 1))
    });
    // Every probe above was a screen coming and going; none found it alone.
    assert_eq!(asked.load(Ordering::SeqCst), 1, "a running program keeps it from waiting alone");
    assert!(!server.is_finished());

    fs::write(&go, "").expect("told to end");
    wait_for("the background ending once alone", || server.is_finished());
    server.join().expect("no panic").expect("served");
    assert!(!path.exists(), "the socket is taken away");
}
