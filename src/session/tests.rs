//! What the desktop's programs do, tried against a real shell: the environment and the folder
//! they get, the size of their first screen, the lines they remember, what they say about
//! themselves, how often they may ask for a frame, how they end and how they start again.
//!
//! Every wait here has a bound: a program that says nothing fails its test instead of hanging it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use qframe::event::MouseKind;
use qframe::prelude::*;

use super::{Change, Report, Sessions, Start, Subtitle, subtitle};
use crate::apps::{Declared, Entry, Environment, Launch, Screen, Source, parse_entry};
use crate::settings::Prefs;
use crate::wm::{WindowId, Windows};

/// The longest a test waits for a program to say the thing it is waiting for.
///
/// It is generous on purpose. What is waited for arrives in milliseconds on a quiet machine, and
/// the only thing a longer wait costs is how long a genuinely broken desktop takes to fail. A
/// tighter bound buys nothing and turns a machine with a few hundred other processes on it into a
/// failing test with nothing wrong: real programs on a real pseudo-terminal are scheduled by the
/// system, not by this test.
const WAIT: Duration = Duration::from_secs(30);

/// The size a window's body has in these tests, in columns and rows.
const BODY: Size = Size::new(100, 18);

/// A folder of its own under the system's temporary folder, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-session-{}-{name}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch folder");
        Self(path)
    }

    /// What the program wrote into the file `name` of this folder, waited for.
    ///
    /// A file that is there is not yet a file that has been written: `printf 'x' > f` makes the
    /// file empty first and fills it a moment later, so a reader that takes the first successful
    /// read gets an empty string and the test fails with nothing broken. The wait is therefore
    /// for something to be in it, not for it to exist.
    fn said(&self, name: &str) -> String {
        let path = self.0.join(name);
        let deadline = Instant::now() + WAIT;
        while Instant::now() < deadline {
            match std::fs::read_to_string(&path) {
                Ok(text) if !text.is_empty() => return text,
                _ => {}
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("the program wrote something into {}", path.display());
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// An environment that reads nothing of the machine the test runs on: a home folder of its own
/// and `/bin/sh`, which every machine these tests run on has.
fn environment(home: &Path) -> Environment {
    Environment {
        home: Some(home.to_path_buf()),
        data_home: None,
        data_dirs: Vec::new(),
        path: None,
        shell: Some(PathBuf::from("/bin/sh")),
        editor: None,
    }
}

/// The entry an entry file declares, as the desktop reads it.
fn entry_from(id: &str, text: &str, source: Source) -> Entry {
    let (declared, diagnostics) = parse_entry(id, Path::new("deneme.toml"), text.as_bytes(), source, None);
    assert!(diagnostics.is_empty(), "the test's own entry file is sound: {diagnostics:?}");
    match declared {
        Some(Declared::Entry(entry)) => *entry,
        other => panic!("the entry file declares an application, not {other:?}"),
    }
}

/// An entry that runs `script` with `/bin/sh` in `folder`, with `env` as its own variables.
///
/// The script is written into a TOML string, so its backslashes are doubled; it holds no double
/// quote of its own, and the values it reads hold no spaces.
fn script(folder: &Path, script: &str, env: &[(&str, &str)]) -> Entry {
    assert!(!script.contains('"'), "a test script is written without double quotes");
    let mut text = format!(
        "name = \"Deneme\"\ncommand = [\"/bin/sh\", \"-c\", \"{}\"]\nfolder = \"{}\"\n",
        script.replace('\\', "\\\\"),
        folder.display()
    );
    if !env.is_empty() {
        text.push_str("\n[env]\n");
        for (name, value) in env {
            text.push_str(&format!("{name} = \"{value}\"\n"));
        }
    }
    entry_from("deneme", &text, Source::User)
}

/// A desktop of one window holding `entry`, with the programs of `prefs` over this link.
fn desk(entry: &Entry, home: &Path, prefs: Prefs, remote: bool) -> (Sessions, WindowId) {
    let mut windows = Windows::new(Size::new(80, 24));
    let window = windows.open(entry);
    (Sessions::new(&environment(home), prefs, remote), window)
}

/// Waits on a thread of its own for the next report of `watch`, failing instead of hanging.
///
/// This is what the desktop's performed command does; a thread still waiting when the bound is
/// over is left to the end of the test process.
fn waited(watch: super::Watch) -> Report {
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(watch.next());
    });
    receiver.recv_timeout(WAIT).expect("the program said something within the wait")
}

/// Waits for the next report of the program of `window`.
fn next(sessions: &Sessions, window: WindowId) -> Report {
    waited(sessions.watch(window).expect("a program to watch"))
}

/// Takes in reports of the program of `window` until `enough` is happy with the changes so far,
/// and gives them all back. A program that ends first stops the gathering too.
fn gather(sessions: &mut Sessions, window: WindowId, enough: impl Fn(&[Change]) -> bool) -> Vec<Change> {
    let mut changes = Vec::new();
    let deadline = Instant::now() + WAIT;
    while !enough(&changes) && Instant::now() < deadline {
        let report = next(sessions, window);
        let Some(change) = sessions.accept(report) else { continue };
        let ended = matches!(change, Change::Ended { .. });
        changes.push(change);
        if ended {
            break;
        }
    }
    assert!(enough(&changes), "the program said what the test waited for: {changes:?}");
    changes
}

/// Takes in reports until the program of `window` ends, and gives back every change.
fn until_ended(sessions: &mut Sessions, window: WindowId) -> Vec<Change> {
    gather(sessions, window, |changes| changes.iter().any(|change| matches!(change, Change::Ended { .. })))
}

#[test]
fn the_program_gets_the_desktops_variables_the_entrys_own_and_its_folder() {
    let scratch = Scratch::new("env");
    let home = Scratch::new("home");
    let entry = script(
        &scratch.0,
        "printf '%s|%s|%s|%s|%s' $TERM $COLORTERM $QDESK $GREETING $PWD > said",
        &[("GREETING", "merhaba")],
    );
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    until_ended(&mut sessions, window);
    assert_eq!(scratch.said("said"), format!("xterm-256color|truecolor|1|merhaba|{}", scratch.0.display()));
}

#[test]
fn an_entrys_own_variable_replaces_the_desktops() {
    let scratch = Scratch::new("replace");
    let home = Scratch::new("home");
    let entry = script(&scratch.0, "printf '%s' $TERM > said", &[("TERM", "dumb")]);
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    until_ended(&mut sessions, window);
    assert_eq!(scratch.said("said"), "dumb");
}

#[test]
fn a_program_starts_on_a_screen_the_size_of_the_windows_body() {
    let scratch = Scratch::new("size");
    let home = Scratch::new("home");
    let entry = script(&scratch.0, "stty size > size", &[]);
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    until_ended(&mut sessions, window);
    // Rows first, as stty writes them: the body is 100 columns and 18 rows.
    assert_eq!(scratch.said("size").trim(), "18 100");
}

#[test]
fn a_terminal_window_runs_the_shell_of_the_environment_in_the_home_folder() {
    let home = Scratch::new("shell-home");
    let entry = entry_from("terminal", "name = \"Terminal\"\nscreen = \"terminal\"\n", Source::Builtin);
    assert_eq!(entry.launch, Launch::Screen(Screen::Terminal));
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    assert!(sessions.terminal(window).is_some(), "the window's body draws the shell");
    assert_eq!(sessions.subtitle(window), Some(Subtitle::Folder(home.0.as_path())));
    assert!(sessions.is_running(window));
    assert_eq!(sessions.running(), 1);
    assert!(sessions.close(window));
    assert_eq!(sessions.running(), 0);
}

#[test]
fn a_screen_of_qdesk_starts_no_program() {
    let home = Scratch::new("screen-home");
    let entry = entry_from("settings", "name = \"Ayarlar\"\nscreen = \"settings\"\n", Source::Builtin);
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Screen));
    assert!(sessions.watch(window).is_none());
    assert!(sessions.terminal(window).is_none());
    assert_eq!(sessions.running(), 0);
}

