//! Workspaces from where a person starts them: the digits of desktop mode, the marks beside the
//! launcher button, and a window's menu on the dock (design 3.10).
//!
//! Every window here is one of qdesk's own screens or a program this file writes itself
//! (`/bin/sh -c ...`); nothing of the person's is run. What a program says is waited for in a loop
//! bounded by time, never slept on.

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::{Desk, Keys};
use qdesk::apps::{Catalog, Declared, Entry, Launch, Source, parse_entry};
use qdesk::desktop::Desktop;
use qdesk::dock;
use qdesk::wm::WindowId;
use qframe::color::ColorDepth;
use qframe::event::{MouseButton, MouseKind};
use qframe::icons::GlyphMode;
use qframe::prelude::*;

use support::{BUDGET, HOME, MACHINE, decoration, desk, harness_with};

/// The button padding of the Quvyta themes.
const PAD: u16 = 2;

/// The column the first mark stands in: after the dock's edge, the launcher button and the gap.
fn first_mark() -> usize {
    usize::from(dock::EDGE + dock::launcher_width(PAD) + dock::MARKS_GAP)
}

/// The last row of the screen.
fn dock_row(harness: &Harness<Desk>) -> String {
    harness.screen().lines().last().expect("the dock row").to_owned()
}

/// The seven cells the four marks stand in, as text.
fn marks(harness: &Harness<Desk>) -> String {
    dock_row(harness).chars().skip(first_mark()).take(7).collect()
}

/// Clicks the mark of the workspace `space`, counted from 0, on the dock at the bottom.
fn click_mark(harness: &mut Harness<Desk>, space: usize) {
    let x = i32::try_from(first_mark() + 2 * space).expect("a column on screen");
    let y = i32::try_from(harness.screen().lines().count() - 1).expect("a row on screen");
    harness.click(x, y);
}

/// Opens the Settings window from its icon on the floor, as a person does, and gives its id.
fn open_settings(harness: &mut Harness<Desk>) -> WindowId {
    harness.click(3, 4);
    harness.click(3, 4);
    let id = harness.app().windows().focus().expect("the Settings window opened");
    assert!(harness.screen().contains("Appearance"), "{}", harness.screen());
    id
}

#[test]
fn a_new_desktop_marks_the_first_workspace_and_three_empty_ones() {
    let harness = desk(80, 24);
    assert_eq!(marks(&harness), "1 ○ ○ ○", "{}", dock_row(&harness));
}

#[test]
fn a_digit_in_desktop_mode_goes_to_that_workspace_and_gives_it_the_keys() {
    let mut harness = desk(80, 24);
    let settings = open_settings(&mut harness);
    assert_eq!(marks(&harness), "1 ○ ○ ○");
    harness.press("ctrl+alt+space").press("2");
    assert_eq!(harness.app().windows().current(), 1);
    assert_eq!(harness.app().keys(), None, "going somewhere leaves desktop mode");
    let screen = harness.screen();
    assert!(!screen.contains("Appearance"), "the window of the first workspace is not drawn:\n{screen}");
    assert!(!dock_row(&harness).contains("Settings"), "nor listed: {}", dock_row(&harness));
    assert_eq!(marks(&harness), "• 2 ○ ○", "the first holds a window, the second is on screen");
    assert!(harness.is_focused("floor"), "an empty workspace leaves the keys on the floor");
    harness.press("ctrl+alt+space").press("1");
    assert!(harness.screen().contains("Appearance"));
    assert_eq!(harness.app().windows().focus(), Some(settings), "the window that had the keys has them again");
    assert!(harness.is_focused("settings-list"), "and the keys are in it");
}

#[test]
fn a_digit_outside_desktop_mode_is_the_window_s() {
    let mut harness = desk(80, 24);
    harness.press("2");
    assert_eq!(harness.app().windows().current(), 0, "a digit typed on the floor goes nowhere else");
}

#[test]
fn alt_and_a_digit_send_the_window_away_and_desktop_mode_stays() {
    let mut harness = desk(80, 24);
    let settings = open_settings(&mut harness);
    harness.press("ctrl+alt+space").press("alt+3");
    let windows = harness.app().windows();
    assert_eq!(windows.get(settings).map(qdesk::wm::Window::space), Some(2));
    assert_eq!(windows.current(), 0, "the person stays where they are");
    assert_eq!(windows.focus(), None);
    assert_eq!(harness.app().keys(), Some(Keys::Pick), "sending is arranging; the mode stays");
    assert!(!harness.screen().contains("Appearance"));
    harness.press("esc");
    assert_eq!(marks(&harness), "1 ○ • ○", "{}", dock_row(&harness));
    harness.press("ctrl+alt+space").press("3");
    assert!(harness.screen().contains("Appearance"));
    assert_eq!(harness.app().windows().focus(), Some(settings), "it leads where it went");
}

