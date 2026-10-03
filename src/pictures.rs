//! The picture viewer: one picture of a folder in a window of the desktop,
//! fitted whole or at its own size, and the other pictures of the folder a key away.
//!
//! What lives here is the window's state, nothing drawn and nothing read but a folder's names: the
//! pictures of the folder in the order of their names, which one is shown, how it fills the window
//! and what became of its decoding. The desktop decodes off the drawing thread and draws it with
//! the framework's [`Image`](qframe::widgets::Image).

use std::fs;
use std::path::{Path, PathBuf};

use qframe::widgets::{Fit, ImageData, ImageError};

use crate::wallpapers;

/// A way through the pictures of the folder, or a change of how the one shown fills the window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// The next picture, the first after the last.
    Next,
    /// The one before, the last before the first.
    Previous,
    /// The first of the folder.
    First,
    /// The last of the folder.
    Last,
    /// Fitted whole, or at its own size.
    ToggleFit,
}

/// What became of the picture shown.
#[derive(Debug, Clone)]
pub enum Decoded {
    /// It is being decoded.
    Decoding,
    /// It is decoded.
    Ready(ImageData),
    /// It could not be decoded, for this reason.
    Failed(ImageError),
}

/// One picture viewer window.
#[derive(Debug, Clone)]
pub struct PictureWindow {
    /// The pictures of the folder, by name; the one opened is always among them.
    list: Vec<PathBuf>,
    /// Which of them is shown.
    index: usize,
    /// How it fills the window: [`Fit::Contain`] whole, or [`Fit::Center`] at its own size.
    fit: Fit,
    /// What became of its decoding.
    decoded: Decoded,
    /// Which decoding is the current one; the answer of an older one is dropped.
    run: u64,
}

impl PictureWindow {
    /// A viewer on `file`, walking the pictures of its folder as `names` lists them: the files of
    /// the folder that are pictures ([`wallpapers::is_picture`]), hidden ones left out, sorted by
    /// name. `file` is among them even when its own name would leave it out.
    #[must_use]
    pub fn new(file: &Path, names: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut list: Vec<PathBuf> = names
            .into_iter()
            .filter(|path| {
                let name = file_name(path);
                !name.starts_with('.') && wallpapers::is_picture(&name)
            })
            .collect();
        if !list.iter().any(|path| path == file) {
            list.push(file.to_path_buf());
        }
        list.sort_by(|a, b| by_name(a, b));
        list.dedup();
        let index = list.iter().position(|path| *path == file).unwrap_or(0);
        Self { list, index, fit: Fit::Contain, decoded: Decoded::Decoding, run: 0 }
    }

    /// A viewer on `file` and the pictures beside it in its folder, read from the disk. A folder
    /// that cannot be read leaves the picture alone in its list.
    #[must_use]
    pub fn open(file: &Path) -> Self {
        let names = file.parent().map(read_folder).unwrap_or_default();
        Self::new(file, names)
    }

    /// The picture shown.
    #[must_use]
    pub fn file(&self) -> &Path {
        &self.list[self.index]
    }

    /// Its file name, as the window and its strip call it.
    #[must_use]
    pub fn name(&self) -> String {
        file_name(self.file())
    }

    /// Where it is in its folder, counted from 1, and how many pictures the folder has.
    #[must_use]
    pub fn position(&self) -> (usize, usize) {
        (self.index + 1, self.list.len())
    }

    /// How it fills the window.
    #[must_use]
    pub fn fit(&self) -> Fit {
        self.fit
    }

    /// What became of its decoding.
    #[must_use]
    pub fn decoded(&self) -> &Decoded {
        &self.decoded
    }

    /// The current decoding's number.
    #[must_use]
    pub fn run(&self) -> u64 {
        self.run
    }

    /// Takes `step`; `true` when another picture is now shown, which has to be decoded. The
    /// decoding of the one before is forgotten.
    pub fn step(&mut self, step: Step) -> bool {
        let count = self.list.len();
        let index = match step {
            Step::Next => (self.index + 1) % count,
            Step::Previous => (self.index + count - 1) % count,
            Step::First => 0,
            Step::Last => count - 1,
            Step::ToggleFit => {
                self.fit = if self.fit == Fit::Contain { Fit::Center } else { Fit::Contain };
                return false;
            }
        };
        if index == self.index {
            return false;
        }
        self.index = index;
        self.start();
        true
    }

