//! The Settings screen: its sections at several sizes, glyph modes and colour depths, an option
//! changed, and the applications section with and without problems to report.

mod support;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use qdesk::apps::{Diagnostic, DiagnosticKind, Expected, Folders, Position};
use qdesk::settings::{self, APP, Applications, DockPosition, DragStyle, FloorColor, Msg, Prefs, Screen};
use qframe::color::ColorDepth;
use qframe::env::{AssetDirs, Env};
use qframe::i18n::I18n;
use qframe::icons::GlyphMode;
use qframe::prelude::*;
use qframe::storage::{Ecosystem, Settings};
use qframe::widgets::{Appearance, AppearanceChange};

/// The Settings screen with the state a test gives it, in an application of its own: the screen
/// is a view, and the desktop that will hold it arrives with the windows.
struct SettingsApp {
    screen: Screen,
    prefs: Prefs,
    remote: bool,
    folders: Folders,
    diagnostics: Vec<Diagnostic>,
    appearance: Appearance,
    stored: Settings,
}

impl App for SettingsApp {
    type Msg = Msg;

    /// The keyboard goes where the desktop gives it as the screen opens: to the list, or to the
    /// button that puts away what the settings file could not be read as while that is shown.
    fn init(&mut self) -> Command<Msg> {
        Command::focus(self.screen.keyboard())
    }

    fn update(&mut self, msg: Msg) -> Command<Msg> {
        // As on the desktop, the list takes the keyboard back when that button goes.
        let read = msg == Msg::ReadProblems;
        let (command, request) = settings::update(&mut self.screen, &self.prefs, msg);
        let command = if read { Command::batch([command, Command::focus(self.screen.keyboard())]) } else { command };
        match request {
            Some(settings::Request::Prefs(prefs)) => self.prefs = prefs,
            Some(settings::Request::Appearance(change)) => {
                return Command::batch([command, self.appearance.update(change, &mut self.stored)]);
            }
            Some(settings::Request::Wallpaper(_) | settings::Request::ShowRecommended) | None => {}
        }
        command
    }

    fn view(&self, ui: &mut View<'_, Msg>) {
        let apps = Applications { folders: &self.folders, diagnostics: &self.diagnostics };
        settings::view(&self.screen, &self.prefs, self.remote, &self.appearance, &apps, ui);
    }
}

/// A folder of the test's own in the system's temporary folder, standing for the ecosystem's: the
/// appearance section reads and saves there, never in the person's own `~/.config/quvyta`.
/// The appearance section and the desktop's settings over a folder of the test's own.
fn appearance() -> (Appearance, Settings) {
    let folder = ecosystem_folder();
    let preferences = Ecosystem::QUVYTA.preferences_in(&folder, APP, &I18n::builtin());
    let appearance = Appearance::new(Ecosystem::QUVYTA, APP, preferences).in_folder(&folder);
    (appearance, Settings::open(folder.join("desktop.conf")).member_of(&Ecosystem::QUVYTA))
}

fn ecosystem_folder() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let once = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("qdesk-settings-screen-{}-{once}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("folder");
    dir
}

/// Turns the wheel over the list until `text` is on screen, the way a person reaches a row below
/// the window's edge. At most as many turns as the screen has rows: the list is not that long.
fn scrolled_to(harness: &mut Harness<SettingsApp>, text: &str) -> String {
    let rows = harness.screen().lines().count();
    for _ in 0..rows {
        let shown = harness.screen();
        if shown.contains(text) {
            return shown;
        }
        harness.mouse(qframe::event::MouseKind::ScrollDown, 10, 5);
    }
    harness.screen()
}

fn env() -> Env {
    let dirs = AssetDirs {
        locale_sources: qdesk::locales().iter().map(|(file, text)| ((*file).to_owned(), (*text).to_owned())).collect(),
        ..AssetDirs::default()
    };
    Env::load_with(&dirs, support::terminal).expect("the built-in files load")
}

