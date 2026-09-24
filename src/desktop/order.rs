//! What this person's desktop looks like: the order of the icons, the cells they were put in, the
//! applications they opened last, and whether the welcome line has been seen.
//!
//! All of it lives in one file, `~/.config/quvyta/desktop/desktop.toml`, because they are answers
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

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use qframe::storage::atomic_write;
use toml::de::{DeTable, DeValue};

use super::grid::Cell;
use crate::apps::{Diagnostic, DiagnosticKind, Expected, Position};
use crate::gadgets::{self, Gadget};

/// The name of the file inside the desktop's configuration folder.
pub const FILE: &str = "desktop.toml";

/// The icons a desktop nobody has changed shows (design 3.7): the shell, the folders and the
/// settings, the three things a person reaches for first on a machine they have just opened.
pub const DEFAULT_ICONS: [&str; 3] = ["terminal", "files", "settings"];

/// How many applications the launcher remembers as recent.
pub const RECENTS: usize = 8;

/// What the id of an entry of the Desktop folder starts with, before its name: `file:Projects`.
/// An application's id is the name of its entry file, and an entry file whose name started with
/// this would be taken for an entry of the folder; the prefix is chosen so that none does.
pub const FILE_PREFIX: &str = "file:";

/// The id an entry of the Desktop folder called `name` stands on the floor under.
#[must_use]
pub fn file_id(name: &str) -> String {
    format!("{FILE_PREFIX}{name}")
}

/// The desktop as the file describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Desktop {
    /// The ids of the applications on the floor, in the order they flow in.
    pub icons: Vec<String>,
    /// The cells icons were put in, by id: applications and entries of the Desktop folder alike.
    /// An icon with no cell here flows into the first free one, in its order. A file of 0.1 has
    /// no places at all, and its icons all flow as they did.
    pub places: BTreeMap<String, Cell>,
    /// The ids of the applications opened last, the newest first.
    pub recents: Vec<String>,
    /// Whether the welcome line has been closed; once it has, it never comes back.
    pub welcome_seen: bool,
    /// Whether the note saying how a window is resized has been shown: it is shown once, when the
    /// first window opens.
    pub resize_hint_seen: bool,
    /// The gadgets on the floor, in their order: an earlier one keeps its spot when two want the
    /// same cells. A file of an older qdesk has none.
    pub widgets: Vec<Gadget>,
}

