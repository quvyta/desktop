//! The Settings screen: what a person can change about the desktop, and what it cannot read.
//!
//! What the framework keeps for every Quvyta application (the language, the theme, the glyph
//! mode) is read from the environment the screen draws in, which is the one place that knows
//! what is in force: a saved value the shell overrides would show a choice nobody made. The
//! desktop's own preferences come from [`Prefs`], which the application owns; a change is
//! applied at once, so it can be seen, and handed back as a [`Request`] for the application to
//! write.
//!
//! The last section says where entries are read from and what could not be read there, each with
//! its file, line and column and a sentence in the person's language. Nothing on the screen is
//! drawn with lines, boxes or brackets: the sections are told apart by their headings and by
//! tone.

use qframe::icons::IconMode;
use qframe::prelude::*;
use qframe::storage::Family;
use qframe::widgets::{EmptyState, NumberInput, ScrollView, Segmented, Select, SettingRow, SettingsList, Switch};

use super::{DockPosition, DragStyle, FRAME_CAP_LEAST, FRAME_CAP_MOST, FloorColor, Prefs, SCROLLBACK_MOST, frame_cap};
use crate::apps::{Diagnostic, Folders};

/// The widget id of the list of settings, which takes the keyboard when the screen opens.
pub const LIST: &str = "settings-list";

/// The widget id of the field that holds a chosen frame cap.
pub const FRAME_CAP_FIELD: &str = "settings-frame-cap";

/// The widget id of the switch of the ecosystem's update notice.
pub const UPDATE_NOTICE: &str = "settings-update-notice";

/// Width of the drop-downs: enough for the longest theme, language and glyph mode name.
const CONTROL_WIDTH: u16 = 18;

/// Cells a number field takes: five digits, the room the cursor needs after them, and the two
/// steppers. Two fewer cut the largest scrollback to its last three digits.
const NUMBER_WIDTH: u16 = 18;

/// How far the frame cap moves with one step of its field.
const FRAME_CAP_STEP: f64 = 5.0;

/// How far the scrollback moves with one step of its field.
const SCROLLBACK_STEP: f64 = 100.0;

/// A change to something every Quvyta application shares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Shared {
    /// The language, by code.
    Language(String),
    /// The theme, by id.
    Theme(String),
    /// The glyph mode.
    Icons(IconMode),
}

/// Something that happened on the screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Msg {
    /// A shared setting was chosen.
    Shared(Shared),
    /// The dock was moved to an edge.
    Dock(DockPosition),
    /// A colour was chosen for the floor.
    Floor(FloorColor),
    /// Folders were set to open in the ecosystem's file explorer (`true`) or in Files.
    FoldersInExplorer(bool),
    /// A drag style was chosen.
    Drag(DragStyle),
    /// A frame cap was chosen, or `None` to follow the link again.
    FrameCap(Option<u16>),
    /// A number of scrollback lines was set.
    Scrollback(u16),
    /// The ecosystem's update notice was switched on (`true`) or off.
    UpdateNotice(bool),
    /// What the settings file could not be read as was read and can go.
    ReadProblems,
    /// The application wrote the settings, or could not.
    Stored(Result<(), String>),
}

/// What the screen asks the application for. The screen has already applied the change; what is
/// left is writing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    /// Store a shared setting.
    Shared(Shared),
    /// Use and store these preferences.
    Prefs(Prefs),
    /// Turn the ecosystem's update notice on (`true`) or off in the ecosystem's shared file.
    UpdateNotice(bool),
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
/// read, why the last write failed, until one succeeds, and the ecosystem's update notice where the
/// desktop asks for its updates.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Screen {
    problems: Vec<Diagnostic>,
    problems_read: bool,
    failure: Option<String>,
    /// The ecosystem's update notice as the screen shows it, or `None` for a desktop that asks for no
    /// newer version and so shows no switch that would do nothing.
    update_notice: Option<bool>,
}

impl Screen {
    /// The screen over a settings file that could not be read in full; `problems` is what
    /// [`super::Loaded::diagnostics`] held.
    #[must_use]
    pub fn new(problems: Vec<Diagnostic>) -> Self {
        Self { problems, problems_read: false, failure: None, update_notice: None }
    }

    /// The same screen showing the ecosystem's update notice as `on`, or showing no switch at all
    /// with `None`.
    #[must_use]
    pub fn with_update_notice(mut self, on: Option<bool>) -> Self {
        self.update_notice = on;
        self
    }

    /// The ecosystem's update notice as the screen shows it; `None` when it shows no switch.
    #[must_use]
    pub fn update_notice(&self) -> Option<bool> {
        self.update_notice
    }
}

