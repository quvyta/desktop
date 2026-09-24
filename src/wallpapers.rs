//! Pictures for the floor (design 3.9): which files can be one, and the three qdesk brings.
//!
//! A picture is laid over the floor by its path in the settings file. qdesk's own pictures are
//! built into the program and written into its data folder the first time one is chosen, so the
//! setting is a path for them too, and `qdesk wallpaper` prints one that another program can
//! read.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use qframe::icons::{KindFamily, file_kind};

/// The endings of the pictures the framework can decode, and so the only ones offered as a
/// wallpaper: PNG, JPEG, GIF (its first frame) and WebP.
pub const EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "gif", "webp"];

/// The most pixels a wallpaper is kept at, width and height: a cell shows two pixels one above
/// the other, so this is a terminal of 960 columns and 270 rows before anything is lost, and at
/// three bytes a pixel it is about 1.5 MB, which a small machine can hold.
pub const MOST: (u32, u32) = (960, 540);

/// One of the pictures qdesk brings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ours {
    /// Its name: the file's, and the key its label is looked up by.
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

/// Whether the file called `name` can be a wallpaper: a picture by the framework's kinds, of a
/// format it decodes. A drawing such as an SVG is a picture too but cannot be decoded, so it is not
/// offered. The name alone decides; decoding says the last word when the picture is chosen.
#[must_use]
pub fn is_picture(name: &str) -> bool {
    if file_kind(name, false, false).family() != KindFamily::Image {
        return false;
    }
    let Some((_, ending)) = name.rsplit_once('.') else { return false };
    EXTENSIONS.iter().any(|known| known.eq_ignore_ascii_case(ending))
}

/// The folder qdesk's own pictures are written into, under the data folder `data_home`.
#[must_use]
pub fn folder(data_home: &Path) -> PathBuf {
    data_home.join("quvyta").join("desktop").join("wallpapers")
}

/// Which of qdesk's own pictures is at `path`, when it is one written under `data_home`.
#[must_use]
pub fn ours_at(path: &Path, data_home: Option<&Path>) -> Option<usize> {
    let folder = folder(data_home?);
    OURS.iter().position(|ours| path == folder.join(format!("{}.png", ours.name)))
}

/// Writes qdesk's own picture `ours` into its folder under `data_home`, unless it is there as it
/// should be already, and answers where it is.
///
/// # Errors
///
/// When the folder cannot be made or the file cannot be written.
pub fn put(ours: Ours, data_home: &Path) -> io::Result<PathBuf> {
    let folder = folder(data_home);
    let path = folder.join(format!("{}.png", ours.name));
    if fs::read(&path).is_ok_and(|kept| kept == ours.bytes) {
        return Ok(path);
    }
    fs::create_dir_all(&folder)?;
    // Written beside it and moved into place, so a desktop reading it never sees half a file.
    let partial = folder.join(format!(".{}.png.part", ours.name));
    fs::write(&partial, ours.bytes)?;
    fs::rename(&partial, &path)?;
    Ok(path)
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
    fn our_pictures_are_pngs_of_a_few_kilobytes() {
        for ours in OURS {
            assert!(ours.bytes.starts_with(b"\x89PNG\r\n\x1a\n"), "{}", ours.name);
            assert!(ours.bytes.len() < 24 * 1024, "{} is {} bytes", ours.name, ours.bytes.len());
        }
    }

    #[test]
    fn our_picture_is_written_once_into_its_folder_and_known_again_by_its_path() {
        let data = std::env::temp_dir().join(format!("qdesk-wallpapers-{}", std::process::id()));
        let _ = fs::remove_dir_all(&data);
        let path = put(OURS[1], &data).expect("written");
        assert_eq!(path, data.join("quvyta/desktop/wallpapers/dusk.png"));
        assert_eq!(fs::read(&path).expect("file"), OURS[1].bytes);
        assert_eq!(ours_at(&path, Some(&data)), Some(1));
        assert_eq!(ours_at(Path::new("/elsewhere/dusk.png"), Some(&data)), None);
        assert_eq!(ours_at(&path, None), None);
        // A file changed on disk is put right; one that is right is left alone.
        fs::write(&path, b"not a picture").expect("file");
        put(OURS[1], &data).expect("written again");
        assert_eq!(fs::read(&path).expect("file"), OURS[1].bytes);
        let names: Vec<_> =
            fs::read_dir(folder(&data)).expect("folder").map(|entry| entry.expect("entry").file_name()).collect();
        assert_eq!(names.len(), 1, "nothing is left beside it: {names:?}");
        let _ = fs::remove_dir_all(&data);
    }
}
