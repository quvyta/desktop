//! The desktop application: the floor with its icons, the launcher, the dock, and the runtime
//! that drives them.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use qframe::date::{DateTime, local_offset};
use qframe::env::{AssetDirs, Env};
use qframe::keymap::Scope;
use qframe::prelude::*;
use qframe::runtime::{ClipboardEvent, Confirm, FrameLimit, Task};
use qframe::storage::{FolderChanges, FolderWatch, Settings, machine_name};
use qframe::widgets::{ContextItem, ContextMenu, EmptyState, HelpLayer, KeyHints, Panel, Toast, Tooltip};

use crate::apps::{Catalog, Diagnostic, Entry, Environment, Launch, Screen};
use crate::desktop::{self, Desktop, Floor, IconCell, grid};
use crate::dock;
use crate::inbox::{Inbox, Notice};
use crate::launcher::{self, Launcher, Shelf, Way};
use crate::notice;
use crate::session::{self, Change, Sessions, Start, Subtitle};
use crate::settings::{self, DockPosition, DragStyle, FRAME_CAP_LEAST, FRAME_CAP_MOST, Prefs};
use crate::wm::{self, Exit, Grip, TooSmall, Window, WindowId, Windows, layout};

/// Below this many columns the desktop cannot be drawn and the screen says so.
pub const MIN_WIDTH: u16 = 40;
/// Below this many rows the desktop cannot be drawn and the screen says so.
pub const MIN_HEIGHT: u16 = 10;
/// Below this width or height the icons are hidden: the design's narrow screen (3.7).
pub const NARROW_WIDTH: u16 = 60;
/// Below this height the icons are hidden.
pub const NARROW_HEIGHT: u16 = 16;

/// The name of the floor, so the keys go back to it when a surface above it closes.
pub const FLOOR: &str = "floor";

/// How far an arrow key with shift moves or sizes a window in desktop mode (design 3.3).
pub const FAR_STEP: u16 = 5;

/// Reads the wall clock, in milliseconds since 1970-01-01 00:00 UTC.
pub type WallClock = Box<dyn Fn() -> i64>;

/// Opens the desktop on this terminal and runs it until the person quits.
///
/// # Errors
///
/// Returns the terminal's error when the screen cannot be taken over or restored.
pub fn run() -> io::Result<()> {
    let environment = Environment::from_process();
    let path = Desktop::path();
    let (desktop, mut notices) = match &path {
        Some(path) => Desktop::load(path),
        None => (Desktop::default(), Vec::new()),
    };
    let loaded = environment.load();
    notices.extend(loaded.diagnostics);
    let catalog = Catalog::with_environment(loaded.entries, &environment);
    let chosen = settings::load();
    let app = Desk::new(machine_name(), local_offset(), Box::new(wall_now))
        .apps(environment)
        .catalog(catalog)
        .desktop(desktop)
        .config(path)
        .settings(chosen.settings, chosen.prefs, chosen.diagnostics)
        .remote(remote_link())
        .notices(notices);
    let mut runtime = Runtime::new(app);
    for &(file, text) in crate::locales() {
        runtime = runtime.locale_source(file, text);
    }
    let (file, text) = crate::keymap();
    runtime.keymap_source(file, text).run()
}

/// Whether the terminal the desktop is drawn on is reached over a network.
///
/// The answer is the framework's own, [`Env::remote`], so every Quvyta application reads the
/// connection the same way and the desktop keeps no rule of its own about SSH. The environment
/// the runtime draws with is built inside [`Runtime::run`], after the application has been made,
/// and an application is never handed it outside a frame; the two settings that follow the
/// connection are decided between frames, so the answer is asked for here. Only the built-in
/// files are loaded: nothing of the person's own is read twice.
fn remote_link() -> bool {
    Env::load(&AssetDirs::default()).is_ok_and(|env| env.remote())
}

/// The system clock in milliseconds since 1970-01-01 00:00 UTC; negative before it.
fn wall_now() -> i64 {
    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_millis()).unwrap_or(i64::MAX),
        Err(before) => i64::try_from(before.duration().as_millis()).unwrap_or(i64::MAX).saturating_neg(),
    }
}

/// Everything the desktop needs to know about one application to act on it, taken from the entry
/// while the language of the screen is at hand: `update` never has to look a name up again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    /// The entry's id.
    pub id: String,
    /// Its name, in the language on screen.
    pub name: String,
    /// The command it would run, when it runs one.
    pub command: Option<String>,
    /// How it is installed, when it is not installed.
    pub way: Option<Way>,
    /// The file the entry was read from, for its properties.
    pub file: Option<PathBuf>,
}

impl Target {
    /// The target of `entry`, named in `language`.
    #[must_use]
    pub fn of(entry: &Entry, language: &str) -> Self {
        Self {
            id: entry.id.clone(),
            name: entry.name.get(language).to_owned(),
            command: match &entry.launch {
                Launch::Command(words) => Some(words.join(" ")),
                Launch::Open(path) => Some(path.display().to_string()),
                Launch::Screen(Screen::Terminal) => Some(t!("target.shell")),
                Launch::Screen(Screen::Settings) => None,
            },
            way: Way::of(entry),
            file: entry.file.clone(),
        }
    }
}

/// What reaches the desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// The wall clock reached a new minute.
    Minute,
    /// A watched entry folder changed, on the watch of this run; `true` when there was something
    /// in the batch, `false` when the watch was dropped.
    Changed(u64, bool),
    /// Something happened on the floor.
    Floor(desktop::Action),
    /// Open the launcher.
    OpenLauncher,
    /// Close whatever surface is open above the floor.
    Close,
    /// Something happened in the launcher.
    Launcher(launcher::Msg),
    /// Open an application.
    Open(Target),
    /// Open an application in a window of its own, even one that otherwise opens once.
    OpenNew(Target),
    /// Install an application.
    Install(Target),
    /// Put an icon for an application on the floor.
    AddIcon(Target),
    /// Take an application's icon off the floor.
    RemoveIcon(Target),
    /// Say where an entry comes from and what it runs.
    Properties(Target),
    /// Put the icons of the floor in the order of their names.
    Arrange(Vec<String>),
    /// The welcome line was closed; it never comes back.
    WelcomeSeen,
    /// The terminal is this many columns and rows.
    Resized(Size),
    /// The pointer did something to a window.
    Window(wm::Action),
    /// A window item of the dock was pressed.
    Dock(WindowId),
    /// Something a window's menu asks for.
    Ask(Ask, WindowId),
    /// Show the windows that do not fit on the dock, or put the list away.
    MoreWindows,
    /// Show the recent notifications, or put their list away. Opening it reads them all.
    Notices,
    /// List what the desktop can be told from the keyboard, or put that list away.
    Help,
    /// The keys go to the desktop instead of to a window, or back.
    DesktopMode,
    /// What the arrow keys do in desktop mode.
    Keys(Keys),
    /// An arrow key in desktop mode: the window it picks, moves or resizes, by one cell or by
    /// five with shift.
    Arrow(Arrow, bool),
    /// Lay every window out at once.
    Tile,
    /// Something happened on the Settings screen of a window.
    Settings(settings::Msg),
    /// A window's program said something, or ended.
    Program(session::Report),
    /// A window's program said nothing before the bound of its watch was up, so the watch is
    /// started again. Only a desktop given a bound by
    /// [`Desk::watch_within`](Desk::watch_within) — a screen test — ever hears this.
    Quiet(WindowId),
    /// Start the program of this window again, from the entry it was opened with.
    Restart(WindowId),
    /// Bring this window forward: what a press on a notification of it does.
    Bring(WindowId),
    /// Close this window although its program is still running: the answer to the question.
    CloseAnyway(WindowId),
    /// Leaving qdesk needs an answer first, because programs are still running.
    AskQuit,
    /// Leave qdesk and end the programs that are still running.
    Quit,
    /// The question about leaving was answered no; qdesk keeps running.
    KeepRunning,
    /// Text was pasted where nothing could take it, and the window in front is one whose program
    /// has ended: the desktop says so instead of letting it vanish.
    PastedNowhere,
    /// Something arrived for an application that is no longer there.
    Ignore,
}

/// What a window's menu asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// Take the window down to the dock.
    Minimize,
    /// Fill the desktop with it, or give it its size back.
    Maximize,
    /// Close it.
    Close,
}

/// What the arrow keys do while the desktop has them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Keys {
    /// They pick the window the next keys act on.
    Pick,
    /// They move the picked window.
    Move,
    /// They size the picked window.
    Resize,
}

/// One of the four arrow keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arrow {
    /// Left.
    Left,
    /// Right.
    Right,
    /// Up.
    Up,
    /// Down.
    Down,
}

/// The desktop.
pub struct Desk {
    machine: Option<String>,
    offset_minutes: Option<i16>,
    wall: WallClock,
    apps: Environment,
    catalog: Catalog,
    desktop: Desktop,
    config: Option<PathBuf>,
    notices: Vec<Diagnostic>,
    selection: Vec<String>,
    cursor: Option<usize>,
    launcher: Option<Launcher>,
    watch: Option<FolderWatch>,
    run: u64,
    stored: Settings,
    prefs: Prefs,
    /// Whether the terminal is reached over a network, as the framework's environment says.
    remote: bool,
    screen: settings::Screen,
    windows: Windows,
    programs: Sessions,
    /// The windows whose programs called for attention while they did not have the keys.
    attention: BTreeSet<WindowId>,
    /// Why the program of a window never started, for the window that stayed open to say so.
    failures: BTreeMap<WindowId, Failure>,
    dragging: Option<wm::Dragging>,
    keys: Option<Keys>,
    more: bool,
    /// Everything the corner has said this run, and how much of it is unread. It is never written
    /// anywhere: the design forgets the list when qdesk closes (3.6).
    inbox: Inbox,
    /// Whether the list of notifications is open.
    inbox_open: bool,
    /// Whether the help layer is open.
    help: bool,
    /// Whether the question about leaving qdesk is already on screen, so asking again does not
    /// stack a second one.
    leaving: bool,
    /// How long one wait for a program's next word may last; `None` is the unbounded wait the
    /// running desktop makes on a thread of its own. See [`Desk::watch_within`].
    patience: Option<Duration>,
}

