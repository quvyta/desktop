//! What a person can set about the desktop, where it is kept, and how a value becomes the one
//! in force.
//!
//! The desktop shares the look of every Quvyta application: the theme, the language and the
//! glyph mode are the framework's keys, kept beside qdesk's own in one file. qdesk's own keys
//! say where the dock sits, how a window follows the mouse while it is dragged, how often the
//! screen may be drawn and how many lines a terminal window remembers.
//!
//! The file is `desktop.conf` in the Quvyta ecosystem's folder, `~/.config/quvyta` on Linux, next
//! to the files of the other Quvyta applications; the desktop's other configuration files
//! live in the `desktop/` folder beside it. It is read and written through the framework's
//! settings storage, so a write is atomic and a value that is the default is never written: the
//! file holds only what was chosen.
//!
//! Reading never fails. A file that is not readable as it is written becomes a [`Diagnostic`]
//! that names the line and the column, every key it could not use keeps its default, and the
//! desktop opens. Nothing is repaired behind the person's back: what they wrote stays as they
//! wrote it, and keys a newer qdesk knows are kept when this one saves.
//!
//! One setting does not name its value outright but follows the link the desktop is drawn over.
//! Whether that link is remote is the framework's answer, [`Env::remote`](qframe::env::Env::remote);
//! [`frame_cap`] takes it as `remote` and resolves to a number. The drag style used to follow it
//! the same way, but a ghost costs less than half of what dragging the window itself costs to
//! send and shows the window's shape before it lands, so it is no longer split by the link: it is
//! `ghost` everywhere, and only Settings changes it.

mod screen;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};

use qframe::runtime::FrameLimit;
use qframe::storage::{Family, Schema, Setting, SettingKind, Settings};

pub use screen::{Applications, LIST, Msg, Request, Screen, Shared, UPDATE_NOTICE, update, view};

use crate::apps::{Diagnostic, DiagnosticKind, Position};

/// The desktop's id in the Quvyta ecosystem: its settings are `desktop.conf` and its other
/// configuration files are under `desktop/`.
pub const APP: &str = "desktop";

/// Where the ecosystem's update notice is kept and where qdesk remembers when it last asked for a
/// newer version of itself.
///
/// The switch is the ecosystem's, one for every Quvyta application, so it is read from the ecosystem's
/// shared file rather than from `desktop.conf`. A test gives folders of its own, so nothing it
/// does reads or turns off the person's own switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateFolders {
    /// The ecosystem's configuration folder, whose shared file holds the switch.
    pub config: PathBuf,
    /// qdesk's state folder, which remembers when the question was last asked.
    pub state: PathBuf,
}

impl UpdateFolders {
    /// This machine's folders, or `None` without a home folder, where nothing could remember the
    /// switch or the last question and so nothing is asked.
    #[must_use]
    pub fn here() -> Option<Self> {
        let ecosystem = Family::QUVYTA;
        ecosystem.config_dir().zip(ecosystem.state_dir(APP)).map(|(config, state)| Self { config, state })
    }
}

/// The key of the dock's side: `top` or `bottom`.
pub const DOCK_POSITION: &str = "dock-position";
/// The key of the drag style: `live` or `ghost`.
pub const DRAG_STYLE: &str = "drag-style";
/// The key of the frame cap in frames a second. Without it the cap follows the link.
pub const FRAME_CAP: &str = "frame-cap";
/// The key of how many lines a terminal window remembers above its top edge.
pub const SCROLLBACK: &str = "scrollback";

/// The lines a terminal window remembers until someone says otherwise.
pub const SCROLLBACK_DEFAULT: u16 = 2_000;
/// The most lines a terminal window may remember. Twenty windows of ten thousand lines are more
/// memory than a small server has, so the number stops here.
pub const SCROLLBACK_MOST: u16 = 10_000;

/// Frames a second on a terminal of this machine: the framework's own local default, so the
/// number the settings screen shows is the number the runtime draws by.
pub const FRAME_CAP_LOCAL: u16 = FrameLimit::LOCAL as u16;
/// Frames a second on a terminal reached over SSH, where every changed cell costs bytes: the
/// framework's remote default.
pub const FRAME_CAP_REMOTE: u16 = FrameLimit::REMOTE as u16;
/// The lowest frame cap that can be chosen: one frame a second still answers a key press, which
/// never waits for the cap.
pub const FRAME_CAP_LEAST: u16 = 1;
/// The highest frame cap that can be chosen.
pub const FRAME_CAP_MOST: u16 = 240;

