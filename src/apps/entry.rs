//! The entry file: one application, written as a small TOML file.

use std::path::{Path, PathBuf};

use toml::Spanned;
use toml::de::{DeTable, DeValue};

use super::diagnostic::{Diagnostic, DiagnosticKind, Expected, Position};
use super::{Category, Localized};

/// Where an entry was read from, highest priority first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Source {
    /// The user's entry folder.
    User,
    /// One of the system's entry folders.
    System,
    /// Built into qdesk.
    Builtin,
    /// A system `.desktop` file of a terminal program.
    DesktopFile,
}

/// A screen qdesk draws itself instead of starting a program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Screen {
    /// The user's shell in a window.
    Terminal,
    /// qdesk's settings.
    Settings,
}

impl Screen {
    /// The screen a built-in entry names.
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "terminal" => Some(Self::Terminal),
            "settings" => Some(Self::Settings),
            _ => None,
        }
    }
}

/// What opening an entry starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// A program and its arguments, started without a shell. Never empty.
    Command(Vec<String>),
    /// A folder or a file, shown by the matching built-in viewer.
    Open(PathBuf),
    /// A screen of qdesk itself.
    Screen(Screen),
}

impl Launch {
    /// The program a command starts: its first word.
    #[must_use]
    pub fn program(&self) -> Option<&str> {
        match self {
            Self::Command(words) => words.first().map(String::as_str),
            Self::Open(_) | Self::Screen(_) => None,
        }
    }
}

/// How the window of an entry first opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct WindowPrefs {
    /// The first size, in columns and rows; the desktop picks one when it is not given.
    pub size: Option<(u16, u16)>,
    /// Whether the window opens maximized.
    pub maximized: bool,
}

/// How a missing program is installed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Install {
    /// The package qpac installs.
    pub qpac: Option<String>,
    /// The family member quvyta installs.
    pub quvyta: Option<String>,
}

impl Install {
    /// Whether the entry says how to install it.
    #[must_use]
    pub fn is_known(&self) -> bool {
        self.qpac.is_some() || self.quvyta.is_some()
    }
}

/// One application the desktop can open.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The id: the name of the file without its extension.
    pub id: String,
    /// The name shown under the icon and in the launcher.
    pub name: Localized,
    /// A short description for the launcher and tooltips.
    pub comment: Option<Localized>,
    /// A framework icon name or a single character; the category's icon when it is not given.
    pub icon: Option<String>,
    /// What opening the entry starts.
    pub launch: Launch,
    /// The folder the program starts in; the home folder when it is not given.
    pub folder: Option<PathBuf>,
    /// Extra environment variables for the program, in the order the file gives them.
    pub env: Vec<(String, String)>,
    /// Where the entry sits in the launcher.
    pub category: Category,
    /// More words search matches.
    pub keywords: Vec<String>,
    /// Whether opening it again brings the open window forward instead of opening another.
    pub single: bool,
    /// Whether the window closes when the program ends.
    pub close_on_exit: bool,
    /// How the window first opens.
    pub window: WindowPrefs,
    /// How the program is installed when it is missing.
    pub install: Install,
    /// A `.desktop` file's `TryExec`: another program that must exist for the entry to count as
    /// installed.
    pub try_exec: Option<String>,
    /// Where the entry was read from.
    pub source: Source,
    /// The file it was read from; `None` for a built-in entry.
    pub file: Option<PathBuf>,
}

/// What an entry file declares.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Declared {
    /// An application.
    Entry(Box<Entry>),
    /// `hidden = true`: the id is hidden from every source below this one.
    Hidden,
}

/// The fields an entry file may have at its top level.
const FIELDS: [&str; 14] = [
    "name",
    "comment",
    "icon",
    "command",
    "open",
    "folder",
    "env",
    "category",
    "keywords",
    "single",
    "close_on_exit",
    "window",
    "install",
    "hidden",
];

