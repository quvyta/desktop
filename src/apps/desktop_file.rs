//! The system's `.desktop` files, read for their terminal programs.
//!
//! A program installed with the system's package manager usually brings a `.desktop` file.
//! Those that say `Terminal=true` (htop, vim, btop) become entries without anyone writing one.
//! The files are only read, never written.

use std::path::Path;

use super::diagnostic::{Diagnostic, DiagnosticKind, Position};
use super::entry::{Entry, Install, Launch, Source, WindowPrefs};
use super::{Category, Localized};

/// What a `.desktop` file means for the desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopFile {
    /// A terminal program to show.
    Entry(Box<Entry>),
    /// `Hidden=true` or `NoDisplay=true`: the file hides its id from the files below it.
    Hidden,
    /// A program with its own window, or not an application: not for this desktop. Like a hidden
    /// one it still hides its id from the files below it.
    NotTerminal,
    /// A terminal program whose file cannot be used; the diagnostics say why.
    Broken,
}

/// Reads the `.desktop` file `bytes` of id `id`, read from `file`.
///
/// Only a file that describes a terminal application is checked closely, so the many files of
/// windowed programs never produce diagnostics. Lines that cannot be read are skipped with a
/// warning; a file without a name or a command, or with a command that cannot be split, is
/// broken.
#[must_use]
pub fn parse_desktop_file(id: &str, file: &Path, bytes: &[u8]) -> (DesktopFile, Vec<Diagnostic>) {
    let text = String::from_utf8_lossy(bytes);
    let parsed = Group::read(&text);
    let mut diagnostics = Vec::new();
    let mut report = |offset: usize, kind: DiagnosticKind| {
        diagnostics.push(Diagnostic::at(file, Some(Position::of_offset(&text, offset)), kind));
    };
    let Some(group) = parsed else {
        report(0, DiagnosticKind::MissingGroup);
        return (DesktopFile::Broken, diagnostics);
    };
    if group.flag("Hidden") {
        return (DesktopFile::Hidden, diagnostics);
    }
    if group.value("Type").map(|(value, _)| value) != Some("Application") || !group.flag("Terminal") {
        return (DesktopFile::NotTerminal, diagnostics);
    }
    if group.flag("NoDisplay") {
        return (DesktopFile::Hidden, diagnostics);
    }

    for offset in &group.malformed {
        report(*offset, DiagnosticKind::MalformedLine);
    }
    let mut broken = false;
    if let Err(error) = std::str::from_utf8(bytes) {
        let valid = String::from_utf8_lossy(&bytes[..error.valid_up_to()]);
        report(valid.len(), DiagnosticKind::NotUtf8);
        broken = true;
    }
    let name = group.localized("Name");
    if name.is_none() {
        report(group.start, DiagnosticKind::MissingKey { key: "Name".to_owned() });
        broken = true;
    }
    let words = match group.value("Exec") {
        None => {
            report(group.start, DiagnosticKind::MissingKey { key: "Exec".to_owned() });
            None
        }
        Some((exec, offset)) => {
            let words = split_exec(&unescape(exec)).filter(|words| words.first().is_some_and(|word| !word.is_empty()));
            if words.is_none() {
                report(offset, DiagnosticKind::BadExec);
            }
            words
        }
    };
    let (Some(name), Some(words), false) = (name, words, broken) else {
        return (DesktopFile::Broken, diagnostics);
    };

    let mut keywords = Vec::new();
    for (key, value, _) in &group.pairs {
        if *key == "Keywords" || key.starts_with("Keywords[") {
            for word in split_list(value) {
                if !keywords.contains(&word) {
                    keywords.push(word);
                }
            }
        }
    }
    let categories = group.value("Categories").map(|(value, _)| split_list(value)).unwrap_or_default();
    let entry = Entry {
        id: id.to_owned(),
        name,
        comment: group.localized("Comment"),
        icon: None,
        launch: Launch::Command(words),
        folder: group.value("Path").map(|(value, _)| unescape(value)).filter(|path| !path.is_empty()).map(Into::into),
        env: Vec::new(),
        unset: Vec::new(),
        category: category_of(&categories),
        keywords,
        single: false,
        close_on_exit: false,
        window: WindowPrefs::default(),
        install: Install::default(),
        try_exec: group.value("TryExec").map(|(value, _)| unescape(value)).filter(|program| !program.is_empty()),
        source: Source::DesktopFile,
        file: Some(file.to_path_buf()),
    };
    (DesktopFile::Entry(Box::new(entry)), diagnostics)
}