/// The folders a machine with one user folder and one system folder reads entries from.
fn folders() -> Folders {
    Folders {
        user: Some(PathBuf::from("/home/deniz/.local/share/quvyta/desktop/apps")),
        system: vec![PathBuf::from("/usr/share/quvyta/desktop/apps")],
        desktop_files: vec![PathBuf::from("/usr/share/applications")],
    }
}

/// Two entries that could not be read: one with a place in the file, one without.
fn problems() -> Vec<Diagnostic> {
    vec![
        Diagnostic::at(
            "/home/deniz/.local/share/quvyta/desktop/apps/htop.toml",
            Some(Position { line: 3, column: 8 }),
            DiagnosticKind::WrongType { key: "window.size".to_owned(), expected: Expected::Size },
        ),
        Diagnostic::at("/usr/share/quvyta/desktop/apps/logs.toml", None, DiagnosticKind::MissingName),
    ]
}

fn screen_of(
    prefs: Prefs,
    remote: bool,
    diagnostics: Vec<Diagnostic>,
    width: u16,
    height: u16,
) -> Harness<SettingsApp> {
    let (appearance, stored) = appearance();
    let app =
        SettingsApp { screen: Screen::default(), prefs, remote, folders: folders(), diagnostics, appearance, stored };
    let mut harness = Harness::with_env(app, env(), width, height);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);
    harness
}

fn screen(width: u16, height: u16) -> Harness<SettingsApp> {
    screen_of(Prefs::default(), false, problems(), width, height)
}

/// The decorations the aesthetic rules forbid in every mode: brackets around things, pipes and
/// box drawing.
fn decoration(text: &str) -> Option<char> {
    text.chars().find(|c| matches!(c, '[' | ']' | '|' | '{' | '}') || ('\u{2500}'..='\u{257F}').contains(c))
}

#[test]
fn a_standard_terminal_shows_every_section_and_what_is_in_force() {
    let mut harness = screen(80, 24);
    let shown = harness.screen();
    for heading in ["Appearance", "Language", "Theme", "Icons", "In every Quvyta application"] {
        assert!(shown.contains(heading), "no `{heading}`:\n{shown}");
    }
    assert_eq!(decoration(&shown), None, "nothing is drawn with lines or brackets:\n{shown}");
    // The shared rows fill a standard terminal; the desktop's own sections are turns of the wheel
    // below them.
    for text in ["Reduce motion", "Pillar", "Desktop", "Dock", "Wallpaper", "Connection"] {
        let shown = scrolled_to(&mut harness, text);
        assert!(shown.contains(text), "no `{text}`:\n{shown}");
        assert_eq!(decoration(&shown), None, "nothing is drawn with lines or brackets:\n{shown}");
    }
}

#[test]
fn a_tall_terminal_shows_the_connection_and_the_applications_as_well() {
    let harness = screen(200, 70);
    let shown = harness.screen();
    for text in ["Dragging a window", "Frames a second", "Remembered lines", "Applications"] {
        assert!(shown.contains(text), "no `{text}`:\n{shown}");
    }
    assert!(shown.contains("/usr/share/applications"), "the folders entries come from are named:\n{shown}");
    assert_eq!(decoration(&shown), None, "{shown}");
}

#[test]
fn the_smallest_desktop_still_draws_the_screen_from_its_top() {
    let harness = screen(60, 16);
    let shown = harness.screen();
    assert!(shown.contains("Appearance"), "{shown}");
    assert_eq!(shown.lines().count(), 16);
    assert!(shown.lines().all(|line| qframe::text::width(line) <= 60), "nothing runs past the edge:\n{shown}");
    assert_eq!(decoration(&shown), None, "{shown}");
}

