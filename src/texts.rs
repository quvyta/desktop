//! The text viewer: a text, code or Markdown file in a window of
//! the desktop, with the person's editor a key away.
//!
//! What lives here is the window's state and the reading of the file, nothing drawn: which file it
//! is, the text read from it, whether that is all of it and what stopped it being read. The
//! desktop reads off the drawing thread and draws the text with the framework's code and Markdown
//! views.

use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use qframe::icons::{KindFamily, file_kind};

/// The most of a file the viewer reads: enough for any note, log or source file a person reads
/// in a window, and little enough that a log of gigabytes never fills the desktop's memory.
pub const LIMIT: u64 = 4 * 1024 * 1024;

/// How much of a file without an ending is looked at to tell text from a program or data.
pub const SNIFF: usize = 8 * 1024;

/// Kinds the framework counts among text, code or data whose files are not text all the same.
const NOT_TEXT: &[&str] = &["file-database", "file-torrent", "file-wasm"];

/// Whether `file` opens in the text viewer: a file whose name the framework calls text, code or
/// data, and a file without an ending whose first [`SNIFF`] bytes are text without a NUL byte.
///
/// Only a file without an ending is looked into, and only that far; every other file is known by
/// its name alone, as the Files window draws it.
#[must_use]
pub fn is_text(file: &Path) -> bool {
    let Some(name) = file.file_name().map(|name| name.to_string_lossy()) else { return false };
    let kind = file_kind(&name, false, false);
    match kind.family() {
        KindFamily::Text | KindFamily::Code | KindFamily::Data => !NOT_TEXT.contains(&kind.icon()),
        KindFamily::File if file.extension().is_none() => sniff(file),
        _ => false,
    }
}

/// Whether the start of `file` reads as text.
fn sniff(file: &Path) -> bool {
    let Ok(opened) = File::open(file) else { return false };
    let mut start = Vec::with_capacity(SNIFF);
    if opened.take(SNIFF as u64).read_to_end(&mut start).is_err() {
        return false;
    }
    looks_like_text(&start)
}

/// Whether `bytes`, the start of a file, are text: no NUL byte, and UTF-8 but for a character the
/// cut at the end may have split.
#[must_use]
pub fn looks_like_text(bytes: &[u8]) -> bool {
    if bytes.contains(&0) {
        return false;
    }
    match std::str::from_utf8(bytes) {
        Ok(_) => true,
        Err(error) => error.error_len().is_none(),
    }
}

/// Whether `file` is Markdown, drawn as a document rather than as code.
#[must_use]
pub fn is_markdown(file: &Path) -> bool {
    file.extension()
        .and_then(|ending| ending.to_str())
        .is_some_and(|ending| ending.eq_ignore_ascii_case("md") || ending.eq_ignore_ascii_case("markdown"))
}

/// The text read from a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadText {
    /// The text, up to [`LIMIT`].
    pub text: String,
    /// The whole file's size in bytes, when only its first [`LIMIT`] bytes are shown.
    pub cut: Option<u64>,
    /// Whether some of it was not UTF-8 and is shown with the replacement character.
    pub lossy: bool,
    /// How many lines it has.
    pub lines: usize,
}

impl ReadText {
    /// What the first `limit` bytes of `bytes` show, of a file of `size` bytes.
    #[must_use]
    pub fn from_bytes(mut bytes: Vec<u8>, size: u64, limit: u64) -> Self {
        let cut = (size > limit).then_some(size);
        if cut.is_some() {
            // A character the cut split in two is left out, not shown as a broken one.
            if let Err(error) = std::str::from_utf8(&bytes)
                && error.error_len().is_none()
            {
                bytes.truncate(error.valid_up_to());
            }
        }
        let (text, lossy) = match String::from_utf8(bytes) {
            Ok(text) => (text, false),
            Err(error) => (String::from_utf8_lossy(error.as_bytes()).into_owned(), true),
        };
        let lines = text.lines().count();
        Self { text, cut, lossy, lines }
    }
}

/// Why a file could not be read, in words the window can say in the person's language.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unread {
    /// It is no longer there.
    Missing,
    /// The person may not read it.
    Denied,
    /// Anything else, in the system's own words.
    Other(String),
}

impl From<io::Error> for Unread {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::NotFound => Self::Missing,
            io::ErrorKind::PermissionDenied => Self::Denied,
            _ => Self::Other(error.to_string()),
        }
    }
}

