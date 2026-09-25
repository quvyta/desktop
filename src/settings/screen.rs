//! The Settings screen: what a person can change about the desktop, and what it cannot read.
//!
//! What every Quvyta application shares (the language, the theme, the icons, reduced motion) is
//! the framework's own appearance section, [`Appearance`], the same rows in every Quvyta
//! application: each with the box that says whether it is changed in every Quvyta application or
//! on the desktop alone, then the pillar and, where the desktop asks for its updates, the
//! ecosystem's update notice. The application holds the section and hands its changes to it. The
//! desktop's own preferences come from [`Prefs`], which the application owns; a change is
//! applied at once, so it can be seen, and handed back as a [`Request`] for the application to
//! write.
//!
//! The last section says where entries are read from and what could not be read there, each with
//! its file, line and column and a sentence in the person's language. Nothing on the screen is
//! drawn with lines, boxes or brackets: the sections are told apart by their headings and by
//! tone.

use qframe::prelude::*;
use qframe::widgets::{
    Appearance, AppearanceChange, EmptyState, ImageError, NumberInput, ScrollView, Segmented, Select, SettingRow,
    SettingsList, SettingsRows, Switch,
};

use super::{
    DockPosition, DragStyle, FRAME_CAP_LEAST, FRAME_CAP_MOST, FloorColor, FloorStyle, Prefs, SCROLLBACK_MOST, frame_cap,
};
use crate::apps::{Diagnostic, Folders};

/// The widget id of the list of settings, which takes the keyboard when the screen opens.
pub const LIST: &str = "settings-list";

/// The widget id of the field that holds a chosen frame cap.
pub const FRAME_CAP_FIELD: &str = "settings-frame-cap";

/// The widget id of the button that chooses a picture for the floor.
pub const CHOOSE_WALLPAPER: &str = "settings-choose-wallpaper";

/// Width of the desktop's own drop-downs: enough for the longest drag style and picture name.
const CONTROL_WIDTH: u16 = 18;

/// Cells a number field takes: five digits, the room the cursor needs after them, and the two
/// steppers. Two fewer cut the largest scrollback to its last three digits.
const NUMBER_WIDTH: u16 = 18;

/// How far the frame cap moves with one step of its field.
const FRAME_CAP_STEP: f64 = 5.0;

/// How far the scrollback moves with one step of its field.
const SCROLLBACK_STEP: f64 = 100.0;

/// Something that happened on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// Something was changed on the appearance section every Quvyta application shows.
    Appearance(AppearanceChange),
    /// The dock was moved to an edge.
    Dock(DockPosition),
    /// A colour was chosen for the floor.
    Floor(FloorColor),
    /// A pattern was chosen for the floor.
    FloorStyle(FloorStyle),
    /// "Choose…" beside the wallpaper: a picture of the person's own is to be picked.
    ChooseWallpaper,
    /// "Remove" beside the wallpaper: the floor goes back to its colour and pattern.
    RemoveWallpaper,
    /// One of qdesk's own pictures was chosen, by its place in [`crate::wallpapers::OURS`].
    OurWallpaper(usize),
    /// Folders were set to open in the ecosystem's file explorer (`true`) or in Files.
    FoldersInExplorer(bool),
    /// The status strip was shown on the dock (`true`) or taken off it.
    StatusStrip(bool),
    /// A drag style was chosen.
    Drag(DragStyle),
    /// A frame cap was chosen, or `None` to follow the link again.
    FrameCap(Option<u16>),
    /// A number of scrollback lines was set.
    Scrollback(u16),
    /// What the settings file could not be read as was read and can go.
    ReadProblems,
    /// The application wrote the settings, or could not.
    Stored(Result<(), String>),
}

/// What the screen asks the application for. The screen has already applied the change; what is
/// left is writing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Hand this change to the appearance section, which applies and saves it.
    Appearance(AppearanceChange),
    /// Use and store these preferences.
    Prefs(Prefs),
    /// Do what was asked about the floor's picture: nothing about it has changed yet, since a
    /// picture is decoded before it is taken.
    Wallpaper(Wallpaper),
}

