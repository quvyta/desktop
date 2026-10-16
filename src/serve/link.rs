//! The screen's side of the socket: the connection a screen draws its programs through.
//!
//! A screen attached to the background starts no program itself. It asks the background to
//! start one ([`Link::open`]) and draws it through a framework terminal session
//! [attached](TerminalSession::attach) to it: what the program writes comes in on the socket and
//! is handed to that session, and the keys, the size and the request to end it go back the same
//! way. The session draws, selects, scrolls back and reports titles and bells exactly as one that
//! runs its program here; the program's questions to the terminal are answered by its session in
//! the background, which saw them first.
//!
//! One thread reads the socket and hands each program's bytes to its session; the end of the
//! connection ends every session, as an ended program does, since none of them can be reached.

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::thread;
use std::time::{Duration, Instant};

use qframe::widgets::{TerminalCarrier, TerminalSession};

use super::wire::{self, Choice, Greeting, Listed, Message, Record, Spec};

/// How long a screen waits for the background to answer a request. The background answers in
/// milliseconds; this only bounds the wait for one that stopped answering.
const PATIENCE: Duration = Duration::from_secs(30);

/// How long a screen waits for a background it started to listen. It listens within
/// milliseconds; a loaded machine only makes it slower.
const STARTING: Duration = Duration::from_secs(20);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the background said a program ended with, read by the program's session once its output
/// ends.
type Exit = Arc<Mutex<Option<u32>>>;

/// Where one program's output goes on the screen's side.
struct Route {
    output: Sender<Vec<u8>>,
    exit: Exit,
}

/// One program's output as its session reads it: what the socket brought, in order, and the end
/// once the program ended or the connection did.
struct Pipe {
    pieces: Receiver<Vec<u8>>,
    held: Vec<u8>,
    at: usize,
}

impl Read for Pipe {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        while self.at >= self.held.len() {
            match self.pieces.recv() {
                Ok(piece) => {
                    self.held = piece;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let count = buffer.len().min(self.held.len() - self.at);
        buffer[..count].copy_from_slice(&self.held[self.at..self.at + count]);
        self.at += count;
        Ok(count)
    }
}

/// What reaches one program in the background: its number on the link.
struct Carrier {
    link: Link,
    program: u64,
    exit: Exit,
}

impl TerminalCarrier for Carrier {
    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.link.send(&Message::Input { program: self.program, bytes: bytes.to_vec() })
    }

    fn resize(&mut self, columns: u16, rows: u16) -> io::Result<()> {
        self.link.send(&Message::Resize { program: self.program, columns, rows })
    }

    fn end(&mut self) {
        let _ = self.link.send(&Message::Kill { program: self.program });
    }

    fn exit_code(&mut self) -> Option<u32> {
        *lock(&self.exit)
    }
}

/// An answer to a request, as the reading thread hands it on.
enum Reply {
    Opened { request: u32, program: u64, pid: Option<u32>, output: Pipe, exit: Exit },
    Refused { request: u32, error: io::Error },
    Attached { desktop: u32, locked: bool, programs: Vec<(Record, Pipe, Exit)> },
    Desktops(Vec<Listed>),
}

struct Inner {
    writer: Mutex<UnixStream>,
    routes: Mutex<BTreeMap<u64, Route>>,
    replies: Mutex<Receiver<Reply>>,
    requests: AtomicU32,
    /// The connection ended: the background went away.
    lost: AtomicBool,
}

impl Drop for Inner {
    /// The screen let go of its last handle: the connection is closed, so the background hears
    /// the screen left and the reading thread hears the end.
    fn drop(&mut self) {
        let _ = lock(&self.writer).shutdown(std::net::Shutdown::Both);
    }
}

/// A screen's connection to the background. Clones share it.
#[derive(Clone)]
pub struct Link {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Link {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Link").field("lost", &self.is_lost()).finish_non_exhaustive()
    }
}

/// Why a screen could not reach the background.
#[derive(Debug)]
pub enum Unreached {
    /// A background of another qdesk answers on the socket: it speaks another protocol, named by
    /// its version. It may hold programs, and those are the person's, so it is left as it is.
    Mismatch {
        /// The version of the qdesk that runs it.
        version: String,
    },
    /// None answered and none could be started, or it answered with something else.
    Failed(io::Error),
}

impl From<io::Error> for Unreached {
    fn from(error: io::Error) -> Self {
        Self::Failed(error)
    }
}

/// A program of the desktop a screen attached to: what the background said about it and the
/// session that draws it.
#[derive(Debug)]
pub struct Kept {
    /// What the background said about it.
    pub record: Record,
    /// The session that draws it, already given its screen as it was.
    pub session: TerminalSession,
}

/// The desktop a screen attached to.
#[derive(Debug)]
pub struct Attachment {
    /// The background's number for it.
    pub desktop: u32,
    /// Whether it was left locked.
    pub locked: bool,
    /// Its programs, oldest first.
    pub programs: Vec<Kept>,
}

/// A program the background started for a window.
#[derive(Debug)]
pub struct Opened {
    /// The background's number for it.
    pub program: u64,
    /// Its process id, when the system gave one.
    pub pid: Option<u32>,
    /// The session that draws it.
    pub session: TerminalSession,
}

impl Link {
    /// Connects to the background on `path` and says hello.
    ///
    /// # Errors
    ///
    /// [`Unreached::Mismatch`] when it speaks another protocol, [`Unreached::Failed`] when nothing
    /// answers on `path` or the answer is not a welcome.
    pub fn connect(path: &Path) -> Result<Self, Unreached> {
        let mut stream = UnixStream::connect(path)?;
        match wire::hello(&mut stream)? {
            Greeting::Agreed => {}
            Greeting::Mismatch { version, .. } => return Err(Unreached::Mismatch { version }),
        }
        let reading = stream.try_clone()?;
        let (replies, answers) = mpsc::channel();
        let link = Self {
            inner: Arc::new(Inner {
                writer: Mutex::new(stream),
                routes: Mutex::new(BTreeMap::new()),
                replies: Mutex::new(answers),
                requests: AtomicU32::new(0),
                lost: AtomicBool::new(false),
            }),
        };
        // Weak, so the reading does not keep the connection open once the screen let it go.
        let inner = Arc::downgrade(&link.inner);
        thread::spawn(move || read(&inner, reading, &replies));
        Ok(link)
    }

