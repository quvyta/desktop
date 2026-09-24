//! The status strip on the dock (design 3.10): what it shows beside the machine name, how it gives
//! way on a narrow row, what a click on its items opens, and the Settings switch that takes it away.
//!
//! The machine it reads is a folder the test wrote, and tmux, btop and htop are scripts the test
//! wrote on a `PATH` of its own: nothing here reads the machine it runs on or starts the person's
//! programs.

mod support;

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use qdesk::app::Desk;
use qdesk::apps::Environment;
use qdesk::desktop::Desktop;
use qdesk::gadgets::{Gadget, Kind};
use qdesk::status::{Probe, SAMPLE_EVERY};
use qframe::color::ColorDepth;
use qframe::event::{MouseButton, MouseKind};
use qframe::icons::GlyphMode;
use qframe::prelude::*;
use support::{BUDGET, HARMLESS, ICONS, MACHINE, base, decoration, draw, screen};

/// A folder of its own under the system's temporary folder, removed when dropped: a `/proc`, a
/// power supply folder, a `bin` folder with the programs the test wrote, and a settings folder.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-strip-{}-{name}-{number}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(path.join("bin")).expect("a scratch folder");
        fs::create_dir_all(path.join("config")).expect("a settings folder");
        let scratch = Self(path);
        scratch.stat(0, 0);
        scratch.memory(41);
        scratch.network(0);
        scratch
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
        fs::write(&path, text).expect("a file");
    }

    /// `/proc/stat` whose processors have been busy `busy` ticks and idle `idle` ticks.
    fn stat(&self, busy: u64, idle: u64) {
        self.write("proc/stat", &format!("cpu  {busy} 0 0 {idle} 0 0 0 0 0 0\n"));
    }

    /// `/proc/meminfo` with `used` of 100 units in use.
    fn memory(&self, used: u64) {
        let total = 8_000_000;
        let available = total - total * used / 100;
        self.write("proc/meminfo", &format!("MemTotal: {total} kB\nMemFree: 1 kB\nMemAvailable: {available} kB\n"));
    }

    /// `/proc/net/dev` with `bytes` received on one interface and nothing sent.
    fn network(&self, bytes: u64) {
        let heading = "Inter-|   Receive\n face |bytes\n";
        self.write(
            "proc/net/dev",
            &format!(
                "{heading}    lo: 999 0 0 0 0 0 0 0 999 0 0 0 0 0 0 0\n  eth0: {bytes} 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0\n"
            ),
        );
    }

    fn battery(&self, percent: u8) {
        self.write("power/BAT0/type", "Battery\n");
        self.write("power/BAT0/capacity", &format!("{percent}\n"));
        self.write("power/BAT0/status", "Discharging\n");
    }

    fn bin(&self) -> PathBuf {
        self.0.join("bin")
    }

    /// A program called `name` on the test's `PATH` that runs `body`.
    fn program(&self, name: &str, body: &str) {
        let path = self.bin().join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).expect("a program is written");
        make_executable(&path);
    }

    /// A tmux with the sessions `dev` and `devops`. Asked anything else, it writes what it was
    /// asked and what `TMUX` it was given (`unset` when none) to the file it returns, whole.
    fn tmux(&self) -> PathBuf {
        let said = self.0.join("tmux-said");
        let part = self.0.join("tmux-said.part");
        self.program(
            "tmux",
            &format!(
                "if [ \"$1\" = list-sessions ]; then printf 'dev\\ndevops\\n'; exit 0; fi\n\
                 printf '%s\\n' \"$*\" \"TMUX=${{TMUX-unset}}\" > '{part}' && mv '{part}' '{said}'",
                part = part.display(),
                said = said.display()
            ),
        );
        said
    }

    /// A process monitor called `name` that leaves a file behind when it runs, and that file.
    fn monitor(&self, name: &str) -> PathBuf {
        let ran = self.0.join(format!("{name}-ran"));
        self.program(name, &format!("touch '{}'", ran.display()));
        ran
    }

    fn probe(&self) -> Probe {
        Probe::rooted(self.0.join("proc"), self.0.join("power"), Some(self.bin().into_os_string()))
    }

    fn config(&self) -> PathBuf {
        self.0.join("config")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[cfg(unix)]
fn make_executable(program: &Path) {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(program, fs::Permissions::from_mode(0o755)).expect("the program can be run");
}

/// The usual desktop reading the machine in `scratch`, finding programs on its `bin` and keeping
/// its settings in its settings folder, with `widgets` on the floor.
fn strip_desk(scratch: &Scratch, widgets: Vec<Gadget>, width: u16, height: u16) -> Harness<Desk> {
    let loaded = qdesk::settings::load_in(&scratch.config());
    let apps = Environment {
        shell: Some(PathBuf::from(HARMLESS)),
        path: Some(scratch.bin().into_os_string()),
        ..Environment::default()
    };
    let desktop = Desktop {
        icons: ICONS.map(str::to_owned).to_vec(),
        welcome_seen: true,
        resize_hint_seen: true,
        widgets,
        ..Desktop::default()
    };
    let clock = Box::new(|| support::MOMENT * 1_000);
    let app = base(clock)
        .apps(apps)
        .desktop(desktop)
        .settings(loaded.settings, loaded.prefs, loaded.diagnostics)
        .probe(scratch.probe());
    draw(app, width, height)
}

/// The dock's row: the last one, where the dock stands by default.
fn dock(harness: &Harness<Desk>) -> String {
    screen(harness).last().cloned().unwrap_or_default()
}

/// Lets the harness's clock run `by`, a reading at a time, drawing after each.
fn pass(harness: &mut Harness<Desk>, by: Duration) {
    let mut left = by;
    while !left.is_zero() {
        let step = left.min(SAMPLE_EVERY);
        harness.advance(step);
        left -= step;
    }
}

/// Renders until `ready` is happy, or gives up after [`BUDGET`] and says what was on the screen.
fn until(harness: &mut Harness<Desk>, what: &str, ready: impl Fn(&Harness<Desk>) -> bool) {
    let deadline = Instant::now() + BUDGET;
    loop {
        if ready(harness) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} never came:\n{}", harness.screen());
        harness.render();
    }
}

