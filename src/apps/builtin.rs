//! The entries that come with qdesk: its own screens and the other Quvyta applications.
//!
//! They are written in the same format as the user's entries and read by the same parser, so a
//! user entry of the same id replaces one exactly as it would replace a system entry.

/// Each built-in entry as its id and its file.
pub(super) const ENTRIES: [(&str, &str); 13] = [
    ("terminal", include_str!("builtin/terminal.toml")),
    ("files", include_str!("builtin/files.toml")),
    ("settings", include_str!("builtin/settings.toml")),
    ("qbrow", include_str!("builtin/qbrow.toml")),
    ("qcli", include_str!("builtin/qcli.toml")),
    ("qcode", include_str!("builtin/qcode.toml")),
    ("qexp", include_str!("builtin/qexp.toml")),
    ("qfocus", include_str!("builtin/qfocus.toml")),
    ("qpac", include_str!("builtin/qpac.toml")),
    ("qtools", include_str!("builtin/qtools.toml")),
    ("quvyta", include_str!("builtin/quvyta.toml")),
    ("chawan", include_str!("builtin/chawan.toml")),
    ("w3m", include_str!("builtin/w3m.toml")),
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
    fn terminal_files_and_settings_are_screens() {
        let entries = built_in();
        let launch = |id: &str| entries.iter().find(|entry| entry.id == id).map(|entry| entry.launch.clone());
        assert_eq!(launch("terminal"), Some(Launch::Screen(Screen::Terminal)));
        assert_eq!(launch("files"), Some(Launch::Screen(Screen::Files)));
        assert_eq!(launch("settings"), Some(Launch::Screen(Screen::Settings)));
    }

    #[test]
    fn files_opens_as_many_windows_as_are_asked_for() {
        // Two folders side by side is the ordinary way to move things between them.
        let files = built_in().into_iter().find(|entry| entry.id == "files").expect("Files is built in");
        assert!(!files.single);
        assert_eq!(files.category, Category::Files);
        assert_eq!(files.name.get("tr"), "Dosyalar");
    }

    #[test]
    fn quvyta_applications_run_their_command_and_install_with_quvyta() {
        let entries = built_in();
        let member = |id: &str| entries.iter().find(|entry| entry.id == id).expect("a Quvyta application");
        for (id, package) in [
            ("qbrow", "quvyta-browser"),
            ("qcli", "quvyta-cli"),
            ("qcode", "quvyta-code"),
            ("qexp", "quvyta-explorer"),
            ("qfocus", "quvyta-focus"),
            ("qpac", "quvyta-packages"),
            ("qtools", "quvyta-tools"),
        ] {
            let entry = member(id);
            assert_eq!(entry.category, Category::Quvyta, "{id}");
            assert_eq!(entry.launch.program(), Some(id), "{id}");
            assert_eq!(entry.install.quvyta.as_deref(), Some(package), "{id}");
        }
        // quvyta installs the others; it cannot install itself.
        assert_eq!(member("quvyta").launch.program(), Some("quvyta"));
        assert!(!member("quvyta").install.is_known());
    }

    #[test]
    fn the_explorer_entry_runs_the_program_folders_open_in() {
        let entries = built_in();
        let explorer = entries.iter().find(|entry| entry.id == crate::app::EXPLORER).expect("qexp is built in");
        assert_eq!(explorer.launch.program(), Some(crate::app::EXPLORER));
    }

    #[test]
    fn text_web_browsers_run_when_they_are_there_and_w3m_installs_with_qpac() {
        let entries = built_in();
        let browser = |id: &str| entries.iter().find(|entry| entry.id == id).expect("a web browser");
        for (id, program) in [("chawan", "cha"), ("w3m", "w3m")] {
            let entry = browser(id);
            assert_eq!(entry.category, Category::Network, "{id}");
            assert_eq!(entry.launch.program(), Some(program), "{id}");
            assert!(entry.install.quvyta.is_none(), "{id} is not a Quvyta application");
        }
        assert_eq!(browser("w3m").install.qpac.as_deref(), Some("w3m"));
        assert!(!browser("chawan").install.is_known());
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
