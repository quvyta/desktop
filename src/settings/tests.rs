//! What the settings hold, what they refuse, what they write, and that every problem has a
//! sentence in both languages. Nothing here touches the machine's own settings: every test works
//! in a folder of its own under the system's temporary folder.

use std::fs;
use std::path::{Path, PathBuf};

use super::*;
use crate::apps::Expected;

/// A fresh folder under the system's temporary folder; never the person's own config folder.
fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("qdesk-test-settings-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("folder");
    dir
}

fn write(folder: &Path, text: &str) {
    fs::write(folder.join(format!("{APP}.conf")), text).expect("file");
}

fn read(folder: &Path) -> String {
    fs::read_to_string(folder.join(format!("{APP}.conf"))).expect("readable")
}

/// The settings `text` holds, read the way the desktop reads them.
fn parse(text: &str) -> Loaded {
    super::read(Settings::parse_str(&format!("{APP}.conf"), text).member_of(&Family::QUVYTA))
}

#[test]
fn the_defaults_are_the_ones_the_design_settled_on() {
    let prefs = Prefs::default();
    assert_eq!(prefs.dock, DockPosition::Bottom, "a task bar is looked for at the bottom");
    assert_eq!(prefs.drag, DragStyle::Ghost, "a ghost is the default everywhere, local or remote");
    assert_eq!(prefs.frame_cap, None, "the cap follows the link until a number is chosen");
    assert_eq!(prefs.scrollback, 2_000);
    assert_eq!(Prefs::from_settings(&Settings::in_memory()), prefs, "an empty file is the defaults");
}

#[test]
fn the_frame_cap_follows_the_link_until_a_number_is_chosen() {
    assert_eq!(frame_cap(None, false), 60);
    assert_eq!(frame_cap(None, true), 20);
    assert_eq!(frame_cap(Some(30), false), 30);
    assert_eq!(frame_cap(Some(30), true), 30, "a chosen number stands over the link");
}

#[test]
fn a_frame_cap_outside_the_range_is_brought_into_it() {
    assert_eq!(frame_cap(Some(0), false), FRAME_CAP_LEAST, "never nought frames a second");
    assert_eq!(frame_cap(Some(FRAME_CAP_MOST + 1), false), FRAME_CAP_MOST);
    assert_eq!(frame_cap(Some(u16::MAX), true), FRAME_CAP_MOST);
    assert_eq!(frame_cap(Some(FRAME_CAP_LEAST), false), FRAME_CAP_LEAST);
    assert_eq!(frame_cap(Some(FRAME_CAP_MOST), false), FRAME_CAP_MOST);
}

#[test]
fn a_number_the_file_does_not_allow_leaves_the_setting_at_its_default() {
    let loaded = parse("scrollback = 10001\nframe-cap = 0\n");
    assert_eq!(loaded.prefs.scrollback, 2_000, "over ten thousand lines is not accepted");
    assert_eq!(loaded.prefs.frame_cap, None, "nought frames a second is not a cap");
    assert_eq!(loaded.diagnostics.len(), 2, "{:?}", loaded.diagnostics);

    let edges = parse("scrollback = 0\nframe-cap = 240\n");
    assert!(edges.diagnostics.is_empty(), "{:?}", edges.diagnostics);
    assert_eq!(edges.prefs.scrollback, 0, "remembering no lines is a choice");
    assert_eq!(edges.prefs.frame_cap, Some(240));

    let negative = parse("scrollback = -1\nframe-cap = -60\n");
    assert_eq!(negative.prefs.scrollback, 2_000);
    assert_eq!(negative.prefs.frame_cap, None);
}

#[test]
fn a_name_the_desktop_does_not_know_leaves_the_setting_at_its_default() {
    let loaded = parse("dock-position = \"left\"\ndrag-style = \"instant\"\n");
    assert_eq!(loaded.prefs.dock, DockPosition::Bottom);
    assert_eq!(loaded.prefs.drag, DragStyle::Ghost);
    assert_eq!(loaded.diagnostics.len(), 2, "{:?}", loaded.diagnostics);
    let names: Vec<String> = loaded.diagnostics.iter().map(Diagnostic::location).collect();
    assert_eq!(names, ["desktop.conf:1:1", "desktop.conf:2:1"], "each problem points at its own line");
}

