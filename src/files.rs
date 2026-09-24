//! The Files window: Quvyta's shared file manager, in a window of the desktop.
//!
//! The manager itself is the framework's [`FileManager`](qframe::widgets::FileManager): it reads
//! the folders, draws them and does every file operation. qdesk writes none of that and copies
//! none of it. What lives here is what only the desktop knows: which folder a window starts in,
//! the shape each window draws its folder in, where a deleted entry goes, what a file is opened
//! with, and which folders the other windows' programs stand in.
//!
//! Which program opens a file is the desktop's own databases' to say, read by the framework's
//! [`qframe::desktop`]. Every window of qdesk is a terminal, so only a terminal program is ever
//! taken from them; a graphical one could show nothing over SSH or on a console.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use qframe::desktop::{Choices, DesktopApp, Openers, XdgDirs};
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
    /// desktop was given none.
    ///
    /// The folders on screen are kept in step with what other programs do to them. `patience`
    /// bounds each wait for such a change, for a screen test, which runs the wait where it stands;
    /// `None` is the unbounded wait the running desktop makes on a thread of its own.
    ///
    /// The list is the first shape: on a server what is wanted of a folder is sizes, dates and
    /// permissions, which only the list shows.
    #[must_use]
    pub fn new(root: PathBuf, data_home: Option<&Path>, patience: Option<Duration>) -> Self {
        let manager = FileManagerState::new(root);
        let manager = match patience {
            Some(bound) => manager.following_within(bound),
            None => manager.following(true),
        };
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

/// The command that opens `file` with the program `app`, program first, as text; `None` when its
/// `Exec` line gives no command or a word of it is not text.
///
/// The line is split by the framework and never handed to a shell, so the file is one word
/// whatever its name holds.
#[must_use]
pub fn with_program(app: &DesktopApp, file: &Path) -> Option<Vec<String>> {
    app.command(file)?.into_iter().map(OsString::into_string).collect::<Result<_, _>>().ok()
}

/// The programs of `choices` a window of qdesk can hold: the terminal ones, the default first
/// when it is one of them, the rest in the order the databases rank them.
#[must_use]
pub fn terminal_programs(choices: &Choices) -> Vec<&DesktopApp> {
    let default = choices.default.and_then(|at| choices.apps.get(at));
    let mut programs: Vec<&DesktopApp> = default.filter(|app| app.terminal).into_iter().collect();
    for app in &choices.apps {
        if app.terminal && !programs.iter().any(|known| known.id == app.id) {
            programs.push(app);
        }
    }
    programs
}

/// The command that opens `file` when nothing else is asked for.
///
/// When the program the databases choose for the file's kind is a terminal program, it is that
/// program; when it is a graphical one, or there is none, it is [`opener`]: the person's editor,
/// else [`READER`]. A graphical program is never started, not even the only one there is: over
/// SSH and on a console it would have no screen to open on. `None` when the file's name is not
/// text.
#[must_use]
pub fn default_command(choices: &Choices, editor: Option<&str>, file: &Path) -> Option<Vec<String>> {
    choices
        .default
        .and_then(|at| choices.apps.get(at))
        .filter(|app| app.terminal)
        .and_then(|app| with_program(app, file))
        .or_else(|| opener(editor, file))
}

/// The name the editor fallback goes by on a menu: the program of `editor` without its folder,
/// else [`READER`].
#[must_use]
pub fn editor_name(editor: Option<&str>) -> String {
    editor
        .and_then(|editor| editor.split_whitespace().next())
        .map(|program| Path::new(program).file_name().map_or(program, |name| name.to_str().unwrap_or(program)))
        .unwrap_or(READER)
        .to_owned()
}

/// The desktop's databases of kinds and programs, read the first time a file is opened or its menu
/// asks which programs open it, and kept until the programs change.
///
/// Reading them takes every desktop entry of the machine, which a desktop that never opens a file
/// has no reason to pay for at start; kept, the second file opens without reading them again.
#[derive(Debug)]
pub struct Programs {
    dirs: XdgDirs,
    lang: String,
    path: Option<OsString>,
    read: OnceLock<Openers>,
}

impl Programs {
    /// The databases of the folders `dirs`, with program names in `lang` (`tr_TR.UTF-8`) and only
    /// the programs found on `path`; nothing is read yet.
    #[must_use]
    pub fn new(dirs: XdgDirs, lang: Option<&str>, path: Option<OsString>) -> Self {
        Self { dirs, lang: lang.unwrap_or_default().to_owned(), path, read: OnceLock::new() }
    }

    /// The databases, read now if they have not been.
    pub fn get(&self) -> &Openers {
        self.read.get_or_init(|| Openers::load(&self.dirs, &self.lang, self.path.as_deref()))
    }

    /// The kind of `file` and the programs that open it.
    #[must_use]
    pub fn for_file(&self, file: &Path) -> Choices {
        self.get().for_file(file)
    }

    /// The program of the desktop file id `id`, when it is installed and runs in a terminal.
    #[must_use]
    pub fn terminal_program(&self, id: &str) -> Option<&DesktopApp> {
        self.get().apps.get(id).filter(|app| app.terminal)
    }
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
        let window = FilesWindow::new(PathBuf::from("/srv"), Some(Path::new("/nowhere/share")), None);
        assert_eq!(window.view, FileView::List);
        assert!(window.manager.is_trashing());
        assert!(!window.manager.is_confined());
        assert!(window.manager.follows_changes());
        assert_eq!(window.manager.root(), Path::new("/srv"));
    }

    #[test]
    fn a_window_follows_the_disk_with_a_bounded_wait_too() {
        let window = FilesWindow::new(PathBuf::from("/srv"), None, Some(Duration::from_millis(20)));
        assert!(window.manager.follows_changes());
        assert!(window.manager.is_trashing());
    }

    fn program(id: &str, exec: &str, terminal: bool) -> DesktopApp {
        DesktopApp {
            id: id.to_owned(),
            name: id.trim_end_matches(".desktop").to_owned(),
            exec: exec.to_owned(),
            terminal,
            mime_types: vec!["text/plain".to_owned()],
            path: PathBuf::from(format!("/apps/{id}")),
            icon: None,
        }
    }

    fn choices(apps: Vec<DesktopApp>, default: Option<usize>) -> Choices {
        Choices { mime: "text/plain".to_owned(), apps, default }
    }

    #[test]
    fn a_terminal_program_chosen_for_the_kind_opens_the_file_as_one_word() {
        let file = Path::new("/srv/my \"odd\" notes.txt");
        let chosen = choices(vec![program("kedi.desktop", "kedi --read %f", true)], Some(0));
        assert_eq!(
            default_command(&chosen, Some("nano"), file),
            Some(vec!["kedi".into(), "--read".into(), "/srv/my \"odd\" notes.txt".into()])
        );
    }

    #[test]
    fn a_graphical_program_or_none_leaves_the_file_to_the_editor() {
        let file = Path::new("/srv/notes.txt");
        let graphical = choices(vec![program("gedit.desktop", "gedit %U", false)], Some(0));
        assert_eq!(default_command(&graphical, Some("nano"), file), Some(vec!["nano".into(), "/srv/notes.txt".into()]));
        assert_eq!(
            default_command(&choices(Vec::new(), None), None, file),
            Some(vec![READER.into(), "/srv/notes.txt".into()])
        );
    }

    #[test]
    fn only_terminal_programs_are_offered_the_default_first() {
        let offered = choices(
            vec![
                program("gedit.desktop", "gedit %U", false),
                program("vi.desktop", "vi %f", true),
                program("kedi.desktop", "kedi %f", true),
            ],
            Some(2),
        );
        let ids: Vec<&str> = terminal_programs(&offered).iter().map(|app| app.id.as_str()).collect();
        assert_eq!(ids, ["kedi.desktop", "vi.desktop"]);
    }

    #[test]
    fn the_editor_goes_by_its_program_s_name() {
        assert_eq!(editor_name(Some("/usr/bin/emacs -nw")), "emacs");
        assert_eq!(editor_name(Some("  ")), READER);
        assert_eq!(editor_name(None), READER);
    }
}