#[test]
fn a_program_that_is_not_there_fails_instead_of_opening_a_window_onto_nothing() {
    let home = Scratch::new("missing-home");
    let entry = entry_from("yok", "name = \"Yok\"\ncommand = [\"/nonexistent/qdesk-deneme-program\"]\n", Source::User);
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    match sessions.start(window, &entry, BODY) {
        Start::Failed(error) => assert!(!error.to_string().is_empty(), "the failure says what went wrong"),
        other => panic!("a program that is not there cannot start: {other:?}"),
    }
    assert_eq!(sessions.running(), 0);
    assert!(sessions.watch(window).is_none());
}

#[test]
fn the_title_the_folder_the_bell_and_both_kinds_of_notification_arrive() {
    let scratch = Scratch::new("notices");
    let home = Scratch::new("home");
    let entry = script(
        &scratch.0,
        "printf '\\033]0;derleniyor\\007'; printf '\\033]7;file://localhost/tmp/isler\\007'; \
         printf '\\a'; printf '\\033]9;bitti\\007'; printf '\\033]777;notify;Yedek;tamam\\007'; printf 'son\\r\\n'",
        &[],
    );
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    let changes = until_ended(&mut sessions, window);
    assert!(changes.contains(&Change::Title(Some("derleniyor".to_owned()))), "{changes:?}");
    assert!(changes.contains(&Change::Folder(PathBuf::from("/tmp/isler"))), "{changes:?}");
    assert!(changes.contains(&Change::Bell), "{changes:?}");
    assert!(
        changes.contains(&Change::Notify { title: None, body: "bitti".to_owned() }),
        "OSC 9 is a notification with no title: {changes:?}"
    );
    assert!(
        changes.contains(&Change::Notify { title: Some("Yedek".to_owned()), body: "tamam".to_owned() }),
        "OSC 777 carries a title: {changes:?}"
    );
    assert!(changes.contains(&Change::Output), "the text it printed is output: {changes:?}");
    // The title it gave itself stands in the strip; the folder it reported is behind it.
    assert_eq!(sessions.subtitle(window), Some(Subtitle::Title("derleniyor")));
}

