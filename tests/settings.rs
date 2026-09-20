//! The Settings screen: its sections at several sizes, glyph modes and colour depths, an option
//! changed, and the applications section with and without problems to report.

use std::path::PathBuf;

use qdesk::apps::{Diagnostic, DiagnosticKind, Expected, Folders, Position};
use qdesk::settings::{self, Applications, DockPosition, DragStyle, Msg, Prefs, Screen, Shared};
use qframe::color::ColorDepth;
use qframe::env::{AssetDirs, Env};
use qframe::icons::{GlyphMode, IconMode};
use qframe::prelude::*;

/// The Settings screen with the state a test gives it, in an application of its own: the screen
/// is a view, and the desktop that will hold it arrives with the windows.
struct SettingsApp {
    screen: Screen,
    prefs: Prefs,
    remote: bool,
    folders: Folders,
    diagnostics: Vec<Diagnostic>,
}

impl App for SettingsApp {
    type Msg = Msg;

    fn update(&mut self, msg: Msg) -> Command<Msg> {
        let (command, request) = settings::update(&mut self.screen, &self.prefs, msg);
        match request {
            Some(settings::Request::Prefs(prefs)) => self.prefs = prefs,
            Some(settings::Request::Shared(_)) | None => {}
        }
        command
    }

    fn view(&self, ui: &mut View<'_, Msg>) {
        let apps = Applications { folders: &self.folders, diagnostics: &self.diagnostics };
        settings::view(&self.screen, &self.prefs, self.remote, &apps, ui);
    }
}

fn env() -> Env {
    let dirs = AssetDirs {
        locale_sources: qdesk::locales().iter().map(|(file, text)| ((*file).to_owned(), (*text).to_owned())).collect(),
        ..AssetDirs::default()
    };
    Env::load(&dirs).expect("the built-in files load")
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
    let app = SettingsApp { screen: Screen::default(), prefs, remote, folders: folders(), diagnostics };
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
    let harness = screen(80, 24);
    let shown = harness.screen();
    for heading in ["Appearance", "Language", "Theme", "Glyphs", "Desktop", "Dock", "Connection"] {
        assert!(shown.contains(heading), "no `{heading}`:\n{shown}");
    }
    assert_eq!(decoration(&shown), None, "nothing is drawn with lines or brackets:\n{shown}");
}

#[test]
fn a_tall_terminal_shows_the_connection_and_the_applications_as_well() {
    let harness = screen(200, 50);
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
    for text in ["Görünüş", "Dil", "Tema", "Masaüstü", "Bağlantı"] {
        assert!(shown.contains(text), "no `{text}`:\n{shown}");
    }
}

#[test]
fn a_changed_drag_style_is_shown_at_once_with_what_it_comes_to_here() {
    let mut harness = screen(80, 24);
    let before = harness.screen();
    assert!(before.contains("a shaded area follows the mouse"), "ghost is the default everywhere:\n{before}");

    harness.send(Msg::Drag(DragStyle::Live));

    assert_eq!(harness.app().prefs.drag, DragStyle::Live, "the change is applied, not only drawn");
    let shown = harness.screen();
    assert!(shown.contains("Live"), "{shown}");
    assert!(shown.contains("the window itself follows the mouse"), "the line says what it comes to:\n{shown}");
}

#[test]
fn the_dock_moves_to_the_edge_that_was_chosen() {
    let mut harness = screen(80, 24);
    harness.send(Msg::Dock(DockPosition::Top));
    assert_eq!(harness.app().prefs.dock, DockPosition::Top);
    harness.send(Msg::Dock(DockPosition::Bottom));
    assert_eq!(harness.app().prefs.dock, DockPosition::Bottom);
}

