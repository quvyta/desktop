//! The desktop's side of the gadgets (design 3.12): what their menus offer, what choosing a row
//! does, the readings and the ticks that keep them current, the note files, and the nodes the
//! floor draws.
//!
//! What a gadget is and where it stands is [`crate::gadgets`]; this is where the desktop's state
//! meets it.

use std::path::PathBuf;
use std::time::Duration;

use qframe::date::DateTime;
use qframe::prelude::*;
use qframe::runtime::Task;
use qframe::storage::atomic_write;
use qframe::widgets::{BigText, ContextItem, ContextMenu, Gauge, Gradient, TextArea, Toast};

use super::{Desk, Msg};
use crate::desktop::grid::{CELL_HEIGHT, CELL_WIDTH, Grid};
use crate::gadgets::face::{Line, Month, Surface};
use crate::gadgets::{self, Change, Clock, Gadget, Kind, Reading, Readings, Shows, Tint, WeekStart};
use crate::inbox::Notice;
use crate::status::{self, Probe, SAMPLE_EVERY, Status};

/// The share of the processor or the memory from which a meter turns to the warning tone: the
/// status strip's own limit (3.10).
const WARN: f32 = 75.0;
/// The share from which it turns to the danger tone.
const DANGER: f32 = 90.0;

/// The name of the text area of the note kept in `file`, so a test or a key can put the keys in
/// it.
#[must_use]
pub fn note_field(file: &str) -> String {
    format!("note:{file}")
}

impl Desk {
    /// The probe the system gadget reads the machine with. Without one the gadget shows only the
    /// names of its readings: a test's desktop never reads the machine it runs on.
    #[must_use]
    pub fn probe(mut self, probe: Probe) -> Self {
        self.probe = Some(probe);
        self
    }

    /// The folder the notes are kept in, one plain text file each. Without one a note lives only
    /// as long as the desktop runs, which is what a test wants.
    #[must_use]
    pub fn notes_folder(mut self, folder: PathBuf) -> Self {
        self.notes_dir = Some(folder);
        self
    }

    /// What the note kept in `file` holds, as the desktop has it.
    #[must_use]
    pub fn note(&self, file: &str) -> Option<&str> {
        self.notes.get(file).map(String::as_str)
    }

    /// What the system gadget shows now: the last reading whose texts differed from the one
    /// before.
    #[must_use]
    pub fn machine(&self) -> &Status {
        &self.status
    }

    /// The local moment the gadgets show, from the wall clock.
    fn moment(&self) -> DateTime {
        DateTime::from_unix((self.wall)().div_euclid(1_000), self.offset_minutes.unwrap_or(0))
    }

    /// Starts what keeps the gadgets current: the readings, while a system gadget stands on the
    /// floor, and the second tick, while a clock shows its seconds. Each starts only once.
    pub(super) fn gadget_ticks(&mut self) -> Command<Msg> {
        let reading = self.sample(Duration::ZERO);
        let ticking = self.next_second();
        Command::batch([reading, ticking])
    }

    /// Reads the machine after `wait`, when a system gadget wants it and no reading is under way.
    fn sample(&mut self, wait: Duration) -> Command<Msg> {
        let wanted = self.desktop.widgets.iter().any(|gadget| gadget.shows.kind() == Kind::System);
        if !wanted {
            return Command::none();
        }
        let Some(probe) = self.probe.take() else { return Command::none() };
        status::sample_when_changed(probe, self.status.clone(), wait, Msg::Sampled)
    }

    /// Waits for the next second of the wall clock, while a clock shows its seconds and no wait
    /// is under way. Measured from the clock each time, as the minute is, so it never drifts.
    fn next_second(&mut self) -> Command<Msg> {
        let wanted =
            self.desktop.widgets.iter().any(|gadget| matches!(gadget.shows, Shows::Clock(Clock { seconds: true, .. })));
        if !wanted || self.ticking {
            return Command::none();
        }
        self.ticking = true;
        let into_second = u64::try_from((self.wall)().rem_euclid(1_000)).unwrap_or(0);
        let until = Duration::from_millis(1_000 - into_second);
        Command::task(Task::new(
            "second",
            move |cx| {
                if cx.sleep(until) { Ok(Msg::Second) } else { Err("stopped".to_owned()) }
            },
        ))
    }

