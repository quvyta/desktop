//! What the screen tests of the floor and the launcher share: a desktop with a few applications
//! on it, built by hand so no test reads the machine it runs on.
//!
//! Each test file compiles this module of its own, and uses the part of it that it needs.

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use qdesk::app::Desk;
use qdesk::apps::{Catalog, Declared, Entry, Environment, Folders, Launch, Source, load, parse_entry};
use qdesk::desktop::Desktop;
use qframe::env::{AssetDirs, Env};
use qframe::icons::GlyphMode;
use qframe::prelude::*;

/// Saturday 19 September 2026, 14:32 three hours east of UTC.
pub const MOMENT: i64 = 1_789_817_550;
/// The time zone of the tests, in minutes east of UTC.
pub const OFFSET: i16 = 180;
/// The machine the tests pretend to be on.
pub const MACHINE: &str = "sunucu-1";
/// The home folder the entries are read against.
pub const HOME: &str = "/home/kisi";

/// The programs that are on this pretend machine. The other entries belong to Installable.
const INSTALLED: [&str; 4] = ["qcode", "mc", "vim", "lf"];

/// The program every entry of these tests really runs, and the shell a Terminal window of theirs
/// opens: one that does nothing and ends at once.
///
/// Opening an application starts its program for real, so a screen test must never start the
/// person's own vim or file manager, any more than it may write in their folders. A test that
/// wants a program of its own writes one (`/bin/sh -c ...`); a live one is driven under
/// [`PATIENCE`].
pub const HARMLESS: &str = "/bin/true";

/// The longest one wait for a program's next word lasts in these tests.
///
/// The desktop itself waits with no bound on a thread of its own and keeps doing so. A screen test
/// runs that wait where it stands, so it asks for a bound instead: a window holding a live program
/// that has said its piece and is waiting to be typed into then lets the test go on drawing, and
/// the watch starts again at the next frame. It is a bound, not a pause — a program that speaks is
/// heard at once.
///
/// It is deliberately short. A wait that ends with nothing to report costs a test nothing but
/// another turn round its loop, so a long one buys no correctness and only makes every quiet
/// frame expensive: at a tenth of a second a loop of a few hundred frames spends most of a minute
/// waiting on a program that had already said everything it was going to say. How long a test is
/// willing to wait altogether is [`BUDGET`]'s business, not this one's.
pub const PATIENCE: Duration = Duration::from_millis(20);

/// How long a test waits altogether for something it is expecting to appear on screen.
///
/// The loops that wait are bounded by this, not by a number of frames. A frame costs whatever the
/// machine and the programs in the windows make it cost, so a frame count is not a length of time:
/// the same four hundred frames are a tenth of a second on a quiet machine and a minute on a busy
/// one, which is how a loaded machine failed these tests with nothing broken.
///
/// This is a bound, not a race. What is waited for normally arrives in a fraction of a second, so
/// the margin is enormous; a test can be slow here but it cannot be wrong, and a desktop that
/// never does the thing still fails instead of hanging.
pub const BUDGET: Duration = Duration::from_secs(60);

/// The icons a test desktop starts with.
pub const ICONS: [&str; 3] = ["terminal", "settings", "mc"];

fn env() -> Env {
    let dirs = AssetDirs {
        locale_sources: qdesk::locales().iter().map(|(file, text)| ((*file).to_owned(), (*text).to_owned())).collect(),
        keymap_source: Some({
            let (file, text) = qdesk::keymap();
            (file.to_owned(), text.to_owned())
        }),
        ..AssetDirs::default()
    };
    // The machine's language and region stay out: a test that starts from the machine's `LANG`
    // would see the week begin on Monday on a Turkish machine and on Sunday elsewhere. The
    // terminal is the one qdesk gives its own windows, whatever the test was started in.
    let terminal = |name: &str| match name {
        "TERM" => Some("xterm-256color".to_owned()),
        "COLORTERM" => Some("truecolor".to_owned()),
        _ => None,
    };
    Env::load_with(&dirs, terminal).expect("the built-in files load")
}

/// One entry written for the tests.
fn extra(id: &str, text: &str) -> Entry {
    let file = format!("{HOME}/.local/share/quvyta/desktop/apps/{id}.toml");
    let (declared, diagnostics) =
        parse_entry(id, Path::new(&file), text.as_bytes(), Source::User, Some(Path::new(HOME)));
    assert!(diagnostics.is_empty(), "{id}: {diagnostics:?}");
    match declared {
        Some(Declared::Entry(entry)) => *entry,
        other => panic!("{id} declares no entry: {other:?}"),
    }
}

