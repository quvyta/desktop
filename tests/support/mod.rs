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
    Env::load_with(&dirs, terminal).expect("the built-in files load")
}

/// What a screen test's environment is read from, in place of the process's own: the terminal
/// is the one qdesk gives its own windows, whatever the tests were started in, and the rest of
/// the machine stays out.
///
/// A screen drawn from the shell's `TERM` changes with it: under `TERM=dumb` or a terminal that
/// does not say `COLORTERM` the colours fall to fewer, and a test of a tone fails on one machine
/// and passes on another with nothing broken. The machine's language and region stay out too: a
/// test that starts from the machine's `LANG` would see the week begin on Monday on a Turkish
/// machine and on Sunday elsewhere. Pass it to [`Env::load_with`].
#[must_use]
pub fn terminal(name: &str) -> Option<String> {
    match name {
        "TERM" => Some("xterm-256color".to_owned()),
        "COLORTERM" => Some("truecolor".to_owned()),
        _ => None,
    }
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
    Catalog::new(sealed(entries), |entry| {
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
        recommended_seen: true,
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

/// A desktop nobody has touched yet: the welcome line is still there, with the recommended
/// applications the pretend machine does not have.
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
        recommended_seen: true,
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
        recommended_seen: true,
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
        recommended_seen: true,
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
        recommended_seen: true,
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

/// The usual test desktop started the way `qdesk` starts on a machine, with `config` as the Quvyta
/// ecosystem's folder: the settings an older qdesk wrote are settled first, and the runtime starts
/// the desktop as a member of the ecosystem, applying the shared language, theme, icons and
/// reduced motion before the first frame. The folder is not watched; after writing a file, as
/// another application would, [`Harness::poll_preferences`] reads it the way the runtime does.
///
/// The runtime is the one the `qdesk` program builds ([`qdesk::app::runtime`]), with the desktop's
/// own words and keys, so the screen is the one a person sees; only the folder differs.
#[must_use]
pub fn member_in(config: &Path, width: u16, height: u16) -> Harness<Desk> {
    let loaded = qdesk::settings::start_in(config);
    let stored = loaded.settings.clone();
    let app = base(Box::new(|| MOMENT * 1_000)).settings(loaded.settings, loaded.prefs, loaded.diagnostics);
    let mut harness =
        qdesk::app::runtime(app, &stored).harness_in(config, width, height).expect("the desktop's runtime starts");
    harness.set_glyph_mode(GlyphMode::Unicode).render();
    harness
}

/// Turns the wheel over the only open window until `text` is on screen, the way a person reaches a
/// row of the Settings screen below the window's edge; at most as many turns as the screen has
/// rows. Says whether the text is shown.
pub fn scroll_to(harness: &mut Harness<Desk>, text: &str) -> bool {
    let rows = harness.screen().lines().count();
    for _ in 0..rows {
        if harness.screen().contains(text) {
            return true;
        }
        let window = harness.app().windows().iter().next().map(qdesk::wm::Window::rect).expect("an open window");
        let (x, y) = (window.x + i32::from(window.width / 2), window.y + i32::from(window.height / 2));
        harness.mouse(qframe::event::MouseKind::ScrollDown, x, y);
    }
    harness.screen().contains(text)
}

/// Where `option` is drawn on the row of the Settings screen whose label is `label`: the cell a
/// person clicks to choose it. `None` while that row is not on screen.
///
/// The label stands alone at the start of its row, set off from its control by a gap, so `Floor`
/// does not find the row of `Floor pattern`, nor `Frames` the row of `Frames a second`. The option
/// is looked for only after the label, so a word the screen also shows elsewhere is found on this
/// row and nowhere else.
#[must_use]
pub fn on_row<A: App>(harness: &Harness<A>, label: &str, option: &str) -> Option<(i32, i32)> {
    harness.screen().lines().enumerate().find_map(|(y, line)| {
        let end = line.match_indices(label).map(|(start, _)| start + label.len()).find(|&end| {
            let rest = &line[end..];
            rest.starts_with("  ") || rest.trim().is_empty()
        })?;
        let at = end + line[end..].find(option)?;
        Some((i32::from(qframe::text::width(&line[..at])), i32::try_from(y).ok()?))
    })
}

/// Where `option` is drawn in the list a drop-down opened on row `row`: the nearest line to the
/// field that shows it, below it or, where the list opened upwards for want of room, above it.
#[must_use]
pub fn in_list_near<A: App>(harness: &Harness<A>, row: i32, option: &str) -> Option<(i32, i32)> {
    let screen = harness.screen();
    let lines: Vec<&str> = screen.lines().collect();
    let row = usize::try_from(row).ok()?;
    (1..lines.len()).flat_map(|step| [row.checked_add(step), row.checked_sub(step)]).flatten().find_map(|y| {
        let line = lines.get(y)?;
        let at = line.find(option)?;
        Some((i32::from(qframe::text::width(&line[..at])), i32::try_from(y).ok()?))
    })
}

/// Opens the drop-down on the row labelled `label`, showing `shown`, with a click, and clicks
/// `option` in the list it opens: the way a person changes a drop-down.
///
/// # Panics
///
/// Panics when the row, the field or the option is not on screen.
pub fn pick<A: App>(harness: &mut Harness<A>, label: &str, shown: &str, option: &str) {
    let (x, y) = on_row(harness, label, shown)
        .unwrap_or_else(|| panic!("no `{shown}` on the row `{label}`:\n{}", harness.screen()));
    harness.click(x, y);
    // The list unfolds as it opens; where motion is not reduced it is whole only after that.
    harness.advance(Duration::from_millis(300));
    let (x, y) = in_list_near(harness, y, option)
        .unwrap_or_else(|| panic!("the list under `{label}` has no `{option}`:\n{}", harness.screen()));
    harness.click(x, y);
}

/// Clicks `option` on the row labelled `label`: a segment of a row of segments, or a button.
///
/// # Panics
///
/// Panics when the row or the option is not on screen.
pub fn click_on_row<A: App>(harness: &mut Harness<A>, label: &str, option: &str) {
    let (x, y) = on_row(harness, label, option)
        .unwrap_or_else(|| panic!("no `{option}` on the row `{label}`:\n{}", harness.screen()));
    harness.click(x, y);
}

/// The variables through which a program reaches the person's own session: their display, their
/// session bus and their runtime folder. A program a test starts is started without them.
pub const SESSION: [&str; 4] = ["DISPLAY", "WAYLAND_DISPLAY", "DBUS_SESSION_BUS_ADDRESS", "XDG_RUNTIME_DIR"];

/// The programs that open a page or a file on the person's desktop.
pub const OPENERS: [&str; 4] = ["xdg-open", "gio", "sensible-browser", "www-browser"];

/// A folder of stand-ins for [`OPENERS`], made once per test binary under the system's temporary
/// folder: each writes its name and its words as a line of the file `called` beside it and does
/// nothing else. Put first on a program's `PATH`, they keep a test from opening anything on the
/// person's desktop, and `called` says whether the program tried.
///
/// # Panics
///
/// Panics when the folder cannot be written.
#[must_use]
pub fn openers() -> &'static Path {
    static FOLDER: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    FOLDER.get_or_init(|| {
        use std::os::unix::fs::PermissionsExt;
        let folder = std::env::temp_dir().join(format!("qdesk-test-openers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).expect("the openers' folder");
        let called = folder.join("called");
        for name in OPENERS {
            let script = folder.join(name);
            let text = format!("#!/bin/sh\nprintf '%s %s\\n' {name} \"$*\" >> '{}'\n", called.display());
            std::fs::write(&script, text).expect("a stand-in opener");
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("it runs");
        }
        folder
    })
}

/// What the stand-in openers were called with so far, one call a line.
#[must_use]
pub fn opened() -> String {
    std::fs::read_to_string(openers().join("called")).unwrap_or_default()
}

/// `entries` as a test starts them: without [`SESSION`], with the stand-in [`openers`] first on
/// their `PATH` and a `BROWSER` that does nothing.
///
/// The desktop hands the programs it starts its own environment, and a test's is the person's
/// shell's: a program started from a test would find their display and could open a browser on
/// it. The variables an entry unsets go after its own, so these hold whatever the entry says.
#[must_use]
pub fn sealed(entries: Vec<Entry>) -> Vec<Entry> {
    let path = std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_owned());
    let path = format!("{}:{path}", openers().display());
    entries
        .into_iter()
        .map(|mut entry| {
            entry.env.push(("PATH".to_owned(), path.clone()));
            entry.env.push(("BROWSER".to_owned(), "true".to_owned()));
            entry.unset.extend(SESSION.iter().map(|name| (*name).to_owned()));
            entry
        })
        .collect()
}
