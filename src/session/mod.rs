//! The programs of the desktop: starting one for a window, hearing what it says, ending it.
//!
//! Every window that holds a program has one live program here, kept under its
//! [`WindowId`]. This module starts them, hands out the widget that draws
//! one, hears what a program says about itself, ends one politely when its window closes, starts
//! one again in place, and counts the ones still running so leaving qdesk can ask about them.
//!
//! Nothing here draws or speaks: a [`Change`] carries the program's own words and the numbers
//! the system gave, never a sentence for the screen. The window list keeps the run state and the
//! title (`Windows::mark_running`, `Windows::mark_ended`, `Windows::set_title`); this module keeps
//! the pseudo-terminal, what the program last said and how it was started.
//!
//! # How the application drives it
//!
//! The desktop holds one [`Sessions`] and a message of its own carrying a [`Report`]:
//!
//! ```ignore
//! enum Msg {
//!     // ...
//!     Program(session::Report),
//! }
//! ```
//!
//! **Opening a window.** After `Windows::open` gives the id, start the program with the size the
//! window's body will have, then watch it:
//!
//! ```ignore
//! let id = self.windows.open(entry);
//! match self.programs.start(id, entry, body_size) {
//!     Start::Running => {
//!         self.windows.mark_running(id);
//!         return self.watch(id);
//!     }
//!     // A screen qdesk draws itself: the window holds no program, so nothing is watched.
//!     Start::Screen => {}
//!     // The window stays open and tells the person; the error names no language of its own,
//!     // so the desktop writes the notice from its locale files.
//!     Start::Failed(error) => return self.failed(id, &error),
//! }
//! ```
//!
//! **Watching.** One command per waiting watch, started again after every change but the last:
//!
//! ```ignore
//! fn watch(&self, id: WindowId) -> Command<Msg> {
//!     match self.programs.watch(id) {
//!         Some(watch) => Command::perform(move || Msg::Program(watch.next())),
//!         None => Command::none(),
//!     }
//! }
//! ```
//!
//! [`Watch::next`] blocks until the program says something, so it belongs in
//! `Command::perform` and never in `update` or `view`.
//!
//! **Two waits, and which is for which.** [`Watch::next`] is the desktop's: the running
//! application has a thread for it and waits with no bound, and nothing about that changes.
//! [`Watch::next_within`] is the screen tests': a test runs the performed command where it stands,
//! so a live program that has drawn its screen and is waiting to be typed into would never let the
//! test go. A bounded wait comes back, says the program was silent, and the desktop starts the
//! watch again at the next frame — so a test that drives a live program is bounded by frames and
//! never by the clock. [`Desk::watch_within`](crate::app::Desk::watch_within) is where a test says
//! so; without it every wait is the unbounded one.
//!
//! **Hearing.** [`Sessions::accept`] records what the program said and answers `None` for a
//! report of a program that has already been replaced or whose window is gone, so a late report
//! of a restarted window changes nothing:
//!
//! ```ignore
//! Msg::Program(report) => {
//!     let id = report.window();
//!     let Some(change) = self.programs.accept(report) else { return Command::none() };
//!     let ended = matches!(change, Change::Ended { .. });
//!     match change {
//!         // The screen changed; the frame that follows this update draws it.
//!         Change::Output | Change::Folder(_) => {}
//!         Change::Title(title) => { self.windows.set_title(id, title); }
//!         Change::Bell => self.dock.mark(id),
//!         Change::Notify { title, body } => return self.notify(id, title, body),
//!         Change::Ended { code } => match self.windows.mark_ended(id, code) {
//!             Exit::Closed => { self.programs.close(id); }
//!             Exit::Kept | Exit::Unknown => {}
//!         },
//!     }
//!     // Every change but the last is followed by the next wait.
//!     if ended { Command::none() } else { self.watch(id) }
//! }
//! ```
//!
//! **Drawing.** The body of a window whose program runs, or whose program ended and whose last
//! screen is still shown, is [`Sessions::terminal`]; the strip above it shows
//! [`Sessions::subtitle`] after the application's name. The widget tells the pseudo-terminal the
//! size it was drawn at, so a window that is resized resizes the program: nothing here resizes by
//! hand, and nothing here stands in the way of it. Only the first size is this module's, so the
//! program's first drawing already fits.
//!
//! **Closing and leaving.** `×` on a window whose [`Sessions::is_running`] is true asks first
//! (`Sessions::pid` names the program in the dialog for one that will not answer, and
//! [`Sessions::kill`] ends it outright); then [`Sessions::close`] gives the program its grace and
//! the window closes. Leaving qdesk asks when [`Sessions::running`] is not zero.

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use qframe::geometry::Size;
use qframe::widgets::{Terminal, TerminalChange, TerminalSession, TerminalWatch};

