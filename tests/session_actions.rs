//! The session actions at the foot of the launcher — lock, log out, restart, power off — and the
//! lock screen.
//!
//! Every test starts where a person starts: the launcher's button on the dock, the action's button,
//! the answer in the question, the password typed into the lock screen. Nothing is ever powered
//! off. The desktop is given a `PATH` of the test's own, with a `systemctl` and a `unix_chkpwd` the
//! test wrote: the first writes down what it was asked, the second says yes only to `dogru`. They
//! are the only ones the desktop can find, since the folders of the system's own programs are not
//! looked in here.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::{Desk, LOCK_FIELD};
use qdesk::apps::Environment;
use qdesk::desktop::Desktop;
use qdesk::wm::Run;
use qframe::color::ColorDepth;
use qframe::env::{AssetDirs, Env};
use qframe::icons::GlyphMode;
use qframe::prelude::*;

use support::{BUDGET, ICONS, MACHINE, MOMENT, OFFSET, PATIENCE, decoration};

/// The password the test's `unix_chkpwd` takes.
const PASSWORD: &str = "dogru";

/// A folder of one test in the temporary folder, taken away with everything in it when the test
/// ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-session-actions-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("bin")).expect("the PATH folder is made");
        Self(path)
    }

    /// Where the test's `systemctl` writes the words it was given, one to a line.
    fn asked(&self) -> PathBuf {
        self.0.join("systemctl-asked")
    }

    /// A `systemctl` that writes down what it was asked and ends with `code`, saying `said` on its
    /// error stream first when it is not empty.
    fn systemctl(&self, said: &str, code: i32) {
        let complaint = if said.is_empty() { String::new() } else { format!("echo '{said}' >&2\n") };
        script(
            &self.0.join("bin/systemctl"),
            &format!("printf '%s\\n' \"$@\" >> '{}'\n{complaint}exit {code}", self.asked().display()),
        );
    }

    /// A `unix_chkpwd` that says yes to [`PASSWORD`] alone, read from a pipe up to its NUL byte as
    /// the system's own does.
    fn chkpwd(&self) {
        script(
            &self.0.join("bin/unix_chkpwd"),
            &format!(
                "[ \"$1 $2\" = 'ada nullok' ] || exit 8\n[ -t 0 ] && exit 9\n\
                 got=$(od -An -c | tr -d ' \\n')\n[ \"$got\" = '{PASSWORD}\\0' ]"
            ),
        );
    }

    /// The shell a Terminal window of these tests runs: the first one opened waits for the file
    /// `go` and ends with code 3, every later one waits quietly until it is closed.
    fn shell(&self) -> PathBuf {
        let path = self.0.join("shell");
        let dir = self.0.display();
        script(
            &path,
            &format!(
                "if [ -e '{dir}/first' ]; then exec sleep 600; fi\ntouch '{dir}/first'\n\
                 while [ ! -e '{dir}/go' ]; do sleep 0.02; done\nexit 3"
            ),
        );
        path
    }

    fn environment(&self) -> Environment {
        Environment {
            home: Some(self.0.clone()),
            path: Some(self.0.join("bin").into_os_string()),
            shell: Some(self.shell()),
            user: Some("ada".to_owned()),
            ..Environment::default()
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Writes an executable shell script at `path` running `body`.
fn script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("the script is written");
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("the script can be run");
}

fn env() -> Env {
    let dirs = AssetDirs {
        locale_sources: qdesk::locales().iter().map(|(file, text)| ((*file).to_owned(), (*text).to_owned())).collect(),
        keymap_source: Some({
            let (file, text) = qdesk::keymap();
            (file.to_owned(), text.to_owned())
        }),
        ..AssetDirs::default()
    };
    Env::load(&dirs).expect("the built-in files load")
}

/// The usual floor at 80 × 24 in the environment of `scratch`, on a terminal that is `remote` or
/// not.
fn desk(scratch: &Scratch, remote: bool) -> Harness<Desk> {
    let desktop = Desktop {
        icons: ICONS.map(str::to_owned).to_vec(),
        welcome_seen: true,
        resize_hint_seen: true,
        ..Desktop::default()
    };
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), Box::new(|| MOMENT * 1_000))
        .apps(scratch.environment())
        .catalog(support::catalog())
        .desktop(desktop)
        .remote(remote)
        .watch_within(PATIENCE);
    let mut harness = Harness::with_env(app, env(), 80, 24);
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

/// Opens the launcher with the button at the left end of the dock.
fn open_launcher(harness: &mut Harness<Desk>) {
    harness.click(2, 23);
    assert!(harness.app().launcher().is_some(), "{}", harness.screen());
}

/// Clicks `label` on the lowest row it stands on that is not a question: the launcher's foot and
/// a dialog's buttons are below their titles.
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

/// Opens a Terminal window from its icon on the floor, as a person does, and waits for its shell.
fn open_terminal(harness: &mut Harness<Desk>) {
    let (x, y) = harness.find("Terminal").expect("the Terminal icon");
    harness.click(x, y);
    harness.click(x, y);
    until(harness, "a running shell", |harness| {
        harness.app().windows().iter().any(|window| window.run() == Some(Run::Running))
    });
}

