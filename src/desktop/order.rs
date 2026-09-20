//! What this person's desktop looks like: the order of the icons, the applications they opened
//! last, and whether the welcome line has been seen.
//!
//! All three live in one file, `~/.config/quvyta/desktop/desktop.toml`, because they are answers
//! to the same question — what the desktop should look like when it opens — and they are read
//! once at the start and written once when something changes. A second file for the recents
//! would be another read on every start and another thing to disagree with the order.
//!
//! Defaults are never written (VISION §5): a desktop nobody has changed has no file at all. The
//! file appears the first time the person moves, adds or removes an icon, opens an application or
//! closes the welcome line, and it is then written whole, atomically, so a machine that loses
//! power in the middle of a write keeps the file it had.
//!
//! A broken file is a diagnostic, not a crash: what can be read is kept, the rest falls back to
//! the defaults, and the interface says which file and which line said it.

use std::io;
use std::path::{Path, PathBuf};

use qframe::storage::atomic_write;
use toml::de::{DeTable, DeValue};

use crate::apps::{Diagnostic, DiagnosticKind, Expected, Position};

/// The name of the file inside the desktop's configuration folder.
pub const FILE: &str = "desktop.toml";

/// The icons a desktop nobody has changed shows. The Files application joins them when it exists
/// (0.2); until then the entry for it does not exist either, so an id is not written here for it.
pub const DEFAULT_ICONS: [&str; 2] = ["terminal", "settings"];

/// How many applications the launcher remembers as recent.
pub const RECENTS: usize = 8;

/// The desktop as the file describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desktop {
    /// The ids of the icons on the floor, in the order they stand in.
    pub icons: Vec<String>,
    /// The ids of the applications opened last, the newest first.
    pub recents: Vec<String>,
    /// Whether the welcome line has been closed; once it has, it never comes back.
    pub welcome_seen: bool,
}

impl Default for Desktop {
    fn default() -> Self {
        Self {
            icons: DEFAULT_ICONS.iter().map(|id| (*id).to_string()).collect(),
            recents: Vec::new(),
            welcome_seen: false,
        }
    }
}

impl Desktop {
    /// The file in the desktop's configuration folder, `None` when the system names no
    /// configuration folder at all.
    #[must_use]
    pub fn path() -> Option<PathBuf> {
        qframe::storage::config_dir("quvyta").map(|dir| dir.join("desktop").join(FILE))
    }

    /// Reads `path`, falling back to the defaults for what it does not hold.
    ///
    /// A file that is not there is the ordinary case and says nothing.
    #[must_use]
    pub fn load(path: &Path) -> (Self, Vec<Diagnostic>) {
        match std::fs::read(path) {
            Ok(bytes) => parse(path, &bytes),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (Self::default(), Vec::new()),
            Err(error) => (
                Self::default(),
                vec![Diagnostic::at(path, None, DiagnosticKind::Unreadable).with_detail(error.to_string())],
            ),
        }
    }

    /// Drops the ids that no entry answers to any more: an application removed from the machine
    /// leaves its icon behind, and a desktop must not keep a name that opens nothing. This is
    /// quiet on purpose — a removed program is not a broken file — and the shortened order is
    /// written the next time something is saved.
    pub fn keep_known(&mut self, known: impl Fn(&str) -> bool) -> bool {
        let before = (self.icons.len(), self.recents.len());
        self.icons.retain(|id| known(id));
        self.recents.retain(|id| known(id));
        before != (self.icons.len(), self.recents.len())
    }

    /// Puts `id` at the front of the recents, keeping at most [`RECENTS`] of them.
    pub fn remember(&mut self, id: &str) {
        self.recents.retain(|kept| kept != id);
        self.recents.insert(0, id.to_owned());
        self.recents.truncate(RECENTS);
    }

    /// Adds `id` to the floor, if it is not already there.
    pub fn add_icon(&mut self, id: &str) -> bool {
        if self.icons.iter().any(|kept| kept == id) {
            return false;
        }
        self.icons.push(id.to_owned());
        true
    }

    /// Takes `id` off the floor.
    pub fn remove_icon(&mut self, id: &str) -> bool {
        let before = self.icons.len();
        self.icons.retain(|kept| kept != id);
        before != self.icons.len()
    }

    /// Puts the icons in `order`.
    pub fn set_icons(&mut self, order: Vec<String>) {
        self.icons = order;
    }

    /// The file's contents. Comments say what the fields are for: the file is meant to be read
    /// and changed by hand as well.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let list = |ids: &[String]| {
            let quoted: Vec<String> = ids.iter().map(|id| format!("\"{}\"", id.replace('"', "\\\""))).collect();
            format!("[{}]", quoted.join(", "))
        };
        let mut text = String::from("# The desktop of qdesk. Written when something changes.\n\n");
        text.push_str("# The icons on the floor, in the order they stand in.\n");
        text.push_str(&format!("icons = {}\n\n", list(&self.icons)));
        text.push_str("# The applications opened last, the newest first.\n");
        text.push_str(&format!("recents = {}\n\n", list(&self.recents)));
        text.push_str("# The welcome line is shown once and never again.\n");
        text.push_str(&format!("welcome_seen = {}\n", self.welcome_seen));
        text
    }

    /// Writes the file, making its folder first.
    ///
    /// # Errors
    ///
    /// Returns the system's error when the folder cannot be made or the file cannot be written.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder)?;
        }
        atomic_write(path, self.to_toml().as_bytes())
    }
}