/// Why a window holds no program: the program that was to be started and what the system said.
///
/// The words are kept apart from the sentence they end up in, so a person who changes the language
/// while the window is open reads the reason in the new one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Failure {
    program: String,
    reason: String,
}

impl Desk {
    /// A desktop on the machine called `machine` (`None` when the system gives no name), in a
    /// time zone `offset_minutes` ahead of UTC (`None` when the system does not say), reading
    /// the time from `wall`.
    ///
    /// It starts with no applications and no file of its own; [`apps`](Self::apps),
    /// [`catalog`](Self::catalog), [`desktop`](Self::desktop) and [`config`](Self::config) give it
    /// those. A desktop without a `config` path never writes anything, which is what tests want.
    #[must_use]
    pub fn new(machine: Option<String>, offset_minutes: Option<i16>, wall: WallClock) -> Self {
        Self {
            machine,
            offset_minutes,
            wall,
            apps: Environment::default(),
            catalog: Catalog::default(),
            desktop: Desktop::default(),
            config: None,
            notices: Vec::new(),
            selection: Vec::new(),
            cursor: None,
            launcher: None,
            watch: None,
            run: 0,
            stored: Settings::in_memory(),
            prefs: Prefs::default(),
            remote: false,
            screen: settings::Screen::default(),
            // The terminal says how large it is before the first frame; until then the desktop
            // has no room and no window.
            windows: Windows::new(Size::new(0, 0)),
            programs: Sessions::new(&Environment::default(), Prefs::default(), false),
            attention: BTreeSet::new(),
            failures: BTreeMap::new(),
            dragging: None,
            keys: None,
            more: false,
            inbox: Inbox::default(),
            inbox_open: false,
            help: false,
            leaving: false,
            patience: None,
        }
    }

    /// Bounds every wait for a program's next word by `bound`, for a screen test.
    ///
    /// The running desktop waits with no bound on a thread of its own, which is what a desktop
    /// should do and what it keeps doing. A screen test runs that wait where it stands, so a
    /// window holding a live program with nothing to say would never let the test go; with a
    /// bound the wait comes back saying nothing was said, and the watch starts again at the next
    /// frame. What such a test waits for is bounded by frames, not by the clock. The rule is
    /// [`session::Watch::next_within`].
    #[must_use]
    pub fn watch_within(mut self, bound: Duration) -> Self {
        self.patience = Some(bound);
        self
    }

    /// The settings file, what it says and what it could not be read as.
    #[must_use]
    pub fn settings(mut self, stored: Settings, prefs: Prefs, problems: Vec<Diagnostic>) -> Self {
        self.stored = stored;
        self.prefs = prefs;
        self.screen = settings::Screen::new(problems);
        self.programs.set_prefs(self.prefs, self.remote);
        self
    }

    /// Whether the terminal the desktop is drawn on is reached over a network, which is what
    /// [`Env::remote`] answers. It decides how often a program's output may ask for a frame; the
    /// drag style no longer asks it anything.
    #[must_use]
    pub fn remote(mut self, remote: bool) -> Self {
        self.remote = remote;
        self.programs.set_prefs(self.prefs, self.remote);
        self
    }

    /// The environment the entries and the programs are looked for in.
    #[must_use]
    pub fn apps(mut self, apps: Environment) -> Self {
        self.apps = apps;
        // The programs need the shell and the home folder of this environment, and keep whatever
        // settings have been given so far, in whichever order the two were set.
        self.programs = Sessions::new(&self.apps, self.prefs, self.remote);
        self
    }

    /// The applications the desktop and the launcher show.
    #[must_use]
    pub fn catalog(mut self, catalog: Catalog) -> Self {
        self.catalog = catalog;
        // An icon or a recent whose entry is gone opens nothing; it leaves the desktop quietly and
        // is written out the next time the file is saved.
        self.desktop.keep_known(|id| self.catalog.get(id).is_some());
        self
    }

    /// The order of the icons, the recents and whether the welcome line has been seen.
    #[must_use]
    pub fn desktop(mut self, desktop: Desktop) -> Self {
        self.desktop = desktop;
        self.desktop.keep_known(|id| self.catalog.get(id).is_some());
        self
    }

    /// Where the desktop file is written. Without one nothing is ever written.
    #[must_use]
    pub fn config(mut self, config: Option<PathBuf>) -> Self {
        self.config = config;
        self
    }

    /// Problems found while reading the entries and the desktop file, to be told as notifications
    /// once the screen is there.
    #[must_use]
    pub fn notices(mut self, notices: Vec<Diagnostic>) -> Self {
        self.notices = notices;
        self
    }

    /// The applications standing on the floor, in their order.
    ///
    /// An entry whose program is not on this machine is not among them: an icon that opens nothing
    /// is worse than no icon, and the launcher's Installable section is where it belongs (4.2).
    #[must_use]
    pub fn icons(&self) -> Vec<&Entry> {
        self.desktop
            .icons
            .iter()
            .filter(|id| self.catalog.is_installed(id))
            .filter_map(|id| self.catalog.get(id))
            .collect()
    }

    /// Whether the welcome line is on screen.
    #[must_use]
    pub fn welcome(&self) -> bool {
        !self.desktop.welcome_seen
    }

    /// What lasts of this desktop: the order of the icons, the recents, the welcome line.
    #[must_use]
    pub fn order(&self) -> &Desktop {
        &self.desktop
    }

    /// The launcher, while it is open.
    #[must_use]
    pub fn launcher(&self) -> Option<&Launcher> {
        self.launcher.as_ref()
    }

    /// The windows of the desktop.
    #[must_use]
    pub fn windows(&self) -> &Windows {
        &self.windows
    }

    /// What the arrow keys do, while the desktop and not a window has them.
    #[must_use]
    pub fn keys(&self) -> Option<Keys> {
        self.keys
    }

    /// What is set about the desktop itself.
    #[must_use]
    pub fn prefs(&self) -> Prefs {
        self.prefs
    }

    /// The ids of the selected icons.
    #[must_use]
    pub fn selection(&self) -> &[String] {
        &self.selection
    }

    /// The clock as the dock shows it: hours and minutes of local time, or of UTC marked as such
    /// when the local time zone is unknown, so it is never taken for local time.
    #[must_use]
    pub fn clock_text(&self) -> String {
        let seconds = (self.wall)().div_euclid(1_000);
        let time = DateTime::from_unix(seconds, self.offset_minutes.unwrap_or(0)).time;
        let shown = format!("{:02}:{:02}", time.hour, time.minute);
        match self.offset_minutes {
            Some(_) => shown,
            None => t!("dock.clock-utc", time = shown.as_str()),
        }
    }

    /// The machine name as the dock shows it.
    fn machine_text(&self) -> String {
        self.machine.clone().unwrap_or_else(|| t!("dock.unknown-machine"))
    }

    /// Waits for the next minute of the wall clock. The wait is measured from the clock every
    /// time instead of repeating sixty seconds, so it never drifts, and the dock is drawn once a
    /// minute, not every second: over SSH every frame costs bytes.
    fn next_minute(&self) -> Command<Msg> {
        let into_minute = u64::try_from((self.wall)().rem_euclid(60_000)).unwrap_or(0);
        let until = Duration::from_millis(60_000 - into_minute);
        // A cancelled wait only happens when the program ends; its message is then ignored.
        Command::task(Task::new(
            "clock",
            move |cx| {
                if cx.sleep(until) { Ok(Msg::Minute) } else { Err("stopped".to_owned()) }
            },
        ))
    }

    /// Starts watching the entry folders, so an application installed while the desktop is open
    /// appears by itself.
    fn watch(&mut self) -> Command<Msg> {
        let folders = self.apps.folders().watched();
        if folders.is_empty() {
            // Nothing to watch: an environment that names no entry folder at all. Waiting on a
            // watch with no folder would wait for ever.
            return Command::none();
        }
        let Ok(mut watch) = FolderWatch::new() else {
            // Without a watch the desktop still works; it just does not see a new application
            // until it is opened again. That is worth no interruption.
            return Command::none();
        };
        for folder in &folders {
            // A folder that is not there yet cannot be watched; a program installed later brings
            // it, and the next start sees it.
            let _ = watch.watch(folder);
        }
        let changes = watch.changes();
        self.watch = Some(watch);
        self.run += 1;
        wait(changes, self.run)
    }

    /// The notification one problem of the loaders becomes: the corner says it and the list keeps
    /// it, because a person who was reading something else still has to be able to find out why an
    /// entry of theirs is missing.
    fn told(&mut self, diagnostic: &Diagnostic) -> Command<Msg> {
        let heading =
            if diagnostic.is_warning() { t!("diagnostic.title-warning") } else { t!("diagnostic.title-error") };
        self.inbox.add(Notice::desktop(heading, notice::sentence(diagnostic)));
        notice::toast(diagnostic)
    }

    /// Reads the entries again and keeps watching.
    fn reload(&mut self) -> Command<Msg> {
        let loaded = self.apps.load();
        let notices = loaded.diagnostics;
        self.catalog = Catalog::with_environment(loaded.entries, &self.apps);
        self.desktop.keep_known(|id| self.catalog.get(id).is_some());
        let again = match &self.watch {
            Some(watch) => wait(watch.changes(), self.run),
            None => Command::none(),
        };
        let said: Vec<Command<Msg>> = notices.iter().map(|problem| self.told(problem)).collect();
        Command::batch(said.into_iter().chain([again]))
    }

