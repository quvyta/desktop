//! What the desktop costs a connection, measured on the real `qdesk` on a pseudo-terminal.
//!
//! Every number here is **bytes qdesk writes to the terminal**: the stream an SSH connection
//! carries, before any compression SSH does for itself. It is not bytes on the wire, and nothing
//! here reaches a network: the desktop is opened on a pseudo-terminal of this machine and what it
//! writes into it is counted.
//!
//! These tests are ignored, so `cargo test` never runs them: they start programs, take seconds
//! and measure a machine rather than check a behaviour. Run them on their own, one at a time,
//! and read the numbers:
//!
//! ```text
//! cargo test --release --test ssh_bytes -- --ignored --nocapture --test-threads 1
//! ```
//!
//! What came out of them is written down beside the budgets of the design's section 5.
//!
//! They touch nothing of the person running them: the desktop is opened with a home folder, a
//! settings folder and an application folder of the test's own under the system's temporary
//! folder, and every application it can open is a `/bin/sh` the test wrote itself.

#![cfg(unix)]

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use portable_pty::{CommandBuilder, PtySize, native_pty_system};

/// The size of the terminal most of these measurements are made on.
const SIZE: (u16, u16) = (100, 30);
/// A second terminal size, the one the design's section 5.3 does its arithmetic for.
const LARGE: (u16, u16) = (200, 50);

/// How long a phase may go on writing before it is called settled.
const AT_MOST: Duration = Duration::from_secs(5);
/// Nothing written for this long ends a phase.
const QUIET: Duration = Duration::from_millis(300);
/// How often the counter is looked at while waiting.
const STEP: Duration = Duration::from_millis(2);

/// A folder of its own under the system's temporary folder, removed when dropped.
///
/// It holds the home folder, the settings and the applications of one measurement, so the desktop
/// under measurement reads nothing of the person running it and writes nothing of theirs.
struct Scratch(PathBuf);