use crate::apps::{Entry, Environment, Launch, Screen};
use crate::settings::{Prefs, frame_cap};
use crate::wm::WindowId;

/// How long a program has to end itself after the hangup before it is killed, when nobody says
/// otherwise. A shell saves its history and an editor its swap file in far less; a person who
/// closed a window should not wait longer than this for it to go.
pub const GRACE: Duration = Duration::from_secs(2);

/// The variable a program can look for to know it was opened by the desktop.
const MARK: &str = "QDESK";

/// What a program of a window said, or what became of it.
///
/// The words are the program's own and the numbers the system's: there is no text for the screen
/// here. A [`Title`](Self::Title) or a [`Notify`](Self::Notify) body is whatever the program
/// wrote, however long or empty.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// The program wrote to its screen; the next frame draws it.
    Output,
    /// The program set its own title (OSC 0 or 2), or cleared it (`None`).
    Title(Option<String>),
    /// The program said which folder it is in (OSC 7).
    Folder(PathBuf),
    /// The program rang the bell. Rings the desktop has not read yet count as one.
    Bell,
    /// The program asked for a notification (OSC 9, or OSC 777 with a title).
    Notify {
        /// The title, when the program gave one.
        title: Option<String>,
        /// The message.
        body: String,
    },
    /// The program ended.
    Ended {
        /// The exit code the system gave, `None` when it could not be read. A program ended by a
        /// signal has no code of its own: the pseudo-terminal reports it as `Some(1)`.
        code: Option<i32>,
    },
}

/// One [`Change`] of the program of one window, as a [`Watch`] heard it.
///
/// It carries which program said it, so a report of a program that has been replaced is told
/// apart from one of the program running now; [`Sessions::accept`] does that.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    window: WindowId,
    /// Which program of that window said it.
    run: u64,
    change: Change,
}

impl Report {
    /// The window whose program said it.
    #[must_use]
    pub fn window(&self) -> WindowId {
        self.window
    }

    /// What the program said.
    #[must_use]
    pub fn change(&self) -> &Change {
        &self.change
    }
}

/// A wait for the next [`Report`] of one window's program, made by [`Sessions::watch`].
///
/// It holds no claim on the program: closing the window still ends it, and the watch then reports
/// the end.
pub struct Watch {
    window: WindowId,
    run: u64,
    watch: TerminalWatch,
}

impl std::fmt::Debug for Watch {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Watch").field("window", &self.window).field("run", &self.run).finish_non_exhaustive()
    }
}

impl Watch {
    /// Waits for the program's next change and reports it. **This is the wait qdesk runs.**
    ///
    /// It blocks until the program says something, so it belongs inside
    /// [`Command::perform`](qframe::runtime::Command::perform), never in `update` or `view`.
    /// Output is reported at most once per frame of the desktop's frame cap, however fast the
    /// program writes; what the program says about itself is never held back.
    ///
    /// The running desktop has a thread for this wait and nothing else to do on it, so it waits
    /// with no bound; a test drives the same wait where it stands and takes
    /// [`next_within`](Self::next_within) instead.
    #[must_use]
    pub fn next(&self) -> Report {
        loop {
            if let Some(report) = self.report(self.watch.next_change()) {
                return report;
            }
        }
    }

    /// The same wait, giving up after `bound` with `None` when the program said nothing in that
    /// time. **This is the wait a screen test runs.**
    ///
    /// A screen test performs the desktop's own command where it stands, so the unbounded
    /// [`next`](Self::next) on a live but silent program — one that has drawn its screen and is
    /// waiting to be typed into — would never come back and the test would hang instead of
    /// failing. With a bound the wait ends, `None` says the program had nothing to say, and the
    /// window's watch is simply started again: the test draws another frame and asks again, so
    /// what it waits for is bounded by frames and never by the clock.
    ///
    /// `bound` is a bound, not a pause: a program that speaks is heard the moment it does. One
    /// shorter than the desktop's frame cap can still answer `None` with output already in hand,
    /// because output waits for its frame.
    #[must_use]
    pub fn next_within(&self, bound: Duration) -> Option<Report> {
        let deadline = Instant::now().checked_add(bound)?;
        loop {
            let left = deadline.checked_duration_since(Instant::now())?;
            let change = self.watch.next_change_within(left)?;
            if let Some(report) = self.report(change) {
                return Some(report);
            }
        }
    }

