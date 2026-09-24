//! Pictures for the floor (design 3.9): which files can be one, the three qdesk brings, and how
//! the settings file names either.
//!
//! A picture of the person's is named in the settings file by its absolute path. qdesk's own
//! pictures are built into the program and decoded from its memory; the file names one as
//! `builtin:` and its name (`builtin:tide`), so nothing is written anywhere to choose one and
//! `qdesk wallpaper` prints a line that sets it again. Version 0.1.7 wrote them into the data
//! folder and named them by that path; such a path is still taken for the picture it held.

use std::fmt;
use std::path::{Path, PathBuf};

use qframe::graphics::Graphics;
use qframe::icons::{KindFamily, file_kind};
use qframe::widgets::{ImageData, ImageError};

/// The most pixels a wallpaper is kept at, width and height: a cell shows two pixels one above
/// the other, so this is a terminal of 960 columns and 270 rows before anything is lost, and at
/// three bytes a pixel it is about 1.5 MB, which a small machine can hold.
pub const MOST: (u32, u32) = (960, 540);

/// The pixels asked for each cell of the floor where the terminal draws the picture itself, with
/// the kitty graphics protocol or sixel: about a cell's size on a usual screen, so every pixel
/// sent shows.
pub const KITTY_CELL: (u32, u32) = (10, 20);

/// The pixels asked for each cell of the floor where the terminal draws the picture itself at the
/// other end of a remote link: twice the half blocks' one by two. Every pixel crosses the network
/// once, and a grainy photograph hardly shrinks when it is packed, so a cell's worth of pixels
/// costs many times what half blocks cost; the terminal stretches these smoothly, which looks no
/// worse than half blocks.
pub const REMOTE_KITTY_CELL: (u32, u32) = (2, 4);

/// The most pixels a picture is kept at for a terminal that draws it itself: a 4K screen.
///
/// The same few pixels a cell over a remote link ([`REMOTE_KITTY_CELL`]) are asked for a sixel
/// terminal too; see [`decode_size`].
pub const SHARPEST: (u32, u32) = (3840, 2160);

/// The prefix the settings file names one of qdesk's own pictures with: `builtin:tide`.
pub const BUILTIN: &str = "builtin:";

/// The size a wallpaper is decoded at, width and height at most, for a floor of `floor` cells
/// drawn with `graphics`, on a terminal that is `remote` or not.
///
/// Half blocks show two pixels a cell and are worked out from [`MOST`] whatever the floor is. A
/// terminal speaking the kitty protocol or sixel shows every pixel it is sent, so on this machine
/// it is given about a cell's worth of pixels for every cell ([`KITTY_CELL`]), never more than
/// [`SHARPEST`]; over a remote link [`REMOTE_KITTY_CELL`] a cell, never more than half blocks
/// would be. A sixel picture is always written at the pixels its cells cover, whatever size it
/// was decoded at, so over a remote link the small picture is stretched into blocks of one
/// colour, which sixel writes as short runs.
#[must_use]
pub fn decode_size(floor: (u16, u16), graphics: Graphics, remote: bool) -> (u32, u32) {
    let per_cell = |cell: (u32, u32), most: (u32, u32)| {
        ((u32::from(floor.0) * cell.0).clamp(1, most.0), (u32::from(floor.1) * cell.1).clamp(1, most.1))
    };
    match graphics {
        Graphics::Kitty | Graphics::Sixel if remote => per_cell(REMOTE_KITTY_CELL, MOST),
        Graphics::Kitty | Graphics::Sixel => per_cell(KITTY_CELL, SHARPEST),
        Graphics::HalfBlock | Graphics::None => MOST,
    }
}

/// One of the pictures qdesk brings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ours {
    /// Its name: the file's, the one the settings file names it by, and the key its label is
    /// looked up by.
    pub name: &'static str,
    /// The PNG file itself.
    pub bytes: &'static [u8],
}

