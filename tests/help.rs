//! The help of the desktop: `f1` lists what qdesk can be told from the keyboard (design 10.1).
//!
//! The list is the framework's help layer, which reads the keymap itself. What is checked here is
//! that it opens on the key the design asks for, that it is honest — every line names a key that
//! really does something today — and that it reads in both languages.

mod support;

use std::collections::BTreeSet;

use qdesk::app::Desk;
use qframe::keymap::Scope;
use qframe::prelude::*;

use support::{decoration, desk};

/// The title of the layer, in the framework's own text.
const TITLE: &str = "Keyboard shortcuts";

/// A desktop with the help open.
fn helped(width: u16, height: u16) -> Harness<Desk> {
    let mut harness = desk(width, height);
    harness.press("f1");
    assert!(harness.screen().contains(TITLE), "f1 opened nothing:\n{}", harness.screen());
    harness
}

#[test]
fn f1_opens_the_help_and_esc_closes_it() {
    let mut harness = helped(100, 30);
    let screen = harness.screen();
    assert!(screen.contains("Open the launcher"), "the desktop's own keys are in it:\n{screen}");
    assert_eq!(decoration(&screen), None, "the help is drawn in tones:\n{screen}");
    harness.press("esc");
    assert!(!harness.screen().contains(TITLE), "esc closes it:\n{}", harness.screen());
    // And the keys are the desktop's again: f1 opens it a second time.
    harness.press("f1");
    assert!(harness.screen().contains(TITLE));
}

#[test]
fn the_ecosystem_s_own_question_mark_opens_the_same_help() {
    let mut harness = desk(100, 30);
    harness.press("?");
    assert!(harness.screen().contains(TITLE), "? opens it too:\n{}", harness.screen());
}

#[test]
fn every_key_of_the_desktop_has_a_line_of_its_own_in_both_languages() {
    // The layer writes one line per keymap action, with the text of `keys.<action>`. An action
    // without one would be a row with a key and no words, which is why every one is checked.
    let harness = helped(100, 30);
    let actions: Vec<String> = harness
        .env()
        .keymap()
        .iter()
        .filter(|(scope, _, chords)| *scope == Scope::App && !chords.is_empty())
        .map(|(_, action, _)| action.to_owned())
        .collect();
    assert!(actions.len() > 10, "the desktop binds more than a handful of keys: {actions:?}");
    let i18n = harness.env().i18n();
    for language in ["en", "tr"] {
        for action in &actions {
            assert!(i18n.has(language, &format!("keys.{action}")), "{language} does not name `{action}`");
        }
    }
}

#[test]
fn the_keys_that_only_answer_in_desktop_mode_say_so() {
    // The single letters do nothing while a window has the keys, so a line that promised otherwise
    // would be a line about a key that does not work.
    let mut harness = helped(100, 30);
    for (language, said) in [("en", "Desktop mode"), ("tr", "Masaüstü kipinde")] {
        harness.set_locale(language);
        let i18n = harness.env().i18n();
        for action in ["window-close", "window-move", "window-tile", "notices"] {
            let text = i18n.translate(&format!("keys.{action}"), &[]);
            assert!(text.starts_with(said), "{language}: `{action}` does not say when it works: {text}");
        }
    }
}

#[test]
fn no_two_keys_of_the_desktop_are_written_the_same_way() {
    let harness = helped(100, 30);
    let i18n = harness.env().i18n();
    let mut said: BTreeSet<String> = BTreeSet::new();
    for (scope, action, chords) in harness.env().keymap().iter() {
        if scope != Scope::App || chords.is_empty() {
            continue;
        }
        let text = i18n.translate(&format!("keys.{action}"), &[]);
        assert!(said.insert(text.clone()), "two actions are both written `{text}`");
    }
}

#[test]
fn the_help_lists_the_keys_of_the_floor_that_are_in_no_keymap() {
    let harness = helped(100, 30);
    let screen = harness.screen();
    for said in ["This screen", "Move between the icons", "Open the icon"] {
        assert!(screen.contains(said), "{said} is missing:\n{screen}");
    }
}

#[test]
fn the_help_speaks_turkish_when_the_language_does() {
    let mut harness = desk(100, 30);
    harness.set_locale("tr");
    harness.press("f1");
    let screen = harness.screen();
    assert!(screen.contains("Başlatıcıyı açar"), "the desktop's keys read Turkish:\n{screen}");
    assert!(screen.contains("Masaüstü kipinde"), "and so does what they answer to:\n{screen}");
}

#[test]
fn the_help_opens_from_desktop_mode_too_and_leaves_it_as_it_found_it() {
    let mut harness = desk(100, 30);
    harness.press("ctrl+alt+space");
    let mode = harness.app().keys();
    assert!(mode.is_some(), "the keys are the desktop's");
    harness.press("f1");
    assert!(harness.screen().contains(TITLE), "f1 works there as well:\n{}", harness.screen());
    harness.press("esc");
    assert!(!harness.screen().contains(TITLE));
    assert_eq!(harness.app().keys(), mode, "esc closed the help and not desktop mode");
}
