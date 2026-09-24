//! Reading the machine: `/proc`, `/sys/class/power_supply` and tmux.
//!
//! Every reader gives `None` for what it cannot read, so a machine without a battery, a container
//! with a narrow `/proc` or a system without tmux shows fewer items and never an error.

use std::ffi::OsString;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::{Status, TMUX_EVERY_SAMPLES};
use crate::apps::{Environment, find_program, is_executable};

/// The longest tmux may take to list its sessions. A tmux stuck on a dead socket must not stop
/// the strip: past this it is stopped and the sessions are left as they were.
const TMUX_PATIENCE: Duration = Duration::from_secs(3);

/// How often the answer of a running tmux is looked for.
const TMUX_POLL: Duration = Duration::from_millis(10);

/// The memory in use, from `/proc/meminfo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Memory {
    /// Bytes in use: all of it but what is available to programs.
    pub used: u64,
    /// All of it, in bytes.
    pub total: u64,
}

impl Memory {
    /// The share in use, in percent.
    #[must_use]
    #[expect(clippy::cast_precision_loss, reason = "a share on screen needs no more than f64 keeps")]
    pub fn share(self) -> f64 {
        if self.total == 0 { 0.0 } else { self.used as f64 * 100.0 / self.total as f64 }
    }
}

/// The machine's battery, from `/sys/class/power_supply`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Battery {
    /// How full it is, in percent; the mean of every battery on a machine with several.
    pub percent: u8,
    /// Whether it is charging.
    pub charging: bool,
}

/// The network's rate in each direction, in bytes a second, on every interface but loopback.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Traffic {
    /// Bytes received a second.
    pub down: f64,
    /// Bytes sent a second.
    pub up: f64,
}

/// Bytes received and sent so far, the counters a [`Traffic`] is measured between.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bytes {
    received: u64,
    sent: u64,
}

/// The processor's counters from the first line of `/proc/stat`, in clock ticks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CpuTicks {
    idle: u64,
    total: u64,
}

/// What a reading keeps for the next one: the counters rates are measured between.
#[derive(Debug, Clone, Copy)]
struct Counters {
    cpu: Option<CpuTicks>,
    network: Option<Bytes>,
    at: Instant,
}

/// Reads the machine. It keeps the counters of its last reading, so the processor's share and the
/// network rate are measured between two readings, and the tmux sessions it last saw.
#[derive(Debug, Clone)]
pub struct Probe {
    proc_root: PathBuf,
    power_root: PathBuf,
    path: Option<OsString>,
    last: Option<Counters>,
    /// Readings since tmux was last asked; tmux is asked at the first reading.
    since_tmux: u32,
    tmux: Vec<String>,
}

impl Probe {
    /// A probe of this machine: `/proc`, `/sys/class/power_supply`, and tmux looked for on the
    /// environment's `PATH`.
    #[must_use]
    pub fn new(env: &Environment) -> Self {
        Self::rooted("/proc", "/sys/class/power_supply", env.path.clone())
    }

    /// A probe reading `proc_root` as `/proc`, `power_root` as `/sys/class/power_supply` and
    /// looking for tmux on `path`.
    #[must_use]
    pub fn rooted(proc_root: impl Into<PathBuf>, power_root: impl Into<PathBuf>, path: Option<OsString>) -> Self {
        Self {
            proc_root: proc_root.into(),
            power_root: power_root.into(),
            path,
            last: None,
            since_tmux: TMUX_EVERY_SAMPLES,
            tmux: Vec::new(),
        }
    }

    /// Reads the machine now.
    pub fn sample(&mut self) -> Status {
        self.sample_at(Instant::now())
    }

