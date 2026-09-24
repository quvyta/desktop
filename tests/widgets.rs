//! The widgets on the floor (design 3.12): added from the floor's menu, carried by the mouse,
//! changed and removed from their own menus, and kept in the desktop file, at several sizes,
//! glyph modes and colour depths. The time is the test's own clock, and the machine the system
//! widget reads is a folder the test wrote: nothing here reads the machine it runs on.

mod support;

use std::cell::Cell as Shared;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use qdesk::app::{Desk, note_field};
use qdesk::desktop::Desktop;
use qdesk::gadgets::{Clock, Gadget, Kind, Readings, Shows, WeekStart};
use qdesk::status::Probe;
use qframe::color::ColorDepth;
use qframe::event::{MouseButton, MouseKind};
use qframe::icons::GlyphMode;
use qframe::prelude::*;
use support::{ICONS, MOMENT, base, decoration, draw, screen};

/// A clock the test moves: milliseconds since 1970, shared with the desktop reading it.
struct Wall(Rc<Shared<i64>>);

impl Wall {
    fn new() -> (Self, qdesk::app::WallClock) {
        let now = Rc::new(Shared::new(MOMENT * 1_000));
        let read = Rc::clone(&now);
        (Self(now), Box::new(move || read.get()))
    }

    /// Moves the wall clock and the harness's own clock on together by `by`.
    fn pass(&self, harness: &mut Harness<Desk>, by: Duration) {
        self.0.set(self.0.get() + i64::try_from(by.as_millis()).unwrap_or(0));
        harness.advance(by);
    }
}

