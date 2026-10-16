//! What the screen and the background say to each other: messages of a few bytes, each after its
//! length, so a reader always knows where one ends.
//!
//! A frame is the length of what follows as four bytes (big end first), one byte that says what
//! the message is, and its body. The first thing a screen says is [`Message::Hello`] with the
//! protocol it speaks; the background answers [`Message::Welcome`] with its own and its version.
//! When the two protocols differ the screen says so to the person and leaves the background as it
//! is: it may be holding programs started by an older qdesk, and those are the person's.
//!
//! After the welcome a screen either asks which desktops the background holds
//! ([`Message::List`]) or attaches to one ([`Message::Attach`]); the background answers the second
//! with every program of that desktop ([`Message::Attached`]) and then each one's screen as
//! [`Message::Output`], so the screen draws them as they were before it goes on hearing them.
//!
//! Numbers are big end first. Text, paths and bytes are a four-byte length and what it counts;
//! paths and the environment travel as the bytes the system gave, since neither need be text.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read, Write};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::PathBuf;

/// The protocol this qdesk speaks. A change to what a message means gives it a new number.
pub const PROTOCOL: u32 = 1;

/// The longest message read, so a confused or hostile writer cannot make the reader take memory
/// without end. A program's screen and scrollback travel in pieces of [`PIECE`] and are far
/// under it; so is everything else.
pub const LONGEST: u32 = 1 << 22;

/// The most bytes of a program's output one [`Message::Output`] carries: a screen with a long
/// scrollback is sent in several, so no message comes near [`LONGEST`].
pub const PIECE: usize = 1 << 16;

const HELLO: u8 = 1;
const WELCOME: u8 = 2;
const ATTACH: u8 = 3;
const ATTACHED: u8 = 4;
const OPEN: u8 = 5;
const OPENED: u8 = 6;
const REFUSED: u8 = 7;
const INPUT: u8 = 8;
const OUTPUT: u8 = 9;
const RESIZE: u8 = 10;
const KILL: u8 = 11;
const CLOSE: u8 = 12;
const EXITED: u8 = 13;
const PLACE: u8 = 14;
const LOCK: u8 = 15;
const FINISH: u8 = 16;
const LIST: u8 = 17;
const DESKTOPS: u8 = 18;

/// Which desktop of the background a screen attaches to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// One no screen is attached to, else a new one: what `qdesk` does.
    Free,
    /// Always a new one: `qdesk --new`.
    New,
}

/// What starting a program for a window needs, all of it worked out by the screen: the
/// background only starts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Spec {
    /// The program, a path or a name to look for on the `PATH` the environment gives.
    pub program: OsString,
    /// Its arguments.
    pub args: Vec<OsString>,
    /// The folder it starts in.
    pub folder: PathBuf,
    /// Its whole environment: what the screen has, with the desktop's own variables and the
    /// entry's on top. Nothing of the background's own environment is added, since the screen is
    /// where the person is: a program started over today's connection gets today's variables.
    pub env: Vec<(OsString, OsString)>,
    /// The size of the window's body, in columns and rows.
    pub columns: u16,
    /// The rows, as `columns` gives the columns.
    pub rows: u16,
    /// How many lines it keeps above its screen.
    pub scrollback: u32,
}

/// One program of a desktop, as a screen that attaches hears of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The background's number for it.
    pub program: u64,
    /// The size of its screen now, which the screen attaching starts its copy at.
    pub columns: u16,
    /// The rows, as `columns` gives the columns.
    pub rows: u16,
    /// How many lines it keeps above its screen.
    pub scrollback: u32,
    /// Its process id, when the system gave one.
    pub pid: Option<u32>,
    /// The title it last gave itself.
    pub title: Option<String>,
    /// The folder it last said it is in.
    pub folder: Option<PathBuf>,
    /// How it ended: `None` while it runs, `Some(code)` once it has, with the code when the
    /// system gave one.
    pub exit: Option<Option<u32>>,
    /// What the screen said about its window, kept for it as it was said; see
    /// [`record`](crate::serve::record).
    pub window: Vec<u8>,
}

/// One desktop of the background, as `qdesk --list` hears of it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listed {
    /// Its number.
    pub desktop: u32,
    /// How many windows with a program it holds.
    pub programs: u32,
    /// How many of those programs still run.
    pub running: u32,
    /// Whether a screen is attached to it.
    pub attached: bool,
}