    /// Reads the machine as of `now`. The processor's share and the network rate need the
    /// reading before, so the first reading has neither. tmux is asked at the first reading and
    /// then every [`TMUX_EVERY`](super::TMUX_EVERY); in between its last answer stands.
    pub fn sample_at(&mut self, now: Instant) -> Status {
        let cpu = read_cpu(&self.proc_root);
        let network = read_network(&self.proc_root);
        let last = self.last.replace(Counters { cpu, network, at: now });
        let cpu_share = match (last.and_then(|last| last.cpu), cpu) {
            (Some(before), Some(after)) => cpu_share(before, after),
            _ => None,
        };
        let rate = match (last, network) {
            (Some(Counters { network: Some(before), at, .. }), Some(after)) => {
                let elapsed = now.saturating_duration_since(at);
                match (rate(before.received, after.received, elapsed), rate(before.sent, after.sent, elapsed)) {
                    (Some(down), Some(up)) => Some(Traffic { down, up }),
                    _ => None,
                }
            }
            _ => None,
        };
        if self.since_tmux >= TMUX_EVERY_SAMPLES {
            self.since_tmux = 0;
            if let Some(sessions) = self.tmux_sessions() {
                self.tmux = sessions;
            }
        }
        self.since_tmux += 1;
        Status {
            cpu: cpu_share,
            memory: read_memory(&self.proc_root),
            battery: read_battery(&self.power_root),
            network: rate,
            tmux: self.tmux.clone(),
        }
    }

    /// The names of the tmux sessions: empty when tmux is not on the path, has no server or
    /// fails; `None` only when tmux took too long to answer, so the last answer stands.
    fn tmux_sessions(&self) -> Option<Vec<String>> {
        let Some(tmux) = find_program("tmux", self.path.as_deref(), is_executable) else {
            return Some(Vec::new());
        };
        let Ok(mut child) = Command::new(tmux)
            .args(["list-sessions", "-F", "#{session_name}"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
        else {
            return Some(Vec::new());
        };
        // The list is a few names; it fits the pipe, so waiting before reading cannot block.
        let started = Instant::now();
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) if started.elapsed() < TMUX_PATIENCE => std::thread::sleep(TMUX_POLL),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        };
        if !status.success() {
            return Some(Vec::new());
        }
        let mut out = String::new();
        let read = child.stdout.take().map(|mut stdout| stdout.read_to_string(&mut out));
        if !matches!(read, Some(Ok(_))) {
            return Some(Vec::new());
        }
        Some(out.lines().map(str::trim).filter(|name| !name.is_empty()).map(str::to_owned).collect())
    }
}

/// The processor's counters from `/proc/stat`.
fn read_cpu(proc_root: &Path) -> Option<CpuTicks> {
    parse_cpu(&std::fs::read_to_string(proc_root.join("stat")).ok()?)
}

/// The first line of `/proc/stat`: `cpu user nice system idle iowait irq softirq steal guest
/// guest_nice`. Time waiting for the disk counts as idle; guest time is already in user time.
fn parse_cpu(text: &str) -> Option<CpuTicks> {
    let line = text.lines().find(|line| line.split_whitespace().next() == Some("cpu"))?;
    let fields: Vec<u64> = line.split_whitespace().skip(1).take(8).map(str::parse).collect::<Result<_, _>>().ok()?;
    if fields.len() < 4 {
        return None;
    }
    let idle = fields[3] + fields.get(4).copied().unwrap_or(0);
    Some(CpuTicks { idle, total: fields.iter().sum() })
}

/// The busy share between two readings, in percent; `None` when no time passed or the counters
/// went back.
#[expect(clippy::cast_precision_loss, reason = "a share on screen needs no more than f64 keeps")]
fn cpu_share(before: CpuTicks, after: CpuTicks) -> Option<f64> {
    let total = after.total.checked_sub(before.total)?;
    let idle = after.idle.checked_sub(before.idle)?;
    if total == 0 {
        return None;
    }
    Some(total.saturating_sub(idle) as f64 * 100.0 / total as f64)
}

/// The memory in use from `/proc/meminfo`.
fn read_memory(proc_root: &Path) -> Option<Memory> {
    parse_memory(&std::fs::read_to_string(proc_root.join("meminfo")).ok()?)
}