#[test]
fn a_chosen_frame_cap_opens_a_field_under_it_and_is_held_inside_its_range() {
    let mut harness = screen_of(Prefs::default(), true, Vec::new(), 100, 30);
    assert!(harness.screen().contains("at most 20 times a second"), "a remote link caps at 20:\n{}", harness.screen());

    harness.send(Msg::FrameCap(Some(45)));

    assert_eq!(harness.app().prefs.frame_cap, Some(45));
    let shown = harness.screen();
    assert!(shown.contains("Frames"), "the field for the number is there:\n{shown}");
    assert!(shown.contains("at most 45 times a second"), "{shown}");

    harness.send(Msg::FrameCap(Some(10_000)));
    assert_eq!(harness.app().prefs.frame_cap, Some(240), "a number past the range stops at the range");
    harness.send(Msg::FrameCap(Some(0)));
    assert_eq!(harness.app().prefs.frame_cap, Some(1), "never nought frames a second");
    harness.send(Msg::FrameCap(None));
    assert_eq!(harness.app().prefs.frame_cap, None, "it can follow the link again");
}

#[test]
fn the_scrollback_is_held_inside_its_range() {
    let mut harness = screen(80, 24);
    harness.send(Msg::Scrollback(0));
    assert_eq!(harness.app().prefs.scrollback, 0);
    harness.send(Msg::Scrollback(10_000));
    assert_eq!(harness.app().prefs.scrollback, 10_000);
    harness.send(Msg::Scrollback(u16::MAX));
    assert_eq!(harness.app().prefs.scrollback, 10_000, "more than ten thousand lines is not offered");
}

#[test]
fn an_entry_that_could_not_be_read_is_shown_with_its_file_line_and_column() {
    let harness = screen_of(Prefs::default(), false, problems(), 120, 40);
    let shown = harness.screen();
    assert!(shown.contains("should hold two whole numbers"), "the problem is a sentence:\n{shown}");
    assert!(shown.contains("htop.toml:3:8"), "with the place in the file:\n{shown}");
    assert!(shown.contains("This entry has no name"), "{shown}");
    assert!(shown.contains("logs.toml"), "a problem without a place still names its file:\n{shown}");
}

#[test]
fn an_entry_that_could_not_be_read_is_shown_in_turkish_too() {
    let mut harness = screen_of(Prefs::default(), false, problems(), 120, 40);
    harness.set_locale("tr").render();
    let shown = harness.screen();
    assert!(shown.contains("iki tam sayı"), "{shown}");
    assert!(shown.contains("Bu girdinin adı yok"), "{shown}");
}

#[test]
fn with_nothing_to_report_the_applications_section_says_so_instead_of_staying_empty() {
    let harness = screen_of(Prefs::default(), false, Vec::new(), 120, 40);
    let shown = harness.screen();
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
    let app = SettingsApp {
        screen: Screen::new(broken),
        prefs: Prefs::default(),
        remote: false,
        folders: folders(),
        diagnostics: Vec::new(),
    };
    let mut harness = Harness::with_env(app, env(), 120, 40);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);

    let shown = harness.screen();
    assert!(shown.contains("Part of the settings file could not be read"), "{shown}");
    assert!(shown.contains("desktop.conf:2:1"), "{shown}");
    assert!(shown.contains("must be a whole number"), "the framework's own words are kept:\n{shown}");

    harness.send(Msg::ReadProblems);

    let after = harness.screen();
    assert!(!after.contains("Part of the settings file could not be read"), "{after}");
    assert!(after.contains("Appearance"), "the settings are still there:\n{after}");
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
fn a_shared_setting_is_applied_to_the_whole_screen_and_handed_back_to_be_stored() {
    let mut screen_state = Screen::default();
    let prefs = Prefs::default();
    let (_command, request) =
        settings::update::<Msg>(&mut screen_state, &prefs, Msg::Shared(Shared::Icons(IconMode::Ascii)));
    assert_eq!(request, Some(settings::Request::Shared(Shared::Icons(IconMode::Ascii))));

    let mut harness = screen(80, 24);
    harness.send(Msg::Shared(Shared::Language("tr".to_owned())));
    assert!(harness.screen().contains("Görünüş"), "the language changes the screen at once:\n{}", harness.screen());
}

#[test]
fn the_numbers_in_force_are_readable_in_their_fields() {
    let prefs = Prefs { frame_cap: Some(45), scrollback: 500, ..Prefs::default() };
    let harness = screen_of(prefs, false, Vec::new(), 100, 30);
    let shown = harness.screen();
    assert!(shown.contains("500"), "the scrollback field shows its number:\n{shown}");
    assert!(shown.contains("45"), "the frame cap field shows its number:\n{shown}");
}
