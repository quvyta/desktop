//! The entry folders are watched: an application whose entry file lands in one while the desktop
//! is open appears in the launcher by itself, with nobody restarting anything.
//!
//! The desktop of a screen test watches them as the running desktop does, each wait bounded by
//! its patience, so the test drops a file in and steps the desktop until the entry is there. The
//! folders are the test's own in the temporary folder; nothing of the person's is read.

mod support;

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Instant;

use qdesk::app::Desk;
use qdesk::apps::{Catalog, Environment, Folders, load};
use qdesk::desktop::Desktop;
use qframe::env::{AssetDirs, Env};
use qframe::icons::GlyphMode;
use qframe::prelude::*;

use support::{BUDGET, HARMLESS, MACHINE, MOMENT, OFFSET, PATIENCE};

/// A folder of one test in the temporary folder, taken away with everything in it when the test
/// ends.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-entries-watch-{}-{once}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("the scratch folder is made");
        Self(path)
    }

    /// The person's entry folder of this test's desktop.
    fn entries(&self) -> PathBuf {
        self.0.join("veri").join("quvyta").join("desktop").join("apps")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// A desktop whose data folder is the scratch folder's, with only the built-in entries.
fn desk(scratch: &Scratch) -> Harness<Desk> {
    let dirs = AssetDirs {
        locale_sources: qdesk::locales().iter().map(|(file, text)| ((*file).to_owned(), (*text).to_owned())).collect(),
        keymap_source: Some({
            let (file, text) = qdesk::keymap();
            (file.to_owned(), text.to_owned())
        }),
        ..AssetDirs::default()
    };
    let env = Env::load(&dirs).expect("the built-in files load");
    let apps = Environment {
        home: Some(scratch.0.clone()),
        data_home: Some(scratch.0.join("veri")),
        shell: Some(PathBuf::from(HARMLESS)),
        ..Environment::default()
    };
    let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
    let catalog = Catalog::new(load(&folders, None).entries, |_| true);
    let desktop = Desktop { icons: vec!["terminal".to_owned()], welcome_seen: true, ..Desktop::default() };
    let clock = Box::new(|| MOMENT * 1_000);
    let app = Desk::new(Some(MACHINE.to_owned()), Some(OFFSET), clock)
        .apps(apps)
        .catalog(catalog)
        .desktop(desktop)
        .watch_within(PATIENCE);
    let mut harness = Harness::with_env(app, env, 100, 30);
    harness.set_locale("en").set_glyph_mode(GlyphMode::Unicode).set_reduced_motion(true);
    harness
}

/// Renders until `ready` is happy, or gives up after [`BUDGET`] and says what was on the screen.
fn until(harness: &mut Harness<Desk>, what: &str, ready: impl Fn(&Harness<Desk>) -> bool) {
    let deadline = Instant::now() + BUDGET;
    loop {
        if ready(harness) {
            return;
        }
        assert!(Instant::now() < deadline, "{what} never came:\n{}", harness.screen());
        harness.render();
    }
}

#[test]
fn an_entry_file_dropped_into_the_entry_folder_appears_in_the_open_launcher() {
    let scratch = Scratch::new();
    fs::create_dir_all(scratch.entries()).expect("the entry folder is made");
    let mut harness = desk(&scratch);
    harness.press("space");
    assert!(harness.app().launcher().is_some(), "the launcher is open:\n{}", harness.screen());
    assert!(!harness.screen().contains("Kedicik"), "nothing of the entry yet:\n{}", harness.screen());
    // Another program installs an application while the launcher is open: it writes one file.
    let entry = format!("name = \"Kedicik\"\ncommand = [\"{HARMLESS}\"]\ncategory = \"system\"\n");
    fs::write(scratch.entries().join("kedicik.toml"), entry).expect("the entry is written");
    until(&mut harness, "the new entry in the launcher", |harness| harness.screen().contains("Kedicik"));
}