    /// Reads the note of every note gadget that the desktop has not read yet. A file that is not
    /// there is an empty note; one that cannot be read is said once and left alone, so typing
    /// into the gadget never writes over a file the desktop could not read.
    pub(super) fn read_notes(&mut self) -> Command<Msg> {
        let files: Vec<String> = self
            .desktop
            .widgets
            .iter()
            .filter_map(|gadget| match &gadget.shows {
                Shows::Note(file) if !self.notes.contains_key(file) => Some(file.clone()),
                _ => None,
            })
            .collect();
        let mut said = Vec::new();
        for file in files {
            let text = match &self.notes_dir {
                Some(folder) => match std::fs::read_to_string(folder.join(&file)) {
                    Ok(text) => text,
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
                    Err(error) => {
                        self.unreadable_notes.insert(file.clone());
                        said.push(self.note_problem(&file, &error));
                        continue;
                    }
                },
                None => String::new(),
            };
            self.notes.insert(file, text);
        }
        Command::batch(said)
    }

    /// Tells the person that the note in `file` could not be read or written.
    fn note_problem(&mut self, file: &str, error: &std::io::Error) -> Command<Msg> {
        let heading = t!("widget.note-unsaved");
        let path = self.notes_dir.as_ref().map_or_else(|| PathBuf::from(file), |folder| folder.join(file));
        let body = format!("{}: {error}", path.display());
        self.inbox.add(Notice::desktop(heading.clone(), body.clone()));
        Command::toast(Toast::warning(heading).body(body))
    }

    /// What the gadgets' messages mean.
    pub(super) fn on_gadget(&mut self, msg: Msg) -> Command<Msg> {
        match msg {
            Msg::AddGadget(kind) => self.add_gadget(kind),
            Msg::RemoveGadget(index) => {
                if index >= self.desktop.widgets.len() {
                    return Command::none();
                }
                // The note's file stays: what the person wrote is theirs, and a note gadget added
                // again shows it.
                self.desktop.widgets.remove(index);
                self.save()
            }
            Msg::SetGadget(index, change) => {
                let Some(gadget) = self.desktop.widgets.get_mut(index) else { return Command::none() };
                if !gadget.apply(change) {
                    return Command::none();
                }
                let ticking = self.next_second();
                Command::batch([self.save(), ticking])
            }
            Msg::Sampled(probe, reading) => {
                // Only a reading that would be drawn differently comes at all (`same_on_screen`),
                // so each one is a frame worth sending.
                self.probe = Some(probe);
                self.status = reading;
                self.sample(SAMPLE_EVERY)
            }
            Msg::Second => {
                self.ticking = false;
                self.next_second()
            }
            Msg::NoteTyped(file, text) => self.write_note(&file, &text),
            _ => Command::none(),
        }
    }

    /// Keeps what was typed into the note of `file` and writes it to its file, whole.
    fn write_note(&mut self, file: &str, text: &str) -> Command<Msg> {
        if self.notes.get(file).is_some_and(|kept| kept == text) {
            return Command::none();
        }
        self.notes.insert(file.to_owned(), text.to_owned());
        let Some(folder) = self.notes_dir.clone() else { return Command::none() };
        if self.unreadable_notes.contains(file) || !gadgets::note_name_is_plain(file) {
            return Command::none();
        }
        let written = std::fs::create_dir_all(&folder).and_then(|()| atomic_write(&folder.join(file), text.as_bytes()));
        match written {
            Ok(()) => Command::none(),
            Err(error) => self.note_problem(file, &error),
        }
    }

    /// Puts a gadget of `kind` on the floor where the design says (3.12), or says there is no room.
    fn add_gadget(&mut self, kind: Kind) -> Command<Msg> {
        let file = gadgets::next_note(&self.desktop.widgets);
        let mut gadget = Gadget::new(kind, (0, 0), &file);
        let area = self.windows.area().size();
        let (columns, rows) = (area.width / CELL_WIDTH, area.height / CELL_HEIGHT);
        let wanted: Vec<gadgets::Spot> = self.desktop.widgets.iter().map(Gadget::spot).collect();
        let spots: Vec<gadgets::Spot> = gadgets::arrange(columns, rows, &wanted).into_iter().flatten().collect();
        let blocked: Vec<(u16, u16)> = spots.iter().flat_map(|spot| spot.cells()).collect();
        let places: Vec<Option<(u16, u16)>> =
            self.floor_ids().iter().map(|id| self.desktop.places.get(id).copied()).collect();
        let icons: Vec<(u16, u16)> =
            Grid::placed_around(area, &places, &blocked).cells().iter().copied().flatten().collect();
        let Some(at) = gadgets::spot_for(columns, rows, gadget.size(), &spots, &icons) else {
            return Command::toast(Toast::info(t!("widget.no-room", name = kind.label().as_str())));
        };
        gadget.place = at;
        self.desktop.widgets.push(gadget);
        let read = self.read_notes();
        let ticks = self.gadget_ticks();
        Command::batch([self.save(), read, ticks])
    }