#[test]
fn a_cleared_title_leaves_the_folder_the_program_reported() {
    let scratch = Scratch::new("cleared");
    let home = Scratch::new("home");
    let entry = script(
        &scratch.0,
        "printf '\\033]2;derleniyor\\007'; printf '\\033]7;file://localhost/tmp/isler\\007'; printf '\\033]2;\\007'",
        &[],
    );
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    until_ended(&mut sessions, window);
    assert_eq!(sessions.subtitle(window), Some(Subtitle::Folder(Path::new("/tmp/isler"))));
}

#[test]
fn the_subtitle_takes_the_title_then_the_reported_folder_then_the_one_it_started_in() {
    let started = Path::new("/home/kisi");
    let said = Path::new("/home/kisi/isler");
    assert_eq!(subtitle(Some("htop"), Some(said), started), Subtitle::Title("htop"));
    assert_eq!(subtitle(None, Some(said), started), Subtitle::Folder(said));
    assert_eq!(subtitle(None, None, started), Subtitle::Folder(started));
    // A title of nothing but spaces is no title, and a title keeps no padding of its own.
    assert_eq!(subtitle(Some("   "), None, started), Subtitle::Folder(started));
    assert_eq!(subtitle(Some("  htop "), None, started), Subtitle::Title("htop"));
}

