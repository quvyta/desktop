//! The picture over the floor (design 3.9).
//!
//! The settings name a picture, one of qdesk's own or a file; the desktop decodes it off the
//! drawing thread with the framework's decoder, shrunk to the size [`wallpapers::decode_size`]
//! gives for the floor, the way the terminal shows pictures and whether it is reached over a
//! network, and decodes it again when either changes. The framework's [`Image`] lays it over
//! the whole floor with [`Fit::Cover`]. The image works its cells out once for a size and keeps
//! them, so a frame that repaints the floor only copies them, and the terminal is sent only the
//! cells that changed.
//!
//! Until the picture is decoded, where it cannot be decoded, and where the terminal draws no
//! pictures ([`Graphics::can_draw`]: sixteen colours, ASCII glyphs), the floor is its colour and pattern as if no picture were
//! set. A picture a person chooses — from a file's menu, the Settings screen or its file picker —
//! is decoded first and written into the settings only when it decodes, so a broken file never
//! becomes the setting. A picture the settings file names is the person's: when it cannot be
//! shown the desktop says why and leaves the file as it is.

use std::path::{Path, PathBuf};

use qframe::env::Env;
use qframe::graphics::Graphics;
use qframe::prelude::*;
use qframe::storage::{UserDir, user_dir_in};
use qframe::widgets::{
    FileBrowser, FilePicker, FilePickerMsg, Fit, Image, ImageData, ImageError, Modal, PickMode, Toast,
};

use super::{Desk, Msg};
use crate::inbox::Notice;
use crate::settings::{self, WallpaperRow};
use crate::wallpapers::{self, OURS, Picture};

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
    picture: Option<Picture>,
    /// What became of it.
    shown: Shown,
    /// Which decoding is the current one; the answer of an older one is dropped.
    run: u64,
    /// The file picker, while it is open.
    picker: Option<FileBrowser>,
    /// The floor's columns and rows, as the last resize said.
    floor: (u16, u16),
    /// How the terminal shows pictures, once the framework has said it; half blocks until then.
    graphics: Option<Graphics>,
    /// The size the last decoding was asked for.
    asked: Option<(u32, u32)>,
    /// The picture a person chose that is being decoded, not yet in force.
    choosing: Option<Picture>,
}

impl Wallpaper {
    /// The picture the settings name.
    #[must_use]
    pub fn picture(&self) -> Option<&Picture> {
        self.picture.as_ref()
    }

