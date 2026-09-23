//! The Files window: the family's shared file manager, in a window of the desktop.
//!
//! The manager itself is the framework's [`FileManager`](qframe::widgets::FileManager): it reads
//! the folders, draws them and does every file operation. qdesk writes none of that and copies
//! none of it. What lives here is what only the desktop knows: which folder a window starts in,
//! the shape each window draws its folder in, where a deleted entry goes, what a file is opened
//! with, and which folders the other windows' programs stand in.

use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

use qframe::widgets::{FileManagerState, FileView, RowMark};

/// What a file is opened with when the person has named no editor: it only reads, so opening a
/// file by mistake changes nothing, and it is on every machine a server is.
pub const READER: &str = "less";

/// The theme's colour a row's mark is drawn in: the desktop's own accent, the colour of the window
/// that has the focus, so the mark reads as "a window of yours is here".
pub const MARK_TONE: &str = "accent";

/// What one Files window holds: its manager and the shape it draws the folder in.
///
/// Each window has its own, so two windows can show two folders, or the same folder two ways, and
/// neither moves when the other does. None of it is written anywhere: the shape is the window's
/// for as long as it is open, which is a decision that can be taken back, not a setting yet.
#[derive(Debug)]
pub struct FilesWindow {
    /// The framework's manager of the window's folder.
    pub manager: FileManagerState,
    /// The shape the folder is drawn in.
    pub view: FileView,
}

impl FilesWindow {
    /// A window onto the folder `root`, nothing read yet.
    ///
    /// Nothing is confined: on a server the folders worth a look are as often `/etc` or
    /// `/var/log` as anything under the home folder, and the desktop is the person's own. Deleting
    /// puts entries in the trash under `data_home` — the desktop's own data folder, the one the
    /// freedesktop trash lives in — and in the person's trash as the framework finds it when the
    /// desktop was given none. `following` keeps the folders on screen in step with what other
    /// programs do to them.
    ///
    /// The list is the first shape: on a server what is wanted of a folder is sizes, dates and
    /// permissions, which only the list shows.
    #[must_use]
    pub fn new(root: PathBuf, data_home: Option<&Path>, following: bool) -> Self {
        let manager = FileManagerState::new(root).following(following);
        let manager = match data_home {
            Some(data) => manager.trashing_in(data.join("Trash")),
            None => manager.trashing(),
        };
        Self { manager, view: FileView::List }
    }
}

/// The command that opens `file` in a terminal window: the person's editor (`editor`, from
/// `VISUAL` or `EDITOR`) with its own arguments, else [`READER`], and the file last.
///
/// `None` when the file's path is not text: a command is a list of words, and a name turned into
/// text by guessing would open a different file, or a new empty one, rather than this one.
#[must_use]
pub fn opener(editor: Option<&str>, file: &Path) -> Option<Vec<String>> {
    let mut words: Vec<String> =
        editor.map(|editor| editor.split_whitespace().map(str::to_owned).collect()).unwrap_or_default();
    if words.is_empty() {
        words.push(READER.to_owned());
    }
    words.push(file.to_str()?.to_owned());
    Some(words)
}

