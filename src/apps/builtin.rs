//! The entries that come with qdesk: its own screens and the other members of the family.
//!
//! They are written in the same format as the user's entries and read by the same parser, so a
//! user entry of the same id replaces one exactly as it would replace a system entry.

/// Each built-in entry as its id and its file.
pub(super) const ENTRIES: [(&str, &str); 7] = [
    ("terminal", include_str!("builtin/terminal.toml")),
    ("settings", include_str!("builtin/settings.toml")),
    ("qcode", include_str!("builtin/qcode.toml")),
    ("qfocus", include_str!("builtin/qfocus.toml")),
    ("qpac", include_str!("builtin/qpac.toml")),
    ("qtools", include_str!("builtin/qtools.toml")),
    ("quvyta", include_str!("builtin/quvyta.toml")),
];

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::super::entry::{Declared, Launch, Screen, Source, parse_entry};
    use super::super::{Category, Entry};
    use super::ENTRIES;

    fn built_in() -> Vec<Entry> {
        ENTRIES
            .iter()
            .map(|(id, text)| {
                let file = format!("{id}.toml");
                match parse_entry(id, Path::new(&file), text.as_bytes(), Source::Builtin, None) {
                    (Some(Declared::Entry(entry)), diagnostics) if diagnostics.is_empty() => *entry,
                    other => panic!("built-in entry {id} does not load cleanly: {other:?}"),
                }
            })
            .collect()
    }

    #[test]
    fn every_built_in_entry_loads_without_a_diagnostic() {
        assert_eq!(built_in().len(), ENTRIES.len());
    }

    #[test]
    fn terminal_and_settings_are_screens() {
        let entries = built_in();
        let launch = |id: &str| entries.iter().find(|entry| entry.id == id).map(|entry| entry.launch.clone());
        assert_eq!(launch("terminal"), Some(Launch::Screen(Screen::Terminal)));
        assert_eq!(launch("settings"), Some(Launch::Screen(Screen::Settings)));
    }

    #[test]
    fn family_members_run_their_command_and_install_with_quvyta() {
        let entries = built_in();
        let member = |id: &str| entries.iter().find(|entry| entry.id == id).expect("a family member");
        for (id, package) in [
            ("qcode", "quvyta-code"),
            ("qfocus", "quvyta-focus"),
            ("qpac", "quvyta-packages"),
            ("qtools", "quvyta-tools"),
        ] {
            let entry = member(id);
            assert_eq!(entry.category, Category::Family, "{id}");
            assert_eq!(entry.launch.program(), Some(id), "{id}");
            assert_eq!(entry.install.quvyta.as_deref(), Some(package), "{id}");
        }
        // quvyta installs the others; it cannot install itself.
        assert_eq!(member("quvyta").launch.program(), Some("quvyta"));
        assert!(!member("quvyta").install.is_known());
    }

    #[test]
    fn names_and_comments_have_turkish() {
        for entry in built_in() {
            let comment = entry.comment.as_ref().expect("every built-in entry has a comment");
            assert_ne!(comment.get("tr"), comment.get("en"), "{}", entry.id);
        }
        assert_eq!(
            built_in().iter().find(|entry| entry.id == "settings").map(|entry| entry.name.get("tr")),
            Some("Ayarlar")
        );
    }
}
