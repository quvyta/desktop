//! A running desktop follows its settings file.
//!
//! Another program changes the floor by running `qdesk wallpaper`, which writes `desktop.conf`;
//! a person may edit the file by hand. Either way the desktop that is already open sees the file
//! change, reads it again and applies what it says, without a restart. The folder is watched with
//! the framework's [`FolderWatch`], as the entry folders are: nothing is polled, and a screen test
//! bounds each wait by its patience.
//!
//! qdesk writes the same file itself whenever something is chosen on the Settings screen. Those
//! writes are remembered by their text, and a change that brings back a text qdesk wrote is its
//! own: nothing is read into the desktop for it while a newer write of its own is still on the way,
//! so two quick choices never show the first one again for a moment. Whatever else is read is
//! compared value by value with what is in force, and only what differs is applied, so reading
//! the file never writes it and never starts a loop.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use qframe::prelude::*;
use qframe::storage::{FolderChangeKind, FolderChanges, FolderWatch, Settings};
use qframe::t;
use qframe::widgets::Toast;

use super::{Desk, Msg};
use crate::apps::Diagnostic;
use crate::inbox::Notice;
use crate::settings::{self, Loaded};

/// How many of its own writes the desktop remembers. A write is forgotten once it is seen, so the
/// list only grows while writes are waiting to land; this bounds it if some are never seen.
const REMEMBERED_WRITES: usize = 16;

/// What one wait on the settings folder heard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderNews {
    /// The settings file was written, replaced, removed, or the folder may have lost track of it.
    File,
    /// Something else in the folder changed: another Quvyta application's file.
    Elsewhere,
    /// A bounded wait ended with nothing changed. Only a desktop given a bound by
    /// [`Desk::watch_within`] — a screen test — ever hears this.
    Quiet,
    /// The watch was let go.
    Dropped,
}

impl Desk {
    /// Starts watching the folder the settings file is in, so a change made to the file by another
    /// program or by hand is applied while the desktop runs. Settings kept only in memory have no
    /// file to follow.
    pub(super) fn watch_settings(&mut self) -> Command<Msg> {
        let Some(file) = self.stored.path().map(Path::to_path_buf) else { return Command::none() };
        let Some(folder) = file.parent() else { return Command::none() };
        // The first save makes the folder anyway. Made now, a file another program writes into
        // it later is seen; without it nothing could be watched until the next start.
        let _ = fs::create_dir_all(folder);
        let Ok(mut watch) = FolderWatch::new() else {
            // Without a watch the desktop works as before: the file is read at the next start.
            return Command::none();
        };
        if watch.watch(folder).is_err() {
            return Command::none();
        }
        self.settings_watch = Some(watch);
        self.settings_run += 1;
        self.wait_settings()
    }

    /// Waits for the next change of the settings folder, on the watch of this run.
    fn wait_settings(&self) -> Command<Msg> {
        let (Some(watch), Some(file)) = (&self.settings_watch, self.stored.path()) else { return Command::none() };
        let name = file.file_name().map(ToOwned::to_owned).unwrap_or_default();
        wait(watch.changes(), name.into(), self.settings_run, self.patience)
    }

    /// What one wait on the settings folder heard means.
    pub(super) fn on_settings_folder(&mut self, run: u64, news: FolderNews) -> Command<Msg> {
        if run != self.settings_run {
            // A wait of a watch that has been replaced.
            return Command::none();
        }
        match news {
            FolderNews::Dropped => Command::none(),
            FolderNews::Quiet | FolderNews::Elsewhere => self.wait_settings(),
            FolderNews::File => {
                let followed = self.follow_file();
                Command::batch([followed, self.wait_settings()])
            }
        }
    }

    /// Remembers the settings as they are about to be written by the desktop itself, so the
    /// change that write makes to the file is known as its own.
    pub(super) fn remember_write(&mut self) {
        if self.stored.path().is_none() {
            return;
        }
        self.written.push(self.stored.to_toml());
        if self.written.len() > REMEMBERED_WRITES {
            self.written.remove(0);
        }
    }

    /// Reads the settings file again and applies what differs from what is in force.
    fn follow_file(&mut self) -> Command<Msg> {
        let Some(path) = self.stored.path().map(Path::to_path_buf) else { return Command::none() };
        let loaded = settings::reread(&path);
        let text = loaded.settings.to_toml();
        if let Some(at) = self.written.iter().position(|written| *written == text) {
            self.written.drain(..=at);
            if !self.written.is_empty() {
                // A newer write of the desktop's own is on its way; it is followed when it lands.
                return Command::none();
            }
        }
        self.follow(loaded)
    }

    /// Takes `loaded` as the settings in force and applies whatever differs.
    ///
    /// Everything of the desktop's own is read from the preferences where it is used, so a changed
    /// value is the next frame's: the floor's colour, pattern and picture (once it is decoded), the dock's edge, the status
    /// strip, folders in the explorer, the drag style and the frame cap. The scrollback is the
    /// next terminal window's, as it is when it is changed on the Settings screen. Of the keys
    /// every Quvyta application shares, a theme, a language, a glyph mode, reduced motion, the
    /// pillar and the slide written in the file are applied; a key taken out of the file, or set
    /// to follow the ecosystem, leaves what is on screen until the next start.
    fn follow(&mut self, loaded: Loaded) -> Command<Msg> {
        let Loaded { settings, prefs, wallpaper, diagnostics } = loaded;
        let shared = shared_changes(&self.stored, &settings);
        self.stored = settings;
        let mut commands = vec![shared, self.follow_wallpaper(wallpaper)];
        if prefs != self.prefs {
            self.prefs = prefs;
            self.programs.set_prefs(prefs, self.remote);
            // The strip switched on reads the machine again, when nothing else was.
            commands.push(self.sample(Duration::ZERO));
        }
        if self.screen.reread(diagnostics.clone()) {
            // Said in the corner too: a person who has just saved the file by hand looks at the
            // desktop, not at the Settings screen, to see whether it was taken.
            commands.extend(diagnostics.iter().map(|problem| self.unread(problem)));
        }
        Command::batch(commands)
    }