#[test]
fn a_file_that_still_says_automatic_does_not_panic_and_falls_back_to_the_default() {
    // `automatic` was a real value before this style lost the connection split. qdesk has never
    // been released, so no file is migrated, but an old file must still open without a panic.
    let loaded = parse("drag-style = \"automatic\"\n");
    assert_eq!(loaded.prefs.drag, DragStyle::Ghost, "an old value is unusable, not special-cased");
    assert_eq!(loaded.diagnostics.len(), 1, "{:?}", loaded.diagnostics);
    assert_eq!(loaded.diagnostics[0].location(), "desktop.conf:1:1");
}

#[test]
fn every_name_the_settings_write_is_a_name_they_read_back() {
    for side in DockPosition::ALL {
        assert_eq!(DockPosition::from_name(side.name()), Some(side));
    }
    for style in DragStyle::ALL {
        assert_eq!(DragStyle::from_name(style.name()), Some(style));
    }
    assert_eq!(DockPosition::from_name("Bottom"), None, "the file's names are lower case");
    assert_eq!(DragStyle::from_name(""), None);
}

#[test]
fn a_file_that_cannot_be_read_at_all_becomes_one_diagnostic_and_the_desktop_runs_on_defaults() {
    let loaded = parse("scrollback = 500\nthis is not a setting at all\ndock-position = \"top\"\n");
    assert_eq!(loaded.diagnostics.len(), 1, "{:?}", loaded.diagnostics);
    let problem = &loaded.diagnostics[0];
    assert_eq!(problem.kind, DiagnosticKind::Syntax);
    assert!(!problem.is_warning(), "a file that cannot be read is not a warning");
    assert!(problem.detail.is_some(), "the parser's own words are kept");
    assert_eq!(problem.position.map(|at| at.line), Some(2), "the mistake is on the second line");
    assert_eq!(loaded.prefs.scrollback, 500, "what was read before the mistake still counts");
    assert_eq!(loaded.prefs.dock, DockPosition::Bottom, "what comes after it is the default");
    assert_eq!(loaded.prefs.drag, Prefs::default().drag);
}

