//! The picture over the floor (design 3.9).
//!
//! The settings name a picture by its path; the desktop decodes it off the drawing thread with the
//! framework's decoder, shrunk to [`wallpapers::MOST`], and the framework's [`Image`] lays it over
//! the whole floor with [`Fit::Cover`]. The image works its cells out once for a size and keeps
//! them, so a frame that repaints the floor only copies them, and the terminal is sent only the
//! cells that changed.
//!
//! Until the picture is decoded, where it cannot be decoded, and in sixteen colours or ASCII
//! glyphs, where pixels cannot be drawn, the floor is its colour and pattern as if no picture were
//! set. A picture a person chooses — from a file's menu, the Settings screen or its file picker —
//! is decoded first and written into the settings only when it decodes, so a broken file never
//! becomes the setting. A picture the settings file names is the person's: when it cannot be
//! shown the desktop says why and leaves the file as it is.

use std::path::{Path, PathBuf};

use qframe::color::ColorDepth;
use qframe::env::Env;
use qframe::icons::GlyphMode;
use qframe::prelude::*;
use qframe::storage::{UserDir, user_dir_in};
use qframe::widgets::{
    FileBrowser, FilePicker, FilePickerMsg, Fit, Image, ImageData, ImageError, Modal, PickMode, Toast,
};

use super::{Desk, Msg};
use crate::inbox::Notice;
use crate::settings::{self, WallpaperRow};
use crate::wallpapers::{self, EXTENSIONS, MOST, OURS};

/// The widget id of the picture on the floor, so it keeps its worked-out cells between frames.
pub const PICTURE: &str = "floor-picture";

/// The widget id of the file picker that chooses a picture.
pub const PICKER: &str = "wallpaper-picker";

/// The width of the picker's dialog, in cells: its footer holds the hidden-files switch, the endings it
/// shows and its button on one row.
const PICKER_WIDTH: u16 = 80;

/// The rows the picker takes inside its dialog: a path, a filter, a list of about a dozen entries
/// and its footer.
const PICKER_HEIGHT: u16 = 18;

/// What became of the picture the settings name.
#[derive(Debug, Clone, Default)]
pub enum Shown {
    /// No picture is set.
    #[default]
    Nothing,
    /// It is being decoded; the floor is its colour meanwhile.
    Decoding,
    /// It is decoded and drawn.
    Ready(ImageData),
    /// It could not be decoded, for this reason.
    Failed(ImageError),
}

/// Why a picture is being decoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Purpose {
    /// The settings name it: it is drawn when it decodes and said when it cannot.
    Settings,
    /// A person chose it: it is written into the settings only when it decodes.
    Chosen,
}

/// The floor's picture and the dialog that chooses one.
#[derive(Debug, Default)]
pub struct Wallpaper {
    /// The picture the settings name.
    path: Option<PathBuf>,
    /// What became of it.
    shown: Shown,
    /// Which decoding is the current one; the answer of an older one is dropped.
    run: u64,
    /// The file picker, while it is open.
    picker: Option<FileBrowser>,
}

impl Wallpaper {
    /// The picture the settings name.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }

    /// What became of it.
    #[must_use]
    pub fn shown(&self) -> &Shown {
        &self.shown
    }

    /// Whether the file picker is open.
    #[must_use]
    pub fn picking(&self) -> bool {
        self.picker.is_some()
    }

    /// The decoded picture, when there is one to draw.
    fn ready(&self) -> Option<&ImageData> {
        match &self.shown {
            Shown::Ready(data) => Some(data),
            _ => None,
        }
    }
}

/// Whether `env`'s terminal can draw a picture: 256 colours or more, and the block elements a
/// half block is. The framework's image answers the same question for itself, but where it cannot
/// draw it says so in the middle of its area, which on a floor would be a sentence across the
/// desktop; the floor asks first and stays its colour.
#[must_use]
pub fn can_draw(env: &Env) -> bool {
    env.depth() != ColorDepth::Ansi16 && env.glyph_mode() != GlyphMode::Ascii
}

impl Desk {
    /// Takes the picture the settings name as the one in force, without decoding it yet: what a
    /// desktop is built with, before it starts.
    pub(super) fn name_wallpaper(&mut self, path: Option<PathBuf>) {
        self.wallpaper.shown = if path.is_some() { Shown::Decoding } else { Shown::Nothing };
        self.wallpaper.path = path;
        self.show_wallpaper_row();
    }

