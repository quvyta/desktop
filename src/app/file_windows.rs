//! The Files windows and opening a file or a folder in a window: the file manager of
//! each window, the "Open with" menu, and a terminal started in a folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use qframe::prelude::*;
use qframe::widgets::{ContextItem, FileManager, FileManagerMsg, FileView, RowMark, Toast};

use super::{Desk, Msg};
use crate::apps::{Category, Launch, Screen};
use crate::desktop;
use crate::files::{self, FilesWindow, Programs};
use crate::texts;
use crate::wallpapers;
use crate::wm::{self, WindowId};

impl Desk {
    /// Opens `folder` in a window of its own: the explorer's, else a Files window.
    pub(super) fn open_folder(&mut self, folder: PathBuf) -> Command<Msg> {
        let opened = match self.explorer_on(&folder) {
            Some(explorer) => self.open_entry(&explorer, true),
            None => {
                let entry = self.screen_entry("files", Screen::Files);
                self.open_files(&entry, folder, true)
            }
        };
        Command::batch([opened, self.body_focus()])
    }

    /// Opens `file`, chosen in a Files window or on the floor: a picture in qdesk's own picture
    /// viewer, a text, code or Markdown file in its text viewer ([`texts::is_text`]), and any other
    /// file in a terminal window of its own, started in the file's folder: with the terminal
    /// program the desktop's databases choose for its kind, else in the person's editor, else in
    /// [`files::READER`] (see [`files::default_command`]).
    pub(super) fn open_file(&mut self, file: &Path) -> Command<Msg> {
        let name = file.file_name().map(|name| name.to_string_lossy());
        if name.is_some_and(|name| wallpapers::is_picture(&name)) {
            return self.open_picture(file);
        }
        if texts::is_text(file) {
            return self.open_text(file);
        }
        let choices = self.openers.for_file(file);
        let words = files::default_command(&choices, self.apps.editor.as_deref(), file);
        self.open_file_with(file, words)
    }

    /// Opens `file` with the program chosen on its "Open with" menu: the terminal program of the
    /// desktop file id `program`, or the person's editor for `None`. A program that has gone since
    /// the menu opened leaves the file to the editor.
    pub(super) fn open_with(&mut self, file: &Path, program: Option<&str>) -> Command<Msg> {
        let openers = Arc::clone(&self.openers);
        let words = program
            .and_then(|id| openers.terminal_program(id))
            .and_then(|app| files::with_program(app, file))
            .or_else(|| files::opener(self.apps.editor.as_deref(), file));
        self.open_file_with(file, words)
    }

    /// Opens `file` in a window running `words`.
    ///
    /// Every terminal program is an application, and an editor is one; the window is the same as
    /// any command's, and stays when the program ends, so what it said last can be read. It is
    /// named after the file and drawn with the icon of the file's kind. A file whose name is not
    /// text (`words` is `None`) is not opened under a guessed name: the corner says why.
    pub(super) fn open_file_with(&mut self, file: &Path, words: Option<Vec<String>>) -> Command<Msg> {
        let name =
            file.file_name().map_or_else(|| file.display().to_string(), |name| name.to_string_lossy().into_owned());
        let Some(words) = words else {
            return Command::toast(Toast::info(t!("files.not-text", name = name.as_str())));
        };
        let icon = desktop::kind_icon(&name, false, false);
        let mut entry = Self::made_entry("files.open", &name, icon, Category::Files, Launch::Command(words));
        entry.folder = file.parent().map(Path::to_path_buf);
        let opened = self.open_entry(&entry, true);
        Command::batch([opened, self.body_focus()])
    }

    /// A Terminal window started in `folder`: the Terminal entry as the person has it, with the
    /// folder in place of the home folder.
    pub(super) fn terminal_here(&mut self, folder: PathBuf) -> Command<Msg> {
        let mut entry = self.screen_entry("terminal", Screen::Terminal);
        entry.folder = Some(folder);
        let opened = self.open_entry(&entry, true);
        Command::batch([opened, self.body_focus()])
    }