impl Scratch {
    /// A scratch folder holding a desktop with `count` applications that each run a quiet shell,
    /// one that pours out lines, and `settings` as the desktop's settings file.
    fn new(name: &str, settings: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-bytes-{}-{name}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let apps = path.join(".local/share/quvyta/desktop/apps");
        std::fs::create_dir_all(&apps).expect("an application folder");
        std::fs::create_dir_all(path.join(".config/quvyta/desktop")).expect("a settings folder");
        // A window runs its program for real, so every entry here runs a shell the test wrote.
        write(
            &apps.join("quiet.toml"),
            "name = \"Quiet\"\ncommand = [\"/bin/sh\", \"-c\", \"while :; do sleep 60; done\"]\ncategory = \"system\"\n",
        );
        // Lines that differ, so each frame really has something new to draw: a program writing
        // the same line over and over changes nothing on screen however fast it writes.
        write(
            &apps.join("loud.toml"),
            "name = \"Loud\"\ncommand = [\"/bin/sh\", \"-c\", \"i=0; while :; do i=$((i+1)); \
             echo $i the quick brown fox jumps over the lazy dog; done\"]\ncategory = \"system\"\n",
        );
        // An empty floor, no welcome line and no note on resizing, so the opening screen and the first
        // window are the same every run.
        write(
            &path.join(".config/quvyta/desktop/desktop.toml"),
            "icons = []\nwelcome_seen = true\nresize_hint_seen = true\n",
        );
        write(&path.join(".config/quvyta/desktop.conf"), settings);
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write(path: &Path, text: &str) {
    std::fs::write(path, text).unwrap_or_else(|why| panic!("writing {}: {why}", path.display()));
}

/// A `qdesk` running on a pseudo-terminal, with everything it has written counted.
struct Desktop {
    written: Arc<AtomicU64>,
    writer: Box<dyn Write + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    pid: u32,
}

impl Desktop {
    /// Opens the desktop of `scratch` on a terminal of `size`, and answers the question it asks
    /// the terminal before it draws anything.
    fn open(scratch: &Scratch, (columns, rows): (u16, u16)) -> Self {
        let home = &scratch.0;
        let pair = native_pty_system()
            .openpty(PtySize { rows, cols: columns, pixel_width: 0, pixel_height: 0 })
            .expect("a pseudo-terminal");
        let mut command = CommandBuilder::new(env!("CARGO_BIN_EXE_qdesk"));
        // Nothing of the person running the test reaches the desktop: it is given its whole
        // environment here, and the folders in it are the scratch folder's.
        command.env_clear();
        command.env("HOME", home);
        command.env("XDG_CONFIG_HOME", home.join(".config"));
        command.env("XDG_DATA_HOME", home.join(".local/share"));
        command.env("XDG_DATA_DIRS", home.join("share"));
        command.env("PATH", "/usr/bin:/bin");
        command.env("SHELL", "/bin/sh");
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("LANG", "C.UTF-8");
        // The same glyphs whatever fonts the machine has, so the byte counts can be compared.
        command.env("QUVYTA_ICONS", "unicode");
        command.cwd(home);
        let child = pair.slave.spawn_command(command).expect("qdesk starts");
        drop(pair.slave);
        let pid = child.process_id().expect("the desktop has a process id");
        let mut reader = pair.master.try_clone_reader().expect("a reader");
        let writer = pair.master.take_writer().expect("a writer");
        let written = Arc::new(AtomicU64::new(0));
        let counted = Arc::clone(&written);
        std::thread::spawn(move || {
            let mut buffer = [0u8; 65536];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                counted.fetch_add(count as u64, Ordering::Relaxed);
            }
        });
        let mut desktop = Self { written, writer, child, pid };
        // The desktop asks the terminal what it can do and draws nothing until it hears back.
        // A terminal that speaks for itself is what a person has at the other end of an SSH
        // connection, so the test answers as one.
        desktop.settle();
        desktop.send(b"\x1b[?62;1;2;6;9;15;22c");
        desktop.settle();
        desktop
    }

    /// How many bytes the desktop has written to the terminal since it started.
    fn written(&self) -> u64 {
        self.written.load(Ordering::Relaxed)
    }

    /// Sends `bytes` to the desktop as a terminal would send what a person typed or clicked.
    fn send(&mut self, bytes: &[u8]) {
        self.writer.write_all(bytes).expect("the desktop reads its terminal");
        self.writer.flush().expect("the desktop reads its terminal");
    }

    /// Waits until the desktop has written nothing for [`QUIET`], and answers how many bytes it
    /// wrote in that time. It gives up after [`AT_MOST`], which a program that keeps writing
    /// reaches on purpose.
    fn settle(&self) -> u64 {
        self.settle_within(QUIET, AT_MOST)
    }

    /// The same, with a quiet and a bound of its own: driving the desktop while a program pours
    /// out lines never reaches a still screen, so those steps wait a short bound instead.
    fn settle_within(&self, quiet: Duration, at_most: Duration) -> u64 {
        let before = self.written();
        let giving_up = Instant::now() + at_most;
        let mut last = self.written();
        let mut quiet_since = Instant::now();
        while Instant::now() < giving_up {
            std::thread::sleep(STEP);
            let now = self.written();
            if now == last {
                if quiet_since.elapsed() >= quiet {
                    break;
                }
            } else {
                last = now;
                quiet_since = Instant::now();
            }
        }
        self.written() - before
    }

    /// Lets the desktop write for `span` and answers how many bytes it wrote.
    fn over(&self, span: Duration) -> u64 {
        let before = self.written();
        std::thread::sleep(span);
        self.written() - before
    }

    /// How long the desktop took to write anything after this moment, `None` when it wrote
    /// nothing within `bound`.
    fn answer_within(&self, bound: Duration) -> Option<Duration> {
        let before = self.written();
        let start = Instant::now();
        while start.elapsed() < bound {
            if self.written() != before {
                return Some(start.elapsed());
            }
            std::thread::sleep(Duration::from_micros(200));
        }
        None
    }

    /// Opens the application called `name` through the launcher: the desktop's own keys, the
    /// launcher, its search field and the entry it finds.
    fn open_window(&mut self, name: &str) {
        // A window already open may hold a program that never stops writing, so each step here
        // waits for a short quiet and then goes on, rather than for a screen that stays still.
        let (quiet, at_most) = (Duration::from_millis(120), Duration::from_millis(900));
        self.send(b"\x1b\x00"); // ctrl+alt+space: the keys are the desktop's
        self.settle_within(quiet, at_most);
        self.send(b" "); // the launcher
        self.settle_within(quiet, at_most);
        self.send(name.as_bytes());
        self.settle_within(quiet, at_most);
        self.send(b"\r");
        self.settle_within(quiet, at_most);
    }

    /// How much memory the desktop itself holds, in kibibytes.
    fn resident_kib(&self) -> u64 {
        proc_field(self.pid, "VmRSS:")
    }

    /// The processor time the desktop itself has used, in seconds.
    fn processor_seconds(&self) -> f64 {
        let text = std::fs::read_to_string(format!("/proc/{}/stat", self.pid)).expect("the desktop is running");
        let after_name = &text[text.rfind(')').expect("the name of a process ends in a bracket") + 2..];
        let fields: Vec<&str> = after_name.split_whitespace().collect();
        let ticks: f64 =
            fields[11].parse::<f64>().expect("user time") + fields[12].parse::<f64>().expect("system time");
        // The clock of /proc is a hundred ticks a second on every Linux qdesk is built for.
        ticks / 100.0
    }

    /// How many programs the desktop has started under it, counted through the process tree.
    fn programs(&self) -> usize {
        children(self.pid).len()
    }

    /// The memory the desktop and every program it started hold together, in kibibytes.
    fn tree_resident_kib(&self) -> u64 {
        self.resident_kib() + children(self.pid).iter().map(|pid| proc_field(*pid, "VmRSS:")).sum::<u64>()
    }
}

impl Drop for Desktop {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A number from `/proc/<pid>/status`, nought when the process is gone.
fn proc_field(pid: u32, name: &str) -> u64 {
    let Ok(text) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
        return 0;
    };
    text.lines()
        .find(|line| line.starts_with(name))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// Every process under `pid`, however deep.
fn children(pid: u32) -> Vec<u32> {
    let mut found = Vec::new();
    let mut stack = vec![pid];
    while let Some(parent) = stack.pop() {
        let Ok(tasks) = std::fs::read_dir(format!("/proc/{parent}/task")) else {
            continue;
        };
        for task in tasks.flatten() {
            let Ok(text) = std::fs::read_to_string(task.path().join("children")) else {
                continue;
            };
            for child in text.split_whitespace().filter_map(|pid| pid.parse::<u32>().ok()) {
                found.push(child);
                stack.push(child);
            }
        }
    }
    found
}

/// The press, the steps and the release of a drag from the title strip of the window that opened
/// in the middle of the screen, and the bytes the steps cost.
///
/// Each step waits for the screen to settle before the next one, so every step really is a frame
/// of its own: a measurement of how much one step of a drag costs, not of how fast the machine
/// was that second.
fn drag(desktop: &mut Desktop, from: u16, row: u16, steps: u16) -> (u64, u64) {
    desktop.settle();
    desktop.send(format!("\x1b[<0;{from};{row}M").as_bytes());
    desktop.settle();
    let before = desktop.written();
    for step in 1..=steps {
        desktop.send(format!("\x1b[<32;{};{row}M", from + step).as_bytes());
        desktop.settle();
    }
    let moving = desktop.written() - before;
    let dropped = desktop.written();
    desktop.send(format!("\x1b[<0;{};{row}m", from + steps).as_bytes());
    desktop.settle();
    (moving, desktop.written() - dropped)
}

/// The three runs every measurement is made of, so the note can say whether the number moves.
const RUNS: usize = 3;

/// The row of the title strip of the window that opens in the middle of a screen of `rows`,
/// counted from one as a terminal counts its rows.
///
/// A window opens two thirds of the floor high, in the middle of it, and its title strip is its
/// top row. The floor is the screen without the dock's row.
fn title_row(rows: u16) -> u16 {
    let floor = rows - 1;
    (floor - floor / 3 * 2) / 2 + 1
}

#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn opening_the_desktop_is_the_cost_of_one_screen() {
    for (columns, rows) in [SIZE, LARGE] {
        let mut bytes = Vec::new();
        for _ in 0..RUNS {
            let scratch = Scratch::new("opening", "");
            let desktop = Desktop::open(&scratch, (columns, rows));
            // Opening settles twice: what the desktop has written by now is the question it asked
            // the terminal and then the whole first screen.
            bytes.push(desktop.written());
        }
        println!("opening {columns}x{rows}: {bytes:?} bytes written to the terminal");
        assert!(bytes.iter().all(|count| *count == bytes[0]), "the first screen is the same every run: {bytes:?}");
    }
}

#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn dragging_a_window_costs_less_as_a_ghost_than_alive() {
    let mut costs = Vec::new();
    for style in ["live", "ghost"] {
        for (columns, rows) in [SIZE, LARGE] {
            let mut steps = Vec::new();
            for _ in 0..RUNS {
                let scratch = Scratch::new("drag", &format!("drag-style = \"{style}\"\n"));
                let mut desktop = Desktop::open(&scratch, (columns, rows));
                desktop.open_window("Quiet");
                let (moving, _) = drag(&mut desktop, columns / 3, title_row(rows), 20);
                steps.push(moving / 20);
            }
            println!("drag {style} {columns}x{rows}: {steps:?} bytes a step");
            costs.push((style, columns, steps[0]));
        }
    }
    for (columns, ..) in [SIZE, LARGE] {
        let live = costs.iter().find(|(style, width, _)| *style == "live" && *width == columns).expect("live");
        let ghost = costs.iter().find(|(style, width, _)| *style == "ghost" && *width == columns).expect("ghost");
        assert!(ghost.2 < live.2, "a ghost costs less than the window itself at {columns} columns: {ghost:?} {live:?}");
    }
}