    /// The rows of the floor's menu that add and remove gadgets.
    pub(super) fn gadget_floor_items(&self) -> Vec<ContextItem<Msg>> {
        let adding = Kind::ALL.into_iter().map(|kind| ContextItem::new(kind.label(), Msg::AddGadget(kind)));
        let mut items = vec![ContextItem::submenu(t!("widget.add"), adding)];
        if !self.desktop.widgets.is_empty() {
            let removing = self
                .desktop
                .widgets
                .iter()
                .enumerate()
                .map(|(index, gadget)| ContextItem::new(gadget.shows.kind().label(), Msg::RemoveGadget(index)));
            items.push(ContextItem::submenu(t!("widget.remove-one"), removing));
        }
        items
    }

    /// The rows of the menu of the gadget at `index`: its options, the chosen ones marked, and
    /// Remove.
    fn gadget_items(index: usize, gadget: &Gadget) -> Vec<ContextItem<Msg>> {
        let marked = |on: bool, item: ContextItem<Msg>| if on { item.icon("check") } else { item };
        let set = |change: Change| Msg::SetGadget(index, change);
        let mut items = match &gadget.shows {
            Shows::Clock(clock) => vec![
                marked(!clock.twelve, ContextItem::new(t!("widget.hours-24"), set(Change::Twelve(false)))),
                marked(clock.twelve, ContextItem::new(t!("widget.hours-12"), set(Change::Twelve(true)))),
                marked(clock.seconds, ContextItem::new(t!("widget.seconds"), set(Change::Seconds(!clock.seconds)))),
                ContextItem::submenu(
                    t!("widget.color"),
                    Tint::ALL.into_iter().map(|tint| {
                        let label = t!(&format!("widget.color-{}", tint.name()));
                        marked(clock.tint == tint, ContextItem::new(label, set(Change::Tint(tint))))
                    }),
                ),
            ],
            Shows::Calendar(start) => vec![ContextItem::submenu(
                t!("widget.week-start"),
                WeekStart::ALL.into_iter().map(|day| {
                    let label = t!(&format!("widget.week-start-{}", day.name()));
                    marked(*start == day, ContextItem::new(label, set(Change::WeekStart(day))))
                }),
            )],
            Shows::System(readings) => Reading::ALL
                .into_iter()
                .map(|reading| {
                    let shown = readings.shows(reading);
                    let item = ContextItem::new(reading.label(), set(Change::Reading(reading, !shown)));
                    // The last reading cannot be taken away: the gadget would be an empty surface.
                    marked(shown, item.disabled(shown && readings.shown().len() == 1))
                })
                .collect(),
            Shows::Note(_) => Vec::new(),
        };
        if !items.is_empty() {
            items.push(ContextItem::gap());
        }
        items.push(ContextItem::new(t!("widget.remove"), Msg::RemoveGadget(index)));
        items
    }

    /// The nodes of the gadgets, one each in their order, each in its own menu: what the floor
    /// draws after the icons' cells.
    pub(super) fn gadget_nodes(&self, ui: &mut View<'_, Msg>) {
        let now = self.moment();
        let first = ui.env().i18n().first_weekday();
        let ascii = ui.env().glyph_mode() == qframe::icons::GlyphMode::Ascii;
        for (index, gadget) in self.desktop.widgets.iter().enumerate() {
            let menu = ContextMenu::new(Self::gadget_items(index, gadget));
            ui.add_with(menu, |ui| match &gadget.shows {
                Shows::Clock(clock) => clock_face(*clock, now, ascii, ui),
                Shows::Calendar(start) => {
                    ui.add_with(Surface::new(0), |ui| {
                        ui.add(Month::new(now.date, start.day(first)));
                    });
                }
                Shows::System(readings) => self.system_face(readings, ui),
                Shows::Note(file) => self.note_face(file, ui),
            });
        }
    }