impl Default for Desktop {
    fn default() -> Self {
        Self {
            icons: DEFAULT_ICONS.iter().map(|id| (*id).to_string()).collect(),
            places: BTreeMap::new(),
            recents: Vec::new(),
            welcome_seen: false,
            resize_hint_seen: false,
            widgets: Vec::new(),
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
    ///
    /// The places of the Desktop folder's entries are not the catalog's to judge; they go with
    /// [`keep_files`](Self::keep_files).
    pub fn keep_known(&mut self, known: impl Fn(&str) -> bool) -> bool {
        let before = (self.icons.len(), self.recents.len(), self.places.len());
        self.icons.retain(|id| known(id));
        self.recents.retain(|id| known(id));
        self.places.retain(|id, _| id.starts_with(FILE_PREFIX) || known(id));
        before != (self.icons.len(), self.recents.len(), self.places.len())
    }

    /// Drops the places of the Desktop folder's entries that `there` says are gone: a file deleted
    /// from the folder, or renamed by another program.
    pub fn keep_files(&mut self, there: impl Fn(&str) -> bool) -> bool {
        let before = self.places.len();
        self.places.retain(|id, _| id.strip_prefix(FILE_PREFIX).is_none_or(&there));
        before != self.places.len()
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

    /// Takes `id` off the floor, and forgets its place: an icon added again later flows in as a
    /// new one does.
    pub fn remove_icon(&mut self, id: &str) -> bool {
        let before = self.icons.len();
        self.icons.retain(|kept| kept != id);
        self.places.remove(id);
        before != self.icons.len()
    }

    /// Puts the icons in `order`.
    pub fn set_icons(&mut self, order: Vec<String>) {
        self.icons = order;
    }

    /// Puts the icon at `index` of `drawn` — every icon of the floor, with the cell it is drawn in
    /// this frame — into `cell`. The icon standing there, if any, takes the moved icon's cell: the
    /// two change places, and nothing else on the floor moves.
    ///
    /// The first move pins every icon that is drawn and has no place yet where it is drawn.
    /// Without that, taking one icon out of the flow would let the icons after it flow up into the
    /// gap, and moving one icon would move others. An icon drawn away from its own place — a
    /// screen too small for it — keeps that place. Returns whether anything changed.
    pub fn place(&mut self, drawn: &[(String, Option<Cell>)], index: usize, cell: Cell) -> bool {
        self.place_all(drawn, &[(index, cell)])
    }

    /// Puts several icons of `drawn` at once, each `(index, cell)` of `moves` into its cell: a
    /// selection carried together. The icons standing in the cells they land in, and not moving
    /// themselves, take the cells the group left, paired in the order of the cells, column by
    /// column; for one icon that is the change of places [`Desktop::place`] makes. Pins the floor
    /// as a single move does. Returns whether anything changed.
    pub fn place_all(&mut self, drawn: &[(String, Option<Cell>)], moves: &[(usize, Cell)]) -> bool {
        let movers: Vec<(&String, Cell, Cell)> = moves
            .iter()
            .filter_map(|(index, to)| match drawn.get(*index) {
                Some((id, Some(from))) => Some((id, *from, *to)),
                _ => None,
            })
            .collect();
        if movers.iter().all(|(_, from, to)| from == to) {
            return false;
        }
        for (other, at) in drawn {
            if let Some(at) = at {
                self.places.entry(other.clone()).or_insert(*at);
            }
        }
        let landing: Vec<Cell> = movers.iter().map(|(.., to)| *to).collect();
        let mut left: Vec<Cell> =
            movers.iter().map(|(_, from, _)| *from).filter(|from| !landing.contains(from)).collect();
        left.sort_unstable();
        let mut standing: Vec<(&String, Cell)> = drawn
            .iter()
            .filter(|(other, _)| !movers.iter().any(|(id, ..)| *id == other))
            .filter_map(|(other, at)| at.filter(|at| landing.contains(at)).map(|at| (other, at)))
            .collect();
        standing.sort_unstable_by_key(|(_, at)| *at);
        for ((other, _), cell) in standing.into_iter().zip(left) {
            self.places.insert(other.clone(), cell);
        }
        for (id, _, to) in movers {
            self.places.insert(id.clone(), to);
        }
        true
    }

    /// Moves the place kept under `from` to `to`: an entry of the Desktop folder that was renamed
    /// stays where it stood.
    pub fn rename_place(&mut self, from: &str, to: &str) {
        if let Some(cell) = self.places.remove(from) {
            self.places.insert(to.to_owned(), cell);
        }
    }

    /// Forgets every place, so all the icons flow again: "Arrange icons".
    pub fn clear_places(&mut self) {
        self.places.clear();
    }

    /// The file's contents. Comments say what the fields are for: the file is meant to be read
    /// and changed by hand as well.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let list = |ids: &[String]| {
            let quoted: Vec<String> = ids.iter().map(|id| quoted(id)).collect();
            format!("[{}]", quoted.join(", "))
        };
        let mut text = String::from("# The desktop of qdesk. Written when something changes.\n\n");
        text.push_str("# The icons on the floor, in the order they stand in.\n");
        text.push_str(&format!("icons = {}\n\n", list(&self.icons)));
        text.push_str("# The applications opened last, the newest first.\n");
        text.push_str(&format!("recents = {}\n\n", list(&self.recents)));
        text.push_str("# The welcome line is shown once and never again.\n");
        text.push_str(&format!("welcome_seen = {}\n", self.welcome_seen));
        // Written only once it is true, so a file of a qdesk that never showed the note is left
        // as that qdesk wrote it.
        if self.resize_hint_seen {
            text.push_str("# The note on how a window is resized is shown once, with the first window.\n");
            text.push_str("resize_hint_seen = true\n");
        }
        if !self.places.is_empty() {
            text.push_str("\n# The cells icons were put in, as [column, row] from the top left. An icon with no\n");
            text.push_str("# cell here flows into the first free one.\n[places]\n");
            for (id, (column, row)) in &self.places {
                text.push_str(&format!("{} = [{column}, {row}]\n", quoted(id)));
            }
        }
        // The gadgets are an array of tables, which TOML allows only after every plain key and
        // table above: they close the file.
        text.push_str(&gadgets::to_toml(&self.widgets));
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

/// `text` as a TOML basic string. The name of a file on the Desktop may hold anything a name can:
/// a quote, a backslash, a tab.
fn quoted(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A place as the file writes it: an array of two whole numbers, the column and the row.
pub(crate) fn place_of(value: &DeValue<'_>) -> Option<Cell> {
    let DeValue::Array(items) = value else { return None };
    let numbers: Vec<u16> = items
        .iter()
        .map(|item| match item.get_ref() {
            DeValue::Integer(number) => u16::from_str_radix(number.as_str(), number.radix()).ok(),
            _ => None,
        })
        .collect::<Option<_>>()?;
    match numbers[..] {
        [column, row] => Some((column, row)),
        _ => None,
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
            ("places", DeValue::Table(places)) => {
                for (id, value) in places {
                    let id = id.get_ref().as_ref();
                    match place_of(value.get_ref()) {
                        Some(cell) if !id.trim().is_empty() => {
                            desktop.places.insert(id.to_owned(), cell);
                        }
                        _ => diagnostics.push(Diagnostic::at(
                            file,
                            Some(Position::of_offset(text, value.span().start)),
                            DiagnosticKind::InvalidValue { key: format!("places.{id}") },
                        )),
                    }
                }
            }
            ("places", _) => diagnostics.push(wrong(name, Expected::Table, offset)),
            ("welcome_seen", DeValue::Boolean(seen)) => desktop.welcome_seen = *seen,
            ("welcome_seen", _) => diagnostics.push(wrong(name, Expected::Boolean, offset)),
            ("resize_hint_seen", DeValue::Boolean(seen)) => desktop.resize_hint_seen = *seen,
            ("resize_hint_seen", _) => diagnostics.push(wrong(name, Expected::Boolean, offset)),
            ("widgets", _) => {
                let (widgets, problems) = gadgets::parse(file, text, value);
                desktop.widgets = widgets;
                diagnostics.extend(problems);
            }
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
    fn a_desktop_nobody_changed_shows_the_terminal_the_files_and_the_settings() {
        let desktop = Desktop::default();
        assert_eq!(desktop.icons, ids(&["terminal", "files", "settings"]));
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
        assert_eq!(desktop.icons, ids(&["terminal", "files", "settings", "htop"]));
        assert!(desktop.remove_icon("settings"));
        assert!(!desktop.remove_icon("settings"));
        assert_eq!(desktop.icons, ids(&["terminal", "files", "htop"]));
        desktop.set_icons(ids(&["htop", "terminal"]));
        assert_eq!(desktop.icons, ids(&["htop", "terminal"]));
    }

    #[test]
    fn what_is_written_reads_back_the_same() {
        let mut desktop = Desktop::default();
        desktop.set_icons(ids(&["htop", "a \"quoted\" id"]));
        desktop.places.insert("htop".to_owned(), (4, 0));
        desktop.places.insert(file_id("a \\ back\tslash \"and\" quote = [1, 2]"), (0, 65_535));
        desktop.places.insert(file_id("Masaüstü notları"), (2, 3));
        desktop.remember("vim");
        desktop.welcome_seen = true;
        let (read_back, diagnostics) = read(&desktop.to_toml());
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(read_back, desktop);
    }

    #[test]
    fn a_file_of_0_1_with_only_the_order_leaves_every_icon_to_flow() {
        let (desktop, diagnostics) = read("icons = [\"htop\", \"terminal\"]\nrecents = []\nwelcome_seen = true\n");
        assert!(diagnostics.is_empty(), "{diagnostics:?}");
        assert_eq!(desktop.icons, ids(&["htop", "terminal"]));
        assert!(desktop.places.is_empty(), "no place is a place to flow into");
    }

    #[test]
    fn the_places_are_read_by_id_and_a_broken_one_names_itself() {
        let (desktop, diagnostics) = read(
            "icons = [\"htop\"]\n\n[places]\nhtop = [3, 1]\n\"file:Projeler\" = [0, 4]\nvim = [1]\nlf = [-1, 2]\n",
        );
        assert_eq!(desktop.places.get("htop"), Some(&(3, 1)));
        assert_eq!(desktop.places.get("file:Projeler"), Some(&(0, 4)));
        assert_eq!(desktop.places.len(), 2, "the broken places are left out: {:?}", desktop.places);
        let keys: Vec<&DiagnosticKind> = diagnostics.iter().map(|d| &d.kind).collect();
        assert_eq!(
            keys,
            vec![
                &DiagnosticKind::InvalidValue { key: "places.vim".to_owned() },
                &DiagnosticKind::InvalidValue { key: "places.lf".to_owned() },
            ]
        );
        assert!(diagnostics.iter().all(|d| d.position.is_some()));
        let (_, diagnostics) = read("places = 3\n");
        assert_eq!(
            diagnostics[0].kind,
            DiagnosticKind::WrongType { key: "places".to_owned(), expected: Expected::Table }
        );
    }

    fn drawn(cells: &[(&str, Option<Cell>)]) -> Vec<(String, Option<Cell>)> {
        cells.iter().map(|(id, cell)| ((*id).to_owned(), *cell)).collect()
    }

    #[test]
    fn a_drop_on_a_free_cell_pins_the_floor_and_moves_only_that_icon() {
        let mut desktop = Desktop::default();
        let floor = drawn(&[("terminal", Some((0, 0))), ("files", Some((0, 1))), ("settings", Some((0, 2)))]);
        assert!(desktop.place(&floor, 0, (4, 2)));
        assert_eq!(desktop.places.get("terminal"), Some(&(4, 2)));
        assert_eq!(desktop.places.get("files"), Some(&(0, 1)), "the others stay where they were drawn");
        assert_eq!(desktop.places.get("settings"), Some(&(0, 2)));
        assert_eq!(desktop.icons, ids(&["terminal", "files", "settings"]), "the order is not touched");
        assert!(!desktop.place(&floor, 1, (0, 1)), "a drop on its own cell changes nothing");
        assert!(!desktop.place(&floor, 7, (0, 1)), "nor does an icon that is not there");
    }

    #[test]
    fn a_drop_on_an_icon_changes_the_two_places_and_keeps_a_place_the_screen_had_no_room_for() {
        let mut desktop = Desktop::default();
        desktop.places.insert("settings".to_owned(), (9, 9));
        // Settings is drawn at (0, 2) on this small screen, away from its own place.
        let floor = drawn(&[("terminal", Some((0, 0))), ("files", Some((0, 1))), ("settings", Some((0, 2)))]);
        assert!(desktop.place(&floor, 0, (0, 1)));
        assert_eq!(desktop.places.get("terminal"), Some(&(0, 1)));
        assert_eq!(desktop.places.get("files"), Some(&(0, 0)), "the icon that stood there took the other cell");
        assert_eq!(desktop.places.get("settings"), Some(&(9, 9)), "a place the screen could not show is kept");
    }

    #[test]
    fn a_group_carried_together_keeps_its_shape_and_the_icons_it_lands_on_take_the_cells_it_left() {
        let mut desktop = Desktop::default();
        let floor = drawn(&[
            ("terminal", Some((0, 0))),
            ("files", Some((0, 1))),
            ("settings", Some((0, 2))),
            ("htop", Some((1, 1))),
            ("vim", Some((1, 2))),
        ]);
        // Terminal and Files go one column right: Files lands on htop, Terminal on bare floor.
        assert!(desktop.place_all(&floor, &[(0, (1, 0)), (1, (1, 1))]));
        assert_eq!(desktop.places.get("terminal"), Some(&(1, 0)));
        assert_eq!(desktop.places.get("files"), Some(&(1, 1)));
        assert_eq!(desktop.places.get("htop"), Some(&(0, 0)), "the icon landed on takes a cell the group left");
        assert_eq!(desktop.places.get("settings"), Some(&(0, 2)), "an icon not in the way stays");
        assert_eq!(desktop.places.get("vim"), Some(&(1, 2)));
        let mut cells: Vec<Cell> = desktop.places.values().copied().collect();
        cells.sort_unstable();
        cells.dedup();
        assert_eq!(cells.len(), 5, "no two icons share a cell: {:?}", desktop.places);
        assert!(
            !desktop.place_all(&floor, &[(0, (0, 0)), (1, (0, 1))]),
            "a group put back where it is drawn is no change"
        );
    }

    #[test]
    fn arranging_forgets_the_places_and_removing_an_icon_forgets_its_own() {
        let mut desktop = Desktop::default();
        desktop.places.insert("terminal".to_owned(), (2, 2));
        desktop.places.insert("files".to_owned(), (3, 3));
        desktop.remove_icon("terminal");
        assert_eq!(desktop.places.len(), 1);
        desktop.clear_places();
        assert!(desktop.places.is_empty());
    }

    #[test]
    fn the_places_of_the_desktop_folder_follow_its_entries() {
        let mut desktop = Desktop::default();
        desktop.places.insert(file_id("Notlar"), (1, 1));
        desktop.places.insert(file_id("Eski"), (2, 1));
        desktop.places.insert("gone".to_owned(), (3, 1));
        assert!(desktop.keep_known(|id| id != "gone"));
        assert_eq!(desktop.places.len(), 2, "the catalog does not judge the folder's entries");
        desktop.rename_place(&file_id("Notlar"), &file_id("Notlar 2026"));
        assert_eq!(desktop.places.get("file:Notlar 2026"), Some(&(1, 1)), "a renamed entry keeps its cell");
        assert!(desktop.keep_files(|name| name != "Eski"));
        assert_eq!(desktop.places.keys().collect::<Vec<_>>(), ["file:Notlar 2026"]);
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