#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn the_frame_cap_holds_back_a_program_pouring_out_lines() {
    let span = Duration::from_secs(3);
    let mut measured = Vec::new();
    for cap in ["", "frame-cap = 5\n", "frame-cap = 20\n", "frame-cap = 60\n", "frame-cap = 240\n"] {
        let mut rates = Vec::new();
        for _ in 0..RUNS {
            let scratch = Scratch::new("loud", cap);
            let mut desktop = Desktop::open(&scratch, SIZE);
            desktop.open_window("Loud");
            // The window is drawing by now; the measurement is of the flow, not of the opening.
            desktop.over(Duration::from_secs(1));
            rates.push(desktop.over(span) * 1000 / span.as_millis() as u64);
        }
        let name = if cap.is_empty() { "no number chosen (60 here, 20 over SSH)" } else { cap.trim() };
        println!("a program pouring out lines, {name}: {rates:?} bytes a second");
        measured.push(rates[0]);
    }
    let (none, five, twenty) = (measured[0], measured[1], measured[2]);
    assert!(five < twenty, "five frames a second costs less than twenty: {five} against {twenty}");
    assert!(twenty < none, "the cap SSH gets costs less than the one this machine gets: {twenty} against {none}");
}

#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn a_typed_key_never_waits_for_the_frame_cap() {
    let scratch = Scratch::new("echo", "frame-cap = 1\n");
    let mut desktop = Desktop::open(&scratch, SIZE);
    // The launcher's search field is the desktop's own screen, so what it draws answers the key
    // itself; that is what the cap must not hold back.
    desktop.send(b"\x1b\x00");
    desktop.settle();
    desktop.send(b" ");
    desktop.settle();
    let mut answers = Vec::new();
    for key in b"quiet" {
        desktop.settle();
        desktop.send(&[*key]);
        let answer = desktop.answer_within(Duration::from_secs(3)).expect("a typed key is answered");
        answers.push(answer.as_micros());
    }
    println!("a key typed at one frame a second is answered in {answers:?} microseconds");
    assert!(
        answers.iter().all(|answer| *answer < 100_000),
        "a key press is not held back by the cap of one frame a second: {answers:?} microseconds"
    );

    // The other side of the promise: what the cap does hold back is the desktop's own work.
    let scratch = Scratch::new("held", "frame-cap = 1\n");
    let mut desktop = Desktop::open(&scratch, SIZE);
    desktop.open_window("Loud");
    desktop.over(Duration::from_millis(1500));
    let mut gaps = Vec::new();
    for _ in 0..5 {
        let gap = desktop.answer_within(Duration::from_secs(3)).expect("the program keeps writing");
        gaps.push(gap.as_millis());
        // The frame that just arrived is read out of the way before the next gap is timed.
        desktop.over(Duration::from_millis(20));
    }
    println!("a program writing at one frame a second is drawn every {gaps:?} milliseconds");
    assert!(gaps.iter().any(|gap| *gap > 500), "the cap really holds a program's own frames back: {gaps:?}");
}

