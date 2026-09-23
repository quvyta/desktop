//! The desktop screen: the floor and the dock, at several sizes, glyph modes and colour depths,
//! in a terminal too small for it, and quitting.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use qdesk::app::{Desk, MIN_HEIGHT, MIN_WIDTH};
use qdesk::desktop::{Desktop, quvyta_icon};
use qframe::color::ColorDepth;
use qframe::env::{AssetDirs, Env};
use qframe::icons::GlyphMode;
use qframe::prelude::*;

/// Saturday 19 September 2026, 11:32:30 UTC: 14:32:30 three hours east.
const MOMENT: i64 = 1_789_817_550;
/// The time zone of the tests, in minutes east of UTC.
const OFFSET: i16 = 180;
const MACHINE: &str = "sunucu-1";

/// A wall clock the tests move by hand.
#[derive(Clone)]
struct Clock(Rc<Cell<i64>>);

impl Clock {
    fn at(seconds: i64) -> Self {
        Self(Rc::new(Cell::new(seconds * 1_000)))
    }

    fn pass(&self, seconds: i64) {
        self.0.set(self.0.get() + seconds * 1_000);
    }

    fn reader(&self) -> Box<dyn Fn() -> i64> {
        let clock = self.clone();
        Box::new(move || clock.0.get())
    }
}

fn env() -> Env {
    let dirs = AssetDirs {
        locale_sources: qdesk::locales().iter().map(|(file, text)| ((*file).to_owned(), (*text).to_owned())).collect(),
        keymap_source: Some({
            let (file, text) = qdesk::keymap();
            (file.to_owned(), text.to_owned())
        }),
        ..AssetDirs::default()
    };
    Env::load(&dirs).expect("the built-in files load")
}

/// A desktop with nothing on its floor and the welcome line already seen, so the tests of the
/// dock and the floor's tone see nothing else.
fn desk_on(machine: Option<&str>, offset: Option<i16>, clock: &Clock, width: u16, height: u16) -> Harness<Desk> {
    let desktop = Desktop { icons: Vec::new(), recents: Vec::new(), welcome_seen: true };
    let app = Desk::new(machine.map(str::to_owned), offset, clock.reader()).desktop(desktop);
    let mut harness = Harness::with_env(app, env(), width, height);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);
    harness
}

fn desk(width: u16, height: u16) -> Harness<Desk> {
    desk_on(Some(MACHINE), Some(OFFSET), &Clock::at(MOMENT), width, height)
}

fn rows(harness: &Harness<Desk>) -> Vec<String> {
    harness.screen().lines().map(str::to_owned).collect()
}

/// The dock row as a terminal of `width` columns shows it: the launcher button at the left end,
/// the parts at the right one, one free cell after them.
fn dock_line(harness: &Harness<Desk>, width: u16, parts: &str) -> String {
    let glyph = harness.env().icons().glyph(quvyta_icon(harness.env().icons())).into_owned();
    let left = format!("  {glyph} ");
    let used = qframe::text::width(&left) + qframe::text::width(parts) + 1;
    format!("{left}{}{parts}", " ".repeat(usize::from(width) - usize::from(used)))
}

/// The decorations the aesthetic rules forbid in every mode: brackets around things, pipes and
/// box drawing.
fn decoration(screen: &str) -> Option<char> {
    screen.chars().find(|c| matches!(c, '[' | ']' | '|' | '{' | '}') || ('\u{2500}'..='\u{257F}').contains(c))
}

#[test]
fn a_standard_terminal_shows_the_bare_floor_and_the_dock_with_the_machine_and_the_clock() {
    let harness = desk(80, 24);
    let rows = rows(&harness);
    assert_eq!(rows.len(), 24);
    assert!(rows[..23].iter().all(String::is_empty), "the floor is bare:\n{}", harness.screen());
    assert_eq!(rows[23], dock_line(&harness, 80, "sunucu-1   14:32"));
}