/// Reads the entry file `bytes` of id `id`, read from `file`.
///
/// `home` replaces a leading `~` in `open` and `folder`. Built-in entries may also name a screen
/// of qdesk with `screen = "terminal"` or `"settings"`; in other files that field is unknown.
///
/// Returns what the file declares, or `None` when it is broken, with every problem found. A file
/// with only warnings still declares its entry.
#[must_use]
pub fn parse_entry(
    id: &str,
    file: &Path,
    bytes: &[u8],
    source: Source,
    home: Option<&Path>,
) -> (Option<Declared>, Vec<Diagnostic>) {
    let text = match std::str::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => {
            let valid = String::from_utf8_lossy(&bytes[..error.valid_up_to()]);
            let position = Position::of_offset(&valid, valid.len());
            return (None, vec![Diagnostic::at(file, Some(position), DiagnosticKind::NotUtf8)]);
        }
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
        return (None, diagnostics);
    }
    let mut reader = Reader { file, text, diagnostics: Vec::new(), failed: false };
    let declared = reader.entry(id, root.get_ref(), source, home);
    let declared = if reader.failed { None } else { declared };
    (declared, reader.diagnostics)
}

/// A value with the place it was written.
type Value<'i> = Spanned<DeValue<'i>>;

/// Walks one parsed file, collecting its problems.
struct Reader<'a> {
    file: &'a Path,
    text: &'a str,
    diagnostics: Vec<Diagnostic>,
    /// Whether an error, not only a warning, was found.
    failed: bool,
}