#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn memory_and_processor_with_ten_and_twenty_windows() {
    for count in [0, 10, 20] {
        let scratch = Scratch::new("windows", "");
        let mut desktop = Desktop::open(&scratch, (120, 40));
        for _ in 0..count {
            desktop.open_window("Quiet");
        }
        desktop.settle();
        let programs = desktop.programs();
        let before = desktop.processor_seconds();
        let idle = Duration::from_secs(5);
        let quiet = desktop.over(idle);
        let busy = (desktop.processor_seconds() - before) / idle.as_secs_f64() * 100.0;
        println!(
            "{count} windows: qdesk holds {:.1} MB, with its programs {:.1} MB, \
             {programs} processes under it, {busy:.2} per cent of a processor and {quiet} bytes \
             written while nothing happens for {} seconds",
            desktop.resident_kib() as f64 / 1024.0,
            desktop.tree_resident_kib() as f64 / 1024.0,
            idle.as_secs(),
        );
        assert!(programs >= count, "every window really started its program: {programs} for {count} windows");
        assert!(busy < 1.0, "an idle desktop of {count} windows uses less than one per cent of a processor: {busy}");
    }
}

/// Four windows pouring out lines at once, which is what the runtime's frame limit is for.
///
/// Each window holds its own output back to one report per frame of the cap, so one window is
/// already paced without the runtime knowing anything. Four windows are four such streams, and
/// only the limit the runtime holds — one for the whole screen — keeps them from asking for four
/// times the frames. This is the measurement that says whether the setting reaches the runtime.
#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn four_windows_pouring_out_lines_cost_what_one_screen_costs() {
    let span = Duration::from_secs(3);
    for windows in [1, 4] {
        let mut rates = Vec::new();
        for _ in 0..RUNS {
            let scratch = Scratch::new("four", "frame-cap = 5\n");
            let mut desktop = Desktop::open(&scratch, (120, 40));
            for _ in 0..windows {
                desktop.open_window("Loud");
            }
            desktop.over(Duration::from_secs(1));
            rates.push(desktop.over(span) * 1000 / span.as_millis() as u64);
        }
        println!("{windows} windows pouring out lines at five frames a second: {rates:?} bytes a second");
    }
}

