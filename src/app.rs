//! The desktop application: the floor with its icons, the launcher, the dock, and the runtime
//! that drives them.

mod file_windows;
mod floor;
mod follow;
mod gadgets;
mod install;
mod lock;
mod notices;
mod opening;
mod pictures;
mod preferences;
mod programs;
mod recommended;
mod strip;
mod texts;
mod view;
mod wallpaper;
mod windows;

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use qframe::date::{DateTime, local_offset};
use qframe::desktop::XdgDirs;
use qframe::env::Env;
use qframe::graphics::Graphics;
use qframe::i18n::I18n;
use qframe::prelude::*;
use qframe::runtime::{ClipboardEvent, FrameLimit, Task, Termination, Update};
use qframe::storage::{Ecosystem, FolderChanges, FolderWatch, Preferences, Settings, machine_name};
use qframe::widgets::{
    Appearance, FileManagerMsg, FileManagerState, FilePickerMsg, FileView, FolderEntry, ImageData, ImageError, Toast,
};

use crate::apps::{Catalog, Diagnostic, Entry, Environment, Launch, Screen};
use crate::desktop::order::file_id;
use crate::desktop::{self, Desktop};
use crate::files::{FilesWindow, Programs};
use crate::gadgets::Kind;
use crate::inbox::{Inbox, Notice};
use crate::launcher::{self, Launcher, Way};
use crate::pictures::{PictureWindow, Step};
use crate::power::{Action, Tools};
use crate::secret::Secret;
use crate::session::{self, Sessions};
use crate::settings::{self, FRAME_CAP_LEAST, FRAME_CAP_MOST, Prefs, UpdateFolders};
use crate::status::{Probe, Status};
use crate::texts::{ReadText, TextWindow, Unread};
use crate::wallpapers;
use crate::wm::{self, WindowId, Windows};

pub use follow::FolderNews;
pub use gadgets::note_field;
pub use lock::LOCK_FIELD;
use lock::{LOCK_MARK, Lock};
use programs::Failure;
pub use wallpaper::{PICKER, PICTURE, Purpose, Shown, Wallpaper};

/// Below this many columns the desktop cannot be drawn and the screen says so.
pub const MIN_WIDTH: u16 = 40;
/// Below this many rows the desktop cannot be drawn and the screen says so.
pub const MIN_HEIGHT: u16 = 10;
/// Below this width or height the icons are hidden: the narrow screen.
pub const NARROW_WIDTH: u16 = 60;
/// Below this height the icons are hidden.
pub const NARROW_HEIGHT: u16 = 16;

/// The name of the floor, so the keys go back to it when a surface above it closes.
pub const FLOOR: &str = "floor";

/// The name of the field that asks for the name of a new folder of the Desktop, or a new name for
/// one of its entries, so the keys go to it when the dialog opens.
pub const NAME_FIELD: &str = "desktop-folder-name";

/// The program a folder opens in when it is on the machine: the ecosystem's file explorer.
pub const EXPLORER: &str = "qexp";

/// How far an arrow key with shift moves or sizes a window in desktop mode.
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
    // The settings an older qdesk wrote are settled with the ecosystem before anything reads them.
    let chosen = settings::start();
    let stored = chosen.settings;
    let probe = Probe::new(&environment);
    let notes = qframe::storage::data_dir("quvyta").map(|dir| dir.join("desktop").join("notes"));
    let mut app = Desk::new(machine_name(), local_offset(), Box::new(wall_now)).probe(probe);
    if let Some(notes) = notes {
        app = app.notes_folder(notes);
    }
    if let Some(state) = Ecosystem::QUVYTA.state_dir(settings::APP) {
        app = app.state_folder(&state);
    }
    let app = app
        .apps(environment)
        .catalog(catalog)
        .desktop(desktop)
        .config(path)
        .settings(stored.clone(), chosen.prefs, chosen.diagnostics)
        .remote(remote_link())
        .notices(notices)
        .update_notice(UpdateFolders::here());
    runtime(app, &stored).run()
}

/// The framework runtime that runs `app`, everything but the terminal: the desktop as a member of
/// the Quvyta ecosystem, with its own words and its own keys.
///
/// As a member, the shared language, theme, icons and reduced motion are applied before the first
/// frame and followed while the desktop runs. The desktop's own file `stored` is handed over as
/// qdesk read it, so it is not read a second time. [`run`] runs it on this terminal; a screen test
/// starts the very same runtime in a harness, so what it draws is what a person sees.
#[must_use]
pub fn runtime(app: Desk, stored: &Settings) -> Runtime<Desk> {
    let mut runtime = Runtime::new(app).settings(stored).member(Ecosystem::QUVYTA, settings::APP);
    for &(file, text) in crate::locales() {
        runtime = runtime.locale_source(file, text);
    }
    let (file, text) = crate::keymap();
    runtime.keymap_source(file, text)
}