/// qdesk's own pictures, made for it and given away under CC0 (`assets/wallpapers/`), in the
/// order the Settings screen offers them.
pub const OURS: [Ours; 3] = [
    Ours { name: "ember", bytes: include_bytes!("../assets/wallpapers/ember.png") },
    Ours { name: "dusk", bytes: include_bytes!("../assets/wallpapers/dusk.png") },
    Ours { name: "tide", bytes: include_bytes!("../assets/wallpapers/tide.png") },
];

/// A picture for the floor: one of qdesk's own or a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Picture {
    /// One of qdesk's own pictures, decoded from the program's memory.
    Ours(Ours),
    /// The picture in this file, named by its absolute path.
    File(PathBuf),
}

impl Picture {
    /// The picture the settings file's `text` names: `builtin:` and the name of one of qdesk's
    /// own, or an absolute path. Anything else names none.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if let Some(name) = text.strip_prefix(BUILTIN) {
            return OURS.iter().find(|ours| ours.name == name).copied().map(Self::Ours);
        }
        let path = Path::new(text);
        path.is_absolute().then(|| Self::File(path.to_path_buf()))
    }

    /// How the settings file names it, or `None` for a path that is not text, which a text file
    /// cannot hold.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        match self {
            Self::Ours(ours) => Some(format!("{BUILTIN}{}", ours.name)),
            Self::File(path) => path.to_str().map(str::to_owned),
        }
    }

    /// The file, when it is one.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match self {
            Self::Ours(_) => None,
            Self::File(path) => Some(path),
        }
    }

    /// Which of qdesk's own pictures it is: one of them, or the file version 0.1.7 wrote one into
    /// under the data folder `data_home`.
    #[must_use]
    pub fn ours(&self, data_home: Option<&Path>) -> Option<Ours> {
        match self {
            Self::Ours(ours) => Some(*ours),
            Self::File(path) => ours_at(path, data_home).map(|index| OURS[index]),
        }
    }

    /// Decodes it to fit within `size` pixels: one of qdesk's own from memory, so the file 0.1.7
    /// wrote one into under `data_home` is never read, and any other file from the disk.
    ///
    /// # Errors
    ///
    /// The framework's reason when it cannot be decoded.
    pub fn decode(&self, data_home: Option<&Path>, size: (u32, u32)) -> Result<ImageData, ImageError> {
        match self {
            Self::Ours(ours) => ImageData::decode_bytes(ours.bytes, size),
            Self::File(path) => match ours_at(path, data_home) {
                Some(index) => ImageData::decode_bytes(OURS[index].bytes, size),
                None => ImageData::decode_file(path, size),
            },
        }
    }
}

impl fmt::Display for Picture {
    /// How the settings file names it; a path that is not text as near as it can be shown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ours(ours) => write!(f, "{BUILTIN}{}", ours.name),
            Self::File(path) => write!(f, "{}", path.display()),
        }
    }
}

/// Whether the file called `name` can be a wallpaper: a picture by the framework's kinds, of a
/// format its decoder reads ([`ImageData::EXTENSIONS`]). A drawing such as an SVG is a picture too
/// but cannot be decoded, so it is not offered. The name alone decides; decoding says the last
/// word when the picture is chosen.
#[must_use]
pub fn is_picture(name: &str) -> bool {
    file_kind(name, false, false).family() == KindFamily::Image && ImageData::reads(Path::new(name))
}

/// The folder version 0.1.7 wrote qdesk's own pictures into, under the data folder `data_home`.
#[must_use]
pub fn folder(data_home: &Path) -> PathBuf {
    data_home.join("quvyta").join("desktop").join("wallpapers")
}