/// A drag driven as fast as a hand really moves it, which is the most a person can ask of a
/// connection at once.
///
/// The other drag measurement lets the screen settle between steps, so each step is a frame of
/// its own and the cap has nothing to merge. A hand sends far more steps a second than that, and
/// this is where the two answers of the design's section 5.3 can be told apart: the frame cap
/// (decision 2) and the ghost (decision 4).
///
/// A frame that answers what the person did is never held back — that is the runtime's promise,
/// and it is what keeps typing from lagging. A pointer being dragged is input too, so the cap
/// merges none of it. What makes a drag cheap is the ghost, and only the ghost.
#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn a_drag_at_a_hand_pace_is_paid_for_by_the_ghost_and_not_by_the_cap() {
    let mut measured = Vec::new();
    for style in ["live", "ghost"] {
        for cap in ["frame-cap = 5\n", "frame-cap = 60\n"] {
            let mut counts = Vec::new();
            for _ in 0..RUNS {
                let scratch = Scratch::new("hand-drag", &format!("{cap}drag-style = \"{style}\"\n"));
                let mut desktop = Desktop::open(&scratch, SIZE);
                desktop.open_window("Quiet");
                desktop.settle();
                let row = title_row(SIZE.1);
                let from = SIZE.0 / 3;
                desktop.send(format!("\x1b[<0;{from};{row}M").as_bytes());
                desktop.settle();
                let before = desktop.written();
                // Sixty steps over two seconds: a pointer moved to and fro at a hand's pace.
                for step in 1..=60u16 {
                    desktop.send(format!("\x1b[<32;{};{row}M", from + step % 20).as_bytes());
                    std::thread::sleep(Duration::from_millis(33));
                }
                desktop.settle();
                counts.push(desktop.written() - before);
                desktop.send(format!("\x1b[<0;{};{row}m", from + 20).as_bytes());
                desktop.settle();
            }
            println!("a two second drag, {style}, {}: {counts:?} bytes", cap.trim());
            measured.push(counts[0]);
        }
    }
    let (live_slow, live_fast, ghost_slow) = (measured[0], measured[1], measured[2]);
    assert!(ghost_slow < live_slow, "the ghost is what makes a drag cheap: {ghost_slow} against {live_slow}");
    let apart = live_slow.abs_diff(live_fast) * 100 / live_fast;
    assert!(apart < 20, "the cap merges none of a drag, because a drag is input: {live_slow} against {live_fast}");
}

