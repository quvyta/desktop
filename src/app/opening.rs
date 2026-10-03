//! Opening an application: the launcher that lists them, and the window an entry
//! opens in, with its program or a screen qdesk draws itself.

use std::path::{Path, PathBuf};

use qframe::prelude::*;
use qframe::widgets::Toast;

use super::{Desk, EXPLORER, FLOOR, Msg, Target};
use crate::apps::{
    Category, Entry, Install, Launch, Localized, Screen, Source, WindowPrefs, find_program, is_executable,
};
use crate::files::FilesWindow;
use crate::launcher::{self, Launcher, Shelf};
use crate::texts;
use crate::wallpapers;

/// The target with this id.
fn find(targets: &[Target], id: &str) -> Option<Target> {
    targets.iter().find(|target| target.id == id).cloned()
}

impl Desk {
    /// What the launcher's messages mean.
    pub(super) fn on_launcher(&mut self, message: launcher::Msg) -> Command<Msg> {
        let Some(launcher) = &mut self.launcher else { return Command::none() };
        match message {
            launcher::Msg::Close => {
                self.launcher = None;
                return Command::focus(FLOOR);
            }
            // A space typed into the empty search is the launcher's key pressed again, not a search.
            launcher::Msg::Query(query) if launcher::closes(&launcher.query, &query) => {
                self.launcher = None;
                return Command::focus(FLOOR);
            }
            launcher::Msg::Query(query) => {
                launcher.query = query;
                // The first hit is what Enter opens, so the card on it is the one shown as chosen.
                launcher.selected = Some(0);
            }
            launcher::Msg::Shelf(key) => {
                if let Some(shelf) = Shelf::from_key(&key) {
                    launcher.shelf = shelf;
                    launcher.selected = None;
                }
            }
            launcher::Msg::Select(index) => launcher.selected = Some(index),
            // The rest carry what they act on; the view turns them into the messages above.
            launcher::Msg::Submit
            | launcher::Msg::Open(_)
            | launcher::Msg::AddIcon(_)
            | launcher::Msg::RemoveIcon(_)
            | launcher::Msg::Install(_)
            | launcher::Msg::Power(_) => {}
        }
        Command::none()
    }

    /// Opens an application: a window with its program running in it, or a screen qdesk draws
    /// itself. With `fresh` a window of its own opens even for an entry that otherwise opens once.
    pub(super) fn open(&mut self, target: &Target, fresh: bool) -> Command<Msg> {
        self.desktop.remember(&target.id);
        // Opening an application puts the launcher away, as it does on any desktop.
        let closing = if self.launcher.take().is_some() { Command::focus(FLOOR) } else { Command::none() };
        let saved = self.save();
        let Some(entry) = self.catalog.get(&target.id).cloned() else {
            return Command::batch([closing, saved]);
        };
        let opened = match &entry.launch {
            // A folder opens where every folder opens: the explorer, else a Files window.
            Launch::Open(path) if path.is_dir() => match self.explorer_on(path) {
                Some(explorer) => self.open_entry(&explorer, true),
                None => self.open_files(&entry, path.clone(), fresh),
            },
            // A picture opens in the desktop's own viewer.
            Launch::Open(path)
                if path.file_name().is_some_and(|name| wallpapers::is_picture(&name.to_string_lossy())) =>
            {
                self.open_picture(path)
            }
            // Text, code and Markdown open in its text viewer.
            Launch::Open(path) if texts::is_text(path) => self.open_text(path),
            // Any other file waits for the viewers; opening a window that could show nothing
            // would be worse than saying so.
            Launch::Open(_) => {
                let toast = Toast::info(t!("notice.no-viewer", name = target.name.as_str()));
                return Command::batch([closing, saved, Command::toast(toast)]);
            }
            Launch::Screen(Screen::Files) => {
                // The home folder is where a person's own files are; a machine that names none
                // still has its root to look through.
                let home = self.apps.home.clone().unwrap_or_else(|| PathBuf::from("/"));
                self.open_files(&entry, home, fresh)
            }
            Launch::Command(_) | Launch::Screen(Screen::Terminal | Screen::Settings) => self.open_entry(&entry, fresh),
        };
        Command::batch([closing, saved, opened, self.body_focus()])
    }

    /// Opens a window for `entry` and starts what it holds, or brings its window forward when the
    /// entry opens only once and no window of its own was asked for.
    ///
    /// Opening a window gives the keys to it: the person asked for that window, so the desktop
    /// steps out of the way even when they came from desktop mode.
    pub(super) fn open_entry(&mut self, entry: &Entry, fresh: bool) -> Command<Msg> {
        self.keys = None;
        let open = if entry.single && !fresh { self.windows.of_entry(&entry.id) } else { None };
        let Some(id) = open else {
            let id = self.windows.open(entry);
            let started = self.start(id, entry);
            return Command::batch([started, self.resize_hint()]);
        };
        // It may be on another workspace; the launcher takes the person there.
        self.windows.bring(id);
        Command::none()
    }