    /// Writes the desktop file, telling the person when it cannot be written: a desktop that
    /// silently forgets where its icons were is worse than one that says why. The file is a few
    /// hundred bytes and is written whole, so it does not need a task of its own.
    fn save(&mut self) -> Command<Msg> {
        let Some(path) = &self.config else { return Command::none() };
        let Err(error) = self.desktop.save(path) else { return Command::none() };
        let heading = t!("notice.unsaved");
        let body = format!("{}: {error}", path.display());
        self.inbox.add(Notice::desktop(heading.clone(), body.clone()));
        Command::toast(Toast::warning(heading).body(body))
    }

    /// What the floor's actions mean.
    fn on_floor(&mut self, action: desktop::Action) -> Command<Msg> {
        let ids: Vec<String> = self.icons().iter().map(|entry| entry.id.clone()).collect();
        // Any touch of the floor puts the launcher away, as a press outside it does.
        let closing = if self.launcher.take().is_some() { Command::focus(FLOOR) } else { Command::none() };
        match action {
            desktop::Action::Select { index, add } => {
                let Some(id) = ids.get(index) else { return closing };
                if add && self.selection.iter().any(|kept| kept == id) {
                    self.selection.retain(|kept| kept != id);
                } else if add {
                    self.selection.push(id.clone());
                } else {
                    self.selection = vec![id.clone()];
                }
                self.cursor = Some(index);
            }
            desktop::Action::Band(indices) => {
                self.selection = indices.iter().filter_map(|index| ids.get(*index).cloned()).collect();
                self.cursor = indices.first().copied();
            }
            desktop::Action::Clear => self.selection.clear(),
            desktop::Action::Cursor(index) => self.cursor = Some(index),
            // Opening is the view's business: it makes the message with the name in it.
            desktop::Action::Open(index) => self.cursor = Some(index),
            desktop::Action::Drop { from, to } => {
                let mut order = self.desktop.icons.clone();
                // The order file may hold ids no entry answers to; the floor counts only the ones
                // it drew, so the move is made on those and the rest keep their places at the end.
                if let (Some(moved_from), Some(moved_to)) = (place(&order, &ids, from), place(&order, &ids, to)) {
                    order = grid::moved(&order, moved_from, moved_to);
                    self.desktop.set_icons(order);
                    // The cursor follows the icon, and it counts the icons the floor draws, not
                    // the places of the file.
                    self.cursor = Some(to);
                    return Command::batch([closing, self.save()]);
                }
            }
        }
        closing
    }

    /// What the launcher's messages mean.
    fn on_launcher(&mut self, message: launcher::Msg) -> Command<Msg> {
        let Some(launcher) = &mut self.launcher else { return Command::none() };
        match message {
            launcher::Msg::Close => {
                self.launcher = None;
                return Command::focus(FLOOR);
            }
            launcher::Msg::Query(query) => {
                launcher.query = query;
                // The first hit is what Enter opens, so the card on it is the one shown as chosen.
                launcher.selected = Some(0);
            }
            launcher::Msg::Shelf(key) => {
                if let Some(shelf) = Shelf::from_key(&key) {
                    launcher.shelf = shelf;
                    launcher.selected = None;
                }
            }
            launcher::Msg::Select(index) | launcher::Msg::Activate(index) => launcher.selected = Some(index),
            // The rest carry what they act on; the view turns them into the messages above.
            launcher::Msg::Submit
            | launcher::Msg::Open(_)
            | launcher::Msg::AddIcon(_)
            | launcher::Msg::RemoveIcon(_)
            | launcher::Msg::Install(_) => {}
        }
        Command::none()
    }

    /// Opens an application: a window with its program running in it, or a screen qdesk draws
    /// itself. With `fresh` a window of its own opens even for an entry that otherwise opens once.
    fn open(&mut self, target: &Target, fresh: bool) -> Command<Msg> {
        self.desktop.remember(&target.id);
        // Opening an application puts the launcher away, as it does on any desktop.
        let closing = if self.launcher.take().is_some() { Command::focus(FLOOR) } else { Command::none() };
        let saved = self.save();
        let Some(entry) = self.catalog.get(&target.id).cloned() else {
            return Command::batch([closing, saved]);
        };
        if matches!(entry.launch, Launch::Open(_)) {
            // A file waits for the viewers; opening a window that could show nothing would be
            // worse than saying so.
            let toast = Toast::info(t!("notice.no-viewer", name = target.name.as_str()));
            return Command::batch([closing, saved, Command::toast(toast)]);
        }
        let opened = self.open_entry(&entry, fresh);
        Command::batch([closing, saved, opened, self.body_focus()])
    }

    /// Opens a window for `entry` and starts what it holds, or brings its window forward when the
    /// entry opens only once and no window of its own was asked for.
    ///
    /// Opening a window gives the keys to it: the person asked for that window, so the desktop
    /// steps out of the way even when they came from desktop mode.
    fn open_entry(&mut self, entry: &Entry, fresh: bool) -> Command<Msg> {
        self.keys = None;
        let open = if entry.single && !fresh { self.windows.of_entry(&entry.id) } else { None };
        let Some(id) = open else {
            let id = self.windows.open(entry);
            return self.start(id, entry);
        };
        if self.windows.get(id).is_some_and(Window::is_minimized) {
            self.windows.restore(id);
        } else {
            self.windows.raise(id);
        }
        Command::none()
    }

    /// Starts the program of `entry` for the window `id`, on a screen the size of that window's
    /// body, and listens to it.
    fn start(&mut self, id: WindowId, entry: &Entry) -> Command<Msg> {
        let Some(body) = self.body_size(id) else { return Command::none() };
        let started = self.programs.start(id, entry, body);
        self.answer(id, started)
    }

    /// Starts the program of the window `id` again, from the entry it was opened with: the Restart
    /// of a window whose program has ended.
    fn restart(&mut self, id: WindowId) -> Command<Msg> {
        let Some(body) = self.body_size(id) else { return Command::none() };
        let Some(started) = self.programs.restart(id, body) else { return Command::none() };
        let answered = self.answer(id, started);
        Command::batch([answered, self.body_focus()])
    }

    /// What came of starting a program: a window that runs one is listened to, one that could not
    /// start says why, and a screen of qdesk has nothing to listen to.
    fn answer(&mut self, id: WindowId, started: Start) -> Command<Msg> {
        match started {
            Start::Running => {
                self.failures.remove(&id);
                self.windows.mark_running(id);
                self.listen(id)
            }
            Start::Screen => Command::none(),
            Start::Failed(error) => self.failed(id, &error),
        }
    }

    /// The size the body of the window `id` is drawn at; `None` when there is no such window.
    fn body_size(&self, id: WindowId) -> Option<Size> {
        self.windows.get(id).map(|window| wm::view::body_size(window.rect()))
    }

    /// A program that could not be started: the window stays open and says why, and the corner
    /// says it once. The system's own words are given a place to belong to, never alone.
    fn failed(&mut self, id: WindowId, error: &io::Error) -> Command<Msg> {
        let program = self.program_word(id);
        let heading = notice::start_failed_title(&program);
        // The list keeps the whole sentence: it names the program, and a row read days later must
        // say what could not be started without the heading above it.
        self.inbox.add(Notice::from(id, notice::start_failed(&program, &error.to_string())));
        let toast = Toast::warning(heading).body(error.to_string());
        self.failures.insert(id, Failure { program, reason: error.to_string() });
        Command::toast(toast)
    }

    /// Waits for the next thing the program of the window `id` says. One wait hears one change, so
    /// another is started after every change but the last.
    fn listen(&self, id: WindowId) -> Command<Msg> {
        let Some(watch) = self.programs.watch(id) else { return Command::none() };
        match self.patience {
            // What the desktop does: one thread, one wait, for as long as it takes.
            None => Command::perform(move || Msg::Program(watch.next())),
            // What a screen test does, so a live program with nothing to say does not hold it.
            Some(bound) => Command::perform(move || match watch.next_within(bound) {
                Some(report) => Msg::Program(report),
                None => Msg::Quiet(id),
            }),
        }
    }

    /// What a window's program said, or what became of it.
    fn on_program(&mut self, report: session::Report) -> Command<Msg> {
        let id = report.window();
        // A report of a program that has been replaced, or of a window that is gone, changes
        // nothing and is not waited on again.
        let Some(change) = self.programs.accept(report) else { return Command::none() };
        let ended = matches!(change, Change::Ended { .. });
        let told = match change {
            // The screen changed; the frame after this update draws it.
            Change::Output | Change::Folder(_) => Command::none(),
            Change::Title(title) => {
                self.windows.set_title(id, title);
                Command::none()
            }
            Change::Bell => {
                self.called(id);
                // A bell carries no words, so the list says what happened and which window it was.
                // It stays out of the corner: a shell rings for every completion it cannot finish,
                // and a toast for each of those would be noise, while the mark on the dock and a
                // line in the list are exactly what a bell is worth.
                self.inbox.add(Notice::from(id, t!("notice.bell")));
                Command::none()
            }
            Change::Notify { title, body } => self.notify(id, title, body),
            Change::Ended { code } => self.ended(id, code),
        };
        if ended { told } else { Command::batch([told, self.listen(id)]) }
    }

    /// A program called for attention: its window's item on the dock carries a mark until the
    /// window has the keys again. A window that already has them needs none.
    fn called(&mut self, id: WindowId) {
        if self.windows.focus() != Some(id) {
            self.attention.insert(id);
        }
    }

    /// A notification a program asked for: the mark on its item, a word in the corner, and a press
    /// on it brings the window forward (design 3.6).
    fn notify(&mut self, id: WindowId, title: Option<String>, body: String) -> Command<Msg> {
        self.called(id);
        // The heading is the program's own: the title it sent, else what its window's strip says.
        let heading = title.filter(|title| !title.trim().is_empty()).or_else(|| self.said(id));
        // In the list the window's own name leads the row, so the title the program sent goes
        // with the words it belongs to.
        let written = match &heading {
            Some(heading) => t!("notice.list-said", title = heading.as_str(), body = body.as_str()),
            None => body.clone(),
        };
        self.inbox.add(Notice::from(id, written));
        let toast = match heading {
            Some(heading) => Toast::info(heading).body(body),
            None => Toast::info(body),
        };
        Command::toast(toast.on_press(Msg::Bring(id)))
    }