/// What the test's `systemctl` was asked, empty when it never ran.
fn asked(scratch: &Scratch) -> String {
    fs::read_to_string(scratch.asked()).unwrap_or_default()
}

#[test]
fn the_launcher_offers_every_session_action_at_its_foot() {
    let scratch = Scratch::new();
    scratch.systemctl("", 0);
    scratch.chkpwd();
    let mut harness = desk(&scratch, false);
    open_launcher(&mut harness);
    let screen = harness.screen();
    let foot = screen.lines().find(|row| row.contains("Power off")).expect("the row of the actions");
    for action in ["Lock", "Log out", "Restart", "Power off"] {
        assert!(foot.contains(action), "{action} is missing:\n{screen}");
    }
    // Gentlest first, the one that ends the most at the far end.
    let at = |label: &str| foot.find(label).unwrap_or(0);
    assert!(at("Lock") < at("Log out") && at("Log out") < at("Restart") && at("Restart") < at("Power off"));
    for depth in [ColorDepth::Ansi16, ColorDepth::TrueColor] {
        for mode in [GlyphMode::Ascii, GlyphMode::Unicode] {
            harness.set_depth(depth).set_glyph_mode(mode);
            assert_eq!(decoration(&harness.screen()), None, "{depth:?} {mode:?}:\n{}", harness.screen());
        }
    }
}

#[test]
fn over_ssh_only_logging_out_is_offered() {
    let scratch = Scratch::new();
    scratch.systemctl("", 0);
    scratch.chkpwd();
    let mut harness = desk(&scratch, true);
    open_launcher(&mut harness);
    let screen = harness.screen();
    assert!(screen.contains("Log out"), "{screen}");
    for hidden in ["Lock", "Restart", "Power off"] {
        assert!(!screen.contains(hidden), "{hidden} would reach everyone on the server:\n{screen}");
    }
}

#[test]
fn a_missing_helper_hides_its_action() {
    let scratch = Scratch::new();
    let mut harness = desk(&scratch, false);
    open_launcher(&mut harness);
    let screen = harness.screen();
    assert!(screen.contains("Log out"), "{screen}");
    for hidden in ["Lock", "Restart", "Power off"] {
        assert!(!screen.contains(hidden), "{hidden} with nothing to carry it out:\n{screen}");
    }
}

#[test]
fn powering_off_asks_first_counting_the_programs_and_then_asks_systemctl() {
    let scratch = Scratch::new();
    scratch.systemctl("", 0);
    let mut harness = desk(&scratch, false);
    open_terminal(&mut harness);
    open_launcher(&mut harness);
    press(&mut harness, "Power off");
    let question = harness.screen();
    assert!(question.contains("Power off this machine?"), "{question}");
    assert!(question.contains("1 program is still running"), "the question counts the programs:\n{question}");
    assert!(asked(&scratch).is_empty(), "nothing is done before the answer");
    // No is no.
    press(&mut harness, "Cancel");
    assert!(!harness.screen().contains("Power off this machine?"), "{}", harness.screen());
    assert!(asked(&scratch).is_empty(), "a cancelled question does nothing");
    // Yes is systemctl, asked without a password prompt.
    open_launcher(&mut harness);
    press(&mut harness, "Power off");
    press(&mut harness, "Power off");
    until(&mut harness, "systemctl being asked", |_| !asked(&scratch).is_empty());
    assert_eq!(asked(&scratch), "--no-ask-password\npoweroff\n");
}

#[test]
fn restarting_an_idle_machine_says_what_happens_and_asks_systemctl_to_reboot() {
    let scratch = Scratch::new();
    scratch.systemctl("", 0);
    let mut harness = desk(&scratch, false);
    open_launcher(&mut harness);
    press(&mut harness, "Restart");
    let question = harness.screen();
    assert!(question.contains("Restart this machine?"), "{question}");
    assert!(question.contains("stops and starts again"), "{question}");
    press(&mut harness, "Restart");
    until(&mut harness, "systemctl being asked", |_| !asked(&scratch).is_empty());
    assert_eq!(asked(&scratch), "--no-ask-password\nreboot\n");
}

#[test]
fn a_machine_that_refuses_to_power_off_says_why_in_the_corner() {
    let scratch = Scratch::new();
    scratch.systemctl("Access denied", 1);
    let mut harness = desk(&scratch, false);
    open_launcher(&mut harness);
    press(&mut harness, "Power off");
    press(&mut harness, "Power off");
    until(&mut harness, "the refusal", |harness| harness.screen().contains("could not be powered off"));
    assert!(harness.screen().contains("Access denied"), "in the system's own words:\n{}", harness.screen());
}