/// Which edge of the screen the dock sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DockPosition {
    /// The top row.
    Top,
    /// The bottom row, where a desktop's task bar is looked for.
    #[default]
    Bottom,
}

impl DockPosition {
    /// Both sides, in the order a chooser lists them.
    pub const ALL: [Self; 2] = [Self::Top, Self::Bottom];

    /// The name written in the settings file, also the key its label is looked up by.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
        }
    }

    /// The side a name means, if it is one of ours.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|side| side.name() == name)
    }
}

/// How a window follows the mouse while it is dragged.
///
/// `Ghost` is the default everywhere, local or over SSH: it costs less than half of what `Live`
/// costs to send at each step, and it shows the window's shape before it lands, which is a good
/// way to drag a window on any machine, not only a slow one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum DragStyle {
    /// The window itself moves under the mouse.
    Live,
    /// Only a shaded area the size of the window moves; the window jumps there when it is let go.
    #[default]
    Ghost,
}

impl DragStyle {
    /// Both styles, in the order a chooser lists them.
    pub const ALL: [Self; 2] = [Self::Live, Self::Ghost];

    /// The name written in the settings file, also the key its label is looked up by.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Live => "live",
            Self::Ghost => "ghost",
        }
    }

    /// The style a name means, if it is one of ours.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|style| style.name() == name)
    }
}

/// The frame cap in force, in frames a second: the number that was chosen, or the one a `remote`
/// link asks for when none was.
#[must_use]
pub fn frame_cap(setting: Option<u16>, remote: bool) -> u16 {
    match setting {
        Some(chosen) => chosen.clamp(FRAME_CAP_LEAST, FRAME_CAP_MOST),
        None if remote => FRAME_CAP_REMOTE,
        None => FRAME_CAP_LOCAL,
    }
}

/// What a person has set about the desktop itself. The theme, the language and the glyph mode
/// are not here: they belong to every Quvyta application and the framework keeps them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Prefs {
    /// Which edge the dock sits on.
    pub dock: DockPosition,
    /// How a window follows the mouse while it is dragged.
    pub drag: DragStyle,
    /// Frames a second, when a number was chosen; `None` follows the link.
    pub frame_cap: Option<u16>,
    /// Lines a terminal window remembers above its top edge; `0` remembers none.
    pub scrollback: u16,
}

impl Default for Prefs {
    /// The dock at the bottom, a ghost drag, the frame cap following the link, two thousand
    /// lines of scrollback.
    fn default() -> Self {
        Self {
            dock: DockPosition::default(),
            drag: DragStyle::default(),
            frame_cap: None,
            scrollback: SCROLLBACK_DEFAULT,
        }
    }
}

impl Prefs {
    /// What the settings file may hold: the framework's keys and the desktop's own, each with
    /// what it accepts and what applies without it.
    #[must_use]
    pub fn schema() -> Schema {
        let least = i64::from(FRAME_CAP_LEAST);
        let most = i64::from(FRAME_CAP_MOST);
        Schema::builtin()
            .choice(DOCK_POSITION, DockPosition::ALL.map(DockPosition::name), DockPosition::default().name())
            .choice(DRAG_STYLE, DragStyle::ALL.map(DragStyle::name), DragStyle::default().name())
            // No default of its own: without the key the cap follows the link, and a number
            // outside the range is left out rather than replaced by one.
            .optional(FRAME_CAP, SettingKind::check(move |frames: &i64| (least..=most).contains(frames)))
            .check(SCROLLBACK, i64::from(SCROLLBACK_DEFAULT), |lines: &i64| {
                (0..=i64::from(SCROLLBACK_MOST)).contains(lines)
            })
    }