/// One message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Message {
    /// A screen's first words: the protocol it speaks.
    Hello {
        /// The screen's [`PROTOCOL`].
        protocol: u32,
    },
    /// The background's answer: the protocol it speaks and the version of the qdesk it runs.
    Welcome {
        /// The background's [`PROTOCOL`].
        protocol: u32,
        /// The background's package version, to name it to the person when the two differ.
        version: String,
    },
    /// A screen asks to show a desktop.
    Attach(Choice),
    /// The desktop a screen attached to: its number, whether it was left locked, and its
    /// programs. Each program's screen follows as [`Output`](Self::Output), and for one that has
    /// ended, its [`Exited`](Self::Exited) after that.
    Attached {
        /// Its number.
        desktop: u32,
        /// Whether its screen was locked when the last screen left it.
        locked: bool,
        /// Its programs, oldest first.
        programs: Vec<Record>,
    },
    /// A screen asks for a program to be started for a window.
    Open {
        /// The screen's number for the request, which the answer repeats.
        request: u32,
        /// What to start.
        spec: Spec,
        /// What the screen says about the window, kept for the next screen.
        window: Vec<u8>,
    },
    /// The program of a request runs.
    Opened {
        /// The request's number.
        request: u32,
        /// The background's number for the program.
        program: u64,
        /// Its process id, when the system gave one.
        pid: Option<u32>,
    },
    /// The program of a request could not be started.
    Refused {
        /// The request's number.
        request: u32,
        /// What kind of failure, as [`kind_code`] numbers them.
        kind: u8,
        /// What the system said.
        reason: String,
    },
    /// Keys, a paste or a report for a program.
    Input {
        /// Which program.
        program: u64,
        /// The bytes, as the program reads them.
        bytes: Vec<u8>,
    },
    /// What a program wrote, or a piece of its screen for a screen that attached.
    Output {
        /// Which program.
        program: u64,
        /// The bytes, as a terminal reads them.
        bytes: Vec<u8>,
    },
    /// A program's window was resized.
    Resize {
        /// Which program.
        program: u64,
        /// The new size of the window's body.
        columns: u16,
        /// The rows, as `columns` gives the columns.
        rows: u16,
    },
    /// End a program outright; its window stays.
    Kill {
        /// Which program.
        program: u64,
    },
    /// A program's window closed: end the program, giving it `grace` to go by itself, and
    /// forget it.
    Close {
        /// Which program.
        program: u64,
        /// How long it has, in milliseconds, before it is killed.
        grace_ms: u32,
    },
    /// A program ended. Nothing it wrote comes after this.
    Exited {
        /// Which program.
        program: u64,
        /// The code it ended with, when the system gave one.
        code: Option<u32>,
    },
    /// What the screen says about a program's window now: where it is, how large, which
    /// workspace. Kept for the next screen.
    Place {
        /// Which program.
        program: u64,
        /// The screen's own description of the window.
        window: Vec<u8>,
    },
    /// The screen of this desktop was locked or unlocked.
    Lock {
        /// Whether it is locked now.
        locked: bool,
    },
    /// The person left the desktop: its programs end and it is gone.
    Finish,
    /// A screen asks which desktops the background holds.
    List,
    /// The desktops the background holds.
    Desktops(Vec<Listed>),
}

/// The number a failure to start travels as, so the screen tells the person the same thing a
/// failure of its own would: the two kinds a person can act on, and everything else.
#[must_use]
pub fn kind_code(kind: io::ErrorKind) -> u8 {
    match kind {
        io::ErrorKind::NotFound => 1,
        io::ErrorKind::PermissionDenied => 2,
        _ => 0,
    }
}

/// The kind a [`kind_code`] stands for.
#[must_use]
pub fn kind_of(code: u8) -> io::ErrorKind {
    match code {
        1 => io::ErrorKind::NotFound,
        2 => io::ErrorKind::PermissionDenied,
        _ => io::ErrorKind::Other,
    }
}

/// Writes a message's body, and the window descriptions of [`record`](super::record).
#[derive(Default)]
pub(super) struct Out(pub(super) Vec<u8>);

impl Out {
    pub(super) fn byte(&mut self, value: u8) {
        self.0.push(value);
    }

    pub(super) fn flag(&mut self, value: bool) {
        self.byte(u8::from(value));
    }

    pub(super) fn u16(&mut self, value: u16) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    pub(super) fn u32(&mut self, value: u32) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    pub(super) fn u64(&mut self, value: u64) {
        self.0.extend_from_slice(&value.to_be_bytes());
    }

