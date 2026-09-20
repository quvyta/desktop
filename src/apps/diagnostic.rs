//! Problems found while reading entry files and `.desktop` files.

use std::path::PathBuf;

/// A place in a file. Lines and columns start at 1; columns count characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    /// The line, from 1.
    pub line: usize,
    /// The column in characters, from 1.
    pub column: usize,
}

impl Position {
    /// The position of byte `offset` in `text`. An offset inside a character points at that
    /// character, and one past the end at the end.
    #[must_use]
    pub fn of_offset(text: &str, offset: usize) -> Self {
        let before = &text[..text.floor_char_boundary(offset)];
        let line = before.matches('\n').count() + 1;
        let line_start = before.rfind('\n').map_or(0, |index| index + 1);
        Self { line, column: before[line_start..].chars().count() + 1 }
    }
}

/// The kind of value a field should have held.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Expected {
    /// A string.
    String,
    /// `true` or `false`.
    Boolean,
    /// A table.
    Table,
    /// A string, or a table of strings by language code.
    Text,
    /// An array of strings.
    StringList,
    /// An array of two whole numbers, columns and rows, each from 1 to 65535.
    Size,
}

/// What went wrong. The interface turns each kind into a sentence in the user's language.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum DiagnosticKind {
    /// The file or folder could not be read; `detail` holds the system's reason.
    Unreadable,
    /// The file is not UTF-8; the position is the first byte that is not.
    NotUtf8,
    /// The file is not valid TOML; `detail` holds the parser's message.
    Syntax,
    /// A field holds the wrong kind of value.
    WrongType {
        /// The field, with its table: `name`, `window.size`.
        key: String,
        /// What it should have held.
        expected: Expected,
    },
    /// A field holds a value of the right kind that is not allowed: an empty name, an empty
    /// command, a window size of zero.
    InvalidValue {
        /// The field, with its table.
        key: String,
    },
    /// The entry has no `name`.
    MissingName,
    /// The entry has neither `command` nor `open`, so there is nothing to start.
    MissingLaunch,
    /// The entry has more than one of `command`, `open` and a built-in screen.
    ConflictingLaunch,
    /// `category` names a category qdesk does not know; the entry goes to `other`. A warning.
    UnknownCategory {
        /// The category the file names.
        value: String,
    },
    /// A field qdesk does not know; the entry still loads, since a newer qdesk may know it. A
    /// warning.
    UnknownField {
        /// The field, with its table.
        key: String,
    },
    /// A path starts with `~` but there is no home folder to put in its place; the field is
    /// left out. A warning.
    NoHome {
        /// The field, with its table.
        key: String,
    },
    /// A `.desktop` line that is neither a group, a key and value, a comment nor empty; the line
    /// is skipped. A warning.
    MalformedLine,
    /// A `.desktop` file with no `[Desktop Entry]` group.
    MissingGroup,
    /// A `.desktop` file without a key it needs.
    MissingKey {
        /// The key: `Name` or `Exec`.
        key: String,
    },
    /// A `.desktop` `Exec` value whose quoting cannot be read: an unclosed quote or an escape the
    /// specification does not allow.
    BadExec,
}

impl DiagnosticKind {
    /// Whether the entry still loads despite the problem.
    #[must_use]
    pub fn is_warning(&self) -> bool {
        matches!(
            self,
            Self::UnknownCategory { .. } | Self::UnknownField { .. } | Self::NoHome { .. } | Self::MalformedLine
        )
    }
}

/// One problem in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// The file or folder, or for a built-in entry its name inside qdesk.
    pub path: PathBuf,
    /// Where in the file, when the problem has a place.
    pub position: Option<Position>,
    /// What went wrong.
    pub kind: DiagnosticKind,
    /// The words of the parser or the system, when the kind alone does not say it all. They are
    /// in English and shown as they are.
    pub detail: Option<String>,
}

impl Diagnostic {
    /// A problem at `position` of `path`.
    #[must_use]
    pub fn at(path: impl Into<PathBuf>, position: Option<Position>, kind: DiagnosticKind) -> Self {
        Self { path: path.into(), position, kind, detail: None }
    }

    /// The same problem with the parser's or the system's words.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Whether the entry still loads despite the problem.
    #[must_use]
    pub fn is_warning(&self) -> bool {
        self.kind.is_warning()
    }

    /// `file:line:column`, or the file alone when the problem has no place.
    #[must_use]
    pub fn location(&self) -> String {
        match self.position {
            Some(position) => format!("{}:{}:{}", self.path.display(), position.line, position.column),
            None => self.path.display().to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positions_count_lines_and_characters() {
        let text = "a = 1\nnäme = 2\n";
        assert_eq!(Position::of_offset(text, 0), Position { line: 1, column: 1 });
        assert_eq!(Position::of_offset(text, 6), Position { line: 2, column: 1 });
        // `ä` is two bytes: the byte after it is the third character.
        assert_eq!(Position::of_offset(text, 9), Position { line: 2, column: 3 });
        // Inside `ä`, and past the end.
        assert_eq!(Position::of_offset(text, 8), Position { line: 2, column: 2 });
        assert_eq!(Position::of_offset(text, 999), Position { line: 3, column: 1 });
    }

    #[test]
    fn location_reads_file_line_column() {
        let at = Diagnostic::at("/a/htop.toml", Some(Position { line: 3, column: 7 }), DiagnosticKind::MissingName);
        assert_eq!(at.location(), "/a/htop.toml:3:7");
        assert_eq!(Diagnostic::at("/a", None, DiagnosticKind::Unreadable).location(), "/a");
    }

    #[test]
    fn only_softer_kinds_are_warnings() {
        assert!(DiagnosticKind::UnknownField { key: "x".into() }.is_warning());
        assert!(DiagnosticKind::MalformedLine.is_warning());
        assert!(!DiagnosticKind::Syntax.is_warning());
        assert!(!DiagnosticKind::MissingName.is_warning());
    }
}