/// How often the same chatty program asked for a frame under a cap of `frames` a second, and how
/// long it ran. `late` gives the cap the way a running desktop gets it: after the programs were
/// set up, as the settings file is read and as the Settings screen changes it.
fn chatter(frames: u16, late: bool) -> (usize, Duration) {
    let scratch = Scratch::new("chatter");
    let home = Scratch::new("home");
    // Two thousand lines of eighty characters: a quarter of a megabyte, dozens of reads of the
    // pseudo-terminal, written as fast as the shell can.
    let entry = script(
        &scratch.0,
        "i=0; while [ $i -lt 2000 ]; do printf '%s\\r\\n' \
         aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa; i=$((i+1)); done",
        &[],
    );
    let prefs = Prefs { frame_cap: Some(frames), ..Prefs::default() };
    let (mut sessions, window) = desk(&entry, &home.0, if late { Prefs::default() } else { prefs }, false);
    if late {
        sessions.set_prefs(prefs, false);
    }
    let started = Instant::now();
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    let changes = until_ended(&mut sessions, window);
    (changes.iter().filter(|change| matches!(change, Change::Output)).count(), started.elapsed())
}

#[test]
fn output_flowing_fast_asks_for_a_frame_at_the_frame_cap_not_at_every_read() {
    let (capped, elapsed) = chatter(1, false);
    let (loose, _) = chatter(240, false);
    // One frame a second: a run of a second or two reports its output that many times, never once
    // per read of the pseudo-terminal.
    let allowed = 2 + usize::try_from(elapsed.as_millis() / 1000).unwrap_or(usize::MAX);
    assert!(capped <= allowed, "a cap of one frame a second reported {capped} times in {elapsed:?}");
    assert!(loose > capped, "the same program with a loose cap reports more often: {loose} against {capped}");
}

#[test]
fn a_frame_cap_given_after_the_programs_were_set_up_is_the_one_the_next_program_reports_at() {
    // The running desktop sets its programs up before it reads the settings file, and the
    // Settings screen changes the cap long after: both reach the programs this way.
    let (capped, elapsed) = chatter(1, true);
    let allowed = 2 + usize::try_from(elapsed.as_millis() / 1000).unwrap_or(usize::MAX);
    assert!(capped <= allowed, "a cap of one frame a second, given late, reported {capped} times in {elapsed:?}");
}

/// A desktop of one window, drawn the way the view draws the body of a window.
struct WindowBody {
    sessions: Sessions,
    window: WindowId,
}

impl App for WindowBody {
    type Msg = ();

    fn update(&mut self, (): ()) -> Command<()> {
        Command::none()
    }

    fn view(&self, ui: &mut View<'_, ()>) {
        if let Some(terminal) = self.sessions.terminal(self.window) {
            ui.add(terminal).width(Length::Fill(1)).height(Length::Fill(1));
        }
    }
}

#[test]
fn a_window_remembers_only_as_many_lines_as_the_settings_allow() {
    /// The number of the line at the top of a window of twenty by five, after scrolling as far
    /// back as a window that remembers `lines` lets anyone scroll.
    fn top_line(lines: u16, scrolled: bool) -> u32 {
        let scratch = Scratch::new("scrollback");
        let home = Scratch::new("home");
        let entry = script(&scratch.0, "i=1; while [ $i -le 60 ]; do printf 'l%s\\r\\n' $i; i=$((i+1)); done", &[]);
        let prefs = Prefs { scrollback: lines, ..Prefs::default() };
        let (mut sessions, window) = desk(&entry, &home.0, prefs, false);
        assert!(matches!(sessions.start(window, &entry, Size::new(20, 5)), Start::Running));
        until_ended(&mut sessions, window);
        let mut harness = Harness::new(WindowBody { sessions, window }, 20, 5);
        harness.render();
        if scrolled {
            // Far more wheel steps than the window could ever give back.
            for _ in 0..20 {
                harness.mouse(MouseKind::ScrollUp, 5, 2);
            }
        }
        let screen = harness.screen();
        let top = screen.lines().next().unwrap_or_default().trim().to_owned();
        top.strip_prefix('l').and_then(|number| number.parse().ok()).unwrap_or_else(|| panic!("a line, not {top:?}"))
    }

    let live = top_line(2, false);
    assert!(live > 50, "the window shows the end of the sixty lines: l{live}");
    // Two lines of memory: scrolling back reaches exactly two lines further, however far the
    // wheel is turned.
    assert_eq!(top_line(2, true), live - 2);
    // No memory at all: there is nothing above the screen to reach.
    assert_eq!(top_line(0, true), top_line(0, false));
}

