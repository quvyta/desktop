//! What the background holds: desktops, each with the programs of its windows, and the screen
//! attached to it, if one is.
//!
//! A program runs in a pseudo-terminal the background opened, and its session keeps its screen and
//! scrollback whether a screen is attached or not. While one is, every byte the program writes is
//! also relayed to it as it is read; a screen that attaches is first sent the session's
//! [snapshot](TerminalSession::snapshot), so it draws what the program had drawn before it goes on
//! with what follows.
//!
//! Locks are taken in one order only: the state, then a program's relay. The thread that reads a
//! program's output takes only the relay, and the thread that watches it takes the relay and lets
//! go before it takes the state.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::Duration;

use qframe::widgets::{TerminalChange, TerminalSession};

use super::wire::{self, Choice, Listed, Message, PIECE, Record, Spec};

/// How often a program's watch here is woken for output. The background reads no output itself,
/// only the title, the folder and the end, which are never held back, so a long pace only spares
/// waking up for nothing.
const PACE: Duration = Duration::from_secs(1);

/// How many bytes may wait for a screen that does not read them. A screen that stopped reading,
/// one suspended by its terminal, say, is let go past this: it would never catch up, and holding
/// more would only take the machine's memory. It finds its programs again when it comes back.
const BEHIND: usize = 64 << 20;

/// What a background's loop hears from the programs, so it knows when it is alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Event {
    /// A screen of the background's own user connected.
    Arrived,
    /// A screen went away: it closed, or said something the background could not read.
    Left,
    /// A program ended.
    Ended,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The messages on their way to one screen, written by a thread of its own so a program that
/// writes fast never waits for the screen to read.
#[derive(Clone)]
pub(super) struct Outbox {
    frames: Sender<Vec<u8>>,
    waiting: Arc<AtomicUsize>,
    stream: Arc<UnixStream>,
}

impl Outbox {
    /// Starts writing to `stream`.
    pub(super) fn start(stream: UnixStream) -> std::io::Result<Self> {
        let (frames, queued) = mpsc::channel::<Vec<u8>>();
        let waiting = Arc::new(AtomicUsize::new(0));
        let mut writer = stream.try_clone()?;
        let written = Arc::clone(&waiting);
        thread::spawn(move || {
            for frame in queued {
                written.fetch_sub(frame.len(), Ordering::SeqCst);
                if writer.write_all(&frame).is_err() {
                    break;
                }
            }
        });
        Ok(Self { frames, waiting, stream: Arc::new(stream) })
    }

    /// Queues `message`. A screen too far behind is let go: its connection is shut, so it reads
    /// to the end and leaves.
    pub(super) fn send(&self, message: &Message) {
        let frame = message.frame();
        let waiting = self.waiting.fetch_add(frame.len(), Ordering::SeqCst) + frame.len();
        if waiting > BEHIND {
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
            return;
        }
        if self.frames.send(frame).is_err() {
            let _ = self.stream.shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Where a program's output goes besides its own session: the screen attached to its desktop.
#[derive(Default)]
struct Relay {
    to: Option<Outbox>,
}

impl Relay {
    fn output(&self, program: u64, bytes: &[u8]) {
        if let Some(to) = &self.to {
            for piece in bytes.chunks(PIECE) {
                to.send(&Message::Output { program, bytes: piece.to_vec() });
            }
        }
    }
}

/// A program the background holds.
struct Held {
    session: TerminalSession,
    relay: Arc<Mutex<Relay>>,
    scrollback: u32,
    pid: Option<u32>,
    title: Option<String>,
    folder: Option<PathBuf>,
    /// What the screen said about the program's window, kept for the next screen.
    window: Vec<u8>,
    /// Its window closed: it is being ended, shown to no screen, and forgotten once it has gone.
    closing: bool,
}

impl Held {
    fn record(&self, program: u64) -> Record {
        let (columns, rows) = self.session.size();
        Record {
            program,
            columns,
            rows,
            scrollback: self.scrollback,
            pid: self.pid,
            title: self.title.clone(),
            folder: self.folder.clone(),
            exit: self.session.exit(),
            window: self.window.clone(),
        }
    }
}

/// One desktop: what a screen shows, and what is left running when it goes.
#[derive(Default)]
struct Desktop {
    programs: BTreeMap<u64, Held>,
    /// The connection of the screen attached to it.
    screen: Option<u64>,
    locked: bool,
    /// The person left it: it is ending, and no screen attaches to it again.
    finished: bool,
}

impl Desktop {
    /// The programs a screen is shown: those whose windows are still open.
    fn shown(&self) -> impl Iterator<Item = (&u64, &Held)> {
        self.programs.iter().filter(|(_, held)| !held.closing)
    }

    fn listed(&self, desktop: u32) -> Listed {
        let count = |running: bool| {
            let shown = self.shown().filter(|(_, held)| !running || held.session.exit().is_none()).count();
            u32::try_from(shown).unwrap_or(u32::MAX)
        };
        Listed { desktop, programs: count(false), running: count(true), attached: self.screen.is_some() }
    }
}

#[derive(Default)]
struct State {
    desktops: BTreeMap<u32, Desktop>,
    next_desktop: u32,
    next_program: u64,
}

impl State {
    /// Forgets a desktop nobody shows and nothing is left of, and a finished one once its
    /// programs have gone.
    fn tidy(&mut self, desktop: u32) {
        if let Some(desk) = self.desktops.get(&desktop)
            && desk.screen.is_none()
            && desk.programs.is_empty()
        {
            self.desktops.remove(&desktop);
        }
    }
}

/// The background's desktops and programs.
pub(super) struct Background {
    state: Mutex<State>,
    events: Sender<Event>,
}

impl Background {
    /// A background holding nothing yet, telling `events` when a program ends.
    pub(super) fn new(events: Sender<Event>) -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(State::default()), events })
    }

