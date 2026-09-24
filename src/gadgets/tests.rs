//! The gadgets without a screen: their sizes, where they stand, where a new one goes, and how the
//! desktop file writes and reads them.

use std::path::PathBuf;

use super::*;
use crate::desktop::order::{Desktop, parse as read_desktop};

fn spot(at: Cell, size: (u16, u16)) -> Spot {
    Spot { at, size }
}

fn file() -> PathBuf {
    PathBuf::from("/home/kisi/.config/quvyta/desktop/desktop.toml")
}

fn read(text: &str) -> (Desktop, Vec<Diagnostic>) {
    read_desktop(&file(), text.as_bytes())
}

#[test]
fn each_gadget_takes_the_cells_its_content_needs() {
    assert_eq!(Gadget::new(Kind::Clock, (0, 0), "").size(), (3, 3));
    assert_eq!(Gadget::new(Kind::Calendar, (0, 0), "").size(), (3, 4));
    assert_eq!(Gadget::new(Kind::Note, (0, 0), "").size(), (3, 3));
    let mut system = Gadget::new(Kind::System, (0, 0), "");
    assert_eq!(system.size(), (3, 3), "four readings need a third row of cells");
    assert!(system.apply(Change::Reading(Reading::Battery, false)));
    assert_eq!(system.size(), (3, 2), "three fit in two");
    let mut clock = Gadget::new(Kind::Clock, (0, 0), "");
    assert!(clock.apply(Change::Seconds(true)));
    assert_eq!(clock.size(), (4, 3), "the seconds make the clock wider");
    assert!(!clock.apply(Change::Seconds(true)), "the same choice again changes nothing");
    assert!(!clock.apply(Change::WeekStart(WeekStart::Monday)), "a calendar's option is not a clock's");
}

#[test]
fn a_gadget_stands_in_its_own_place_when_it_is_free_and_on_the_floor() {
    let drawn = arrange(12, 13, &[spot((9, 0), (3, 3)), spot((9, 3), (3, 4))]);
    assert_eq!(drawn, [Some(spot((9, 0), (3, 3))), Some(spot((9, 3), (3, 4)))]);
}

#[test]
fn a_gadget_whose_place_is_taken_or_off_the_floor_stands_in_the_nearest_free_spot() {
    // The second wants the first one's cells; the nearest free spot is right under it.
    let drawn = arrange(12, 13, &[spot((9, 0), (3, 3)), spot((9, 1), (3, 3))]);
    assert_eq!(drawn[1], Some(spot((9, 3), (3, 3))));
    // A place the terminal has shrunk away from: the spot nearest to it that fits.
    let drawn = arrange(8, 7, &[spot((9, 0), (3, 3))]);
    assert_eq!(drawn[0], Some(spot((5, 0), (3, 3))));
    // No room at all: not drawn.
    let drawn = arrange(2, 7, &[spot((0, 0), (3, 3))]);
    assert_eq!(drawn[0], None);
    let drawn = arrange(3, 3, &[spot((0, 0), (3, 3)), spot((0, 0), (3, 3))]);
    assert_eq!(drawn, [Some(spot((0, 0), (3, 3))), None], "the first keeps its spot");
}

#[test]
fn a_new_gadget_goes_to_the_top_right_clear_of_icons_and_gadgets() {
    assert_eq!(spot_for(12, 13, (3, 3), &[], &[(0, 0), (0, 1)]), Some((9, 0)));
    // Under the gadget already there.
    assert_eq!(spot_for(12, 13, (3, 4), &[spot((9, 0), (3, 3))], &[]), Some((9, 3)));
    // Where the icons fill the right edge, left of them.
    let icons: Vec<Cell> = (0..13).map(|row| (11, row)).collect();
    assert_eq!(spot_for(12, 13, (3, 3), &[], &icons), Some((8, 0)));
    // A floor full of icons: the gadget goes where no gadget is, and the icons make way.
    let everywhere: Vec<Cell> = (0..4).flat_map(|column| (0..3).map(move |row| (column, row))).collect();
    assert_eq!(spot_for(4, 3, (3, 3), &[], &everywhere), Some((1, 0)));
    // A floor too small, or full of gadgets: nowhere.
    assert_eq!(spot_for(2, 3, (3, 3), &[], &[]), None);
    assert_eq!(spot_for(3, 3, (3, 3), &[spot((0, 0), (3, 3))], &[]), None);
}

#[test]
fn spots_overlap_only_when_they_share_a_cell() {
    let one = spot((2, 2), (3, 3));
    assert!(one.overlaps(spot((4, 4), (1, 1))));
    assert!(!one.overlaps(spot((5, 2), (3, 3))), "side by side");
    assert!(!one.overlaps(spot((2, 5), (3, 3))), "one above the other");
    assert!(one.holds((4, 4)) && !one.holds((5, 4)));
    assert_eq!(one.cells().len(), 9);
}