/// The applications of the pretend machine: the ones built into qdesk, a file manager with a long
/// name, and a package qpac would install.
#[must_use]
pub fn catalog() -> Catalog {
    let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
    let mut entries = load(&folders, Some(Path::new(HOME))).entries;
    entries.push(extra(
        "mc",
        &format!("name = \"Midnight Commander\"\ncommand = [\"{HARMLESS}\"]\ncategory = \"files\"\n"),
    ));
    entries.push(extra("vim", &format!("name = \"Vim\"\ncommand = [\"{HARMLESS}\"]\ncategory = \"development\"\n")));
    entries.push(extra("lf", &format!("name = \"lf\"\ncommand = [\"{HARMLESS}\"]\ncategory = \"files\"\n")));
    entries.push(extra(
        "btop",
        "name = \"btop\"\ncomment = \"Processes and load\"\ncommand = [\"btop\"]\ncategory = \"system\"\n\n[install]\nqpac = \"btop\"\n",
    ));
    Catalog::new(entries, |entry| {
        // A screen of qdesk is always there; a command needs its program.
        matches!(entry.launch, Launch::Screen(_)) || INSTALLED.contains(&entry.id.as_str())
    })
}

/// A desktop with `icons` on its floor, the welcome line already seen.
#[must_use]
pub fn desk_with(icons: &[&str], width: u16, height: u16) -> Harness<Desk> {
    let desktop = Desktop {
        icons: icons.iter().map(|id| (*id).to_owned()).collect(),
        recents: Vec::new(),
        welcome_seen: true,
        resize_hint_seen: true,
        ..Desktop::default()
    };
    harness(desktop, width, height)
}

/// The usual test desktop: Terminal, Settings and Midnight Commander on the floor.
#[must_use]
pub fn desk(width: u16, height: u16) -> Harness<Desk> {
    desk_with(&ICONS, width, height)
}

/// A desktop nobody has touched yet: the welcome line is still there.
#[must_use]
pub fn untouched(width: u16, height: u16) -> Harness<Desk> {
    harness(Desktop { icons: ICONS.iter().map(|id| (*id).to_owned()).collect(), ..Desktop::default() }, width, height)
}

fn harness(desktop: Desktop, width: u16, height: u16) -> Harness<Desk> {
    harness_with(catalog(), desktop, width, height)
}

/// The same desktop with a catalog of its own, for a test that needs entries the others do not.
#[must_use]
pub fn harness_with(catalog: Catalog, desktop: Desktop, width: u16, height: u16) -> Harness<Desk> {
    built(catalog, desktop, width, height, false)
}

/// The usual test desktop, drawn on a terminal reached over a network.
///
/// The running desktop takes the framework's answer about the connection once, as it starts
/// ([`Desk::remote`]); a test gives it here instead, because a test has to draw the same wherever
/// it runs. The harness is told too ([`Harness::set_remote`]), so the framework's side of the
/// screen draws as it would over SSH.
#[must_use]
pub fn desk_over_ssh(width: u16, height: u16) -> Harness<Desk> {
    let desktop = Desktop {
        icons: ICONS.iter().map(|id| (*id).to_owned()).collect(),
        recents: Vec::new(),
        welcome_seen: true,
        resize_hint_seen: true,
        ..Desktop::default()
    };
    built(catalog(), desktop, width, height, true)
}

fn built(catalog: Catalog, desktop: Desktop, width: u16, height: u16, remote: bool) -> Harness<Desk> {
    let clock = Box::new(|| MOMENT * 1_000);
    let apps = Environment { shell: Some(PathBuf::from(HARMLESS)), ..Environment::default() };
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), clock)
        .apps(apps)
        .catalog(catalog)
        .desktop(desktop)
        .remote(remote)
        .watch_within(PATIENCE);
    let mut harness = Harness::with_env(app, env(), width, height);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true).set_remote(remote);
    harness
}

/// The rows of the screen.
#[must_use]
pub fn screen(harness: &Harness<Desk>) -> Vec<String> {
    harness.screen().lines().map(str::to_owned).collect()
}