    /// What one change of the framework's terminal means to a window; `None` for a notice this
    /// qdesk knows nothing about, which its caller answers by waiting again.
    fn report(&self, change: TerminalChange) -> Option<Report> {
        let change = match change {
            TerminalChange::Output => Change::Output,
            TerminalChange::Title(title) => Change::Title(Some(title).filter(|title| !title.is_empty())),
            TerminalChange::WorkingFolder(folder) => Change::Folder(folder),
            TerminalChange::Bell => Change::Bell,
            TerminalChange::Notify { title, body } => Change::Notify { title, body },
            TerminalChange::Exited(code) => Change::Ended { code: exit_code(code) },
            // A notice a later framework adds is nothing this qdesk knows what to do with;
            // waiting for the next change again is what ignoring it means.
            _ => return None,
        };
        Some(Report { window: self.window, run: self.run, change })
    }
}

/// What the title strip shows after the application's name, see [`subtitle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Subtitle<'a> {
    /// The title the program gave itself.
    Title(&'a str),
    /// The folder the program is in.
    Folder(&'a Path),
}

/// What a window's title strip shows after the application's name: the title the program gave
/// itself (OSC 0 or 2), else the folder it said it is in (OSC 7), else the folder it was started
/// in.
///
/// A title of nothing but spaces is no title: a program that clears its title leaves the strip
/// showing the folder rather than an empty gap.
#[must_use]
pub fn subtitle<'a>(title: Option<&'a str>, said: Option<&'a Path>, started_in: &'a Path) -> Subtitle<'a> {
    match title.map(str::trim).filter(|title| !title.is_empty()) {
        Some(title) => Subtitle::Title(title),
        None => Subtitle::Folder(said.unwrap_or(started_in)),
    }
}

/// What came of asking for a window's program to be started.
#[derive(Debug)]
pub enum Start {
    /// The program is running: mark the window running and start watching it.
    Running,
    /// The entry names a screen qdesk draws itself, or a file for one of its viewers, so there is
    /// no program to start or watch.
    Screen,
    /// The program could not be started, because it is not there, is not a program, or no
    /// pseudo-terminal could be opened. The window stays open and the desktop says so in the
    /// person's own language.
    Failed(io::Error),
}

/// One window's program: the pseudo-terminal, what it last said, and what started it.
#[derive(Debug)]
struct Program {
    session: TerminalSession,
    /// The entry it was started from, so the window can start it again in place.
    entry: Entry,
    /// Which program of this window it is; a report of an earlier one is stale.
    run: u64,
    started_in: PathBuf,
    title: Option<String>,
    folder: Option<PathBuf>,
}

/// A program of a closed window, kept while its grace runs.
///
/// Dropping the last handle of a session ends its program at once, so a program that was asked
/// politely to go has to be held until it goes; otherwise the grace would be a promise nobody
/// keeps.
#[derive(Debug)]
struct Closing {
    session: TerminalSession,
    /// When the grace is over. By then the program has had its hangup and its kill.
    until: Instant,
}

/// The live programs of the desktop, one for each window that holds one.
///
/// See the [module documentation](self) for how the application drives this.
#[derive(Debug)]
pub struct Sessions {
    /// The shell a Terminal window runs.
    shell: PathBuf,
    /// The folder a program starts in when its entry names none.
    home: Option<PathBuf>,
    scrollback: usize,
    coalesce: Duration,
    grace: Duration,
    programs: BTreeMap<WindowId, Program>,
    closing: Vec<Closing>,
    /// Numbers the programs, so a report of one that has been replaced is recognised.
    runs: u64,
}

impl Sessions {
    /// A desktop with no program running yet.
    ///
    /// `environment` says which shell a Terminal window runs and which folder a program starts in
    /// when its entry names none. `prefs` and `remote` say how many lines a window remembers and how
    /// often output may ask for a frame.
    #[must_use]
    pub fn new(environment: &Environment, prefs: Prefs, remote: bool) -> Self {
        Self {
            shell: environment.terminal_shell(),
            home: environment.home.clone(),
            scrollback: usize::from(prefs.scrollback),
            coalesce: coalesce(prefs, remote),
            grace: GRACE,
            programs: BTreeMap::new(),
            closing: Vec::new(),
            runs: 0,
        }
    }

    /// The same, giving a closing program a different grace than [`GRACE`].
    #[must_use]
    pub fn grace(mut self, grace: Duration) -> Self {
        self.grace = grace;
        self
    }

