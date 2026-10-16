//! The real programs over a hangup: `qdesk` runs on a pseudo-terminal, starts its background by
//! itself, opens a window, and is hung up as a dropped SSH connection hangs it up. The window's
//! program lives on in the background, and the next `qdesk` shows it again.
//!
//! Everything is the test's own: the home, the settings, the applications and the temporary
//! folder the socket is in, so nothing reaches the person's own desktop or background. Nothing is
//! timed: each step waits, within a generous bound, for something it can see — words on the
//! screen, or what the background says over its socket.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};
use qdesk::serve::link::Link;
use qdesk::serve::socket;
use qdesk::serve::wire::Listed;

/// Generous: each step takes milliseconds, a loaded machine only makes it slower.
const BUDGET: Duration = Duration::from_secs(60);

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let number = NEXT.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("qdesk-hangup-{name}-{}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let apps = path.join(".local/share/quvyta/desktop/apps");
        std::fs::create_dir_all(&apps).expect("an application folder");
        std::fs::create_dir_all(path.join(".config/quvyta/desktop")).expect("a settings folder");
        std::fs::create_dir_all(path.join("tmp")).expect("a temporary folder");
        let pid_file = path.join("stayer.pid");
        std::fs::write(
            apps.join("stayer.toml"),
            format!(
                "name = \"Stayer\"\ncommand = [\"/bin/sh\", \"-c\", \"echo stayer-said; echo $$ > {}; \
                 while :; do sleep 1; done\"]\ncategory = \"system\"\n",
                pid_file.display()
            ),
        )
        .expect("the entry is written");
        std::fs::write(
            path.join(".config/quvyta/desktop/desktop.toml"),
            "icons = [\"stayer\"]\nwelcome_seen = true\nrecommended_seen = true\nresize_hint_seen = true\n",
        )
        .expect("the desktop file is written");
        Self(path)
    }

    fn socket(&self) -> PathBuf {
        socket::place(Some(self.0.join("tmp").as_os_str()), socket::own_uid())
    }

    /// The desktops the background holds; empty when none answers.
    fn listed(&self) -> Vec<Listed> {
        Link::connect(&self.socket()).ok().and_then(|link| link.list().ok()).unwrap_or_default()
    }

    /// The process id the program of the window wrote down.
    fn program(&self) -> Option<i32> {
        std::fs::read_to_string(self.0.join("stayer.pid")).ok()?.trim().parse().ok()
    }
}

impl Drop for Scratch {
    /// Ends the background and the programs it holds before the folder goes.
    fn drop(&mut self) {
        if let Ok(stream) = std::os::unix::net::UnixStream::connect(self.socket())
            && let Ok(peer) = rustix::net::sockopt::socket_peercred(&stream)
        {
            let _ = rustix::process::kill_process(peer.pid, rustix::process::Signal::KILL);
        }
        if let Some(pid) = self.program().and_then(rustix::process::Pid::from_raw) {
            let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A `qdesk` on a pseudo-terminal of 100 by 30, and everything it has written.
struct Screen {
    seen: Arc<Mutex<Vec<u8>>>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
}

impl Screen {
    fn open(scratch: &Scratch) -> Self {
        let home = &scratch.0;
        let pair = native_pty_system()
            .openpty(PtySize { rows: 30, cols: 100, pixel_width: 0, pixel_height: 0 })
            .expect("a pseudo-terminal");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_qdesk"));
        command.env_clear();
        command.env("HOME", home);
        command.env("TMPDIR", home.join("tmp"));
        command.env("XDG_CONFIG_HOME", home.join(".config"));
        command.env("XDG_DATA_HOME", home.join(".local/share"));
        command.env("XDG_DATA_DIRS", home.join("share"));
        command.env("XDG_STATE_HOME", home.join(".local/state"));
        command.env("XDG_CACHE_HOME", home.join(".cache"));
        command.env("PATH", "/usr/bin:/bin");
        command.env("SHELL", "/bin/sh");
        command.env("TERM", "xterm-256color");
        command.env("LANG", "C.UTF-8");
        command.env("QUVYTA_ICONS", "unicode");
        command.env("BROWSER", "true");
        command.cwd(home);
        let child = pair.slave.spawn_command(command).expect("qdesk starts");
        drop(pair.slave);
        let mut reader = pair.master.try_clone_reader().expect("a reader");
        let writer = pair.master.take_writer().expect("a writer");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let into = Arc::clone(&seen);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 65536];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                into.lock().expect("not poisoned").extend_from_slice(&buffer[..count]);
            }
        });
        Self { seen, writer, child, _master: pair.master }
    }

    fn pid(&self) -> rustix::process::Pid {
        let pid = self.child.process_id().expect("a process id");
        rustix::process::Pid::from_raw(i32::try_from(pid).expect("a pid")).expect("not zero")
    }

    fn shows(&self, text: &str) -> bool {
        String::from_utf8_lossy(&self.seen.lock().expect("not poisoned")).contains(text)
    }

    fn until_shows(&self, text: &str) {
        let start = Instant::now();
        while !self.shows(text) {
            assert!(start.elapsed() < BUDGET, "{text} never came on the screen");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("qdesk reads its terminal");
        self.writer.flush().expect("qdesk reads its terminal");
    }

    /// Whether qdesk has ended, without waiting.
    fn ended(&mut self) -> bool {
        self.child.try_wait().is_ok_and(|status| status.is_some())
    }
}