#[test]
fn every_glyph_mode_and_sixteen_colours_draw_the_screen_without_decoration() {
    for mode in [GlyphMode::Nerd, GlyphMode::Unicode, GlyphMode::Ascii] {
        for depth in [ColorDepth::TrueColor, ColorDepth::Ansi256, ColorDepth::Ansi16] {
            let mut harness = screen(80, 24);
            harness.set_glyph_mode(mode).set_depth(depth).render();
            let shown = harness.screen();
            assert!(shown.contains("Appearance"), "{mode:?} {depth:?}:\n{shown}");
            assert_eq!(decoration(&shown), None, "{mode:?} {depth:?}:\n{shown}");
        }
    }
}

#[test]
fn the_screen_speaks_turkish_when_the_language_does() {
    let mut harness = screen(80, 24);
    harness.set_locale("tr").render();
    let shown = harness.screen();
    for text in ["Görünüm", "Dil", "Renk teması"] {
        assert!(shown.contains(text), "no `{text}`:\n{shown}");
    }
    for text in ["Masaüstü", "Bağlantı"] {
        let shown = scrolled_to(&mut harness, text);
        assert!(shown.contains(text), "no `{text}`:\n{shown}");
    }
}

#[test]
fn the_screen_speaks_every_language_qdesk_carries() {
    // The word for the connection section, as each file has it: a file the runtime did not load
    // would show the English word instead.
    let words = [
        ("de", "Verbindung"),
        ("es", "Conexión"),
        ("fr", "Connexion"),
        ("ja", "接続"),
        ("pt-BR", "Conexão"),
        ("ru", "Соединение"),
        ("zh-Hans", "连接"),
    ];
    for (code, word) in words {
        let mut harness = screen(100, 30);
        harness.set_locale(code).render();
        let shown = scrolled_to(&mut harness, word);
        assert!(shown.contains(word), "{code}: no `{word}`:\n{shown}");
        assert!(!shown.contains("Connection"), "{code}: English is left:\n{shown}");
    }
}

#[test]
fn a_changed_drag_style_is_shown_at_once_with_what_it_comes_to_here() {
    let mut harness = screen(80, 24);
    let before = scrolled_to(&mut harness, "a shaded area follows the mouse");
    assert!(before.contains("a shaded area follows the mouse"), "ghost is the default everywhere:\n{before}");

    support::pick(&mut harness, "Dragging a window", "Ghost", "Live");

    assert_eq!(harness.app().prefs.drag, DragStyle::Live, "the change is applied, not only drawn");
    let shown = harness.screen();
    assert!(support::on_row(&harness, "Dragging a window", "Live").is_some(), "the field shows it:\n{shown}");
    assert!(shown.contains("the window itself follows the mouse"), "the line says what it comes to:\n{shown}");

    // And back: the list gives the style that was clicked, whichever place it has in the list.
    support::pick(&mut harness, "Dragging a window", "Live", "Ghost");
    assert_eq!(harness.app().prefs.drag, DragStyle::Ghost);
    assert!(harness.screen().contains("a shaded area follows the mouse"), "{}", harness.screen());
}

#[test]
fn the_drag_style_is_changed_from_the_keyboard_too() {
    let prefs = Prefs { drag: DragStyle::Live, ..Prefs::default() };
    let mut harness = screen_of(prefs, false, Vec::new(), 100, 30);
    // The list has the keyboard from the start; ↓ walks its rows, and the keyboard's row is
    // raised with the pillar at its edge. As many presses as the list has rows, at most.
    let reached = |harness: &Harness<SettingsApp>| {
        harness.screen().lines().any(|line| line.trim_start().starts_with('▌') && line.contains("Dragging a window"))
    };
    for _ in 0..40 {
        if reached(&harness) {
            break;
        }
        harness.press("down");
    }
    assert!(reached(&harness), "the keys reach the row:\n{}", harness.screen());

    harness.press("enter").press("down").press("enter");

    assert_eq!(harness.app().prefs.drag, DragStyle::Ghost, "{}", harness.screen());
    assert!(harness.screen().contains("a shaded area follows the mouse"), "{}", harness.screen());
}