impl Reader<'_> {
    fn report(&mut self, offset: usize, kind: DiagnosticKind) {
        self.failed |= !kind.is_warning();
        self.diagnostics.push(Diagnostic::at(self.file, Some(Position::of_offset(self.text, offset)), kind));
    }

    fn wrong_type(&mut self, key: &str, value: &Value<'_>, expected: Expected) {
        self.report(value.span().start, DiagnosticKind::WrongType { key: key.to_owned(), expected });
    }

    fn entry(&mut self, id: &str, root: &DeTable<'_>, source: Source, home: Option<&Path>) -> Option<Declared> {
        if let Some(hidden) = field(root, "hidden") {
            match hidden.get_ref() {
                DeValue::Boolean(true) => return Some(Declared::Hidden),
                DeValue::Boolean(false) => {}
                _ => self.wrong_type("hidden", hidden, Expected::Boolean),
            }
        }
        for (key, _) in root {
            let name = key.get_ref().as_ref();
            let allowed = FIELDS.contains(&name) || (source == Source::Builtin && name == "screen");
            if !allowed {
                self.report(key.span().start, DiagnosticKind::UnknownField { key: name.to_owned() });
            }
        }

        let name = match field(root, "name") {
            Some(value) => self.text("name", value),
            None => {
                self.report(0, DiagnosticKind::MissingName);
                None
            }
        };
        let comment = field(root, "comment").and_then(|value| self.text("comment", value));
        let icon = field(root, "icon").and_then(|value| self.non_empty_string("icon", value));
        let launch = self.launch(root, source, home);
        let folder = field(root, "folder").and_then(|value| self.path("folder", value, home));
        let env = field(root, "env").map(|value| self.env(value)).unwrap_or_default();
        let category = field(root, "category").map_or(Category::Other, |value| self.category(value));
        let keywords = field(root, "keywords").and_then(|value| self.strings("keywords", value)).unwrap_or_default();
        let single = field(root, "single").and_then(|value| self.boolean("single", value)).unwrap_or(false);
        let close_on_exit =
            field(root, "close_on_exit").and_then(|value| self.boolean("close_on_exit", value)).unwrap_or(false);
        let window = field(root, "window").map(|value| self.window(value)).unwrap_or_default();
        let install = field(root, "install").map(|value| self.install(value)).unwrap_or_default();

        let file = (source != Source::Builtin).then(|| self.file.to_path_buf());
        Some(Declared::Entry(Box::new(Entry {
            id: id.to_owned(),
            name: name?,
            comment,
            icon,
            launch: launch?,
            folder,
            env,
            category,
            keywords,
            single,
            close_on_exit,
            window,
            install,
            try_exec: None,
            source,
            file,
        })))
    }

    /// `command`, `open` or a built-in `screen`: exactly one of them.
    fn launch(&mut self, root: &DeTable<'_>, source: Source, home: Option<&Path>) -> Option<Launch> {
        let screen = if source == Source::Builtin { field(root, "screen") } else { None };
        let given: Vec<(&str, &Value<'_>)> = [("command", field(root, "command")), ("open", field(root, "open"))]
            .into_iter()
            .chain([("screen", screen)])
            .filter_map(|(key, value)| value.map(|value| (key, value)))
            .collect();
        match given[..] {
            [] => {
                self.report(0, DiagnosticKind::MissingLaunch);
                None
            }
            [(key, value)] => match key {
                "command" => {
                    let words = self.strings("command", value)?;
                    if words.first().is_none_or(String::is_empty) {
                        self.report(value.span().start, DiagnosticKind::InvalidValue { key: "command".to_owned() });
                        return None;
                    }
                    Some(Launch::Command(words))
                }
                "open" => self.path("open", value, home).map(Launch::Open),
                _ => {
                    let name = self.string("screen", value)?;
                    let screen = Screen::from_name(&name);
                    if screen.is_none() {
                        self.report(value.span().start, DiagnosticKind::InvalidValue { key: "screen".to_owned() });
                    }
                    screen.map(Launch::Screen)
                }
            },
            [_, (_, second), ..] => {
                self.report(second.span().start, DiagnosticKind::ConflictingLaunch);
                None
            }
        }
    }

    fn string(&mut self, key: &str, value: &Value<'_>) -> Option<String> {
        match value.get_ref() {
            DeValue::String(text) => Some(text.to_string()),
            _ => {
                self.wrong_type(key, value, Expected::String);
                None
            }
        }
    }

    fn non_empty_string(&mut self, key: &str, value: &Value<'_>) -> Option<String> {
        let text = self.string(key, value)?;
        if text.trim().is_empty() {
            self.report(value.span().start, DiagnosticKind::InvalidValue { key: key.to_owned() });
            return None;
        }
        Some(text)
    }

    fn boolean(&mut self, key: &str, value: &Value<'_>) -> Option<bool> {
        match value.get_ref() {
            DeValue::Boolean(flag) => Some(*flag),
            _ => {
                self.wrong_type(key, value, Expected::Boolean);
                None
            }
        }
    }

    fn strings(&mut self, key: &str, value: &Value<'_>) -> Option<Vec<String>> {
        let DeValue::Array(items) = value.get_ref() else {
            self.wrong_type(key, value, Expected::StringList);
            return None;
        };
        let mut words = Vec::with_capacity(items.len());
        for item in items.iter() {
            match item.get_ref() {
                DeValue::String(text) => words.push(text.to_string()),
                _ => self.wrong_type(key, item, Expected::StringList),
            }
        }
        (words.len() == items.len()).then_some(words)
    }

    fn table<'v, 'i>(&mut self, key: &str, value: &'v Value<'i>) -> Option<&'v DeTable<'i>> {
        match value.get_ref() {
            DeValue::Table(table) => Some(table),
            _ => {
                self.wrong_type(key, value, Expected::Table);
                None
            }
        }
    }

    /// A plain string, or a table of strings by language code.
    fn text(&mut self, key: &str, value: &Value<'_>) -> Option<Localized> {
        let text = match value.get_ref() {
            DeValue::String(text) => Localized::plain(text.to_string()),
            DeValue::Table(table) => {
                let mut translations = Vec::with_capacity(table.len());
                for (code, text) in table {
                    let inner = format!("{key}.{}", code.get_ref());
                    if let Some(text) = self.string(&inner, text) {
                        translations.push((code.get_ref().to_string(), text));
                    }
                }
                if translations.len() != table.len() {
                    return None;
                }
                let english = translations.iter().find(|(code, _)| code == "en").or(translations.first());
                let Some((_, default)) = english else {
                    self.report(value.span().start, DiagnosticKind::InvalidValue { key: key.to_owned() });
                    return None;
                };
                Localized { default: default.clone(), translations }
            }
            _ => {
                self.wrong_type(key, value, Expected::Text);
                return None;
            }
        };
        if text.all().any(|form| form.trim().is_empty()) {
            self.report(value.span().start, DiagnosticKind::InvalidValue { key: key.to_owned() });
            return None;
        }
        Some(text)
    }

    /// A path, with a leading `~` put in place of the home folder.
    fn path(&mut self, key: &str, value: &Value<'_>, home: Option<&Path>) -> Option<PathBuf> {
        let text = self.non_empty_string(key, value)?;
        let rest = if text == "~" { Some("") } else { text.strip_prefix("~/") };
        match (rest, home) {
            (None, _) => Some(PathBuf::from(text)),
            (Some(rest), Some(home)) => Some(if rest.is_empty() { home.to_path_buf() } else { home.join(rest) }),
            (Some(_), None) => {
                self.report(value.span().start, DiagnosticKind::NoHome { key: key.to_owned() });
                None
            }
        }
    }

    fn env(&mut self, value: &Value<'_>) -> Vec<(String, String)> {
        let Some(table) = self.table("env", value) else {
            return Vec::new();
        };
        let mut pairs = Vec::with_capacity(table.len());
        for (name, setting) in table {
            let key = format!("env.{}", name.get_ref());
            if name.get_ref().is_empty() || name.get_ref().contains(['=', '\0']) {
                self.report(name.span().start, DiagnosticKind::InvalidValue { key });
                continue;
            }
            if let Some(setting) = self.string(&key, setting) {
                pairs.push((name.get_ref().to_string(), setting));
            }
        }
        pairs
    }

    fn category(&mut self, value: &Value<'_>) -> Category {
        let Some(name) = self.string("category", value) else {
            return Category::Other;
        };
        Category::from_name(&name).unwrap_or_else(|| {
            self.report(value.span().start, DiagnosticKind::UnknownCategory { value: name });
            Category::Other
        })
    }

    fn window(&mut self, value: &Value<'_>) -> WindowPrefs {
        let mut prefs = WindowPrefs::default();
        let Some(table) = self.table("window", value) else {
            return prefs;
        };
        for (key, setting) in table {
            match key.get_ref().as_ref() {
                "size" => prefs.size = self.size(setting),
                "maximized" => prefs.maximized = self.boolean("window.maximized", setting).unwrap_or(false),
                other => {
                    let key_start = key.span().start;
                    self.report(key_start, DiagnosticKind::UnknownField { key: format!("window.{other}") });
                }
            }
        }
        prefs
    }

    fn size(&mut self, value: &Value<'_>) -> Option<(u16, u16)> {
        let number = |item: &Value<'_>| match item.get_ref() {
            DeValue::Integer(integer) => Some(i64::from_str_radix(integer.as_str(), integer.radix()).ok()),
            _ => None,
        };
        let numbers: Option<Vec<Option<i64>>> = match value.get_ref() {
            DeValue::Array(items) if items.len() == 2 => items.iter().map(number).collect(),
            _ => None,
        };
        let Some(numbers) = numbers else {
            self.wrong_type("window.size", value, Expected::Size);
            return None;
        };
        let fit = |number: Option<i64>| number.and_then(|n| u16::try_from(n).ok()).filter(|n| *n > 0);
        match (fit(numbers[0]), fit(numbers[1])) {
            (Some(columns), Some(rows)) => Some((columns, rows)),
            _ => {
                self.report(value.span().start, DiagnosticKind::InvalidValue { key: "window.size".to_owned() });
                None
            }
        }
    }

    fn install(&mut self, value: &Value<'_>) -> Install {
        let mut install = Install::default();
        let Some(table) = self.table("install", value) else {
            return install;
        };
        for (key, setting) in table {
            match key.get_ref().as_ref() {
                "qpac" => install.qpac = self.non_empty_string("install.qpac", setting),
                "quvyta" => install.quvyta = self.non_empty_string("install.quvyta", setting),
                other => {
                    let key_start = key.span().start;
                    self.report(key_start, DiagnosticKind::UnknownField { key: format!("install.{other}") });
                }
            }
        }
        install
    }
}