    /// What `settings` hold. A key that is missing, or holds what the schema would not accept,
    /// gives what applies without it.
    #[must_use]
    pub fn from_settings(settings: &Settings) -> Self {
        let defaults = Self::default();
        Self {
            dock: settings
                .get::<String>(DOCK_POSITION)
                .and_then(|name| DockPosition::from_name(&name))
                .unwrap_or(defaults.dock),
            drag: settings
                .get::<String>(DRAG_STYLE)
                .and_then(|name| DragStyle::from_name(&name))
                .unwrap_or(defaults.drag),
            frame_cap: settings
                .get::<i64>(FRAME_CAP)
                .and_then(|frames| u16::try_from(frames).ok())
                .filter(|frames| (FRAME_CAP_LEAST..=FRAME_CAP_MOST).contains(frames)),
            scrollback: settings
                .get::<i64>(SCROLLBACK)
                .and_then(|lines| u16::try_from(lines).ok())
                .filter(|lines| *lines <= SCROLLBACK_MOST)
                .unwrap_or(defaults.scrollback),
        }
    }

    /// Writes these preferences into `settings`. A value that is the default is taken out of the
    /// file instead of written, so the file holds only what was chosen and a later qdesk may
    /// change a default without arguing with an old file.
    pub fn write(&self, settings: &mut Settings) {
        let defaults = Self::default();
        store(settings, DOCK_POSITION, self.dock.name().to_owned(), self.dock == defaults.dock);
        store(settings, DRAG_STYLE, self.drag.name().to_owned(), self.drag == defaults.drag);
        let frames = self.frame_cap.unwrap_or(FRAME_CAP_LOCAL);
        store(settings, FRAME_CAP, i64::from(frames), self.frame_cap.is_none());
        store(settings, SCROLLBACK, i64::from(self.scrollback), self.scrollback == defaults.scrollback);
    }
}

/// Stores `value` under `key`, or removes the key when the value `is_default`.
fn store<T: Setting>(settings: &mut Settings, key: &str, value: T, is_default: bool) {
    if is_default {
        settings.remove(key);
    } else {
        settings.set(key, value);
    }
}

/// The settings as they were read, with what could not be read reported rather than thrown.
#[derive(Debug)]
pub struct Loaded {
    /// The settings file, to change a value in and save.
    pub settings: Settings,
    /// What the file says the desktop should be, with defaults where it says nothing usable.
    pub prefs: Prefs,
    /// What the file could not be read as; each names the file, the line and the column.
    pub diagnostics: Vec<Diagnostic>,
}

/// Reads the settings from the ecosystem's folder on this machine. Without a home folder to write
/// in, the settings stay in memory and changing one does nothing more than apply it.
#[must_use]
pub fn load() -> Loaded {
    match Family::QUVYTA.config_dir() {
        Some(folder) => load_in(&folder),
        None => read(Settings::load_member(&Family::QUVYTA, APP)),
    }
}

/// [`load`] with `config_dir` as the ecosystem's folder, so a test or a demo never touches the
/// person's own settings.
#[must_use]
pub fn load_in(config_dir: &Path) -> Loaded {
    read(Settings::open(config_dir.join(format!("{APP}.conf"))).member_of(&Family::QUVYTA))
}

/// Checks `settings` against the desktop's keys and reads the preferences out of them.
fn read(settings: Settings) -> Loaded {
    // Not self-healing: the file belongs to the person who wrote it. A value qdesk cannot use is
    // said out loud and the default stands in its place, and the line stays as it was written.
    let settings = settings.schema(Prefs::schema());
    let prefs = Prefs::from_settings(&settings);
    let diagnostics = problems(&settings);
    Loaded { settings, prefs, diagnostics }
}

/// The problems of the settings file as the desktop's own diagnostics, so the Settings screen
/// shows them with the file, the line and the column beside the problems of the entry files.
///
/// The framework says in one English sentence what it could not use and does not sort its
/// problems into kinds; that sentence is the detail under the desktop's own line, which says
/// that the file could not be read as it is written.
fn problems(settings: &Settings) -> Vec<Diagnostic> {
    let path = settings.path().map_or_else(|| PathBuf::from(format!("{APP}.conf")), Path::to_path_buf);
    settings
        .diagnostics()
        .iter()
        .map(|problem| {
            let position =
                problem.location.as_ref().map(|location| Position { line: location.line, column: location.column });
            Diagnostic::at(path.clone(), position, DiagnosticKind::Syntax).with_detail(problem.message.clone())
        })
        .collect()
}
