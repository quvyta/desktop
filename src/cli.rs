//! The command line: `--help` and `--version` answer and exit; `wallpaper` shows or sets the
//! floor, its colour, its pattern and the picture over it; no argument opens the desktop.
//!
//! [`run`] is what the binaries call, in the system's language. [`execute`] underneath takes the
//! language as a value, so a test can check the text in each one, and [`execute_in`] takes the
//! ecosystem's configuration folder as well, so nothing a test does reaches the person's own.
//!
//! `qdesk wallpaper` is how another program changes the floor, the ecosystem's file explorer
//! first: it runs the command, qdesk writes its own settings file through the same code the
//! Settings screen uses, and a running desktop sees the file change and draws the new floor. The
//! other program never reads or writes qdesk's files, so the file stays qdesk's to change.

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use qframe::i18n::{I18n, scope};
use qframe::storage::Ecosystem;
use qframe::t;
use qframe::widgets::ImageData;

use crate::locales;
use crate::settings::{self, FloorColor, FloorStyle};
use crate::wallpapers;

/// The command did what was asked.
pub const EXIT_OK: u8 = 0;
/// The output could not be written, or the setting could not be kept.
pub const EXIT_FAILED: u8 = 1;
/// The arguments made no sense.
pub const EXIT_USAGE: u8 = 2;

/// The option of `qdesk wallpaper` that names the floor's colour.
pub const COLOR: &str = "--color";
/// The option of `qdesk wallpaper` that names the floor's pattern.
pub const PATTERN: &str = "--pattern";
/// The option of `qdesk wallpaper` that takes the picture off the floor.
pub const NO_PICTURE: &str = "--no-picture";

/// What the arguments ask for, besides opening the desktop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    /// Print the usage text.
    Help,
    /// Print the program name and version.
    Version,
    /// Set what the floor is given, or print what is set when it is given nothing.
    Wallpaper(Floor),
    /// `qdesk wallpaper` with something it cannot take.
    Refused(Refusal),
    /// An argument that is not known.
    Unknown(String),
}

/// The floor `qdesk wallpaper` is asked for: a colour, a pattern, a picture, any of them or none.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Floor {
    /// The colour, when [`COLOR`] was given.
    pub color: Option<FloorColor>,
    /// The pattern, when [`PATTERN`] was given.
    pub pattern: Option<FloorStyle>,
    /// The picture, when a file or [`NO_PICTURE`] was given.
    pub picture: Option<Picture>,
}

/// What `qdesk wallpaper` is asked to do with the picture over the floor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picture {
    /// Lay the picture in this file over the floor, as it was typed; it is checked and made
    /// absolute before it is written. `builtin:` and the name of one of qdesk's own pictures
    /// (`builtin:tide`) is that picture, as the settings file and the printed line name it.
    Set(PathBuf),
    /// Take the picture away.
    Clear,
}

/// Why `qdesk wallpaper` took nothing and wrote nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// An option was given a value it does not have.
    Value {
        /// [`COLOR`] or [`PATTERN`].
        option: &'static str,
        /// What was given.
        value: String,
    },
    /// An option was given no value at all.
    Missing(&'static str),
    /// A file that cannot be the picture: nothing is there, it cannot be read, it is not a
    /// picture, or it is a damaged one.
    Picture {
        /// The file as it was typed.
        file: String,
        /// Why, in the active language.
        reason: String,
    },
    /// An option `qdesk wallpaper` does not have.
    Option(String),
}

/// The names `--pattern` takes, in the order the help lists them. They are the file's own names
/// but for the gradient with the dots, which is `both` on the command line: a person types the
/// word the Settings screen shows, not the key the file keeps.
const PATTERNS: [(&str, FloorStyle); 4] = [
    ("plain", FloorStyle::Plain),
    ("gradient", FloorStyle::Gradient),
    ("dots", FloorStyle::Dots),
    ("both", FloorStyle::GradientDots),
];

/// The name `--pattern` gives `style`.
#[must_use]
pub fn pattern_name(style: FloorStyle) -> &'static str {
    PATTERNS.iter().find(|(_, known)| *known == style).map_or("plain", |(name, _)| name)
}