    pub(super) fn bytes(&mut self, value: &[u8]) {
        self.u32(u32::try_from(value.len()).unwrap_or(u32::MAX));
        self.0.extend_from_slice(value);
    }

    pub(super) fn os(&mut self, value: &OsStr) {
        self.bytes(value.as_bytes());
    }

    pub(super) fn maybe_u32(&mut self, value: Option<u32>) {
        self.flag(value.is_some());
        if let Some(value) = value {
            self.u32(value);
        }
    }

    pub(super) fn maybe_bytes(&mut self, value: Option<&[u8]>) {
        self.flag(value.is_some());
        if let Some(value) = value {
            self.bytes(value);
        }
    }

    pub(super) fn count(&mut self, count: usize) {
        self.u32(u32::try_from(count).unwrap_or(u32::MAX));
    }
}

/// Reads a message's body, and the window descriptions of [`record`](super::record).
pub(super) struct In<'a>(pub(super) &'a [u8]);

impl In<'_> {
    pub(super) fn take(&mut self, count: usize) -> io::Result<&[u8]> {
        if self.0.len() < count {
            return Err(invalid("a message cut short"));
        }
        let (taken, rest) = self.0.split_at(count);
        self.0 = rest;
        Ok(taken)
    }

    pub(super) fn byte(&mut self) -> io::Result<u8> {
        Ok(self.take(1)?[0])
    }

    pub(super) fn flag(&mut self) -> io::Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(invalid("a yes or no that is neither")),
        }
    }

    pub(super) fn u16(&mut self) -> io::Result<u16> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().map_err(|_| invalid("a number cut short"))?))
    }

    pub(super) fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().map_err(|_| invalid("a number cut short"))?))
    }

    pub(super) fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().map_err(|_| invalid("a number cut short"))?))
    }

    pub(super) fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let length = self.u32()? as usize;
        Ok(self.take(length)?.to_vec())
    }

    pub(super) fn text(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| invalid("text that is not text"))
    }

    pub(super) fn os(&mut self) -> io::Result<OsString> {
        Ok(OsString::from_vec(self.bytes()?))
    }

    pub(super) fn maybe_u32(&mut self) -> io::Result<Option<u32>> {
        if self.flag()? { self.u32().map(Some) } else { Ok(None) }
    }

    pub(super) fn maybe_bytes(&mut self) -> io::Result<Option<Vec<u8>>> {
        if self.flag()? { self.bytes().map(Some) } else { Ok(None) }
    }

    /// A count of things to follow, each at least `least` bytes long: a count the rest of the
    /// message cannot hold is refused before anything is made room for.
    pub(super) fn count(&mut self, least: usize) -> io::Result<usize> {
        let count = self.u32()? as usize;
        if count.saturating_mul(least.max(1)) > self.0.len() {
            return Err(invalid("more things than the message holds"));
        }
        Ok(count)
    }

    pub(super) fn end(&self) -> io::Result<()> {
        if self.0.is_empty() { Ok(()) } else { Err(invalid("a message longer than what it says")) }
    }
}

impl Spec {
    fn write(&self, out: &mut Out) {
        out.os(&self.program);
        out.count(self.args.len());
        for arg in &self.args {
            out.os(arg);
        }
        out.os(self.folder.as_os_str());
        out.count(self.env.len());
        for (name, value) in &self.env {
            out.os(name);
            out.os(value);
        }
        out.u16(self.columns);
        out.u16(self.rows);
        out.u32(self.scrollback);
    }

    fn read(from: &mut In<'_>) -> io::Result<Self> {
        let program = from.os()?;
        let mut args = Vec::new();
        for _ in 0..from.count(4)? {
            args.push(from.os()?);
        }
        let folder = PathBuf::from(from.os()?);
        let mut env = Vec::new();
        for _ in 0..from.count(8)? {
            env.push((from.os()?, from.os()?));
        }
        Ok(Self { program, args, folder, env, columns: from.u16()?, rows: from.u16()?, scrollback: from.u32()? })
    }
}

impl Record {
    fn write(&self, out: &mut Out) {
        out.u64(self.program);
        out.u16(self.columns);
        out.u16(self.rows);
        out.u32(self.scrollback);
        out.maybe_u32(self.pid);
        out.maybe_bytes(self.title.as_deref().map(str::as_bytes));
        out.maybe_bytes(self.folder.as_deref().map(|folder| folder.as_os_str().as_bytes()));
        out.flag(self.exit.is_some());
        if let Some(code) = self.exit {
            out.maybe_u32(code);
        }
        out.bytes(&self.window);
    }