#[test]
fn the_dock_moves_to_the_edge_that_was_chosen() {
    let mut harness = screen(80, 24);
    scrolled_to(&mut harness, "Dock");
    support::click_on_row(&mut harness, "Dock", "Top");
    assert_eq!(harness.app().prefs.dock, DockPosition::Top);
    support::click_on_row(&mut harness, "Dock", "Bottom");
    assert_eq!(harness.app().prefs.dock, DockPosition::Bottom);
}

#[test]
fn a_chosen_frame_cap_opens_a_field_under_it_and_is_held_inside_its_range() {
    let mut harness = screen_of(Prefs::default(), true, Vec::new(), 100, 30);
    let shown = scrolled_to(&mut harness, "at most 20 times a second");
    assert!(shown.contains("at most 20 times a second"), "a remote link caps at 20:\n{shown}");
    assert!(support::on_row(&harness, "Frames", "").is_none(), "no field while it follows the link:\n{shown}");

    support::click_on_row(&mut harness, "Frames a second", "A number");

    assert_eq!(harness.app().prefs.frame_cap, Some(20), "the number starts at what the link gave");
    let shown = scrolled_to(&mut harness, "Remembered lines");
    let field = support::on_row(&harness, "Frames", "20");
    assert!(field.is_some(), "the field for the number is there, holding it:\n{shown}");

    // A number typed in the field is the one in force.
    let (x, y) = field.expect("the field");
    harness.click(x, y).press("ctrl+a").type_text("45");
    assert_eq!(harness.app().prefs.frame_cap, Some(45));
    assert!(harness.screen().contains("at most 45 times a second"), "{}", harness.screen());

    // The field never gives a number outside the range: a number past it is not taken, and the
    // keys stop at its ends.
    harness.press("ctrl+a").type_text("999");
    assert_eq!(harness.app().prefs.frame_cap, Some(99), "a fourth digit past the range is not taken");
    for _ in 0..30 {
        harness.press("pgup");
    }
    assert_eq!(harness.app().prefs.frame_cap, Some(240), "the keys stop at the top of the range");
    harness.press("ctrl+a").type_text("1").press("down");
    assert_eq!(harness.app().prefs.frame_cap, Some(1), "never nought frames a second");

    support::click_on_row(&mut harness, "Frames a second", "Automatic");
    assert_eq!(harness.app().prefs.frame_cap, None, "it can follow the link again");
    let shown = harness.screen();
    assert!(support::on_row(&harness, "Frames", "").is_none(), "the field goes with the number:\n{shown}");
    assert!(shown.contains("at most 20 times a second"), "{shown}");
}

#[test]
fn the_scrollback_is_held_inside_its_range() {
    let mut harness = screen(80, 24);
    let shown = scrolled_to(&mut harness, "Remembered lines");
    let (x, y) = support::on_row(&harness, "Remembered lines", "2000")
        .unwrap_or_else(|| panic!("the field holds the default:\n{shown}"));
    harness.click(x, y).press("ctrl+a").type_text("0");
    assert_eq!(harness.app().prefs.scrollback, 0);
    harness.press("ctrl+a").type_text("10000");
    assert_eq!(harness.app().prefs.scrollback, 10_000);
    support::click_on_row(&mut harness, "Remembered lines", "+");
    assert_eq!(harness.app().prefs.scrollback, 10_000, "more than ten thousand lines is not offered");
    harness.click(x, y).press("ctrl+a").type_text("99999");
    assert_eq!(harness.app().prefs.scrollback, 9_999, "a fifth digit past the range is not taken");
}