/// Reads the arguments after the program name; `None` means there are none and the desktop
/// opens. The first argument decides: qdesk takes nothing besides `--help`, `--version` and
/// `wallpaper`, so anything else is refused rather than ignored.
#[must_use]
pub fn parse(args: &[String]) -> Option<Command> {
    let first = args.first()?;
    Some(match first.as_str() {
        "--help" | "-h" => Command::Help,
        "--version" | "-V" => Command::Version,
        "wallpaper" => match wallpaper(&args[1..]) {
            Ok(floor) => Command::Wallpaper(floor),
            Err(refusal) => Command::Refused(refusal),
        },
        other => Command::Unknown(other.to_owned()),
    })
}

/// Reads the arguments of `qdesk wallpaper`. Every value is checked here, before anything is
/// written, so a refused command leaves the file as it was; a picture's file is read when the
/// command runs. An option given twice takes its last value, as most commands do, and so does a
/// picture given twice or given with [`NO_PICTURE`].
fn wallpaper(args: &[String]) -> Result<Floor, Refusal> {
    let mut floor = Floor::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        if arg == NO_PICTURE {
            floor.picture = Some(Picture::Clear);
            continue;
        }
        if !arg.starts_with('-') {
            floor.picture = Some(Picture::Set(PathBuf::from(arg)));
            continue;
        }
        let (option, inline) = match arg.split_once('=') {
            Some((option, value)) => (option, Some(value.to_owned())),
            None => (arg.as_str(), None),
        };
        let option = match option {
            COLOR => COLOR,
            PATTERN => PATTERN,
            other => return Err(Refusal::Option(other.to_owned())),
        };
        let Some(value) = inline.or_else(|| rest.next().cloned()) else { return Err(Refusal::Missing(option)) };
        if option == COLOR {
            let color = FloorColor::from_name(&value).ok_or(Refusal::Value { option, value })?;
            floor.color = Some(color);
        } else {
            let pattern = PATTERNS.iter().find(|(name, _)| *name == value).map(|(_, style)| *style);
            floor.pattern = Some(pattern.ok_or(Refusal::Value { option, value })?);
        }
    }
    Ok(floor)
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
    // Asked only when the arguments need it: `--help` reads no folder.
    let needs_folder = matches!(parse(args), Some(Command::Wallpaper(_)));
    let config = if needs_folder { Ecosystem::QUVYTA.config_dir() } else { None };
    execute_in(args, code, config.as_deref(), out, err)
}

/// [`execute`] with `config` as the ecosystem's configuration folder, where `desktop.conf` is;
/// `None` is a machine without a home folder, where nothing can be kept.
pub fn execute_in(
    args: &[String],
    code: Option<&str>,
    config: Option<&Path>,
    out: &mut impl Write,
    err: &mut impl Write,
) -> Option<u8> {
    let command = parse(args)?;
    let answered = scope(translator(code), || -> io::Result<u8> {
        match command {
            Command::Help => writeln!(out, "{}", t!("cli.usage")).map(|()| EXIT_OK),
            Command::Version => writeln!(out, "{}", version_line()).map(|()| EXIT_OK),
            Command::Wallpaper(floor) => set_wallpaper(&floor, config, out, err),
            Command::Refused(refusal) => {
                writeln!(err, "{}", refused(&refusal))?;
                Ok(EXIT_USAGE)
            }
            Command::Unknown(argument) => {
                writeln!(err, "{}", t!("cli.unknown-argument", argument = argument.as_str()))?;
                Ok(EXIT_USAGE)
            }
        }
    });
    Some(answered.unwrap_or(EXIT_FAILED))
}

