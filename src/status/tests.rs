//! The status strip read from folders shaped like `/proc` and `/sys/class/power_supply`, and a
//! tmux written by the test: nothing here reads the machine it runs on or runs its programs.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use qframe::i18n::{I18n, scope};
use qframe::icons::{GlyphMode, IconSetRegistry};

use super::*;

/// A folder of its own under the system's temporary folder, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-status-{}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch folder");
        Self(path)
    }

    fn write(&self, relative: &str, text: &str) -> PathBuf {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
        std::fs::write(&path, text).expect("a file");
        path
    }

    /// A shell script at `bin/tmux` that runs `body`.
    fn tmux(&self, body: &str) {
        let path = self.write("bin/tmux", &format!("#!/bin/sh\n{body}\n"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("executable");
        }
    }

    fn proc(&self) -> PathBuf {
        self.0.join("proc")
    }

    fn power(&self) -> PathBuf {
        self.0.join("power_supply")
    }

    fn bin(&self) -> Option<OsString> {
        Some(self.0.join("bin").into_os_string())
    }

    fn probe(&self) -> Probe {
        Probe::rooted(self.proc(), self.power(), self.bin())
    }

    /// `/proc/stat` with the given total counters; the per-processor lines follow, as on a
    /// real machine, and must not be read in their place.
    fn stat(&self, user: u64, system: u64, idle: u64, iowait: u64) {
        self.write(
            "proc/stat",
            &format!(
                "cpu  {user} 20 {system} {idle} {iowait} 0 5 0 0 0\n\
                 cpu0 1 1 1 999999 0 0 0 0 0 0\n\
                 intr 1234 0 0\nctxt 99\nbtime 1758000000\nprocesses 4242\n"
            ),
        );
    }

    fn meminfo(&self, total_kb: u64, available_kb: u64) {
        self.write(
            "proc/meminfo",
            &format!(
                "MemTotal:       {total_kb} kB\nMemFree:          123456 kB\n\
                 MemAvailable:   {available_kb} kB\nBuffers:           65536 kB\nCached:          1048576 kB\n\
                 SwapTotal:       4194300 kB\n"
            ),
        );
    }

    /// `/proc/net/dev` with loopback, a wired and a wireless interface.
    fn net(&self, lo: u64, eth_rx: u64, eth_tx: u64, wlan_rx: u64, wlan_tx: u64) {
        self.write(
            "proc/net/dev",
            &format!(
                "Inter-|   Receive                                                |  Transmit\n \
                 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    \
                 lo: {lo} 100 0 0 0 0 0 0 {lo} 100 0 0 0 0 0 0\n  \
                 eth0: {eth_rx} 2000 0 0 0 0 0 12 {eth_tx} 1500 0 0 0 0 0 0\n\
                 wlan0: {wlan_rx} 10 0 0 0 0 0 0 {wlan_tx} 10 0 0 0 0 0 0\n"
            ),
        );
    }

    fn supply(&self, name: &str, fields: &[(&str, &str)]) {
        for (field, value) in fields {
            self.write(&format!("power_supply/{name}/{field}"), &format!("{value}\n"));
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn translator(code: &str) -> Arc<I18n> {
    let mut i18n = I18n::builtin();
    for (file, text) in crate::locales() {
        assert!(i18n.add_source(file, text), "{file} loads");
    }
    assert!(i18n.set_active(code));
    Arc::new(i18n)
}

fn english<T>(work: impl FnOnce() -> T) -> T {
    scope(translator("en"), work)
}

fn texts(status: &Status) -> Vec<(Kind, String)> {
    english(|| items(status).into_iter().map(|item| (item.kind, item.text)).collect())
}

fn text_of(status: &Status, kind: Kind) -> Option<String> {
    texts(status).into_iter().find(|(of, _)| *of == kind).map(|(_, text)| text)
}

/// A machine as a laptop has it: every file there, no tmux.
fn laptop(scratch: &Scratch) {
    scratch.stat(1000, 500, 8000, 500);
    scratch.meminfo(8_000_000, 4_720_000);
    scratch.net(999_999, 10_000, 5_000, 1_000, 500);
    scratch.supply("AC", &[("type", "Mains"), ("online", "1")]);
    scratch.supply("BAT0", &[("type", "Battery"), ("capacity", "87"), ("status", "Charging")]);
}

#[test]
fn the_processor_share_is_measured_between_two_readings() {
    let scratch = Scratch::new();
    laptop(&scratch);
    let mut probe = scratch.probe();
    let start = Instant::now();
    assert_eq!(probe.sample_at(start).cpu, None, "one reading says nothing about how busy it was");
    // 300 busy ticks and 700 idle ones (600 idle, 100 waiting for the disk) since.
    scratch.stat(1200, 600, 8600, 600);
    let status = probe.sample_at(start + SAMPLE_EVERY);
    let cpu = status.cpu.expect("a share");
    assert!((cpu - 30.0).abs() < 1e-9, "{cpu}");
    assert_eq!(text_of(&status, Kind::Cpu).as_deref(), Some("30%"));
}

#[test]
fn memory_in_use_is_all_of_it_but_what_is_available() {
    let scratch = Scratch::new();
    laptop(&scratch);
    let status = scratch.probe().sample_at(Instant::now());
    assert_eq!(status.memory, Some(Memory { used: 3_280_000 * 1024, total: 8_000_000 * 1024 }));
    assert_eq!(text_of(&status, Kind::Memory).as_deref(), Some("41%"));
}

#[test]
fn an_old_kernel_without_mem_available_counts_free_buffers_and_cache() {
    let scratch = Scratch::new();
    scratch.write("proc/meminfo", "MemTotal: 1000 kB\nMemFree: 100 kB\nBuffers: 50 kB\nCached: 250 kB\n");
    let memory = scratch.probe().sample_at(Instant::now()).memory.expect("memory");
    assert_eq!(memory, Memory { used: 600 * 1024, total: 1000 * 1024 });
}

#[test]
fn the_network_rate_sums_every_interface_but_loopback() {
    let scratch = Scratch::new();
    laptop(&scratch);
    let mut probe = scratch.probe();
    let start = Instant::now();
    assert_eq!(probe.sample_at(start).network, None, "one reading has no rate");
    // Loopback moves a lot and must not count; the others move 2.4 MiB in two seconds.
    scratch.net(999_999_999, 10_000 + 2_000_000, 5_000 + 400_000, 1_000 + 100_000, 500 + 16_583);
    let status = probe.sample_at(start + Duration::from_secs(2));
    let rate = status.network.expect("a rate");
    assert!((rate - 1_258_291.5).abs() < 1e-6, "{rate}");
    assert_eq!(text_of(&status, Kind::Network).as_deref(), Some("1.2M/s"));
}

#[test]
fn a_counter_that_goes_back_reads_as_no_traffic() {
    let scratch = Scratch::new();
    laptop(&scratch);
    let mut probe = scratch.probe();
    let start = Instant::now();
    probe.sample_at(start);
    scratch.net(0, 0, 0, 0, 0);
    assert_eq!(probe.sample_at(start + SAMPLE_EVERY).network, Some(0.0));
}

#[test]
fn the_battery_shows_its_level_and_whether_it_charges() {
    let scratch = Scratch::new();
    laptop(&scratch);
    let status = scratch.probe().sample_at(Instant::now());
    assert_eq!(status.battery, Some(Battery { percent: 87, charging: true }));
    assert_eq!(text_of(&status, Kind::Battery).as_deref(), Some("87%+"));

    scratch.supply("BAT0", &[("capacity", "9"), ("status", "Discharging")]);
    let status = scratch.probe().sample_at(Instant::now());
    assert_eq!(text_of(&status, Kind::Battery).as_deref(), Some("9%"));
    let tone = english(|| items(&status).into_iter().find(|item| item.kind == Kind::Battery).map(|item| item.tone));
    assert_eq!(tone, Some(Tone::Alert));
}

#[test]
fn a_mouse_battery_is_not_the_machine_s() {
    let scratch = Scratch::new();
    laptop(&scratch);
    scratch.supply(
        "hidpp_battery_0",
        &[("type", "Battery"), ("scope", "Device"), ("capacity", "5"), ("status", "Discharging")],
    );
    let status = scratch.probe().sample_at(Instant::now());
    assert_eq!(status.battery, Some(Battery { percent: 87, charging: true }));
}

#[test]
fn a_machine_with_no_battery_has_no_battery_item() {
    let scratch = Scratch::new();
    laptop(&scratch);
    std::fs::remove_dir_all(scratch.power().join("BAT0")).expect("the battery removed");
    let status = scratch.probe().sample_at(Instant::now());
    assert_eq!(status.battery, None);
    assert_eq!(text_of(&status, Kind::Battery), None);

    // Nor does a machine with no power_supply folder at all, as most servers are.
    std::fs::remove_dir_all(scratch.power()).expect("the folder removed");
    assert_eq!(scratch.probe().sample_at(Instant::now()).battery, None);
}

#[test]
fn tmux_sessions_come_from_tmux_on_the_path_asked_for_their_names() {
    let scratch = Scratch::new();
    laptop(&scratch);
    let asked = scratch.0.join("asked");
    scratch.tmux(&format!("printf '%s\\n' \"$@\" > '{}'\nprintf 'main\\nwork\\n'", asked.display()));
    let status = scratch.probe().sample_at(Instant::now());
    assert_eq!(status.tmux, ["main", "work"]);
    assert_eq!(std::fs::read_to_string(&asked).expect("tmux was asked"), "list-sessions\n-F\n#{session_name}\n");
    let tmux = english(|| items(&status).into_iter().find(|item| item.kind == Kind::Tmux)).expect("a tmux item");
    assert_eq!(tmux.text, "2");
    assert_eq!(tmux.menu, ["main", "work"]);
}

#[test]
fn a_machine_with_no_tmux_has_no_tmux_item() {
    let scratch = Scratch::new();
    laptop(&scratch);
    let status = scratch.probe().sample_at(Instant::now());
    assert!(status.tmux.is_empty());
    assert_eq!(text_of(&status, Kind::Tmux), None);
    // And no PATH at all is the same.
    let status = Probe::rooted(scratch.proc(), scratch.power(), None).sample_at(Instant::now());
    assert!(status.tmux.is_empty());
}

#[test]
fn a_tmux_with_no_server_means_no_sessions() {
    let scratch = Scratch::new();
    laptop(&scratch);
    // What tmux does with no server: a line on stderr and exit status 1.
    scratch.tmux("echo 'stale' \necho 'no server running on /tmp/tmux-1000/default' >&2\nexit 1");
    let status = scratch.probe().sample_at(Instant::now());
    assert!(status.tmux.is_empty(), "{:?}", status.tmux);
    assert_eq!(text_of(&status, Kind::Tmux), None);
}

#[test]
fn tmux_is_asked_at_the_first_reading_and_then_every_ten_seconds() {
    let scratch = Scratch::new();
    laptop(&scratch);
    let log = scratch.0.join("log");
    scratch.tmux(&format!("echo asked >> '{}'\necho main", log.display()));
    let mut probe = scratch.probe();
    let start = Instant::now();
    let readings = TMUX_EVERY_SAMPLES * 2 + 1;
    for reading in 0..readings {
        let status = probe.sample_at(start + SAMPLE_EVERY * reading);
        assert_eq!(status.tmux, ["main"], "the last answer stands between questions");
    }
    let asked = std::fs::read_to_string(&log).expect("tmux was asked").lines().count();
    assert_eq!(asked, 3, "{readings} readings two seconds apart ask tmux at 0, 10 and 20 seconds");
    assert_eq!(TMUX_EVERY_SAMPLES, 5);
}

#[test]
fn a_bare_machine_shows_nothing_and_never_fails() {
    let scratch = Scratch::new();
    let mut probe = scratch.probe();
    let start = Instant::now();
    probe.sample_at(start);
    let status = probe.sample_at(start + SAMPLE_EVERY);
    assert_eq!(status, Status::default());
    assert!(texts(&status).is_empty());
}

#[test]
fn texts_are_short_and_rates_step_through_units() {
    let rate = |bytes: f64| english(|| rate_text(bytes));
    assert_eq!(rate(0.0), "0B/s");
    assert_eq!(rate(999.0), "999B/s");
    assert_eq!(rate(1000.0), "1.0K/s");
    assert_eq!(rate(340.0 * 1024.0), "340K/s");
    assert_eq!(rate(9.96 * 1024.0 * 1024.0), "10M/s");
    assert_eq!(rate(3.0 * 1024.0 * 1024.0 * 1024.0), "3.0G/s");
    assert_eq!(rate(5000.0 * 1024.0 * 1024.0 * 1024.0), "5000G/s");
}

#[test]
fn numbers_follow_the_language_on_screen() {
    let status = Status {
        cpu: Some(12.4),
        network: Some(1.2 * 1024.0 * 1024.0),
        battery: Some(Battery { percent: 50, charging: false }),
        ..Status::default()
    };
    let turkish = scope(translator("tr"), || items(&status).into_iter().map(|item| item.text).collect::<Vec<_>>());
    assert_eq!(turkish, ["1,2M/sn", "%12", "%50"]);
    let french = scope(translator("fr"), || items(&status)[0].text.clone());
    assert!(french.starts_with("1,2"), "{french}");
}

#[test]
fn load_warns_and_alarms_as_it_fills() {
    let tone = |cpu: f64| english(|| items(&Status { cpu: Some(cpu), ..Status::default() })[0].tone);
    assert_eq!(tone(74.0), Tone::Calm);
    assert_eq!(tone(75.0), Tone::Warn);
    assert_eq!(tone(90.0), Tone::Alert);
    let battery = |percent, charging| {
        english(|| items(&Status { battery: Some(Battery { percent, charging }), ..Status::default() })[0].tone)
    };
    assert_eq!(battery(21, false), Tone::Calm);
    assert_eq!(battery(20, false), Tone::Warn);
    assert_eq!(battery(10, false), Tone::Alert);
    assert_eq!(battery(5, true), Tone::Calm, "a battery that is charging is on its way back");
}

#[test]
fn only_a_shown_change_asks_for_a_redraw() {
    let status = Status { cpu: Some(12.2), memory: Some(Memory { used: 41, total: 100 }), ..Status::default() };
    let before = english(|| items(&status));
    // A share that rounds to the same text is not sent again over SSH.
    let same = english(|| items(&Status { cpu: Some(11.9), ..status.clone() }));
    assert!(!changed(&before, &same));
    let other = english(|| items(&Status { cpu: Some(13.0), ..status.clone() }));
    assert!(changed(&before, &other));
    let gone = english(|| items(&Status { memory: None, ..status.clone() }));
    assert!(changed(&before, &gone));
    let sessions = english(|| items(&Status { tmux: vec!["a".into()], ..status.clone() }));
    let renamed = english(|| items(&Status { tmux: vec!["b".into()], ..status }));
    assert!(changed(&sessions, &renamed), "a renamed session changes the menu, not the count");
}

#[test]
fn attaching_asks_tmux_for_exactly_that_session() {
    assert_eq!(attach_command("dev"), ["tmux", "attach", "-t", "=dev"]);
}

#[test]
fn every_glyph_is_in_the_framework_icon_set() {
    let icons = IconSetRegistry::builtin().icons("default", &BTreeMap::new(), GlyphMode::Ascii);
    for kind in [Kind::Tmux, Kind::Network, Kind::Cpu, Kind::Memory, Kind::Battery] {
        assert!(icons.contains(kind.glyph_key()), "{kind:?}: `{}` is not an icon", kind.glyph_key());
        assert!(english(|| !kind.label().starts_with("status.")), "{kind:?} has a name");
    }
}

#[test]
fn two_readings_look_the_same_when_every_text_and_tone_of_the_strip_would() {
    let reading = |cpu: f64, rate: f64| Status {
        cpu: Some(cpu),
        memory: Some(Memory { used: 40, total: 100 }),
        battery: None,
        network: Some(rate),
        tmux: vec!["main".to_owned()],
    };
    let same = |a: &Status, b: &Status| {
        let by_items = english(|| !changed(&items(a), &items(b)));
        assert_eq!(same_on_screen(a, b), by_items, "{a:?} against {b:?}");
        by_items
    };
    assert!(same(&reading(50.0, 1000.0), &reading(50.2, 1010.0)), "50% and 1.0K/s both times");
    assert!(!same(&reading(50.0, 1000.0), &reading(51.0, 1000.0)));
    assert!(!same(&reading(74.6, 0.0), &reading(75.2, 0.0)), "both 75%, but only the second warns");
    assert!(!same(&reading(10.0, 1000.0), &reading(10.0, 12_000.0)), "1.0K/s against 12K/s");
    let mut other = reading(10.0, 0.0);
    other.tmux.push("logs".to_owned());
    assert!(!same(&reading(10.0, 0.0), &other), "a new tmux session is drawn");
}

#[test]
fn the_widest_texts_hold_every_number_their_items_can_show() {
    english(|| {
        assert_eq!(widest(Kind::Cpu).as_deref(), Some("100%"));
        assert_eq!(widest(Kind::Tmux), None, "the count changes at a person's pace");
        let network = widest(Kind::Network).expect("a widest rate");
        let room = qframe::text::width(&network);
        for bytes in [0.0, 9.0, 999.0, 1_000.0, 10_189.0, 10_240.0, 1_048_000.0, 5.0e9] {
            let shown = rate_text(bytes);
            assert!(qframe::text::width(&shown) <= room, "{shown} is wider than {network}");
        }
        for share in [0.0, 9.4, 55.5, 100.0] {
            assert!(qframe::text::width(&percent(share)) <= 4, "{share}");
        }
    });
}