/// Which of qdesk's own pictures is at `path`, when it is one version 0.1.7 wrote under
/// `data_home`.
#[must_use]
pub fn ours_at(path: &Path, data_home: Option<&Path>) -> Option<usize> {
    let folder = folder(data_home?);
    OURS.iter().position(|ours| path == folder.join(format!("{}.png", ours.name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_pictures_the_framework_decodes_are_offered() {
        for name in ["deniz.png", "Deniz.JPG", "a.jpeg", "k.gif", "w.webp", "iki.nokta.png"] {
            assert!(is_picture(name), "{name}");
        }
        for name in ["çizim.svg", "katman.psd", "notes.txt", "png", "resim.png.txt", "arşiv.tar.gz"] {
            assert!(!is_picture(name), "{name}");
        }
    }

    #[test]
    fn a_kitty_or_sixel_terminal_is_sent_a_cells_worth_of_pixels_for_every_cell_and_half_blocks_keep_the_most() {
        for graphics in [Graphics::Kitty, Graphics::Sixel] {
            assert_eq!(decode_size((100, 29), graphics, false), (1000, 580), "{graphics:?}");
            assert_eq!(decode_size((200, 49), graphics, false), (2000, 980), "{graphics:?}");
            assert_eq!(decode_size((500, 150), graphics, false), SHARPEST, "{graphics:?}: never more than a 4K screen");
        }
        for graphics in [Graphics::HalfBlock, Graphics::None] {
            for remote in [false, true] {
                assert_eq!(decode_size((100, 29), graphics, remote), MOST, "{graphics:?}");
                assert_eq!(decode_size((500, 150), graphics, remote), MOST, "{graphics:?}");
            }
        }
    }

    #[test]
    fn a_kitty_or_sixel_terminal_over_ssh_is_sent_twice_the_half_blocks_and_never_more_than_them() {
        for graphics in [Graphics::Kitty, Graphics::Sixel] {
            assert_eq!(decode_size((100, 29), graphics, true), (200, 116), "{graphics:?}");
            assert_eq!(decode_size((200, 49), graphics, true), (400, 196), "{graphics:?}");
            assert_eq!(decode_size((600, 200), graphics, true), MOST, "{graphics:?}");
        }
    }

    #[test]
    fn our_pictures_are_pngs_of_a_few_kilobytes() {
        for ours in OURS {
            assert!(ours.bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "{}", ours.name);
            assert!(ours.bytes.len() < 24 * 1024, "{} is {} bytes", ours.name, ours.bytes.len());
        }
    }

    #[test]
    fn our_pictures_are_named_builtin_and_a_path_is_a_file() {
        for ours in OURS {
            let text = format!("builtin:{}", ours.name);
            assert_eq!(Picture::parse(&text), Some(Picture::Ours(ours)));
            assert_eq!(Picture::Ours(ours).text(), Some(text));
        }
        let file = Picture::parse("/home/k/deniz.png").expect("a path");
        assert_eq!(file.path(), Some(Path::new("/home/k/deniz.png")));
        assert_eq!(file.text().as_deref(), Some("/home/k/deniz.png"));
        for text in ["builtin:coral", "builtin:", "deniz.png", "", "Builtin:tide"] {
            assert_eq!(Picture::parse(text), None, "{text}");
        }
    }

    #[test]
    fn a_path_version_0_1_7_wrote_one_of_ours_into_is_that_one_and_decodes_without_its_file() {
        let data = std::env::temp_dir().join(format!("qdesk-wallpapers-{}", std::process::id()));
        let old = Picture::File(data.join("quvyta/desktop/wallpapers/dusk.png"));
        assert_eq!(old.ours(Some(&data)), Some(OURS[1]));
        assert_eq!(old.ours(None), None);
        assert_eq!(Picture::File(PathBuf::from("/elsewhere/dusk.png")).ours(Some(&data)), None);
        // The file is not there: the picture comes from memory.
        assert!(!data.exists());
        let decoded = old.decode(Some(&data), MOST).expect("decoded from memory");
        let ours = Picture::Ours(OURS[1]).decode(None, MOST).expect("decoded");
        assert_eq!((decoded.width(), decoded.height()), (ours.width(), ours.height()));
        assert!(ours.width() > 1 && ours.height() > 1);
    }
}