#[test]
fn the_floor_has_the_canvas_tone_and_the_dock_a_tone_of_its_own() {
    let harness = desk(80, 24);
    let theme = harness.env().theme();
    let canvas = theme.color("canvas").expect("the theme has a canvas");
    let surface = theme.color("surface").expect("the theme has a surface");
    assert_ne!(canvas, surface);
    for (x, y) in [(0, 0), (40, 12), (79, 22)] {
        assert_eq!(harness.bg(x, y), Some(canvas), "floor at {x},{y}");
    }
    for x in [0, 40, 79] {
        assert_eq!(harness.bg(x, 23), Some(surface), "dock at column {x}");
    }
    let (name, _) = harness.find(MACHINE).expect("the machine name is on screen");
    assert_eq!(harness.fg(u16::try_from(name).unwrap_or(0), 23), theme.color("text"), "the name reads in full text");
}

#[test]
fn a_large_terminal_keeps_the_dock_on_the_bottom_row_and_the_rest_bare() {
    let harness = desk(200, 50);
    let rows = rows(&harness);
    assert_eq!(rows.len(), 50);
    assert!(rows[..49].iter().all(String::is_empty));
    assert_eq!(rows[49], dock_line(&harness, 200, "sunucu-1   14:32"));
}

#[test]
fn a_narrow_terminal_still_shows_the_whole_dock_when_it_fits() {
    let harness = desk(60, 16);
    assert_eq!(rows(&harness)[15], dock_line(&harness, 60, "sunucu-1   14:32"));
    let smallest = desk(MIN_WIDTH, MIN_HEIGHT);
    assert_eq!(rows(&smallest)[9], dock_line(&smallest, MIN_WIDTH, "sunucu-1   14:32"));
}

#[test]
fn a_long_machine_name_pushes_the_clock_out_before_it_is_shortened() {
    let clock = Clock::at(MOMENT);
    let name = "build-server-europe-west-17";
    let wide = desk_on(Some(name), Some(OFFSET), &clock, 80, 24);
    assert_eq!(rows(&wide)[23], dock_line(&wide, 80, "build-server-europe-west-17   14:32"));
    let narrow = desk_on(Some(name), Some(OFFSET), &clock, 32, 12);
    assert!(rows(&narrow).iter().any(|row| row.contains("Terminal too small")), "32 columns is below the desktop");
    // 34 cells fit the 35 a 40-column dock leaves beside the launcher button, but not with the
    // clock beside them.
    let alone = desk_on(Some("build-server-europe-west-3-tail"), Some(OFFSET), &clock, 40, 10);
    assert_eq!(rows(&alone)[9], dock_line(&alone, 40, "build-server-europe-west-3-tail"), "the clock went first");
    let cut = desk_on(Some("build-server-europe-west-17-very-long-tail"), Some(OFFSET), &clock, 40, 10);
    let dock = &rows(&cut)[9];
    assert!(dock.contains("build") && dock.ends_with("long-tail"), "the name keeps both ends: {dock}");
    assert!(dock.contains('…') && !dock.contains("14:32"), "{dock}");
    assert_eq!(qframe::text::width(dock), 39, "the name fills the row up to its edge cell");
}

#[test]
fn below_forty_by_ten_the_screen_says_the_terminal_is_too_small() {
    for (width, height) in [(39, 9), (39, 24), (80, 9), (20, 5)] {
        let harness = desk(width, height);
        let screen = harness.screen();
        assert!(screen.contains("Terminal too small"), "{width}x{height}:\n{screen}");
        assert!(!screen.contains(MACHINE), "no dock at {width}x{height}");
    }
    let harness = desk(39, 9);
    let screen = harness.screen();
    assert!(screen.contains("40 columns") && screen.contains("10 rows"), "the size it needs:\n{screen}");
    assert_eq!(harness.bg(0, 0), harness.env().theme().color("canvas"), "the message stands on the floor's tone");
}

#[test]
fn growing_the_terminal_brings_the_desktop_back() {
    let mut harness = desk(39, 9);
    assert!(harness.screen().contains("Terminal too small"));
    harness.resize(80, 24);
    assert_eq!(rows(&harness)[23], dock_line(&harness, 80, "sunucu-1   14:32"));
    harness.resize(30, 24);
    assert!(harness.screen().contains("Terminal too small"));
}

