//! `qdesk wallpaper`, run as another program runs it: the real binary, in a home folder of the
//! test's own, with nothing of the person's session reachable from it.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU32, Ordering};

/// A home folder of one test in the temporary folder, taken away when the test ends.
struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-wallpaper-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the home folder is made");
        Self(path)
    }

    /// The configuration folder the command is pointed at.
    fn config(&self) -> PathBuf {
        self.0.join("ayarlar")
    }

    /// qdesk's settings file there.
    fn file(&self) -> PathBuf {
        self.config().join("quvyta").join("desktop.conf")
    }

    /// Runs `qdesk` with `args` in this home, in English.
    fn qdesk(&self, args: &[&str]) -> Output {
        run(&self.0, Some(&self.config()), args)
    }

    /// Runs `qdesk` with `args` in this home, in English, standing in the folder `folder`.
    fn qdesk_in(&self, folder: &Path, args: &[&str]) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_qdesk"));
        command
            .args(args)
            .env_clear()
            .env("HOME", &self.0)
            .env("XDG_CONFIG_HOME", self.config())
            .env("PATH", "/usr/bin:/bin")
            .env("LANG", "en_US.UTF-8")
            .env("BROWSER", "true")
            .current_dir(folder);
        command.output().expect("qdesk runs")
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Runs the real `qdesk` with `home` as the home folder and `config` as `XDG_CONFIG_HOME`.
///
/// The display, the session bus and the runtime folder are taken away as well as the folders:
/// the command has no business with the person's session, and a test that left them in place
/// would reach it if it ever tried.
fn run(home: &Path, config: Option<&Path>, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_qdesk"));
    command
        .args(args)
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "en_US.UTF-8")
        .env("BROWSER", "true");
    if let Some(config) = config {
        command.env("XDG_CONFIG_HOME", config);
    }
    command.output().expect("qdesk runs")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).expect("text")
}

#[test]
fn the_command_writes_the_floor_into_qdesks_own_file_and_prints_it_back() {
    let home = Home::new();
    let set = home.qdesk(&["wallpaper", "--color", "accent", "--pattern", "gradient"]);
    assert_eq!(set.status.code(), Some(0), "{}", text(&set.stderr));
    assert!(set.stdout.is_empty() && set.stderr.is_empty());
    let written = fs::read_to_string(home.file()).expect("the settings file is written");
    assert_eq!(written, "floor-color = \"accent\"\nfloor-style = \"gradient\"\n");
    assert!(!home.0.join(".config").exists(), "nothing lands outside the folder it was pointed at");

    let shown = home.qdesk(&["wallpaper"]);
    assert_eq!(shown.status.code(), Some(0));
    assert_eq!(text(&shown.stdout), "--color accent --pattern gradient\n");

    // Only the key given is written; the other stays as it was.
    let pattern = home.qdesk(&["wallpaper", "--pattern", "both"]);
    assert_eq!(pattern.status.code(), Some(0));
    let written = fs::read_to_string(home.file()).expect("file");
    assert_eq!(written, "floor-color = \"accent\"\nfloor-style = \"gradient-dots\"\n");
}

#[test]
fn a_line_the_person_wrote_stays_as_they_wrote_it() {
    let home = Home::new();
    fs::create_dir_all(home.file().parent().expect("folder")).expect("folder");
    fs::write(home.file(), "theme = \"amber\"\ndock-position = \"sideways\"\nscrollback = 500\n").expect("file");
    let set = home.qdesk(&["wallpaper", "--color", "deep"]);
    assert_eq!(set.status.code(), Some(0), "{}", text(&set.stderr));
    let written = fs::read_to_string(home.file()).expect("file");
    for kept in ["theme = \"amber\"", "dock-position = \"sideways\"", "scrollback = 500", "floor-color = \"deep\""] {
        assert!(written.contains(kept), "{kept} is in:\n{written}");
    }
}