/// The `[Desktop Entry]` group of a file.
struct Group<'t> {
    /// Where the group's header starts.
    start: usize,
    /// Every key in order, with its value and where the value starts. A key given twice keeps
    /// its first value, as the specification allows only one.
    pairs: Vec<(&'t str, &'t str, usize)>,
    /// Where each line that could not be read starts.
    malformed: Vec<usize>,
}

impl<'t> Group<'t> {
    /// The group, or `None` when the file has none.
    fn read(text: &'t str) -> Option<Self> {
        let mut group: Option<Self> = None;
        let mut inside = false;
        let mut offset = 0;
        for line in text.split('\n') {
            let start = offset;
            offset += line.len() + 1;
            let line = line.strip_suffix('\r').unwrap_or(line);
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with('[') {
                // Only the first `[Desktop Entry]` counts; actions and other groups are not ours.
                inside = trimmed == "[Desktop Entry]" && group.is_none();
                if inside {
                    group = Some(Self { start, pairs: Vec::new(), malformed: Vec::new() });
                }
                continue;
            }
            let Some(current) = group.as_mut().filter(|_| inside) else {
                continue;
            };
            match line.split_once('=') {
                Some((key, value)) if valid_key(key.trim()) => {
                    let key = key.trim();
                    let value_start = start + line.len() - value.trim_start().len();
                    if !current.pairs.iter().any(|(seen, _, _)| *seen == key) {
                        current.pairs.push((key, value.trim(), value_start));
                    }
                }
                _ => current.malformed.push(start),
            }
        }
        group
    }

    /// The raw value of `key` and where it starts.
    fn value(&self, key: &str) -> Option<(&'t str, usize)> {
        self.pairs.iter().find(|(name, _, _)| *name == key).map(|(_, value, offset)| (*value, *offset))
    }

    /// Whether boolean `key` is `true`.
    fn flag(&self, key: &str) -> bool {
        self.value(key).is_some_and(|(value, _)| value.trim() == "true")
    }

    /// `key` with its translations, `Name` and `Name[tr]`, when the plain key is given.
    fn localized(&self, key: &str) -> Option<Localized> {
        let default = unescape(self.value(key)?.0);
        if default.trim().is_empty() {
            return None;
        }
        let translations = self
            .pairs
            .iter()
            .filter_map(|(name, value, _)| {
                let code = name.strip_prefix(key)?.strip_prefix('[')?.strip_suffix(']')?;
                let text = unescape(value);
                (!code.is_empty() && !text.trim().is_empty()).then(|| (code.to_owned(), text))
            })
            .collect();
        Some(Localized { default, translations })
    }
}

/// A key is letters, digits and `-`, with an optional `[locale]` after it.
fn valid_key(key: &str) -> bool {
    let (name, locale) = match key.split_once('[') {
        Some((name, rest)) => (name, rest.strip_suffix(']')),
        None => (key, Some("")),
    };
    let Some(locale) = locale else {
        return false;
    };
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && locale.chars().all(|c| c.is_ascii_alphanumeric() || "_@.-".contains(c))
}