    /// How many programs still run, on every desktop, those whose windows closed and are being
    /// ended included.
    pub(super) fn running(&self) -> usize {
        let state = lock(&self.state);
        state
            .desktops
            .values()
            .flat_map(|desk| desk.programs.values())
            .filter(|held| held.session.exit().is_none())
            .count()
    }

    /// The desktops, for `qdesk --list`.
    pub(super) fn list(&self) -> Vec<Listed> {
        let state = lock(&self.state);
        state.desktops.iter().filter(|(_, desk)| !desk.finished).map(|(desktop, desk)| desk.listed(*desktop)).collect()
    }

    /// Attaches the screen of connection `screen` to a desktop as `choice` says, and sends it the
    /// desktop and the screens of its programs; the number of the desktop it is attached to.
    pub(super) fn attach(&self, choice: Choice, screen: u64, outbox: &Outbox) -> u32 {
        let mut state = lock(&self.state);
        let free = state
            .desktops
            .iter()
            .find(|(_, desk)| desk.screen.is_none() && !desk.finished)
            .map(|(desktop, _)| *desktop);
        let desktop = match (choice, free) {
            (Choice::Free, Some(desktop)) => desktop,
            _ => {
                state.next_desktop += 1;
                let desktop = state.next_desktop;
                state.desktops.insert(desktop, Desktop::default());
                desktop
            }
        };
        let desk = state.desktops.entry(desktop).or_default();
        desk.screen = Some(screen);
        // Every relay is held while the desktop is described and the screens are taken, so no
        // program's output is relayed before its screen and none falls between the two.
        let mut relays = Vec::new();
        let mut records = Vec::new();
        for (program, held) in desk.shown() {
            let relay = lock(&held.relay);
            records.push(held.record(*program));
            relays.push((*program, held.session.snapshot(), held.session.exit(), relay));
        }
        outbox.send(&Message::Attached { desktop, locked: desk.locked, programs: records });
        for (program, snapshot, exit, mut relay) in relays {
            for piece in snapshot.chunks(PIECE) {
                outbox.send(&Message::Output { program, bytes: piece.to_vec() });
            }
            match exit {
                Some(code) => outbox.send(&Message::Exited { program, code }),
                None => relay.to = Some(outbox.clone()),
            }
        }
        desktop
    }

    /// The screen of connection `screen` left `desktop`: its programs go on for nobody, and a
    /// desktop with nothing left is forgotten.
    pub(super) fn leave(&self, desktop: u32, screen: u64) {
        let mut state = lock(&self.state);
        if let Some(desk) = state.desktops.get_mut(&desktop)
            && desk.screen == Some(screen)
        {
            desk.screen = None;
            for held in desk.programs.values() {
                lock(&held.relay).to = None;
            }
        }
        state.tidy(desktop);
    }

    /// Starts a program for a window of `desktop`, answering the screen on `outbox`.
    pub(super) fn open(self: &Arc<Self>, desktop: u32, request: u32, spec: &Spec, window: Vec<u8>, outbox: &Outbox) {
        let mut state = lock(&self.state);
        state.next_program += 1;
        let program = state.next_program;
        let Some(desk) = state.desktops.get_mut(&desktop) else { return };
        let relay = Arc::new(Mutex::new(Relay::default()));
        // Held until the screen has been told the program runs, so its first output cannot reach
        // the screen before the program's number does.
        let mut relaying = lock(&relay);
        let teed = Arc::clone(&relay);
        let mut builder = TerminalSession::builder(&spec.program)
            .args(&spec.args)
            .folder(&spec.folder)
            .size(spec.columns, spec.rows)
            .scrollback(spec.scrollback as usize)
            .coalesce(PACE)
            .tee(move |bytes| lock(&teed).output(program, bytes));
        // The program gets the screen's environment, not the background's: the background was
        // started by an earlier screen, perhaps over a connection long gone.
        let given: Vec<&OsString> = spec.env.iter().map(|(name, _)| name).collect();
        for (name, _) in std::env::vars_os() {
            if !given.contains(&&name) {
                builder = builder.env_remove(name);
            }
        }
        for (name, value) in &spec.env {
            builder = builder.env(name, value);
        }
        match builder.spawn() {
            Ok(session) => {
                let pid = session.pid();
                outbox.send(&Message::Opened { request, program, pid });
                relaying.to = Some(outbox.clone());
                drop(relaying);
                let watch = session.watch();
                desk.programs.insert(
                    program,
                    Held {
                        session,
                        relay: Arc::clone(&relay),
                        scrollback: spec.scrollback,
                        pid,
                        title: None,
                        folder: None,
                        window,
                        closing: false,
                    },
                );
                let background = Arc::downgrade(self);
                thread::spawn(move || listen(&background, desktop, program, &watch, &relay));
            }
            Err(error) => outbox.send(&Message::Refused {
                request,
                kind: wire::kind_code(error.kind()),
                reason: error.to_string(),
            }),
        }
    }

