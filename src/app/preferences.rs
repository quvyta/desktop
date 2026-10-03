//! The Settings screen inside its window: what its changes mean, writing them, the appearance
//! section, and the question for a newer version of qdesk.

use std::time::Duration;

use qframe::prelude::*;
use qframe::runtime::UpdateCheck;
use qframe::storage::Ecosystem;
use qframe::widgets::AppearanceChange;

use super::{Desk, Msg, recommended};
use crate::settings;

impl Desk {
    /// What the Settings screen of a window asks for: the change is already applied, what is left
    /// is writing it.
    pub(super) fn on_settings(&mut self, message: settings::Msg) -> Command<Msg> {
        // The button that had the keyboard goes with the notice; the list takes it back.
        let read = message == settings::Msg::ReadProblems;
        let (applied, request) = settings::update::<Msg>(&mut self.screen, &self.prefs, message);
        let stored = match request {
            Some(settings::Request::Prefs(prefs)) => {
                self.prefs = prefs;
                // A running pseudo-terminal keeps what it was opened with; the windows opened from
                // now on take the new numbers.
                self.programs.set_prefs(prefs, self.remote);
                prefs.write(&mut self.stored);
                // The strip switched on reads the machine again, when nothing else was.
                let reading = self.sample(Duration::ZERO);
                Command::batch([self.store(), reading])
            }
            Some(settings::Request::Appearance(change)) => self.on_appearance(change),
            Some(settings::Request::Wallpaper(asked)) => self.on_wallpaper_asked(asked),
            Some(settings::Request::ShowRecommended) => {
                self.recommended_open = true;
                Command::focus(recommended::LIST)
            }
            None => Command::none(),
        };
        let focus = if read { self.body_focus() } else { Command::none() };
        Command::batch([applied, stored, focus])
    }

    /// The question for a newer version of qdesk, when the ecosystem's update notice is on.
    ///
    /// The switch is read here, not only where the question is sent: a person who turned it off
    /// asks nothing at all, whoever runs the question.
    pub(super) fn ask_for_update(&self) -> Command<Msg> {
        let Some(folders) = &self.updates else { return Command::none() };
        if !Ecosystem::QUVYTA.update_notice_in(&folders.config) {
            return Command::none();
        }
        let check = UpdateCheck::new(
            Ecosystem::QUVYTA,
            settings::APP,
            env!("CARGO_PKG_NAME"),
            env!("CARGO_PKG_VERSION"),
            Msg::NewVersion,
        )
        .in_folders(folders.config.clone(), folders.state.clone());
        Command::check_for_update(check)
    }

    /// A change on the appearance section: the section applies it and saves it in the file the box
    /// under its row names, the shared one or `desktop.conf`, and keeps the settings in memory as the
    /// file now says them. A change to `desktop.conf` is then written once more from those
    /// settings, as every other choice on the screen is: a write of the desktop's own still on its
    /// way, made before this change, cannot then be the last word in the file and take it back.
    pub(super) fn on_appearance(&mut self, change: AppearanceChange) -> Command<Msg> {
        let shared_only = matches!(change, AppearanceChange::UpdateNotice(_));
        let shown = self.appearance.update(change, &mut self.stored);
        if shared_only {
            return shown;
        }
        Command::batch([shown, self.store()])
    }

    /// Writes the settings on a background thread, so a slow disk never holds up drawing. The
    /// text is remembered, so the watch on the file knows the change as the desktop's own.
    pub(super) fn store(&mut self) -> Command<Msg> {
        self.remember_write();
        self.stored.save_command(|result| Msg::Settings(settings::Msg::Stored(result)))
    }

    /// The Settings screen inside its window.
    pub(super) fn settings_body(&self, ui: &mut View<'_, Msg>) {
        let folders = self.apps.folders();
        let apps = settings::Applications { folders: &folders, diagnostics: &self.notices };
        settings::view(&self.screen, &self.prefs, self.remote, &self.appearance, &apps, ui);
    }
}