    /// The system gadget: a meter for each share, the network rate as a number.
    fn system_face(&self, readings: &Readings, ui: &mut View<'_, Msg>) {
        let items = status::items(&self.status);
        let text_of = |kind: status::Kind| {
            items.iter().find(|item| item.kind == kind).map(|item| item.text.clone()).unwrap_or_default()
        };
        let width = readings.shown().iter().map(|reading| qframe::text::width(&reading.label())).max().unwrap_or(0);
        #[expect(clippy::cast_possible_truncation, reason = "a share on screen needs no more than f32 keeps")]
        let share = |value: f64| value as f32;
        let status = &self.status;
        ui.add_with(Surface::new(0), |ui| {
            for reading in readings.shown() {
                let label = reading.label();
                let meter = |value: f32, kind: status::Kind| {
                    Gauge::new(value).label(label.clone()).label_width(width).value_text(text_of(kind))
                };
                match reading {
                    Reading::Cpu => match status.cpu {
                        Some(cpu) => {
                            ui.add(meter(share(cpu), status::Kind::Cpu).thresholds(WARN, DANGER));
                        }
                        None => {
                            ui.add(Line::new(label, width, ""));
                        }
                    },
                    Reading::Memory => match status.memory {
                        Some(memory) => {
                            ui.add(meter(share(memory.share()), status::Kind::Memory).thresholds(WARN, DANGER));
                        }
                        None => {
                            ui.add(Line::new(label, width, ""));
                        }
                    },
                    // A machine without a battery, which most servers are, has no row for it.
                    Reading::Battery => {
                        if let Some(battery) = status.battery {
                            ui.add(meter(f32::from(battery.percent), status::Kind::Battery));
                        }
                    }
                    Reading::Network => {
                        ui.add(Line::new(label, width, text_of(status::Kind::Network)));
                    }
                }
            }
        });
    }

    /// The note gadget: the framework's text area over the whole surface inside its padding.
    fn note_face(&self, file: &str, ui: &mut View<'_, Msg>) {
        let text = self.notes.get(file).cloned().unwrap_or_default();
        let kept = file.to_owned();
        ui.add_with(Surface::filled(), |ui| {
            let area = TextArea::new(text)
                .placeholder(t!("widget.note-placeholder"))
                .on_change(move |text| Msg::NoteTyped(kept.clone(), text));
            ui.add(area).id(note_field(file)).fill();
        });
    }

    /// The gadgets on the floor, in their order.
    #[must_use]
    pub fn gadgets(&self) -> &[Gadget] {
        &self.desktop.widgets
    }
}

/// The clock: the time in large digits, the morning or afternoon mark beside them on a twelve-hour
/// clock, and the date under them.
///
/// ASCII draws the digits five rows tall instead of three, so there the date stands right under
/// them rather than a row apart: the clock keeps its size in every glyph mode.
fn clock_face(clock: Clock, now: DateTime, ascii: bool, ui: &mut View<'_, Msg>) {
    let time = now.time;
    let (hour, mark) = if clock.twelve {
        let hour = match time.hour % 12 {
            0 => 12,
            hour => hour,
        };
        (format!("{hour}"), Some(if time.hour < 12 { t!("widget.am") } else { t!("widget.pm") }))
    } else {
        (format!("{:02}", time.hour), None)
    };
    let digits = if clock.seconds {
        format!("{hour}:{:02}:{:02}", time.minute, time.second)
    } else {
        format!("{hour}:{:02}", time.minute)
    };
    let big = match clock.tint {
        Tint::Accent => BigText::new(digits).variant("accent"),
        Tint::Text => BigText::new(digits),
        Tint::Gradient => BigText::new(digits).variant("accent").gradient("accent-2", Gradient::Rows),
    };
    let weekday = t!(&format!("quvyta.date.weekday-long-{}", now.date.weekday().number()));
    let date = t!("widget.clock-date", weekday = weekday.as_str(), date = now.date.day_and_month().as_str());
    ui.add_with(Surface::new(u16::from(!ascii)), |ui| {
        ui.row(|ui| {
            ui.add(big);
            if let Some(mark) = mark {
                ui.add(Text::new(mark).role("secondary").no_wrap());
            }
        })
        .gap(1);
        ui.add(Text::new(date).role("secondary").no_wrap());
    });
}