/// A string value with its escapes read: `\s`, `\n`, `\t`, `\r` and `\\`. Any other backslash
/// stays as it is.
fn unescape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// A list value: items separated by `;`, where `\;` is a `;` inside an item. Empty items are
/// left out.
fn split_list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => match chars.next() {
                Some(';') => current.push(';'),
                Some(other) => {
                    current.push('\\');
                    current.push(other);
                }
                None => current.push('\\'),
            },
            ';' => items.push(std::mem::take(&mut current)),
            _ => current.push(c),
        }
    }
    items.push(current);
    items.into_iter().map(|item| unescape(item.trim())).filter(|item| !item.is_empty()).collect()
}

/// Splits an `Exec` value into the program and its arguments, as the desktop entry
/// specification quotes them. Field codes (`%f`, `%U` and the others) are dropped, since the
/// desktop starts programs without files; `%%` is a `%`. `None` when a quote is not closed or a
/// quoted backslash escapes something other than `"`, `` ` ``, `$` or `\`.
fn split_exec(exec: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    // Whether the word has begun: a quoted empty word is still a word.
    let mut started = false;
    let mut had_code = false;
    let mut quoted = false;
    let mut chars = exec.chars();
    let mut finish = |word: &mut String, started: &mut bool, had_code: &mut bool| {
        if !word.is_empty() || (*started && !*had_code) {
            words.push(std::mem::take(word));
        }
        word.clear();
        *started = false;
        *had_code = false;
    };
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => match chars.next() {
                Some(escaped @ ('"' | '`' | '$' | '\\')) => word.push(escaped),
                _ => return None,
            },
            ' ' | '\t' | '\n' if !quoted => finish(&mut word, &mut started, &mut had_code),
            '%' => match chars.next() {
                Some('%') => {
                    word.push('%');
                    started = true;
                }
                // Every other code, including the deprecated ones, stands for something the
                // desktop does not pass.
                _ => had_code = true,
            },
            _ => {
                word.push(c);
                started = true;
            }
        }
    }
    if quoted {
        return None;
    }
    finish(&mut word, &mut started, &mut had_code);
    Some(words)
}

/// Our category for a file's `Categories`. The more specific kinds win: a file manager that also
/// says `System` is a file manager.
fn category_of(categories: &[String]) -> Category {
    const MAP: [(&[&str], Category); 6] = [
        (&["FileManager", "FileTools"], Category::Files),
        (&["AudioVideo", "Audio", "Video", "Graphics"], Category::Media),
        (&["Network"], Category::Network),
        (&["Office"], Category::Office),
        (&["Development", "TextEditor", "IDE"], Category::Development),
        (&["System", "Monitor", "Settings"], Category::System),
    ];
    MAP.iter()
        .find(|(names, _)| categories.iter().any(|category| names.contains(&category.as_str())))
        .map_or(Category::Other, |(_, category)| *category)
}

#[cfg(test)]
mod tests {
    use super::*;

    const HTOP: &str = "[Desktop Entry]
Type=Application
Version=1.0
Name=Htop
Name[tr]=Htop süreç görüntüleyici
GenericName=Process Viewer
Comment=Show System Processes
Comment[de]=Systemprozesse anzeigen
Icon=htop
Exec=htop
Terminal=true
Categories=ConsoleOnly;System;Monitor;
Keywords=system;process;task
Keywords[tr]=süreç;görev
";

    fn parse(text: &str) -> (DesktopFile, Vec<Diagnostic>) {
        parse_desktop_file("htop", Path::new("/usr/share/applications/htop.desktop"), text.as_bytes())
    }

    fn entry(text: &str) -> Entry {
        match parse(text) {
            (DesktopFile::Entry(entry), _) => *entry,
            other => panic!("expected an entry, got {other:?}"),
        }
    }