#[test]
fn an_exit_code_arrives_and_the_window_stops_counting_as_running() {
    let scratch = Scratch::new("exit");
    let home = Scratch::new("home");
    let entry = script(&scratch.0, "exit 3", &[]);
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    let changes = until_ended(&mut sessions, window);
    assert_eq!(changes.last(), Some(&Change::Ended { code: Some(3) }));
    assert_eq!(sessions.exit(window), Some(Some(3)));
    assert!(!sessions.is_running(window));
    assert_eq!(sessions.running(), 0);
    // The window keeps its last screen, so its body still has something to draw.
    assert!(sessions.terminal(window).is_some());
}

#[test]
fn the_process_id_is_the_programs_own() {
    let scratch = Scratch::new("pid");
    let home = Scratch::new("home");
    let entry = script(&scratch.0, "printf '%s' $$ > pid; while :; do sleep 1; done", &[]);
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    assert_eq!(sessions.pid(window).map(|pid| pid.to_string()).as_deref(), Some(scratch.said("pid").as_str()));
    assert!(sessions.close(window));
}

#[test]
fn killing_a_program_that_will_not_answer_ends_it_and_the_window_hears_of_it() {
    let scratch = Scratch::new("kill");
    let home = Scratch::new("home");
    let entry = script(&scratch.0, "trap '' HUP TERM INT; printf 'hazir' > ready; while :; do sleep 1; done", &[]);
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    assert_eq!(scratch.said("ready"), "hazir");
    assert!(sessions.kill(window));
    let changes = until_ended(&mut sessions, window);
    assert!(matches!(changes.last(), Some(Change::Ended { .. })), "{changes:?}");
    assert!(!sessions.is_running(window));
}

#[test]
fn a_closing_window_asks_its_program_to_go_and_one_that_answers_goes_before_the_grace() {
    let scratch = Scratch::new("polite");
    let home = Scratch::new("home");
    let entry = script(&scratch.0, "printf 'hazir' > ready; while :; do sleep 1; done", &[]);
    // A grace far longer than the wait below, so what this proves cannot be a matter of timing:
    // the end can only arrive within `WAIT` because the hangup was heard. If the program had to
    // be killed instead, the grace would still be running and the wait would come back empty.
    // Measuring how long it took and calling anything under the grace a pass would be the same
    // claim made by a race, and a loaded machine would fail it with nothing broken.
    let grace = WAIT * 12;
    let mut windows = Windows::new(Size::new(80, 24));
    let window = windows.open(&entry);
    let mut sessions = Sessions::new(&environment(&home.0), Prefs::default(), false).grace(grace);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    assert_eq!(scratch.said("ready"), "hazir");
    // The watch is made before the window closes: the program is asked to go, not dropped, so the
    // watch still hears the end.
    let watch = sessions.watch(window).expect("a program to watch");
    assert!(sessions.close(window));
    // How many programs are still closing cannot be claimed here: a program that answers the
    // hangup may already have gone by the next line, and asserting it had not would be a race
    // that a loaded machine loses with nothing broken. The count while the grace runs is claimed
    // by the test below, whose program cannot go, and what this one proves is the end arriving.
    // The wait is bounded well inside the grace, so an end that arrives at all is one the hangup
    // brought about.
    let report = waited(watch);
    assert!(matches!(report.change(), Change::Ended { .. }), "{report:?}");
    // Its window is gone, so what it says is stale, and nothing holds it any more.
    assert_eq!(sessions.accept(report), None);
    assert_eq!(sessions.closing(), 0);
}