/// What the Settings screen asks about the floor's picture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Wallpaper {
    /// Open the file picker.
    Choose,
    /// Take the picture away.
    Remove,
    /// Lay qdesk's own picture of this place in [`crate::wallpapers::OURS`] over the floor.
    Ours(usize),
}

/// The floor's picture as the Settings screen shows it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WallpaperRow {
    /// The picture's file name, when one is set.
    pub name: Option<String>,
    /// Which of qdesk's own pictures it is, when it is one.
    pub ours: Option<usize>,
    /// Why the picture set cannot be shown, when it cannot.
    pub problem: Option<ImageError>,
}

/// Where the entries of the applications come from and what could not be read there.
#[derive(Debug, Clone, Copy)]
pub struct Applications<'a> {
    /// The folders entries are read from.
    pub folders: &'a Folders,
    /// The entries that could not be read, in the order the files were read.
    pub diagnostics: &'a [Diagnostic],
}

/// The state of the screen: what the settings file itself could not be read as, until it is
/// read, why the last write failed, until one succeeds, and whether the ecosystem's update notice
/// is shown.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Screen {
    problems: Vec<Diagnostic>,
    problems_read: bool,
    failure: Option<String>,
    /// Whether the switch of the ecosystem's update notice is shown: only on a desktop that asks
    /// for a newer version of itself, so no switch is shown that would do nothing.
    updates: bool,
    /// The floor's picture as the application last told it.
    wallpaper: WallpaperRow,
}

impl Screen {
    /// The screen over a settings file that could not be read in full; `problems` is what
    /// [`super::Loaded::diagnostics`] held.
    #[must_use]
    pub fn new(problems: Vec<Diagnostic>) -> Self {
        Self { problems, problems_read: false, failure: None, updates: false, wallpaper: WallpaperRow::default() }
    }

    /// The same screen showing the switch of the ecosystem's update notice, or not.
    #[must_use]
    pub fn with_updates(mut self, shown: bool) -> Self {
        self.updates = shown;
        self
    }

    /// Puts `problems` in place of what the settings file could not be read as, after the file was
    /// read again. New problems are shown again even when the old ones had been put away; the same
    /// problems stay as they were. Whether anything changed is the answer.
    pub fn reread(&mut self, problems: Vec<Diagnostic>) -> bool {
        if self.problems == problems {
            return false;
        }
        self.problems = problems;
        self.problems_read = false;
        true
    }

    /// Shows `row` as the floor's picture: the application tells the screen whenever the picture
    /// or what became of it changes.
    pub fn set_wallpaper(&mut self, row: WallpaperRow) {
        self.wallpaper = row;
    }

    /// The floor's picture as the screen shows it.
    #[must_use]
    pub fn wallpaper(&self) -> &WallpaperRow {
        &self.wallpaper
    }

    /// Whether the switch of the ecosystem's update notice is shown.
    #[must_use]
    pub fn updates(&self) -> bool {
        self.updates
    }
}