/// The key of the folder `folder` in a file manager rooted at `root`: `""` for the root itself,
/// `"a/b"` for a folder under it; `None` for a folder outside it, or one whose name is not text.
#[must_use]
pub fn key_of(root: &Path, folder: &Path) -> Option<String> {
    let inside = folder.strip_prefix(root).ok()?;
    let mut parts = Vec::new();
    for part in inside.components() {
        match part {
            Component::Normal(name) => parts.push(name.to_str()?),
            // `a/./b` is `a/b`; anything that climbs or starts again is not a plain folder under
            // the root, and is left unmarked rather than guessed at.
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    Some(parts.join("/"))
}

/// The marks of the rows of a file manager rooted at `root`, by key, from the folders the windows'
/// programs stand in: `windows` gives each window's folder and the name of its icon.
///
/// A row whose folder holds a window's program shows that window's icon in the accent, so the
/// sign and the colour say it together and the row is told apart in sixteen colours and in ASCII
/// too. When two windows stand in one folder, the first given keeps the row: windows are given
/// in the order they were opened, so a row does not change sign because a later window came.
#[must_use]
pub fn row_marks<'a>(root: &Path, windows: impl IntoIterator<Item = (&'a Path, &'a str)>) -> BTreeMap<String, RowMark> {
    let mut marks = BTreeMap::new();
    for (folder, icon) in windows {
        if let Some(key) = key_of(root, folder) {
            marks.entry(key).or_insert_with(|| RowMark::new().sign(icon, MARK_TONE));
        }
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_opens_in_the_editor_with_its_arguments_else_in_the_reader() {
        let file = Path::new("/srv/notes.txt");
        assert_eq!(opener(Some("emacs -nw"), file), Some(vec!["emacs".into(), "-nw".into(), "/srv/notes.txt".into()]));
        assert_eq!(opener(None, file), Some(vec![READER.into(), "/srv/notes.txt".into()]));
        assert_eq!(opener(Some("  "), file), Some(vec![READER.into(), "/srv/notes.txt".into()]));
    }

    #[cfg(unix)]
    #[test]
    fn a_file_whose_name_is_not_text_is_not_opened_under_a_guessed_name() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let file = Path::new(OsStr::from_bytes(b"/srv/not\xffes"));
        assert_eq!(opener(None, file), None);
    }

    #[test]
    fn a_folder_s_key_is_its_path_under_the_root() {
        let root = Path::new("/home/ada");
        assert_eq!(key_of(root, Path::new("/home/ada")).as_deref(), Some(""));
        assert_eq!(key_of(root, Path::new("/home/ada/src/app")).as_deref(), Some("src/app"));
        assert_eq!(key_of(root, Path::new("/home/ada/./src")).as_deref(), Some("src"));
        assert_eq!(key_of(root, Path::new("/home/adam")), None, "a longer name is not a folder under it");
        assert_eq!(key_of(root, Path::new("/etc")), None);
        assert_eq!(key_of(root, Path::new("/home/ada/../bob")), None);
    }

    #[test]
    fn a_window_s_folder_carries_its_icon_in_the_accent_and_the_first_window_keeps_the_row() {
        let root = Path::new("/home/ada");
        let marks = row_marks(
            root,
            [
                (Path::new("/home/ada/src"), "prompt"),
                (Path::new("/home/ada/src"), "project"),
                (Path::new("/home/ada"), "project"),
                (Path::new("/etc"), "prompt"),
            ],
        );
        assert_eq!(marks.len(), 2, "a folder outside the root marks nothing: {marks:?}");
        let src = &marks["src"];
        assert_eq!((src.icon(), src.tone()), (Some("prompt"), Some(MARK_TONE)));
        assert!(!src.is_faint(), "a window's folder is louder than the rest, never fainter");
        assert_eq!(marks[""].icon(), Some("project"));
        assert!(row_marks(root, []).is_empty(), "no window, no mark");
    }

    #[test]
    fn a_window_starts_as_a_list_that_deletes_into_the_trash_and_can_leave_its_folder() {
        let window = FilesWindow::new(PathBuf::from("/srv"), Some(Path::new("/nowhere/share")), true);
        assert_eq!(window.view, FileView::List);
        assert!(window.manager.is_trashing());
        assert!(!window.manager.is_confined());
        assert!(window.manager.follows_changes());
        assert_eq!(window.manager.root(), Path::new("/srv"));
    }

    #[test]
    fn a_window_follows_the_disk_only_when_asked() {
        let window = FilesWindow::new(PathBuf::from("/srv"), None, false);
        assert!(!window.manager.follows_changes());
        assert!(window.manager.is_trashing());
    }
}