/// Whether the terminal the desktop is drawn on is reached over a network.
///
/// The answer is the framework's own rule, the one [`Env::remote`] answers in a view, so every
/// Quvyta application reads the connection the same way and the desktop keeps no rule of its own
/// about SSH. The settings that follow the connection (how often a program's output may ask for a
/// frame, the power actions offered, the size a wallpaper is decoded at) are decided between
/// frames, where no environment is handed out, so the desktop asks once here, before the runtime
/// starts, and keeps the answer. [`Env::remote_session`] reads two variables and no file.
fn remote_link() -> bool {
    Env::remote_session()
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
                Launch::Screen(Screen::Settings | Screen::Files) => None,
            },
            way: Way::of(entry),
            file: entry.file.clone(),
        }
    }
}

/// What reaches the desktop.
#[derive(Debug, Clone)]
pub enum Msg {
    /// The wall clock reached a new minute.
    Minute,
    /// A watched entry folder changed, on the watch of this run; `true` when there was something
    /// in the batch, `false` when the watch was dropped.
    Changed(u64, bool),
    /// A bounded wait of the entry folders' watch of this run ended with nothing changed, so the
    /// watch waits again. Only a desktop given a bound by
    /// [`Desk::watch_within`](Desk::watch_within) — a screen test — ever hears this.
    EntriesQuiet(u64),
    /// Something happened on the floor.
    Floor(desktop::Action),
    /// Open the launcher.
    OpenLauncher,
    /// Open the launcher, or close it while it is open: the dock's button and the key of the
    /// launcher, which work like a Start button.
    ToggleLauncher,
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
    /// Installing the installer of an application was confirmed: do it in a window of its own.
    InstallHere(Target),
    /// Put an icon for an application on the floor.
    AddIcon(Target),
    /// Take an application's icon off the floor.
    RemoveIcon(Target),
    /// Say where an entry comes from and what it runs.
    Properties(Target),
    /// Put the icons of the floor in the order of their names.
    Arrange(Vec<String>),
    /// The panel of the first start was closed: the welcome line and the recommended applications
    /// never come back by themselves.
    WelcomeSeen,
    /// Install on a row of the recommended applications: the panel closes and the application is
    /// installed as the launcher installs it.
    InstallRecommended(Target),
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
    /// Go to the workspace of this number, counted from 0, and give the keys to the window that
    /// had them there.
    Workspace(usize),
    /// Send this window to the workspace of this number, counted from 0.
    SendTo(WindowId, usize),
    /// Something happened on the Settings screen of a window.
    Settings(settings::Msg),
    /// The shared preferences as the runtime resolved them at start or after the ecosystem's files
    /// changed; see [`App::preferences`].
    Preferences(Preferences),
    /// A wait on the folder of the settings file, on the watch of this run, heard this.
    SettingsFolder(u64, FolderNews),
    /// Something happened in the file manager of a Files window.
    Files(WindowId, FileManagerMsg),
    /// A Files window draws its folder in another shape, from the window's menu.
    FilesView(WindowId, FileView),
    /// A file chosen in a Files window, to open in a terminal window of its own.
    OpenFile(PathBuf),
    /// "Open with" on a file's menu in a Files window: the file, and the desktop file id of the
    /// terminal program chosen, or `None` for the person's editor.
    OpenWith(PathBuf, Option<String>),
    /// "Open a terminal here" on a folder of a Files window.
    TerminalHere(PathBuf),
    /// "Open in a new window" on a folder of a Files window, and opening a folder of the Desktop.
    FilesHere(PathBuf),
    /// Something happened to the Desktop folder: it was read, an operation on it ended, or the
    /// dialog asking for a name changed.
    DesktopFolder(FileManagerMsg),
    /// "New folder" on the floor's menu: ask for a name and make it in the Desktop folder.
    NewFolder,
    /// "Rename" on the menu of an entry of the Desktop folder, by its name.
    RenameEntry(String),
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
    /// A newer version of qdesk is out.
    NewVersion(Update),
    /// A session action at the launcher's foot was pressed.
    Power(Action),
    /// Restarting or powering off was confirmed: carry it out.
    PowerConfirmed(Action),
    /// Restarting or powering off was asked of the system, which agreed or said why not.
    PowerDone(Action, Result<(), String>),
    /// The password field of the lock screen changed. The text moves into the lock screen and is
    /// overwritten once it is replaced or checked.
    LockTyped(Secret),
    /// Enter in the password field of the lock screen: check what is typed.
    Unlock,
    /// The password was checked: `true` when it was the person's.
    Unlocked(bool),
    /// A second of the wait after a wrong password is over.
    LockTick,
    /// Put a gadget of this kind on the floor: the floor's menu.
    AddGadget(Kind),
    /// Take the gadget at this index off the floor.
    RemoveGadget(usize),
    /// Change one option of the gadget at this index: its menu.
    SetGadget(usize, crate::gadgets::Change),
    /// The machine was read for the system gadget; the probe comes back for the next reading.
    Sampled(Probe, Status),
    /// The wall clock reached a new second, for a clock that shows its seconds.
    Second,
    /// The note kept in this file was typed into; this is all it holds now.
    NoteTyped(String, String),
    /// Open a new terminal window attached to the tmux session of this name.
    Attach(String),
    /// Open the machine's process monitor, `btop` or `htop`: a press on the processor or the
    /// memory of the status strip.
    Monitor,
    /// "Set as wallpaper" on a picture's menu: decode it, and lay it over the floor when it
    /// decodes.
    SetWallpaper(PathBuf),
    /// A picture was decoded for the floor, on the decoding of this run, for this purpose.
    WallpaperDecoded(u64, wallpapers::Picture, Purpose, Result<ImageData, ImageError>),
    /// The terminal draws pictures this way now: before the first frame and whenever it changes.
    Graphics(Graphics),
    /// Something happened in the file picker that chooses a picture for the floor.
    WallpaperPicker(FilePickerMsg),
    /// The file picker was closed without a choice.
    CloseWallpaperPicker,
    /// A key or a click in the picture viewer of this window.
    Picture(WindowId, Step),
    /// A picture was decoded for the viewer of this window, on its decoding of this number.
    PictureDecoded(WindowId, u64, Result<ImageData, ImageError>),
    /// The file of the text viewer of this window was read, or could not be.
    TextRead(WindowId, Result<ReadText, Unread>),
    /// The text viewer of this window gives its file to the person's editor, in the same window.
    EditText(WindowId),
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
    /// Bring it forward and let the arrow keys size it.
    Resize,
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
    /// The watch on the folder of the settings file, so a change another program makes to the
    /// file is applied while the desktop runs.
    settings_watch: Option<FolderWatch>,
    /// Which watch of the settings folder is the current one.
    settings_run: u64,
    /// The settings files the desktop wrote itself and has not seen land yet, as text.
    written: Vec<String>,
    /// Whether the terminal is reached over a network, as the framework's environment says.
    remote: bool,
    screen: settings::Screen,
    /// The appearance section of the Settings screen: the language, the theme, the icons and
    /// reduced motion as the Quvyta ecosystem shares them, where each comes from, and where a
    /// change is saved. See [`Desk::settings`] and [`App::preferences`].
    appearance: Appearance,
    windows: Windows,
    programs: Sessions,
    /// The windows whose programs called for attention while they did not have the keys.
    attention: BTreeSet<WindowId>,
    /// Why the program of a window never started, for the window that stayed open to say so.
    failures: BTreeMap<WindowId, Failure>,
    /// The file manager of every Files window, gone with its window.
    files: BTreeMap<WindowId, FilesWindow>,
    /// The picture of every picture viewer window, gone with its window.
    pictures: BTreeMap<WindowId, PictureWindow>,
    /// The file of every text viewer window, gone with its window or when its editor takes it.
    texts: BTreeMap<WindowId, TextWindow>,
    /// Which programs open which kind of file, as the desktop's own databases say; read the first
    /// time it is asked and again after the programs change. Shared with the menus of the Files
    /// windows, which are built when they open.
    openers: Arc<Programs>,
    /// The manager of the Desktop folder, whose entries stand on the floor after the applications;
    /// `None` when there is no Desktop folder. Nothing of it is drawn as a file manager: it reads
    /// and watches the folder, checks and makes names, and the floor shows what it read.
    folder: Option<FileManagerState>,
    dragging: Option<wm::Dragging>,
    keys: Option<Keys>,
    more: bool,
    /// Everything the corner has said this run, and how much of it is unread. It is never written
    /// anywhere: the list is forgotten when qdesk closes.
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
    /// Where the ecosystem's update notice is kept, or `None` where qdesk asks for no newer version.
    updates: Option<UpdateFolders>,
    /// The helpers of the session actions, found on the environment's `PATH`.
    tools: Tools,
    /// The lock screen, while the screen is locked.
    lock: Option<Lock>,
    /// The file whose presence says the screen is locked, in the desktop's state folder; `None`
    /// keeps the lock only while the desktop runs.
    lock_mark: Option<PathBuf>,
    /// What the system gadget reads the machine with; away while a reading is under way, and
    /// `None` for a desktop given none, which never reads the machine.
    probe: Option<Probe>,
    /// The reading the system gadget and the status strip show.
    status: Status,
    /// The process monitor a press on the strip's processor or memory opens, found on the
    /// environment's `PATH`; `None` when the machine has neither `btop` nor `htop`.
    monitor: Option<PathBuf>,
    /// Whether a wait for the next second is under way, for a clock showing its seconds.
    ticking: bool,
    /// The folder the notes are kept in; `None` keeps them only while the desktop runs.
    notes_dir: Option<PathBuf>,
    /// What every note holds, by its file.
    notes: BTreeMap<String, String>,
    /// The notes whose files could not be read: never written over.
    unreadable_notes: BTreeSet<String>,
    /// The picture over the floor, and the dialog that chooses one.
    wallpaper: Wallpaper,
    /// Whether Settings asked for the recommended applications again.
    recommended_open: bool,
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
            settings_watch: None,
            settings_run: 0,
            written: Vec::new(),
            remote: false,
            screen: settings::Screen::default(),
            appearance: detached_appearance(),
            // The terminal says how large it is before the first frame; until then the desktop
            // has no room and no window.
            windows: Windows::new(Size::new(0, 0)),
            programs: Sessions::new(&Environment::default(), Prefs::default(), false),
            attention: BTreeSet::new(),
            failures: BTreeMap::new(),
            files: BTreeMap::new(),
            pictures: BTreeMap::new(),
            texts: BTreeMap::new(),
            openers: Arc::new(Programs::new(XdgDirs::default(), None, None)),
            folder: None,
            dragging: None,
            keys: None,
            more: false,
            inbox: Inbox::default(),
            inbox_open: false,
            help: false,
            leaving: false,
            patience: None,
            updates: None,
            tools: Tools::default(),
            lock: None,
            lock_mark: None,
            probe: None,
            status: Status::default(),
            monitor: None,
            ticking: false,
            notes_dir: None,
            notes: BTreeMap::new(),
            unreadable_notes: BTreeSet::new(),
            wallpaper: Wallpaper::default(),
            recommended_open: false,
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
    ///
    /// The file is in the ecosystem's folder, so the appearance section saves its changes in that
    /// folder too: the shared file beside `desktop.conf`, or `desktop.conf` itself. Settings kept
    /// only in memory save nothing there either. What the section starts with is what the two
    /// files say now; a desktop started as a member hears it again from the runtime before its
    /// first frame and whenever the files change ([`App::preferences`]).
    #[must_use]
    pub fn settings(mut self, stored: Settings, prefs: Prefs, problems: Vec<Diagnostic>) -> Self {
        if let Some(folder) = stored.path().and_then(Path::parent) {
            let preferences = Ecosystem::QUVYTA.preferences_without_saving_in(folder, settings::APP, &I18n::builtin());
            self.appearance = Appearance::new(Ecosystem::QUVYTA, settings::APP, preferences).in_folder(folder);
        }
        self.stored = stored;
        self.prefs = prefs;
        // The switch of the update notice is kept, in whichever order the two were given.
        self.screen = settings::Screen::new(problems).with_updates(self.screen.updates());
        self.programs.set_prefs(self.prefs, self.remote);
        let picture = settings::wallpaper(&self.stored);
        self.name_wallpaper(picture);
        self
    }