#[test]
fn the_hint_row_of_desktop_mode_names_the_digits() {
    let mut harness = desk(80, 24);
    harness.press("ctrl+alt+space");
    let row = dock_row(&harness);
    assert!(row.contains("1-4  spaces"), "{row}");
    assert!(row.contains("esc"), "the way back keeps its place before it: {row}");
}

#[test]
fn a_click_on_a_mark_goes_to_its_workspace() {
    let mut harness = desk(80, 24);
    let settings = open_settings(&mut harness);
    click_mark(&mut harness, 2);
    assert_eq!(harness.app().windows().current(), 2, "{}", dock_row(&harness));
    assert_eq!(marks(&harness), "• ○ 3 ○");
    assert!(!harness.screen().contains("Appearance"));
    click_mark(&mut harness, 0);
    assert_eq!(harness.app().windows().current(), 0);
    assert_eq!(harness.app().windows().focus(), Some(settings));
    assert!(harness.screen().contains("Appearance"));
}

#[test]
fn the_dock_lists_only_the_windows_of_the_workspace_on_screen() {
    let mut harness = desk(80, 24);
    open_settings(&mut harness);
    click_mark(&mut harness, 1);
    // The Terminal icon is the first of the floor: two clicks open it here.
    harness.click(5, 1);
    harness.click(5, 1);
    assert_eq!(harness.app().windows().len(), 2);
    let row = dock_row(&harness);
    assert!(row.contains("Terminal") && !row.contains("Settings"), "{row}");
    assert_eq!(marks(&harness), "• 2 ○ ○");
    click_mark(&mut harness, 0);
    let row = dock_row(&harness);
    assert!(row.contains("Settings") && !row.contains("Terminal"), "{row}");
    assert_eq!(marks(&harness), "1 • ○ ○");
}

#[test]
fn the_menu_of_a_window_on_the_dock_moves_it_to_another_workspace() {
    let mut harness = desk(80, 24);
    let settings = open_settings(&mut harness);
    // "Settings" is on the floor and on the window's strip too; the dock's item is on the last row.
    let row = dock_row(&harness);
    let at = row.find("Settings").expect("the window's item");
    let x = i32::from(qframe::text::width(&row[..at]));
    let y = i32::try_from(harness.screen().lines().count() - 1).expect("a row on screen");
    harness.mouse(MouseKind::Down(MouseButton::Right), x, y);
    harness.mouse(MouseKind::Up(MouseButton::Right), x, y);
    let screen = harness.screen();
    for space in 2..=4 {
        assert!(screen.contains(&format!("Move to workspace {space}")), "{screen}");
    }
    assert!(!screen.contains("Move to workspace 1"), "it is on the first already:\n{screen}");
    assert_eq!(decoration(&screen), None, "the menu is drawn in tones:\n{screen}");
    harness.click_text("Move to workspace 4");
    assert_eq!(harness.app().windows().get(settings).map(qdesk::wm::Window::space), Some(3));
    assert_eq!(harness.app().windows().current(), 0, "moving from the menu does not go along");
    assert!(!dock_row(&harness).contains("Settings"));
    assert_eq!(marks(&harness), "1 ○ ○ •");
}

#[test]
fn a_narrow_dock_keeps_the_number_and_a_click_on_it_goes_on_to_the_next() {
    let mut harness = desk(40, 10);
    assert_eq!(marks(&harness), "1 ○ ○ ○", "an empty desktop has room for every mark");
    harness.press("ctrl+alt+space").press("space");
    harness.type_text("sett");
    harness.press("enter");
    assert_eq!(harness.app().windows().len(), 1, "{}", harness.screen());
    let row = dock_row(&harness);
    let number: String = row.chars().skip(first_mark()).take(3).collect();
    assert_eq!(number, "1  ", "only the number, and the window beside it: {row}");
    assert!(row.contains(MACHINE), "the machine name is whole: {row}");
    assert!(qframe::text::width(&row) <= 40, "{row}");
    let x = i32::try_from(first_mark()).expect("a column");
    harness.click(x, 9);
    assert_eq!(harness.app().windows().current(), 1, "{}", dock_row(&harness));
    assert_eq!(marks(&harness), "• 2 ○ ○", "an empty workspace has room for every mark again");
    for _ in 0..3 {
        let next = (harness.app().windows().current() + 1) % 4;
        click_mark(&mut harness, next);
    }
    assert_eq!(harness.app().windows().current(), 0, "round from the last to the first");
}