/// Applies `msg` over the preferences `prefs` and says what the application should do.
pub fn update<M: From<Msg> + Clone + Send + 'static>(
    screen: &mut Screen,
    prefs: &Prefs,
    msg: Msg,
) -> (Command<M>, Option<Request>) {
    match msg {
        // A screen without the switch was never offered it; nothing is written for it.
        Msg::Appearance(AppearanceChange::UpdateNotice(_)) if !screen.updates => (Command::none(), None),
        // The section applies a change itself as it saves it, so the application does both.
        Msg::Appearance(change) => (Command::none(), Some(Request::Appearance(change))),
        Msg::Dock(dock) => (Command::none(), Some(Request::Prefs(Prefs { dock, ..*prefs }))),
        Msg::Floor(floor) => (Command::none(), Some(Request::Prefs(Prefs { floor, ..*prefs }))),
        Msg::FloorStyle(floor_style) => (Command::none(), Some(Request::Prefs(Prefs { floor_style, ..*prefs }))),
        Msg::ChooseWallpaper => (Command::none(), Some(Request::Wallpaper(Wallpaper::Choose))),
        Msg::RemoveWallpaper => (Command::none(), Some(Request::Wallpaper(Wallpaper::Remove))),
        Msg::OurWallpaper(index) => (Command::none(), Some(Request::Wallpaper(Wallpaper::Ours(index)))),
        Msg::FoldersInExplorer(folders_in_explorer) => {
            (Command::none(), Some(Request::Prefs(Prefs { folders_in_explorer, ..*prefs })))
        }
        Msg::StatusStrip(status_strip) => (Command::none(), Some(Request::Prefs(Prefs { status_strip, ..*prefs }))),
        Msg::Drag(drag) => (Command::none(), Some(Request::Prefs(Prefs { drag, ..*prefs }))),
        Msg::FrameCap(frames) => {
            let frame_cap = frames.map(|frames| frames.clamp(FRAME_CAP_LEAST, FRAME_CAP_MOST));
            (Command::none(), Some(Request::Prefs(Prefs { frame_cap, ..*prefs })))
        }
        Msg::Scrollback(lines) => {
            let scrollback = lines.min(SCROLLBACK_MOST);
            (Command::none(), Some(Request::Prefs(Prefs { scrollback, ..*prefs })))
        }
        Msg::ReadProblems => {
            screen.problems_read = true;
            (Command::none(), None)
        }
        Msg::Stored(result) => {
            screen.failure = result.err();
            (Command::none(), None)
        }
    }
}