    #[test]
    fn a_terminal_program_becomes_an_entry() {
        let (DesktopFile::Entry(entry), diagnostics) = parse(HTOP) else { panic!("htop is a terminal program") };
        assert!(diagnostics.is_empty());
        assert_eq!(entry.id, "htop");
        assert_eq!(entry.name.get("en"), "Htop");
        assert_eq!(entry.name.get("tr"), "Htop süreç görüntüleyici");
        assert_eq!(entry.comment.as_ref().map(|comment| comment.get("de_DE")), Some("Systemprozesse anzeigen"));
        assert_eq!(entry.launch, Launch::Command(vec!["htop".to_owned()]));
        assert_eq!(entry.category, Category::System);
        assert_eq!(entry.keywords, vec!["system", "process", "task", "süreç", "görev"]);
        assert_eq!(entry.source, Source::DesktopFile);
        assert_eq!(entry.icon, None);
    }

    #[test]
    fn programs_with_their_own_window_are_skipped_quietly() {
        let text = HTOP.replace("Terminal=true", "Terminal=false");
        assert_eq!(parse(&text), (DesktopFile::NotTerminal, Vec::new()));
        let text = HTOP.replace("Terminal=true\n", "");
        assert_eq!(parse(&text), (DesktopFile::NotTerminal, Vec::new()));
        let text = HTOP.replace("Type=Application", "Type=Link");
        assert_eq!(parse(&text), (DesktopFile::NotTerminal, Vec::new()));
        // Broken lines in a file that is not ours are not reported.
        let text = "[Desktop Entry]\nType=Application\nName=Gimp\nnot a line\nExec=gimp %U\n";
        assert_eq!(parse(text), (DesktopFile::NotTerminal, Vec::new()));
    }

    #[test]
    fn hidden_and_no_display_files_hide_their_id() {
        assert_eq!(parse(&format!("{HTOP}NoDisplay=true\n")).0, DesktopFile::Hidden);
        assert_eq!(parse(&format!("{HTOP}Hidden=true\n")).0, DesktopFile::Hidden);
        assert!(matches!(parse(&format!("{HTOP}NoDisplay=false\n")).0, DesktopFile::Entry(_)));
    }

    #[test]
    fn field_codes_are_dropped_and_percent_percent_is_kept() {
        let exec = |line: &str| entry(&HTOP.replace("Exec=htop", line)).launch;
        let command = |words: &[&str]| Launch::Command(words.iter().map(|word| (*word).to_owned()).collect());
        assert_eq!(exec("Exec=vim %F"), command(&["vim"]));
        assert_eq!(exec("Exec=vim %f %u %U %d %D %n %N %i %c %k %v %m"), command(&["vim"]));
        assert_eq!(exec("Exec=printf 100%% %u"), command(&["printf", "100%"]));
        assert_eq!(exec("Exec=tool --file=%f -x"), command(&["tool", "--file=", "-x"]));
    }