    /// Decodes the picture the settings name, when they name one: the first thing the desktop
    /// does about it once it starts.
    pub(super) fn start_wallpaper(&mut self) -> Command<Msg> {
        match self.wallpaper.path.clone() {
            Some(path) => self.decode_wallpaper(path, Purpose::Settings),
            None => Command::none(),
        }
    }

    /// Takes `path` as the picture the settings file now names, when it is not the one in force:
    /// the file was changed by another program or by hand.
    pub(super) fn follow_wallpaper(&mut self, path: Option<PathBuf>) -> Command<Msg> {
        if path == self.wallpaper.path {
            return Command::none();
        }
        self.name_wallpaper(path);
        // A decoding still on its way is for the old picture.
        self.wallpaper.run += 1;
        self.start_wallpaper()
    }

    /// Decodes `path` off the drawing thread, for `purpose`.
    fn decode_wallpaper(&mut self, path: PathBuf, purpose: Purpose) -> Command<Msg> {
        self.wallpaper.run += 1;
        let run = self.wallpaper.run;
        Command::perform(move || {
            let decoded = ImageData::decode_file(&path, MOST);
            Msg::WallpaperDecoded(run, path, purpose, decoded)
        })
    }

    /// A person chose `path` as the wallpaper: from a file's menu, the file picker or one of
    /// qdesk's own pictures. It is decoded first.
    pub(super) fn choose_wallpaper(&mut self, path: PathBuf) -> Command<Msg> {
        self.decode_wallpaper(path, Purpose::Chosen)
    }

    /// A decoding of run `run` ended.
    pub(super) fn on_wallpaper_decoded(
        &mut self,
        run: u64,
        path: PathBuf,
        purpose: Purpose,
        decoded: Result<ImageData, ImageError>,
    ) -> Command<Msg> {
        if run != self.wallpaper.run {
            return Command::none();
        }
        match (decoded, purpose) {
            (Ok(data), Purpose::Settings) => {
                self.wallpaper.shown = Shown::Ready(data);
                self.show_wallpaper_row();
                Command::none()
            }
            (Ok(data), Purpose::Chosen) => {
                if !settings::set_wallpaper(&mut self.stored, Some(&path)) {
                    let reason = t!("wallpaper.not-text");
                    return Self::refused_wallpaper(&path, &reason);
                }
                self.wallpaper.path = Some(path);
                self.wallpaper.shown = Shown::Ready(data);
                self.show_wallpaper_row();
                self.store()
            }
            (Err(problem), Purpose::Settings) => {
                self.wallpaper.shown = Shown::Failed(problem);
                self.show_wallpaper_row();
                let heading = t!("wallpaper.unshown");
                let body = format!("{}: {problem}", path.display());
                self.inbox.add(Notice::desktop(heading.clone(), body.clone()));
                Command::toast(Toast::warning(heading).body(body))
            }
            // What was in force stays: the picture before it, or none.
            (Err(problem), Purpose::Chosen) => Self::refused_wallpaper(&path, &problem.to_string()),
        }
    }