#[test]
fn the_marks_read_apart_in_ascii_and_sixteen_colours_without_brackets_or_lines() {
    let mut harness = desk(80, 24);
    harness.set_glyph_mode(GlyphMode::Ascii).set_depth(ColorDepth::Ansi16);
    harness.click(3, 4);
    harness.click(3, 4);
    click_mark(&mut harness, 1);
    assert_eq!(marks(&harness), "- 2 o o", "{}", dock_row(&harness));
    let screen = harness.screen();
    assert_eq!(decoration(&screen), None, "{screen}");
    let y = u16::try_from(screen.lines().count() - 1).expect("a row");
    let at = |space: usize| u16::try_from(first_mark() + 2 * space).expect("a column");
    assert!(harness.is_bold(at(1), y), "the workspace on screen is bold");
    assert!(!harness.is_bold(at(0), y) && !harness.is_bold(at(2), y));
    // In true colour each look takes its tone of the theme: the accent, the text, the faint text.
    // The monochrome theme's accent is its text colour, which is why the shapes carry the meaning.
    harness.set_depth(ColorDepth::TrueColor).set_glyph_mode(GlyphMode::Unicode);
    let theme = harness.env().theme();
    let (accent, text, dim) = (theme.color("accent"), theme.color("text"), theme.color("dim"));
    assert_eq!(harness.fg(at(1), y), accent, "the workspace on screen");
    assert_eq!(harness.fg(at(0), y), text, "a workspace with windows");
    assert_eq!(harness.fg(at(2), y), dim, "an empty one");
    assert_ne!(text, dim);
}

/// A path of this test's own to tell a program with.
fn signal_path(what: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let once = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!("qdesk-spaces-{what}-{}-{once}", std::process::id()))
}

/// An entry of this test that runs `command`.
fn entry(id: &str, name: &str, command: &[&str]) -> Entry {
    let words = command
        .iter()
        .map(|word| {
            assert!(!word.contains('"'), "a test command is written without double quotes");
            format!("\"{}\"", word.replace('\\', "\\\\"))
        })
        .collect::<Vec<String>>()
        .join(", ");
    let text = format!("name = \"{name}\"\ncommand = [{words}]\ncategory = \"system\"\n");
    let file = format!("{HOME}/.local/share/quvyta/desktop/apps/{id}.toml");
    let (declared, diagnostics) =
        parse_entry(id, Path::new(&file), text.as_bytes(), Source::User, Some(Path::new(HOME)));
    assert!(diagnostics.is_empty(), "{id}: {diagnostics:?}");
    match declared {
        Some(Declared::Entry(entry)) => *entry,
        other => panic!("{id} declares no entry: {other:?}"),
    }
}

#[test]
fn a_program_that_ends_on_another_workspace_says_so_and_its_notice_goes_there() {
    let signal = signal_path("biten");
    let _ = std::fs::remove_file(&signal);
    let command = format!("while [ ! -e {} ]; do sleep 0.02; done; exit 4", signal.display());
    let ending = entry("biten", "Biten", &["/bin/sh", "-c", &command]);
    let catalog = Catalog::new(vec![ending], |entry| !matches!(entry.launch, Launch::Open(_)));
    let desktop = Desktop { icons: Vec::new(), recents: Vec::new(), welcome_seen: true, ..Desktop::default() };
    let mut harness = harness_with(catalog, desktop, 100, 30);
    // Opened from the launcher, as a person opens it.
    harness.press("ctrl+alt+space").press("space");
    harness.type_text("Biten");
    harness.press("enter");
    let id = harness.app().windows().focus().expect("the window opened");
    harness.press("ctrl+alt+space").press("2");
    // The program ends only now, while the person is on the second workspace.
    std::fs::write(&signal, b"").expect("the signal file is written");
    let deadline = Instant::now() + BUDGET;
    while !dock_row(&harness).contains("●1") {
        assert!(Instant::now() < deadline, "the end never reached the dock: {}", dock_row(&harness));
        harness.render();
    }
    let _ = std::fs::remove_file(&signal);
    harness.click_text("●1");
    harness.press("enter");
    assert_eq!(harness.app().windows().current(), 0, "the notice went to the window's workspace");
    assert_eq!(harness.app().windows().focus(), Some(id), "{}", harness.screen());
}