    fn read(from: &mut In<'_>) -> io::Result<Self> {
        let program = from.u64()?;
        let columns = from.u16()?;
        let rows = from.u16()?;
        let scrollback = from.u32()?;
        let pid = from.maybe_u32()?;
        let title =
            from.maybe_bytes()?.map(String::from_utf8).transpose().map_err(|_| invalid("a title that is not text"))?;
        let folder = from.maybe_bytes()?.map(|bytes| PathBuf::from(OsString::from_vec(bytes)));
        let exit = if from.flag()? { Some(from.maybe_u32()?) } else { None };
        let window = from.bytes()?;
        Ok(Self { program, columns, rows, scrollback, pid, title, folder, exit, window })
    }
}

impl Message {
    /// The frame this message travels as.
    #[must_use]
    pub fn frame(&self) -> Vec<u8> {
        let mut out = Out::default();
        match self {
            Self::Hello { protocol } => {
                out.byte(HELLO);
                out.u32(*protocol);
            }
            Self::Welcome { protocol, version } => {
                out.byte(WELCOME);
                out.u32(*protocol);
                out.bytes(version.as_bytes());
            }
            Self::Attach(choice) => {
                out.byte(ATTACH);
                out.byte(match choice {
                    Choice::Free => 0,
                    Choice::New => 1,
                });
            }
            Self::Attached { desktop, locked, programs } => {
                out.byte(ATTACHED);
                out.u32(*desktop);
                out.flag(*locked);
                out.count(programs.len());
                for record in programs {
                    record.write(&mut out);
                }
            }
            Self::Open { request, spec, window } => {
                out.byte(OPEN);
                out.u32(*request);
                spec.write(&mut out);
                out.bytes(window);
            }
            Self::Opened { request, program, pid } => {
                out.byte(OPENED);
                out.u32(*request);
                out.u64(*program);
                out.maybe_u32(*pid);
            }
            Self::Refused { request, kind, reason } => {
                out.byte(REFUSED);
                out.u32(*request);
                out.byte(*kind);
                out.bytes(reason.as_bytes());
            }
            Self::Input { program, bytes } => {
                out.byte(INPUT);
                out.u64(*program);
                out.bytes(bytes);
            }
            Self::Output { program, bytes } => {
                out.byte(OUTPUT);
                out.u64(*program);
                out.bytes(bytes);
            }
            Self::Resize { program, columns, rows } => {
                out.byte(RESIZE);
                out.u64(*program);
                out.u16(*columns);
                out.u16(*rows);
            }
            Self::Kill { program } => {
                out.byte(KILL);
                out.u64(*program);
            }
            Self::Close { program, grace_ms } => {
                out.byte(CLOSE);
                out.u64(*program);
                out.u32(*grace_ms);
            }
            Self::Exited { program, code } => {
                out.byte(EXITED);
                out.u64(*program);
                out.maybe_u32(*code);
            }
            Self::Place { program, window } => {
                out.byte(PLACE);
                out.u64(*program);
                out.bytes(window);
            }
            Self::Lock { locked } => {
                out.byte(LOCK);
                out.flag(*locked);
            }
            Self::Finish => out.byte(FINISH),
            Self::List => out.byte(LIST),
            Self::Desktops(desktops) => {
                out.byte(DESKTOPS);
                out.count(desktops.len());
                for listed in desktops {
                    out.u32(listed.desktop);
                    out.u32(listed.programs);
                    out.u32(listed.running);
                    out.flag(listed.attached);
                }
            }
        }
        let body = out.0;
        let length = u32::try_from(body.len()).unwrap_or(u32::MAX);
        let mut frame = length.to_be_bytes().to_vec();
        frame.extend_from_slice(&body);
        frame
    }

