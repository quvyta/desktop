//! The background desktop, `qdesk serve`: the process that outlives the connection.
//!
//! Over SSH a dropped connection hangs up on everything started from it. The background is
//! started apart from the terminal, in a session of its own, so the hangup does not reach it; a
//! screen (the `qdesk` the person sees) talks to it over a socket only its user can open
//! ([`socket`]), in short messages ([`wire`]). The background holds the programs of the screen's
//! windows (its `background` module); the screen draws them through [`link`] and tells it where their
//! windows are ([`record`]), so a screen that comes back after the connection dropped finds them
//! where they were.
//!
//! It ends by itself once no screen has been connected and no program has run for the while the
//! person chose in Settings (`background-exit`, ten minutes unless chosen otherwise, or never).
//! The setting is read again every time the background is left alone, so a change made in
//! Settings reaches a background that is already running.

mod background;
pub mod link;
pub mod record;
pub mod socket;
pub mod wire;

use std::io;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::thread;
use std::time::Duration;

use crate::settings;
use background::{Background, Event, Outbox};
use wire::{Message, PROTOCOL};

/// How long the programs of a desktop the person left have to end by themselves before they are
/// killed: as long as a closing window gives its program.
const GRACE: Duration = crate::session::GRACE;

/// How long the background waits alone before it ends, as the settings in `config_dir` say, or
/// those in the ecosystem's folder on this machine without one. `None` is never.
#[must_use]
pub fn idle_limit(config_dir: Option<&Path>) -> Option<Duration> {
    let loaded = match config_dir {
        Some(folder) => settings::load_in(folder),
        None => settings::load(),
    };
    loaded.prefs.background_exit.after()
}

/// The socket a background listens on, and what it removes when it ends.
pub struct Listening {
    listener: UnixListener,
    path: PathBuf,
    /// The socket file's inode: at the end only this file is removed, not one a newer background
    /// put in its place.
    inode: u64,
}

impl Listening {
    /// Listens on `path`; see [`socket::bind`].
    ///
    /// # Errors
    ///
    /// As [`socket::bind`].
    pub fn on(path: &Path) -> Result<Self, socket::BindError> {
        let listener = socket::bind(path)?;
        let inode = std::fs::symlink_metadata(path)?.ino();
        Ok(Self { listener, path: path.to_path_buf(), inode })
    }