    /// Whether the terminal the desktop is drawn on is reached over a network, which is what
    /// [`Env::remote_session`] answers before the runtime starts. It is the desktop's one answer
    /// about the connection: how often a program's output may ask for a frame, whether the power
    /// actions are offered, the frame cap the Settings screen shows and the size a wallpaper is
    /// decoded at all read it. The drag style no longer asks it anything. A screen test that says
    /// `true` here also calls [`Harness::set_remote`](qframe::runtime::Harness::set_remote), so the
    /// framework's own pace agrees.
    #[must_use]
    pub fn remote(mut self, remote: bool) -> Self {
        self.remote = remote;
        self.programs.set_prefs(self.prefs, self.remote);
        self
    }

    /// The environment the entries and the programs are looked for in.
    #[must_use]
    pub fn apps(mut self, apps: Environment) -> Self {
        // The Desktop folder is read when the desktop starts, after every other part is given.
        self.folder = apps.desktop.clone().map(FileManagerState::new);
        self.apps = apps;
        self.openers = self.fresh_openers();
        // The programs need the shell and the home folder of this environment, and keep whatever
        // settings have been given so far, in whichever order the two were set.
        self.programs = Sessions::new(&self.apps, self.prefs, self.remote);
        self.tools = Tools::find(&self.apps);
        self.monitor = strip::monitor(&self.apps);
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

    /// The folder the desktop keeps its state in. A locked screen leaves a mark there that only
    /// the right password takes away, and a desktop that starts while the mark is there starts
    /// locked: a session service that starts qdesk again after it was ended opens no unlocked
    /// desktop. A desktop given no folder leaves no mark, which is what tests want.
    #[must_use]
    pub fn state_folder(mut self, folder: &Path) -> Self {
        self.lock_mark = Some(folder.join(LOCK_MARK));
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

    /// The same desktop, asking at start whether a newer version is out while the ecosystem's update
    /// notice in `folders` is on, and showing that switch on the Settings screen. `None` asks
    /// nothing and shows no switch, which is every test that has not said otherwise.
    #[must_use]
    pub fn update_notice(mut self, folders: Option<UpdateFolders>) -> Self {
        self.screen = std::mem::take(&mut self.screen).with_updates(folders.is_some());
        self.updates = folders;
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
    /// is worse than no icon, and the launcher's Installable section is where it belongs.
    #[must_use]
    pub fn icons(&self) -> Vec<&Entry> {
        self.desktop
            .icons
            .iter()
            .filter(|id| self.catalog.is_installed(id))
            .filter_map(|id| self.catalog.get(id))
            .collect()
    }

    /// The entries of the Desktop folder the floor shows after the applications: folders first,
    /// then files, by name, the hidden ones left out. Empty until the folder has been read, and
    /// with no Desktop folder.
    #[must_use]
    pub fn folder_entries(&self) -> Vec<&FolderEntry> {
        self.folder.as_ref().and_then(|folder| folder.shown_children("")).unwrap_or_default()
    }

    /// The manager of the Desktop folder, while there is one.
    #[must_use]
    pub fn desktop_folder(&self) -> Option<&FileManagerState> {
        self.folder.as_ref()
    }

    /// The ids of every icon on the floor, in the order the floor counts them: the applications,
    /// then the entries of the Desktop folder.
    #[must_use]
    pub fn floor_ids(&self) -> Vec<String> {
        let apps = self.icons().into_iter().map(|entry| entry.id.clone());
        apps.chain(self.folder_entries().into_iter().map(|entry| file_id(&entry.name))).collect()
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

    /// Whether the lock screen covers the desktop.
    #[must_use]
    pub fn locked(&self) -> bool {
        self.lock.is_some()
    }

    /// The shortest time between two reports of output from a program started now: one frame
    /// of the frame cap in force on this connection.
    #[must_use]
    pub fn output_pace(&self) -> Duration {
        self.programs.pace()
    }

    /// The windows of the desktop.
    #[must_use]
    pub fn windows(&self) -> &Windows {
        &self.windows
    }

    /// What the Files window `id` holds: its manager and the shape it draws its folder in; `None`
    /// for a window that is not a Files window, or is gone.
    #[must_use]
    pub fn files(&self, id: WindowId) -> Option<&FilesWindow> {
        self.files.get(&id)
    }

    /// What the picture viewer `id` shows; `None` for a window that is not one, or is gone.
    #[must_use]
    pub fn picture(&self, id: WindowId) -> Option<&PictureWindow> {
        self.pictures.get(&id)
    }

    /// What the text viewer `id` shows; `None` for a window that is not one, or is gone.
    #[must_use]
    pub fn text(&self, id: WindowId) -> Option<&TextWindow> {
        self.texts.get(&id)
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

    /// The picture over the floor: which one the settings name, what became of it, and whether
    /// its file picker is open.
    #[must_use]
    pub fn wallpaper(&self) -> &Wallpaper {
        &self.wallpaper
    }

    /// The Settings screen's state, as the next frame shows it.
    #[must_use]
    pub fn settings_screen(&self) -> &settings::Screen {
        &self.screen
    }

    /// The appearance section of the Settings screen, with the shared preferences as it last heard
    /// them.
    #[must_use]
    pub fn appearance(&self) -> &Appearance {
        &self.appearance
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
    ///
    /// A screen test's desktop watches them too, each wait bounded by its patience (see
    /// [`watch_within`](Self::watch_within)): a test runs the wait where it stands.
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
        // A folder that is not there yet cannot be watched; a program installed later brings it,
        // and the next start sees it.
        let watched = folders.iter().filter(|folder| watch.watch(folder).is_ok()).count();
        if watched == 0 {
            // A watch of no folder never says anything: no thread is kept waiting on it.
            return Command::none();
        }
        let changes = watch.changes();
        self.watch = Some(watch);
        self.run += 1;
        wait(changes, self.run, self.patience)
    }

    /// The databases of kinds and programs this environment names, not read yet.
    fn fresh_openers(&self) -> Arc<Programs> {
        let path = self.apps.path.clone();
        Arc::new(Programs::new(self.apps.xdg(), self.apps.lang.as_deref(), path))
    }

    /// Reads the entries again and keeps watching.
    fn reload(&mut self) -> Command<Msg> {
        let loaded = self.apps.load();
        let notices = loaded.diagnostics;
        self.catalog = Catalog::with_environment(loaded.entries, &self.apps);
        self.desktop.keep_known(|id| self.catalog.get(id).is_some());
        // The programs changed, so which of them opens a file is read again when next asked.
        self.openers = self.fresh_openers();
        let again = match &self.watch {
            Some(watch) => wait(watch.changes(), self.run, self.patience),
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
        if self.files.contains_key(&window.id())
            || self.pictures.contains_key(&window.id())
            || self.texts.contains_key(&window.id())
        {
            return Command::focus(wm::view::body_id(window.id()));
        }
        match window.screen() {
            Some(Screen::Settings) => Command::focus(self.screen.keyboard()),
            Some(Screen::Terminal | Screen::Files) | None => Command::focus(FLOOR),
        }
    }

    /// Applies one message. [`App::update`] wraps it, so what every message leaves behind is
    /// settled in one place.
    fn applied(&mut self, msg: Msg) -> Command<Msg> {
        match msg {
            Msg::Minute => self.next_minute(),
            Msg::Changed(run, changed) if run == self.run && changed => self.reload(),
            // A batch of an older run, or the last empty answer of a dropped watch.
            Msg::Changed(..) | Msg::Ignore => Command::none(),
            Msg::EntriesQuiet(run) => match &self.watch {
                Some(watch) if run == self.run => wait(watch.changes(), run, self.patience),
                // A watch that has been let go, or replaced, is not waited on again.
                _ => Command::none(),
            },
            // It is said in the corner and nowhere else: this is the answer to something the
            // person just did, not an event of the desktop worth keeping in the list.
            Msg::PastedNowhere => Command::toast(Toast::info(t!("notice.paste-nowhere"))),
            Msg::NewVersion(update) => Command::toast(update.toast()),
            Msg::Power(action) => self.on_power(action),
            Msg::PowerConfirmed(action) => self.carry_out(action),
            Msg::PowerDone(_, Ok(())) => Command::none(),
            Msg::PowerDone(action, Err(reason)) => self.power_failed(action, reason),
            Msg::LockTyped(typed) => self.lock_typed(typed),
            Msg::Unlock => self.unlock(),
            Msg::Unlocked(right) => self.unlocked(right),
            Msg::LockTick => self.lock_ticked(),
            Msg::Floor(action) => self.on_floor(action),
            msg @ (Msg::AddGadget(_)
            | Msg::RemoveGadget(_)
            | Msg::SetGadget(..)
            | Msg::Sampled(..)
            | Msg::Second
            | Msg::NoteTyped(..)) => self.on_gadget(msg),
            msg @ (Msg::Attach(_) | Msg::Monitor) => self.on_strip(msg),
            Msg::OpenLauncher => self.open_launcher(),
            Msg::ToggleLauncher => self.toggle_launcher(),
            Msg::Close => self.on_close(),
            Msg::Resized(size) => {
                self.windows.resize(size);
                self.wallpaper_resized(size)
            }
            Msg::Window(action) => self.on_window(action),
            Msg::Dock(id) => self.on_dock(id),
            Msg::Ask(Ask::Minimize, id) => self.minimize(id),
            Msg::Ask(Ask::Maximize, id) => self.maximize(id),
            Msg::Ask(Ask::Resize, id) => self.resize_from_keys(id),
            Msg::Ask(Ask::Close, id) => self.ask_close(id),
            Msg::CloseAnyway(id) => self.close_window(id),
            Msg::Program(report) => self.on_program(report),
            Msg::Quiet(id) => self.listen(id),
            Msg::Restart(id) => self.restart(id),
            Msg::Bring(id) => self.bring(id),
            Msg::AskQuit => self.ask_quit(),
            Msg::Quit => Command::quit(),
            Msg::KeepRunning => self.keep_running(),
            Msg::MoreWindows => self.more_windows(),
            Msg::Notices => self.on_notices(),
            Msg::Help => self.toggle_help(),
            Msg::DesktopMode => self.desktop_mode(),
            Msg::Keys(keys) => self.keys_step(keys),
            Msg::Arrow(arrow, far) => self.on_arrow(arrow, far),
            Msg::Tile => self.tile(),
            Msg::Workspace(space) => self.switch(space),
            Msg::SendTo(id, space) => self.send_to(id, space),
            Msg::Settings(message) => self.on_settings(message),
            // The runtime has switched the screen already; the section only shows where each
            // value now comes from, so the next change is saved where its box says.
            Msg::Preferences(preferences) => {
                self.appearance.refresh(preferences);
                Command::none()
            }
            Msg::SettingsFolder(run, news) => self.on_settings_folder(run, news),
            Msg::Files(id, message) => self.on_files(id, message),
            Msg::OpenFile(file) => self.open_file(&file),
            Msg::OpenWith(file, program) => self.open_with(&file, program.as_deref()),
            Msg::TerminalHere(folder) => self.terminal_here(folder),
            Msg::FilesHere(folder) => self.open_folder(folder),
            Msg::DesktopFolder(message) => self.on_folder(message),
            Msg::NewFolder => self.ask_name(FileManagerMsg::NewFolder(String::new())),
            Msg::RenameEntry(name) => self.ask_name(FileManagerMsg::Rename(name)),
            Msg::SetWallpaper(path) => self.choose_wallpaper(wallpapers::Picture::File(path)),
            Msg::WallpaperDecoded(run, picture, purpose, decoded) => {
                self.on_wallpaper_decoded(run, picture, purpose, decoded)
            }
            Msg::Graphics(graphics) => self.wallpaper_graphics(graphics),
            Msg::WallpaperPicker(message) => self.on_wallpaper_picker(message),
            Msg::CloseWallpaperPicker => self.close_wallpaper_picker(),
            Msg::FilesView(id, view) => self.files_view(id, view),
            Msg::Picture(id, step) => self.on_picture(id, step),
            Msg::PictureDecoded(id, run, decoded) => self.on_picture_decoded(id, run, decoded),
            Msg::TextRead(id, read) => self.on_text_read(id, read),
            Msg::EditText(id) => self.edit_text(id),
            Msg::Launcher(message) => self.on_launcher(message),
            Msg::Open(target) => self.open(&target, false),
            Msg::OpenNew(target) => self.open(&target, true),
            Msg::Install(target) => self.install(&target),
            Msg::InstallHere(target) => self.install_here(&target),
            Msg::AddIcon(target) => self.add_icon(&target),
            Msg::RemoveIcon(target) => self.remove_icon(&target),
            Msg::Properties(target) => Self::properties(&target),
            Msg::Arrange(order) => self.arrange(&order),
            Msg::WelcomeSeen => {
                let saved = self.close_first_start();
                Command::batch([saved, self.body_focus()])
            }
            Msg::InstallRecommended(target) => self.install_recommended(&target),
        }
    }
}

/// The appearance section of a desktop given no settings file: it saves nothing, and starts from
/// what the machine is detected to have, since there are no files to read.
///
/// The framework resolves preferences only from a folder, so it is given one that cannot exist: a
/// folder inside `/dev/null`, which is not a folder. Nothing is read there and nothing written.
/// The machine is looked at once for all such desktops, since detecting the icons reads the font
/// folders.
fn detached_appearance() -> Appearance {
    static DETECTED: std::sync::OnceLock<Preferences> = std::sync::OnceLock::new();
    let preferences = DETECTED.get_or_init(|| {
        Ecosystem::QUVYTA.preferences_without_saving_in(Path::new("/dev/null/quvyta"), settings::APP, &I18n::builtin())
    });
    Appearance::new(Ecosystem::QUVYTA, settings::APP, preferences.clone()).without_saving()
}

impl From<settings::Msg> for Msg {
    fn from(message: settings::Msg) -> Self {
        Self::Settings(message)
    }
}

/// Waits for the next batch of changes of the watch of run `run`.
///
/// With a `patience` the wait lasts at most that long, and ends with [`Msg::EntriesQuiet`] when
/// nothing changed; without one it lasts until something does.
fn wait(changes: FolderChanges, run: u64, patience: Option<Duration>) -> Command<Msg> {
    Command::perform(move || match patience {
        None => Msg::Changed(run, !changes.next().is_empty()),
        Some(bound) => match changes.next_within(bound) {
            Some(batch) => Msg::Changed(run, !batch.is_empty()),
            None => Msg::EntriesQuiet(run),
        },
    })
}

impl App for Desk {
    type Msg = Msg;

    fn init(&mut self) -> Command<Msg> {
        let problems = self.notices.clone();
        let notices: Vec<Command<Msg>> = problems.iter().map(|problem| self.told(problem)).collect();
        let watching = self.watch();
        let following = self.watch_settings();
        let reading = self.read_folder();
        let picture = self.start_wallpaper();
        let asked = self.ask_for_update();
        let notes = self.read_notes();
        let ticks = self.gadget_ticks();
        let keys = if self.starts_locked() { self.lock_screen() } else { Command::focus(FLOOR) };
        Command::batch(
            [self.next_minute(), watching, following, reading, picture, keys, asked, notes, ticks]
                .into_iter()
                .chain(notices),
        )
    }

    fn resized(&self, size: Size) -> Option<Msg> {
        Some(Msg::Resized(size))
    }

    /// The desktop runs as a member of the Quvyta ecosystem ([`run`]), so the runtime applies the
    /// shared language, theme, icons and reduced motion before the first frame and follows both
    /// files, the shared `quvyta.conf` and `desktop.conf`, while the desktop runs. What it resolved
    /// reaches the appearance section here, at start and after every change, so the boxes under
    /// its rows say where each value comes from now.
    ///
    /// The desktop's own watch on `desktop.conf` (its `follow` module) takes care of the desktop's own keys
    /// only, and the runtime of the shared ones: neither applies what the other does, so the two
    /// never undo each other. The runtime reads the files without writing them and applies only
    /// what differs from what it heard last, so the desktop's own writes start no loop either.
    fn preferences(&self, preferences: &Preferences) -> Option<Msg> {
        Some(Msg::Preferences(preferences.clone()))
    }

    /// The wallpaper is decoded at the size the terminal shows: a kitty terminal is sent many
    /// more pixels than half blocks draw.
    fn graphics(&self, graphics: Graphics) -> Option<Msg> {
        Some(Msg::Graphics(graphics))
    }

    /// How often the screen may be drawn: what the person set, or the framework's own pace when
    /// they set nothing.
    ///
    /// Without a chosen number the default already draws sixty frames a second here and twenty
    /// over SSH, so the desktop repeats nothing. A chosen
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
        // The lock screen answers none of the desktop's keys: they would reach what it covers.
        if self.lock.is_some() {
            return None;
        }
        match name {
            "launcher" => Some(Msg::ToggleLauncher),
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
            "workspace-1" => Some(Msg::Workspace(0)),
            "workspace-2" => Some(Msg::Workspace(1)),
            "workspace-3" => Some(Msg::Workspace(2)),
            "workspace-4" => Some(Msg::Workspace(3)),
            "send-to-1" => self.windows.focus().map(|id| Msg::SendTo(id, 0)),
            "send-to-2" => self.windows.focus().map(|id| Msg::SendTo(id, 1)),
            "send-to-3" => self.windows.focus().map(|id| Msg::SendTo(id, 2)),
            "send-to-4" => self.windows.focus().map(|id| Msg::SendTo(id, 3)),
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
        if self.lock.is_some() {
            return None;
        }
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
        // A locked screen is not left: on a machine whose session starts qdesk again, leaving would
        // be a way past the lock.
        if self.lock.is_some() {
            return Some(Msg::Ignore);
        }
        // The person's programs are their data: leaving with one running is asked about, counted.
        (self.programs.running() > 0).then_some(Msg::AskQuit)
    }

    /// The system ending qdesk is answered as the framework does, except that a locked screen
    /// does not keep a machine that is going down waiting: nobody is there to answer a question.
    fn terminating(&self, cause: Termination) -> Option<Msg> {
        match cause {
            Termination::Terminate if self.lock.is_none() => self.before_quit(),
            // A locked screen, a hangup and whatever else the system may end qdesk for: at once.
            _ => None,
        }
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
        if let Some(lock) = &self.lock {
            self.lock_view(lock, ui);
        } else if size.width < MIN_WIDTH || size.height < MIN_HEIGHT {
            Self::too_small(ui);
        } else {
            self.desktop_view(ui);
        }
    }
}