/// Draws the screen: what the settings file could not be read as until it is read, the last
/// failure to write, the settings by section, and where the applications come from. `remote` says
/// what the frame cap comes to on this terminal, since that is the one setting still tied to the
/// connection. `appearance` is the section every Quvyta application shows first.
pub fn view<M: From<Msg> + Clone + Send + 'static>(
    screen: &Screen,
    prefs: &Prefs,
    remote: bool,
    appearance: &Appearance,
    apps: &Applications<'_>,
    ui: &mut View<'_, M>,
) {
    ui.add_with(ScrollView::new(), |ui| {
        ui.column(|ui| {
            if !screen.problems_read && !screen.problems.is_empty() {
                file_problems(&screen.problems, ui);
            }
            if let Some(reason) = &screen.failure {
                warning_line(t!("settings.store-failed", reason = reason.clone()), ui);
            }
            let list = SettingsList::show(ui, |list| {
                // The same rows, words and order as in every other Quvyta application.
                let change = |change| M::from(Msg::Appearance(change));
                appearance.section(list, change);
                if screen.updates {
                    appearance.updates(list, change);
                }

                list.heading(t!("settings.desktop"));
                let sides = DockPosition::ALL.map(|side| t!(&format!("settings.dock-{}", side.name())));
                let chosen = DockPosition::ALL.iter().position(|side| *side == prefs.dock).unwrap_or(0);
                let row = SettingRow::new(t!("settings.dock")).description(t!("settings.dock-text"));
                list.row(row, |ui| {
                    ui.add(
                        Segmented::new(sides)
                            .selected(chosen)
                            .on_select(|index| M::from(Msg::Dock(DockPosition::ALL[index]))),
                    );
                });
                // Named choices: the floor around this window shows the tone the moment it is
                // chosen, which says more than a swatch beside the name would.
                let tones = FloorColor::ALL.map(|tone| t!(&format!("settings.floor-color-{}", tone.name())));
                let chosen = FloorColor::ALL.iter().position(|tone| *tone == prefs.floor).unwrap_or(0);
                let row = SettingRow::new(t!("settings.floor-color")).description(t!("settings.floor-color-text"));
                list.row(row, |ui| {
                    ui.add(
                        Segmented::new(tones)
                            .selected(chosen)
                            .on_select(|index| M::from(Msg::Floor(FloorColor::ALL[index]))),
                    );
                });
                // The pattern beside the colour, and like it shown at once on the floor around.
                let styles = FloorStyle::ALL.map(|style| t!(&format!("settings.floor-style-{}", style.name())));
                let chosen = FloorStyle::ALL.iter().position(|style| *style == prefs.floor_style).unwrap_or(0);
                let row = SettingRow::new(t!("settings.floor-style")).description(t!("settings.floor-style-text"));
                list.row(row, |ui| {
                    ui.add(
                        Segmented::new(styles)
                            .selected(chosen)
                            .on_select(|index| M::from(Msg::FloorStyle(FloorStyle::ALL[index]))),
                    );
                });
                wallpaper_rows(&screen.wallpaper, list);

                // The explorer's name is the program's, the one a person installs and would type.
                let about = t!("settings.folders-in-explorer-text", program = crate::app::EXPLORER);
                let row = SettingRow::new(t!("settings.folders-in-explorer", program = crate::app::EXPLORER))
                    .description(about);
                list.row(row, |ui| {
                    ui.add(Switch::new(prefs.folders_in_explorer).on_toggle(|on| M::from(Msg::FoldersInExplorer(on))));
                });
                let row = SettingRow::new(t!("settings.status-strip")).description(t!("settings.status-strip-text"));
                list.row(row, |ui| {
                    ui.add(Switch::new(prefs.status_strip).on_toggle(|on| M::from(Msg::StatusStrip(on))));
                });

                list.heading(t!("settings.connection"));
                let styles = DragStyle::ALL.map(|style| t!(&format!("settings.drag-{}", style.name())));
                let chosen = DragStyle::ALL.iter().position(|style| *style == prefs.drag);
                let row = SettingRow::new(t!("settings.drag")).description(drag_note(prefs.drag));
                list.row(row, |ui| {
                    ui.add(
                        Select::new(styles)
                            .selected(chosen)
                            .on_select(|index| M::from(Msg::Drag(DragStyle::ALL[index]))),
                    )
                    .width(Length::Cells(CONTROL_WIDTH));
                });
                let follows = prefs.frame_cap.is_none();
                let caps = [t!("settings.frame-cap-automatic"), t!("settings.frame-cap-chosen")];
                let row =
                    SettingRow::new(t!("settings.frame-cap")).description(frame_cap_note(prefs.frame_cap, remote));
                list.row(row, |ui| {
                    ui.add(Segmented::new(caps).selected(usize::from(!follows)).on_select(move |index| {
                        M::from(Msg::FrameCap((index == 1).then_some(frame_cap(None, remote))))
                    }));
                });
                if let Some(frames) = prefs.frame_cap {
                    let row = SettingRow::new(t!("settings.frame-cap-frames")).nested(true);
                    list.row(row, |ui| {
                        ui.add(
                            NumberInput::new(f64::from(frames))
                                .range(f64::from(FRAME_CAP_LEAST), f64::from(FRAME_CAP_MOST))
                                .step(FRAME_CAP_STEP)
                                .steppers(true)
                                .on_change(|frames| M::from(Msg::FrameCap(Some(whole(frames))))),
                        )
                        .id(FRAME_CAP_FIELD)
                        .width(Length::Cells(NUMBER_WIDTH));
                    });
                }
                let row = SettingRow::new(t!("settings.scrollback")).description(t!("settings.scrollback-text"));
                list.row(row, |ui| {
                    ui.add(
                        NumberInput::new(f64::from(prefs.scrollback))
                            .range(0.0, f64::from(SCROLLBACK_MOST))
                            .step(SCROLLBACK_STEP)
                            .steppers(true)
                            .on_change(|lines| M::from(Msg::Scrollback(whole(lines)))),
                    )
                    .width(Length::Cells(NUMBER_WIDTH));
                });
            });
            list.id(LIST);

            applications(apps, ui);
        })
        .gap(1)
        .padding(Padding { top: 1, right: 2, bottom: 1, left: 2 })
        .fill_width();
    })
    .fill();
}