    /// Where it listens.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Takes the socket file away, if it is still this one's.
    fn remove(&self) {
        if std::fs::symlink_metadata(&self.path).is_ok_and(|meta| meta.ino() == self.inode) {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// Serves on `listening` until it has been alone for as long as `limit` says: no screen
/// connected and no program running. `limit` is asked anew each time it is left alone (and once
/// at the start, before any screen came); `None` waits for the next screen however long it takes.
/// The socket file is taken away before it returns, so a screen that comes after finds none and
/// starts a new background.
///
/// # Errors
///
/// When the socket can no longer be listened on.
pub fn run(listening: &Listening, mut limit: impl FnMut() -> Option<Duration>) -> io::Result<()> {
    let (events, heard) = mpsc::channel();
    let background = Background::new(events.clone());
    let accepting = listening.listener.try_clone()?;
    let owner = socket::own_uid();
    let serving = Arc::clone(&background);
    thread::spawn(move || accept(&accepting, owner, &serving, &events));
    let stopped = || io::Error::other("the socket stopped answering");
    let mut screens = 0_usize;
    loop {
        let alone = screens == 0 && background.running() == 0;
        let event = if alone {
            match limit() {
                None => heard.recv().map_err(|_| stopped())?,
                Some(after) => match heard.recv_timeout(after) {
                    Ok(event) => event,
                    // A program may have ended on its own while the limit was asked: that is
                    // being alone, which is what the wait was for.
                    Err(RecvTimeoutError::Timeout) if screens == 0 && background.running() == 0 => {
                        listening.remove();
                        return Ok(());
                    }
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => return Err(stopped()),
                },
            }
        } else {
            heard.recv().map_err(|_| stopped())?
        };
        match event {
            Event::Arrived => screens += 1,
            Event::Left => screens = screens.saturating_sub(1),
            Event::Ended => {}
        }
    }
}

/// Takes every connection: one of another user's is closed unanswered, one of the owner's is
/// counted and answered on a thread of its own.
fn accept(listener: &UnixListener, owner: u32, background: &Arc<Background>, events: &Sender<Event>) {
    let mut connections = 0_u64;
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        if !socket::same_user(&stream, owner).unwrap_or(false) {
            continue;
        }
        // Counted here, before its thread can say it left, so the count never goes below zero.
        if events.send(Event::Arrived).is_err() {
            return;
        }
        connections += 1;
        let screen = connections;
        let events = events.clone();
        let background = Arc::clone(background);
        thread::spawn(move || {
            let _ = converse(&background, screen, stream);
            let _ = events.send(Event::Left);
        });
    }
}

/// One screen's conversation. Its hello is answered with the background's protocol and version,
/// whatever protocol the screen speaks, since telling the person is the screen's to do. A screen
/// may then ask for the list of desktops or attach to one; once attached, what it says is about
/// that desktop's programs. When it goes, its desktop stays with whatever still runs on it.
fn converse(background: &Arc<Background>, screen: u64, stream: UnixStream) -> io::Result<()> {
    let mut reading = stream.try_clone()?;
    let outbox = Outbox::start(stream)?;
    let mut desktop = None;
    let said = converse_on(background, screen, &mut reading, &outbox, &mut desktop);
    if let Some(desktop) = desktop {
        background.leave(desktop, screen);
    }
    said
}

fn converse_on(
    background: &Arc<Background>,
    screen: u64,
    reading: &mut UnixStream,
    outbox: &Outbox,
    desktop: &mut Option<u32>,
) -> io::Result<()> {
    while let Some(message) = wire::receive(reading)? {
        match (message, *desktop) {
            (Message::Hello { .. }, _) => {
                outbox.send(&Message::Welcome { protocol: PROTOCOL, version: env!("CARGO_PKG_VERSION").to_owned() });
            }
            (Message::List, _) => outbox.send(&Message::Desktops(background.list())),
            (Message::Attach(choice), None) => *desktop = Some(background.attach(choice, screen, outbox)),
            (Message::Open { request, spec, window }, Some(desk)) => {
                background.open(desk, request, &spec, window, outbox);
            }
            (Message::Input { program, bytes }, Some(desk)) => background.input(desk, program, &bytes),
            (Message::Resize { program, columns, rows }, Some(desk)) => {
                background.resize(desk, program, columns, rows);
            }
            (Message::Kill { program }, Some(desk)) => background.kill(desk, program),
            (Message::Close { program, grace_ms }, Some(desk)) => {
                background.close(desk, program, Duration::from_millis(u64::from(grace_ms)));
            }
            (Message::Place { program, window }, Some(desk)) => background.place(desk, program, window),
            (Message::Lock { locked }, Some(desk)) => background.lock(desk, locked),
            (Message::Finish, Some(desk)) => background.finish(desk, GRACE),
            // What only a background says, a second attach, and what needs a desktop said before
            // one was chosen: nothing to do.
            _ => {}
        }
    }
    Ok(())
}

/// `qdesk serve`: leaves the terminal's session so a hangup does not reach it, then serves on
/// this user's socket with the limit the settings on this machine give.
///
/// Leaving the session fails when the process leads a process group, as a command typed at a
/// shell does; it then serves all the same, tied to that terminal. A screen starts it as a child of
/// its own, which never leads one.
///
/// # Errors
///
/// [`socket::BindError::Serving`] when a background answers already, else what kept it from
/// listening.
pub fn serve() -> Result<(), socket::BindError> {
    let _ = rustix::process::setsid();
    let listening = Listening::on(&socket::here())?;
    run(&listening, || idle_limit(None))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    use super::*;
    use wire::Greeting;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qdesk-serve-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch folder");
        dir
    }

    /// Generous: what is waited for comes in milliseconds, and a slow machine only makes it slower.
    const BUDGET: Duration = Duration::from_secs(60);

    fn wait_for(what: &str, done: impl Fn() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < BUDGET, "{what} never happened");
            thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn the_setting_decides_how_long_the_background_waits_alone() {
        let dir = scratch("limit");
        assert_eq!(idle_limit(Some(&dir)), Some(Duration::from_secs(600)), "ten minutes unless chosen");
        std::fs::write(dir.join("desktop.conf"), "background-exit = \"30m\"\n").expect("written");
        assert_eq!(idle_limit(Some(&dir)), Some(Duration::from_secs(1_800)));
        std::fs::write(dir.join("desktop.conf"), "background-exit = \"never\"\n").expect("written");
        assert_eq!(idle_limit(Some(&dir)), None);
        std::fs::write(dir.join("desktop.conf"), "background-exit = \"soon\"\n").expect("written");
        assert_eq!(idle_limit(Some(&dir)), Some(Duration::from_secs(600)), "what cannot be read is the default");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_screen_is_welcomed_and_the_background_waits_alone_only_as_long_as_it_is_told() {
        let dir = scratch("run");
        let path = dir.join("quvyta").join(socket::FILE);
        let listening = Listening::on(&path).expect("listening");
        // Never at the start, so it waits for the first screen; never again when that one leaves,
        // and a short while once the second has left.
        let asked = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&asked);
        let limits = [None, None, Some(Duration::from_millis(10))];
        let server = thread::spawn(move || {
            run(&listening, || limits[counted.fetch_add(1, Ordering::SeqCst).min(2)]).expect("served");
        });

        let mut first = UnixStream::connect(&path).expect("connected");
        assert_eq!(wire::hello(&mut first).expect("answered"), Greeting::Agreed);
        drop(first);
        wait_for("the background noticing the screen left", || asked.load(Ordering::SeqCst) == 2);
        assert!(!server.is_finished(), "told never, it does not end");

        let mut second = UnixStream::connect(&path).expect("still answering after the first left");
        assert_eq!(wire::hello(&mut second).expect("answered"), Greeting::Agreed);
        assert!(!server.is_finished(), "a screen is connected");
        drop(second);
        wait_for("the background ending", || server.is_finished());
        server.join().expect("no panic");
        assert!(!path.exists(), "the socket is taken away");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_screen_of_another_protocol_is_answered_and_the_background_lives_on() {
        let dir = scratch("protocol");
        let path = dir.join("quvyta").join(socket::FILE);
        let listening = Listening::on(&path).expect("listening");
        thread::spawn(move || run(&listening, || None));

        let mut older = UnixStream::connect(&path).expect("connected");
        wire::send(&mut older, &Message::Hello { protocol: PROTOCOL + 1 }).expect("said");
        let answer = wire::receive(&mut older).expect("answered");
        assert_eq!(
            answer,
            Some(Message::Welcome { protocol: PROTOCOL, version: env!("CARGO_PKG_VERSION").to_owned() })
        );
        drop(older);
        let mut next = UnixStream::connect(&path).expect("the background is still there");
        assert_eq!(wire::hello(&mut next).expect("answered"), Greeting::Agreed);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_screen_hears_when_the_background_speaks_another_protocol() {
        let (mut screen, mut background) = UnixStream::pair().expect("pair");
        let answering = thread::spawn(move || {
            let hello = wire::receive(&mut background).expect("read");
            assert_eq!(hello, Some(Message::Hello { protocol: PROTOCOL }));
            wire::send(&mut background, &Message::Welcome { protocol: PROTOCOL + 1, version: "9.9.9".to_owned() })
        });
        assert_eq!(
            wire::hello(&mut screen).expect("answered"),
            Greeting::Mismatch { protocol: PROTOCOL + 1, version: "9.9.9".to_owned() }
        );
        answering.join().expect("no panic").expect("sent");
    }

    #[test]
    fn a_newer_socket_in_the_same_place_is_not_taken_away() {
        let dir = scratch("newer");
        let path = dir.join("quvyta").join(socket::FILE);
        let old = Listening::on(&path).expect("listening");
        std::fs::remove_file(&path).expect("gone");
        let _newer = Listening::on(&path).expect("a newer one");
        old.remove();
        assert!(path.exists(), "the newer one's file stays");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