    /// The program of a window ended: the window keeps its last screen and says how it ended, or
    /// closes when its entry asked to. A window the person was not looking at says it in the
    /// corner as well.
    fn ended(&mut self, id: WindowId, code: Option<i32>) -> Command<Msg> {
        let elsewhere = self.windows.focus() != Some(id);
        let said = self.said(id);
        match self.windows.mark_ended(id, code) {
            // The entry asked for the window to go with its program, so nothing is said about a
            // window that is no longer there.
            Exit::Closed => {
                self.programs.close(id);
                self.forget(id);
                if self.windows.is_empty() {
                    self.more = false;
                    self.keys = None;
                }
                self.body_focus()
            }
            Exit::Kept if elsewhere => {
                let line = match code {
                    Some(code) => t!("window.ended", code = code),
                    None => t!("window.ended-signal"),
                };
                self.inbox.add(Notice::from(id, line.clone()));
                let toast = match said {
                    Some(said) => Toast::info(said).body(line),
                    None => Toast::info(line),
                };
                Command::toast(toast.on_press(Msg::Bring(id)))
            }
            // The keys were in the program's own screen, and that screen only shows now; they are
            // put where a window without a program keeps them, so the next key is the desktop's and
            // the two ways on are a Tab away.
            Exit::Kept => self.body_focus(),
            Exit::Unknown => Command::none(),
        }
    }

    /// Brings the window `id` forward, from the dock or from a press on one of its notifications.
    fn bring(&mut self, id: WindowId) -> Command<Msg> {
        // The list has been acted on, so it steps out of the way of the window it just raised.
        self.inbox_open = false;
        if self.windows.get(id).is_some_and(Window::is_minimized) {
            self.windows.restore(id);
        } else if !self.windows.raise(id) {
            return Command::none();
        }
        self.body_focus()
    }

    /// What the strip of the window `id` shows after the name: what its program says about itself
    /// (its own title, else the folder it is in), else the title of a window without a program.
    fn said(&self, id: WindowId) -> Option<String> {
        match self.programs.subtitle(id) {
            Some(Subtitle::Title(title)) => Some(title.to_owned()),
            Some(Subtitle::Folder(folder)) => Some(wm::view::folder_text(folder, self.apps.home.as_deref())),
            None => self.windows.get(id).and_then(Window::title).map(str::to_owned),
        }
    }

    /// The program of the window `id` as the questions and the notices name it: the first word of
    /// its command, or the shell a Terminal window runs, without the folders it lives in.
    fn program_word(&self, id: WindowId) -> String {
        let shell = self.apps.terminal_shell();
        let Some(window) = self.windows.get(id) else { return String::new() };
        let program: &Path = match window.launch() {
            Launch::Command(words) => match words.first() {
                Some(word) => Path::new(word),
                None => return String::new(),
            },
            Launch::Screen(Screen::Terminal) => shell.as_path(),
            Launch::Screen(Screen::Settings) | Launch::Open(_) => return String::new(),
        };
        program.file_name().map_or_else(|| program.display().to_string(), |name| name.to_string_lossy().into_owned())
    }

    /// Leaving qdesk while programs are running: the question counts them (design 3.3). Asking
    /// again while it is on screen keeps the one question, instead of stacking a second.
    fn ask_quit(&mut self) -> Command<Msg> {
        if self.leaving {
            return Command::none();
        }
        self.leaving = true;
        let running = self.programs.running();
        Command::confirm(
            Confirm::new(t!("window.quit-title"), Msg::Quit)
                .message(t!("window.quit-running", n = running))
                .confirm_label(t!("window.quit"))
                .danger()
                .on_cancel(Msg::KeepRunning),
        )
    }

    /// Forgets what the desktop kept about a window that is gone.
    fn forget(&mut self, id: WindowId) {
        self.attention.remove(&id);
        self.failures.remove(&id);
        // What its program said is still worth reading; what is gone is the window to go back to.
        self.inbox.closed(id);
    }

    /// Where the keys go: to the program of the focused window, to what its screen reads, or to the
    /// floor when no window holds anything that reads them, and always to the floor while the
    /// desktop has the keys.
    fn body_focus(&self) -> Command<Msg> {
        if self.keys.is_some() {
            return Command::focus(FLOOR);
        }
        let Some(window) = self.windows.focused() else { return Command::focus(FLOOR) };
        // A running program takes every key; the screen a program that has ended left behind only
        // shows, so it is not a place the keys can land at all, and Esc, the desktop's own keys and
        // the two ways on under its line are what a key reaches instead.
        if self.programs.is_running(window.id()) {
            return Command::focus(wm::view::body_id(window.id()));
        }
        match window.screen() {
            Some(Screen::Settings) => Command::focus(settings::LIST),
            Some(Screen::Terminal) | None => Command::focus(FLOOR),
        }
    }

    /// What the pointer did to a window.
    fn on_window(&mut self, action: wm::Action) -> Command<Msg> {
        match action {
            // The press that raises a window is not taken away from what it landed on: the click
            // goes on to the body, and only the stacking order changes here.
            wm::Action::Focus(id) => {
                self.windows.raise(id);
                Command::none()
            }
            wm::Action::Move { id, dx, dy } => {
                self.drag_move(id, dx, dy);
                Command::none()
            }
            wm::Action::Resize { id, grip, dx, dy } => {
                if self.windows.resize_by(id, grip, dx, dy) {
                    self.dragging = Some(wm::Dragging::Sizing(id));
                }
                Command::none()
            }
            wm::Action::Dropped(id) => {
                self.drop_window(id);
                Command::none()
            }
            wm::Action::Minimize(id) => self.minimize(id),
            wm::Action::ToggleMaximize(id) => {
                self.windows.toggle_maximized(id);
                Command::none()
            }
            // The mark on the title strip is the way a window is closed, so it asks about a
            // running program exactly as the dock's menu and the desktop mode's key do. It used to
            // close outright: every other way in asked, and this one — the obvious one — did not.
            wm::Action::Close(id) => self.ask_close(id),
        }
    }

    /// One step of a drag of the window `id`.
    ///
    /// A ghost drag leaves the window where it is and only the ghost follows the pointer; the
    /// window lands in one frame when the button comes up. That is one changed area a frame
    /// instead of every cell of the window, cheap enough to be the default everywhere, not only
    /// over a remote link.
    fn drag_move(&mut self, id: WindowId, dx: i32, dy: i32) {
        if self.prefs.drag == DragStyle::Live {
            if self.windows.move_by(id, dx, dy) {
                self.dragging = Some(wm::Dragging::Moving(id));
            }
            return;
        }
        let area = self.windows.area();
        let running = match self.dragging {
            Some(wm::Dragging::Ghosting { id: dragged, from, rect }) if dragged == id => Some((from, rect)),
            _ => None,
        };
        let Some((from, rect)) = running.or_else(|| {
            let start = wm::view::ghost_start(self.windows.get(id)?, area);
            Some((start, start))
        }) else {
            return;
        };
        self.dragging = Some(wm::Dragging::Ghosting { id, from, rect: layout::moved(rect, dx, dy, area) });
    }

    /// The end of a drag: a ghost lands, and a window that was moved against an edge snaps to it.
    fn drop_window(&mut self, id: WindowId) {
        let dragged = self.dragging.take();
        match dragged {
            Some(wm::Dragging::Ghosting { id: dragged, from, rect }) if dragged == id => {
                // One movement from where the ghost started puts the window exactly where it
                // stood, whether it was floating, maximized or snapped to an edge.
                self.windows.move_by(id, rect.x - from.x, rect.y - from.y);
                self.windows.drop_dragged(id);
            }
            Some(wm::Dragging::Moving(dragged)) if dragged == id => {
                self.windows.drop_dragged(id);
            }
            // A resize never snaps: an edge pulled to the screen's edge is being sized.
            _ => {}
        }
    }

    /// Takes the window `id` down to the dock.
    fn minimize(&mut self, id: WindowId) -> Command<Msg> {
        if self.windows.minimize(id) { self.body_focus() } else { Command::none() }
    }

    /// What the close mark and `x` mean: a window whose program is still running is asked about
    /// first, with the framework's question and the design's sentence (3.3); one whose program has
    /// ended, or that holds none, closes at once.
    fn ask_close(&mut self, id: WindowId) -> Command<Msg> {
        if !self.programs.is_running(id) {
            return self.close_window(id);
        }
        let name = self.program_word(id);
        Command::confirm(
            Confirm::new(t!("window.close-title", name = name.as_str()), Msg::CloseAnyway(id))
                .message(t!("window.close-running", name = name.as_str()))
                .confirm_label(t!("window.close"))
                .danger(),
        )
    }

    /// Closes the window `id` and ends its program politely: it is sent the hangup a closing
    /// terminal window sends and killed if it has not gone when its grace is over.
    fn close_window(&mut self, id: WindowId) -> Command<Msg> {
        if self.dragging.map(wm::Dragging::id) == Some(id) {
            self.dragging = None;
        }
        if !self.windows.close(id) {
            return Command::none();
        }
        self.programs.close(id);
        self.forget(id);
        if self.windows.is_empty() {
            self.more = false;
            self.keys = None;
        }
        self.body_focus()
    }

    /// Opening the list of recent notifications, or putting it away. Opening it reads them: the
    /// count is what has not been read, and the list is the reading (design 3.6).
    fn on_notices(&mut self) -> Command<Msg> {
        self.inbox_open = !self.inbox_open;
        if !self.inbox_open {
            return self.body_focus();
        }
        self.more = false;
        self.inbox.read();
        // The keys land on the first notice that leads somewhere, so the list is walked and
        // answered without a mouse. A list of notices that lead nowhere takes no focus of its own.
        match self.leading() {
            Some(index) => Command::focus(notice_id(index)),
            None => Command::focus(FLOOR),
        }
    }

