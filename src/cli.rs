//! The command line: `--help` and `--version` answer and exit; no argument opens the desktop.
//!
//! [`run`] is what the binaries call, in the system's language. [`execute`] underneath takes the
//! language as a value, so a test can check the text in each one.

use std::io::{self, Write};
use std::sync::Arc;

use qframe::i18n::{I18n, scope};
use qframe::t;

use crate::locales;

/// The command did what was asked.
pub const EXIT_OK: u8 = 0;
/// The output could not be written.
pub const EXIT_FAILED: u8 = 1;
/// The arguments made no sense.
pub const EXIT_USAGE: u8 = 2;

/// What the arguments ask for, besides opening the desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Print the usage text.
    Help,
    /// Print the program name and version.
    Version,
    /// An argument that is not known.
    Unknown(String),
}

/// Reads the arguments after the program name; `None` means there are none and the desktop
/// opens. The first argument decides: qdesk takes nothing besides `--help` and `--version`, so
/// anything else is refused rather than ignored.
#[must_use]
pub fn parse(args: &[String]) -> Option<Command> {
    let first = args.first()?;
    Some(match first.as_str() {
        "--help" | "-h" => Command::Help,
        "--version" | "-V" => Command::Version,
        other => Command::Unknown(other.to_owned()),
    })
}

/// Answers the arguments in the system's language.
///
/// `args` are the arguments after the program name. `None` means the desktop should open;
/// otherwise the answer has been written to `out` or `err` and the value is the exit code, one
/// of the `EXIT_` constants.
pub fn run(args: &[String], out: &mut impl Write, err: &mut impl Write) -> Option<u8> {
    execute(args, None, out, err)
}

/// Answers the arguments in the language `code`, or the system's language when `None`; English
/// when neither is known. The return value is that of [`run`].
pub fn execute(args: &[String], code: Option<&str>, out: &mut impl Write, err: &mut impl Write) -> Option<u8> {
    let command = parse(args)?;
    let answered = scope(translator(code), || -> io::Result<u8> {
        match command {
            Command::Help => writeln!(out, "{}", t!("cli.usage")).map(|()| EXIT_OK),
            Command::Version => writeln!(out, "{}", version_line()).map(|()| EXIT_OK),
            Command::Unknown(argument) => {
                writeln!(err, "{}", t!("cli.unknown-argument", argument = argument.as_str()))?;
                Ok(EXIT_USAGE)
            }
        }
    });
    Some(answered.unwrap_or(EXIT_FAILED))
}

/// The one line `--version` prints: the command name and the crate version.
///
/// It is not translated: scripts and bug reports read it, and both binaries are the same
/// program, so both name it `qdesk`. The version comes from the build, so it cannot drift from
/// the published package.
#[must_use]
pub fn version_line() -> String {
    format!("qdesk {}", env!("CARGO_PKG_VERSION"))
}

/// The application's text with the language `code` active, or the system's; English when
/// neither is known.
fn translator(code: Option<&str>) -> Arc<I18n> {
    let mut i18n = I18n::builtin();
    for &(file, text) in locales() {
        i18n.add_source(file, text);
    }
    let detected = code.map(str::to_owned).or_else(|| i18n.detect(|name| std::env::var(name).ok()));
    if let Some(code) = detected {
        i18n.set_active(&code);
    }
    Arc::new(i18n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn answer(args: &[&str], code: &str) -> (Option<u8>, String, String) {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let exit = execute(&args, Some(code), &mut out, &mut err);
        (exit, String::from_utf8(out).expect("text"), String::from_utf8(err).expect("text"))
    }

    #[test]
    fn no_argument_opens_the_desktop_and_prints_nothing() {
        assert_eq!(answer(&[], "en"), (None, String::new(), String::new()));
    }

    #[test]
    fn help_prints_the_usage_in_the_language_and_succeeds() {
        for flag in ["--help", "-h"] {
            let (exit, out, err) = answer(&[flag], "en");
            assert_eq!(exit, Some(EXIT_OK));
            assert!(out.contains("--version") && out.contains("ctrl+q"), "{out}");
            assert!(err.is_empty());
        }
        let (_, turkish, _) = answer(&["--help"], "tr");
        let (_, english, _) = answer(&["--help"], "en");
        assert_ne!(turkish, english, "the help is translated");
        assert!(turkish.contains("--help"));
    }

    #[test]
    fn version_prints_the_name_and_the_crate_version_in_every_language() {
        for flag in ["--version", "-V"] {
            for code in ["en", "tr"] {
                let (exit, out, err) = answer(&[flag], code);
                assert_eq!(exit, Some(EXIT_OK));
                assert_eq!(out, format!("qdesk {}\n", env!("CARGO_PKG_VERSION")));
                assert!(err.is_empty());
            }
        }
    }

    #[test]
    fn an_unknown_argument_is_refused_on_the_error_stream_with_status_2() {
        for args in [&["--frobnicate"][..], &["files"], &["-x", "--help"]] {
            let (exit, out, err) = answer(args, "en");
            assert_eq!(exit, Some(EXIT_USAGE), "{args:?}");
            assert!(out.is_empty());
            assert!(err.contains(args[0]), "the error names the argument: {err}");
            assert!(err.contains("--help"), "the error says where to look: {err}");
            assert_eq!(err.lines().count(), 1, "the error is short: {err}");
        }
        let (_, _, turkish) = answer(&["--frobnicate"], "tr");
        assert!(turkish.contains("--frobnicate") && !turkish.contains("Unknown"), "{turkish}");
    }

    #[test]
    fn a_closed_output_is_a_failure_not_a_panic() {
        struct Closed;
        impl Write for Closed {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let args = vec!["--help".to_owned()];
        assert_eq!(execute(&args, Some("en"), &mut Closed, &mut Closed), Some(EXIT_FAILED));
    }
}