    /// The message a frame's body holds, without its length.
    ///
    /// # Errors
    ///
    /// [`io::ErrorKind::InvalidData`] when it is not one this qdesk knows, is cut short or goes
    /// on past its end.
    pub fn parse(body: &[u8]) -> io::Result<Self> {
        let mut from = In(body);
        let message = match from.byte().map_err(|_| invalid("an empty message"))? {
            HELLO => Self::Hello { protocol: from.u32()? },
            WELCOME => Self::Welcome { protocol: from.u32()?, version: from.text()? },
            ATTACH => Self::Attach(match from.byte()? {
                0 => Choice::Free,
                1 => Choice::New,
                _ => return Err(invalid("a desktop choice this qdesk does not know")),
            }),
            ATTACHED => {
                let desktop = from.u32()?;
                let locked = from.flag()?;
                let mut programs = Vec::new();
                for _ in 0..from.count(24)? {
                    programs.push(Record::read(&mut from)?);
                }
                Self::Attached { desktop, locked, programs }
            }
            OPEN => Self::Open { request: from.u32()?, spec: Spec::read(&mut from)?, window: from.bytes()? },
            OPENED => Self::Opened { request: from.u32()?, program: from.u64()?, pid: from.maybe_u32()? },
            REFUSED => Self::Refused { request: from.u32()?, kind: from.byte()?, reason: from.text()? },
            INPUT => Self::Input { program: from.u64()?, bytes: from.bytes()? },
            OUTPUT => Self::Output { program: from.u64()?, bytes: from.bytes()? },
            RESIZE => Self::Resize { program: from.u64()?, columns: from.u16()?, rows: from.u16()? },
            KILL => Self::Kill { program: from.u64()? },
            CLOSE => Self::Close { program: from.u64()?, grace_ms: from.u32()? },
            EXITED => Self::Exited { program: from.u64()?, code: from.maybe_u32()? },
            PLACE => Self::Place { program: from.u64()?, window: from.bytes()? },
            LOCK => Self::Lock { locked: from.flag()? },
            FINISH => Self::Finish,
            LIST => Self::List,
            DESKTOPS => {
                let mut desktops = Vec::new();
                for _ in 0..from.count(13)? {
                    desktops.push(Listed {
                        desktop: from.u32()?,
                        programs: from.u32()?,
                        running: from.u32()?,
                        attached: from.flag()?,
                    });
                }
                Self::Desktops(desktops)
            }
            _ => return Err(invalid("a message this qdesk does not know")),
        };
        from.end()?;
        Ok(message)
    }
}

fn invalid(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

/// Writes `message` to `to`.
///
/// # Errors
///
/// When it cannot be written.
pub fn send(to: &mut impl Write, message: &Message) -> io::Result<()> {
    to.write_all(&message.frame())?;
    to.flush()
}

/// Reads the next message from `from`, or `None` when the other end closed between messages.
///
/// # Errors
///
/// When reading fails, the other end closes inside a message, or the message is longer than
/// [`LONGEST`] or unknown.
pub fn receive(from: &mut impl Read) -> io::Result<Option<Message>> {
    let mut length = [0; 4];
    match from.read_exact(&mut length) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(error) => return Err(error),
    }
    let length = u32::from_be_bytes(length);
    if length > LONGEST {
        return Err(invalid("a message longer than any this qdesk sends"));
    }
    let mut body = vec![0; length as usize];
    from.read_exact(&mut body)?;
    Message::parse(&body).map(Some)
}

/// What the background answered a screen's hello with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Greeting {
    /// It speaks this screen's protocol.
    Agreed,
    /// It speaks another: a different qdesk started it. Its version names it to the person.
    Mismatch {
        /// The background's protocol.
        protocol: u32,
        /// The background's package version.
        version: String,
    },
}