    /// The notification one problem of the settings file becomes: said in the corner and kept in
    /// the list, with the place in the file and what the framework could not use there.
    fn unread(&mut self, problem: &Diagnostic) -> Command<Msg> {
        let heading = t!("notice.settings-unread");
        let place = problem.location();
        let body = match &problem.detail {
            Some(detail) => format!("{place}: {detail}"),
            None => place,
        };
        self.inbox.add(Notice::desktop(heading.clone(), body.clone()));
        Command::toast(Toast::warning(heading).body(body))
    }
}

/// The commands that apply the keys every Quvyta application shares which `after` sets to
/// something other than `before` did.
fn shared_changes(before: &Settings, after: &Settings) -> Command<Msg> {
    let mut commands = Vec::new();
    if let Some(theme) = after.theme().filter(|theme| before.theme().as_ref() != Some(theme)) {
        commands.push(Command::set_theme(theme));
    }
    if let Some(language) = after.language().filter(|language| before.language().as_ref() != Some(language)) {
        commands.push(Command::set_locale(language));
    }
    if let Some(mode) = after.icon_mode().filter(|mode| before.icon_mode() != Some(*mode)) {
        commands.push(Command::set_icon_mode(mode));
    }
    if let Some(reduced) = after.reduced_motion().filter(|reduced| before.reduced_motion() != Some(*reduced)) {
        commands.push(Command::set_reduced_motion(reduced));
    }
    if let Some(style) = after.pillar_style().filter(|style| before.pillar_style() != Some(*style)) {
        commands.push(Command::set_pillar(style));
    }
    if let Some(slide) = after.slide().filter(|slide| before.slide() != Some(*slide)) {
        commands.push(Command::set_slide(slide));
    }
    Command::batch(commands)
}

/// Waits for the next batch of changes of the settings folder's watch of run `run`, and says
/// whether the file named `file` was among them.
///
/// With a `patience` the wait lasts at most that long, and ends with [`FolderNews::Quiet`] when
/// nothing changed; without one it lasts until something does.
fn wait(changes: FolderChanges, file: PathBuf, run: u64, patience: Option<Duration>) -> Command<Msg> {
    Command::perform(move || {
        let batch = match patience {
            None => changes.next(),
            Some(bound) => match changes.next_within(bound) {
                Some(batch) => batch,
                None => return Msg::SettingsFolder(run, FolderNews::Quiet),
            },
        };
        if batch.is_empty() {
            return Msg::SettingsFolder(run, FolderNews::Dropped);
        }
        // The framework writes the file whole under another name and renames it over the old one,
        // so the file's own name is in the batch however it was written. A folder that lost
        // track of its entries may have lost this one too.
        let touched = batch.iter().any(|change| {
            change.name.as_deref() == Some(file.as_os_str())
                || matches!(change.kind, FolderChangeKind::Overflow | FolderChangeKind::Gone)
        });
        Msg::SettingsFolder(run, if touched { FolderNews::File } else { FolderNews::Elsewhere })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::FloorStyle;

    /// A desktop whose settings file is in a folder of the test's own.
    fn desk_in(folder: &Path) -> Desk {
        let loaded = settings::load_in(folder);
        Desk::new(None, None, Box::new(|| 0)).settings(loaded.settings, loaded.prefs, loaded.diagnostics)
    }

    #[test]
    fn an_older_write_of_its_own_landing_does_not_take_back_a_newer_choice() {
        let folder = std::env::temp_dir().join(format!("qdesk-follow-own-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).expect("folder");
        let mut desk = desk_in(&folder);

        // Two choices in a row: the first is written, the second is still on its way.
        desk.prefs.floor = settings::FloorColor::Deep;
        desk.prefs.write(&mut desk.stored);
        desk.remember_write();
        desk.stored.clone().save().expect("the first write");
        desk.prefs.floor_style = FloorStyle::Gradient;
        desk.prefs.write(&mut desk.stored);
        desk.remember_write();

        // The first write lands and is read back: the second choice stays in force.
        let _ = desk.follow_file();
        assert_eq!(desk.prefs.floor_style, FloorStyle::Gradient, "the newer choice is not taken back");
        assert_eq!(desk.written.len(), 1, "the first write is forgotten once it is seen");

        // The second lands: nothing is left to wait for and nothing changes.
        desk.stored.clone().save().expect("the second write");
        let _ = desk.follow_file();
        assert!(desk.written.is_empty());
        assert_eq!((desk.prefs.floor, desk.prefs.floor_style), (settings::FloorColor::Deep, FloorStyle::Gradient));

        // A write of another program is followed.
        fs::write(folder.join("desktop.conf"), "floor-color = \"mist\"\n").expect("file");
        let _ = desk.follow_file();
        assert_eq!((desk.prefs.floor, desk.prefs.floor_style), (settings::FloorColor::Mist, FloorStyle::Plain));
        let _ = fs::remove_dir_all(&folder);
    }
}