    /// Opens a Files window for `entry` onto the folder `root` and starts reading it, or brings the
    /// entry's window forward when it opens only once and no window of its own was asked for.
    ///
    /// Each window has a manager of its own, kept under the window's id and let go with it. The
    /// folders on screen are followed, in a screen test too, whose waits are bounded by the
    /// desktop's patience (see [`watch_within`](Self::watch_within)).
    pub(super) fn open_files(&mut self, entry: &Entry, root: PathBuf, fresh: bool) -> Command<Msg> {
        self.keys = None;
        if entry.single
            && !fresh
            && let Some(id) = self.windows.of_entry(&entry.id)
        {
            // It may be on another workspace; the launcher takes the person there.
            self.windows.bring(id);
            return Command::none();
        }
        let id = self.windows.open(entry);
        let mut window = FilesWindow::new(root, self.apps.data_home.as_deref(), self.patience);
        let read = window.manager.load(move |message| Msg::Files(id, message));
        self.files.insert(id, window);
        Command::batch([read, self.resize_hint()])
    }

    /// The window that shows `folder` in the ecosystem's file explorer, when Settings says folders
    /// open there and [`EXPLORER`] is on the `PATH` of the environment — the same `PATH` the entries' programs
    /// are looked for in, so a test decides whether there is one. `None` opens a Files window.
    ///
    /// The window is named after the folder and started in it, and its program is given the folder
    /// by its whole path; a folder whose path is not text keeps the Files window, rather than
    /// giving the explorer a guessed name.
    pub(super) fn explorer_on(&self, folder: &Path) -> Option<Entry> {
        if !self.prefs.folders_in_explorer {
            return None;
        }
        let program = find_program(EXPLORER, self.apps.path.as_deref(), is_executable)?;
        let words = vec![program.to_str()?.to_owned(), folder.to_str()?.to_owned()];
        let name =
            folder.file_name().map_or_else(|| folder.display().to_string(), |name| name.to_string_lossy().into_owned());
        let mut entry = Self::made_entry(EXPLORER, &name, "folder", Category::Files, Launch::Command(words));
        entry.folder = Some(folder.to_path_buf());
        Some(entry)
    }

    /// The entry of the screen `screen`: the one of id `id` in the catalog, which a person may have
    /// changed, else one made here — a person may hide a built-in entry from the launcher, and a
    /// folder's "Open a terminal here" still has to open a terminal.
    pub(super) fn screen_entry(&self, id: &str, screen: Screen) -> Entry {
        match self.catalog.get(id) {
            Some(entry) if entry.launch == Launch::Screen(screen) => entry.clone(),
            _ => {
                let (name, icon, category) = match screen {
                    Screen::Terminal => ("Terminal", "terminal", Category::System),
                    Screen::Files => ("Files", "folder", Category::Files),
                    Screen::Settings => ("Settings", "settings", Category::System),
                };
                Self::made_entry(id, name, icon, category, Launch::Screen(screen))
            }
        }
    }

    /// An entry the desktop makes for a window of its own, read from no file.
    pub(super) fn made_entry(id: &str, name: &str, icon: &str, category: Category, launch: Launch) -> Entry {
        Entry {
            id: id.to_owned(),
            name: Localized::plain(name),
            comment: None,
            icon: Some(icon.to_owned()),
            launch,
            folder: None,
            env: Vec::new(),
            unset: Vec::new(),
            category,
            keywords: Vec::new(),
            single: false,
            close_on_exit: false,
            window: WindowPrefs::default(),
            install: Install::default(),
            try_exec: None,
            source: Source::Builtin,
            file: None,
        }
    }

    /// The launcher, rising from the left above the dock.
    pub(super) fn launcher_view(&self, launcher: &Launcher, ui: &mut View<'_, Msg>) {
        let language = ui.env().i18n().active().to_owned();
        let apps = launcher::shown(&self.catalog, &self.desktop, launcher, &language, ui.env().icons());
        let shelves = launcher::shelves(&self.catalog, &self.desktop, &language);
        let targets: Vec<Target> =
            apps.iter().filter_map(|app| self.catalog.get(&app.id)).map(|entry| Target::of(entry, &language)).collect();
        let actions = self.tools.offered(self.remote);
        self.against_dock(ui, |ui| {
            ui.row(|ui| {
                ui.map(
                    move |message| match message {
                        launcher::Msg::Submit => targets.first().cloned().map_or(Msg::Ignore, Msg::Open),
                        launcher::Msg::Open(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::Open),
                        launcher::Msg::Install(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::Install),
                        launcher::Msg::AddIcon(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::AddIcon),
                        launcher::Msg::RemoveIcon(id) => find(&targets, &id).map_or(Msg::Ignore, Msg::RemoveIcon),
                        launcher::Msg::Power(action) => Msg::Power(action),
                        other => Msg::Launcher(other),
                    },
                    |ui| launcher::view(launcher, &apps, &shelves, &actions, ui),
                );
                ui.spacer();
            });
        });
    }

    /// Opens the launcher with the keys in its search.
    pub(super) fn open_launcher(&mut self) -> Command<Msg> {
        self.launcher = Some(Launcher::default());
        Command::focus(launcher::SEARCH)
    }

    /// The launcher's key and the dock's button: the launcher opens, or goes away when it is open.
    pub(super) fn toggle_launcher(&mut self) -> Command<Msg> {
        if self.launcher.is_some() {
            self.launcher = None;
            return Command::focus(FLOOR);
        }
        self.open_launcher()
    }
}