#[test]
fn a_program_that_ignores_the_hangup_is_killed_when_the_grace_runs_out() {
    let scratch = Scratch::new("stubborn");
    let home = Scratch::new("home");
    let entry = script(&scratch.0, "trap '' HUP; printf 'hazir' > ready; while :; do sleep 1; done", &[]);
    let grace = Duration::from_millis(600);
    let mut windows = Windows::new(Size::new(80, 24));
    let window = windows.open(&entry);
    let mut sessions = Sessions::new(&environment(&home.0), Prefs::default(), false).grace(grace);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    assert_eq!(scratch.said("ready"), "hazir");
    let watch = sessions.watch(window).expect("a program to watch");
    let started = Instant::now();
    assert!(sessions.close(window));
    // This program cannot answer the hangup, so it is certainly still there: the closed window's
    // program is held, not dropped, until its grace is over.
    assert_eq!(sessions.closing(), 1, "the program is held while its grace runs");
    let report = waited(watch);
    let waited_for = started.elapsed();
    assert!(matches!(report.change(), Change::Ended { .. }), "{report:?}");
    assert!(waited_for >= grace, "a program that ignores the hangup lives out its grace: {waited_for:?}");
}

#[test]
fn a_window_starts_its_program_again_in_place_and_the_old_ones_words_are_stale() {
    let scratch = Scratch::new("restart");
    let home = Scratch::new("home");
    let entry = script(&scratch.0, "printf 'x' >> runs; printf '\\033]0;birinci\\007'", &[]);
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(matches!(sessions.start(window, &entry, BODY), Start::Running));
    let changes = until_ended(&mut sessions, window);
    assert!(changes.contains(&Change::Title(Some("birinci".to_owned()))), "{changes:?}");
    // The first program of this desktop; after the restart its reports are of a program that is
    // no longer there.
    let stale = Report { window, run: 1, change: Change::Output };

    assert!(matches!(sessions.restart(window, BODY), Some(Start::Running)));
    assert_eq!(sessions.accept(stale), None, "what the program before it said no longer counts");
    let changes = until_ended(&mut sessions, window);
    assert!(changes.contains(&Change::Title(Some("birinci".to_owned()))), "{changes:?}");
    assert_eq!(scratch.said("runs"), "xx", "the same entry ran twice");
    // A window that never held a program has nothing to start again.
    let (mut empty, other) = desk(&entry, &home.0, Prefs::default(), false);
    assert!(empty.restart(other, BODY).is_none());
}

#[test]
fn running_counts_only_the_programs_still_going() {
    let scratch = Scratch::new("count");
    let home = Scratch::new("home");
    let long = script(&scratch.0, "printf 'hazir' > ready; while :; do sleep 1; done", &[]);
    let short = script(&scratch.0, "exit 0", &[]);
    let mut windows = Windows::new(Size::new(80, 24));
    let waiting = windows.open(&long);
    let ending = windows.open(&short);
    let mut sessions = Sessions::new(&environment(&home.0), Prefs::default(), false);
    assert!(matches!(sessions.start(waiting, &long, BODY), Start::Running));
    assert!(matches!(sessions.start(ending, &short, BODY), Start::Running));
    assert_eq!(scratch.said("ready"), "hazir");
    until_ended(&mut sessions, ending);
    assert_eq!(sessions.running(), 1, "one of the two windows still has a program");
    assert!(sessions.is_running(waiting));
    assert!(!sessions.is_running(ending));
    assert!(sessions.close(waiting));
    assert_eq!(sessions.running(), 0);
}

#[test]
fn a_report_of_a_window_that_holds_no_program_is_stale() {
    let home = Scratch::new("home");
    let entry = entry_from("settings", "name = \"Ayarlar\"\nscreen = \"settings\"\n", Source::Builtin);
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    assert_eq!(sessions.accept(Report { window, run: 1, change: Change::Bell }), None);
}

#[test]
fn settings_the_person_changes_reach_the_programs_that_start_after_them() {
    let scratch = Scratch::new("prefs");
    let home = Scratch::new("home");
    let entry = script(&scratch.0, "stty size > size", &[]);
    let (mut sessions, window) = desk(&entry, &home.0, Prefs::default(), false);
    sessions.set_prefs(Prefs { scrollback: 0, frame_cap: Some(20), ..Prefs::default() }, true);
    assert!(matches!(sessions.start(window, &entry, Size::new(90, 12)), Start::Running));
    until_ended(&mut sessions, window);
    assert_eq!(scratch.said("size").trim(), "12 90");
}
