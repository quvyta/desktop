//! The ecosystem's update notice: qdesk asks once at start whether a newer version is out while the
//! ecosystem's switch is on, says so in the corner when one is, and the switch on the Settings
//! screen turns the question off for every Quvyta application.
//!
//! Every test gives the desktop folders of its own under the system's temporary folder, so none
//! reads or turns off the person's own switch; the harness answers the question itself and never
//! reaches the network.

mod support;

use std::path::PathBuf;

use qdesk::app::Desk;
use qdesk::apps::Environment;
use qdesk::desktop::Desktop;
use qdesk::settings::UpdateFolders;
use qframe::env::{AssetDirs, Env};
use qframe::icons::GlyphMode;
use qframe::prelude::*;
use qframe::storage::Family;

use support::{HARMLESS, ICONS, MACHINE, MOMENT, OFFSET, PATIENCE, catalog};

/// A terminal tall enough for the whole Appearance section of the Settings window.
const SIZE: (u16, u16) = (140, 60);

/// A version newer than any qdesk will be for a long while, and not the one the tests run.
const NEWER: &str = "9.4.7";

/// The ecosystem's folders of a test named `what`, empty, so the notice starts on as it does on a
/// machine that never chose.
fn ecosystem(what: &str) -> UpdateFolders {
    let root = std::env::temp_dir().join(format!("qdesk-updates-{what}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    UpdateFolders { config: root.join("config"), state: root.join("state") }
}

/// Removes the folders of `folders`.
fn clean(folders: &UpdateFolders) {
    let root: PathBuf = folders.config.parent().expect("the ecosystem's root").to_path_buf();
    let _ = std::fs::remove_dir_all(root);
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

/// qdesk started over the ecosystem's `folders`, as a person starts it, with the usual floor.
fn started(folders: Option<UpdateFolders>) -> Harness<Desk> {
    let desktop = Desktop {
        icons: ICONS.iter().map(|id| (*id).to_owned()).collect(),
        recents: Vec::new(),
        welcome_seen: true,
        ..Desktop::default()
    };
    let apps = Environment { shell: Some(PathBuf::from(HARMLESS)), ..Environment::default() };
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), Box::new(|| MOMENT * 1_000))
        .apps(apps)
        .catalog(catalog())
        .desktop(desktop)
        .update_notice(folders)
        .watch_within(PATIENCE);
    let mut harness = Harness::with_env(app, env(), SIZE.0, SIZE.1);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);
    harness.render();
    harness
}

/// Opens the Settings window from its icon on the floor, as a person does.
fn open_settings(harness: &mut Harness<Desk>) {
    let (x, y) = harness.find("Settings").expect("the Settings icon is on the floor");
    harness.click(x, y);
    harness.click(x, y);
    assert_eq!(harness.app().windows().len(), 1, "the window opened:\n{}", harness.screen());
}

/// Moves the switch of the ecosystem's update notice where it is drawn: a switch is colour alone
/// and stands at the right edge of the row, where the language drop-down's arrow stands too.
fn click_update_notice(harness: &mut Harness<Desk>) {
    open_settings(harness);
    let (_, row) = harness.find("Say when an update is out").expect("the switch is on screen");
    let (edge, _) = harness.find("▾").expect("the language drop-down");
    harness.click(edge - 1, row);
}

#[test]
fn a_newer_version_is_asked_for_once_and_said_in_the_corner() {
    let folders = ecosystem("newer");
    let mut harness = started(Some(folders.clone()));
    let asked = harness.update_checks().to_vec();
    assert_eq!(asked.len(), 1, "qdesk asks once at start");
    assert_eq!((asked[0].package(), asked[0].current()), ("quvyta-desktop", env!("CARGO_PKG_VERSION")));
    assert!(!harness.screen().contains("is out"), "nothing is said before the answer:\n{}", harness.screen());

    harness.set_latest_version(Some(NEWER));
    let screen = harness.screen();
    assert!(screen.contains(&format!("quvyta-desktop {NEWER} is out")), "the notice names the new version:\n{screen}");
    assert!(screen.contains(env!("CARGO_PKG_VERSION")), "and the one running:\n{screen}");
    assert_eq!(harness.update_checks().len(), 1, "and it was asked only once");

    let mut same = started(Some(ecosystem("same")));
    same.set_latest_version(Some("0.0.1"));
    assert!(!same.screen().contains("is out"), "an older version is not news:\n{}", same.screen());
    clean(&folders);
}

#[test]
fn the_switch_on_the_settings_screen_turns_the_question_off_for_every_quvyta_application() {
    let folders = ecosystem("switch");
    let mut harness = started(Some(folders.clone()));
    assert_eq!(harness.update_checks().len(), 1, "on until someone turns it off");
    click_update_notice(&mut harness);
    // The write happens off the drawing path; a few frames let it land, bounded.
    for _ in 0..50 {
        if !Family::QUVYTA.update_notice_in(&folders.config) {
            break;
        }
        harness.render();
    }
    assert!(!Family::QUVYTA.update_notice_in(&folders.config), "the ecosystem's file says off:\n{}", harness.screen());
    let shared = std::fs::read_to_string(folders.config.join("quvyta.conf")).expect("the ecosystem's file");
    assert!(shared.contains("update-notice = false"), "{shared}");

    let mut off = started(Some(folders.clone()));
    assert!(off.update_checks().is_empty(), "a qdesk started with it off asks nothing at all");
    off.set_latest_version(Some(NEWER));
    assert!(!off.screen().contains("is out"), "{}", off.screen());
    clean(&folders);
}

#[test]
fn without_the_ecosystems_folders_nothing_is_asked_and_no_switch_is_shown() {
    let mut harness = started(None);
    assert!(harness.update_checks().is_empty(), "nothing is asked");
    harness.set_latest_version(Some(NEWER));
    assert!(!harness.screen().contains("is out"), "{}", harness.screen());
    open_settings(&mut harness);
    let screen = harness.screen();
    assert!(screen.contains("Glyphs") || screen.contains("Theme"), "the Settings screen is open:\n{screen}");
    assert!(!screen.contains("Say when an update is out"), "no switch that would do nothing:\n{screen}");
}