/// A machine the strip has read twice: two tmux sessions, a network rate, the processor a tenth
/// busy, the memory 41% full and a battery at 87%. The rate is measured over the time that really
/// passed between the two readings, so its number is not the test's to say.
fn read_twice(scratch: &Scratch, width: u16, height: u16) -> Harness<Desk> {
    scratch.tmux();
    scratch.battery(87);
    scratch.stat(0, 0);
    scratch.network(0);
    let mut harness = strip_desk(scratch, Vec::new(), width, height);
    scratch.stat(100, 900);
    scratch.network(3_072);
    pass(&mut harness, SAMPLE_EVERY);
    harness
}

#[test]
fn the_dock_shows_the_machines_readings_before_its_name_and_clock() {
    let scratch = Scratch::new("wide");
    let harness = read_twice(&scratch, 120, 24);
    let row = dock(&harness);
    println!("120x24 dock: {row:?}");
    // tmux, the network, the processor, the memory and the battery, then the name and the clock.
    let order = ["❯ 2", "◎ ", "/s", "▣  10%", "◰  41%", "○  87%", MACHINE, "14:32"];
    let mut from = 0;
    for part in order {
        let at = row[from..].find(part).unwrap_or_else(|| panic!("{part} after column {from}: {row:?}"));
        from += at + part.len();
    }
    // The workspace marks keep their place beside the launcher button.
    assert!(row.contains("1 ○ ○ ○"), "{row:?}");
    // The calm items are written in the clock's secondary tone.
    let (x, y) = harness.find("▣  10%").expect("the processor");
    let (clock_x, clock_y) = harness.find("14:32").expect("the clock");
    let tone = |x: i32, y: i32| harness.fg(u16::try_from(x).unwrap_or(0), u16::try_from(y).unwrap_or(0));
    assert_eq!(tone(x + 2, y), tone(clock_x, clock_y), "the secondary tone");
}

#[test]
fn at_80_by_24_an_empty_desktop_shows_the_whole_strip() {
    let scratch = Scratch::new("eighty");
    let harness = read_twice(&scratch, 80, 24);
    let row = dock(&harness);
    println!("80x24 dock: {row:?}");
    for part in ["❯ 2", "◎ ", "▣  10%", "◰  41%", "○  87%", MACHINE, "14:32"] {
        assert!(row.contains(part), "{part}: {row:?}");
    }
}