/// A folder of its own under the system's temporary folder, removed when dropped: a `/proc`, a
/// power supply folder, the notes and the desktop file of one test.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let number = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-widgets-{}-{name}-{number}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch folder");
        Self(path)
    }

    fn write(&self, relative: &str, text: &str) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("a folder");
        std::fs::write(&path, text).expect("a file");
    }

    /// `/proc/stat` whose processors have been busy `busy` ticks and idle `idle` ticks.
    fn stat(&self, busy: u64, idle: u64) {
        self.write("proc/stat", &format!("cpu  {busy} 0 0 {idle} 0 0 0 0 0 0\ncpu0 1 1 1 1 0 0 0 0 0 0\n"));
    }

    /// `/proc/meminfo` with `used` of 100 units in use.
    fn memory(&self, used: u64) {
        let total = 8_000_000;
        let available = total - total * used / 100;
        self.write("proc/meminfo", &format!("MemTotal: {total} kB\nMemFree: 1 kB\nMemAvailable: {available} kB\n"));
    }

    fn battery(&self, percent: u8) {
        self.write("power/BAT0/type", "Battery\n");
        self.write("power/BAT0/capacity", &format!("{percent}\n"));
        self.write("power/BAT0/status", "Discharging\n");
    }

    fn probe(&self) -> Probe {
        Probe::rooted(self.0.join("proc"), self.0.join("power"), None)
    }

    fn notes(&self) -> PathBuf {
        self.0.join("notes")
    }

    fn file(&self) -> PathBuf {
        self.0.join("desktop.toml")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The usual desktop with `widgets` on its floor.
fn desktop(widgets: Vec<Gadget>) -> Desktop {
    Desktop {
        icons: ICONS.iter().map(|id| (*id).to_owned()).collect(),
        welcome_seen: true,
        resize_hint_seen: true,
        widgets,
        ..Desktop::default()
    }
}

/// A desktop with `widgets` on a terminal of `width` by `height`, and the clock that drives it.
fn desk(widgets: Vec<Gadget>, width: u16, height: u16) -> (Harness<Desk>, Wall) {
    let (wall, clock) = Wall::new();
    (draw(base(clock).desktop(desktop(widgets)), width, height), wall)
}

fn clock_at(place: (u16, u16)) -> Gadget {
    Gadget::new(Kind::Clock, place, "")
}

/// Opens the floor's menu on empty floor at `at` and adds the widget called `name`.
fn add(harness: &mut Harness<Desk>, at: (i32, i32), name: &str) {
    harness.mouse(MouseKind::Down(MouseButton::Right), at.0, at.1);
    harness.click_text("Add a widget");
    let (x, y) = harness.find(name).unwrap_or_else(|| panic!("{name} is offered:\n{}", harness.screen()));
    harness.click(x, y);
}

/// Opens the menu of the widget under `at` and chooses the row called `row`.
fn choose(harness: &mut Harness<Desk>, at: (i32, i32), row: &str) {
    harness.mouse(MouseKind::Down(MouseButton::Right), at.0, at.1);
    let (x, y) = harness.find(row).unwrap_or_else(|| panic!("{row} is in the menu:\n{}", harness.screen()));
    harness.click(x, y);
}

/// The first cell of the rectangle `x`, `y`, `width` by `height` holding a block of the large
/// digits, and its colour.
fn digit(harness: &Harness<Desk>, (x, y, width, height): (u16, u16, u16, u16)) -> Option<(u16, u16)> {
    (y..y + height)
        .flat_map(|row| (x..x + width).map(move |column| (column, row)))
        .find(|(column, row)| matches!(harness.buffer()[(*column, *row)].symbol(), "█" | "▀" | "▄"))
}

/// The rows `from` to `to` of the screen.
fn rows(harness: &Harness<Desk>, from: usize, to: usize) -> Vec<String> {
    screen(harness)[from..to].to_vec()
}

/// A clock added at 120 by 40 stands in the top right corner: its surface at column 90, the
/// digits in its middle and the date under them.
const DATE: &str = "Saturday, September 19";
/// The calendar's weekdays in Turkish, where the week starts on Monday.
const TURKISH_WEEK: &str = "Pt Sa Ça Pe Cu Ct Pz";

#[test]
fn the_floors_menu_adds_a_clock_in_the_top_right_corner() {
    let (mut harness, _wall) = desk(Vec::new(), 120, 40);
    add(&mut harness, (60, 20), "Clock");
    let gadgets = harness.app().gadgets();
    assert_eq!(gadgets.len(), 1);
    assert_eq!(gadgets[0].shows, Shows::Clock(Clock::default()));
    assert_eq!(gadgets[0].place, (9, 0), "the last column of cells, at the top");
    let theme = harness.env().theme().clone();
    assert_eq!(harness.bg(90, 0), theme.color("surface"), "the widget is a surface one tone up");
    assert_eq!(harness.bg(118, 7), theme.color("surface"));
    assert_eq!(harness.bg(119, 0), theme.color("canvas"), "its last column is the floor's");
    assert_eq!(harness.bg(90, 8), theme.color("canvas"), "and so is its last row");
    let (x, y) = harness.find(DATE).unwrap_or_else(|| panic!("the date is under the digits:\n{}", harness.screen()));
    assert!((90..119).contains(&x) && y == 5, "the date stands in the widget: {x}, {y}");
    assert!(digit(&harness, (90, 0, 29, 5)).is_some(), "the digits are drawn large:\n{}", harness.screen());
    // The icons keep their column on the left.
    assert_eq!(harness.find("Terminal"), Some((1, 1)));
}

#[test]
fn at_80_by_24_the_clock_takes_the_right_edge_and_the_icons_keep_the_left() {
    let (mut harness, _wall) = desk(Vec::new(), 80, 24);
    add(&mut harness, (40, 18), "Clock");
    assert_eq!(harness.app().gadgets()[0].place, (5, 0));
    assert!(harness.find(DATE).is_some_and(|(x, _)| x >= 50), "{}", harness.screen());
    assert_eq!(harness.find("Terminal"), Some((1, 1)));
}

#[test]
fn a_new_widget_goes_where_nothing_stands_and_the_next_one_beside_it() {
    let (mut harness, _wall) = desk(Vec::new(), 120, 40);
    add(&mut harness, (60, 30), "Clock");
    add(&mut harness, (60, 30), "Calendar");
    add(&mut harness, (60, 30), "System");
    add(&mut harness, (60, 30), "Note");
    let places: Vec<(u16, u16)> = harness.app().gadgets().iter().map(|gadget| gadget.place).collect();
    assert_eq!(places, [(9, 0), (9, 3), (9, 7), (9, 10)], "one column down the right edge");
    assert!(harness.find("September 2026").is_some(), "the calendar names its month");
    assert!(harness.find("Processor").is_some(), "the system widget names its readings");
    assert!(harness.find("Write something").is_some(), "an empty note says what it is for");
    // A fifth has no room left in that column and goes to the next one.
    add(&mut harness, (40, 30), "Clock");
    assert_eq!(harness.app().gadgets()[4].place, (6, 0));
}

#[test]
fn the_clock_is_drawn_again_when_the_minute_changes() {
    let (mut harness, wall) = desk(vec![clock_at((9, 0))], 120, 40);
    let before = rows(&harness, 0, 8);
    // Half a minute in, the digits are the same: the time shown is hours and minutes.
    wall.pass(&mut harness, Duration::from_secs(5));
    assert_eq!(rows(&harness, 0, 8), before, "seconds are not shown");
    wall.pass(&mut harness, Duration::from_secs(60));
    assert_ne!(rows(&harness, 0, 8), before, "a new minute is a new time");
    assert!(screen(&harness).last().is_some_and(|dock| dock.contains("14:33")), "the dock agrees");
}

#[test]
fn twelve_hours_and_seconds_come_from_the_clocks_own_menu() {
    let (mut harness, wall) = desk(vec![clock_at((9, 0))], 120, 40);
    assert!(harness.find("PM").is_none(), "a 24-hour clock has no mark");
    choose(&mut harness, (100, 6), "12-hour clock");
    assert!(matches!(harness.app().gadgets()[0].shows, Shows::Clock(Clock { twelve: true, .. })), "the choice is kept");
    assert!(harness.find("PM").is_some(), "half past two in the afternoon:\n{}", harness.screen());
    choose(&mut harness, (100, 6), "Show seconds");
    let gadget = &harness.app().gadgets()[0];
    assert!(matches!(gadget.shows, Shows::Clock(Clock { seconds: true, .. })));
    assert_eq!(gadget.size(), (4, 3), "the seconds make the clock a cell wider");
    // The widget would leave the floor at its place, so it stands a cell to the left.
    assert_eq!(harness.bg(80, 0), harness.env().theme().color("surface"), "{}", harness.screen());
    let before = rows(&harness, 0, 8);
    wall.pass(&mut harness, Duration::from_secs(1));
    assert_ne!(rows(&harness, 0, 8), before, "the seconds move");
}

#[test]
fn the_clock_takes_the_accent_or_the_plain_text_colour() {
    let (mut harness, _wall) = desk(vec![clock_at((9, 0))], 120, 40);
    // A theme whose accent is not its text colour, so the two can be told apart.
    harness.set_theme("amber");
    let (accent, text) = (harness.env().theme().color("accent"), harness.env().theme().color("text"));
    assert_ne!(accent, text);
    let (x, y) = digit(&harness, (90, 0, 29, 8)).expect("a digit");
    assert_eq!(harness.fg(x, y), accent, "the accent by default");
    harness.mouse(MouseKind::Down(MouseButton::Right), 100, 6);
    harness.click_text("Colour");
    harness.click_text("Plain");
    let (x, y) = digit(&harness, (90, 0, 29, 8)).expect("a digit");
    assert_eq!(harness.fg(x, y), text, "plain is the text colour");
}

#[test]
fn a_widget_is_dragged_to_a_lit_landing_place_and_stays_there() {
    let (mut harness, _wall) = desk(vec![clock_at((9, 0))], 120, 40);
    harness.set_theme("amber");
    let theme = harness.env().theme().clone();
    // Held by the cell of its second column and third row, and carried to the cell at 40, 20.
    harness.mouse(MouseKind::Down(MouseButton::Left), 100, 6);
    harness.mouse(MouseKind::Drag(MouseButton::Left), 40, 20);
    let lit = theme.color("canvas").zip(theme.color("accent")).map(|(canvas, accent)| canvas.mix(accent, 0.20));
    assert_eq!(harness.bg(31, 13), lit, "the landing place is lit:\n{}", harness.screen());
    assert_eq!(harness.bg(58, 19), lit, "all of it");
    assert_eq!(harness.bg(90, 0), theme.color("surface"), "the widget stays until it is dropped");
    harness.mouse(MouseKind::Up(MouseButton::Left), 40, 20);
    assert_eq!(harness.app().gadgets()[0].place, (3, 4));
    assert_eq!(harness.bg(31, 13), theme.color("surface"), "it stands there now");
    assert_eq!(harness.bg(90, 0), theme.color("canvas"), "and its old place is floor");
}

#[test]
fn a_widget_dragged_past_the_edge_stops_at_it() {
    let (mut harness, _wall) = desk(vec![clock_at((5, 2))], 120, 40);
    harness.drag((60, 8), (119, 38));
    assert_eq!(harness.app().gadgets()[0].place, (9, 10), "the last spot it fits in");
}

#[test]
fn a_widget_is_never_dropped_over_another() {
    let calendar = Gadget::new(Kind::Calendar, (9, 3), "");
    let (mut harness, _wall) = desk(vec![clock_at((9, 0)), calendar], 120, 40);
    harness.mouse(MouseKind::Down(MouseButton::Left), 100, 6);
    harness.mouse(MouseKind::Drag(MouseButton::Left), 100, 14);
    let canvas = harness.env().theme().color("canvas");
    assert_eq!(harness.bg(119, 14), canvas, "nothing is lit where it cannot go");
    harness.mouse(MouseKind::Up(MouseButton::Left), 100, 14);
    assert_eq!(harness.app().gadgets()[0].place, (9, 0), "the drop is refused");
}

#[test]
fn the_icons_flow_around_a_widget_and_come_back_when_it_leaves() {
    let (mut harness, _wall) = desk(vec![clock_at((9, 0))], 120, 40);
    assert_eq!(harness.find("Terminal"), Some((1, 1)));
    // The clock is carried to the top left corner, where Terminal and Settings stand.
    harness.drag((95, 6), (5, 6));
    assert_eq!(harness.app().gadgets()[0].place, (0, 0), "{}", harness.screen());
    let (x, y) = harness.find("Terminal").expect("Terminal is still on the floor");
    assert!(x >= 30 || y >= 9, "Terminal makes way for the clock: {x}, {y}\n{}", harness.screen());
    assert!(harness.app().order().places.is_empty(), "no icon was pinned by it");
    harness.drag((5, 6), (95, 6));
    assert_eq!(harness.find("Terminal"), Some((1, 1)), "the icons flow back");
}

#[test]
fn an_icon_is_never_dropped_on_a_widget() {
    let (mut harness, _wall) = desk(vec![clock_at((9, 0))], 120, 40);
    harness.mouse(MouseKind::Down(MouseButton::Left), 3, 1);
    harness.mouse(MouseKind::Drag(MouseButton::Left), 95, 4);
    assert_eq!(harness.bg(119, 4), harness.env().theme().color("canvas"), "no landing is lit");
    harness.mouse(MouseKind::Up(MouseButton::Left), 95, 4);
    assert_eq!(harness.find("Terminal"), Some((1, 1)), "the icon stays");
    assert!(harness.app().order().places.is_empty());
    // Nor moved onto it with the keys.
    let (mut harness, _wall) = desk(vec![clock_at((1, 0))], 120, 40);
    harness.click(3, 1).press("shift+right");
    assert_eq!(harness.find("Terminal"), Some((1, 1)), "shift and an arrow stop at the widget");
    assert!(harness.app().order().places.is_empty(), "and nothing is pinned");
}

#[test]
fn remove_on_a_widgets_menu_takes_it_off_the_floor_and_out_of_the_file() {
    let scratch = Scratch::new("remove");
    let (wall, clock) = Wall::new();
    let app = base(clock).desktop(desktop(vec![clock_at((9, 0))])).config(Some(scratch.file()));
    let mut harness = draw(app, 120, 40);
    let _ = wall;
    choose(&mut harness, (100, 6), "Remove widget");
    assert!(harness.app().gadgets().is_empty());
    assert!(harness.find(DATE).is_none());
    let written = std::fs::read_to_string(scratch.file()).expect("the file is written");
    assert!(!written.contains("widgets"), "{written}");
}

#[test]
fn the_floors_menu_removes_a_widget_too() {
    let (mut harness, _wall) = desk(vec![clock_at((9, 0))], 120, 40);
    harness.mouse(MouseKind::Down(MouseButton::Right), 60, 20);
    harness.click_text("Remove a widget");
    let (x, y) = harness.find("Clock").expect("the clock is listed");
    harness.click(x, y);
    assert!(harness.app().gadgets().is_empty());
}

#[test]
fn widgets_their_places_and_their_options_are_read_back_from_the_file() {
    let scratch = Scratch::new("file");
    let (_wall, clock) = Wall::new();
    let app = base(clock).desktop(desktop(Vec::new())).config(Some(scratch.file()));
    let mut harness = draw(app, 120, 40);
    add(&mut harness, (60, 30), "Clock");
    choose(&mut harness, (100, 6), "12-hour clock");
    harness.drag((100, 6), (40, 20));
    let written = std::fs::read_to_string(scratch.file()).expect("the file is written");
    assert!(written.contains("[[widgets]]\nkind = \"clock\"\nplace = [3, 4]\nhours = 12\n"), "{written}");
    let (read, problems) = Desktop::load(&scratch.file());
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(read.widgets, harness.app().gadgets());
}

#[test]
fn the_calendar_shows_this_month_with_today_in_the_accent() {
    let (mut harness, _wall) = desk(vec![Gadget::new(Kind::Calendar, (9, 0), "")], 120, 40);
    harness.set_theme("amber");
    assert!(harness.find("September 2026").is_some(), "{}", harness.screen());
    // The weeks start on the language's own first day until the person picks one: Sunday in
    // English, whatever the machine running the test is set to.
    assert!(harness.find("Su Mo Tu We Th Fr Sa").is_some(), "{}", harness.screen());
    let (x, y) = harness.find("19").expect("today is on it");
    let theme = harness.env().theme().clone();
    let (x, y) = (u16::try_from(x).unwrap_or(0), u16::try_from(y).unwrap_or(0));
    assert_ne!(theme.color("accent"), theme.color("text"));
    assert_eq!(harness.fg(x, y), theme.color("accent"), "today is the accent");
    assert!(harness.is_bold(x, y), "and bold, so it reads without the colour");
    assert_ne!(harness.bg(x, y), theme.color("surface"), "on a raised cell");
    harness.mouse(MouseKind::Down(MouseButton::Right), 100, 1);
    harness.click_text("Week starts on");
    harness.click_text("Sunday");
    assert_eq!(harness.app().gadgets()[0].shows, Shows::Calendar(WeekStart::Sunday));
    assert!(harness.find("Su Mo Tu We Th Fr Sa").is_some(), "{}", harness.screen());
    harness.mouse(MouseKind::Down(MouseButton::Right), 100, 1);
    harness.click_text("Week starts on");
    harness.click_text("Monday");
    assert_eq!(harness.app().gadgets()[0].shows, Shows::Calendar(WeekStart::Monday));
    assert!(harness.find("Mo Tu We Th Fr Sa Su").is_some(), "{}", harness.screen());
    // In Turkish they start on Monday.
    let (mut harness, _wall) = desk(vec![Gadget::new(Kind::Calendar, (9, 0), "")], 120, 40);
    harness.set_locale("tr");
    assert!(harness.find("Eylül 2026").is_some(), "{}", harness.screen());
    assert!(harness.find(TURKISH_WEEK).is_some(), "{}", harness.screen());
    // The day after the month turns, the calendar shows the next one.
    let (mut harness, wall) = desk(vec![Gadget::new(Kind::Calendar, (9, 0), "")], 120, 40);
    wall.pass(&mut harness, Duration::from_secs(12 * 24 * 3600));
    assert!(harness.find("October 2026").is_some(), "{}", harness.screen());
}

/// A desktop whose system widget reads the machine in `scratch`.
fn reading(scratch: &Scratch, readings: Option<Readings>) -> (Harness<Desk>, Wall) {
    let mut gadget = Gadget::new(Kind::System, (9, 0), "");
    if let Some(readings) = readings {
        gadget.shows = Shows::System(readings);
    }
    let (wall, clock) = Wall::new();
    let app = base(clock).desktop(desktop(vec![gadget])).probe(scratch.probe());
    (draw(app, 120, 40), wall)
}

#[test]
fn the_system_widget_shows_the_processor_the_memory_and_the_battery_with_their_tones() {
    let scratch = Scratch::new("system");
    scratch.stat(100, 900);
    scratch.memory(40);
    scratch.battery(87);
    let (mut harness, wall) = reading(&scratch, None);
    // The processor's share needs two readings, so at first only its name is there.
    assert!(harness.find("Processor").is_some());
    assert!(harness.find("40%").is_some(), "the memory is known at once:\n{}", harness.screen());
    assert!(harness.find("87%").is_some(), "and the battery:\n{}", harness.screen());
    // Between the two readings the processor was busy 200 ticks of 400.
    scratch.stat(300, 1100);
    scratch.memory(95);
    wall.pass(&mut harness, qdesk::status::SAMPLE_EVERY);
    assert!(harness.find("50%").is_some(), "the processor's share:\n{}", harness.screen());
    let (x, y) = harness.find("95%").expect("the memory");
    let glyph = harness.env().icons().glyph("error").into_owned();
    let row = &screen(&harness)[usize::try_from(y).unwrap_or(0)];
    assert!(row.contains(&glyph), "a full memory carries the danger mark, not only its colour: {row:?}");
    let _ = x;
    // A machine without a battery has no row for it.
    let bare = Scratch::new("bare");
    bare.stat(1, 1);
    bare.memory(10);
    let (harness, _wall) = reading(&bare, None);
    assert!(harness.find("Battery").is_none(), "{}", harness.screen());
}

#[test]
fn a_reading_that_shows_the_same_texts_is_not_taken_so_nothing_is_drawn_again() {
    let scratch = Scratch::new("same");
    scratch.stat(0, 0);
    scratch.memory(40);
    let (mut harness, wall) = reading(&scratch, None);
    scratch.stat(1_000, 1_000);
    wall.pass(&mut harness, qdesk::status::SAMPLE_EVERY);
    assert_eq!(harness.app().machine().cpu, Some(50.0));
    // A share of 50.2 is written 50% as well: the reading is not taken.
    scratch.stat(1_000 + 251, 1_000 + 249);
    wall.pass(&mut harness, qdesk::status::SAMPLE_EVERY);
    assert_eq!(harness.app().machine().cpu, Some(50.0), "the same texts, the same reading");
    // One that reads differently is.
    scratch.stat(1_251 + 900, 1_249 + 100);
    wall.pass(&mut harness, qdesk::status::SAMPLE_EVERY);
    assert_eq!(harness.app().machine().cpu, Some(90.0));
}

#[test]
fn the_system_widgets_menu_hides_a_reading_but_never_the_last() {
    let scratch = Scratch::new("readings");
    scratch.stat(1, 1);
    scratch.memory(10);
    let (mut harness, _wall) = reading(&scratch, None);
    choose(&mut harness, (100, 1), "Processor");
    // Not even cut short: the row is gone.
    assert!(harness.find("Proces").is_none(), "{}", harness.screen());
    let Shows::System(readings) = &harness.app().gadgets()[0].shows else { panic!("a system widget") };
    assert!(!readings.shows(qdesk::gadgets::Reading::Cpu));
    let mut only = Readings::default();
    for reading in [qdesk::gadgets::Reading::Cpu, qdesk::gadgets::Reading::Battery, qdesk::gadgets::Reading::Network] {
        only.set(reading, false);
    }
    let (mut harness, _wall) = reading(&scratch, Some(only));
    choose(&mut harness, (100, 2), "Memory");
    assert!(harness.find("Memory").is_some(), "the last reading stays:\n{}", harness.screen());
}

#[test]
fn a_note_is_written_by_clicking_into_it_and_typing_and_kept_in_its_file() {
    let scratch = Scratch::new("note");
    let (_wall, clock) = Wall::new();
    let note = Gadget::new(Kind::Note, (9, 0), "notes.txt");
    let app = base(clock).desktop(desktop(vec![note.clone()])).notes_folder(scratch.notes());
    let mut harness = draw(app, 120, 40);
    let (x, y) = harness.find("Write something").expect("an empty note");
    harness.click(x, y);
    assert!(harness.is_focused(&note_field("notes.txt")), "the keys are in the note");
    harness.type_text("milk");
    assert_eq!(harness.app().note("notes.txt"), Some("milk"));
    let file = scratch.notes().join("notes.txt");
    assert_eq!(std::fs::read_to_string(&file).expect("the note's file"), "milk");
    // Removing the widget keeps what was written; a desktop opened again shows it.
    choose(&mut harness, (91, 7), "Remove widget");
    assert!(file.exists(), "the person's note is theirs");
    let (_wall, clock) = Wall::new();
    let app = base(clock).desktop(desktop(vec![note])).notes_folder(scratch.notes());
    let harness = draw(app, 120, 40);
    assert!(harness.find("milk").is_some(), "{}", harness.screen());
}

#[test]
fn a_note_is_written_on_the_widgets_own_surface_like_paper() {
    let scratch = Scratch::new("paper");
    let (_wall, clock) = Wall::new();
    let note = Gadget::new(Kind::Note, (9, 0), "notes.txt");
    let app = base(clock).desktop(desktop(vec![note])).notes_folder(scratch.notes());
    let mut harness = draw(app, 120, 40);
    harness.set_theme("amber");
    let theme = harness.env().theme().clone();
    let tone = |widget: &str| theme.style(widget, None, &[]).paint("bg").map(|paint| paint.at(0.0));
    // The widget's surface is the panel's tone, as every widget's is.
    let surface = tone("panel").or_else(|| theme.color("surface"));
    let field = tone("text-area");
    assert!(field.is_some() && field != surface, "the theme raises a text area: {field:?} on {surface:?}");
    let (x, y) = harness.find("Write something").expect("an empty note");
    let (column, row) = (u16::try_from(x).unwrap_or(0), u16::try_from(y).unwrap_or(0));
    // The placeholder's row and the empty row under it are the widget's own tone.
    assert_eq!(harness.bg(column, row), surface, "the note's text stands on the widget:\n{}", harness.screen());
    assert_eq!(harness.bg(column, row + 1), surface, "and so does its empty room");
    harness.click(x, y);
    assert_eq!(harness.bg(column + 2, row + 1), surface, "being written in, it is still paper");
}

#[test]
fn a_narrow_screen_hides_the_widgets_with_the_icons() {
    let (harness, _wall) = desk(vec![clock_at((9, 0))], 59, 16);
    assert!(harness.find("Saturday").is_none(), "{}", harness.screen());
    assert!(harness.find("Terminal").is_none());
    assert!(harness.find("Applications").is_some(), "the narrow floor's line is there");
    // At 60 by 16 the floor is back, and the clock, whose place is off this screen, stands in the
    // nearest spot it fits.
    let (harness, _wall) = desk(vec![clock_at((9, 0))], 60, 16);
    assert!(harness.find("Saturday").is_some(), "{}", harness.screen());
    assert_eq!(harness.find("Terminal"), Some((1, 1)));
}

#[test]
fn every_widget_is_drawn_without_brackets_or_lines_in_ascii_and_sixteen_colours() {
    for (mode, depth) in [
        (GlyphMode::Ascii, ColorDepth::Ansi16),
        (GlyphMode::Unicode, ColorDepth::Ansi16),
        (GlyphMode::Ascii, ColorDepth::TrueColor),
    ] {
        let scratch = Scratch::new("modes");
        scratch.stat(1, 1);
        scratch.memory(50);
        let widgets = vec![
            clock_at((5, 0)),
            Gadget::new(Kind::Calendar, (2, 0), ""),
            Gadget::new(Kind::System, (5, 3), ""),
            Gadget::new(Kind::Note, (2, 4), "notes.txt"),
        ];
        let (_wall, clock) = Wall::new();
        let app = base(clock).desktop(desktop(widgets)).probe(scratch.probe());
        let mut harness = draw(app, 80, 24);
        harness.set_glyph_mode(mode).set_depth(depth);
        let shown = harness.screen();
        assert_eq!(decoration(&shown), None, "{mode:?} {depth:?}:\n{shown}");
        assert!(shown.contains("Saturday"), "the date fits under the digits in {mode:?}:\n{shown}");
        assert!(shown.contains("September 2026") && shown.contains("Memory") && shown.contains("Write"));
    }
}

#[test]
fn widgets_stand_on_the_floor_of_every_workspace() {
    let (mut harness, _wall) = desk(vec![clock_at((9, 0))], 120, 40);
    // The dock's third workspace mark, two cells after the second.
    let (x, y) = harness.find("1 ○ ○ ○").expect("the workspace marks are on the dock");
    harness.click(x + 4, y);
    assert!(harness.find("1 ○ ○ ○").is_none(), "another workspace is on screen");
    assert!(harness.find(DATE).is_some(), "{}", harness.screen());
}

/// What a terminal would be sent to go from `before` to `after`: the same writer the running
/// desktop draws with, over only the cells that changed. The harness draws a frame at every step;
/// the running desktop draws one only when a message or an input came, so a frame that changes no
/// cell is one it would not have sent at all.
fn sent(before: &ratatui_core::buffer::Buffer, after: &ratatui_core::buffer::Buffer) -> usize {
    use ratatui_core::backend::Backend;
    let changed = before.diff(after);
    if changed.is_empty() {
        // The running desktop draws a frame only when something happened; this one it never draws.
        return 0;
    }
    let mut bytes = Vec::new();
    ratatui_crossterm::CrosstermBackend::new(&mut bytes).draw(changed.into_iter()).expect("drawn into memory");
    bytes.len()
}

/// What every frame the running desktop draws costs on top of the cells it changes: the
/// synchronized update around it (`ESC [ ? 2026 h` and `l`) and hiding the cursor.
const FRAMING: usize = 8 + 8 + 6;

/// What an idle desktop with a clock and the system widget sends a terminal in a minute. The
/// system widget is read over a `/proc` the test writes: on a busy machine the processor's share
/// changes at every reading, the worst case; on a quiet one it stays the same. The desktop draws a
/// frame only for a message that changes the screen — the minute, a second, a reading that looks
/// different — so each step that changed cells is one frame. Run it on its own:
///
/// ```text
/// cargo test --test widgets -- --ignored --nocapture
/// ```
#[test]
#[ignore = "a measurement, not a check: it prints what a minute costs"]
fn an_idle_clock_and_system_widget_cost_this_many_bytes_a_minute() {
    let system = || Gadget::new(Kind::System, (9, 3), "");
    let seconds = Gadget { shows: Shows::Clock(Clock { seconds: true, ..Clock::default() }), place: (8, 0) };
    for (name, widgets, busy) in [
        ("clock", vec![clock_at((9, 0))], false),
        ("clock and system, busy machine", vec![clock_at((9, 0)), system()], true),
        ("clock and system, quiet machine", vec![clock_at((9, 0)), system()], false),
        ("clock with seconds", vec![seconds], false),
    ] {
        let scratch = Scratch::new("bytes");
        scratch.stat(0, 0);
        scratch.memory(40);
        let (wall, clock) = Wall::new();
        let app = base(clock).desktop(desktop(widgets)).probe(scratch.probe());
        let mut harness = draw(app, 120, 40);
        let (mut content, mut frames) = (0, 0);
        let (mut used, mut idle) = (0, 0);
        for step in 0..60_u64 {
            // Busy: a share that wanders between readings. Quiet: always a tenth.
            used += if busy { 37 + step * 13 % 50 } else { 10 };
            idle += if busy { 100 } else { 90 };
            scratch.stat(used, idle);
            let before = harness.buffer().clone();
            wall.pass(&mut harness, Duration::from_secs(1));
            let bytes = sent(&before, harness.buffer());
            content += bytes;
            frames += usize::from(bytes > 0);
        }
        let total = content + frames * FRAMING;
        println!("{name}: about {total} bytes a minute at 120x40 ({frames} frames)");
    }
}

#[test]
fn a_file_without_widgets_is_the_floor_it_always_was() {
    let path = Path::new("/home/kisi/.config/quvyta/desktop/desktop.toml");
    let (read, problems) = qdesk::desktop::order::parse(path, b"icons = [\"terminal\"]\nwelcome_seen = true\n");
    assert!(problems.is_empty());
    assert!(read.widgets.is_empty());
}