    /// The place in the list of the newest notice that has a window to bring forward.
    fn leading(&self) -> Option<usize> {
        self.inbox.notices().position(|notice| notice.window.is_some())
    }

    /// What a press on a window item of the dock means: a minimized window comes back, the
    /// focused one goes down to the dock, and any other one comes forward.
    fn on_dock(&mut self, id: WindowId) -> Command<Msg> {
        let Some(window) = self.windows.get(id) else {
            return Command::none();
        };
        if window.is_minimized() {
            self.windows.restore(id);
            return self.body_focus();
        }
        if self.windows.focus() == Some(id) {
            return self.minimize(id);
        }
        self.windows.raise(id);
        self.body_focus()
    }

    /// Lays every window out at once, or says why it cannot be done.
    fn tile(&mut self) -> Command<Msg> {
        match self.windows.tile() {
            Ok(()) => Command::none(),
            Err(TooSmall::Narrow) => Command::toast(Toast::info(t!("window.tile-narrow"))),
            Err(TooSmall::Short { fits }) => Command::toast(Toast::info(t!("window.tile-short", fits = fits))),
        }
    }

    /// An arrow key while the desktop has the keys: it picks a window, or moves or sizes the
    /// picked one by one cell, or by [`FAR_STEP`] with shift.
    fn on_arrow(&mut self, arrow: Arrow, far: bool) -> Command<Msg> {
        let Some(keys) = self.keys else {
            return Command::none();
        };
        let step = if far { i32::from(FAR_STEP) } else { 1 };
        let (dx, dy) = match arrow {
            Arrow::Left => (-step, 0),
            Arrow::Right => (step, 0),
            Arrow::Up => (0, -step),
            Arrow::Down => (0, step),
        };
        match keys {
            Keys::Pick => {
                let forward = matches!(arrow, Arrow::Right | Arrow::Down);
                if let Some(id) = wm::view::pick(&self.windows, self.windows.focus(), forward) {
                    self.windows.raise(id);
                }
            }
            Keys::Move => {
                if let Some(id) = self.windows.focus() {
                    self.windows.move_by(id, dx, dy);
                }
            }
            Keys::Resize => {
                // The right and the bottom edge are the ones the keys move: they are the two the
                // mouse holds as well, and a window keeps its corner while it is sized.
                if let Some(id) = self.windows.focus() {
                    let grip = if dx == 0 { Grip::Bottom } else { Grip::Right };
                    self.windows.resize_by(id, grip, dx, dy);
                }
            }
        }
        Command::none()
    }

    /// What Esc and the close action mean: they put away the topmost thing that is open, one step
    /// at a time, and the last step leaves desktop mode.
    fn on_close(&mut self) -> Command<Msg> {
        if self.help {
            self.help = false;
            return self.body_focus();
        }
        if self.launcher.take().is_some() {
            return Command::focus(FLOOR);
        }
        if self.more {
            self.more = false;
            return Command::none();
        }
        if self.inbox_open {
            self.inbox_open = false;
            return self.body_focus();
        }
        match self.keys {
            // Moving or sizing a window ends where Enter ends it: back to picking windows.
            Some(Keys::Move | Keys::Resize) => {
                self.keys = Some(Keys::Pick);
                Command::none()
            }
            Some(Keys::Pick) => {
                self.keys = None;
                self.body_focus()
            }
            None => Command::none(),
        }
    }

    /// What the Settings screen of a window asks for: the change is already applied, what is left
    /// is writing it.
    fn on_settings(&mut self, message: settings::Msg) -> Command<Msg> {
        let (applied, request) = settings::update::<Msg>(&mut self.screen, &self.prefs, message);
        let stored = match request {
            Some(settings::Request::Prefs(prefs)) => {
                self.prefs = prefs;
                // A running pseudo-terminal keeps what it was opened with; the windows opened from
                // now on take the new numbers.
                self.programs.set_prefs(prefs, self.remote);
                prefs.write(&mut self.stored);
                self.store()
            }
            Some(settings::Request::Shared(shared)) => {
                self.share(&shared);
                self.store()
            }
            None => Command::none(),
        };
        Command::batch([applied, stored])
    }

    /// Writes the settings on a background thread, so a slow disk never holds up drawing.
    fn store(&self) -> Command<Msg> {
        self.stored.save_command(|result| Msg::Settings(settings::Msg::Stored(result)))
    }

    /// Puts a setting every Quvyta application shares into the settings file.
    fn share(&mut self, shared: &settings::Shared) {
        let _ = match shared {
            settings::Shared::Language(code) => self.stored.set(Settings::LANGUAGE, code.clone()),
            settings::Shared::Theme(id) => self.stored.set(Settings::THEME, id.clone()),
            settings::Shared::Icons(mode) => self.stored.set(Settings::ICONS, mode.name().to_owned()),
        };
    }

    /// Installs an application: qpac and quvyta do the installing, each in a window of its own, and
    /// the corner says which package or member is meant.
    ///
    /// When the installer itself is not on this machine the sentence is the whole of what can be
    /// done for the person: a window onto a program that is not there would show nothing.
    fn install(&mut self, target: &Target) -> Command<Msg> {
        let Some(way) = target.way.clone() else {
            return Command::toast(Toast::warning(t!("notice.install-unknown", name = target.name.as_str())));
        };
        let told = Command::toast(
            Toast::info(t!("notice.install", name = target.name.as_str())).body(way.sentence(&target.name)),
        );
        let installer = match &way {
            Way::Quvyta(_) => "quvyta",
            Way::Qpac(_) => "qpac",
        };
        let Some(entry) = self.catalog.get(installer).filter(|entry| self.catalog.is_installed(&entry.id)).cloned()
        else {
            return told;
        };
        let entry = match &way {
            // quvyta opens the member's own page, which is where its install button is.
            Way::Quvyta(member) => Entry {
                launch: Launch::Command(vec![installer.to_owned(), "show".to_owned(), member.clone()]),
                ..entry
            },
            // qpac is opened as it is: qdesk does not invent a command line for another
            // application.
            Way::Qpac(_) => entry,
        };
        let opened = self.open_entry(&entry, false);
        Command::batch([told, opened, self.body_focus()])
    }

    /// Applies one message. [`App::update`] wraps it, so what every message leaves behind is
    /// settled in one place.
    fn applied(&mut self, msg: Msg) -> Command<Msg> {
        match msg {
            Msg::Minute => self.next_minute(),
            Msg::Changed(run, changed) if run == self.run && changed => self.reload(),
            // A batch of an older run, or the last empty answer of a dropped watch.
            Msg::Changed(..) | Msg::Ignore => Command::none(),
            // It is said in the corner and nowhere else: this is the answer to something the
            // person just did, not an event of the desktop worth keeping in the list.
            Msg::PastedNowhere => Command::toast(Toast::info(t!("notice.paste-nowhere"))),
            Msg::Floor(action) => self.on_floor(action),
            Msg::OpenLauncher => {
                self.launcher = Some(Launcher::default());
                Command::focus(launcher::SEARCH)
            }
            Msg::Close => self.on_close(),
            Msg::Resized(size) => {
                self.windows.resize(size);
                Command::none()
            }
            Msg::Window(action) => self.on_window(action),
            Msg::Dock(id) => self.on_dock(id),
            Msg::Ask(Ask::Minimize, id) => self.minimize(id),
            Msg::Ask(Ask::Maximize, id) => {
                self.windows.toggle_maximized(id);
                Command::none()
            }
            Msg::Ask(Ask::Close, id) => self.ask_close(id),
            Msg::CloseAnyway(id) => self.close_window(id),
            Msg::Program(report) => self.on_program(report),
            Msg::Quiet(id) => self.listen(id),
            Msg::Restart(id) => self.restart(id),
            Msg::Bring(id) => self.bring(id),
            Msg::AskQuit => self.ask_quit(),
            Msg::Quit => Command::quit(),
            Msg::KeepRunning => {
                self.leaving = false;
                Command::none()
            }
            Msg::MoreWindows => {
                self.more = !self.more;
                self.inbox_open = false;
                Command::none()
            }
            Msg::Notices => self.on_notices(),
            Msg::Help => {
                self.help = !self.help;
                if self.help { Command::none() } else { self.body_focus() }
            }
            Msg::DesktopMode => {
                if self.keys.is_some() {
                    self.keys = None;
                    return self.body_focus();
                }
                self.keys = Some(Keys::Pick);
                Command::focus(FLOOR)
            }
            Msg::Keys(keys) => {
                if self.keys.is_some() {
                    self.keys = Some(keys);
                }
                Command::none()
            }
            Msg::Arrow(arrow, far) => self.on_arrow(arrow, far),
            Msg::Tile => self.tile(),
            Msg::Settings(message) => self.on_settings(message),
            Msg::Launcher(message) => self.on_launcher(message),
            Msg::Open(target) => self.open(&target, false),
            Msg::OpenNew(target) => self.open(&target, true),
            Msg::Install(target) => self.install(&target),
            Msg::AddIcon(target) => {
                self.desktop.add_icon(&target.id);
                self.save()
            }
            Msg::RemoveIcon(target) => {
                self.desktop.remove_icon(&target.id);
                self.selection.retain(|id| *id != target.id);
                self.save()
            }
            Msg::Properties(target) => {
                let file = target
                    .file
                    .as_ref()
                    .map(|file| file.display().to_string())
                    .unwrap_or_else(|| t!("properties.built-in"));
                let command = target.command.clone().unwrap_or_else(|| t!("properties.no-command"));
                Command::toast(Toast::info(target.name.clone()).body(t!(
                    "properties.body",
                    id = target.id.as_str(),
                    file = file.as_str(),
                    command = command.as_str()
                )))
            }
            Msg::Arrange(order) => {
                // The ids the floor did not draw keep their places after the arranged ones.
                let mut icons = order.clone();
                icons.extend(self.desktop.icons.iter().filter(|id| !order.contains(id)).cloned());
                self.desktop.set_icons(icons);
                self.cursor = None;
                self.save()
            }
            Msg::WelcomeSeen => {
                self.desktop.welcome_seen = true;
                Command::batch([self.save(), Command::focus(FLOOR)])
            }
        }
    }