/// Reads the contents of the desktop file `bytes`, read from `file`.
#[must_use]
pub fn parse(file: &Path, bytes: &[u8]) -> (Desktop, Vec<Diagnostic>) {
    let mut desktop = Desktop::default();
    let mut diagnostics = Vec::new();
    let Ok(text) = std::str::from_utf8(bytes) else {
        let valid = String::from_utf8_lossy(bytes);
        let position = Position::of_offset(&valid, valid.len());
        return (desktop, vec![Diagnostic::at(file, Some(position), DiagnosticKind::NotUtf8)]);
    };
    let (root, errors) = DeTable::parse_recoverable(text);
    if !errors.is_empty() {
        let diagnostics = errors
            .iter()
            .map(|error| {
                let position = error.span().map(|span| Position::of_offset(text, span.start));
                Diagnostic::at(file, position, DiagnosticKind::Syntax).with_detail(error.message())
            })
            .collect();
        return (desktop, diagnostics);
    }
    let wrong = |key: &str, expected: Expected, offset: usize| {
        let position = Position::of_offset(text, offset);
        Diagnostic::at(file, Some(position), DiagnosticKind::WrongType { key: key.to_owned(), expected })
    };
    for (key, value) in root.get_ref() {
        let name = key.get_ref().as_ref();
        let offset = value.span().start;
        match (name, value.get_ref()) {
            ("icons" | "recents", DeValue::Array(items)) => {
                let mut ids = Vec::new();
                let mut broken = false;
                for item in items {
                    match item.get_ref() {
                        DeValue::String(id) if !id.trim().is_empty() => ids.push(id.to_string()),
                        _ => broken = true,
                    }
                }
                if broken {
                    diagnostics.push(wrong(name, Expected::StringList, offset));
                }
                if name == "icons" {
                    desktop.icons = ids;
                } else {
                    desktop.recents = ids;
                }
            }
            ("icons" | "recents", _) => diagnostics.push(wrong(name, Expected::StringList, offset)),
            ("welcome_seen", DeValue::Boolean(seen)) => desktop.welcome_seen = *seen,
            ("welcome_seen", _) => diagnostics.push(wrong(name, Expected::Boolean, offset)),
            _ => {
                let position = Position::of_offset(text, key.span().start);
                diagnostics.push(Diagnostic::at(
                    file,
                    Some(position),
                    DiagnosticKind::UnknownField { key: name.to_owned() },
                ));
            }
        }
    }
    (desktop, diagnostics)
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn file() -> PathBuf {
        PathBuf::from("/home/kisi/.config/quvyta/desktop/desktop.toml")
    }

    fn read(text: &str) -> (Desktop, Vec<Diagnostic>) {
        parse(&file(), text.as_bytes())
    }

    fn ids(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn a_desktop_nobody_changed_shows_the_terminal_and_the_settings() {
        let desktop = Desktop::default();
        assert_eq!(desktop.icons, ids(&["terminal", "settings"]));
        assert!(desktop.recents.is_empty());
        assert!(!desktop.welcome_seen);
    }

    #[test]
    fn a_file_that_is_not_there_gives_the_defaults_says_nothing_and_stays_unwritten() {
        let path = std::env::temp_dir().join(format!("qdesk-absent-{}", std::process::id())).join(FILE);
        let _ = std::fs::remove_file(&path);
        let (desktop, diagnostics) = Desktop::load(&path);
        assert_eq!(desktop, Desktop::default());
        assert!(diagnostics.is_empty());
        assert!(!path.exists(), "reading never writes the defaults");
    }

    #[test]
    fn the_file_gives_the_order_the_recents_and_the_welcome() {
        let (desktop, diagnostics) =
            read("icons = [\"htop\", \"terminal\"]\nrecents = [\"vim\"]\nwelcome_seen = true\n");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(desktop.icons, ids(&["htop", "terminal"]));
        assert_eq!(desktop.recents, ids(&["vim"]));
        assert!(desktop.welcome_seen);
    }

    #[test]
    fn an_empty_order_is_an_empty_desktop_not_the_defaults() {
        let (desktop, diagnostics) = read("icons = []\n");
        assert!(diagnostics.is_empty());
        assert!(desktop.icons.is_empty(), "a person may keep a bare floor");
    }

    #[test]
    fn a_broken_file_is_a_diagnostic_with_its_place_and_keeps_the_defaults() {
        let (desktop, diagnostics) = read("icons = [\"htop\"\nrecents = []\n");
        assert_eq!(desktop, Desktop::default());
        assert!(!diagnostics.is_empty());
        assert!(diagnostics.iter().all(|problem| problem.kind == DiagnosticKind::Syntax));
        assert!(diagnostics[0].location().starts_with("/home/kisi/.config/quvyta/desktop/desktop.toml:"));
        assert!(diagnostics[0].detail.is_some());
    }

    #[test]
    fn a_field_of_the_wrong_kind_names_itself_and_what_it_should_have_been() {
        let (desktop, diagnostics) = read("icons = \"htop\"\nwelcome_seen = 1\n");
        assert_eq!(desktop, Desktop::default(), "neither field was taken");
        let kinds: Vec<&DiagnosticKind> = diagnostics.iter().map(|d| &d.kind).collect();
        assert_eq!(
            kinds,
            vec![
                &DiagnosticKind::WrongType { key: "icons".to_owned(), expected: Expected::StringList },
                &DiagnosticKind::WrongType { key: "welcome_seen".to_owned(), expected: Expected::Boolean },
            ]
        );
        assert!(diagnostics.iter().all(|d| d.position.is_some()));
    }

    #[test]
    fn an_order_with_something_that_is_not_an_id_keeps_the_ids_it_has() {
        let (desktop, diagnostics) = read("icons = [\"htop\", 7, \"\", \"vim\"]\n");
        assert_eq!(desktop.icons, ids(&["htop", "vim"]));
        assert_eq!(diagnostics.len(), 1);
        assert!(matches!(diagnostics[0].kind, DiagnosticKind::WrongType { .. }));
    }

    #[test]
    fn a_field_qdesk_does_not_know_is_a_warning_and_the_rest_still_loads() {
        let (desktop, diagnostics) = read("icons = [\"htop\"]\nwallpaper = \"beach.png\"\n");
        assert_eq!(desktop.icons, ids(&["htop"]));
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::UnknownField { key: "wallpaper".to_owned() });
        assert!(diagnostics[0].is_warning());
    }

    #[test]
    fn ids_no_entry_answers_to_are_dropped_quietly() {
        let (mut desktop, _) = read("icons = [\"htop\", \"gone\"]\nrecents = [\"gone\", \"vim\"]\n");
        let known = |id: &str| id != "gone";
        assert!(desktop.keep_known(known));
        assert_eq!(desktop.icons, ids(&["htop"]));
        assert_eq!(desktop.recents, ids(&["vim"]));
        assert!(!desktop.keep_known(known), "a second pass has nothing left to drop");
    }

    #[test]
    fn the_recents_keep_the_newest_first_without_repeating() {
        let mut desktop = Desktop::default();
        for id in ["htop", "vim", "htop"] {
            desktop.remember(id);
        }
        assert_eq!(desktop.recents, ids(&["htop", "vim"]));
        for n in 0..RECENTS {
            desktop.remember(&format!("app-{n}"));
        }
        assert_eq!(desktop.recents.len(), RECENTS);
        assert_eq!(desktop.recents[0], format!("app-{}", RECENTS - 1));
    }

    #[test]
    fn icons_are_added_once_and_removed_by_id() {
        let mut desktop = Desktop::default();
        assert!(desktop.add_icon("htop"));
        assert!(!desktop.add_icon("htop"));
        assert_eq!(desktop.icons, ids(&["terminal", "settings", "htop"]));
        assert!(desktop.remove_icon("settings"));
        assert!(!desktop.remove_icon("settings"));
        assert_eq!(desktop.icons, ids(&["terminal", "htop"]));
        desktop.set_icons(ids(&["htop", "terminal"]));
        assert_eq!(desktop.icons, ids(&["htop", "terminal"]));
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        let mut desktop = Desktop::default();
        desktop.set_icons(ids(&["htop", "a \"quoted\" id"]));
        desktop.remember("vim");
        desktop.welcome_seen = true;
        let (read_back, diagnostics) = read(&desktop.to_toml());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(read_back, desktop);
    }

    #[test]
    fn saving_writes_the_file_whole_and_leaves_no_half_file_behind() {
        let folder = std::env::temp_dir().join(format!("qdesk-order-{}", std::process::id()));
        let path = folder.join("desktop").join(FILE);
        let _ = std::fs::remove_dir_all(&folder);
        let mut desktop = Desktop::default();
        desktop.add_icon("htop");
        desktop.save(&path).expect("the file is written");
        let (read_back, diagnostics) = Desktop::load(&path);
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(read_back, desktop);
        // Writing again over a file that is there leaves one file, not a temporary beside it.
        desktop.welcome_seen = true;
        desktop.save(&path).expect("the file is written again");
        let left: Vec<String> = std::fs::read_dir(path.parent().expect("a folder"))
            .expect("the folder can be read")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec![FILE.to_owned()]);
        assert_eq!(Desktop::load(&path).0, desktop);
        std::fs::remove_dir_all(&folder).expect("the test cleans up after itself");
    }
}