    /// Takes in settings the person changed.
    ///
    /// It reaches the programs started from now on: a running pseudo-terminal keeps the
    /// scrollback it was opened with, because the lines it has already forgotten cannot be found
    /// again, and keeps the frame it reports output at, because that is how its own watch waits.
    pub fn set_prefs(&mut self, prefs: Prefs, remote: bool) {
        self.scrollback = usize::from(prefs.scrollback);
        self.coalesce = coalesce(prefs, remote);
    }

    /// Starts the program of `entry` for the window `window`, on a screen of `body` cells.
    ///
    /// `body` is the size the window's body is drawn at, so the program's first drawing already
    /// fits; the widget takes over from there. The program gets `TERM=xterm-256color`,
    /// `COLORTERM=truecolor` and `QDESK=1`, then the entry's own variables, which may replace any
    /// of them. It starts in the entry's folder, else the home folder, and remembers as many lines
    /// as the settings allow.
    ///
    /// A window that already has a program keeps none: the old one is closed as
    /// [`close`](Self::close) closes it and its later reports become stale.
    pub fn start(&mut self, window: WindowId, entry: &Entry, body: Size) -> Start {
        self.close(window);
        let (program, args) = match &entry.launch {
            Launch::Command(words) => match words.split_first() {
                Some((program, args)) => (OsString::from(program), args.to_vec()),
                // The entry reader keeps no empty command; a file that held one declared nothing.
                None => return Start::Failed(io::Error::from(io::ErrorKind::InvalidInput)),
            },
            Launch::Screen(Screen::Terminal) => (OsString::from(&self.shell), Vec::new()),
            Launch::Screen(Screen::Settings | Screen::Files) | Launch::Open(_) => return Start::Screen,
        };
        let started_in = self.folder_for(entry);
        let mut builder = TerminalSession::builder(program)
            .args(args)
            .folder(&started_in)
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .env(MARK, "1")
            .size(body.width, body.height)
            .scrollback(self.scrollback)
            .coalesce(self.coalesce);
        for (name, value) in &entry.env {
            builder = builder.env(name, value);
        }
        match builder.spawn() {
            Ok(session) => {
                self.runs += 1;
                let program =
                    Program { session, entry: entry.clone(), run: self.runs, started_in, title: None, folder: None };
                self.programs.insert(window, program);
                Start::Running
            }
            Err(error) => Start::Failed(error),
        }
    }

    /// Starts the program of the window `window` again, from the entry it was opened with, on a
    /// screen of `body` cells. `None` when that window has no program to start again.
    ///
    /// This is the Restart of a window whose program ended. A program still running is closed
    /// first, as [`close`](Self::close) closes it.
    pub fn restart(&mut self, window: WindowId, body: Size) -> Option<Start> {
        let entry = self.programs.get(&window).map(|program| program.entry.clone())?;
        Some(self.start(window, &entry, body))
    }

    /// A wait for the next change of the program of `window`; `None` when it has none.
    ///
    /// Start a new one after every change but [`Change::Ended`]; a watch reports one change and
    /// is done with.
    #[must_use]
    pub fn watch(&self, window: WindowId) -> Option<Watch> {
        let program = self.programs.get(&window)?;
        Some(Watch { window, run: program.run, watch: program.session.watch() })
    }

    /// Takes in a report: records what the program said and gives back the change, or `None` when
    /// the report is stale — its window has no program any more, or the program that said it has
    /// been replaced by a later one.
    pub fn accept(&mut self, report: Report) -> Option<Change> {
        self.sweep();
        let program = self.programs.get_mut(&report.window)?;
        if program.run != report.run {
            return None;
        }
        match &report.change {
            Change::Title(title) => program.title = title.clone(),
            Change::Folder(folder) => program.folder = Some(folder.clone()),
            Change::Output | Change::Bell | Change::Notify { .. } | Change::Ended { .. } => {}
        }
        Some(report.change)
    }

    /// The widget for the body of `window`; `None` when it holds no program.
    ///
    /// It draws the program's screen and types into it, and tells the pseudo-terminal the size it
    /// was drawn at, so resizing the window resizes the program without anyone asking. The session
    /// stays alive for as long as its window does, because it is what holds the screen and the
    /// scrollback a window whose program has ended is kept open for.
    #[must_use]
    pub fn terminal(&self, window: WindowId) -> Option<Terminal> {
        self.programs.get(&window).map(|program| Terminal::new(&program.session))
    }