/// Reads the first `limit` bytes of `file`.
///
/// # Errors
///
/// Why the file could not be opened or read.
pub fn read(file: &Path, limit: u64) -> Result<ReadText, Unread> {
    let opened = File::open(file)?;
    let size = opened.metadata()?.len();
    let mut bytes = Vec::with_capacity(usize::try_from(size.min(limit)).unwrap_or(0));
    opened.take(limit).read_to_end(&mut bytes)?;
    Ok(ReadText::from_bytes(bytes, size, limit))
}

/// What became of the reading of the file.
#[derive(Debug, Clone)]
pub enum Reading {
    /// It is being read.
    Reading,
    /// It was read.
    Ready(ReadText),
    /// It could not be read, for this reason.
    Failed(Unread),
}

/// One text viewer window.
#[derive(Debug, Clone)]
pub struct TextWindow {
    file: PathBuf,
    reading: Reading,
}

impl TextWindow {
    /// A viewer on `file`, being read.
    #[must_use]
    pub fn new(file: &Path) -> Self {
        Self { file: file.to_path_buf(), reading: Reading::Reading }
    }

    /// The file shown.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.file
    }

    /// Its file name, as the window and its strip call it.
    #[must_use]
    pub fn name(&self) -> String {
        self.file
            .file_name()
            .map_or_else(|| self.file.display().to_string(), |name| name.to_string_lossy().into_owned())
    }

    /// What became of its reading.
    #[must_use]
    pub fn reading(&self) -> &Reading {
        &self.reading
    }

    /// The reading ended with `result`.
    pub fn read_as(&mut self, result: Result<ReadText, Unread>) {
        self.reading = match result {
            Ok(shown) => Reading::Ready(shown),
            Err(why) => Reading::Failed(why),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_code_and_data_open_in_the_viewer_and_pictures_archives_and_databases_do_not() {
        for name in ["notlar.txt", "main.rs", "README.md", "Cargo.toml", "ayar.yaml", "veri.json", "sunucu.log"] {
            assert!(is_text(Path::new("/yok").join(name).as_path()), "{name}");
        }
        for name in ["deniz.png", "arsiv.zip", "kitap.pdf", "veri.sqlite", "modul.wasm", "sarki.ogg"] {
            assert!(!is_text(Path::new("/yok").join(name).as_path()), "{name}");
        }
    }

    #[test]
    fn a_file_without_an_ending_is_text_when_its_start_is() {
        let folder = std::env::temp_dir().join(format!("qdesk-texts-{}", std::process::id()));
        std::fs::create_dir_all(&folder).expect("a folder");
        let note = folder.join("NOTLAR");
        std::fs::write(&note, "bir iki üç\n").expect("written");
        let program = folder.join("program");
        std::fs::write(&program, b"\x7fELF\x02\x01\x01\x00\x00").expect("written");
        let missing = folder.join("yok");
        let (text, binary, gone) = (is_text(&note), is_text(&program), is_text(&missing));
        let _ = std::fs::remove_dir_all(&folder);
        assert!(text, "plain UTF-8 without an ending");
        assert!(!binary, "a NUL byte says it is not text");
        assert!(!gone, "a file that cannot be read is left to the editor");
    }

    #[test]
    fn a_character_split_by_the_end_of_what_was_looked_at_is_still_text() {
        let bytes = "ağaç".as_bytes();
        assert!(looks_like_text(&bytes[..bytes.len() - 1]));
        assert!(!looks_like_text(b"a\xffb"), "a byte that is never UTF-8");
    }

    #[test]
    fn a_long_file_is_cut_at_the_limit_without_breaking_a_character() {
        let shown = ReadText::from_bytes("ağaç\nçiçek".as_bytes()[..2].to_vec(), 14, 2);
        assert_eq!(shown.text, "a", "the half of ğ is left out");
        assert_eq!(shown.cut, Some(14));
        assert!(!shown.lossy);
        let whole = ReadText::from_bytes(b"bir\niki\n".to_vec(), 8, 100);
        assert_eq!((whole.cut, whole.lines), (None, 2));
    }

    #[test]
    fn bytes_that_are_not_utf8_are_shown_with_the_replacement_character() {
        let shown = ReadText::from_bytes(b"caf\xe9 au lait".to_vec(), 12, 100);
        assert!(shown.lossy);
        assert_eq!(shown.text, "caf\u{fffd} au lait");
    }

    #[test]
    fn markdown_is_known_by_its_ending() {
        assert!(is_markdown(Path::new("README.md")) && is_markdown(Path::new("x.MARKDOWN")));
        assert!(!is_markdown(Path::new("main.rs")) && !is_markdown(Path::new("md")));
    }
}