    /// The file the settings name as the picture, when they name a file.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        self.picture.as_ref().and_then(Picture::path)
    }

    /// The size, in pixels at most, the last decoding was asked for.
    #[must_use]
    pub fn asked(&self) -> Option<(u32, u32)> {
        self.asked
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

impl Desk {
    /// Takes the picture the settings name as the one in force, without decoding it yet: what a
    /// desktop is built with, before it starts.
    pub(super) fn name_wallpaper(&mut self, picture: Option<Picture>) {
        self.wallpaper.shown = if picture.is_some() { Shown::Decoding } else { Shown::Nothing };
        self.wallpaper.picture = picture;
        self.show_wallpaper_row();
    }

    /// Decodes the picture the settings name, when they name one: the first thing the desktop
    /// does about it once it starts.
    pub(super) fn start_wallpaper(&mut self) -> Command<Msg> {
        match self.wallpaper.picture.clone() {
            Some(picture) => self.decode_wallpaper(picture, Purpose::Settings),
            None => Command::none(),
        }
    }

    /// Takes `picture` as the one the settings file now names, when it is not the one in force:
    /// the file was changed by another program or by hand.
    pub(super) fn follow_wallpaper(&mut self, picture: Option<Picture>) -> Command<Msg> {
        if picture == self.wallpaper.picture {
            return Command::none();
        }
        self.name_wallpaper(picture);
        // A decoding still on its way is for the old picture.
        self.wallpaper.run += 1;
        self.start_wallpaper()
    }

    /// The size a picture is decoded at for this floor, this terminal and this link.
    fn wallpaper_size(&self) -> (u32, u32) {
        let graphics = self.wallpaper.graphics.unwrap_or(Graphics::HalfBlock);
        wallpapers::decode_size(self.wallpaper.floor, graphics, self.remote)
    }

    /// The screen is now `size`: the picture is decoded again when the floor asks for another size
    /// of it, which only a terminal drawing pictures itself does.
    pub(super) fn wallpaper_resized(&mut self, size: Size) -> Command<Msg> {
        // The dock takes one row of the screen.
        self.wallpaper.floor = (size.width, size.height.saturating_sub(1));
        self.decode_wallpaper_again()
    }

    /// The terminal now draws pictures with `graphics`, as the framework tells before the first
    /// frame and whenever it changes: the picture is decoded again when that asks for another size
    /// of it.
    pub(super) fn wallpaper_graphics(&mut self, graphics: Graphics) -> Command<Msg> {
        self.wallpaper.graphics = Some(graphics);
        self.decode_wallpaper_again()
    }

    /// Decodes the picture again when the floor and the terminal now ask for another size of it
    /// than the last decoding was asked for. Before the first decoding nothing is done: the
    /// desktop's start decodes it at the size due then.
    fn decode_wallpaper_again(&mut self) -> Command<Msg> {
        let wanted = self.wallpaper_size();
        if self.wallpaper.asked.is_none_or(|asked| asked == wanted) {
            return Command::none();
        }
        // A choice on its way is decoded again at the new size, else the picture in force.
        match (self.wallpaper.choosing.clone(), self.wallpaper.picture.clone()) {
            (Some(chosen), _) => self.decode_wallpaper(chosen, Purpose::Chosen),
            (None, Some(picture)) => self.decode_wallpaper(picture, Purpose::Settings),
            (None, None) => Command::none(),
        }
    }

    /// Decodes `picture` off the drawing thread, for `purpose`.
    fn decode_wallpaper(&mut self, picture: Picture, purpose: Purpose) -> Command<Msg> {
        self.wallpaper.run += 1;
        let run = self.wallpaper.run;
        let size = self.wallpaper_size();
        self.wallpaper.asked = Some(size);
        self.wallpaper.choosing = (purpose == Purpose::Chosen).then(|| picture.clone());
        let data = self.apps.data_home.clone();
        Command::perform(move || {
            let decoded = picture.decode(data.as_deref(), size);
            Msg::WallpaperDecoded(run, picture, purpose, decoded)
        })
    }

    /// A person chose `picture` as the wallpaper: a file from a file's menu or the file picker, or
    /// one of qdesk's own pictures. It is decoded first.
    pub(super) fn choose_wallpaper(&mut self, picture: Picture) -> Command<Msg> {
        self.decode_wallpaper(picture, Purpose::Chosen)
    }

    /// A decoding of run `run` ended.
    pub(super) fn on_wallpaper_decoded(
        &mut self,
        run: u64,
        picture: Picture,
        purpose: Purpose,
        decoded: Result<ImageData, ImageError>,
    ) -> Command<Msg> {
        if run != self.wallpaper.run {
            return Command::none();
        }
        self.wallpaper.choosing = None;
        match (decoded, purpose) {
            (Ok(data), Purpose::Settings) => {
                self.wallpaper.shown = Shown::Ready(data);
                self.show_wallpaper_row();
                Command::none()
            }
            (Ok(data), Purpose::Chosen) => {
                if !settings::set_wallpaper(&mut self.stored, Some(&picture)) {
                    let reason = t!("wallpaper.not-text");
                    return Self::refused_wallpaper(&picture, &reason);
                }
                self.wallpaper.picture = Some(picture);
                self.wallpaper.shown = Shown::Ready(data);
                self.show_wallpaper_row();
                self.store()
            }
            (Err(problem), Purpose::Settings) => {
                self.wallpaper.shown = Shown::Failed(problem);
                self.show_wallpaper_row();
                let heading = t!("wallpaper.unshown");
                let body = format!("{picture}: {problem}");
                self.inbox.add(Notice::desktop(heading.clone(), body.clone()));
                Command::toast(Toast::warning(heading).body(body))
            }
            // What was in force stays: the picture before it, or none.
            (Err(problem), Purpose::Chosen) => Self::refused_wallpaper(&picture, &problem.to_string()),
        }
    }

    /// Says that `picture`, which a person chose, is not the wallpaper, and why.
    fn refused_wallpaper(picture: &Picture, reason: &str) -> Command<Msg> {
        let name = picture_name(picture);
        Command::toast(Toast::warning(t!("wallpaper.refused", name = name.as_str())).body(reason.to_owned()))
    }

    /// Takes the picture away: the floor is its colour and pattern again.
    pub(super) fn remove_wallpaper(&mut self) -> Command<Msg> {
        self.wallpaper.run += 1;
        self.wallpaper.choosing = None;
        self.name_wallpaper(None);
        settings::set_wallpaper(&mut self.stored, None);
        self.store()
    }

    /// What the Settings screen asks about the picture.
    pub(super) fn on_wallpaper_asked(&mut self, asked: settings::Wallpaper) -> Command<Msg> {
        match asked {
            settings::Wallpaper::Choose => self.open_wallpaper_picker(),
            settings::Wallpaper::Remove => self.remove_wallpaper(),
            // Decoded from the program's own memory: nothing is written to choose one.
            settings::Wallpaper::Ours(index) => match OURS.get(index) {
                Some(ours) => self.choose_wallpaper(Picture::Ours(*ours)),
                None => Command::none(),
            },
        }
    }

    /// Opens the file picker on the person's Pictures folder, else their home folder, showing only
    /// the pictures that can be decoded.
    fn open_wallpaper_picker(&mut self) -> Command<Msg> {
        let start = self.pictures_folder();
        let mut browser =
            FileBrowser::new(start.clone(), PickMode::Files).extensions(ImageData::EXTENSIONS.iter().copied());
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
            return Command::batch([self.choose_wallpaper(Picture::File(path)), self.body_focus()]);
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
        let picture = self.wallpaper.picture.as_ref();
        let ours = picture.and_then(|picture| picture.ours(self.apps.data_home.as_deref()));
        let row = WallpaperRow {
            // A file 0.1.7 wrote one of qdesk's own into is called by that one's name.
            name: picture.map(|picture| match ours {
                Some(ours) => picture_name(&Picture::Ours(ours)),
                None => picture_name(picture),
            }),
            ours: ours.and_then(|ours| OURS.iter().position(|known| *known == ours)),
            problem: match self.wallpaper.shown {
                Shown::Failed(problem) => Some(problem),
                _ => None,
            },
        };
        self.screen.set_wallpaper(row);
    }

    /// Whether a picture lies on the floor in `env`: set, decoded and drawable there.
    ///
    /// The framework's image answers where it cannot draw by saying so in the middle of its area,
    /// which on a floor would be a sentence across the desktop, so the floor asks the image's own
    /// question first ([`Graphics::can_draw`]) and stays its colour.
    pub(super) fn wallpaper_drawn(&self, env: &Env) -> bool {
        self.wallpaper.ready().is_some() && env.graphics().can_draw()
    }

    /// Lays the picture over the floor, under what comes after it, when there is one to draw.
    pub(super) fn wallpaper_view(&self, ui: &mut View<'_, Msg>) {
        if !ui.env().graphics().can_draw() {
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

/// What a person calls `picture`: the label of one of qdesk's own, in their language, or the
/// file's name.
fn picture_name(picture: &Picture) -> String {
    match picture {
        Picture::Ours(ours) => t!(&format!("settings.wallpaper-{}", ours.name)),
        Picture::File(path) => {
            path.file_name().map_or_else(|| path.display().to_string(), |name| name.to_string_lossy().into_owned())
        }
    }
}