    /// Does `act` to a program of `desktop`, if it has one by that number.
    fn with(&self, desktop: u32, program: u64, act: impl FnOnce(&mut Held)) {
        let mut state = lock(&self.state);
        if let Some(held) = state.desktops.get_mut(&desktop).and_then(|desk| desk.programs.get_mut(&program)) {
            act(held);
        }
    }

    /// Keys, a paste or a report for a program.
    pub(super) fn input(&self, desktop: u32, program: u64, bytes: &[u8]) {
        self.with(desktop, program, |held| {
            // A program that has ended reads nothing; there is no one to tell.
            let _ = held.session.write(bytes);
        });
    }

    /// A program's window was resized.
    pub(super) fn resize(&self, desktop: u32, program: u64, columns: u16, rows: u16) {
        self.with(desktop, program, |held| held.session.resize(columns, rows));
    }

    /// Ends a program outright; its window stays.
    pub(super) fn kill(&self, desktop: u32, program: u64) {
        self.with(desktop, program, |held| held.session.terminate(Duration::ZERO));
    }

    /// What the screen says about a program's window now.
    pub(super) fn place(&self, desktop: u32, program: u64, window: Vec<u8>) {
        self.with(desktop, program, |held| held.window = window);
    }

    /// A program's window closed: the program is given `grace` to go and forgotten once it has.
    pub(super) fn close(&self, desktop: u32, program: u64, grace: Duration) {
        let mut state = lock(&self.state);
        let Some(desk) = state.desktops.get_mut(&desktop) else { return };
        let Some(held) = desk.programs.get_mut(&program) else { return };
        lock(&held.relay).to = None;
        if held.session.exit().is_some() {
            desk.programs.remove(&program);
        } else {
            held.closing = true;
            held.session.terminate(grace);
        }
    }

    /// The screen of `desktop` was locked or unlocked.
    pub(super) fn lock(&self, desktop: u32, locked: bool) {
        if let Some(desk) = lock(&self.state).desktops.get_mut(&desktop) {
            desk.locked = locked;
        }
    }

    /// The person left `desktop`: every program of it is ended as its window closing would end
    /// it, and the desktop goes once they have.
    pub(super) fn finish(&self, desktop: u32, grace: Duration) {
        let programs: Vec<u64> = {
            let mut state = lock(&self.state);
            let Some(desk) = state.desktops.get_mut(&desktop) else { return };
            desk.finished = true;
            desk.programs.keys().copied().collect()
        };
        for program in programs {
            self.close(desktop, program, grace);
        }
    }

    /// A program said its title or its folder, or ended.
    fn heard(&self, desktop: u32, program: u64, change: TerminalChange) {
        let mut state = lock(&self.state);
        let Some(desk) = state.desktops.get_mut(&desktop) else { return };
        let Some(held) = desk.programs.get_mut(&program) else { return };
        match change {
            TerminalChange::Title(title) => held.title = Some(title).filter(|title| !title.is_empty()),
            TerminalChange::WorkingFolder(folder) => held.folder = Some(folder),
            TerminalChange::Exited(_) => {
                if held.closing {
                    desk.programs.remove(&program);
                    state.tidy(desktop);
                }
                drop(state);
                let _ = self.events.send(Event::Ended);
            }
            _ => {}
        }
    }
}

/// Listens to one program until it ends: what it says about itself is kept for the next screen,
/// and its end is told to the screen after the last of its output.
fn listen(
    background: &Weak<Background>,
    desktop: u32,
    program: u64,
    watch: &qframe::widgets::TerminalWatch,
    relay: &Mutex<Relay>,
) {
    loop {
        let change = watch.next_change();
        let ended = matches!(change, TerminalChange::Exited(_));
        if let TerminalChange::Exited(code) = change
            && let Some(to) = &lock(relay).to
        {
            to.send(&Message::Exited { program, code });
        }
        let Some(background) = background.upgrade() else { return };
        background.heard(desktop, program, change);
        if ended {
            return;
        }
    }
}