/// The screen holds a number that reaches it by a message inside its range too, wherever the
/// message came from: this tests that clamp alone. The fields that send these messages are
/// driven by the tests above.
#[test]
fn a_number_sent_past_its_range_is_held_at_the_range() {
    let prefs = Prefs::default();
    let held = |msg| match settings::update::<Msg>(&mut Screen::default(), &prefs, msg).1 {
        Some(settings::Request::Prefs(prefs)) => prefs,
        other => panic!("no preferences: {other:?}"),
    };
    assert_eq!(held(Msg::FrameCap(Some(10_000))).frame_cap, Some(240), "a number past the range stops at it");
    assert_eq!(held(Msg::FrameCap(Some(0))).frame_cap, Some(1), "never nought frames a second");
    assert_eq!(held(Msg::Scrollback(u16::MAX)).scrollback, 10_000, "more than ten thousand lines is not offered");
}

#[test]
fn an_entry_that_could_not_be_read_is_shown_with_its_file_line_and_column() {
    let mut harness = screen_of(Prefs::default(), false, problems(), 120, 40);
    let shown = scrolled_to(&mut harness, "logs.toml");
    assert!(shown.contains("should hold two whole numbers"), "the problem is a sentence:\n{shown}");
    assert!(shown.contains("htop.toml:3:8"), "with the place in the file:\n{shown}");
    assert!(shown.contains("This entry has no name"), "{shown}");
    assert!(shown.contains("logs.toml"), "a problem without a place still names its file:\n{shown}");
}

#[test]
fn an_entry_that_could_not_be_read_is_shown_in_turkish_too() {
    let mut harness = screen_of(Prefs::default(), false, problems(), 120, 40);
    harness.set_locale("tr").render();
    let shown = scrolled_to(&mut harness, "Bu girdinin adı yok");
    assert!(shown.contains("iki tam sayı"), "{shown}");
    assert!(shown.contains("Bu girdinin adı yok"), "{shown}");
}

#[test]
fn with_nothing_to_report_the_applications_section_says_so_instead_of_staying_empty() {
    let mut harness = screen_of(Prefs::default(), false, Vec::new(), 120, 40);
    let shown = scrolled_to(&mut harness, "Every entry was read");
    assert!(shown.contains("Every entry was read"), "{shown}");
    assert!(!shown.contains("could not be read and are not shown"), "{shown}");
    assert_eq!(decoration(&shown), None, "{shown}");
}

#[test]
fn a_settings_file_that_could_not_be_read_is_said_above_the_settings_and_can_be_put_away() {
    let broken = vec![
        Diagnostic::at(
            "/home/deniz/.config/quvyta/desktop.conf",
            Some(Position { line: 2, column: 1 }),
            DiagnosticKind::Syntax,
        )
        .with_detail("`scrollback` must be a whole number from 0 to 10000, found \"many\"; it is ignored"),
    ];
    let (appearance, stored) = appearance();
    let app = SettingsApp {
        screen: Screen::new(broken),
        prefs: Prefs::default(),
        remote: false,
        folders: folders(),
        diagnostics: Vec::new(),
        appearance,
        stored,
    };
    let mut harness = Harness::with_env(app, env(), 120, 40);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);

    // What a person sees the moment the screen opens, before any scrolling: the list takes the
    // keyboard then, and the notice above it must stay in view with the way to put it away.
    let shown = harness.screen();
    assert!(shown.contains("Part of the settings file could not be read"), "{shown}");
    assert!(shown.contains("desktop.conf:2:1"), "{shown}");
    assert!(shown.contains("must be a whole number"), "the framework's own words are kept:\n{shown}");
    assert!(shown.contains("Understood"), "the way to put it away is in view:\n{shown}");
    assert!(harness.is_focused(settings::READ), "the keyboard is on it");

    // Enter on the button puts the notice away, and the keys go back to the list.
    harness.press("enter");

    let after = harness.screen();
    assert!(!after.contains("Part of the settings file could not be read"), "{after}");
    assert!(after.contains("Appearance"), "the settings are still there:\n{after}");
    assert!(harness.is_focused(settings::LIST), "the list has the keyboard again");
}

