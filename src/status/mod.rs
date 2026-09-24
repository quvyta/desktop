//! The status strip's data: what the machine is doing, as short texts for the dock's right side.
//!
//! A system tray means little over SSH, where the graphical programs whose icons it holds do not
//! run. The strip shows the machine itself instead: its tmux sessions, the network rate, how busy
//! the processor is, how full the memory is and, where there is one, the battery. Everything is
//! read without root from `/proc` and `/sys`; tmux is asked through the program itself.
//!
//! Nothing here draws. A [`Probe`] reads the machine into a [`Status`]; [`items`] turns that into
//! short texts in the language on screen; [`changed`] says whether they differ from what is
//! already drawn, so the dock redraws only then. [`sample_after`] runs the probe off the render
//! path, as a task the screen never waits for.

mod probe;
#[cfg(test)]
mod tests;

use std::time::Duration;

use qframe::i18n;
use qframe::prelude::*;
use qframe::runtime::Task;

pub use probe::{Battery, Memory, Probe};

/// How often the machine is read.
///
/// Over SSH every redraw costs bytes: an item that changes is a cursor move, a colour and a few
/// letters, 20 to 40 bytes. The processor's share changes at almost every reading, so this wait
/// is what the strip costs the connection, about 20 bytes a second at 2 seconds. Reading more
/// often makes the numbers jitter without telling the person anything more.
pub const SAMPLE_EVERY: Duration = Duration::from_secs(2);

/// How often tmux is asked for its sessions. Unlike a file under `/proc`, asking starts a
/// program, and sessions come and go at the pace of a person, not of the processor.
pub const TMUX_EVERY: Duration = Duration::from_secs(10);

/// How many readings pass between two questions to tmux.
pub(crate) const TMUX_EVERY_SAMPLES: u32 = {
    let every = TMUX_EVERY.as_millis() / SAMPLE_EVERY.as_millis();
    assert!(every >= 1 && every <= u32::MAX as u128);
    every as u32
};

/// The processor's or the memory's share, in percent, from which the item warns.
const LOAD_WARN: f64 = 75.0;
/// The share from which the item alarms.
const LOAD_ALERT: f64 = 90.0;
/// The battery level, in percent, below which a battery that is not charging warns.
const BATTERY_WARN: u8 = 20;
/// The level below which it alarms.
const BATTERY_ALERT: u8 = 10;

/// Written after the level of a battery that is charging. ASCII, so every terminal draws it.
const CHARGING_MARK: &str = "+";

/// What the machine is doing, as one reading saw it. A part the machine does not have, or that
/// needs two readings to be known, is `None`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Status {
    /// How busy the processors were since the reading before, in percent.
    pub cpu: Option<f64>,
    /// The memory in use.
    pub memory: Option<Memory>,
    /// The machine's battery, on a machine that has one.
    pub battery: Option<Battery>,
    /// Bytes received and sent per second on every interface but loopback, since the reading
    /// before.
    pub network: Option<f64>,
    /// The names of the tmux sessions, in tmux's order. Empty when tmux is not installed, has no
    /// server or fails.
    pub tmux: Vec<String>,
}

/// Which part of the machine an item shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    /// The tmux sessions.
    Tmux,
    /// The network rate.
    Network,
    /// The processor.
    Cpu,
    /// The memory.
    Memory,
    /// The battery.
    Battery,
}

impl Kind {
    /// The part's name in the language on screen, for a tooltip or a menu heading.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::Tmux => t!("status.tmux"),
            Self::Network => t!("status.network"),
            Self::Cpu => t!("status.cpu"),
            Self::Memory => t!("status.memory"),
            Self::Battery => t!("status.battery"),
        }
    }

    /// The key of the part's glyph in the framework's icon set.
    ///
    /// The set has no processor, memory, battery or terminal icon of its own; these are the
    /// nearest it has until it does.
    #[must_use]
    pub fn glyph_key(self) -> &'static str {
        match self {
            Self::Tmux => "prompt",
            Self::Network => "category-network",
            Self::Cpu => "category-system",
            Self::Memory => "workspace",
            Self::Battery => "power",
        }
    }
}

/// How urgently an item asks to be looked at; the dock picks the theme's colour for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tone {
    /// Nothing to see.
    #[default]
    Calm,
    /// Getting high, or a battery getting low.
    Warn,
    /// Nearly full, or a battery nearly empty.
    Alert,
}