    #[test]
    fn exec_quoting_follows_the_specification() {
        let exec = |line: &str| entry(&HTOP.replace("Exec=htop", line)).launch;
        let command = |words: &[&str]| Launch::Command(words.iter().map(|word| (*word).to_owned()).collect());
        assert_eq!(exec(r#"Exec="/opt/my tools/top" -d 5"#), command(&["/opt/my tools/top", "-d", "5"]));
        // The file's own escapes come first: `\\` is one backslash, which then escapes the quote.
        assert_eq!(exec(r#"Exec=sh -c "echo \\"hi\\" \\$HOME""#), command(&["sh", "-c", r#"echo "hi" $HOME"#]));
        assert_eq!(exec(r#"Exec=tool "" last"#), command(&["tool", "", "last"]));
        assert_eq!(exec("Exec=tool\\sname"), command(&["tool", "name"]));
    }

    #[test]
    fn bad_exec_quoting_breaks_the_file() {
        for line in [r#"Exec="unclosed"#, r#"Exec="bad \\q escape""#, "Exec=%f", "Exec= "] {
            let (outcome, diagnostics) = parse(&HTOP.replace("Exec=htop", line));
            assert_eq!(outcome, DesktopFile::Broken, "{line}");
            assert_eq!(diagnostics[0].kind, DiagnosticKind::BadExec, "{line}");
            assert_eq!(diagnostics[0].position.map(|position| position.line), Some(10), "{line}");
        }
    }

    #[test]
    fn a_terminal_program_needs_a_name_and_a_command() {
        let (outcome, diagnostics) = parse(&HTOP.replace("Name=Htop\n", ""));
        assert_eq!(outcome, DesktopFile::Broken);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::MissingKey { key: "Name".into() });
        let (outcome, diagnostics) = parse(&HTOP.replace("Exec=htop\n", ""));
        assert_eq!(outcome, DesktopFile::Broken);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::MissingKey { key: "Exec".into() });
        assert_eq!(diagnostics[0].location(), "/usr/share/applications/htop.desktop:1:1");
    }

    #[test]
    fn a_file_without_the_group_is_broken() {
        let (outcome, diagnostics) = parse("Name=x\nExec=x\n");
        assert_eq!(outcome, DesktopFile::Broken);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::MissingGroup);
    }

    #[test]
    fn malformed_lines_are_skipped_with_a_warning() {
        let text = HTOP.replace("Version=1.0\n", "Version=1.0\nthis is not a key\nBad Key=1\n");
        let (DesktopFile::Entry(_), diagnostics) = parse(&text) else { panic!("the entry still loads") };
        let lines: Vec<_> = diagnostics.iter().map(|d| (d.kind.clone(), d.position.map(|p| p.line))).collect();
        assert_eq!(lines, vec![(DiagnosticKind::MalformedLine, Some(4)), (DiagnosticKind::MalformedLine, Some(5))]);
    }

    #[test]
    fn other_groups_and_comments_do_not_count() {
        let text = format!("# A comment\n\n{HTOP}\n[Desktop Action new]\nName=New window\nExec=other\nweird line\n");
        let entry = entry(&text);
        assert_eq!(entry.name.default, "Htop");
        assert_eq!(entry.launch.program(), Some("htop"));
    }

    #[test]
    fn crlf_line_ends_are_read() {
        let entry = entry(&HTOP.replace('\n', "\r\n"));
        assert_eq!(entry.name.default, "Htop");
        assert_eq!(entry.launch.program(), Some("htop"));
    }

    #[test]
    fn path_and_try_exec_are_kept() {
        let entry = entry(&format!("{HTOP}Path=/srv\nTryExec=/usr/bin/htop\n"));
        assert_eq!(entry.folder.as_deref(), Some(Path::new("/srv")));
        assert_eq!(entry.try_exec.as_deref(), Some("/usr/bin/htop"));
    }

    #[test]
    fn categories_map_to_ours() {
        let of = |list: &str| category_of(&split_list(list));
        assert_eq!(of("ConsoleOnly;System;FileTools;FileManager;"), Category::Files);
        assert_eq!(of("AudioVideo;Audio;Player;"), Category::Media);
        assert_eq!(of("Network;FileTransfer;"), Category::Network);
        assert_eq!(of("Office;"), Category::Office);
        assert_eq!(of("Utility;TextEditor;"), Category::Development);
        assert_eq!(of("System;Settings;"), Category::System);
        assert_eq!(of("Game;"), Category::Other);
        assert_eq!(of(""), Category::Other);
    }

    #[test]
    fn lists_honour_escaped_separators() {
        assert_eq!(split_list(r"a\;b;c;;"), vec!["a;b", "c"]);
    }

    #[test]
    fn invalid_utf8_in_a_terminal_program_breaks_it() {
        let mut bytes = HTOP.as_bytes().to_vec();
        bytes.extend_from_slice(b"Comment[fr]=\xff\n");
        let (outcome, diagnostics) = parse_desktop_file("htop", Path::new("htop.desktop"), &bytes);
        assert_eq!(outcome, DesktopFile::Broken);
        assert_eq!(diagnostics[0].kind, DiagnosticKind::NotUtf8);
        assert_eq!(diagnostics[0].position, Some(Position { line: 15, column: 13 }));
    }
}