#[test]
fn logging_out_leaves_qdesk_and_asks_first_while_programs_run() {
    let scratch = Scratch::new();
    let mut idle = desk(&scratch, false);
    open_launcher(&mut idle);
    press(&mut idle, "Log out");
    assert!(idle.quit_requested(), "nothing running, nothing to ask:\n{}", idle.screen());

    let scratch = Scratch::new();
    let mut busy = desk(&scratch, false);
    open_terminal(&mut busy);
    open_launcher(&mut busy);
    press(&mut busy, "Log out");
    let question = busy.screen();
    assert!(!busy.quit_requested(), "{question}");
    assert!(question.contains("Quit qdesk?") && question.contains("1 program is still running"), "{question}");
    press(&mut busy, "Quit");
    assert!(busy.quit_requested(), "{}", busy.screen());
}

#[test]
fn the_lock_screen_covers_everything_and_only_the_right_password_opens_it() {
    let scratch = Scratch::new();
    scratch.chkpwd();
    let mut harness = desk(&scratch, false);
    open_terminal(&mut harness);
    open_launcher(&mut harness);
    press(&mut harness, "Lock");
    assert!(harness.app().locked(), "{}", harness.screen());
    assert!(harness.is_focused(LOCK_FIELD), "the keys are in the password field at once");
    let screen = harness.screen();
    assert!(screen.contains("Password for ada") && screen.contains(MACHINE), "{screen}");
    assert!(!screen.contains("Terminal") && !screen.contains('❖'), "nothing of the desktop shows:\n{screen}");
    assert_eq!(decoration(&screen), None, "{screen}");

    // The desktop's keys reach nothing under it, and qdesk is not left.
    harness.press("ctrl+alt+space").press("space");
    assert!(harness.app().locked() && harness.app().keys().is_none() && harness.app().launcher().is_none());
    harness.press("ctrl+q");
    assert!(!harness.quit_requested(), "leaving would be a way past the lock:\n{}", harness.screen());

    // The space went where every key goes now: into the field.
    harness.press("backspace");
    // A wrong password keeps it locked and says so; the field is emptied.
    harness.type_text("yanlis").press("enter");
    until(&mut harness, "the answer", |harness| harness.screen().contains("That is not the password"));
    assert!(harness.app().locked());
    assert!(harness.is_focused(LOCK_FIELD), "the keys stay in the field");
    assert!(harness.screen().contains("Password for ada"), "the field is empty again:\n{}", harness.screen());

    harness.type_text(PASSWORD).press("enter");
    until(&mut harness, "the desktop", |harness| !harness.app().locked());
    assert!(harness.screen().contains('❖'), "the dock is back:\n{}", harness.screen());
    assert_eq!(harness.app().windows().len(), 1, "the window is where it was");
}

#[test]
fn a_program_that_ends_behind_the_lock_says_nothing_until_it_is_opened() {
    let scratch = Scratch::new();
    scratch.chkpwd();
    let mut harness = desk(&scratch, false);
    // The first window waits for the file `go`; a second one, opened from the launcher's card,
    // stands in front of it, so the first is not the one being looked at when it ends.
    open_terminal(&mut harness);
    until(&mut harness, "the first shell", |_| scratch.0.join("first").exists());
    open_launcher(&mut harness);
    let (x, y) = harness.find("❯ Terminal").expect("the Terminal card");
    harness.click(x, y);
    until(&mut harness, "the second shell", |harness| {
        let windows = harness.app().windows();
        windows.len() == 2 && windows.iter().all(|window| window.run() == Some(Run::Running))
    });
    open_launcher(&mut harness);
    press(&mut harness, "Lock");
    fs::write(scratch.0.join("go"), "").expect("the first program is told to end");
    until(&mut harness, "the end of the first program", |harness| {
        harness.app().windows().iter().any(|window| matches!(window.run(), Some(Run::Ended { .. })))
    });
    let screen = harness.screen();
    assert!(!screen.contains("exit code 3"), "the corner would show it to anyone passing:\n{screen}");
    harness.type_text(PASSWORD).press("enter");
    until(&mut harness, "the desktop", |harness| !harness.app().locked());
    // The list kept it for the person who unlocked: the dock counts it.
    let dock = harness.screen().lines().last().unwrap_or_default().to_owned();
    assert!(dock.contains("●1"), "the dock counts what the list kept: {dock:?}");
}

#[test]
fn a_machine_going_down_is_not_kept_waiting_by_the_lock_screen() {
    let scratch = Scratch::new();
    scratch.chkpwd();
    let mut harness = desk(&scratch, false);
    open_terminal(&mut harness);
    open_launcher(&mut harness);
    press(&mut harness, "Lock");
    assert!(harness.app().locked());
    // The system ending qdesk is not a person at the keyboard: nobody is there to unlock and answer.
    harness.terminate(qframe::runtime::Termination::Terminate);
    assert!(harness.quit_requested(), "{}", harness.screen());
}

#[test]
fn an_answer_the_launcher_never_asked_for_powers_nothing_off_over_ssh() {
    let scratch = Scratch::new();
    scratch.systemctl("", 0);
    let mut harness = desk(&scratch, true);
    // No button leads here over SSH; the message alone must not be enough either.
    harness.send(qdesk::app::Msg::PowerConfirmed(qdesk::power::Action::PowerOff));
    for _ in 0..20 {
        harness.render();
    }
    assert!(asked(&scratch).is_empty(), "systemctl was asked: {}", asked(&scratch));
}