#[test]
fn a_narrow_dock_lets_the_strip_go_from_the_left_and_keeps_the_name_and_the_clock() {
    for (width, kept, gone) in [
        (60, &["◰  41%", "○  87%"][..], &["❯ 2", "◎", "▣"][..]),
        (50, &["○  87%"][..], &["❯ 2", "◎", "▣", "◰"][..]),
        (40, &[][..], &["❯ 2", "◎", "▣", "◰", "○  87%"][..]),
    ] {
        let scratch = Scratch::new("narrow");
        let harness = read_twice(&scratch, width, 24);
        let row = dock(&harness);
        println!("{width}x24 dock: {row:?}");
        for part in kept {
            assert!(row.contains(part), "{width}: {part} stays: {row:?}");
        }
        for part in gone {
            assert!(!row.contains(part), "{width}: {part} went first: {row:?}");
        }
        assert!(row.contains(MACHINE) && row.contains("14:32"), "{width}: {row:?}");
    }
}

#[test]
fn a_full_memory_says_so_with_a_mark_and_the_themes_warning_and_danger_colours() {
    let scratch = Scratch::new("tones");
    scratch.memory(80);
    let harness = strip_desk(&scratch, Vec::new(), 120, 24);
    let (x, y) = harness.find("◰ ▲  80%").unwrap_or_else(|| panic!("a warning mark:\n{}", dock(&harness)));
    let at = |x: i32| harness.fg(u16::try_from(x).unwrap_or(0), u16::try_from(y).unwrap_or(0));
    let theme = harness.env().theme();
    assert_eq!(at(x + 4), theme.color("warning"), "the warning colour, as well as the mark");

    scratch.memory(95);
    let harness = strip_desk(&scratch, Vec::new(), 120, 24);
    let (x, y) = harness.find("◰ ✕  95%").unwrap_or_else(|| panic!("an alarm mark:\n{}", dock(&harness)));
    let colour = harness.fg(u16::try_from(x + 4).unwrap_or(0), u16::try_from(y).unwrap_or(0));
    assert_eq!(colour, harness.env().theme().color("danger"), "the danger colour");
}

#[test]
fn a_click_on_the_tmux_item_opens_the_sessions_menu_and_one_opens_attached_without_tmux_set() {
    let scratch = Scratch::new("attach");
    let said = scratch.tmux();
    let mut harness = strip_desk(&scratch, Vec::new(), 120, 24);
    let (x, y) = harness.find("❯ 2").unwrap_or_else(|| panic!("the tmux item:\n{}", dock(&harness)));
    harness.click(x, y);
    assert!(harness.find("dev ").is_some(), "the sessions are offered:\n{}", harness.screen());
    let (x, y) = harness.find("devops").unwrap_or_else(|| panic!("the sessions are offered:\n{}", harness.screen()));
    harness.click(x, y);
    assert_eq!(harness.app().windows().len(), 1, "a window opened:\n{}", harness.screen());
    until(&mut harness, "the attach", |_| said.exists());
    let words = fs::read_to_string(&said).expect("tmux wrote what it was asked");
    assert_eq!(words, "attach -t =devops\nTMUX=unset\n", "exactly that session, with no TMUX at all");
    assert!(harness.find("dev ").is_none(), "the menu went away:\n{}", harness.screen());
}

#[test]
fn a_right_click_on_the_tmux_item_offers_the_same_menu() {
    let scratch = Scratch::new("menu");
    let said = scratch.tmux();
    let mut harness = strip_desk(&scratch, Vec::new(), 120, 24);
    let (x, y) = harness.find("❯ 2").unwrap_or_else(|| panic!("the tmux item:\n{}", dock(&harness)));
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    harness.click_text("dev ");
    until(&mut harness, "the attach", |_| said.exists());
    let words = fs::read_to_string(&said).expect("tmux wrote what it was asked");
    assert!(words.starts_with("attach -t =dev\n"), "{words:?}");
}

#[test]
fn a_second_click_on_the_tmux_item_puts_the_sessions_menu_away() {
    let scratch = Scratch::new("again");
    scratch.tmux();
    let mut harness = strip_desk(&scratch, Vec::new(), 120, 24);
    let (x, y) = harness.find("❯ 2").unwrap_or_else(|| panic!("the tmux item:\n{}", dock(&harness)));
    harness.click(x, y);
    assert!(harness.find("devops").is_some(), "the first click opens the menu:\n{}", harness.screen());
    // The item sits inside its name's tooltip, which takes the pointer itself; the same click
    // that opened the menu closes it.
    harness.click(x, y);
    assert!(harness.find("devops").is_none(), "the second click closes it:\n{}", harness.screen());
    assert!(harness.app().windows().is_empty(), "and attaches nothing");
    harness.click(x, y);
    assert!(harness.find("devops").is_some(), "a third opens it again:\n{}", harness.screen());
}

