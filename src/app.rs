//! The desktop application: the floor with its icons, the launcher, the dock, and the runtime
//! that drives them.

mod follow;
mod gadgets;
mod strip;
mod wallpaper;

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use qframe::date::{DateTime, local_offset};
use qframe::desktop::XdgDirs;
use qframe::env::Env;
use qframe::graphics::Graphics;
use qframe::keymap::Scope;
use qframe::prelude::*;
use qframe::runtime::{ClipboardEvent, Confirm, FrameLimit, Task, Termination, Update, UpdateCheck};
use qframe::storage::{Family, FolderChanges, FolderWatch, Settings, machine_name};
use qframe::widgets::{
    BigText, ContextItem, ContextMenu, EmptyState, Field, FileChange, FileManager, FileManagerMsg, FileManagerState,
    FilePickerMsg, FileView, FolderEntry, Form, HelpLayer, ImageData, ImageError, KeyHints, Modal, NameFor, Panel,
    RowMark, TextInput, Toast, Tooltip,
};

use crate::apps::{
    Catalog, Category, Diagnostic, Entry, Environment, Install, Launch, Localized, Screen, Source, WindowPrefs,
    find_program, is_executable,
};
use crate::desktop::order::file_id;
use crate::desktop::{self, Cell, Desktop, Floor, IconCell, grid};
use crate::dock;
use crate::files::{self, FilesWindow, Programs};
use crate::gadgets::{Gadget, Kind};
use crate::inbox::{Inbox, Notice};
use crate::launcher::{self, Launcher, Shelf, Way};
use crate::notice;
use crate::power::{self, Action, Tools};
use crate::session::{self, Change, Sessions, Start, Subtitle};
use crate::settings::{self, DockPosition, DragStyle, FRAME_CAP_LEAST, FRAME_CAP_MOST, Prefs, UpdateFolders};
use crate::status::{Probe, Status};
use crate::wallpapers;
use crate::wm::{self, Exit, Grip, SPACES, TooSmall, Window, WindowId, Windows, layout};

pub use follow::FolderNews;
pub use gadgets::note_field;
pub use wallpaper::{PICKER, PICTURE, Purpose, Shown, Wallpaper};

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

/// The name of the password field of the lock screen, so the keys are in it the moment the screen
/// locks and after a wrong password.
pub const LOCK_FIELD: &str = "lock-password";

/// The name of the field that asks for the name of a new folder of the Desktop, or a new name for
/// one of its entries, so the keys go to it when the dialog opens.
pub const NAME_FIELD: &str = "desktop-folder-name";

/// The width of that dialog, in cells: the framework's file manager asks with the same.
const NAMING_WIDTH: u16 = 48;

/// The width of the password field of the lock screen, in cells.
const LOCK_FIELD_WIDTH: u16 = 36;

/// The program a folder opens in when it is on the machine: the ecosystem's file explorer.
pub const EXPLORER: &str = "qexp";

/// How far an arrow key with shift moves or sizes a window in desktop mode (design 3.3).
pub const FAR_STEP: u16 = 5;

/// How long the note on resizing stays while the pointer is not on it: two sentences take longer
/// to read than a toast's usual five seconds.
const RESIZE_HINT_FOR: Duration = Duration::from_secs(12);

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
    let probe = Probe::new(&environment);
    let notes = qframe::storage::data_dir("quvyta").map(|dir| dir.join("desktop").join("notes"));
    let mut app = Desk::new(machine_name(), local_offset(), Box::new(wall_now)).probe(probe);
    if let Some(notes) = notes {
        app = app.notes_folder(notes);
    }
    let app = app
        .apps(environment)
        .catalog(catalog)
        .desktop(desktop)
        .config(path)
        .settings(chosen.settings, chosen.prefs, chosen.diagnostics)
        .remote(remote_link())
        .notices(notices)
        .update_notice(UpdateFolders::here());
    let mut runtime = Runtime::new(app);
    for &(file, text) in crate::locales() {
        runtime = runtime.locale_source(file, text);
    }
    let (file, text) = crate::keymap();
    runtime.keymap_source(file, text).run()
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
    /// Go to the workspace of this number, counted from 0, and give the keys to the window that
    /// had them there.
    Workspace(usize),
    /// Send this window to the workspace of this number, counted from 0.
    SendTo(WindowId, usize),
    /// Something happened on the Settings screen of a window.
    Settings(settings::Msg),
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
    /// The password field of the lock screen changed.
    LockTyped(String),
    /// Enter in the password field of the lock screen: check what is typed.
    Unlock,
    /// The password was checked: `true` when it was the person's.
    Unlocked(bool),
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
    windows: Windows,
    programs: Sessions,
    /// The windows whose programs called for attention while they did not have the keys.
    attention: BTreeSet<WindowId>,
    /// Why the program of a window never started, for the window that stayed open to say so.
    failures: BTreeMap<WindowId, Failure>,
    /// The file manager of every Files window, gone with its window.
    files: BTreeMap<WindowId, FilesWindow>,
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
    /// Where the ecosystem's update notice is kept, or `None` where qdesk asks for no newer version.
    updates: Option<UpdateFolders>,
    /// The helpers of the session actions, found on the environment's `PATH`.
    tools: Tools,
    /// The lock screen, while the screen is locked.
    lock: Option<Lock>,
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
}