#[test]
fn a_bad_value_is_refused_in_one_line_with_status_2_and_nothing_is_written() {
    let home = Home::new();
    for args in [&["wallpaper", "--color", "red"][..], &["wallpaper", "--pattern"], &["wallpaper", "deniz.png"]] {
        let refused = home.qdesk(args);
        assert_eq!(refused.status.code(), Some(2), "{args:?}");
        assert!(refused.stdout.is_empty());
        let reason = text(&refused.stderr);
        assert_eq!(reason.lines().count(), 1, "{reason}");
        assert!(reason.starts_with("qdesk wallpaper: "), "{reason}");
    }
    assert!(!home.file().exists(), "a refused command writes nothing");
}

#[test]
fn without_a_home_folder_the_command_says_so_and_fails() {
    let home = Home::new();
    let mut command = Command::new(env!("CARGO_BIN_EXE_qdesk"));
    let output = command
        .args(["wallpaper", "--color", "deep"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LANG", "en_US.UTF-8")
        .current_dir(&home.0)
        .output()
        .expect("qdesk runs");
    assert_eq!(output.status.code(), Some(1), "{}", text(&output.stderr));
    assert!(text(&output.stderr).contains("home folder"));
}

/// Writes a small PNG of two colours at `path`.
fn picture(path: &Path) {
    let image =
        image::RgbImage::from_fn(
            8,
            4,
            |_, y| if y < 2 { image::Rgb([200, 40, 60]) } else { image::Rgb([30, 90, 200]) },
        );
    image.save_with_format(path, image::ImageFormat::Png).expect("the picture is written");
}

#[test]
fn a_picture_is_kept_by_its_absolute_path_printed_back_and_taken_away_again() {
    let home = Home::new();
    let folder = home.0.join("Resimler");
    fs::create_dir_all(&folder).expect("folder");
    picture(&folder.join("deniz feneri.png"));
    // Named as a person in that folder names it.
    let set = home.qdesk_in(&folder, &["wallpaper", "deniz feneri.png", "--pattern", "dots"]);
    assert_eq!(set.status.code(), Some(0), "{}", text(&set.stderr));
    assert!(set.stdout.is_empty() && set.stderr.is_empty());
    let written = fs::read_to_string(home.file()).expect("the settings file is written");
    let absolute = folder.join("deniz feneri.png");
    assert!(written.contains(&format!("wallpaper = \"{}\"", absolute.display())), "{written}");
    assert!(written.contains("floor-style = \"dots\""), "{written}");

    let shown = home.qdesk(&["wallpaper"]);
    assert_eq!(text(&shown.stdout), format!("--color theme --pattern dots '{}'\n", absolute.display()));

    let cleared = home.qdesk(&["wallpaper", "--no-picture"]);
    assert_eq!(cleared.status.code(), Some(0), "{}", text(&cleared.stderr));
    let written = fs::read_to_string(home.file()).expect("file");
    assert_eq!(written, "floor-style = \"dots\"\n", "only the picture's line went");
    assert_eq!(text(&home.qdesk(&["wallpaper"]).stdout), "--color theme --pattern dots\n");
}

#[test]
fn a_file_that_cannot_be_the_picture_is_refused_in_one_line_with_status_2_and_nothing_is_written() {
    let home = Home::new();
    let folder = home.0.join("Resimler");
    fs::create_dir_all(&folder).expect("folder");
    fs::write(folder.join("sahte.png"), "only text in a picture's name").expect("file");
    picture(&folder.join("yarim.png"));
    let whole = fs::read(folder.join("yarim.png")).expect("file");
    fs::write(folder.join("yarim.png"), &whole[..whole.len() / 2]).expect("cut in half");
    let cases = [
        (folder.join("yok.png"), "does not exist"),
        (folder.join("sahte.png"), "not a PNG, JPEG, GIF or WebP"),
        (folder.join("yarim.png"), "damaged"),
        (folder.clone(), "could not be read"),
    ];
    for (file, reason) in cases {
        let file = file.display().to_string();
        let refused = home.qdesk(&["wallpaper", "--color", "deep", &file]);
        assert_eq!(refused.status.code(), Some(2), "{file}");
        assert!(refused.stdout.is_empty());
        let said = text(&refused.stderr);
        assert_eq!(said.lines().count(), 1, "{said}");
        assert!(said.starts_with("qdesk wallpaper: ") && said.contains(&file) && said.contains(reason), "{said}");
    }
    assert!(!home.file().exists(), "a refused command writes nothing, not even the colour given with it");
}