    /// Connects to the background on `path`, starting one with `qdesk serve` when none answers.
    ///
    /// `program` is the qdesk to start it with, this one's own file. The background leaves this
    /// screen's session as it starts, so a hangup of this terminal does not reach it.
    ///
    /// # Errors
    ///
    /// As [`connect`](Self::connect), and when the background cannot be started or does not
    /// listen within a generous while.
    pub fn reach(path: &Path, program: &Path) -> Result<Self, Unreached> {
        match Self::connect(path) {
            Ok(link) => return Ok(link),
            Err(Unreached::Mismatch { version }) => return Err(Unreached::Mismatch { version }),
            Err(Unreached::Failed(_)) => {}
        }
        let mut child = Command::new(program)
            .arg("serve")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        // It outlives this screen; while the screen is here it is waited for, so it never lingers
        // as a process that ended and was not asked how.
        thread::spawn(move || child.wait());
        let start = Instant::now();
        loop {
            match Self::connect(path) {
                Ok(link) => return Ok(link),
                Err(Unreached::Mismatch { version }) => return Err(Unreached::Mismatch { version }),
                Err(Unreached::Failed(error)) if start.elapsed() >= STARTING => return Err(Unreached::Failed(error)),
                Err(Unreached::Failed(_)) => thread::sleep(Duration::from_millis(20)),
            }
        }
    }

    /// Whether the connection has ended: the background is gone and nothing sent arrives.
    #[must_use]
    pub fn is_lost(&self) -> bool {
        self.inner.lost.load(Ordering::SeqCst)
    }

    /// Sends `message` to the background.
    ///
    /// # Errors
    ///
    /// When the connection has ended.
    pub fn send(&self, message: &Message) -> io::Result<()> {
        wire::send(&mut *lock(&self.inner.writer), message)
    }

    /// Waits for the answer `wanted` picks out of the replies to come.
    fn answer<T>(&self, mut wanted: impl FnMut(Reply) -> Option<T>) -> io::Result<T> {
        let replies = lock(&self.inner.replies);
        let start = Instant::now();
        loop {
            let left = PATIENCE.saturating_sub(start.elapsed());
            match replies.recv_timeout(left) {
                Ok(reply) => {
                    if let Some(found) = wanted(reply) {
                        return Ok(found);
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    return Err(io::Error::new(io::ErrorKind::TimedOut, "the background did not answer"));
                }
                Err(RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::new(io::ErrorKind::BrokenPipe, "the background went away"));
                }
            }
        }
    }