#[test]
fn a_key_from_another_version_is_kept_in_the_file_rather_than_lost() {
    let folder = temp("unknown-keys");
    let text = "dock-position = \"top\"\n\n[windows]\nremember = true\n";
    write(&folder, text);

    let mut loaded = load_in(&folder);

    assert_eq!(loaded.prefs.dock, DockPosition::Top, "what this version knows is used");
    assert_eq!(loaded.diagnostics.len(), 1, "the key it does not know is said out loud: {:?}", loaded.diagnostics);
    loaded.prefs.write(&mut loaded.settings);
    loaded.settings.save().expect("saved");

    let after = read(&folder);
    assert!(after.contains("remember = true"), "the key of another version is still there:\n{after}");
    assert!(after.contains("dock-position = \"top\""), "{after}");
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn saving_writes_what_was_chosen_and_nothing_that_is_the_default() {
    let folder = temp("only-chosen");

    let mut loaded = load_in(&folder);
    assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
    assert!(!folder.join(format!("{APP}.conf")).exists(), "a first start writes nothing");

    Prefs::default().write(&mut loaded.settings);
    loaded.settings.save().expect("saved");
    assert_eq!(read(&folder).trim(), "", "the defaults are not written");

    let chosen = Prefs { scrollback: 500, drag: DragStyle::Live, ..Prefs::default() };
    chosen.write(&mut loaded.settings);
    loaded.settings.save().expect("saved");
    let text = read(&folder);
    assert!(text.contains("scrollback = 500"), "{text}");
    assert!(text.contains("drag-style = \"live\""), "{text}");
    assert!(!text.contains("dock-position"), "the dock is where it always was:\n{text}");
    assert!(!text.contains("frame-cap"), "the cap still follows the link:\n{text}");

    // Going back to a default takes the key out again instead of writing the default down.
    Prefs { scrollback: 500, ..Prefs::default() }.write(&mut loaded.settings);
    loaded.settings.save().expect("saved");
    let text = read(&folder);
    assert!(!text.contains("drag-style"), "{text}");
    assert!(text.contains("scrollback = 500"), "{text}");
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn two_saves_in_a_row_leave_a_file_that_reads_back_the_same() {
    let folder = temp("twice");
    let chosen = Prefs { dock: DockPosition::Top, drag: DragStyle::Live, frame_cap: Some(45), scrollback: 0 };

    let mut loaded = load_in(&folder);
    chosen.write(&mut loaded.settings);
    loaded.settings.save().expect("saved");
    let once = read(&folder);
    loaded.settings.save().expect("saved again");
    assert_eq!(read(&folder), once, "the second save writes the same file");

    let again = load_in(&folder);
    assert!(again.diagnostics.is_empty(), "{:?}", again.diagnostics);
    assert_eq!(again.prefs, chosen);
    let _ = fs::remove_dir_all(&folder);
}

#[test]
fn a_file_without_a_folder_keeps_the_settings_in_memory_and_saving_does_nothing() {
    let loaded = super::read(Settings::in_memory());
    assert_eq!(loaded.prefs, Prefs::default());
    assert!(loaded.diagnostics.is_empty());
    assert_eq!(loaded.settings.path(), None);
}

/// One of every kind of problem, so a kind added later has to be named here too.
fn every_kind() -> Vec<DiagnosticKind> {
    vec![
        DiagnosticKind::Unreadable,
        DiagnosticKind::NotUtf8,
        DiagnosticKind::Syntax,
        DiagnosticKind::WrongType { key: "window.size".to_owned(), expected: Expected::Size },
        DiagnosticKind::InvalidValue { key: "name".to_owned() },
        DiagnosticKind::MissingName,
        DiagnosticKind::MissingLaunch,
        DiagnosticKind::ConflictingLaunch,
        DiagnosticKind::UnknownCategory { value: "games".to_owned() },
        DiagnosticKind::UnknownField { key: "colour".to_owned() },
        DiagnosticKind::NoHome { key: "open".to_owned() },
        DiagnosticKind::MalformedLine,
        DiagnosticKind::MissingGroup,
        DiagnosticKind::MissingKey { key: "Exec".to_owned() },
        DiagnosticKind::BadExec,
    ]
}

/// Every kind of problem and every kind of wanted value says something in each language: the
/// screen shows [`crate::notice::what`], so the words are checked where they are read from.
#[test]
fn every_problem_has_a_sentence_in_every_language() {
    use qframe::i18n::{I18n, scope};
    use std::sync::Arc;

    for (file, _) in crate::locales() {
        let code = file.trim_end_matches(".toml");
        let mut i18n = I18n::builtin();
        for (name, text) in crate::locales() {
            assert!(i18n.add_source(name, text), "{name} loads");
        }
        assert!(i18n.set_active(code), "{code} is a language of qdesk");
        scope(Arc::new(i18n), || {
            let mut said = Vec::new();
            for kind in every_kind() {
                let words = crate::notice::what(&kind);
                assert!(!words.starts_with('\u{27e6}'), "{kind:?} has no words in {code}: {words}");
                assert!(!said.contains(&words), "{kind:?} says the same as another kind in {code}");
                said.push(words);
            }
            for wanted in [
                Expected::String,
                Expected::Boolean,
                Expected::Table,
                Expected::Text,
                Expected::StringList,
                Expected::Size,
            ] {
                let words = crate::notice::expected(wanted);
                assert!(!words.starts_with('\u{27e6}'), "{wanted:?} has no words in {code}: {words}");
            }
        });
    }
}