/// What a desktop nobody is touching costs a connection.
///
/// The design's section 5.3 asks for nought bytes while nothing happens and a dozen cells when
/// the clock changes. The span is longer than a minute, so exactly one minute change falls inside
/// it and what is written is that change and nothing else.
#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn a_desktop_nobody_touches_writes_only_its_clock() {
    let span = Duration::from_secs(70);
    for count in [0, 10] {
        let scratch = Scratch::new("idle", "");
        let mut desktop = Desktop::open(&scratch, SIZE);
        for _ in 0..count {
            desktop.open_window("Quiet");
        }
        desktop.settle();
        let written = desktop.over(span);
        println!(
            "{count} windows, nothing happening for {} seconds: {written} bytes, one minute change among them",
            span.as_secs()
        );
        assert!(written < 4_000, "an untouched desktop costs a connection almost nothing: {written} bytes");
    }
}

/// The memory a window's remembered lines really cost: windows whose programs have filled the
/// scrollback the settings allow, which is what the design's section 5.2 weighs.
#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn memory_with_ten_windows_whose_scrollback_is_full() {
    let filling = Duration::from_secs(20);
    for lines in ["scrollback = 2000\n", "scrollback = 10000\n"] {
        let scratch = Scratch::new("scrollback", lines);
        let mut desktop = Desktop::open(&scratch, (120, 40));
        for _ in 0..10 {
            desktop.open_window("Loud");
        }
        desktop.over(filling);
        println!(
            "ten windows pouring out lines for {} seconds, {}: qdesk holds {:.1} MB",
            filling.as_secs(),
            lines.trim(),
            desktop.resident_kib() as f64 / 1024.0,
        );
    }
}