/// Applies `msg` over the preferences `prefs` and says what the application should do.
pub fn update<M: From<Msg> + Clone + Send + 'static>(
    screen: &mut Screen,
    prefs: &Prefs,
    msg: Msg,
) -> (Command<M>, Option<Request>) {
    match msg {
        Msg::Shared(change) => {
            let command = match &change {
                Shared::Language(code) => Command::set_locale(code.clone()),
                Shared::Theme(id) => Command::set_theme(id.clone()),
                Shared::Icons(mode) => Command::set_icon_mode(*mode),
            };
            (command, Some(Request::Shared(change)))
        }
        Msg::Dock(dock) => (Command::none(), Some(Request::Prefs(Prefs { dock, ..*prefs }))),
        Msg::Floor(floor) => (Command::none(), Some(Request::Prefs(Prefs { floor, ..*prefs }))),
        Msg::FoldersInExplorer(folders_in_explorer) => {
            (Command::none(), Some(Request::Prefs(Prefs { folders_in_explorer, ..*prefs })))
        }
        Msg::Drag(drag) => (Command::none(), Some(Request::Prefs(Prefs { drag, ..*prefs }))),
        Msg::FrameCap(frames) => {
            let frame_cap = frames.map(|frames| frames.clamp(FRAME_CAP_LEAST, FRAME_CAP_MOST));
            (Command::none(), Some(Request::Prefs(Prefs { frame_cap, ..*prefs })))
        }
        Msg::Scrollback(lines) => {
            let scrollback = lines.min(SCROLLBACK_MOST);
            (Command::none(), Some(Request::Prefs(Prefs { scrollback, ..*prefs })))
        }
        Msg::UpdateNotice(on) => {
            // A screen without the switch was never offered it; nothing is written for it.
            if screen.update_notice.is_none() {
                return (Command::none(), None);
            }
            screen.update_notice = Some(on);
            (Command::none(), Some(Request::UpdateNotice(on)))
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
/// connection.
pub fn view<M: From<Msg> + Clone + Send + 'static>(
    screen: &Screen,
    prefs: &Prefs,
    remote: bool,
    apps: &Applications<'_>,
    ui: &mut View<'_, M>,
) {
    let languages = ui.env().i18n().list();
    let language = ui.env().i18n().active().to_owned();
    let themes = ui.env().themes();
    let theme = ui.env().theme().id().to_owned();
    let icons = ui.env().icon_mode();

    ui.add_with(ScrollView::new(), |ui| {
        ui.column(|ui| {
            if !screen.problems_read && !screen.problems.is_empty() {
                file_problems(&screen.problems, ui);
            }
            if let Some(reason) = &screen.failure {
                warning_line(t!("settings.store-failed", reason = reason.clone()), ui);
            }
            let list = SettingsList::show(ui, |list| {
                list.heading(t!("settings.appearance"));
                let codes: Vec<String> = languages.iter().map(|(code, _)| code.clone()).collect();
                let names: Vec<String> = languages.iter().map(|(_, name)| name.clone()).collect();
                let chosen = codes.iter().position(|code| *code == language);
                list.row(SettingRow::new(t!("settings.language")), |ui| {
                    ui.add(
                        Select::new(names)
                            .selected(chosen)
                            .on_select(move |index| M::from(Msg::Shared(Shared::Language(codes[index].clone())))),
                    )
                    .width(Length::Cells(CONTROL_WIDTH));
                });
                let ids: Vec<String> = themes.iter().map(|(id, _)| id.clone()).collect();
                let titles: Vec<String> = themes.iter().map(|(_, name)| name.clone()).collect();
                let chosen = ids.iter().position(|id| *id == theme);
                list.row(SettingRow::new(t!("settings.theme")), |ui| {
                    ui.add(
                        Select::new(titles)
                            .selected(chosen)
                            .on_select(move |index| M::from(Msg::Shared(Shared::Theme(ids[index].clone())))),
                    )
                    .width(Length::Cells(CONTROL_WIDTH));
                });
                let modes = IconMode::ALL.map(|mode| t!(&format!("settings.glyphs-{}", mode.name())));
                let chosen = IconMode::ALL.iter().position(|mode| *mode == icons);
                let row = SettingRow::new(t!("settings.glyphs")).description(t!("settings.glyphs-text"));
                list.row(row, |ui| {
                    ui.add(
                        Select::new(modes)
                            .selected(chosen)
                            .on_select(|index| M::from(Msg::Shared(Shared::Icons(IconMode::ALL[index])))),
                    )
                    .width(Length::Cells(CONTROL_WIDTH));
                });
                // The ecosystem's own words, the same in every Quvyta application that asks.
                if let Some(on) = screen.update_notice {
                    let about = t!("quvyta.appearance.updates-text", family = Family::QUVYTA.title());
                    list.row(SettingRow::new(t!("quvyta.appearance.updates")).description(about), |ui| {
                        ui.add(Switch::new(on).on_toggle(|on| M::from(Msg::UpdateNotice(on)))).id(UPDATE_NOTICE);
                    });
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

                // The explorer's name is the program's, the one a person installs and would type.
                let about = t!("settings.folders-in-explorer-text", program = crate::app::EXPLORER);
                let row = SettingRow::new(t!("settings.folders-in-explorer", program = crate::app::EXPLORER))
                    .description(about);
                list.row(row, |ui| {
                    ui.add(Switch::new(prefs.folders_in_explorer).on_toggle(|on| M::from(Msg::FoldersInExplorer(on))));
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