    fn too_small(ui: &mut View<'_, Msg>) {
        let hint = t!("screen.too-small-hint", columns = MIN_WIDTH, rows = MIN_HEIGHT);
        ui.add(EmptyState::new(t!("screen.too-small")).message(hint)).fill();
    }

    /// Whether the screen is too narrow for icons (3.7): the windows and the dock take it all.
    fn narrow(size: Size) -> bool {
        size.width < NARROW_WIDTH || size.height < NARROW_HEIGHT
    }

    /// The open windows as the dock shows them, in the order they were opened.
    fn items(&self, ui: &View<'_, Msg>) -> Vec<dock::Item> {
        let language = ui.env().i18n().active();
        let icons = ui.env().icons();
        let mark = icons.glyph("window-minimize").into_owned();
        let called = icons.glyph("dot").into_owned();
        let focus = self.windows.focus();
        let mut windows: Vec<&Window> = self.windows.iter().collect();
        windows.sort_by_key(|window| window.id());
        windows
            .into_iter()
            .map(|window| dock::Item {
                id: window.id(),
                focused: focus == Some(window.id()),
                label: dock::Label {
                    glyph: desktop::glyph_of(window.entry(), icons),
                    name: window.entry().name.get(language).to_owned(),
                    mark: window.is_minimized().then(|| mark.clone()),
                    attention: self.attention.contains(&window.id()).then(|| called.clone()),
                },
            })
            .collect()
    }

