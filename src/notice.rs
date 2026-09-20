//! Turning what the loaders found into words the person reads.
//!
//! A [`Diagnostic`] carries no sentence: the kind, the file and the place. Here each kind becomes
//! text in the language on screen, so the same problem reads naturally in English and in Turkish,
//! and becomes a notification the person can dismiss. Every notice names its file with its line
//! and column, because the answer to "my entry does not show" is the line to look at.

use qframe::prelude::*;
use qframe::widgets::Toast;

use crate::apps::{Diagnostic, DiagnosticKind, Expected};

/// What a field should have held, in words.
#[must_use]
pub fn expected(expected: Expected) -> String {
    let key = match expected {
        Expected::String => "string",
        Expected::Boolean => "boolean",
        Expected::Table => "table",
        Expected::Text => "text",
        Expected::StringList => "string-list",
        Expected::Size => "size",
    };
    t!(&format!("expected.{key}"))
}

/// What went wrong, in one sentence, without the place.
#[must_use]
pub fn what(kind: &DiagnosticKind) -> String {
    match kind {
        DiagnosticKind::Unreadable => t!("diagnostic.unreadable"),
        DiagnosticKind::NotUtf8 => t!("diagnostic.not-utf8"),
        DiagnosticKind::Syntax => t!("diagnostic.syntax"),
        DiagnosticKind::WrongType { key, expected: wanted } => {
            t!("diagnostic.wrong-type", key = key.as_str(), expected = expected(*wanted).as_str())
        }
        DiagnosticKind::InvalidValue { key } => t!("diagnostic.invalid-value", key = key.as_str()),
        DiagnosticKind::MissingName => t!("diagnostic.missing-name"),
        DiagnosticKind::MissingLaunch => t!("diagnostic.missing-launch"),
        DiagnosticKind::ConflictingLaunch => t!("diagnostic.conflicting-launch"),
        DiagnosticKind::UnknownCategory { value } => t!("diagnostic.unknown-category", value = value.as_str()),
        DiagnosticKind::UnknownField { key } => t!("diagnostic.unknown-field", key = key.as_str()),
        DiagnosticKind::NoHome { key } => t!("diagnostic.no-home", key = key.as_str()),
        DiagnosticKind::MalformedLine => t!("diagnostic.malformed-line"),
        DiagnosticKind::MissingGroup => t!("diagnostic.missing-group"),
        DiagnosticKind::MissingKey { key } => t!("diagnostic.missing-key", key = key.as_str()),
        DiagnosticKind::BadExec => t!("diagnostic.bad-exec"),
    }
}

/// The whole notice: where it is and what it is, with the words of the parser or the system when
/// it had any.
#[must_use]
pub fn sentence(diagnostic: &Diagnostic) -> String {
    let what = what(&diagnostic.kind);
    let mut text = t!("diagnostic.at", place = diagnostic.location().as_str(), what = what.as_str());
    if let Some(detail) = &diagnostic.detail {
        text.push(' ');
        text.push_str(detail);
    }
    text
}

/// What a program that could not be started reads as: which program it was and what the system
/// said about it.
///
/// The system's own words never stand alone. `reason` is whatever the system gave, in whatever
/// language it gave it; the sentence around it names the program it belongs to, so a person reading
/// "No such file or directory" knows what file is meant.
#[must_use]
pub fn start_failed(program: &str, reason: &str) -> String {
    t!("notice.start-failed-why", program = program, reason = reason)
}

/// The heading of the same failure, for the corner: short, and still naming the program.
#[must_use]
pub fn start_failed_title(program: &str) -> String {
    t!("notice.start-failed", program = program)
}

/// The notification one diagnostic becomes: a warning when the entry still loads, and the louder
/// kind when it does not.
#[must_use]
pub fn toast<Msg: Send + 'static>(diagnostic: &Diagnostic) -> Command<Msg> {
    let body = sentence(diagnostic);
    let toast = if diagnostic.is_warning() {
        Toast::info(t!("diagnostic.title-warning")).body(body)
    } else {
        Toast::warning(t!("diagnostic.title-error")).body(body)
    };
    Command::toast(toast)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use qframe::i18n::{I18n, scope};

    use super::*;
    use crate::apps::Position;

    fn english() -> Arc<I18n> {
        let mut i18n = I18n::builtin();
        for (file, text) in crate::locales() {
            assert!(i18n.add_source(file, text), "{file} loads");
        }
        assert!(i18n.set_active("en"));
        Arc::new(i18n)
    }

    #[test]
    fn every_kind_has_words_of_its_own() {
        let kinds = [
            DiagnosticKind::Unreadable,
            DiagnosticKind::NotUtf8,
            DiagnosticKind::Syntax,
            DiagnosticKind::WrongType { key: "window.size".into(), expected: Expected::Size },
            DiagnosticKind::InvalidValue { key: "name".into() },
            DiagnosticKind::MissingName,
            DiagnosticKind::MissingLaunch,
            DiagnosticKind::ConflictingLaunch,
            DiagnosticKind::UnknownCategory { value: "games".into() },
            DiagnosticKind::UnknownField { key: "wallpaper".into() },
            DiagnosticKind::NoHome { key: "folder".into() },
            DiagnosticKind::MalformedLine,
            DiagnosticKind::MissingGroup,
            DiagnosticKind::MissingKey { key: "Exec".into() },
            DiagnosticKind::BadExec,
        ];
        scope(english(), || {
            let mut said = Vec::new();
            for kind in &kinds {
                let words = what(kind);
                assert!(!words.starts_with('⟦'), "{kind:?} has no text: {words}");
                assert!(!said.contains(&words), "{kind:?} says the same as another kind");
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
                assert!(!expected(wanted).starts_with('⟦'), "{wanted:?} has no text");
            }
        });
    }

    #[test]
    fn a_program_that_could_not_be_started_names_the_program_in_both_languages() {
        let i18n = english();
        for language in ["en", "tr"] {
            let mut copy = I18n::builtin();
            for (file, text) in crate::locales() {
                assert!(copy.add_source(file, text), "{file} loads");
            }
            assert!(copy.set_active(language), "{language} is a language of qdesk");
            scope(Arc::new(copy), || {
                let whole = start_failed("htop", "No such file or directory");
                assert!(whole.contains("htop"), "{language}: {whole}");
                assert!(whole.contains("No such file or directory"), "{language}: {whole}");
                assert!(whole.len() > "No such file or directory".len() + 4, "the reason is not alone: {whole}");
                let title = start_failed_title("htop");
                assert!(title.contains("htop") && !title.starts_with('⟦'), "{language}: {title}");
            });
        }
        // The English file is the reference and says both.
        assert!(i18n.has("en", "notice.start-failed") && i18n.has("en", "notice.start-failed-why"));
    }

    #[test]
    fn a_notice_names_the_file_the_line_and_the_column() {
        let diagnostic =
            Diagnostic::at("/a/htop.toml", Some(Position { line: 3, column: 7 }), DiagnosticKind::MissingName)
                .with_detail("expected a name");
        scope(english(), || {
            let sentence = sentence(&diagnostic);
            assert!(sentence.contains("/a/htop.toml:3:7"), "{sentence}");
            assert!(sentence.ends_with("expected a name"), "{sentence}");
        });
    }
}