/// Says hello on `stream` as a screen does and reads the answer.
///
/// # Errors
///
/// When the background cannot be written to or read from, closes before it answers, or answers
/// with something other than a welcome.
pub fn hello(stream: &mut (impl Read + Write)) -> io::Result<Greeting> {
    send(stream, &Message::Hello { protocol: PROTOCOL })?;
    match receive(stream)? {
        Some(Message::Welcome { protocol, .. }) if protocol == PROTOCOL => Ok(Greeting::Agreed),
        Some(Message::Welcome { protocol, version }) => Ok(Greeting::Mismatch { protocol, version }),
        Some(_) => Err(invalid("an answer that is not a welcome")),
        None => Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the background closed before it answered")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(program: u64, exit: Option<Option<u32>>) -> Record {
        Record {
            program,
            columns: 80,
            rows: 24,
            scrollback: 1000,
            pid: Some(4321),
            title: Some("htop".to_owned()),
            folder: Some(PathBuf::from("/srv/a")),
            exit,
            window: b"x = 1".to_vec(),
        }
    }

    fn every_message() -> Vec<Message> {
        vec![
            Message::Hello { protocol: PROTOCOL },
            Message::Welcome { protocol: 7, version: "0.2.0".to_owned() },
            Message::Welcome { protocol: 7, version: String::new() },
            Message::Attach(Choice::Free),
            Message::Attach(Choice::New),
            Message::Attached { desktop: 3, locked: true, programs: vec![] },
            Message::Attached {
                desktop: 1,
                locked: false,
                programs: vec![record(1, None), record(2, Some(None)), record(9, Some(Some(3)))],
            },
            Message::Open {
                request: 5,
                spec: Spec {
                    program: OsString::from("/bin/sh"),
                    args: vec![OsString::from("-c"), OsString::from_vec(vec![b'e', 0xff])],
                    folder: PathBuf::from("/home/a"),
                    env: vec![(OsString::from("TERM"), OsString::from("xterm-256color"))],
                    columns: 120,
                    rows: 40,
                    scrollback: 5000,
                },
                window: vec![1, 2, 3],
            },
            Message::Opened { request: 5, program: 77, pid: None },
            Message::Opened { request: 5, program: 77, pid: Some(12) },
            Message::Refused { request: 6, kind: 1, reason: "No such file".to_owned() },
            Message::Input { program: 1, bytes: b"ls\r".to_vec() },
            Message::Output { program: 1, bytes: vec![0x1b, b'[', b'm'] },
            Message::Resize { program: 2, columns: 100, rows: 30 },
            Message::Kill { program: 4 },
            Message::Close { program: 4, grace_ms: 2000 },
            Message::Exited { program: 4, code: Some(0) },
            Message::Exited { program: 4, code: None },
            Message::Place { program: 4, window: b"rect".to_vec() },
            Message::Lock { locked: true },
            Message::Finish,
            Message::List,
            Message::Desktops(vec![
                Listed { desktop: 1, programs: 3, running: 2, attached: true },
                Listed { desktop: 2, programs: 0, running: 0, attached: false },
            ]),
        ]
    }

    #[test]
    fn every_message_reads_back_as_it_was_written() {
        for message in every_message() {
            let frame = message.frame();
            assert_eq!(receive(&mut frame.as_slice()).expect("read"), Some(message));
        }
    }

    #[test]
    fn a_message_cut_anywhere_or_with_bytes_left_over_is_refused() {
        for message in every_message() {
            let frame = message.frame();
            let body = &frame[4..];
            for cut in 0..body.len() {
                assert!(Message::parse(&body[..cut]).is_err(), "{message:?} cut at {cut}");
            }
            let mut longer = body.to_vec();
            longer.push(0);
            assert!(Message::parse(&longer).is_err(), "{message:?} with a byte more");
        }
    }

    #[test]
    fn two_messages_in_a_row_are_told_apart_and_a_clean_end_is_none() {
        let mut bytes = Message::Hello { protocol: 1 }.frame();
        bytes.extend(Message::Hello { protocol: 2 }.frame());
        let mut reader = bytes.as_slice();
        assert_eq!(receive(&mut reader).expect("first"), Some(Message::Hello { protocol: 1 }));
        assert_eq!(receive(&mut reader).expect("second"), Some(Message::Hello { protocol: 2 }));
        assert_eq!(receive(&mut reader).expect("the end"), None);
    }

    #[test]
    fn a_long_cut_or_unknown_message_is_refused() {
        let long = (LONGEST + 1).to_be_bytes();
        assert_eq!(receive(&mut long.as_slice()).expect_err("too long").kind(), io::ErrorKind::InvalidData);
        let cut = &Message::Hello { protocol: 1 }.frame()[..6];
        assert!(receive(&mut &cut[..]).is_err(), "cut inside a message");
        let unknown = [0, 0, 0, 1, 99];
        assert_eq!(receive(&mut unknown.as_slice()).expect_err("unknown").kind(), io::ErrorKind::InvalidData);
        let short = [0, 0, 0, 2, HELLO, 0];
        assert_eq!(receive(&mut short.as_slice()).expect_err("short").kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn a_count_larger_than_the_message_makes_no_room_for_it() {
        // Four billion records announced in a message of a few bytes.
        let body = [ATTACHED, 0, 0, 0, 1, 0, 0xff, 0xff, 0xff, 0xff];
        assert_eq!(Message::parse(&body).expect_err("refused").kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn the_failure_kinds_a_person_can_act_on_travel_and_the_rest_are_other() {
        for kind in [io::ErrorKind::NotFound, io::ErrorKind::PermissionDenied, io::ErrorKind::Other] {
            assert_eq!(kind_of(kind_code(kind)), kind);
        }
        assert_eq!(kind_of(kind_code(io::ErrorKind::Interrupted)), io::ErrorKind::Other);
    }
}