/// The value of `key` in `table`.
fn field<'t, 'i>(table: &'t DeTable<'i>, key: &str) -> Option<&'t Value<'i>> {
    table.iter().find(|(name, _)| name.get_ref() == key).map(|(_, value)| value)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/ada";

    fn parse(text: &str) -> (Option<Declared>, Vec<Diagnostic>) {
        parse_entry("htop", Path::new("/apps/htop.toml"), text.as_bytes(), Source::User, Some(Path::new(HOME)))
    }

    fn load(text: &str) -> Entry {
        match parse(text) {
            (Some(Declared::Entry(entry)), _) => *entry,
            other => panic!("expected an entry, got {other:?}"),
        }
    }

    fn kinds(text: &str) -> Vec<DiagnosticKind> {
        parse(text).1.into_iter().map(|diagnostic| diagnostic.kind).collect()
    }

    fn broken(text: &str) -> Vec<Diagnostic> {
        let (declared, diagnostics) = parse(text);
        assert_eq!(declared, None, "a broken file declares nothing");
        assert!(!diagnostics.is_empty(), "a broken file says why");
        diagnostics
    }

    #[test]
    fn the_design_example_loads() {
        let entry = load(
            r#"
name = "htop"
comment = { en = "Processes and system load", tr = "Süreçler ve sistem yükü" }
icon = "gauge"
command = ["htop"]
category = "system"
"#,
        );
        assert_eq!(entry.id, "htop");
        assert_eq!(entry.name.get("tr"), "htop");
        assert_eq!(entry.comment.as_ref().map(|comment| comment.get("tr")), Some("Süreçler ve sistem yükü"));
        assert_eq!(entry.icon.as_deref(), Some("gauge"));
        assert_eq!(entry.launch, Launch::Command(vec!["htop".to_owned()]));
        assert_eq!(entry.category, Category::System);
        assert_eq!(entry.source, Source::User);
        assert_eq!(entry.file.as_deref(), Some(Path::new("/apps/htop.toml")));
        assert!(!entry.single && !entry.close_on_exit);
    }

    #[test]
    fn every_field_is_read() {
        let entry = load(
            r#"
name = { en = "Editor", tr = "Düzenleyici" }
command = ["vim", "-p"]
folder = "~/notes"
env = { EDITOR = "vim", LANG = "tr_TR.UTF-8" }
category = "development"
keywords = ["text", "edit"]
single = true
close_on_exit = true

[window]
size = [100, 30]
maximized = true

[install]
qpac = "vim"
quvyta = "quvyta-code"
"#,
        );
        assert_eq!(entry.name.get("en"), "Editor");
        assert_eq!(entry.name.get("tr"), "Düzenleyici");
        assert_eq!(entry.launch.program(), Some("vim"));
        assert_eq!(entry.folder, Some(PathBuf::from("/home/ada/notes")));
        assert_eq!(
            entry.env,
            vec![("EDITOR".to_owned(), "vim".to_owned()), ("LANG".to_owned(), "tr_TR.UTF-8".to_owned())]
        );
        assert_eq!(entry.category, Category::Development);
        assert_eq!(entry.keywords, vec!["text", "edit"]);
        assert!(entry.single && entry.close_on_exit);
        assert_eq!(entry.window, WindowPrefs { size: Some((100, 30)), maximized: true });
        assert_eq!(entry.install.qpac.as_deref(), Some("vim"));
        assert_eq!(entry.install.quvyta.as_deref(), Some("quvyta-code"));
    }

    #[test]
    fn open_expands_the_home_folder() {
        let entry = load("name = \"Notes\"\nopen = \"~\"\n");
        assert_eq!(entry.launch, Launch::Open(PathBuf::from(HOME)));
        let entry = load("name = \"Logs\"\nopen = \"/var/log\"\n");
        assert_eq!(entry.launch, Launch::Open(PathBuf::from("/var/log")));
        // Only `~` and `~/` are the home folder; `~ada` is an ordinary name.
        let entry = load("name = \"x\"\nopen = \"~ada/x\"\n");
        assert_eq!(entry.launch, Launch::Open(PathBuf::from("~ada/x")));
    }

    #[test]
    fn tilde_without_a_home_is_a_warning_for_folder_and_an_error_for_open() {
        let parse = |text: &str| parse_entry("x", Path::new("/x.toml"), text.as_bytes(), Source::User, None);
        let (declared, diagnostics) = parse("name = \"x\"\ncommand = [\"x\"]\nfolder = \"~/w\"\n");
        let Some(Declared::Entry(entry)) = declared else { panic!("the entry still loads") };
        assert_eq!(entry.folder, None);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::NoHome { key: "folder".to_owned() });
        let (declared, diagnostics) = parse("name = \"x\"\nopen = \"~/w\"\n");
        assert_eq!(declared, None);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::NoHome { key: "open".to_owned() });
    }

    #[test]
    fn broken_toml_points_at_the_place() {
        let diagnostics = broken("name = \"htop\"\ncommand = [\"htop\"\n");
        assert_eq!(diagnostics[0].kind, DiagnosticKind::Syntax);
        assert!(diagnostics[0].detail.as_deref().is_some_and(|detail| !detail.is_empty()));
        assert_eq!(diagnostics[0].position.map(|position| position.line), Some(2));
    }

    #[test]
    fn invalid_utf8_points_at_the_first_bad_byte() {
        let (declared, diagnostics) =
            parse_entry("x", Path::new("/x.toml"), b"name = \"x\"\nicon = \"\xff\"\n", Source::User, None);
        assert_eq!(declared, None);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::NotUtf8);
        assert_eq!(diagnostics[0].position, Some(Position { line: 2, column: 9 }));
    }

    #[test]
    fn a_missing_name_is_an_error() {
        let diagnostics = broken("command = [\"htop\"]\n");
        assert_eq!(diagnostics[0].kind, DiagnosticKind::MissingName);
        assert_eq!(diagnostics[0].location(), "/apps/htop.toml:1:1");
    }

    #[test]
    fn empty_names_are_errors() {
        broken("name = \"\"\ncommand = [\"x\"]\n");
        broken("name = {}\ncommand = [\"x\"]\n");
        broken("name = { en = \"x\", tr = \" \" }\ncommand = [\"x\"]\n");
    }

    #[test]
    fn command_and_open_exactly_one() {
        assert_eq!(broken("name = \"x\"\n")[0].kind, DiagnosticKind::MissingLaunch);
        let both = broken("name = \"x\"\ncommand = [\"x\"]\nopen = \"/tmp\"\n");
        assert_eq!(both[0].kind, DiagnosticKind::ConflictingLaunch);
        assert_eq!(both[0].position, Some(Position { line: 3, column: 8 }));
    }

    #[test]
    fn an_empty_command_is_an_error() {
        let kinds = |text| broken(text).into_iter().map(|diagnostic| diagnostic.kind).collect::<Vec<_>>();
        assert_eq!(kinds("name = \"x\"\ncommand = []\n"), vec![DiagnosticKind::InvalidValue { key: "command".into() }]);
        assert_eq!(
            kinds("name = \"x\"\ncommand = [\"\"]\n"),
            vec![DiagnosticKind::InvalidValue { key: "command".into() }]
        );
    }

    #[test]
    fn wrong_types_name_the_field_and_the_expected_kind() {
        let wrong = |text: &str, key: &str, expected: Expected| {
            let diagnostics = broken(text);
            assert!(
                diagnostics.iter().any(|d| d.kind == DiagnosticKind::WrongType { key: key.to_owned(), expected }),
                "{text}: {diagnostics:?}"
            );
        };
        wrong("name = 3\ncommand = [\"x\"]\n", "name", Expected::Text);
        wrong("name = { en = 3 }\ncommand = [\"x\"]\n", "name.en", Expected::String);
        wrong("name = \"x\"\ncommand = \"htop\"\n", "command", Expected::StringList);
        wrong("name = \"x\"\ncommand = [\"htop\", 1]\n", "command", Expected::StringList);
        wrong("name = \"x\"\nopen = [\"/\"]\n", "open", Expected::String);
        wrong("name = \"x\"\ncommand = [\"x\"]\nsingle = \"yes\"\n", "single", Expected::Boolean);
        wrong("name = \"x\"\ncommand = [\"x\"]\nenv = [\"A=b\"]\n", "env", Expected::Table);
        wrong("name = \"x\"\ncommand = [\"x\"]\nenv = { A = 1 }\n", "env.A", Expected::String);
        wrong("name = \"x\"\ncommand = [\"x\"]\nkeywords = \"top\"\n", "keywords", Expected::StringList);
        wrong("name = \"x\"\ncommand = [\"x\"]\ncategory = 1\n", "category", Expected::String);
        wrong("name = \"x\"\ncommand = [\"x\"]\nwindow = 1\n", "window", Expected::Table);
        wrong("name = \"x\"\ncommand = [\"x\"]\nwindow = { size = [80] }\n", "window.size", Expected::Size);
        wrong("name = \"x\"\ncommand = [\"x\"]\nwindow = { size = [80, 2.5] }\n", "window.size", Expected::Size);
        wrong("name = \"x\"\ncommand = [\"x\"]\nwindow = { maximized = 1 }\n", "window.maximized", Expected::Boolean);
        wrong("name = \"x\"\ncommand = [\"x\"]\ninstall = { qpac = [] }\n", "install.qpac", Expected::String);
        wrong("name = \"x\"\ncommand = [\"x\"]\nhidden = \"no\"\n", "hidden", Expected::Boolean);
    }

    #[test]
    fn window_sizes_must_fit_a_screen() {
        for size in ["[0, 30]", "[80, -1]", "[70000, 30]"] {
            let diagnostics = broken(&format!("name = \"x\"\ncommand = [\"x\"]\nwindow = {{ size = {size} }}\n"));
            assert_eq!(diagnostics[0].kind, DiagnosticKind::InvalidValue { key: "window.size".to_owned() });
        }
        let entry = load("name = \"x\"\ncommand = [\"x\"]\nwindow = { size = [0x50, 24] }\n");
        assert_eq!(entry.window.size, Some((80, 24)));
    }

    #[test]
    fn unknown_fields_are_warnings_and_the_entry_loads() {
        let text = "name = \"x\"\ncommand = [\"x\"]\nopens = [\"pdf\"]\n[window]\nx = 1\n[install]\napt = \"x\"\n";
        let (declared, diagnostics) = parse(text);
        assert!(matches!(declared, Some(Declared::Entry(_))));
        let kinds: Vec<_> = diagnostics.iter().map(|diagnostic| diagnostic.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                DiagnosticKind::UnknownField { key: "opens".into() },
                DiagnosticKind::UnknownField { key: "window.x".into() },
                DiagnosticKind::UnknownField { key: "install.apt".into() },
            ]
        );
        assert!(diagnostics.iter().all(Diagnostic::is_warning));
        assert_eq!(diagnostics[0].position, Some(Position { line: 3, column: 1 }));
    }

    #[test]
    fn an_unknown_category_is_a_warning_and_goes_to_other() {
        assert_eq!(
            kinds("name = \"x\"\ncommand = [\"x\"]\ncategory = \"games\"\n"),
            vec![DiagnosticKind::UnknownCategory { value: "games".into() }]
        );
        assert_eq!(load("name = \"x\"\ncommand = [\"x\"]\ncategory = \"games\"\n").category, Category::Other);
        assert_eq!(load("name = \"x\"\ncommand = [\"x\"]\n").category, Category::Other);
    }

    #[test]
    fn localized_names_fall_back_to_english() {
        let entry = load("name = { tr = \"Dosyalar\", en = \"Files\" }\nopen = \"/\"\n");
        assert_eq!(entry.name.default, "Files");
        assert_eq!(entry.name.get("tr"), "Dosyalar");
        assert_eq!(entry.name.get("de"), "Files");
        let without_english = load("name = { tr = \"Dosyalar\" }\nopen = \"/\"\n");
        assert_eq!(without_english.name.get("de"), "Dosyalar");
    }

    #[test]
    fn hidden_needs_nothing_else() {
        assert_eq!(parse("hidden = true\n"), (Some(Declared::Hidden), Vec::new()));
        assert!(matches!(parse("hidden = false\nname = \"x\"\ncommand = [\"x\"]\n").0, Some(Declared::Entry(_))));
    }

    #[test]
    fn screens_belong_to_built_in_entries() {
        let text = "name = \"Terminal\"\nscreen = \"terminal\"\n";
        let (declared, diagnostics) =
            parse_entry("terminal", Path::new("terminal.toml"), text.as_bytes(), Source::Builtin, None);
        let Some(Declared::Entry(entry)) = declared else { panic!("built-in entries may name a screen") };
        assert_eq!(entry.launch, Launch::Screen(Screen::Terminal));
        assert_eq!(entry.file, None);
        assert!(diagnostics.is_empty());
        // In a user's file the field is unknown, and without a command there is nothing to start.
        let kinds = kinds(text);
        assert!(kinds.contains(&DiagnosticKind::UnknownField { key: "screen".into() }));
        assert!(kinds.contains(&DiagnosticKind::MissingLaunch));
        let text = "name = \"x\"\nscreen = \"clock\"\n";
        let (declared, _) = parse_entry("x", Path::new("x.toml"), text.as_bytes(), Source::Builtin, None);
        assert_eq!(declared, None);
    }
}