/// Going to another workspace and back: one frame each way (design 3.10).
///
/// Two quiet windows stand on the first workspace. Going to the empty second one takes them off
/// the screen and puts the floor there instead; coming back draws them again. The digit is
/// measured on its own: the key that takes the keys to the desktop draws the hint row, and that
/// is desktop mode's cost, not the switch's.
#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn going_to_another_workspace_and_back_is_one_frame_each_way() {
    let scratch = Scratch::new("spaces", "");
    let mut desktop = Desktop::open(&scratch, SIZE);
    let screen = desktop.written();
    desktop.open_window("Quiet");
    desktop.open_window("Quiet");
    desktop.settle();
    desktop.send(b"\x1b\x00"); // ctrl+alt+space
    desktop.settle();
    desktop.send(b"2");
    let away = desktop.settle();
    desktop.send(b"\x1b\x00");
    desktop.settle();
    desktop.send(b"1");
    let back = desktop.settle();
    println!(
        "workspaces {}x{}: going to an empty one {away} bytes, coming back to two windows {back} bytes; \
         the whole first screen was {screen} bytes",
        SIZE.0, SIZE.1
    );
    assert!(away > 0 && back > 0, "both switches drew something");
    assert!(away < screen * 2 && back < screen * 2, "a switch costs about a screen at most");
}

/// What the floor's pattern costs a connection (design 3.9): the first screen at both sizes, and
/// one step of a window dragged over the floor, as a ghost (the default) and alive.
///
/// The floor is drawn again only where something on it changes, so a drag over a patterned floor
/// pays for the cells it uncovers and nothing more: the gradient a colour change on each row the
/// window leaves, the dots a mark where a blank would have been.
#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn the_floor_pattern_costs_little_on_the_first_screen_and_on_a_drag() {
    let mut plain = Vec::new();
    for style in ["plain", "gradient", "dots", "gradient-dots"] {
        let setting = if style == "plain" { String::new() } else { format!("floor-style = \"{style}\"\n") };
        let mut screens = Vec::new();
        for size in [SIZE, LARGE] {
            let scratch = Scratch::new("floor", &setting);
            let desktop = Desktop::open(&scratch, size);
            screens.push(desktop.written());
        }
        let mut steps = Vec::new();
        for drag_style in ["ghost", "live"] {
            let scratch = Scratch::new("floor-drag", &format!("{setting}drag-style = \"{drag_style}\"\n"));
            let mut desktop = Desktop::open(&scratch, SIZE);
            desktop.open_window("Quiet");
            let (moving, _) = drag(&mut desktop, SIZE.0 / 3, title_row(SIZE.1), 20);
            steps.push(moving / 20);
        }
        println!(
            "floor {style}: first screen {} bytes at {}x{}, {} bytes at {}x{}; a drag step at {}x{} {} bytes as a ghost, {} alive",
            screens[0], SIZE.0, SIZE.1, screens[1], LARGE.0, LARGE.1, SIZE.0, SIZE.1, steps[0], steps[1],
        );
        if style == "plain" {
            plain = screens.clone();
        } else {
            // A pattern is a few colour changes and marks, never a second screen.
            assert!(screens[0] < plain[0] * 2, "{style} at {}x{}: {} against {}", SIZE.0, SIZE.1, screens[0], plain[0]);
        }
    }
}

#[test]
#[ignore = "measures the real program on a pseudo-terminal; run it on its own"]
fn a_clock_widget_writes_nothing_between_minutes_unless_it_shows_seconds() {
    // Only clocks: the system widget would read this machine's /proc, and a test reads nothing of
    // the machine it runs on. What the system widget costs is measured in tests/widgets.rs over a
    // /proc the test writes.
    let mut quiet = Vec::new();
    for (name, options) in [("clock", ""), ("clock with seconds", "seconds = true\n")] {
        let scratch = Scratch::new("clock", "");
        write(
            &scratch.0.join(".config/quvyta/desktop/desktop.toml"),
            &format!(
                "icons = []\nwelcome_seen = true\nresize_hint_seen = true\n\
                 [[widgets]]\nkind = \"clock\"\nplace = [6, 0]\n{options}"
            ),
        );
        let desktop = Desktop::open(&scratch, SIZE);
        let span = Duration::from_secs(5);
        let written = desktop.over(span);
        println!("{name}: {written} bytes in {} seconds idle at {}x{}", span.as_secs(), SIZE.0, SIZE.1);
        quiet.push(written);
    }
    // Five seconds may cross the turn of a minute once; seconds are drawn every one of them.
    assert!(quiet[1] > quiet[0], "seconds cost more than minutes: {quiet:?}");
}