    /// What the title strip of `window` shows after the application's name; `None` when the window
    /// holds no program. The rule is [`subtitle`].
    #[must_use]
    pub fn subtitle(&self, window: WindowId) -> Option<Subtitle<'_>> {
        self.programs
            .get(&window)
            .map(|program| subtitle(program.title.as_deref(), program.folder.as_deref(), &program.started_in))
    }

    /// The folder each window's program is in, window by window: the one its shell last said, else
    /// the one it started in. A window whose program has ended keeps the folder it ended in, since
    /// the window still stands there until it is closed.
    pub fn folders(&self) -> impl Iterator<Item = (WindowId, &Path)> {
        self.programs.iter().map(|(id, program)| (*id, program.folder.as_deref().unwrap_or(&program.started_in)))
    }

    /// The process id of the program of `window`, for the dialog about a program that will not
    /// answer; `None` when the window holds no program or the system gave no id.
    #[must_use]
    pub fn pid(&self, window: WindowId) -> Option<u32> {
        self.programs.get(&window).and_then(|program| program.session.pid())
    }

    /// Whether the program of `window` is still going. A window whose program ended, and one that
    /// holds no program, closes without asking.
    #[must_use]
    pub fn is_running(&self, window: WindowId) -> bool {
        self.programs.get(&window).is_some_and(|program| program.session.exit().is_none())
    }

    /// How the program of `window` ended: its exit code, `None` when it could not be read. The
    /// outer `None` means it has not ended, or the window holds no program.
    #[must_use]
    pub fn exit(&self, window: WindowId) -> Option<Option<i32>> {
        self.programs.get(&window).and_then(|program| program.session.exit()).map(exit_code)
    }

    /// How many programs are still going, for the question leaving qdesk asks. Programs of closed
    /// windows are not counted: their windows are gone and nobody waits for them.
    #[must_use]
    pub fn running(&self) -> usize {
        self.programs.values().filter(|program| program.session.exit().is_none()).count()
    }

    /// How many closed windows' programs are still being given their grace.
    #[must_use]
    pub fn closing(&self) -> usize {
        self.closing.iter().filter(|closing| closing.session.exit().is_none()).count()
    }

    /// Ends the program of `window` politely and forgets it; `false` when it had none.
    ///
    /// The program is sent the hangup a closing terminal window sends, and killed if it has not
    /// gone when the grace is over. It is held until then, so the grace is real; after this its
    /// reports are stale.
    pub fn close(&mut self, window: WindowId) -> bool {
        self.sweep();
        let Some(program) = self.programs.remove(&window) else { return false };
        if program.session.exit().is_none() {
            program.session.terminate(self.grace);
            self.closing.push(Closing { session: program.session, until: Instant::now() + self.grace });
        }
        true
    }

    /// Ends the program of `window` outright, for one that did not answer the polite ask;
    /// `false` when the window has no program. The window stays and its watch reports the end.
    ///
    /// This is the same ending as [`close`](Self::close) with no grace at all: the hangup, then
    /// the kill, to the program and to whatever it started in the foreground. A program that
    /// catches the hangup and stays is the reason this exists, so nothing here waits for it to
    /// think it over.
    pub fn kill(&self, window: WindowId) -> bool {
        let Some(program) = self.programs.get(&window) else { return false };
        program.session.terminate(Duration::ZERO);
        true
    }

    /// The folder a program of `entry` starts in: the entry's own, else the home folder, else the
    /// folder qdesk itself was started in. A folder that is not there is passed over, so the
    /// folder kept here is the one the program really got.
    fn folder_for(&self, entry: &Entry) -> PathBuf {
        entry
            .folder
            .iter()
            .chain(self.home.iter())
            .find(|folder| folder.is_dir())
            .cloned()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
    }

    /// Lets go of the closing programs that have gone, and of those whose grace is over: by then
    /// the session has sent the kill itself, so letting go only closes the pseudo-terminal.
    fn sweep(&mut self) {
        let now = Instant::now();
        self.closing.retain(|closing| closing.session.exit().is_none() && now < closing.until);
    }
}

/// The shortest time between two reports of output: one frame of the cap in force. Output that
/// arrives faster than that asks for one frame per interval instead of one per read, which is what
/// keeps a program writing fast from spending a slow link on frames nobody can see.
fn coalesce(prefs: Prefs, remote: bool) -> Duration {
    let frames = u32::from(frame_cap(prefs.frame_cap, remote)).max(1);
    Duration::from_secs(1) / frames
}

/// The exit code as the window list counts them. The pseudo-terminal reports codes the system
/// gave, which do not go past a byte; anything else, and a code that could not be read, is `None`.
fn exit_code(code: Option<u32>) -> Option<i32> {
    code.and_then(|code| i32::try_from(code).ok())
}