    fn desktop_view(&self, ui: &mut View<'_, Msg>) {
        let clock = self.clock_text();
        let items = self.items(ui);
        let labels: Vec<dock::Label> = items.iter().map(|item| item.label.clone()).collect();
        let count = dock::count_label(&ui.env().icons().glyph("dot"), self.inbox.unread());
        // The plan measures the items as the buttons that draw them, so the row it plans is the
        // row that is painted whatever the theme's padding is.
        let padding = ui.env().theme().style("button", None, &[]).pair("padding").map_or(2, |(_, sides)| sides);
        let plan = dock::plan(ui.size().width, &self.machine_text(), &clock, &count, &labels, padding);
        let desktop_keys = self.keys;
        let row = |ui: &mut View<'_, Msg>| match desktop_keys {
            // In desktop mode the dock's row says what the keys do, as the design asks (3.3).
            // It replaces the dock wherever the dock now is, so the row the person reads never
            // moves under them.
            Some(keys) => Self::hints_view(keys, ui),
            None => {
                let presses = dock::Presses {
                    launcher: Msg::OpenLauncher,
                    more: Msg::MoreWindows,
                    notices: Msg::Notices,
                    press: &|item: &dock::Item| Msg::Dock(item.id),
                    menu: &|item: &dock::Item| Self::window_menu(item.id),
                };
                dock::view(&plan, &clock, &count, &items, &presses, ui);
            }
        };
        // The shell's own header and footer are what keep the row out of the body: whichever end
        // the dock takes, the windows get every other row and nothing is drawn behind it.
        let shell = AppShell::new().body(|ui| self.body(&items, &plan, ui));
        match self.prefs.dock {
            DockPosition::Top => shell.header(row),
            DockPosition::Bottom => shell.footer(row),
        }
        .show(ui);
    }

    /// Puts what hangs off the dock — the launcher, the notification list, the list of the windows
    /// that do not fit — against the dock's own row, filling the rest of the desktop with space.
    ///
    /// The design has the launcher rise from above the dock (3.5); a dock on the top row is above
    /// it instead, so the surface comes down from it. Either way it touches the row it belongs to
    /// and grows into the desktop.
    fn against_dock(&self, ui: &mut View<'_, Msg>, build: impl FnOnce(&mut View<'_, Msg>)) {
        let top = self.prefs.dock == DockPosition::Top;
        ui.column(|ui| {
            if !top {
                ui.spacer();
            }
            build(ui);
            if top {
                ui.spacer();
            }
        })
        .fill();
    }

    /// The floor, with the windows over it and the launcher and the welcome line above them.
    fn body(&self, items: &[dock::Item], plan: &dock::Plan, ui: &mut View<'_, Msg>) {
        ui.stack(|ui| {
            self.floor(ui);
            self.windows_view(ui);
            if self.more && plan.hidden > 0 {
                self.more_view(items, plan, ui);
            }
            if self.inbox_open {
                self.notices_view(ui);
            }
            if let Some(launcher) = &self.launcher {
                self.launcher_view(launcher, ui);
            }
            if self.welcome() {
                Self::welcome_view(ui);
            }
            if self.help {
                self.help_view(ui);
            }
        })
        .fill();
    }

    /// Every window of the desktop, from the back forwards, and the ghost over them.
    fn windows_view(&self, ui: &mut View<'_, Msg>) {
        if self.windows.is_empty() {
            return;
        }
        let language = ui.env().i18n().active();
        let icons = ui.env().icons();
        // The names and the glyphs are read before the windows are drawn: drawing borrows the
        // view, and the language and the icon set live in it.
        let written: Vec<(WindowId, String, String)> = self
            .windows
            .visible()
            .map(|window| {
                (window.id(), window.entry().name.get(language).to_owned(), desktop::glyph_of(window.entry(), icons))
            })
            .collect();
        let find = |wanted: WindowId| written.iter().find(|(id, ..)| *id == wanted);
        let look = wm::Look { shadow: true, preview: wm::view::preview(&self.windows, self.dragging) };
        let strip = wm::view::Strip {
            name: &|window| find(window.id()).map_or_else(String::new, |(_, name, _)| name.clone()),
            glyph: &|window| find(window.id()).map_or_else(String::new, |(.., glyph)| glyph.clone()),
            subtitle: &|window| self.said(window.id()),
        };
        wm::view::view(&self.windows, &look, Msg::Window, &strip, &mut |window, ui| self.window_body(window, ui), ui);
    }

    /// What a window holds.
    fn window_body(&self, window: &Window, ui: &mut View<'_, Msg>) {
        match window.body() {
            wm::Body::Screen => {
                if window.screen() == Some(Screen::Settings) {
                    self.settings_body(ui);
                }
            }
            wm::Body::Program(run) => self.program_body(window.id(), run, ui),
        }
    }

    /// The body of a window that holds a program: the program's own screen, and under it the line
    /// of a program that has ended or the reason one never started.
    ///
    /// A window whose program has ended keeps that screen, drawn as something that only shows: the
    /// whole reason such a window stays open is that the last thing the program said is read after
    /// it is gone, and a window that forgot it would be worse than one that closed itself. Faded,
    /// out of the focus order and written to by nobody — not even with the size it is drawn at, so
    /// the window changing shape cannot reflow a screen somebody is reading. What is left of the
    /// terminal is what reading needs: the program's own colours, its wide characters, the cursor
    /// it left behind, selecting its output and scrolling back through it.
    fn program_body(&self, id: WindowId, run: wm::Run, ui: &mut View<'_, Msg>) {
        if let Some(terminal) = self.programs.terminal(id) {
            let ended = matches!(run, wm::Run::Ended { .. });
            let terminal = if ended {
                terminal.read_only()
            } else {
                // Every key is the program's, as the framework's terminal has it, except the one
                // that takes the keys out to the desktop: without it a window of `vim` would be a
                // room with no door.
                terminal.pass_through(Scope::App, "desktop-mode")
            };
            ui.add(terminal).id(wm::view::body_id(id)).fill();
        }
        match run {
            wm::Run::Ended { code } => Self::ended_body(code, id, ui),
            // A window whose program never started says why, and the only way on is to close it:
            // there is nothing to start again.
            wm::Run::Waiting => {
                if let Some(failure) = self.failures.get(&id) {
                    Self::failed_body(failure, id, ui);
                }
            }
            wm::Run::Running => {}
        }
    }

    /// The Settings screen inside its window.
    fn settings_body(&self, ui: &mut View<'_, Msg>) {
        let folders = self.apps.folders();
        let apps = settings::Applications { folders: &folders, diagnostics: &self.notices };
        settings::view(&self.screen, &self.prefs, self.remote, &apps, ui);
    }

    /// The line a window shows when its program has ended: what it ended with, and the two ways on.
    /// It does not close by itself, so an error message is read before it goes.
    fn ended_body(code: Option<i32>, id: WindowId, ui: &mut View<'_, Msg>) {
        let said = match code {
            Some(code) => t!("window.ended", code = code),
            None => t!("window.ended-signal"),
        };
        // The line stands at the foot of the body, where the design puts it under the last screen,
        // so the window keeps its shape and the screen can come back above it unchanged. The
        // screen above keeps every other row: its last lines are where an error message is.
        ui.row(|ui| {
            // The sentence is what yields on a narrow window: the two ways on must not be the
            // things a row drops from its end. It is cut rather than wrapped, so the line stays one
            // row and takes no more of the last screen than that.
            ui.add(Text::new(said).role("secondary").no_wrap()).width(Length::Fill(1));
            ui.add(Button::new(t!("window.restart")).on_press(Msg::Restart(id)));
            ui.add(Button::new(t!("window.close")).on_press(Msg::Ask(Ask::Close, id)));
        })
        .gap(1)
        .fill_width();
    }

    /// The line a window shows when its program never started: which program it was and what the
    /// system said about it, and the way to put the window away.
    fn failed_body(failure: &Failure, id: WindowId, ui: &mut View<'_, Msg>) {
        let said = notice::start_failed(&failure.program, &failure.reason);
        ui.row(|ui| {
            ui.add(Text::new(said).role("secondary")).width(Length::Fill(1));
            ui.add(Button::new(t!("window.close")).on_press(Msg::Ask(Ask::Close, id)));
        })
        .gap(1)
        .fill_width();
        ui.spacer().fill();
    }

    /// The rows of a window's menu, on the dock and in desktop mode.
    fn window_menu(id: WindowId) -> Vec<ContextItem<Msg>> {
        vec![
            ContextItem::new(t!("window.minimize"), Msg::Ask(Ask::Minimize, id)),
            ContextItem::new(t!("window.maximize"), Msg::Ask(Ask::Maximize, id)),
            ContextItem::new(t!("window.tile"), Msg::Tile),
            ContextItem::gap(),
            ContextItem::new(t!("window.close"), Msg::Ask(Ask::Close, id)),
        ]
    }

    /// The windows that do not fit on the dock, listed against its row.
    fn more_view(&self, items: &[dock::Item], plan: &dock::Plan, ui: &mut View<'_, Msg>) {
        self.against_dock(ui, |ui| {
            ui.row(|ui| {
                ui.add_with(Panel::new().title(t!("dock.more")), |ui| {
                    for item in items.iter().skip(plan.shown) {
                        ui.add(Button::new(item.label.text(true)).selected(item.focused).on_press(Msg::Dock(item.id)))
                            .id(format!("more-{}", item.id.number()));
                    }
                });
                ui.spacer();
            });
        });
    }

    /// The recent notifications, listed against the dock's row at the end the count that opens
    /// them stands on. The newest is first; one that came from a window is a way back to it, and
    /// one of qdesk's own is there to be read (design 3.6).
    fn notices_view(&self, ui: &mut View<'_, Msg>) {
        let language = ui.env().i18n().active().to_owned();
        let rows: Vec<(String, Option<WindowId>)> = self
            .inbox
            .notices()
            .map(|notice| {
                // A notice that came from a window is led by that window's name, in the language on
                // screen; one of qdesk's own by the heading it was said with.
                let named = notice
                    .window
                    .and_then(|id| self.windows.get(id))
                    .map(|window| window.entry().name.get(&language).to_owned());
                let line = match named.or_else(|| notice.lead.clone()) {
                    Some(lead) => t!("notice.list-line", heading = lead.as_str(), body = notice.body.as_str()),
                    None => notice.body.clone(),
                };
                (line, notice.window)
            })
            .collect();
        // The surface is as wide as its longest line and no wider, up to the screen: a row cut in
        // the middle would lose the end of a sentence, which is where an exit code stands. The
        // paddings are read from the theme, so the width is the one that is really painted.
        // The fallbacks are the widgets' own, so a theme that sets no padding is measured the way
        // it is drawn: three cells each side for a surface, two for a button.
        let padding = |key: &str, bare: u16| {
            ui.env().theme().style(key, None, &[]).pair("padding").map_or(bare, |(_, sides)| sides)
        };
        let chrome = 2 * padding("panel", 3) + 2 * padding("button", 2);
        let empty = t!("notice.list-empty-hint");
        // A list with nothing in it is as wide as the sentence that says so.
        let longest =
            rows.iter().map(|(line, _)| qframe::text::width(line)).max().unwrap_or_else(|| qframe::text::width(&empty));
        let width = longest.saturating_add(chrome).min(ui.size().width.saturating_sub(2 * dock::EDGE));
        self.against_dock(ui, |ui| {
            ui.row(|ui| {
                ui.add_with(Panel::new().title(t!("notice.list")), |ui| {
                    if rows.is_empty() {
                        ui.add(EmptyState::new(t!("notice.list-empty")).message(empty));
                        return;
                    }
                    for (index, (line, window)) in rows.into_iter().enumerate() {
                        match window {
                            // A press does what a press on its toast did: brings the window forward.
                            Some(id) => {
                                ui.add(Button::new(line).on_press(Msg::Bring(id))).id(notice_id(index));
                            }
                            // Nothing to go back to, so nothing offers to go there.
                            None => {
                                ui.add(Text::new(line));
                            }
                        }
                    }
                })
                .width(Length::Cells(width));
                // The corner the dock's count stands in is where the toasts live; a list opened
                // under them would be read through them. It opens at the end the window list opens
                // at instead, so the two surfaces of the dock behave the same way.
                ui.spacer();
            })
            .fill_width();
        });
    }

    /// What the desktop can be told from the keyboard: the framework's help layer, which reads the
    /// keymap itself, so a rebound key is listed by the key it now has.
    ///
    /// The keys of the floor are not in any keymap — the icons are walked and opened by the grid
    /// itself — so they are given to the layer as the keys of this screen.
    fn help_view(&self, ui: &mut View<'_, Msg>) {
        let icons = ui.env().icons();
        let arrow = |key: &str| icons.glyph(key).into_owned();
        let arrows =
            format!("{}{}{}{}", arrow("arrow-left"), arrow("arrow-up"), arrow("arrow-right"), arrow("arrow-down"));
        ui.add(
            HelpLayer::new(Msg::Help)
                .hint(arrows, t!("help.icons-move"))
                .hint("enter", t!("help.icons-open"))
                .hint("esc", t!("help.icons-clear"))
                .hint(t!("help.icons-jump"), t!("help.icons-jump-label")),
        );
    }

    /// What the keys do while the desktop has them, in the dock's row.
    fn hints_view(keys: Keys, ui: &mut View<'_, Msg>) {
        let icons = ui.env().icons();
        let arrow = |key: &str| icons.glyph(key).into_owned();
        let arrows =
            format!("{}{}{}{}", arrow("arrow-left"), arrow("arrow-up"), arrow("arrow-right"), arrow("arrow-down"));
        let hints = match keys {
            // A narrow row drops hints from its end (the framework's rule), so the order is what
            // matters least last: picking, moving, sizing, closing and the way back are the ones a
            // person cannot guess, and the rest is in the help layer.
            Keys::Pick => KeyHints::new()
                .hint(arrows, t!("mode.pick"))
                .hint("m", t!("mode.move"))
                .hint("r", t!("mode.resize"))
                .hint("x", t!("mode.close"))
                .hint("esc", t!("mode.leave"))
                .hint("z", t!("mode.maximize"))
                .hint("n", t!("mode.minimize"))
                .hint("t", t!("mode.tile"))
                .hint("space", t!("mode.launcher"))
                .hint("b", t!("mode.notices")),
            Keys::Move | Keys::Resize => KeyHints::new()
                .hint(arrows, if keys == Keys::Move { t!("mode.move") } else { t!("mode.resize") })
                .hint("shift", t!("mode.far", cells = FAR_STEP))
                .hint("enter", t!("mode.release"))
                .hint("esc", t!("mode.back")),
        };
        ui.add(hints).fill_width().height(Length::Cells(dock::HEIGHT));
    }

    /// The icons on the floor, each with its own menu, and the menu of the empty floor.
    fn floor(&self, ui: &mut View<'_, Msg>) {
        let language = ui.env().i18n().active().to_owned();
        let entries = self.icons();
        let targets: Vec<Target> = entries.iter().map(|entry| Target::of(entry, &language)).collect();
        let names: Vec<String> = targets.iter().map(|target| target.name.clone()).collect();
        let glyphs: Vec<String> = entries.iter().map(|entry| desktop::glyph_of(entry, ui.env().icons())).collect();
        let selection: Vec<bool> = targets.iter().map(|target| self.selection.contains(&target.id)).collect();
        let cursor = self.cursor;
        // In desktop mode the arrows pick a window, so the floor lets them through.
        let keys = self.launcher.is_none() && self.keys.is_none();
        let opening = targets.clone();
        let hidden = Self::narrow(ui.size());
        let hint = hidden.then(|| Self::narrow_hint(ui));
        ui.add_with(ContextMenu::new(self.floor_items(&language)), |ui| {
            if let Some(line) = hint {
                // A narrow screen keeps its room for the windows; the launcher still reaches
                // every application, and one line says how. The welcome, while it is up,
                // stands over that line and says the same.
                Self::narrow_hint_view(line, ui);
                return;
            }
            let floor = Floor::new(names.clone(), move |action| match action {
                desktop::Action::Open(index) => opening.get(index).cloned().map_or(Msg::Ignore, Msg::Open),
                other => Msg::Floor(other),
            })
            .cursor(cursor)
            .keys(keys);
            ui.add_with(floor, |ui| {
                for (index, target) in targets.iter().enumerate() {
                    let cell = IconCell::new(glyphs[index].clone(), target.name.clone())
                        .selected(selection[index])
                        .cursor(cursor == Some(index));
                    let menu = ContextMenu::new(Self::icon_items(target, &self.desktop));
                    let shortened = grid::shown_name(&target.name) != target.name;
                    if shortened {
                        // The cell shows what fits; the whole name is one hover away.
                        ui.add_with(Tooltip::new(target.name.clone()), |ui| {
                            ui.add_with(menu, |ui| {
                                ui.add(cell);
                            });
                        });
                    } else {
                        ui.add_with(menu, |ui| {
                            ui.add(cell);
                        });
                    }
                }
            })
            .id(FLOOR)
            .fill();
        })
        .fill();
    }

    /// The one line a floor too narrow for icons shows in their place (3.7): where the
    /// applications went. The whole line names the keys too; where it does not fit, the shorter
    /// one names only the dock's button, so nothing is cut half way.
    fn narrow_hint(ui: &View<'_, Msg>) -> String {
        let icons = ui.env().icons();
        let launcher = icons.glyph(desktop::family_icon(icons)).into_owned();
        let whole = t!("floor.narrow-hint", icon = launcher.as_str());
        // A cell of floor on either side keeps the line off the screen's edges.
        if qframe::text::width(&whole) + 2 <= ui.size().width {
            whole
        } else {
            t!("floor.narrow-hint-short", icon = launcher.as_str())
        }
    }

    /// The narrow floor's line, quiet and in the middle of the floor.
    fn narrow_hint_view(line: String, ui: &mut View<'_, Msg>) {
        ui.column(|ui| {
            ui.spacer();
            ui.add(Text::new(line).role("secondary").align(Align::Center).no_wrap()).fill_width();
            ui.spacer();
        })
        .fill();
    }

    /// The rows of an icon's menu.
    fn icon_items(target: &Target, desktop: &Desktop) -> Vec<ContextItem<Msg>> {
        let mut items = vec![
            ContextItem::new(t!("icon.open"), Msg::Open(target.clone())),
            ContextItem::new(t!("icon.open-new"), Msg::OpenNew(target.clone())),
        ];
        if desktop.icons.contains(&target.id) {
            items.push(ContextItem::new(t!("icon.remove"), Msg::RemoveIcon(target.clone())));
        }
        items.push(ContextItem::gap());
        items.push(ContextItem::new(t!("icon.properties"), Msg::Properties(target.clone())));
        items
    }

    /// The rows of the empty floor's menu.
    fn floor_items(&self, language: &str) -> Vec<ContextItem<Msg>> {
        let entry = |id: &str| self.catalog.get(id).map(|entry| Target::of(entry, language));
        let mut items = Vec::new();
        if let Some(terminal) = entry("terminal") {
            items.push(ContextItem::new(t!("floor.new-terminal"), Msg::Open(terminal)));
        }
        items.push(ContextItem::new(t!("floor.add-application"), Msg::OpenLauncher));
        let mut arranged: Vec<(String, String)> =
            self.icons().iter().map(|entry| (entry.name.get(language).to_lowercase(), entry.id.clone())).collect();
        arranged.sort();
        items.push(ContextItem::new(
            t!("floor.arrange"),
            Msg::Arrange(arranged.into_iter().map(|(_, id)| id).collect()),
        ));
        if let Some(settings) = entry("settings") {
            items.push(ContextItem::gap());
            items.push(ContextItem::new(t!("floor.settings"), Msg::Open(settings)));
        }
        items
    }

    /// The launcher, rising from the left above the dock.
    fn launcher_view(&self, launcher: &Launcher, ui: &mut View<'_, Msg>) {
        let language = ui.env().i18n().active().to_owned();
        let apps = launcher::shown(&self.catalog, &self.desktop, launcher, &language, ui.env().icons());
        let shelves = launcher::shelves(&self.catalog, &self.desktop, &language);
        let targets: Vec<Target> =
            apps.iter().filter_map(|app| self.catalog.get(&app.id)).map(|entry| Target::of(entry, &language)).collect();
        self.against_dock(ui, |ui| {
            ui.row(|ui| {
                ui.map(
                    move |message| match message {
                        launcher::Msg::Submit => targets.first().cloned().map_or(Msg::Ignore, Msg::Open),
                        launcher::Msg::Open(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::Open),
                        launcher::Msg::Install(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::Install),
                        launcher::Msg::AddIcon(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::AddIcon),
                        launcher::Msg::RemoveIcon(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::RemoveIcon),
                        other => Msg::Launcher(other),
                    },
                    |ui| launcher::view(launcher, &apps, &shelves, ui),
                );
                ui.spacer();
            });
        });
    }

    /// The welcome line, shown once and never again once it is closed.
    fn welcome_view(ui: &mut View<'_, Msg>) {
        let launcher = ui.env().icons().glyph(desktop::family_icon(ui.env().icons())).into_owned();
        ui.column(|ui| {
            ui.spacer();
            ui.row(|ui| {
                ui.spacer();
                ui.add_with(Panel::new(), |ui| {
                    ui.add(Text::new(t!("welcome.line", icon = launcher.as_str())));
                    ui.add(Button::new(t!("welcome.close")).on_press(Msg::WelcomeSeen));
                });
                ui.spacer();
            });
            ui.spacer();
        })
        .fill();
    }
}