/// The one line that says why `qdesk wallpaper` was refused, in the active language.
fn refused(refusal: &Refusal) -> String {
    match refusal {
        Refusal::Value { option, value } if *option == COLOR => t!("cli.wallpaper-color", value = value.as_str()),
        Refusal::Value { value, .. } => t!("cli.wallpaper-pattern", value = value.as_str()),
        Refusal::Missing(option) => t!("cli.wallpaper-missing", option = *option),
        Refusal::Picture { file, reason } => {
            t!("cli.wallpaper-picture", file = file.as_str(), reason = reason.as_str())
        }
        Refusal::Option(option) => t!("cli.unknown-argument", argument = option.as_str()),
    }
}

/// Prints the floor that is set, or writes what `floor` gives into the settings file in `config`.
///
/// The line printed is the command that would set the floor as it is, so a script can keep it
/// and run it again; a picture's path comes last, quoted as a shell reads it when it needs to be.
/// Nothing is written when nothing would change, so a running desktop is not woken for nothing.
///
/// A picture is decoded before anything is written, with the same decoder the desktop draws it
/// with, so a file the desktop could not show is refused here, in one line, rather than said in
/// the corner of a desktop the person may not be looking at.
fn set_wallpaper(floor: &Floor, config: Option<&Path>, out: &mut impl Write, err: &mut impl Write) -> io::Result<u8> {
    let Some(config) = config else {
        writeln!(err, "{}", t!("cli.wallpaper-nowhere"))?;
        return Ok(EXIT_FAILED);
    };
    let picture = match &floor.picture {
        Some(Picture::Set(file)) => match checked(file) {
            Ok(picture) => Some(Some(picture)),
            Err(refusal) => {
                writeln!(err, "{}", refused(&refusal))?;
                return Ok(EXIT_USAGE);
            }
        },
        Some(Picture::Clear) => Some(None),
        None => None,
    };
    let mut loaded = settings::load_in(config);
    if *floor == Floor::default() {
        let (color, pattern) = (loaded.prefs.floor.name(), pattern_name(loaded.prefs.floor_style));
        match &loaded.wallpaper {
            Some(picture) => writeln!(out, "{COLOR} {color} {PATTERN} {pattern} {}", quoted(&picture.to_string()))?,
            None => writeln!(out, "{COLOR} {color} {PATTERN} {pattern}")?,
        }
        return Ok(EXIT_OK);
    }
    let before = loaded.settings.clone();
    settings::set_floor(&mut loaded.settings, floor.color, floor.pattern);
    if let Some(picture) = picture
        && !settings::set_wallpaper(&mut loaded.settings, picture.as_ref())
    {
        let file = picture.map(|picture| picture.to_string()).unwrap_or_default();
        let reason = t!("cli.wallpaper-not-text");
        writeln!(err, "{}", refused(&Refusal::Picture { file, reason }))?;
        return Ok(EXIT_USAGE);
    }
    if loaded.settings == before {
        return Ok(EXIT_OK);
    }
    match loaded.settings.save() {
        Ok(()) => Ok(EXIT_OK),
        Err(error) => {
            let file = loaded.settings.path().map_or_else(|| PathBuf::from(config), Path::to_path_buf);
            let (file, error) = (file.display().to_string(), error.to_string());
            writeln!(err, "{}", t!("cli.wallpaper-unsaved", file = file.as_str(), error = error.as_str()))?;
            Ok(EXIT_FAILED)
        }
    }
}

/// The picture `file` names, made absolute, when it can be the floor's; otherwise why not.
/// `builtin:` and the name of one of qdesk's own pictures is that picture, built into the program,
/// so nothing is read.
fn checked(file: &Path) -> Result<wallpapers::Picture, Refusal> {
    if let Some(ours) =
        file.to_str().filter(|text| text.starts_with(wallpapers::BUILTIN)).and_then(wallpapers::Picture::parse)
    {
        return Ok(ours);
    }
    let refusal = |reason: String| Refusal::Picture { file: file.display().to_string(), reason };
    let path = std::path::absolute(file).map_err(|error| refusal(error.to_string()))?;
    // Decoded small: only whether it decodes matters here, and the desktop decodes it again at
    // the size it draws it.
    ImageData::decode_file(&path, (1, 1)).map_err(|problem| refusal(problem.to_string()))?;
    Ok(wallpapers::Picture::File(path))
}