/// The decorations the aesthetic rules forbid in every mode: brackets around things, pipes and box
/// drawing.
#[must_use]
pub fn decoration(screen: &str) -> Option<char> {
    screen.chars().find(|c| matches!(c, '[' | ']' | '|' | '{' | '}') || ('\u{2500}'..='\u{257F}').contains(c))
}

/// The usual test desktop reading and writing its settings in `config`, a folder the test made:
/// what one run chooses in Settings, the next run built over the same folder reads back.
#[must_use]
pub fn desk_in(config: &Path, width: u16, height: u16) -> Harness<Desk> {
    let loaded = qdesk::settings::load_in(config);
    let clock = Box::new(|| MOMENT * 1_000);
    let apps = Environment { shell: Some(PathBuf::from(HARMLESS)), ..Environment::default() };
    let desktop = Desktop {
        icons: ICONS.iter().map(|id| (*id).to_owned()).collect(),
        recents: Vec::new(),
        welcome_seen: true,
        resize_hint_seen: true,
        ..Desktop::default()
    };
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), clock)
        .apps(apps)
        .catalog(catalog())
        .desktop(desktop)
        .settings(loaded.settings, loaded.prefs, loaded.diagnostics)
        .watch_within(PATIENCE);
    let mut harness = Harness::with_env(app, env(), width, height);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);
    harness
}

/// A desktop as `desktop` says, on an 80 by 24 terminal, writing its desktop file to `file` as
/// the running desktop does: what one run changes, the next run built from that file reads back.
#[must_use]
pub fn desk_writing(file: &Path, desktop: Desktop) -> Harness<Desk> {
    let clock = Box::new(|| MOMENT * 1_000);
    let apps = Environment { shell: Some(PathBuf::from(HARMLESS)), ..Environment::default() };
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), clock)
        .apps(apps)
        .catalog(catalog())
        .desktop(desktop)
        .config(Some(file.to_path_buf()))
        .watch_within(PATIENCE);
    let mut harness = Harness::with_env(app, env(), 80, 24);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);
    harness
}

/// The pretend machine's desktop with the usual icons, reading the wall clock from `clock`, not
/// drawn yet: a test adds what it needs before [`draw`] puts it on a terminal.
#[must_use]
pub fn base(clock: qdesk::app::WallClock) -> Desk {
    let apps = Environment { shell: Some(PathBuf::from(HARMLESS)), ..Environment::default() };
    let desktop = Desktop {
        icons: ICONS.iter().map(|id| (*id).to_owned()).collect(),
        recents: Vec::new(),
        welcome_seen: true,
        resize_hint_seen: true,
        ..Desktop::default()
    };
    Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), clock)
        .apps(apps)
        .catalog(catalog())
        .desktop(desktop)
        .watch_within(PATIENCE)
}

/// `app` on a terminal of `width` by `height`, in the language, glyphs and motion every screen
/// test draws in.
#[must_use]
pub fn draw(app: Desk, width: u16, height: u16) -> Harness<Desk> {
    let mut harness = Harness::with_env(app, env(), width, height);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);
    harness
}

/// A desktop with `icons` on its floor in the environment `apps`, reading and writing its settings
/// in `config`, a folder the test made: for a test that needs a home, a data folder or a Desktop
/// folder of its own as well as a settings file.
#[must_use]
pub fn desk_at(config: &Path, apps: Environment, icons: &[&str], width: u16, height: u16) -> Harness<Desk> {
    desk_over(config, apps, icons, (width, height), false)
}

/// [`desk_at`] on a terminal that is `remote` or not: reached over SSH, as
/// [`Env::remote_session`] answers where the desktop really runs. The desktop and the harness are
/// both told, as in [`desk_over_ssh`].
pub fn desk_over(
    config: &Path,
    apps: Environment,
    icons: &[&str],
    (width, height): (u16, u16),
    remote: bool,
) -> Harness<Desk> {
    let loaded = qdesk::settings::load_in(config);
    let clock = Box::new(|| MOMENT * 1_000);
    let desktop = Desktop {
        icons: icons.iter().map(|id| (*id).to_owned()).collect(),
        recents: Vec::new(),
        welcome_seen: true,
        resize_hint_seen: true,
        ..Desktop::default()
    };
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), clock)
        .apps(apps)
        .catalog(catalog())
        .desktop(desktop)
        .settings(loaded.settings, loaded.prefs, loaded.diagnostics)
        .watch_within(PATIENCE)
        .remote(remote);
    let mut harness = Harness::with_env(app, env(), width, height);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true).set_remote(remote);
    harness
}