impl From<settings::Msg> for Msg {
    fn from(message: settings::Msg) -> Self {
        Self::Settings(message)
    }
}

/// Waits for the next batch of changes of the watch of run `run`.
fn wait(changes: FolderChanges, run: u64) -> Command<Msg> {
    Command::perform(move || Msg::Changed(run, !changes.next().is_empty()))
}

/// The place of the icon the floor drew at `index` in the whole order, which may hold ids the
/// floor did not draw.
fn place(order: &[String], drawn: &[String], index: usize) -> Option<usize> {
    let id = drawn.get(index)?;
    order.iter().position(|kept| kept == id)
}

/// The id of the row the notification `index` is drawn on, so the keys can reach it.
fn notice_id(index: usize) -> String {
    format!("notice-{index}")
}

/// The target with this id.
fn find(targets: &[Target], id: &str) -> Option<Target> {
    targets.iter().find(|target| target.id == id).cloned()
}

impl App for Desk {
    type Msg = Msg;

    fn init(&mut self) -> Command<Msg> {
        let problems = self.notices.clone();
        let notices: Vec<Command<Msg>> = problems.iter().map(|problem| self.told(problem)).collect();
        let watching = self.watch();
        Command::batch([self.next_minute(), watching, Command::focus(FLOOR)].into_iter().chain(notices))
    }

    fn resized(&self, size: Size) -> Option<Msg> {
        Some(Msg::Resized(size))
    }

    /// How often the screen may be drawn: what the person set, or the framework's own pace when
    /// they set nothing.
    ///
    /// Without a chosen number the default already draws sixty frames a second here and twenty
    /// over SSH, which is the design's decision (5.3), so the desktop repeats nothing. A chosen
    /// number holds on every connection: someone who says ten meant ten. The runtime asks before
    /// every frame, so a number changed in Settings is the next frame's, and a frame that answers
    /// a key press is never held back by it.
    fn frame_limit(&self) -> FrameLimit {
        match self.prefs.frame_cap {
            // The chosen number, held inside the range the settings screen offers.
            Some(chosen) => FrameLimit::per_second(u32::from(chosen.clamp(FRAME_CAP_LEAST, FRAME_CAP_MOST))),
            // Nothing chosen: the framework's own pace, which the runtime reads the connection for.
            None => FrameLimit::default(),
        }
    }

    fn action(&self, name: &str) -> Option<Msg> {
        match name {
            "launcher" => Some(Msg::OpenLauncher),
            "close" => Some(Msg::Close),
            "desktop-mode" => Some(Msg::DesktopMode),
            // The help layer is reached wherever the keys are the desktop's, and says so itself.
            "help" => Some(Msg::Help),
            // The keys of desktop mode are single letters and arrows. They answer only while the
            // desktop has the keys; otherwise the key is nobody's and goes on as it did before.
            _ if self.keys.is_none() => None,
            "window-move" => Some(Msg::Keys(Keys::Move)),
            "window-resize" => Some(Msg::Keys(Keys::Resize)),
            "window-release" => Some(Msg::Keys(Keys::Pick)),
            "window-maximize" => self.windows.focus().map(|id| Msg::Ask(Ask::Maximize, id)),
            "window-minimize" => self.windows.focus().map(|id| Msg::Ask(Ask::Minimize, id)),
            "window-close" => self.windows.focus().map(|id| Msg::Ask(Ask::Close, id)),
            "window-tile" => Some(Msg::Tile),
            "notices" => Some(Msg::Notices),
            "window-left" => Some(Msg::Arrow(Arrow::Left, false)),
            "window-right" => Some(Msg::Arrow(Arrow::Right, false)),
            "window-up" => Some(Msg::Arrow(Arrow::Up, false)),
            "window-down" => Some(Msg::Arrow(Arrow::Down, false)),
            "window-left-far" => Some(Msg::Arrow(Arrow::Left, true)),
            "window-right-far" => Some(Msg::Arrow(Arrow::Right, true)),
            "window-up-far" => Some(Msg::Arrow(Arrow::Up, true)),
            "window-down-far" => Some(Msg::Arrow(Arrow::Down, true)),
            _ => None,
        }
    }

    /// Text pasted where nothing took it.
    ///
    /// A person who pastes into something that looks like a terminal expects the text to arrive,
    /// and the last screen of a program that has ended looks exactly like one. Nothing can take it
    /// there — the program it belonged to is gone — so the desktop says that plainly rather than
    /// let the paste disappear without a word.
    ///
    /// It says it only then. A paste onto the floor, into the launcher or over any other surface
    /// is not aimed at a program: nobody expects pasting onto a desktop background to do anything,
    /// and a word for every stray paste would be noise. Copies are not answered at all.
    fn clipboard(&self, event: &ClipboardEvent) -> Option<Msg> {
        let ClipboardEvent::Pasted(_) = event else { return None };
        // Only while the keys belong to the window in front and nothing stands over it; otherwise
        // the paste was never aimed at the window's body.
        if self.keys.is_some() || self.launcher.is_some() || self.help || self.inbox_open {
            return None;
        }
        let window = self.windows.focused()?;
        if window.is_minimized() || !matches!(window.run(), Some(wm::Run::Ended { .. })) {
            return None;
        }
        Some(Msg::PastedNowhere)
    }

    fn before_quit(&self) -> Option<Msg> {
        // The person's programs are their data: leaving with one running is asked about, counted.
        (self.programs.running() > 0).then_some(Msg::AskQuit)
    }

    fn update(&mut self, msg: Msg) -> Command<Msg> {
        let command = self.applied(msg);
        // A window the keys are in needs no mark on the dock: the person is looking at it.
        if let Some(id) = self.windows.focus() {
            self.attention.remove(&id);
        }
        command
    }

    fn view(&self, ui: &mut View<'_, Msg>) {
        let size = ui.size();
        if size.width < MIN_WIDTH || size.height < MIN_HEIGHT {
            Self::too_small(ui);
        } else {
            self.desktop_view(ui);
        }
    }
}