#[test]
fn the_too_small_message_is_in_the_language_of_the_screen() {
    let mut harness = desk(39, 9);
    harness.set_locale("tr");
    let screen = harness.screen();
    assert!(screen.contains("Terminal çok küçük"), "{screen}");
    assert!(screen.contains("en az 40 sütun") && screen.contains("satır gerekiyor"), "{screen}");
}

#[test]
fn every_glyph_mode_draws_the_same_desktop_without_decoration() {
    for mode in [GlyphMode::Nerd, GlyphMode::Unicode, GlyphMode::Ascii] {
        for (width, height) in [(80, 24), (60, 16), (39, 9)] {
            let mut harness = desk(width, height);
            harness.set_glyph_mode(mode);
            let screen = harness.screen();
            assert_eq!(decoration(&screen), None, "{mode:?} {width}x{height}:\n{screen}");
            if mode == GlyphMode::Ascii {
                assert!(screen.is_ascii(), "ASCII mode draws only ASCII:\n{screen}");
            }
        }
    }
    let mut ascii = desk(80, 24);
    ascii.set_glyph_mode(GlyphMode::Ascii);
    assert_eq!(rows(&ascii)[23], dock_line(&ascii, 80, "sunucu-1   14:32"));
}

#[test]
fn sixteen_and_256_colours_keep_the_dock_readable() {
    for depth in [ColorDepth::Ansi16, ColorDepth::Ansi256] {
        let mut harness = desk(80, 24);
        harness.set_depth(depth);
        assert_eq!(rows(&harness)[23], dock_line(&harness, 80, "sunucu-1   14:32"), "{depth:?}");
        let buffer = harness.buffer();
        let (x, _) = harness.find(MACHINE).expect("the name is drawn");
        let cell = &buffer[(u16::try_from(x).unwrap_or(0), 23)];
        assert_ne!(cell.fg, cell.bg, "{depth:?}: the name stands out from the dock");
    }
    // 256 colours keep the dock a step above the floor; the 16 standard colours have one
    // black for both, and the dock is then told by its text alone.
    let mut harness = desk(80, 24);
    harness.set_depth(ColorDepth::Ansi256);
    let buffer = harness.buffer();
    assert_ne!(buffer[(5, 5)].bg, buffer[(5, 23)].bg, "the dock has a tone of its own in 256 colours");
}

#[test]
fn ctrl_q_quits() {
    let mut harness = desk(80, 24);
    assert!(!harness.quit_requested());
    harness.press("ctrl+q");
    assert!(harness.quit_requested());
}

#[test]
fn ctrl_q_quits_from_the_too_small_screen_too() {
    let mut harness = desk(39, 9);
    harness.press("ctrl+q");
    assert!(harness.quit_requested());
}

#[test]
fn the_clock_turns_on_the_minute_not_before() {
    let clock = Clock::at(MOMENT);
    let mut harness = desk_on(Some(MACHINE), Some(OFFSET), &clock, 80, 24);
    // 14:32:30: the next minute is thirty seconds away.
    clock.pass(29);
    harness.advance(Duration::from_secs(29));
    assert!(rows(&harness)[23].ends_with("14:32"));
    clock.pass(1);
    harness.advance(Duration::from_secs(1));
    assert!(rows(&harness)[23].ends_with("14:33"), "{}", harness.screen());
    // And every minute after that.
    clock.pass(60);
    harness.advance(Duration::from_secs(60));
    assert!(rows(&harness)[23].ends_with("14:34"), "{}", harness.screen());
}

#[test]
fn an_unknown_time_zone_shows_utc_and_says_so() {
    let harness = desk_on(Some(MACHINE), None, &Clock::at(MOMENT), 80, 24);
    assert_eq!(rows(&harness)[23], dock_line(&harness, 80, "sunucu-1   11:32 UTC"));
}

#[test]
fn a_machine_without_a_name_is_called_this_machine() {
    let mut harness = desk_on(None, Some(OFFSET), &Clock::at(MOMENT), 80, 24);
    assert_eq!(rows(&harness)[23], dock_line(&harness, 80, "this machine   14:32"));
    harness.set_locale("tr");
    assert_eq!(rows(&harness)[23], dock_line(&harness, 80, "bu makine   14:32"));
}