#[test]
fn each_note_takes_a_file_of_its_own_and_never_a_path() {
    assert_eq!(next_note(&[]), "notes.txt");
    let first = Gadget::new(Kind::Note, (0, 0), "notes.txt");
    assert_eq!(next_note(std::slice::from_ref(&first)), "notes-2.txt");
    let third = Gadget::new(Kind::Note, (0, 0), "notes-2.txt");
    assert_eq!(next_note(&[first, third]), "notes-3.txt");
    assert!(note_name_is_plain("notes.txt"));
    for bad in ["", ".", "..", "../x", "a/b", "/etc/passwd", "a\\b"] {
        assert!(!note_name_is_plain(bad), "{bad:?}");
    }
}

#[test]
fn the_last_reading_of_the_system_gadget_cannot_be_taken_away() {
    let mut readings = Readings::default();
    for reading in [Reading::Cpu, Reading::Battery, Reading::Network] {
        assert!(readings.set(reading, false));
    }
    assert_eq!(readings.shown(), [Reading::Memory]);
    assert!(!readings.set(Reading::Memory, false), "the last one stays");
    assert!(readings.set(Reading::Cpu, true));
    assert_eq!(readings.shown(), [Reading::Cpu, Reading::Memory], "in their own order");
}

#[test]
fn what_is_written_reads_back_the_same_with_only_the_options_that_differ() {
    let mut clock = Gadget::new(Kind::Clock, (9, 0), "");
    clock.apply(Change::Twelve(true));
    clock.apply(Change::Seconds(true));
    clock.apply(Change::Tint(Tint::Gradient));
    let mut calendar = Gadget::new(Kind::Calendar, (9, 3), "");
    calendar.apply(Change::WeekStart(WeekStart::Sunday));
    let mut system = Gadget::new(Kind::System, (6, 0), "");
    system.apply(Change::Reading(Reading::Network, false));
    let plain = Gadget::new(Kind::Clock, (0, 5), "");
    let note = Gadget::new(Kind::Note, (6, 3), "notes-2.txt");
    let desktop = Desktop { widgets: vec![clock, calendar, system, plain, note], ..Desktop::default() };
    let text = desktop.to_toml();
    assert!(
        text.contains("[[widgets]]\nkind = \"clock\"\nplace = [0, 5]\n[[widgets]]"),
        "defaults are not written:\n{text}"
    );
    assert!(text.contains("hours = 12\nseconds = true\ncolor = \"gradient\"\n"), "{text}");
    assert!(text.contains("week-start = \"sunday\"\n"), "{text}");
    assert!(text.contains("readings = [\"cpu\", \"memory\", \"battery\"]\n"), "{text}");
    assert!(text.contains("file = \"notes-2.txt\"\n"), "{text}");
    let (read, problems) = read(&text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(read, desktop);
}

#[test]
fn a_desktop_without_gadgets_writes_no_table_for_them() {
    assert!(!Desktop::default().to_toml().contains("widgets"));
}

#[test]
fn a_broken_gadget_is_a_diagnostic_at_its_line_and_the_others_still_load() {
    let text = "icons = []\n\
                [[widgets]]\nkind = \"clock\"\nplace = [9, 0]\nhours = 13\n\
                [[widgets]]\nkind = \"weather\"\nplace = [0, 0]\n\
                [[widgets]]\nkind = \"calendar\"\n\
                [[widgets]]\nkind = \"note\"\nplace = [1, 1]\nfile = \"../../.bashrc\"\n\
                [[widgets]]\nkind = \"system\"\nplace = [2, 2]\nsparkles = true\n";
    let (desktop, problems) = read(text);
    let kinds: Vec<Kind> = desktop.widgets.iter().map(|gadget| gadget.shows.kind()).collect();
    assert_eq!(kinds, [Kind::Clock, Kind::Note, Kind::System], "what can be read is kept");
    assert_eq!(desktop.widgets[0].shows, Shows::Clock(Clock::default()), "a bad option keeps its default");
    assert_eq!(desktop.widgets[1].shows, Shows::Note(FIRST_NOTE.to_owned()), "a path is never a note's file");
    let said: Vec<(String, DiagnosticKind)> =
        problems.iter().map(|problem| (problem.location(), problem.kind.clone())).collect();
    let at = |line: usize| format!("{}:{line}", file().display());
    assert!(
        said.iter().any(|(place, kind)| place.starts_with(&at(5))
            && *kind == DiagnosticKind::InvalidValue { key: "widgets.hours".to_owned() }),
        "{said:?}"
    );
    assert!(
        said.iter().any(|(place, kind)| place.starts_with(&at(7))
            && *kind == DiagnosticKind::InvalidValue { key: "widgets.kind".to_owned() }),
        "{said:?}"
    );
    assert!(said.iter().any(|(_, kind)| *kind == DiagnosticKind::InvalidValue { key: "widgets.place".to_owned() }));
    assert!(said.iter().any(|(_, kind)| *kind == DiagnosticKind::InvalidValue { key: "widgets.file".to_owned() }));
    assert!(
        said.iter().any(|(place, kind)| place.starts_with(&at(18))
            && *kind == DiagnosticKind::UnknownField { key: "widgets.sparkles".to_owned() }),
        "{said:?}"
    );
    let (_, problems) = read("widgets = 3\n");
    assert_eq!(problems.len(), 1, "a wrong type is said, not a crash");
}