#[test]
fn escape_puts_the_sessions_menu_away() {
    let scratch = Scratch::new("escape");
    scratch.tmux();
    let mut harness = strip_desk(&scratch, Vec::new(), 120, 24);
    let (x, y) = harness.find("❯ 2").expect("the tmux item");
    harness.click(x, y);
    assert!(harness.find("devops").is_some(), "{}", harness.screen());
    harness.press("esc");
    assert!(harness.find("devops").is_none(), "{}", harness.screen());
    assert!(harness.app().windows().is_empty());
}

#[test]
fn a_click_on_the_processor_opens_btop_else_htop_else_nothing() {
    let scratch = Scratch::new("monitor");
    let htop = scratch.monitor("htop");
    let btop = scratch.monitor("btop");
    let mut harness = read_twice(&scratch, 120, 24);
    let (x, y) = harness.find("▣  10%").expect("the processor");
    harness.click(x, y);
    until(&mut harness, "btop", |_| btop.exists());
    assert!(!htop.exists(), "btop comes first");

    fs::remove_file(scratch.bin().join("btop")).expect("btop goes");
    let mut harness = read_twice(&scratch, 120, 24);
    let (x, y) = harness.find("◰  41%").expect("the memory");
    harness.click(x, y);
    until(&mut harness, "htop", |_| htop.exists());

    fs::remove_file(scratch.bin().join("htop")).expect("htop goes");
    let mut harness = read_twice(&scratch, 120, 24);
    let (x, y) = harness.find("▣  10%").expect("the processor");
    harness.click(x, y);
    assert!(harness.app().windows().is_empty(), "nothing to open:\n{}", harness.screen());
}

#[test]
fn a_changing_number_never_moves_the_items_beside_it() {
    let scratch = Scratch::new("still");
    let mut harness = read_twice(&scratch, 120, 24);
    let memory = harness.find("◰").expect("the memory");
    // From a tenth busy to nine in a hundred: one digit fewer.
    scratch.stat(100 + 9, 900 + 91);
    pass(&mut harness, SAMPLE_EVERY);
    assert!(dock(&harness).contains("▣   9%"), "{}", dock(&harness));
    assert_eq!(harness.find("◰"), Some(memory), "the memory stayed where it was: {}", dock(&harness));
}

/// Opens Settings from its icon on the floor, clicks the switch of "Status on the dock" where it
/// is drawn, at the right edge of its row, and closes the window again from its title.
fn switch_strip_in_settings(harness: &mut Harness<Desk>) {
    let (x, y) = harness.find("Settings").expect("the Settings icon");
    harness.click(x, y);
    harness.click(x, y);
    let (_, row) = harness.find("Status on the dock").unwrap_or_else(|| panic!("the row:\n{}", harness.screen()));
    // A switch stands at the right edge of its row, where the arrow of the language drop-down
    // stands too.
    let (edge, _) = harness.find("▾").expect("the language drop-down");
    harness.click(edge - 1, row);
    let (x, y) = harness.find("×").unwrap_or_else(|| panic!("the window's close mark:\n{}", harness.screen()));
    harness.click(x, y);
    assert!(harness.app().windows().is_empty(), "the Settings window closed:\n{}", harness.screen());
}

#[test]
fn the_settings_switch_takes_the_strip_off_the_dock_and_stops_reading_the_machine() {
    let scratch = Scratch::new("switch");
    let mut harness = strip_desk(&scratch, Vec::new(), 140, 44);
    assert!(harness.app().prefs().status_strip, "on until someone turns it off");
    assert!(dock(&harness).contains("◰  41%"), "{}", dock(&harness));
    switch_strip_in_settings(&mut harness);
    assert!(!dock(&harness).contains("◰"), "the strip is gone: {}", dock(&harness));
    // A reading under way may still come back; after that, nothing reads the machine.
    pass(&mut harness, Duration::from_secs(120));
    let before = harness.app().machine().clone();
    scratch.memory(60);
    pass(&mut harness, Duration::from_secs(120));
    assert_eq!(harness.app().machine(), &before, "the machine is not read while nothing shows it");

    // Kept in the file, and read back by the next run.
    let written = fs::read_to_string(scratch.config().join("desktop.conf")).expect("the settings file is written");
    assert!(written.contains("status-on-dock = false"), "{written}");
    drop(harness);
    let mut again = strip_desk(&scratch, Vec::new(), 140, 44);
    assert!(!dock(&again).contains("◰"), "still off after a restart: {}", dock(&again));
    // On again: the strip comes back with a fresh reading, and the default leaves the file.
    switch_strip_in_settings(&mut again);
    assert!(dock(&again).contains("◰  60%"), "{}", dock(&again));
    let written = fs::read_to_string(scratch.config().join("desktop.conf")).expect("the settings file");
    assert!(!written.contains("status-on-dock"), "the default is not written:\n{written}");
}