impl Drop for Screen {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let start = Instant::now();
    while !done() {
        assert!(start.elapsed() < BUDGET, "{what} never happened");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn alive(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .is_ok_and(|stat| stat.rsplit_once(") ").is_some_and(|(_, rest)| !rest.starts_with('Z')))
}

/// Opens the application called `name` from the launcher: a click on its button at the left end of
/// the dock, its name in the search, and enter.
fn open_from_launcher(screen: &mut Screen, name: &str) {
    // The desktop is drawn, with the application's icon on the floor, before anything is clicked.
    screen.until_shows(name);
    // A press and a release at the third column of the last row, as a terminal reports them.
    screen.send(b"\x1b[<0;3;30M\x1b[<0;3;30m");
    screen.until_shows("Search");
    screen.send(name.as_bytes());
    screen.send(b"\r");
}

#[test]
fn a_hung_up_screen_leaves_its_programs_running_for_the_next_one() {
    let scratch = Scratch::new("drop");
    let mut first = Screen::open(&scratch);
    wait_for("the screen to start its background and attach", || {
        scratch.listed().first().is_some_and(|desktop| desktop.attached)
    });
    open_from_launcher(&mut first, "Stayer");
    first.until_shows("stayer-said");
    wait_for("the program to say who it is", || scratch.program().is_some());
    let program = scratch.program().expect("written");

    // The connection drops: the terminal's hangup reaches every process of the screen's group.
    let group = first.pid();
    rustix::process::kill_process_group(group, rustix::process::Signal::HUP).expect("hung up");
    wait_for("the screen to end on the hangup", || first.ended());
    assert!(alive(program), "the window's program outlived its screen");
    wait_for("the background to see the screen go", || {
        scratch.listed().first().is_some_and(|desktop| !desktop.attached && desktop.running == 1)
    });

    let second = Screen::open(&scratch);
    wait_for("the next screen to attach", || scratch.listed().first().is_some_and(|desktop| desktop.attached));
    assert_eq!(scratch.listed().len(), 1, "the same desktop, not a second one");
    second.until_shows("stayer-said");
    assert!(alive(program), "and the same program");
    drop(second);
}

#[test]
fn qdesk_list_names_the_desktops_the_background_keeps() {
    let scratch = Scratch::new("list");
    let list = || {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_qdesk"))
            .arg("--list")
            .env_clear()
            .env("HOME", &scratch.0)
            .env("TMPDIR", scratch.0.join("tmp"))
            .env("LANG", "en_US.UTF-8")
            .output()
            .expect("qdesk --list runs");
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    assert_eq!(list().trim(), "No background desktop is running.");
    assert!(!scratch.socket().exists(), "asking starts no background");

    let mut screen = Screen::open(&scratch);
    wait_for("the screen to attach", || scratch.listed().first().is_some_and(|desktop| desktop.attached));
    open_from_launcher(&mut screen, "Stayer");
    wait_for("the program to run", || scratch.listed().first().is_some_and(|desktop| desktop.running == 1));
    assert_eq!(list().trim(), "Desktop 1: 1 window, 1 running; shown on a screen now");
}