/// `MemTotal` less `MemAvailable`. A kernel older than `MemAvailable` gives free memory, buffers
/// and the page cache instead, which is what it approximates.
fn parse_memory(text: &str) -> Option<Memory> {
    let field = |name: &str| {
        text.lines().find_map(|line| {
            let rest = line.strip_prefix(name)?.strip_prefix(':')?;
            let kilobytes: u64 = rest.split_whitespace().next()?.parse().ok()?;
            Some(kilobytes * 1024)
        })
    };
    let total = field("MemTotal")?;
    if total == 0 {
        return None;
    }
    let available = field("MemAvailable")
        .or_else(|| Some(field("MemFree")? + field("Buffers").unwrap_or(0) + field("Cached").unwrap_or(0)))?;
    Some(Memory { used: total.saturating_sub(available), total })
}

/// The machine's battery from `/sys/class/power_supply`: every supply whose `type` is `Battery`
/// and that has a `capacity`, but not a mouse's or a keyboard's (`scope` is `Device`).
fn read_battery(power_root: &Path) -> Option<Battery> {
    let mut supplies: Vec<PathBuf> =
        std::fs::read_dir(power_root).ok()?.filter_map(Result::ok).map(|entry| entry.path()).collect();
    supplies.sort();
    let read =
        |supply: &Path, name: &str| std::fs::read_to_string(supply.join(name)).ok().map(|text| text.trim().to_owned());
    let mut levels = Vec::new();
    let mut charging = false;
    for supply in &supplies {
        if read(supply, "type").as_deref() != Some("Battery") || read(supply, "scope").as_deref() == Some("Device") {
            continue;
        }
        let Some(level) = read(supply, "capacity").and_then(|text| text.parse::<u8>().ok()) else { continue };
        levels.push(u32::from(level.min(100)));
        charging |= read(supply, "status").as_deref() == Some("Charging");
    }
    let count = u32::try_from(levels.len()).ok().filter(|count| *count > 0)?;
    let mean = levels.iter().sum::<u32>() / count;
    Some(Battery { percent: u8::try_from(mean).unwrap_or(100), charging })
}

/// Bytes received and sent on every interface but loopback, from `/proc/net/dev`.
fn read_network(proc_root: &Path) -> Option<Bytes> {
    parse_network(&std::fs::read_to_string(proc_root.join("net").join("dev")).ok()?)
}

/// `/proc/net/dev`: two heading lines, then `name: rx_bytes rx_packets ... tx_bytes ...`, the
/// sent bytes being the ninth number.
fn parse_network(text: &str) -> Option<Bytes> {
    let mut sum = Bytes { received: 0, sent: 0 };
    let mut any = false;
    for line in text.lines().skip(2) {
        let Some((name, numbers)) = line.split_once(':') else { continue };
        if name.trim() == "lo" {
            continue;
        }
        let numbers: Vec<&str> = numbers.split_whitespace().collect();
        let (Some(received), Some(sent)) = (numbers.first(), numbers.get(8)) else { continue };
        let (Ok(received), Ok(sent)) = (received.parse::<u64>(), sent.parse::<u64>()) else { continue };
        sum.received = sum.received.saturating_add(received);
        sum.sent = sum.sent.saturating_add(sent);
        any = true;
    }
    any.then_some(sum)
}

/// Bytes a second between two byte counts `elapsed` apart. Counters that went back (an interface
/// gone, a counter wrapped) read as no traffic rather than a leap.
#[expect(clippy::cast_precision_loss, reason = "a rate on screen needs no more than f64 keeps")]
fn rate(before: u64, after: u64, elapsed: Duration) -> Option<f64> {
    let seconds = elapsed.as_secs_f64();
    if seconds <= 0.0 {
        return None;
    }
    Some(after.saturating_sub(before) as f64 / seconds)
}