/// What the lock screen holds while it covers the desktop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Lock {
    /// What is typed in the password field.
    typed: String,
    /// Whether a password is being checked; another Enter waits for the answer.
    checking: bool,
    /// Whether the last password was not the person's.
    wrong: bool,
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
            settings_watch: None,
            settings_run: 0,
            written: Vec::new(),
            remote: false,
            screen: settings::Screen::default(),
            // The terminal says how large it is before the first frame; until then the desktop
            // has no room and no window.
            windows: Windows::new(Size::new(0, 0)),
            programs: Sessions::new(&Environment::default(), Prefs::default(), false),
            attention: BTreeSet::new(),
            failures: BTreeMap::new(),
            files: BTreeMap::new(),
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
            probe: None,
            status: Status::default(),
            monitor: None,
            ticking: false,
            notes_dir: None,
            notes: BTreeMap::new(),
            unreadable_notes: BTreeSet::new(),
            wallpaper: Wallpaper::default(),
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
        // The switch of the update notice is kept, in whichever order the two were given.
        self.screen = settings::Screen::new(problems).with_update_notice(self.screen.update_notice());
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
        let on = folders.as_ref().map(|folders| Family::QUVYTA.update_notice_in(&folders.config));
        self.screen = std::mem::take(&mut self.screen).with_update_notice(on);
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

    /// What the floor's actions mean.
    fn on_floor(&mut self, action: desktop::Action) -> Command<Msg> {
        let ids = self.floor_ids();
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
            desktop::Action::Extend(index) => {
                // The range runs from the cursor, which stays where it is: a second shift click
                // draws the range again from the same icon, as in a list of files.
                let from = self.cursor.filter(|from| *from < ids.len()).unwrap_or(index);
                let (low, high) = (from.min(index), from.max(index));
                self.selection = ids.get(low..=high).map(<[String]>::to_vec).unwrap_or_default();
                if self.cursor.is_none() {
                    self.cursor = Some(index);
                }
            }
            desktop::Action::Band(indices) => {
                self.selection = indices.iter().filter_map(|index| ids.get(*index).cloned()).collect();
                self.cursor = indices.first().copied();
            }
            desktop::Action::Clear => self.selection.clear(),
            desktop::Action::Cursor(index) => self.cursor = Some(index),
            // Opening is the view's business: it makes the message with the name in it.
            desktop::Action::Open(index) => self.cursor = Some(index),
            desktop::Action::PlaceGadget { index, at } => {
                if let Some(gadget) = self.desktop.widgets.get_mut(index)
                    && gadget.place != at
                {
                    gadget.place = at;
                    return Command::batch([closing, self.save()]);
                }
            }
            desktop::Action::Place { index, moves, layout } => {
                let drawn: Vec<(String, Option<Cell>)> =
                    ids.into_iter().zip(layout.into_iter().chain(std::iter::repeat(None))).collect();
                if self.desktop.place_all(&drawn, &moves) {
                    // The order of the icons does not change, so the cursor, which counts in it,
                    // stays on the icon that moved.
                    self.cursor = Some(index);
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
            // A space typed into the empty search is the launcher's key pressed again, not a search.
            launcher::Msg::Query(query) if launcher::closes(&launcher.query, &query) => {
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
            launcher::Msg::Select(index) => launcher.selected = Some(index),
            // The rest carry what they act on; the view turns them into the messages above.
            launcher::Msg::Submit
            | launcher::Msg::Open(_)
            | launcher::Msg::AddIcon(_)
            | launcher::Msg::RemoveIcon(_)
            | launcher::Msg::Install(_)
            | launcher::Msg::Power(_) => {}
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
        let opened = match &entry.launch {
            // A folder opens where every folder opens: the explorer, else a Files window.
            Launch::Open(path) if path.is_dir() => match self.explorer_on(path) {
                Some(explorer) => self.open_entry(&explorer, true),
                None => self.open_files(&entry, path.clone(), fresh),
            },
            // A file waits for the viewers; opening a window that could show nothing would be
            // worse than saying so.
            Launch::Open(_) => {
                let toast = Toast::info(t!("notice.no-viewer", name = target.name.as_str()));
                return Command::batch([closing, saved, Command::toast(toast)]);
            }
            Launch::Screen(Screen::Files) => {
                // The home folder is where a person's own files are; a machine that names none
                // still has its root to look through.
                let home = self.apps.home.clone().unwrap_or_else(|| PathBuf::from("/"));
                self.open_files(&entry, home, fresh)
            }
            Launch::Command(_) | Launch::Screen(Screen::Terminal | Screen::Settings) => self.open_entry(&entry, fresh),
        };
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
            let started = self.start(id, entry);
            return Command::batch([started, self.resize_hint()]);
        };
        // It may be on another workspace; the launcher takes the person there.
        self.windows.bring(id);
        Command::none()
    }

    /// Opens a Files window for `entry` onto the folder `root` and starts reading it, or brings the
    /// entry's window forward when it opens only once and no window of its own was asked for.
    ///
    /// Each window has a manager of its own, kept under the window's id and let go with it. The
    /// folders on screen are followed, in a screen test too, whose waits are bounded by the
    /// desktop's patience (see [`watch_within`](Self::watch_within)).
    fn open_files(&mut self, entry: &Entry, root: PathBuf, fresh: bool) -> Command<Msg> {
        self.keys = None;
        if entry.single
            && !fresh
            && let Some(id) = self.windows.of_entry(&entry.id)
        {
            // It may be on another workspace; the launcher takes the person there.
            self.windows.bring(id);
            return Command::none();
        }
        let id = self.windows.open(entry);
        let mut window = FilesWindow::new(root, self.apps.data_home.as_deref(), self.patience);
        let read = window.manager.load(move |message| Msg::Files(id, message));
        self.files.insert(id, window);
        Command::batch([read, self.resize_hint()])
    }

    /// The window that shows `folder` in the ecosystem's file explorer, when Settings says folders
    /// open there and [`EXPLORER`] is on the `PATH` of the environment — the same `PATH` the entries' programs
    /// are looked for in, so a test decides whether there is one. `None` opens a Files window.
    ///
    /// The window is named after the folder and started in it, and its program is given the folder
    /// by its whole path; a folder whose path is not text keeps the Files window, rather than
    /// giving the explorer a guessed name.
    fn explorer_on(&self, folder: &Path) -> Option<Entry> {
        if !self.prefs.folders_in_explorer {
            return None;
        }
        let program = find_program(EXPLORER, self.apps.path.as_deref(), is_executable)?;
        let words = vec![program.to_str()?.to_owned(), folder.to_str()?.to_owned()];
        let name =
            folder.file_name().map_or_else(|| folder.display().to_string(), |name| name.to_string_lossy().into_owned());
        let mut entry = Self::made_entry(EXPLORER, &name, "folder", Category::Files, Launch::Command(words));
        entry.folder = Some(folder.to_path_buf());
        Some(entry)
    }

    /// Opens `folder` in a window of its own: the explorer's, else a Files window.
    fn open_folder(&mut self, folder: PathBuf) -> Command<Msg> {
        let opened = match self.explorer_on(&folder) {
            Some(explorer) => self.open_entry(&explorer, true),
            None => {
                let entry = self.screen_entry("files", Screen::Files);
                self.open_files(&entry, folder, true)
            }
        };
        Command::batch([opened, self.body_focus()])
    }

    /// Reads the Desktop folder and follows it, so what another program puts in it comes to the
    /// floor by itself. A screen test's desktop follows it too, each wait bounded by its patience
    /// (see [`watch_within`](Self::watch_within)).
    fn read_folder(&mut self) -> Command<Msg> {
        let patience = self.patience;
        let Some(folder) = self.folder.take() else { return Command::none() };
        let folder = self.folder.insert(match patience {
            Some(bound) => folder.following_within(bound),
            None => folder.following(true),
        });
        folder.load(Msg::DesktopFolder)
    }

    /// Hands a message to the manager of the Desktop folder, keeping the floor in step with it: a
    /// renamed entry keeps its place, the places of entries gone from the folder are dropped, and
    /// the keys go back to the floor when the dialog asking for a name closes.
    fn on_folder(&mut self, message: FileManagerMsg) -> Command<Msg> {
        let Some(folder) = &mut self.folder else { return Command::none() };
        let mut renamed = false;
        if let FileManagerMsg::Done(results) = &message {
            for (_, result) in results {
                // Only an entry of the folder itself stands on the floor; one moved inside a
                // folder of it leaves, and its place goes when the folder is read again.
                if let Ok(FileChange::Moved(from, to)) = result
                    && !from.contains('/')
                    && !to.contains('/')
                {
                    let (from, to) = (file_id(from), file_id(to));
                    self.desktop.rename_place(&from, &to);
                    for selected in &mut self.selection {
                        if *selected == from {
                            selected.clone_from(&to);
                        }
                    }
                    renamed = true;
                }
            }
        }
        let root_read = matches!(&message, FileManagerMsg::Listed(key, Ok(_)) if key.is_empty());
        let asking = folder.naming().is_some();
        let answer = folder.update(message, Msg::DesktopFolder);
        let asked = asking && folder.naming().is_none();
        if root_read && let Some(entries) = folder.children("") {
            let names: BTreeSet<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
            renamed |= self.desktop.keep_files(|name| names.contains(name));
        }
        let saved = if renamed { self.save() } else { Command::none() };
        let back = if asked { Command::focus(FLOOR) } else { Command::none() };
        Command::batch([answer, saved, back])
    }

    /// Opens the dialog asking for a name in the Desktop folder, with the keys in its field.
    fn ask_name(&mut self, message: FileManagerMsg) -> Command<Msg> {
        let Some(folder) = &mut self.folder else { return Command::none() };
        let asked = folder.update(message, Msg::DesktopFolder);
        Command::batch([asked, Command::focus(NAME_FIELD)])
    }

    /// Opens `file`, chosen in a Files window or on the floor, in a terminal window of its own,
    /// started in the file's folder: with the terminal program the desktop's databases choose for
    /// its kind, else in the person's editor, else in [`files::READER`] (see
    /// [`files::default_command`]).
    fn open_file(&mut self, file: &Path) -> Command<Msg> {
        let choices = self.openers.for_file(file);
        let words = files::default_command(&choices, self.apps.editor.as_deref(), file);
        self.open_file_with(file, words)
    }

    /// Opens `file` with the program chosen on its "Open with" menu: the terminal program of the
    /// desktop file id `program`, or the person's editor for `None`. A program that has gone since
    /// the menu opened leaves the file to the editor.
    fn open_with(&mut self, file: &Path, program: Option<&str>) -> Command<Msg> {
        let openers = Arc::clone(&self.openers);
        let words = program
            .and_then(|id| openers.terminal_program(id))
            .and_then(|app| files::with_program(app, file))
            .or_else(|| files::opener(self.apps.editor.as_deref(), file));
        self.open_file_with(file, words)
    }

    /// Opens `file` in a window running `words`.
    ///
    /// Every terminal program is an application, and an editor is one; the window is the same as
    /// any command's, and stays when the program ends, so what it said last can be read. It is
    /// named after the file and drawn with the icon of the file's kind. A file whose name is not
    /// text (`words` is `None`) is not opened under a guessed name: the corner says why.
    fn open_file_with(&mut self, file: &Path, words: Option<Vec<String>>) -> Command<Msg> {
        let name =
            file.file_name().map_or_else(|| file.display().to_string(), |name| name.to_string_lossy().into_owned());
        let Some(words) = words else {
            return Command::toast(Toast::info(t!("files.not-text", name = name.as_str())));
        };
        let icon = desktop::kind_icon(&name, false, false);
        let mut entry = Self::made_entry("files.open", &name, icon, Category::Files, Launch::Command(words));
        entry.folder = file.parent().map(Path::to_path_buf);
        let opened = self.open_entry(&entry, true);
        Command::batch([opened, self.body_focus()])
    }

    /// A Terminal window started in `folder`: the Terminal entry as the person has it, with the
    /// folder in place of the home folder.
    fn terminal_here(&mut self, folder: PathBuf) -> Command<Msg> {
        let mut entry = self.screen_entry("terminal", Screen::Terminal);
        entry.folder = Some(folder);
        let opened = self.open_entry(&entry, true);
        Command::batch([opened, self.body_focus()])
    }

    /// The entry of the screen `screen`: the one of id `id` in the catalog, which a person may have
    /// changed, else one made here — a person may hide a built-in entry from the launcher, and a
    /// folder's "Open a terminal here" still has to open a terminal.
    fn screen_entry(&self, id: &str, screen: Screen) -> Entry {
        match self.catalog.get(id) {
            Some(entry) if entry.launch == Launch::Screen(screen) => entry.clone(),
            _ => {
                let (name, icon, category) = match screen {
                    Screen::Terminal => ("Terminal", "prompt", Category::System),
                    Screen::Files => ("Files", "folder", Category::Files),
                    Screen::Settings => ("Settings", "settings", Category::System),
                };
                Self::made_entry(id, name, icon, category, Launch::Screen(screen))
            }
        }
    }

    /// An entry the desktop makes for a window of its own, read from no file.
    fn made_entry(id: &str, name: &str, icon: &str, category: Category, launch: Launch) -> Entry {
        Entry {
            id: id.to_owned(),
            name: Localized::plain(name),
            comment: None,
            icon: Some(icon.to_owned()),
            launch,
            folder: None,
            env: Vec::new(),
            unset: Vec::new(),
            category,
            keywords: Vec::new(),
            single: false,
            close_on_exit: false,
            window: WindowPrefs::default(),
            install: Install::default(),
            try_exec: None,
            source: Source::Builtin,
            file: None,
        }
    }

    /// What the file manager of the Files window `id` said: the manager takes it and answers with
    /// the reading and the file work it asks for. A message for a window that has closed since is
    /// dropped, as a late word of a closed window's program is.
    fn on_files(&mut self, id: WindowId, message: FileManagerMsg) -> Command<Msg> {
        let Some(window) = self.files.get_mut(&id) else { return Command::none() };
        window.manager.update(message, move |message| Msg::Files(id, message))
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
        // Over the lock screen the corner would show what the program said to anyone passing; the
        // list keeps it for the person who unlocks.
        if self.lock.is_some() {
            return Command::none();
        }
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
                if self.lock.is_some() {
                    return Command::none();
                }
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

    /// Brings the window `id` forward, from the dock or from a press on one of its notifications,
    /// going to its workspace when it is on another one.
    /// "Resize" on a window's menu: the window comes forward and desktop mode opens at its sizing
    /// step, where the arrows size it and the dock's row says the mouse does it too.
    fn resize_from_keys(&mut self, id: WindowId) -> Command<Msg> {
        self.inbox_open = false;
        self.more = false;
        if !self.windows.bring(id) {
            return Command::none();
        }
        self.keys = Some(Keys::Resize);
        Command::focus(FLOOR)
    }

    /// The note on how a window is resized, the first time a window opens and never again: the
    /// edges that size a window say nothing of themselves until the pointer is over them.
    fn resize_hint(&mut self) -> Command<Msg> {
        if self.desktop.resize_hint_seen {
            return Command::none();
        }
        self.desktop.resize_hint_seen = true;
        let toast = Toast::info(t!("window.resize-hint"))
            .body(t!("window.resize-hint-body"))
            .key("resize-hint")
            .duration(RESIZE_HINT_FOR);
        Command::batch([Command::toast(toast), self.save()])
    }

    fn bring(&mut self, id: WindowId) -> Command<Msg> {
        // The list has been acted on, so it steps out of the way of the window it just raised.
        self.inbox_open = false;
        let away = self.windows.get(id).is_some_and(|window| window.space() != self.windows.current());
        if !self.windows.bring(id) {
            return Command::none();
        }
        if away {
            self.more = false;
        }
        self.body_focus()
    }

    /// Goes to the workspace `space` and gives the keys to the window that had them there, or to
    /// the floor: like opening a window from the launcher, going somewhere is a step out of desktop
    /// mode (design 3.10).
    fn switch(&mut self, space: usize) -> Command<Msg> {
        if !self.windows.switch(space) {
            return Command::none();
        }
        // The list of the windows that did not fit named the windows of the workspace just left.
        self.more = false;
        self.keys = None;
        self.dragging = None;
        self.body_focus()
    }

    /// Sends the window `id` to the workspace `space`. Desktop mode stays: a person sending windows
    /// away is arranging them, and the next key is likely another such step.
    fn send_to(&mut self, id: WindowId, space: usize) -> Command<Msg> {
        if !self.windows.send(id, space) {
            return Command::none();
        }
        if self.dragging.map(wm::Dragging::id) == Some(id) {
            self.dragging = None;
        }
        self.body_focus()
    }

    /// What the strip of the window `id` shows after the name: what its program says about itself
    /// (its own title, else the folder it is in), else the title of a window without a program.
    fn said(&self, id: WindowId) -> Option<String> {
        match self.programs.subtitle(id) {
            Some(Subtitle::Title(title)) => Some(title.to_owned()),
            Some(Subtitle::Folder(folder)) => Some(wm::view::folder_text(folder, self.apps.home.as_deref())),
            // A Files window says which folder it shows, as a Terminal window says which folder
            // its shell is in.
            None if self.files.contains_key(&id) => self.files.get(&id).map(|window| {
                let shown = window.manager.path(window.manager.folder());
                wm::view::folder_text(&shown, self.apps.home.as_deref())
            }),
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
            Launch::Screen(Screen::Settings | Screen::Files) | Launch::Open(_) => return String::new(),
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
        // A closed Files window's manager goes with it, and its folder watch with the manager.
        self.files.remove(&id);
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
        if self.files.contains_key(&window.id()) {
            return Command::focus(wm::view::body_id(window.id()));
        }
        match window.screen() {
            Some(Screen::Settings) => Command::focus(settings::LIST),
            Some(Screen::Terminal | Screen::Files) | None => Command::focus(FLOOR),
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
                self.drag_size(id, grip, dx, dy);
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

    /// One step of a drag of the edge or corner `grip` of the window `id`.
    ///
    /// The steps are added up and the window is sized from the rectangle it had when the drag
    /// began, so the held edge stays under the pointer: one that stopped at the smallest size or
    /// at the screen does not start back until the pointer is over it again.
    fn drag_size(&mut self, id: WindowId, grip: Grip, dx: i32, dy: i32) {
        let running = match self.dragging {
            Some(wm::Dragging::Sizing { id: held, grip: holding, from, by }) if held == id && holding == grip => {
                Some((from, by))
            }
            _ => None,
        };
        let Some((from, by)) = running.or_else(|| Some((self.windows.get(id)?.rect(), (0, 0)))) else {
            return;
        };
        let by = (by.0.saturating_add(dx), by.1.saturating_add(dy));
        if self.windows.resize_from(id, from, grip, by.0, by.1) {
            self.dragging = Some(wm::Dragging::Sizing { id, grip, from, by });
        }
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
                // The keys move the right and the bottom edge, so a window sized from the keys
                // keeps its top left corner; the mouse can hold any edge or corner instead.
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
                // The strip switched on reads the machine again, when nothing else was.
                let reading = self.sample(Duration::ZERO);
                Command::batch([self.store(), reading])
            }
            Some(settings::Request::Shared(shared)) => {
                self.share(&shared);
                self.store()
            }
            Some(settings::Request::UpdateNotice(on)) => self.store_update_notice(on),
            Some(settings::Request::Wallpaper(asked)) => self.on_wallpaper_asked(asked),
            None => Command::none(),
        };
        Command::batch([applied, stored])
    }

    /// The question for a newer version of qdesk, when the ecosystem's update notice is on.
    ///
    /// The switch is read here, not only where the question is sent: a person who turned it off
    /// asks nothing at all, whoever runs the question.
    fn ask_for_update(&self) -> Command<Msg> {
        let Some(folders) = &self.updates else { return Command::none() };
        if !Family::QUVYTA.update_notice_in(&folders.config) {
            return Command::none();
        }
        let check = UpdateCheck::new(
            Family::QUVYTA,
            settings::APP,
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            Msg::NewVersion,
        )
        .in_folders(folders.config.clone(), folders.state.clone());
        Command::check_for_update(check)
    }

    /// Turns the ecosystem's update notice on or off in its shared file, off the render path.
    fn store_update_notice(&self, on: bool) -> Command<Msg> {
        let Some(folders) = &self.updates else { return Command::none() };
        let folder = folders.config.clone();
        Command::perform(move || {
            let stored = Family::QUVYTA.set_update_notice_in(&folder, on).map_err(|error| error.to_string());
            Msg::Settings(settings::Msg::Stored(stored))
        })
    }

    /// Writes the settings on a background thread, so a slow disk never holds up drawing. The
    /// text is remembered, so the watch on the file knows the change as the desktop's own.
    fn store(&mut self) -> Command<Msg> {
        self.remember_write();
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

    /// A session action pressed at the launcher's foot. The launcher goes away first, as it does
    /// for an application.
    fn on_power(&mut self, action: Action) -> Command<Msg> {
        self.launcher = None;
        // Only what the launcher offered can be asked for: a message that names another action,
        // on a machine or a connection where it is hidden, does nothing.
        if !self.tools.offered(self.remote).contains(&action) {
            return Command::focus(FLOOR);
        }
        match action {
            Action::Lock => {
                self.lock = Some(Lock::default());
                self.keys = None;
                self.help = false;
                self.more = false;
                self.inbox_open = false;
                Command::focus(LOCK_FIELD)
            }
            // Logging out is leaving qdesk, with the question leaving asks when programs run.
            Action::LogOut if self.programs.running() > 0 => Command::batch([Command::focus(FLOOR), self.ask_quit()]),
            Action::LogOut => Command::quit(),
            Action::Restart | Action::PowerOff => {
                let running = self.programs.running();
                let key = action.key();
                let message = if running > 0 {
                    t!(&format!("power.{key}-running"), n = running)
                } else {
                    t!(&format!("power.{key}-text"))
                };
                let question = Confirm::new(t!(&format!("power.{key}-title")), Msg::PowerConfirmed(action))
                    .message(message)
                    .confirm_label(action.label())
                    .danger();
                Command::batch([Command::focus(FLOOR), Command::confirm(question)])
            }
        }
    }

    /// Restarting or powering off, once confirmed: `systemctl` is asked off the render path, and
    /// the corner says why when the system does not agree.
    ///
    /// The only way here is the answer to the question [`on_power`](Self::on_power) asks. The
    /// action is looked at again all the same: an answer that arrives after the terminal became a
    /// remote one, or a message from anywhere else, never powers off what the launcher would not
    /// have offered.
    fn carry_out(&self, action: Action) -> Command<Msg> {
        if !self.tools.offered(self.remote).contains(&action) {
            return Command::none();
        }
        match self.tools.command(action) {
            Some(process) => Command::perform(move || Msg::PowerDone(action, power::carry_out(process))),
            None => Command::none(),
        }
    }

    /// Checks what is typed on the lock screen, off the render path. The field is emptied at once:
    /// the password is not kept on screen or in the desktop while it is checked.
    fn unlock(&mut self) -> Command<Msg> {
        let Some(lock) = &mut self.lock else { return Command::none() };
        if lock.checking {
            return Command::none();
        }
        let Some(checker) = self.tools.checker().cloned() else { return Command::none() };
        lock.checking = true;
        lock.wrong = false;
        let typed = std::mem::take(&mut lock.typed);
        Command::perform(move || Msg::Unlocked(checker.check(&typed)))
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
            Msg::PowerDone(action, Err(reason)) => {
                let heading = t!(&format!("power.{}-failed", action.key()));
                self.inbox.add(Notice::desktop(heading.clone(), reason.clone()));
                Command::toast(Toast::warning(heading).body(reason))
            }
            Msg::LockTyped(typed) => {
                if let Some(lock) = &mut self.lock {
                    lock.typed = typed;
                    lock.wrong = false;
                }
                Command::none()
            }
            Msg::Unlock => self.unlock(),
            Msg::Unlocked(true) => {
                self.lock = None;
                self.body_focus()
            }
            Msg::Unlocked(false) => {
                if let Some(lock) = &mut self.lock {
                    lock.checking = false;
                    lock.wrong = true;
                }
                Command::focus(LOCK_FIELD)
            }
            Msg::Floor(action) => self.on_floor(action),
            msg @ (Msg::AddGadget(_)
            | Msg::RemoveGadget(_)
            | Msg::SetGadget(..)
            | Msg::Sampled(..)
            | Msg::Second
            | Msg::NoteTyped(..)) => self.on_gadget(msg),
            msg @ (Msg::Attach(_) | Msg::Monitor) => self.on_strip(msg),
            Msg::OpenLauncher => {
                self.launcher = Some(Launcher::default());
                Command::focus(launcher::SEARCH)
            }
            Msg::ToggleLauncher if self.launcher.is_some() => {
                self.launcher = None;
                Command::focus(FLOOR)
            }
            Msg::ToggleLauncher => {
                self.launcher = Some(Launcher::default());
                Command::focus(launcher::SEARCH)
            }
            Msg::Close => self.on_close(),
            Msg::Resized(size) => {
                self.windows.resize(size);
                self.wallpaper_resized(size)
            }
            Msg::Window(action) => self.on_window(action),
            Msg::Dock(id) => self.on_dock(id),
            Msg::Ask(Ask::Minimize, id) => self.minimize(id),
            Msg::Ask(Ask::Maximize, id) => {
                self.windows.toggle_maximized(id);
                Command::none()
            }
            Msg::Ask(Ask::Resize, id) => self.resize_from_keys(id),
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
            Msg::Workspace(space) => self.switch(space),
            Msg::SendTo(id, space) => self.send_to(id, space),
            Msg::Settings(message) => self.on_settings(message),
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
            Msg::FilesView(id, view) => {
                if let Some(window) = self.files.get_mut(&id) {
                    window.view = view;
                }
                Command::none()
            }
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
                // Every icon flows again, in that order.
                self.desktop.clear_places();
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

    /// The open windows of the workspace on screen as the dock shows them, in the order they were
    /// opened.
    fn items(&self, ui: &View<'_, Msg>) -> Vec<dock::Item> {
        let language = ui.env().i18n().active();
        let icons = ui.env().icons();
        let mark = icons.glyph("window-minimize").into_owned();
        let called = icons.glyph("dot").into_owned();
        let focus = self.windows.focus();
        let mut windows: Vec<&Window> = self.windows.here().collect();
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
        let strip = self.chips(ui.env().icons());
        let texts: Vec<String> = strip.iter().map(|chip| chip.text.clone()).collect();
        // The plan measures the items as the buttons that draw them, so the row it plans is the
        // row that is painted whatever the theme's padding is.
        let padding = ui.env().theme().style("button", None, &[]).pair("padding").map_or(2, |(_, sides)| sides);
        let plan = dock::plan(ui.size().width, &self.machine_text(), &clock, &count, &labels, &texts, padding);
        let desktop_keys = self.keys;
        let workspaces = self.workspaces();
        let row = |ui: &mut View<'_, Msg>| match desktop_keys {
            // In desktop mode the dock's row says what the keys do, as the design asks (3.3).
            // It replaces the dock wherever the dock now is, so the row the person reads never
            // moves under them.
            Some(keys) => Self::hints_view(keys, ui),
            None => {
                let presses = dock::Presses {
                    space: &Msg::Workspace,
                    workspaces,
                    launcher: Msg::ToggleLauncher,
                    launcher_open: self.launcher.is_some(),
                    more: Msg::MoreWindows,
                    notices: Msg::Notices,
                    press: &|item: &dock::Item| Msg::Dock(item.id),
                    menu: &|item: &dock::Item| self.window_menu(item.id),
                    chip: &|chip: &dock::Chip| self.chip_press(chip.kind),
                    chip_menu: &|chip: &dock::Chip| self.chip_menu(chip.kind),
                };
                let parts = dock::Parts { clock: &clock, count: &count, items: &items, strip: &strip };
                dock::view(&plan, &parts, &presses, ui);
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
            // The floor's colour and pattern from the settings, under the icons; the windows keep
            // the theme. A picture covers the whole floor, so nothing is laid under it, and its
            // colour and pattern are what shows wherever the picture cannot be.
            if self.wallpaper_drawn(ui.env()) {
                self.wallpaper_view(ui);
            } else {
                settings::ground(self.prefs.floor, self.prefs.floor_style, ui);
            }
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
            if let Some(folder) = &self.folder {
                Self::naming_view(folder, ui);
            }
            if self.help {
                self.help_view(ui);
            }
            self.wallpaper_picker_view(ui);
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
                } else if let Some(files) = self.files.get(&window.id()) {
                    self.files_body(window.id(), files, ui);
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

    /// The folder of a Files window, drawn by the framework's file manager in the window's shape.
    ///
    /// A file is opened in a terminal window, a folder's menu opens it in a new Files window or a
    /// terminal there, and the rows of folders another window's program stands in carry that
    /// window's icon.
    fn files_body(&self, id: WindowId, files: &FilesWindow, ui: &mut View<'_, Msg>) {
        let icons = ui.env().icons();
        let standing = self.programs.folders().filter_map(|(window, folder)| {
            self.windows.get(window).map(|window| (folder, desktop::icon_name(window.entry(), icons)))
        });
        let marks = files::row_marks(files.manager.root(), standing);
        // The menu is built when it opens, long after this frame, so it takes the folders along.
        let folders = files.manager.folder_keys();
        let root = files.manager.root().to_path_buf();
        let openers = Arc::clone(&self.openers);
        let editor = self.apps.editor.clone();
        FileManager::new(&files.manager, move |message| Msg::Files(id, message))
            .view(files.view)
            .kind_icons(true)
            .on_open(|file| Msg::OpenFile(file.to_path_buf()))
            .on_open_terminal(|folder| Msg::TerminalHere(folder.to_path_buf()))
            .menu_items(move |key, _| {
                let path = key.split('/').filter(|part| !part.is_empty()).fold(root.clone(), |at, part| at.join(part));
                if key.is_empty() || folders.contains(key) {
                    vec![ContextItem::new(t!("files.open-new-window"), Msg::FilesHere(path))]
                } else {
                    let picture = wallpapers::is_picture(key.rsplit('/').next().unwrap_or(key));
                    let mut items = vec![Self::open_with_menu(&openers, editor.as_deref(), path.clone())];
                    if picture {
                        items.push(ContextItem::new(t!("files.set-wallpaper"), Msg::SetWallpaper(path)));
                    }
                    items
                }
            })
            .row_mark(move |key| marks.get(key).cloned().unwrap_or_else(RowMark::new))
            .show(ui)
            .id(wm::view::body_id(id))
            .fill();
    }

    /// The "Open with" row of a file's menu: the terminal programs the desktop's databases name
    /// for the file's kind, the default first, and the person's editor last, which opens any file.
    ///
    /// Graphical programs are left out, as they would have no screen to open on. The databases
    /// are read here, when the menu opens, not while the window is drawn.
    fn open_with_menu(openers: &Programs, editor: Option<&str>, file: PathBuf) -> ContextItem<Msg> {
        let choices = openers.for_file(&file);
        let mut items: Vec<ContextItem<Msg>> = files::terminal_programs(&choices)
            .into_iter()
            .map(|app| ContextItem::new(app.name.clone(), Msg::OpenWith(file.clone(), Some(app.id.clone()))))
            .collect();
        let program = files::editor_name(editor);
        let label = if editor.is_some_and(|editor| !editor.trim().is_empty()) {
            t!("files.open-with-editor", program = program.as_str())
        } else {
            t!("files.open-with-reader", program = program.as_str())
        };
        items.push(ContextItem::new(label, Msg::OpenWith(file, None)));
        ContextItem::submenu(t!("files.open-with"), items)
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
    ///
    /// A Files window adds the shapes its folder can be drawn in, the one in use marked with a
    /// sign rather than a colour alone.
    fn window_menu(&self, id: WindowId) -> Vec<ContextItem<Msg>> {
        let mut items = vec![
            ContextItem::new(t!("window.minimize"), Msg::Ask(Ask::Minimize, id)),
            ContextItem::new(t!("window.maximize"), Msg::Ask(Ask::Maximize, id)),
            ContextItem::new(t!("window.resize"), Msg::Ask(Ask::Resize, id)),
            ContextItem::new(t!("window.tile"), Msg::Tile),
        ];
        // The other workspaces, each a row of its own: four is few enough to name them all, and a
        // submenu would hide the one thing the row is for.
        let here = self.windows.get(id).map_or(self.windows.current(), Window::space);
        items.push(ContextItem::gap());
        for space in (0..SPACES).filter(|space| *space != here) {
            items.push(ContextItem::new(t!("window.send-to", n = space + 1), Msg::SendTo(id, space)));
        }
        if let Some(files) = self.files.get(&id) {
            items.push(ContextItem::gap());
            for (view, label) in [
                (FileView::List, t!("files.view-list")),
                (FileView::Tree, t!("files.view-tree")),
                (FileView::Icons, t!("files.view-icons")),
            ] {
                let item = ContextItem::new(label, Msg::FilesView(id, view));
                items.push(if files.view == view { item.icon("check") } else { item });
            }
        }
        items.push(ContextItem::gap());
        items.push(ContextItem::new(t!("window.close"), Msg::Ask(Ask::Close, id)));
        items
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
                .hint(arrows.clone(), t!("help.icons-move"))
                .hint(format!("shift {arrows}"), t!("floor.help-move"))
                .hint("enter", t!("help.icons-open"))
                .hint("esc", t!("help.icons-clear"))
                .hint(t!("help.icons-jump"), t!("help.icons-jump-label")),
        );
    }

    /// The workspaces as the dock's marks show them.
    fn workspaces(&self) -> dock::Workspaces {
        let mut occupied = [false; SPACES];
        for (space, held) in occupied.iter_mut().enumerate() {
            *held = self.windows.occupied(space);
        }
        dock::Workspaces { current: self.windows.current(), occupied }
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
                // Going to a workspace leaves desktop mode, and the dock that comes back shows the
                // marks: the row needs no marks of its own, only the key.
                .hint(format!("1-{SPACES}"), t!("mode.workspace"))
                .hint("z", t!("mode.maximize"))
                .hint("n", t!("mode.minimize"))
                .hint("t", t!("mode.tile"))
                .hint("space", t!("mode.launcher"))
                .hint("b", t!("mode.notices")),
            Keys::Move => KeyHints::new()
                .hint(arrows, t!("mode.move"))
                .hint("shift", t!("mode.far", cells = FAR_STEP))
                .hint("enter", t!("mode.release"))
                .hint("esc", t!("mode.back")),
            // Sizing also says the mouse does it: this row is where a person who came from the
            // window's menu looks, and the edges say nothing until the pointer is over them. It
            // comes before shift, which a narrow row drops first and the help layer still lists.
            Keys::Resize => KeyHints::new()
                .hint(arrows, t!("mode.resize"))
                .hint(t!("mode.drag"), t!("mode.drag-label"))
                .hint("enter", t!("mode.release"))
                .hint("esc", t!("mode.back"))
                .hint("shift", t!("mode.far", cells = FAR_STEP)),
        };
        ui.add(hints).fill_width().height(Length::Cells(dock::HEIGHT));
    }

    /// The icons on the floor, each with its own menu, and the menu of the empty floor.
    ///
    /// The applications come first, in their order, and the entries of the Desktop folder after
    /// them, each standing in its own place when it has one.
    fn floor(&self, ui: &mut View<'_, Msg>) {
        let language = ui.env().i18n().active().to_owned();
        let icons = ui.env().icons();
        let mut shown: Vec<FloorIcon> = self
            .icons()
            .into_iter()
            .map(|entry| {
                let target = Target::of(entry, &language);
                FloorIcon {
                    id: target.id.clone(),
                    name: target.name.clone(),
                    glyph: desktop::glyph_of(entry, icons),
                    open: Msg::Open(target.clone()),
                    menu: Self::icon_items(&target, &self.desktop),
                }
            })
            .collect();
        if let Some(folder) = &self.folder {
            for entry in self.folder_entries() {
                let path = folder.root().join(&entry.name);
                let open = if entry.folder { Msg::FilesHere(path.clone()) } else { Msg::OpenFile(path.clone()) };
                shown.push(FloorIcon {
                    id: file_id(&entry.name),
                    name: entry.name.clone(),
                    glyph: icons.glyph(desktop::kind_icon(&entry.name, entry.folder, entry.executable)).into_owned(),
                    menu: Self::entry_items(&entry.name, &open, (!entry.folder).then_some(&path)),
                    open,
                });
            }
        }
        let names: Vec<String> = shown.iter().map(|icon| icon.name.clone()).collect();
        let places: Vec<Option<Cell>> = shown.iter().map(|icon| self.desktop.places.get(&icon.id).copied()).collect();
        let selected: Vec<usize> = shown
            .iter()
            .enumerate()
            .filter(|(_, icon)| self.selection.contains(&icon.id))
            .map(|(index, _)| index)
            .collect();
        let cursor = self.cursor;
        // In desktop mode the arrows pick a window, so the floor lets them through; while a name
        // is asked for, the keys are the dialog's.
        let naming = self.folder.as_ref().is_some_and(|folder| folder.naming().is_some());
        let keys = self.launcher.is_none() && self.keys.is_none() && !naming && !self.wallpaper.picking();
        let opening: Vec<Msg> = shown.iter().map(|icon| icon.open.clone()).collect();
        // Over a picture every icon stands on a tile of the theme's card tone, so its name reads
        // whatever the picture is under it.
        let backed = self.wallpaper_drawn(ui.env());
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
            let floor = Floor::new(names, move |action| match action {
                desktop::Action::Open(index) => opening.get(index).cloned().unwrap_or(Msg::Ignore),
                other => Msg::Floor(other),
            })
            .places(places)
            .gadgets(self.desktop.widgets.iter().map(Gadget::spot).collect())
            .selected(selected)
            .cursor(cursor)
            .keys(keys);
            ui.add_with(floor, |ui| {
                for (index, icon) in shown.into_iter().enumerate() {
                    let cell = IconCell::new(icon.glyph, icon.name.clone())
                        .selected(self.selection.contains(&icon.id))
                        .cursor(cursor == Some(index))
                        .backed(backed);
                    let menu = ContextMenu::new(icon.menu);
                    let shortened = grid::shown_name(&icon.name) != icon.name;
                    if shortened {
                        // The cell shows what fits; the whole name is one hover away.
                        ui.add_with(Tooltip::new(icon.name), |ui| {
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
                self.gadget_nodes(ui);
            })
            .id(FLOOR)
            .fill();
        })
        .fill();
    }

    /// The dialog asking for the name of a new folder of the Desktop, or a new name for one of its
    /// entries, while one is asked for. It is the framework's file manager's own dialog in words
    /// and in checks — the same title, field and buttons, and the manager says what is wrong with a
    /// name as it is typed — standing over the desktop instead of inside a window.
    fn naming_view(folder: &FileManagerState, ui: &mut View<'_, Msg>) {
        let Some(naming) = folder.naming() else { return };
        let (title, confirm) = match &naming.purpose {
            NameFor::Rename(key) => {
                (t!("quvyta.file-manager.rename-title", name = key.as_str()), t!("quvyta.file-manager.rename-do"))
            }
            _ => (t!("quvyta.file-manager.new-folder-title"), t!("quvyta.file-manager.create")),
        };
        let close = Msg::DesktopFolder(FileManagerMsg::CloseNaming);
        let submit = Msg::DesktopFolder(FileManagerMsg::Submit);
        let dialog = Modal::new()
            .title(title)
            .width(NAMING_WIDTH)
            .on_close(close.clone())
            .action(Button::new(t!("quvyta.file-manager.cancel")).on_press(close))
            .action(Button::new(confirm).variant("primary").on_press(submit.clone()));
        let problem = folder.naming_problem().map(|problem| problem.message());
        let value = naming.value.clone();
        ui.add_with(dialog, |ui| {
            Form::new().show(ui, |fields| {
                let label = t!("quvyta.file-manager.name-label");
                fields.field(Field::new(label).error(problem.clone()), |ui| {
                    let input = TextInput::new(value)
                        .invalid(problem.is_some())
                        .on_change(|value| Msg::DesktopFolder(FileManagerMsg::Name(value)))
                        .on_submit(move |_| submit.clone());
                    ui.add(input).id(NAME_FIELD).fill_width();
                });
            });
        });
    }

    /// The one line a floor too narrow for icons shows in their place (3.7): where the
    /// applications went. The whole line names the keys too; where it does not fit, the shorter
    /// one names only the dock's button, so nothing is cut half way.
    fn narrow_hint(ui: &View<'_, Msg>) -> String {
        let icons = ui.env().icons();
        let launcher = icons.glyph(desktop::quvyta_icon(icons)).into_owned();
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

    /// The rows of the menu of an entry of the Desktop folder called `name`, which `open` opens;
    /// `file` is where it is when it is a file, and a picture among them can be the wallpaper.
    fn entry_items(name: &str, open: &Msg, file: Option<&PathBuf>) -> Vec<ContextItem<Msg>> {
        let mut items = vec![
            ContextItem::new(t!("icon.open"), open.clone()),
            ContextItem::new(t!("icon.rename"), Msg::RenameEntry(name.to_owned())),
        ];
        if let Some(file) = file.filter(|_| wallpapers::is_picture(name)) {
            items.push(ContextItem::new(t!("files.set-wallpaper"), Msg::SetWallpaper(file.clone())));
        }
        items
    }

    /// The rows of the empty floor's menu.
    fn floor_items(&self, language: &str) -> Vec<ContextItem<Msg>> {
        let entry = |id: &str| self.catalog.get(id).map(|entry| Target::of(entry, language));
        let mut items = Vec::new();
        if let Some(terminal) = entry("terminal") {
            items.push(ContextItem::new(t!("floor.new-terminal"), Msg::Open(terminal)));
        }
        if self.folder.is_some() {
            items.push(ContextItem::new(t!("floor.new-folder"), Msg::NewFolder));
        }
        items.push(ContextItem::new(t!("floor.add-application"), Msg::OpenLauncher));
        items.extend(self.gadget_floor_items());
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
        let actions = self.tools.offered(self.remote);
        self.against_dock(ui, |ui| {
            ui.row(|ui| {
                ui.map(
                    move |message| match message {
                        launcher::Msg::Submit => targets.first().cloned().map_or(Msg::Ignore, Msg::Open),
                        launcher::Msg::Open(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::Open),
                        launcher::Msg::Install(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::Install),
                        launcher::Msg::AddIcon(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::AddIcon),
                        launcher::Msg::RemoveIcon(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::RemoveIcon),
                        launcher::Msg::Power(action) => Msg::Power(action),
                        other => Msg::Launcher(other),
                    },
                    |ui| launcher::view(launcher, &apps, &shelves, &actions, ui),
                );
                ui.spacer();
            });
        });
    }

    /// The lock screen: the time, the machine and a field for the person's password, over
    /// everything else. Nothing of the desktop is drawn under it, so nothing of it can be read or
    /// pressed.
    fn lock_view(&self, lock: &Lock, ui: &mut View<'_, Msg>) {
        let user = self.tools.checker().map(|checker| checker.user().to_owned()).unwrap_or_default();
        let clock = self.clock_text();
        let machine = self.machine_text();
        ui.column(|ui| {
            ui.spacer();
            ui.add(BigText::new(clock));
            ui.add(Text::new(machine).role("secondary").no_wrap());
            ui.spacer().height(Length::Cells(1));
            ui.row(|ui| {
                ui.add(
                    TextInput::new(lock.typed.clone())
                        .password(true)
                        .placeholder(t!("power.lock-password", user = user.as_str()))
                        .disabled(lock.checking)
                        .invalid(lock.wrong)
                        .on_change(Msg::LockTyped)
                        .on_submit(|_| Msg::Unlock),
                )
                .id(LOCK_FIELD)
                .width(Length::Cells(LOCK_FIELD_WIDTH));
            });
            let (line, role) = if lock.checking {
                (t!("power.lock-checking"), "secondary")
            } else if lock.wrong {
                (t!("power.lock-wrong"), "")
            } else {
                (t!("power.lock-hint"), "faint")
            };
            let line = Text::new(line).no_wrap();
            let line = if role.is_empty() { line.color("danger") } else { line.role(role) };
            ui.add(line);
            ui.spacer();
        })
        .align(Align::Center)
        .fill();
    }

    /// The welcome line, shown once and never again once it is closed.
    fn welcome_view(ui: &mut View<'_, Msg>) {
        let launcher = ui.env().icons().glyph(desktop::quvyta_icon(ui.env().icons())).into_owned();
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

/// One icon as the floor draws it: what it is called and drawn with, what opening it does, and
/// the rows of its menu.
struct FloorIcon {
    id: String,
    name: String,
    glyph: String,
    open: Msg,
    menu: Vec<ContextItem<Msg>>,
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
        let following = self.watch_settings();
        // The theme, the language and the rest every Quvyta application shares, as the file says
        // them, before the first frame is drawn.
        let shared = self.stored.apply();
        let reading = self.read_folder();
        let picture = self.start_wallpaper();
        let asked = self.ask_for_update();
        let notes = self.read_notes();
        let ticks = self.gadget_ticks();
        Command::batch(
            [
                self.next_minute(),
                shared,
                watching,
                following,
                reading,
                picture,
                Command::focus(FLOOR),
                asked,
                notes,
                ticks,
            ]
            .into_iter()
            .chain(notices),
        )
    }

    fn resized(&self, size: Size) -> Option<Msg> {
        Some(Msg::Resized(size))
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