    /// Starts a decoding of the picture shown, forgetting any before it, and gives its number.
    pub fn start(&mut self) -> u64 {
        self.run += 1;
        self.decoded = Decoded::Decoding;
        self.run
    }

    /// The decoding `run` ended with `result`; an older one's answer is dropped.
    pub fn decoded_as(&mut self, run: u64, result: Result<ImageData, ImageError>) -> bool {
        if run != self.run {
            return false;
        }
        self.decoded = match result {
            Ok(data) => Decoded::Ready(data),
            Err(problem) => Decoded::Failed(problem),
        };
        true
    }

    /// Whether a decoding is under way.
    #[must_use]
    pub fn decoding(&self) -> bool {
        matches!(self.decoded, Decoded::Decoding)
    }

    /// The picture's own size in pixels, once it is decoded.
    #[must_use]
    pub fn pixels(&self) -> Option<(u32, u32)> {
        match &self.decoded {
            Decoded::Ready(data) => Some(data.original_size()),
            Decoded::Decoding | Decoded::Failed(_) => None,
        }
    }
}

/// The name of the file at `path`, or the whole path when it has none.
fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
}

/// Two files in the order a folder lists them: by name whatever the case, so `b.png` comes before
/// `C.png`; two names that differ only in case keep the order of their bytes.
fn by_name(a: &Path, b: &Path) -> std::cmp::Ordering {
    let (a, b) = (file_name(a), file_name(b));
    a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(&b))
}

/// The files of `folder`; nothing when it cannot be read.
fn read_folder(folder: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(folder) else { return Vec::new() };
    entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| !kind.is_dir()))
        .map(|entry| entry.path())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viewer(open: &str, names: &[&str]) -> PictureWindow {
        let folder = Path::new("/resimler");
        PictureWindow::new(&folder.join(open), names.iter().map(|name| folder.join(name)))
    }

    #[test]
    fn the_folder_s_pictures_are_walked_by_name_and_wrap_around_both_ends() {
        let mut window = viewer("b.png", &["c.JPG", "notlar.txt", "a.png", "B.png", ".gizli.png", "b.png"]);
        let order: Vec<String> = window.list.iter().map(|path| file_name(path)).collect();
        assert_eq!(order, ["a.png", "B.png", "b.png", "c.JPG"]);
        assert_eq!(window.position(), (3, 4));
        assert!(window.step(Step::Previous));
        assert_eq!(window.name(), "B.png");
        assert!(window.step(Step::Previous));
        assert_eq!(window.name(), "a.png");
        assert!(window.step(Step::Previous), "before the first is the last");
        assert_eq!(window.name(), "c.JPG");
        assert!(window.step(Step::Next), "after the last is the first");
        assert_eq!(window.name(), "a.png");
        assert!(window.step(Step::Last));
        assert_eq!(window.position(), (4, 4));
        assert!(!window.step(Step::Last), "standing on the last, End shows nothing new");
        assert!(window.step(Step::First));
        assert_eq!(window.position(), (1, 4));
    }

    #[test]
    fn the_picture_opened_is_walked_from_even_when_its_name_hides_it() {
        let window = viewer(".gizli.png", &["a.png", ".gizli.png"]);
        assert_eq!(window.name(), ".gizli.png");
        assert_eq!(window.position(), (1, 2));
    }

    #[test]
    fn a_decoding_answered_after_the_picture_changed_is_dropped() {
        let mut window = viewer("a.png", &["a.png", "b.png"]);
        let first = window.start();
        assert!(window.step(Step::Next));
        assert!(!window.decoded_as(first, Err(ImageError::Broken)), "the answer for a.png is late");
        assert!(window.decoding());
        assert!(window.decoded_as(window.run(), Err(ImageError::Missing)));
        assert!(matches!(window.decoded(), Decoded::Failed(ImageError::Missing)));
    }

    #[test]
    fn the_fit_toggles_between_whole_and_its_own_size_without_decoding_again() {
        let mut window = viewer("a.png", &["a.png"]);
        assert_eq!(window.fit(), Fit::Contain);
        assert!(!window.step(Step::ToggleFit));
        assert_eq!(window.fit(), Fit::Center);
        assert!(!window.step(Step::ToggleFit));
        assert_eq!(window.fit(), Fit::Contain);
        assert!(!window.step(Step::Next), "one picture alone: nothing new to show");
    }
}