    /// Attaches to a desktop of the background as `choice` says. Each program's session reports
    /// output at most once per `coalesce`.
    ///
    /// # Errors
    ///
    /// When the background cannot be asked or does not answer.
    pub fn attach(&self, choice: Choice, coalesce: Duration) -> io::Result<Attachment> {
        self.send(&Message::Attach(choice))?;
        let (desktop, locked, programs) = self.answer(|reply| match reply {
            Reply::Attached { desktop, locked, programs } => Some((desktop, locked, programs)),
            _ => None,
        })?;
        let programs = programs
            .into_iter()
            .map(|(record, output, exit)| {
                let session = self.session(
                    record.program,
                    output,
                    exit,
                    (record.columns, record.rows),
                    record.scrollback,
                    coalesce,
                );
                Kept { record, session }
            })
            .collect();
        Ok(Attachment { desktop, locked, programs })
    }

    /// Asks the background to start the program `spec` describes, for a window the screen
    /// describes as `window`.
    ///
    /// # Errors
    ///
    /// What kept the background from starting it, as the system said it, or why the background
    /// could not be asked.
    pub fn open(&self, spec: &Spec, window: Vec<u8>, coalesce: Duration) -> io::Result<Opened> {
        let request = self.inner.requests.fetch_add(1, Ordering::SeqCst);
        self.send(&Message::Open { request, spec: spec.clone(), window })?;
        let (program, pid, output, exit) = self.answer(|reply| match reply {
            Reply::Opened { request: answered, program, pid, output, exit } if answered == request => {
                Some(Ok((program, pid, output, exit)))
            }
            Reply::Refused { request: answered, error } if answered == request => Some(Err(error)),
            _ => None,
        })??;
        let session = self.session(program, output, exit, (spec.columns, spec.rows), spec.scrollback, coalesce);
        Ok(Opened { program, pid, session })
    }

    /// The desktops the background holds.
    ///
    /// # Errors
    ///
    /// When the background cannot be asked or does not answer.
    pub fn list(&self) -> io::Result<Vec<Listed>> {
        self.send(&Message::List)?;
        self.answer(|reply| match reply {
            Reply::Desktops(desktops) => Some(desktops),
            _ => None,
        })
    }

    fn session(
        &self,
        program: u64,
        output: Pipe,
        exit: Exit,
        (columns, rows): (u16, u16),
        scrollback: u32,
        coalesce: Duration,
    ) -> TerminalSession {
        let carrier = Carrier { link: self.clone(), program, exit };
        TerminalSession::attach(output, carrier)
            .size(columns, rows)
            .scrollback(scrollback as usize)
            .coalesce(coalesce)
            .start()
    }
}

/// Reads the socket until it ends, handing each program's bytes to its session and each answer to
/// whoever waits for it. The routes are made here, as the answers that name programs are read, so
/// no output can arrive for a program the screen has no route for yet.
fn read(link: &Weak<Inner>, mut reading: UnixStream, replies: &Sender<Reply>) {
    let route = |inner: &Inner, program: u64| {
        let (output, pieces) = mpsc::channel();
        let exit = Exit::default();
        lock(&inner.routes).insert(program, Route { output, exit: Arc::clone(&exit) });
        (Pipe { pieces, held: Vec::new(), at: 0 }, exit)
    };
    while let Ok(Some(message)) = wire::receive(&mut reading) {
        let Some(inner) = link.upgrade() else { return };
        let reply = match message {
            Message::Output { program, bytes } => {
                if let Some(route) = lock(&inner.routes).get(&program) {
                    let _ = route.output.send(bytes);
                }
                continue;
            }
            Message::Exited { program, code } => {
                // Letting the route go ends the program's output, after everything it wrote.
                if let Some(route) = lock(&inner.routes).remove(&program) {
                    *lock(&route.exit) = code;
                }
                continue;
            }
            Message::Opened { request, program, pid } => {
                let (output, exit) = route(&inner, program);
                Reply::Opened { request, program, pid, output, exit }
            }
            Message::Refused { request, kind, reason } => {
                Reply::Refused { request, error: io::Error::new(wire::kind_of(kind), reason) }
            }
            Message::Attached { desktop, locked, programs } => {
                let programs = programs
                    .into_iter()
                    .map(|record| {
                        let (output, exit) = route(&inner, record.program);
                        (record, output, exit)
                    })
                    .collect();
                Reply::Attached { desktop, locked, programs }
            }
            Message::Desktops(desktops) => Reply::Desktops(desktops),
            _ => continue,
        };
        if replies.send(reply).is_err() {
            break;
        }
    }
    if let Some(inner) = link.upgrade() {
        inner.lost.store(true, Ordering::SeqCst);
        // Every program's output ends with the connection: none of them can be reached.
        lock(&inner.routes).clear();
    }
}