#[test]
fn a_failed_write_is_said_and_taken_back_when_one_succeeds() {
    let mut harness = screen(100, 30);
    harness.send(Msg::Stored(Err("no space left on device".to_owned())));
    let shown = harness.screen();
    assert!(shown.contains("could not be written"), "{shown}");
    assert!(shown.contains("no space left on device"), "{shown}");

    harness.send(Msg::Stored(Ok(())));
    assert!(!harness.screen().contains("could not be written"), "{}", harness.screen());
}

#[test]
fn a_change_on_the_appearance_section_is_handed_to_it_and_changes_the_whole_screen() {
    let mut screen_state = Screen::default();
    let prefs = Prefs::default();
    let change = AppearanceChange::Theme("nordic".to_owned());
    let (_command, request) = settings::update::<Msg>(&mut screen_state, &prefs, Msg::Appearance(change.clone()));
    assert_eq!(request, Some(settings::Request::Appearance(change)));

    // A screen that shows no update notice never passes a change of it on.
    let off = Msg::Appearance(AppearanceChange::UpdateNotice(false));
    assert_eq!(settings::update::<Msg>(&mut screen_state, &prefs, off.clone()).1, None);
    let mut shown = Screen::default().with_updates(true);
    assert!(settings::update::<Msg>(&mut shown, &prefs, off).1.is_some());

    let mut harness = screen(80, 24);
    support::pick(&mut harness, "Language", "English", "Türkçe");
    assert!(harness.screen().contains("Görünüm"), "the language changes the screen at once:\n{}", harness.screen());
    assert!(harness.screen().contains("Renk teması"), "{}", harness.screen());
}

#[test]
fn the_numbers_in_force_are_readable_in_their_fields() {
    let prefs = Prefs { frame_cap: Some(45), scrollback: 500, ..Prefs::default() };
    let mut harness = screen_of(prefs, false, Vec::new(), 100, 30);
    let shown = scrolled_to(&mut harness, "Remembered lines");
    assert!(shown.contains("500"), "the scrollback field shows its number:\n{shown}");
    assert!(shown.contains("45"), "the frame cap field shows its number:\n{shown}");
}

#[test]
fn the_largest_numbers_are_whole_in_their_fields() {
    // The widest numbers the fields take: nothing of them is cut away behind the steppers.
    let prefs = Prefs { frame_cap: Some(60), scrollback: 10_000, ..Prefs::default() };
    let mut harness = screen_of(prefs, false, Vec::new(), 100, 30);
    let shown = scrolled_to(&mut harness, "Remembered lines");
    assert!(shown.contains("10000"), "the scrollback field shows all five digits:\n{shown}");
    let lines = shown.lines().find(|line| line.contains("Remembered lines")).unwrap_or_default();
    assert!(!lines.contains('…'), "the row is not cut: {lines}");
}

/// A fresh folder under the system's temporary folder for one test's settings file; never the
/// person's own config folder.
fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qdesk-test-floor-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("folder");
    dir
}

/// Opens the Settings window from its icon on the floor.
fn open_settings(harness: &mut Harness<qdesk::app::Desk>) {
    let (x, y) = harness.find("Settings").expect("the Settings icon is on the floor");
    harness.click(x, y);
    harness.click(x, y);
    assert_eq!(harness.app().windows().len(), 1, "the window opened:\n{}", harness.screen());
}

/// A cell of the floor that no window covers and no icon stands on: the far corner above the
/// dock, or the one below the window when the window reaches it.
fn floor_cell(harness: &Harness<qdesk::app::Desk>, width: u16, height: u16) -> (u16, u16) {
    let corner = (width - 1, height - 2);
    let covered =
        harness.app().windows().iter().any(|window| window.rect().contains(i32::from(corner.0), i32::from(corner.1)));
    assert!(!covered, "the corner is free of windows:\n{}", harness.screen());
    corner
}