    /// Says that the picture at `path`, which a person chose, is not the wallpaper, and why.
    fn refused_wallpaper(path: &Path, reason: &str) -> Command<Msg> {
        let name =
            path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned());
        Command::toast(Toast::warning(t!("wallpaper.refused", name = name.as_str())).body(reason.to_owned()))
    }

    /// Takes the picture away: the floor is its colour and pattern again.
    pub(super) fn remove_wallpaper(&mut self) -> Command<Msg> {
        self.wallpaper.run += 1;
        self.name_wallpaper(None);
        settings::set_wallpaper(&mut self.stored, None);
        self.store()
    }

    /// What the Settings screen asks about the picture.
    pub(super) fn on_wallpaper_asked(&mut self, asked: settings::Wallpaper) -> Command<Msg> {
        match asked {
            settings::Wallpaper::Choose => self.open_wallpaper_picker(),
            settings::Wallpaper::Remove => self.remove_wallpaper(),
            settings::Wallpaper::Ours(index) => {
                let Some(ours) = OURS.get(index).copied() else { return Command::none() };
                let Some(data) = self.apps.data_home.clone() else {
                    return Command::toast(Toast::warning(t!("wallpaper.nowhere")));
                };
                // Written off the drawing thread, like every other write of the desktop.
                Command::perform(move || {
                    Msg::OurWallpaper(wallpapers::put(ours, &data).map_err(|error| error.to_string()))
                })
            }
        }
    }

    /// One of qdesk's own pictures was written into its folder, or could not be.
    pub(super) fn on_our_wallpaper(&mut self, written: Result<PathBuf, String>) -> Command<Msg> {
        match written {
            Ok(path) => self.choose_wallpaper(path),
            Err(reason) => Command::toast(Toast::warning(t!("wallpaper.nowhere")).body(reason)),
        }
    }

    /// Opens the file picker on the person's Pictures folder, else their home folder, showing only
    /// the pictures that can be decoded.
    fn open_wallpaper_picker(&mut self) -> Command<Msg> {
        let start = self.pictures_folder();
        let mut browser = FileBrowser::new(start.clone(), PickMode::Files).extensions(EXTENSIONS);
        let reading = browser.open(start, Msg::WallpaperPicker);
        self.wallpaper.picker = Some(browser);
        Command::batch([reading, Command::focus(PICKER)])
    }

    /// Where the picker starts: the Pictures folder the XDG user directories name, when it is
    /// there, else the home folder, else the root.
    fn pictures_folder(&self) -> PathBuf {
        let Some(home) = self.apps.home.clone() else { return PathBuf::from("/") };
        let config = self.apps.config_home.clone().unwrap_or_else(|| home.join(".config"));
        let pictures = user_dir_in(UserDir::Pictures, &home, &config);
        if pictures != home && pictures.is_dir() { pictures } else { home }
    }

    /// Something happened in the file picker.
    pub(super) fn on_wallpaper_picker(&mut self, message: FilePickerMsg) -> Command<Msg> {
        let Some(browser) = self.wallpaper.picker.as_mut() else { return Command::none() };
        if let FilePickerMsg::Chosen(path) = message {
            self.wallpaper.picker = None;
            return Command::batch([self.choose_wallpaper(path), self.body_focus()]);
        }
        browser.update(message, Msg::WallpaperPicker)
    }

    /// The picker was closed without a choice.
    pub(super) fn close_wallpaper_picker(&mut self) -> Command<Msg> {
        if self.wallpaper.picker.take().is_none() {
            return Command::none();
        }
        self.body_focus()
    }

    /// Tells the Settings screen what the picture is now.
    fn show_wallpaper_row(&mut self) {
        let path = self.wallpaper.path.as_deref();
        let row = WallpaperRow {
            name: path.map(|path| {
                path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
            }),
            ours: path.and_then(|path| wallpapers::ours_at(path, self.apps.data_home.as_deref())),
            problem: match self.wallpaper.shown {
                Shown::Failed(problem) => Some(problem),
                _ => None,
            },
        };
        self.screen.set_wallpaper(row);
    }

    /// Whether a picture lies on the floor in `env`: set, decoded and drawable there.
    pub(super) fn wallpaper_drawn(&self, env: &Env) -> bool {
        self.wallpaper.ready().is_some() && can_draw(env)
    }

    /// Lays the picture over the floor, under what comes after it, when there is one to draw.
    pub(super) fn wallpaper_view(&self, ui: &mut View<'_, Msg>) {
        if !can_draw(ui.env()) {
            return;
        }
        if let Some(data) = self.wallpaper.ready() {
            ui.add(Image::new(data).fit(Fit::Cover)).id(PICTURE).fill();
        }
    }

    /// The file picker's dialog, over the desktop, while it is open.
    pub(super) fn wallpaper_picker_view(&self, ui: &mut View<'_, Msg>) {
        let Some(browser) = &self.wallpaper.picker else { return };
        let dialog = Modal::new()
            .title(t!("wallpaper.pick-title"))
            .width(PICKER_WIDTH)
            .on_close(Msg::CloseWallpaperPicker)
            .action(Button::new(t!("wallpaper.pick-cancel")).on_press(Msg::CloseWallpaperPicker));
        ui.add_with(dialog, |ui| {
            FilePicker::new(browser, Msg::WallpaperPicker)
                .show(ui)
                .id(PICKER)
                .height(Length::Cells(PICKER_HEIGHT))
                .fill_width();
        });
    }
}