/// One item of the strip: a glyph and a short text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The part it shows.
    pub kind: Kind,
    /// The key of its glyph in the framework's icon set, which also gives the ASCII glyph.
    pub glyph_key: &'static str,
    /// The text after the glyph: `12%`, `1.2M/s`, `2`.
    pub text: String,
    /// How urgently it asks to be looked at.
    pub tone: Tone,
    /// What a click lists: the tmux sessions' names. Empty for every other part.
    pub menu: Vec<String>,
}

impl Item {
    fn new(kind: Kind, text: String, tone: Tone) -> Self {
        Self { kind, glyph_key: kind.glyph_key(), text, tone, menu: Vec::new() }
    }
}

/// The strip's items for `status`, left to right, in the language on screen. A part the machine
/// does not have, or that is not known yet, has no item.
#[must_use]
pub fn items(status: &Status) -> Vec<Item> {
    let mut items = Vec::new();
    if !status.tmux.is_empty() {
        let mut item = Item::new(Kind::Tmux, status.tmux.len().to_string(), Tone::Calm);
        item.menu.clone_from(&status.tmux);
        items.push(item);
    }
    if let Some(rate) = status.network {
        items.push(Item::new(Kind::Network, rate_text(rate), Tone::Calm));
    }
    if let Some(cpu) = status.cpu {
        items.push(Item::new(Kind::Cpu, percent(cpu), load_tone(cpu)));
    }
    if let Some(memory) = status.memory {
        let share = memory.share();
        items.push(Item::new(Kind::Memory, percent(share), load_tone(share)));
    }
    if let Some(battery) = status.battery {
        let mut text = percent(f64::from(battery.percent));
        if battery.charging {
            text.push_str(CHARGING_MARK);
        }
        items.push(Item::new(Kind::Battery, text, battery_tone(battery)));
    }
    items
}

/// The widest text the item of `kind` can show, in the language on screen: `100%` for a share,
/// the widest rate for the network. The dock keeps each item that wide, its number to the right,
/// so a number that changes never moves the items beside it: the row holds still, and over SSH
/// only the changed number is sent. `None` for the tmux count, which changes at a person's pace.
///
/// A battery that starts charging, or an item that turns to a warning, still grows by its mark:
/// that happens seldom, and the mark is the point.
#[must_use]
pub fn widest(kind: Kind) -> Option<String> {
    match kind {
        Kind::Tmux => None,
        Kind::Cpu | Kind::Memory | Kind::Battery => Some(percent(100.0)),
        Kind::Network => {
            // Below ten a rate has a decimal; from ten to 999 it is whole.
            [999.0, 9.9 * 1024.0, 999.0 * 1024.0]
                .into_iter()
                .map(rate_text)
                .max_by_key(|text| qframe::text::width(text))
        }
    }
}

/// Whether the strip drawn from `previous` must be drawn again for `next`: some text, tone or
/// menu differs. Over SSH an unchanged strip is not sent at all.
#[must_use]
pub fn changed(previous: &[Item], next: &[Item]) -> bool {
    previous != next
}

/// The command words that attach a terminal to the tmux session `name`.
///
/// `=` asks tmux for exactly that name: without it tmux takes a name as a prefix or a pattern,
/// and `dev` could attach to `devops`.
#[must_use]
pub fn attach_command(name: &str) -> Vec<String> {
    vec!["tmux".to_owned(), "attach".to_owned(), "-t".to_owned(), format!("={name}")]
}

/// Waits `wait`, reads the machine with `probe` on a background task, and delivers the probe
/// back with its reading through `done`, so the caller keeps it for the next reading. The screen
/// never waits for it. A cancelled wait, when the program ends, delivers nothing.
pub fn sample_after<Msg: Send + 'static>(
    mut probe: Probe,
    wait: Duration,
    done: impl FnOnce(Probe, Status) -> Msg + Send + 'static,
) -> Command<Msg> {
    Command::task(Task::new("status", move |cx| {
        if !cx.sleep(wait) {
            return Err("stopped".to_owned());
        }
        let status = probe.sample();
        Ok(done(probe, status))
    }))
}

/// How many readings in a row that look the same the task behind [`sample_when_changed`] keeps to
/// itself before it delivers one anyway: a minute. The caller hears from it at least that often,
/// so a caller that no longer wants readings stops them within a minute, and the task never runs
/// unheard for longer.
pub const SAME_AT_MOST: u32 = 30;