#[test]
fn with_the_strip_off_the_system_widget_still_reads_the_machine() {
    let scratch = Scratch::new("widget");
    scratch.write("config/desktop.conf", "status-on-dock = false\n");
    let mut harness = strip_desk(&scratch, vec![Gadget::new(Kind::System, (9, 0), "")], 120, 40);
    assert!(!dock(&harness).contains("◰"), "{}", dock(&harness));
    scratch.memory(90);
    pass(&mut harness, SAMPLE_EVERY);
    assert!(harness.find("90%").is_some(), "the widget reads on:\n{}", harness.screen());
}

#[test]
fn the_strip_is_drawn_without_brackets_or_lines_in_ascii_and_sixteen_colours() {
    for (mode, depth) in [
        (GlyphMode::Ascii, ColorDepth::Ansi16),
        (GlyphMode::Unicode, ColorDepth::Ansi16),
        (GlyphMode::Ascii, ColorDepth::TrueColor),
    ] {
        let scratch = Scratch::new("modes");
        scratch.memory(95);
        let mut harness = read_twice(&scratch, 80, 24);
        harness.set_glyph_mode(mode).set_depth(depth);
        let row = dock(&harness);
        assert_eq!(decoration(&harness.screen()), None, "{mode:?} {depth:?}:\n{}", harness.screen());
        let alarm = harness.env().icons().glyph("error").into_owned();
        assert!(row.contains(&format!("{alarm}  95%")), "{mode:?}: the alarm has its mark: {row:?}");
        assert!(row.contains(MACHINE), "{row:?}");
    }
}

/// What a terminal would be sent to go from `before` to `after`: the same writer the running
/// desktop draws with, over only the cells that changed; nothing when no cell changed, since the
/// running desktop draws a frame only for a message that came.
fn sent(before: &ratatui_core::buffer::Buffer, after: &ratatui_core::buffer::Buffer) -> usize {
    use ratatui_core::backend::Backend;
    let changed = before.diff(after);
    if changed.is_empty() {
        return 0;
    }
    let mut bytes = Vec::new();
    ratatui_crossterm::CrosstermBackend::new(&mut bytes).draw(changed.into_iter()).expect("drawn into memory");
    bytes.len()
}

/// What every frame costs on top of the cells it changes: the synchronized update around it and
/// hiding the cursor.
const FRAMING: usize = 8 + 8 + 6;

/// What an idle desktop with the status strip sends a terminal in a minute, at 120 by 24, over a
/// `/proc` the test writes: on a busy machine the processor's share and the network rate change at
/// every reading, the worst case; on a quiet one they stay the same. Run it on its own:
///
/// ```text
/// cargo test --test strip -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, not a check: it prints what a minute costs"]
fn an_idle_status_strip_costs_this_many_bytes_a_minute() {
    for (name, busy, on) in [("strip off", false, false), ("quiet machine", false, true), ("busy machine", true, true)]
    {
        let scratch = Scratch::new("bytes");
        scratch.tmux();
        if !on {
            scratch.write("config/desktop.conf", "status-on-dock = false\n");
        }
        let mut harness = strip_desk(&scratch, Vec::new(), 120, 24);
        let (mut content, mut frames) = (0, 0);
        let (mut used, mut idle, mut received) = (0, 0, 0);
        for step in 0..60_u64 {
            used += if busy { 37 + step * 13 % 50 } else { 10 };
            idle += if busy { 100 } else { 90 };
            received += if busy { 40_000 + step * 7_919 % 30_000 } else { 0 };
            scratch.stat(used, idle);
            scratch.network(received);
            let before = harness.buffer().clone();
            harness.advance(Duration::from_secs(1));
            let bytes = sent(&before, harness.buffer());
            content += bytes;
            frames += usize::from(bytes > 0);
        }
        let total = content + frames * FRAMING;
        println!("{name}: about {total} bytes a minute at 120x24 ({frames} frames)");
    }
}