/// `text` as a shell reads it back as one word: as it is when it holds nothing a shell would
/// split or change, else in single quotes, with a single quote in it written as `'\''`.
fn quoted(text: &str) -> String {
    let plain = !text.is_empty()
        && text
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '/' | '.' | '-' | '_' | '+' | ',' | ':' | '@' | '%'));
    if plain { text.to_owned() } else { format!("'{}'", text.replace('\'', "'\\''")) }
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
            assert!(out.contains("qdesk wallpaper") && out.contains("--color") && out.contains("--pattern"), "{out}");
            assert!(out.contains("qdesk wallpaper FILE") && out.contains("--no-picture"), "{out}");
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

    /// A folder of one test in the temporary folder, standing in for the ecosystem's
    /// configuration folder; never the person's own.
    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("qdesk-cli-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("folder");
        dir
    }

    fn answer_in(args: &[&str], config: Option<&Path>) -> (Option<u8>, String, String) {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let exit = execute_in(&args, Some("en"), config, &mut out, &mut err);
        (exit, String::from_utf8(out).expect("text"), String::from_utf8(err).expect("text"))
    }

    fn wallpaper_of(args: &[&str]) -> Option<Command> {
        let args: Vec<String> = ["wallpaper"].iter().chain(args).map(|arg| (*arg).to_owned()).collect();
        parse(&args)
    }

    #[test]
    fn wallpaper_reads_a_colour_and_a_pattern_in_either_order_and_either_spelling() {
        let both = Floor { color: Some(FloorColor::Deep), pattern: Some(FloorStyle::GradientDots), picture: None };
        assert_eq!(wallpaper_of(&["--color", "deep", "--pattern", "both"]), Some(Command::Wallpaper(both.clone())));
        assert_eq!(wallpaper_of(&["--pattern=both", "--color=deep"]), Some(Command::Wallpaper(both)));
        assert_eq!(
            wallpaper_of(&["--pattern", "dots"]),
            Some(Command::Wallpaper(Floor { pattern: Some(FloorStyle::Dots), ..Floor::default() }))
        );
        assert_eq!(wallpaper_of(&[]), Some(Command::Wallpaper(Floor::default())), "nothing asks what is set");
        let picture = |file: &str| Some(Picture::Set(PathBuf::from(file)));
        assert_eq!(
            wallpaper_of(&["deniz.png", "--color", "deep"]),
            Some(Command::Wallpaper(Floor {
                color: Some(FloorColor::Deep),
                picture: picture("deniz.png"),
                ..Floor::default()
            }))
        );
        assert_eq!(
            wallpaper_of(&["--no-picture"]),
            Some(Command::Wallpaper(Floor { picture: Some(Picture::Clear), ..Floor::default() }))
        );
        assert_eq!(
            wallpaper_of(&["a.png", "--no-picture", "b.png"]),
            Some(Command::Wallpaper(Floor { picture: picture("b.png"), ..Floor::default() })),
            "the last word about the picture holds"
        );
        for (name, style) in PATTERNS {
            assert_eq!(pattern_name(style), name, "every pattern prints the name it is typed as");
        }
    }

    #[test]
    fn wallpaper_refuses_what_it_cannot_take_in_one_line_with_status_2_and_writes_nothing() {
        let config = scratch("refused");
        let cases: [(&[&str], &str); 7] = [
            (&["--color", "red"], "red"),
            (&["--pattern", "tartan"], "tartan"),
            (&["--pattern", "gradient-dots"], "gradient-dots"),
            (&["--color"], "--color"),
            (&["/nowhere/kisi/deniz.png"], "deniz.png"),
            (&["--no-picture=yes"], "--no-picture"),
            (&["--colour", "deep"], "--colour"),
        ];
        for (args, named) in cases {
            let (exit, out, err) = answer_in(&[&["wallpaper"][..], args].concat(), Some(&config));
            assert_eq!(exit, Some(EXIT_USAGE), "{args:?}: {err}");
            assert!(out.is_empty(), "{args:?}: {out}");
            assert!(err.contains(named), "the reason names what was wrong: {err}");
            assert_eq!(err.lines().count(), 1, "one line: {err}");
        }
        // A good value before a bad one is not written either.
        let (exit, _, _) = answer_in(&["wallpaper", "--color", "deep", "--pattern", "tartan"], Some(&config));
        assert_eq!(exit, Some(EXIT_USAGE));
        assert!(!config.join("desktop.conf").exists(), "a refused command writes nothing");
        let _ = std::fs::remove_dir_all(&config);
    }

    #[test]
    fn wallpaper_writes_only_what_it_was_given_and_prints_what_is_set() {
        let config = scratch("writes");
        let file = config.join("desktop.conf");
        std::fs::write(&file, "dock-position = \"top\"\nfloor-style = \"dots\"\n").expect("file");
        let (exit, out, err) = answer_in(&["wallpaper", "--color", "mist"], Some(&config));
        assert_eq!((exit, out.as_str(), err.as_str()), (Some(EXIT_OK), "", ""));
        let written = std::fs::read_to_string(&file).expect("file");
        assert!(written.contains("floor-color = \"mist\""), "{written}");
        assert!(written.contains("floor-style = \"dots\"") && written.contains("dock-position = \"top\""), "{written}");
        let (exit, out, _) = answer_in(&["wallpaper"], Some(&config));
        assert_eq!((exit, out.as_str()), (Some(EXIT_OK), "--color mist --pattern dots\n"));
        // The default is taken out of the file, as the Settings screen does.
        answer_in(&["wallpaper", "--color", "theme", "--pattern", "plain"], Some(&config));
        let written = std::fs::read_to_string(&file).expect("file");
        assert_eq!(written, "dock-position = \"top\"\n");
        let _ = std::fs::remove_dir_all(&config);
    }

    #[test]
    fn one_of_our_pictures_is_set_by_its_builtin_name_and_printed_as_one_that_sets_it_again() {
        let config = scratch("builtin");
        let file = config.join("desktop.conf");
        let (exit, out, err) = answer_in(&["wallpaper", "builtin:dusk"], Some(&config));
        assert_eq!((exit, out.as_str(), err.as_str()), (Some(EXIT_OK), "", ""));
        let written = std::fs::read_to_string(&file).expect("file");
        assert_eq!(written, "wallpaper = \"builtin:dusk\"\n");
        let (exit, out, _) = answer_in(&["wallpaper"], Some(&config));
        assert_eq!((exit, out.as_str()), (Some(EXIT_OK), "--color theme --pattern plain builtin:dusk\n"));
        // A name qdesk does not bring is a file that is not there.
        let (exit, _, err) = answer_in(&["wallpaper", "builtin:coral"], Some(&config));
        assert_eq!(exit, Some(EXIT_USAGE), "{err}");
        assert!(err.contains("builtin:coral"), "{err}");
        assert_eq!(std::fs::read_to_string(&file).expect("file"), written, "nothing is written");
        let _ = std::fs::remove_dir_all(&config);
    }

    #[test]
    fn a_path_a_shell_would_split_is_printed_in_quotes_it_reads_back() {
        assert_eq!(quoted("/home/kisi/deniz.png"), "/home/kisi/deniz.png");
        assert_eq!(quoted("/home/kişi/Resimler/deniz.png"), "/home/kişi/Resimler/deniz.png");
        assert_eq!(quoted("/home/kisi/deniz feneri.png"), "'/home/kisi/deniz feneri.png'");
        assert_eq!(quoted("/home/kisi/it's.png"), "'/home/kisi/it'\\''s.png'");
        assert_eq!(quoted("/tmp/$HOME.png"), "'/tmp/$HOME.png'");
    }

    #[test]
    fn wallpaper_without_a_home_folder_says_so_and_fails() {
        let (exit, out, err) = answer_in(&["wallpaper", "--color", "deep"], None);
        assert_eq!(exit, Some(EXIT_FAILED));
        assert!(out.is_empty() && err.contains("home folder"), "{err}");
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