/// Reads the machine with `probe` after `wait`, and again every [`SAMPLE_EVERY`], on a background
/// task, until a reading would be drawn differently from `shown` or [`SAME_AT_MOST`] readings in a
/// row have looked the same; then delivers the probe back with that reading through `done`.
///
/// A reading that changes nothing on screen is mostly not delivered, so the desktop draws no frame
/// for it: over SSH even a frame that changes no cell costs its framing, a few dozen bytes, and on
/// a quiet machine the readings come out the same most of the time. Whether two readings look the
/// same is [`same_on_screen`], which needs no language and so can be asked on the task. A
/// cancelled wait, when the program ends, delivers nothing.
pub fn sample_when_changed<Msg: Send + 'static>(
    mut probe: Probe,
    shown: Status,
    wait: Duration,
    done: impl FnOnce(Probe, Status) -> Msg + Send + 'static,
) -> Command<Msg> {
    Command::task(Task::new("status", move |cx| {
        let mut wait = wait;
        for _ in 0..SAME_AT_MOST {
            if !cx.sleep(wait) {
                return Err("stopped".to_owned());
            }
            let status = probe.sample();
            if !same_on_screen(&shown, &status) {
                return Ok(done(probe, status));
            }
            wait = SAMPLE_EVERY;
        }
        // A minute of the same readings: the caller hears the last one, and decides whether to go
        // on. On a quiet machine that is one frame a minute.
        if !cx.sleep(wait) {
            return Err("stopped".to_owned());
        }
        let status = probe.sample();
        Ok(done(probe, status))
    }))
}

/// Whether `a` and `b` give the same [`items`] in every language: the same rounded numbers in the
/// same units, the same tones and the same tmux sessions. Unlike comparing the items themselves it
/// needs no translator, so a background task can ask it.
#[must_use]
pub fn same_on_screen(a: &Status, b: &Status) -> bool {
    glance(a) == glance(b)
}

/// What decides how a reading is drawn, without the words: each number as it is rounded for the
/// strip, with its tone.
type Glance = (Option<(i64, Tone)>, Option<(i64, Tone)>, Option<(u8, bool, Tone)>, Option<(usize, i64)>, Vec<String>);

fn glance(status: &Status) -> Glance {
    #[expect(clippy::cast_possible_truncation, reason = "a share or a shown rate is far inside i64")]
    let whole = |value: f64| value.round() as i64;
    let rate = status.network.map(|bytes| {
        let (unit, value, decimals) = rate_parts(bytes);
        #[expect(clippy::cast_possible_truncation, reason = "a shown rate is far inside i64")]
        let shown = (value * 10_f64.powi(i32::try_from(decimals).unwrap_or(0))).round() as i64;
        (unit, shown)
    });
    (
        status.cpu.map(|cpu| (whole(cpu), load_tone(cpu))),
        status.memory.map(|memory| (whole(memory.share()), load_tone(memory.share()))),
        status.battery.map(|battery| (battery.percent, battery.charging, battery_tone(battery))),
        rate,
        status.tmux.clone(),
    )
}

/// `value` percent, rounded, the language's way (`12%`, `%12`).
fn percent(value: f64) -> String {
    t!("status.percent", value = i18n::number(value.round(), 0).as_str())
}

/// A rate in bytes a second, in binary units, one decimal below ten: `0B/s`, `340K/s`,
/// `1.2M/s`.
fn rate_text(bytes_per_second: f64) -> String {
    let keys = ["status.rate-bytes", "status.rate-kilo", "status.rate-mega", "status.rate-giga"];
    let (unit, value, decimals) = rate_parts(bytes_per_second);
    t!(keys[unit], value = i18n::number(value, decimals).as_str())
}

/// The unit a rate is written in (bytes, kilo, mega, giga), its value in that unit and how many
/// decimals it is written with.
fn rate_parts(bytes_per_second: f64) -> (usize, f64, usize) {
    const STEP: f64 = 1024.0;
    const UNITS: usize = 4;
    let mut value = bytes_per_second.max(0.0);
    let mut unit = 0;
    // 999.5 would round to 1000: past that the next unit reads better.
    while value >= 999.5 && unit + 1 < UNITS {
        value /= STEP;
        unit += 1;
    }
    (unit, value, usize::from(unit > 0 && value < 9.95))
}

fn load_tone(share: f64) -> Tone {
    if share >= LOAD_ALERT {
        Tone::Alert
    } else if share >= LOAD_WARN {
        Tone::Warn
    } else {
        Tone::Calm
    }
}

fn battery_tone(battery: Battery) -> Tone {
    if battery.charging {
        Tone::Calm
    } else if battery.percent <= BATTERY_ALERT {
        Tone::Alert
    } else if battery.percent <= BATTERY_WARN {
        Tone::Warn
    } else {
        Tone::Calm
    }
}
