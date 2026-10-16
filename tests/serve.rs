//! `qdesk serve`, the real program: it leaves the session it was started in, listens on the
//! socket of its user in the temporary folder it is given, welcomes a screen, and a second one
//! started beside it leaves the first alone.
//!
//! Every folder is the test's own: the temporary folder, the home and the configuration, so nothing
//! reaches the person's own background or settings.

use std::fs;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use qdesk::serve::socket;
use qdesk::serve::wire::{self, Greeting};

/// Generous: the program listens within milliseconds, a slow machine only makes it slower.
const BUDGET: Duration = Duration::from_secs(60);

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("qdesk-serve-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("tmp")).expect("the temporary folder is made");
        fs::create_dir_all(path.join("config")).expect("the configuration folder is made");
        Self(path)
    }

    fn socket(&self) -> PathBuf {
        socket::place(Some(self.0.join("tmp").as_os_str()), socket::own_uid())
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_qdesk"));
        command
            .arg("serve")
            .env_clear()
            .env("HOME", &self.0)
            .env("TMPDIR", self.0.join("tmp"))
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("LANG", "en_US.UTF-8")
            .stdin(Stdio::null());
        command
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Ends the background however the test ends.
struct Background(Child);

impl Drop for Background {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn wait_for_socket(path: &Path) -> UnixStream {
    let start = Instant::now();
    loop {
        if let Ok(stream) = UnixStream::connect(path) {
            return stream;
        }
        assert!(start.elapsed() < BUDGET, "nothing listened on {}", path.display());
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// The session a process is in, from the sixth field of `/proc/<pid>/stat`.
fn session_of(pid: u32) -> u32 {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).expect("the process is there");
    // The name in brackets may hold spaces; the fields after it are plain.
    let after = &stat[stat.rfind(')').expect("a name in brackets") + 2..];
    after.split(' ').nth(3).and_then(|field| field.parse().ok()).expect("a session")
}

#[test]
fn the_background_leaves_its_session_and_welcomes_a_screen_on_its_users_socket() {
    let scratch = Scratch::new("welcome");
    let background =
        Background(scratch.command().stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("started"));
    let mut screen = wait_for_socket(&scratch.socket());

    assert_eq!(wire::hello(&mut screen).expect("answered"), Greeting::Agreed);
    let pid = background.0.id();
    assert_eq!(session_of(pid), pid, "a session of its own, so a hangup of this one does not reach it");
    assert_ne!(session_of(std::process::id()), pid);
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(scratch.socket()).expect("the socket").permissions().mode();
    assert_eq!(mode & 0o777, 0o600, "only its user may open it");
}

#[test]
fn a_second_background_says_one_is_running_and_leaves_it_alone() {
    let scratch = Scratch::new("second");
    let _first = Background(scratch.command().stdout(Stdio::null()).stderr(Stdio::null()).spawn().expect("started"));
    drop(wait_for_socket(&scratch.socket()));

    let second = scratch.command().output().expect("the second one ran");
    assert_eq!(second.status.code(), Some(0), "{second:?}");
    let said = String::from_utf8_lossy(&second.stderr);
    assert!(said.contains("already running"), "{said}");

    let mut screen = UnixStream::connect(scratch.socket()).expect("the first one is still there");
    assert_eq!(wire::hello(&mut screen).expect("answered"), Greeting::Agreed);
}