/// The floor's picture: its name with the way to choose another and to take it away, and under it
/// qdesk's own pictures by name.
fn wallpaper_rows<M: From<Msg> + Clone + Send + 'static>(row: &WallpaperRow, list: &mut SettingsRows<'_, M>) {
    let about = match row.problem {
        Some(problem) => t!("settings.wallpaper-unshown", reason = problem.to_string()),
        None => t!("settings.wallpaper-text"),
    };
    let shown = row.name.clone().unwrap_or_else(|| t!("settings.wallpaper-none"));
    let set = row.name.is_some();
    list.row(SettingRow::new(t!("settings.wallpaper")).description(about), |ui| {
        ui.row(|ui| {
            ui.add(Text::new(shown).role(if set { "text" } else { "secondary" }).no_wrap());
            ui.add(Button::new(t!("settings.wallpaper-choose")).on_press(M::from(Msg::ChooseWallpaper)))
                .id(CHOOSE_WALLPAPER);
            if set {
                ui.add(Button::new(t!("settings.wallpaper-remove")).on_press(M::from(Msg::RemoveWallpaper)));
            }
        })
        .gap(1);
    });
    let names = crate::wallpapers::OURS.map(|ours| t!(&format!("settings.wallpaper-{}", ours.name)));
    // A drop-down rather than segments: segments always have one chosen, and a picture of the
    // person's own is none of these.
    list.row(SettingRow::new(t!("settings.wallpaper-ours")).nested(true), |ui| {
        ui.add(
            Select::new(names)
                .selected(row.ours)
                .placeholder(t!("settings.wallpaper-ours-pick"))
                .on_select(|index| M::from(Msg::OurWallpaper(index))),
        )
        .width(Length::Cells(CONTROL_WIDTH));
    });
}

/// A number field's value as a whole number, never below zero.
fn whole(value: f64) -> u16 {
    if value <= 0.0 { 0 } else { value.round().min(f64::from(u16::MAX)) as u16 }
}

/// The line under the drag style: what it means in practice, so `live` and `ghost` are not words
/// without a meaning.
fn drag_note(style: DragStyle) -> String {
    match style {
        DragStyle::Live => t!("settings.drag-in-force-live"),
        DragStyle::Ghost => t!("settings.drag-in-force-ghost"),
    }
}

/// The line under the frame cap: how many frames a second it comes to here.
fn frame_cap_note(setting: Option<u16>, remote: bool) -> String {
    t!("settings.frame-cap-in-force", frames = frame_cap(setting, remote))
}

/// Where the entries come from, and the ones that could not be read.
fn applications<M: From<Msg> + Clone + Send + 'static>(apps: &Applications<'_>, ui: &mut View<'_, M>) {
    ui.column(|ui| {
        ui.add(Text::new(t!("settings.applications")).bold()).fill_width();
        ui.add(Text::new(t!("settings.folders")).role("secondary")).fill_width();
        for folder in apps.folders.watched() {
            ui.add(Text::new(folder.display().to_string()).no_wrap());
        }
        if apps.diagnostics.is_empty() {
            ui.add(EmptyState::new(t!("settings.no-problems")).message(t!("settings.no-problems-text"))).fill_width();
        } else {
            ui.add(Text::new(t!("settings.problems")).role("secondary")).fill_width();
            for diagnostic in apps.diagnostics {
                problem(diagnostic, ui);
            }
        }
    })
    .gap(0)
    .fill_width();
}

/// What the settings file itself could not be read as, with the way to put it away.
fn file_problems<M: From<Msg> + Clone + Send + 'static>(problems: &[Diagnostic], ui: &mut View<'_, M>) {
    ui.column(|ui| {
        warning_line(t!("settings.file-problems"), ui);
        for diagnostic in problems {
            problem(diagnostic, ui);
        }
        ui.add(Button::new(t!("settings.read")).on_press(M::from(Msg::ReadProblems)));
    })
    .gap(0)
    .fill_width();
}

/// One problem: what is wrong in the person's language, and under it the place and, when there
/// is one, the words of the parser or of the system.
fn problem<M: 'static>(diagnostic: &Diagnostic, ui: &mut View<'_, M>) {
    ui.add(Text::new(crate::notice::what(&diagnostic.kind))).fill_width();
    let place = match &diagnostic.detail {
        Some(detail) => format!("{} — {detail}", diagnostic.location()),
        None => diagnostic.location(),
    };
    ui.add(Text::new(place).role("faint")).fill_width();
}

/// A remark marked as a warning, without a box around it.
fn warning_line<M: 'static>(text: String, ui: &mut View<'_, M>) {
    let mark = ui.env().icons().glyph("warning").into_owned();
    ui.add(Text::rich([Span::new(format!("{mark} ")).color("warning"), Span::new(text).role("secondary")]))
        .fill_width();
}