    /// What the file manager of the Files window `id` said: the manager takes it and answers with
    /// the reading and the file work it asks for. A message for a window that has closed since is
    /// dropped, as a late word of a closed window's program is.
    pub(super) fn on_files(&mut self, id: WindowId, message: FileManagerMsg) -> Command<Msg> {
        let Some(window) = self.files.get_mut(&id) else { return Command::none() };
        window.manager.update(message, move |message| Msg::Files(id, message))
    }

    /// The folder of a Files window, drawn by the framework's file manager in the window's shape.
    ///
    /// A file is opened in a terminal window, a folder's menu opens it in a new Files window or a
    /// terminal there, and the rows of folders another window's program stands in carry that
    /// window's icon.
    pub(super) fn files_body(&self, id: WindowId, files: &FilesWindow, ui: &mut View<'_, Msg>) {
        let icons = ui.env().icons();
        let standing = self.programs.folders().filter_map(|(window, folder)| {
            self.windows.get(window).map(|window| (folder, desktop::icon_name(window.entry(), icons)))
        });
        let marks = files::row_marks(files.manager.root(), standing);
        // The menu is built when it opens, long after this frame, so it takes the folders along.
        let folders = files.manager.folder_keys();
        let root = files.manager.root().to_path_buf();
        let openers = Arc::clone(&self.openers);
        let editor = self.apps.editor.clone();
        FileManager::new(&files.manager, move |message| Msg::Files(id, message))
            .view(files.view)
            .kind_icons(true)
            .on_open(|file| Msg::OpenFile(file.to_path_buf()))
            .on_open_terminal(|folder| Msg::TerminalHere(folder.to_path_buf()))
            .menu_items(move |key, _| {
                let path = key.split('/').filter(|part| !part.is_empty()).fold(root.clone(), |at, part| at.join(part));
                if key.is_empty() || folders.contains(key) {
                    vec![ContextItem::new(t!("files.open-new-window"), Msg::FilesHere(path))]
                } else {
                    let picture = wallpapers::is_picture(key.rsplit('/').next().unwrap_or(key));
                    let mut items = vec![Self::open_with_menu(&openers, editor.as_deref(), path.clone())];
                    if picture {
                        items.push(ContextItem::new(t!("files.set-wallpaper"), Msg::SetWallpaper(path)));
                    }
                    items
                }
            })
            .row_mark(move |key| marks.get(key).cloned().unwrap_or_else(RowMark::new))
            .show(ui)
            .id(wm::view::body_id(id))
            .fill();
    }

    /// The "Open with" row of a file's menu: the terminal programs the desktop's databases name
    /// for the file's kind, the default first, and the person's editor last, which opens any file.
    ///
    /// Graphical programs are left out, as they would have no screen to open on. The databases
    /// are read here, when the menu opens, not while the window is drawn.
    pub(super) fn open_with_menu(openers: &Programs, editor: Option<&str>, file: PathBuf) -> ContextItem<Msg> {
        let choices = openers.for_file(&file);
        let mut items: Vec<ContextItem<Msg>> = files::terminal_programs(&choices)
            .into_iter()
            .map(|app| ContextItem::new(app.name.clone(), Msg::OpenWith(file.clone(), Some(app.id.clone()))))
            .collect();
        let program = files::editor_name(editor);
        let label = if editor.is_some_and(|editor| !editor.trim().is_empty()) {
            t!("files.open-with-editor", program = program.as_str())
        } else {
            t!("files.open-with-reader", program = program.as_str())
        };
        items.push(ContextItem::new(label, Msg::OpenWith(file, None)));
        ContextItem::submenu(t!("files.open-with"), items)
    }

    /// The shape the folder of the Files window `id` is drawn in, chosen on its menu.
    pub(super) fn files_view(&mut self, id: WindowId, view: FileView) -> Command<Msg> {
        if let Some(window) = self.files.get_mut(&id) {
            window.view = view;
        }
        Command::none()
    }
}