#[test]
fn the_floor_takes_the_colour_clicked_on_the_settings_screen_and_keeps_it_after_a_restart() {
    let config = folder("restart");
    let (width, height) = (140, 44);
    let mut harness = support::desk_in(&config, width, height);
    let theme = harness.env().theme();
    let canvas = theme.color("canvas");
    let deep = FloorColor::Deep.in_theme(theme).expect("the theme has a canvas");
    assert_ne!(Some(deep), canvas, "the chosen tone is not the theme's own");
    let dock_before = harness.bg(width / 2, height - 1);

    open_settings(&mut harness);
    let (x, y) = floor_cell(&harness, width, height);
    assert_eq!(harness.bg(x, y), canvas, "the floor starts in the theme's canvas");
    harness.click_text("Deep");
    assert_eq!(harness.app().prefs().floor, FloorColor::Deep);
    assert_eq!(harness.bg(x, y), Some(deep), "the floor changes at once:\n{}", harness.screen());
    assert_eq!(harness.bg(width / 2, height - 1), dock_before, "the dock keeps the theme");
    let written = std::fs::read_to_string(config.join("desktop.conf")).expect("the settings file is written");
    assert!(written.contains("floor-color = \"deep\""), "{written}");

    // The next run reads it back from the file.
    let again = support::desk_in(&config, width, height);
    assert_eq!(again.app().prefs().floor, FloorColor::Deep);
    assert_eq!(again.bg(x, y), Some(deep), "after a restart the floor is still deep:\n{}", again.screen());
    let _ = std::fs::remove_dir_all(&config);
}

#[test]
fn a_floor_colour_the_file_does_not_know_is_said_and_the_theme_stays() {
    let config = folder("unknown");
    std::fs::write(config.join("desktop.conf"), "floor-color = \"plaid\"\n").expect("file");
    let loaded = settings::load_in(&config);
    assert_eq!(loaded.prefs.floor, FloorColor::Theme);
    assert_eq!(loaded.diagnostics.len(), 1, "{:?}", loaded.diagnostics);
    assert!(loaded.diagnostics[0].location().contains("desktop.conf:1"), "{:?}", loaded.diagnostics);
    let harness = support::desk_in(&config, 100, 30);
    let canvas = harness.env().theme().color("canvas");
    assert_eq!(harness.bg(99, 28), canvas, "{}", harness.screen());
    let _ = std::fs::remove_dir_all(&config);
}

#[test]
fn the_floor_colour_is_chosen_on_the_desktop_section_in_both_languages() {
    let mut harness = screen(100, 30);
    let shown = harness.screen();
    for text in ["Floor", "Theme", "Deep", "Mist", "Accent"] {
        assert!(shown.contains(text), "no `{text}`:\n{shown}");
    }
    harness.set_locale("tr").render();
    let shown = harness.screen();
    for text in ["Zemin", "Derin", "Sis", "Vurgu"] {
        assert!(shown.contains(text), "no `{text}`:\n{shown}");
    }
    assert_eq!(decoration(&shown), None, "{shown}");
}

#[test]
fn settings_opened_on_the_desktop_over_an_unreadable_file_show_what_it_could_not_read_at_once() {
    let config = folder("unreadable");
    std::fs::write(config.join("desktop.conf"), "scrollback = \"many\"\n").expect("file");
    let mut harness = support::desk_in(&config, 140, 44);

    open_settings(&mut harness);

    // What a person sees the moment the window opens, before any scrolling.
    let shown = harness.screen();
    assert!(shown.contains("Part of the settings file could not be read"), "{shown}");
    assert!(shown.contains("Understood"), "the way to put it away is in view:\n{shown}");
    harness.click_text("Understood");
    let after = harness.screen();
    assert!(!after.contains("Part of the settings file could not be read"), "{after}");
    assert!(after.contains("Appearance"), "the settings are still there:\n{after}");
    assert!(harness.is_focused(settings::LIST), "the list has the keyboard again");
    let _ = std::fs::remove_dir_all(&config);
}
