//! Real terminal programs in a qdesk window: each opens from its icon on the floor, draws what it
//! always draws, answers a key and ends, and the window says it ended.
//!
//! Every terminal program is an application here, and a window runs it through the framework's
//! terminal widget. These tests are the check that common programs really draw in one. They are
//! ignored, so `cargo test` never runs them: they start programs installed on the machine, which a
//! screen test otherwise never does. Run them by hand:
//!
//! ```text
//! cargo test --test acceptance -- --ignored --nocapture --test-threads 1
//! ```
//!
//! A program that is not installed is skipped with a printed line, never failed. Nothing of the
//! person running them is touched: each program gets a home folder, a settings, data, state and
//! cache folder of the test's own under the system's temporary folder, none of the variables that
//! reach their display or their session bus, and the options that keep it from reading their own
//! settings (`-u NONE` for vim, `--clean` for nvim, `-I` for nano, `-f /dev/null` and a socket of
//! its own for tmux). What a program is shown — a file, a folder, a repository — is made up by the
//! test in that folder.

mod support;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use qdesk::app::Desk;
use qdesk::apps::{Catalog, Declared, Entry, Folders, Launch, Source, load, parse_entry};
use qdesk::desktop::Desktop;
use qdesk::wm::{Run, Window};
use qframe::prelude::*;

use support::{BUDGET, HOME, harness_with};

/// The screen these tests draw on. A window opens two thirds of the floor wide and high, so its
/// program gets about 90 columns and 28 rows, a size every one of these programs is made for.
const SCREEN: (u16, u16) = (136, 44);

/// How long a key that should end a program is given before the next one is tried.
const AFTER_A_KEY: Duration = Duration::from_secs(10);

/// The folders of one program's run under the system's temporary folder, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let once = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("qdesk-accept-{}-{name}-{once}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        for folder in ["home", "config", "data", "state", "cache", "work"] {
            std::fs::create_dir_all(path.join(folder)).expect("a scratch folder");
        }
        Self(path)
    }

    /// The folder the program starts in and finds what the test made for it.
    fn work(&self) -> PathBuf {
        self.0.join("work")
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Where `program` is on the `PATH` of the tests, `None` when this machine does not have it.
fn installed(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .filter(|folder| !folder.as_os_str().is_empty())
        .map(|folder| folder.join(program))
        .find(|candidate| candidate.is_file())
}

/// An entry that runs `words`, started in the scratch's work folder with only the scratch's
/// folders for a home, and none of the person's session.
fn entry(scratch: &Scratch, id: &str, words: &[String]) -> Entry {
    let command = words
        .iter()
        .map(|word| {
            assert!(!word.contains('"') && !word.contains('\\'), "a test command is written plainly: {word}");
            format!("\"{word}\"")
        })
        .collect::<Vec<String>>()
        .join(", ");
    let text = format!("name = \"Kabul\"\ncommand = [{command}]\ncategory = \"system\"\n");
    let file = format!("{HOME}/.local/share/quvyta/desktop/apps/{id}.toml");
    let (declared, diagnostics) =
        parse_entry(id, Path::new(&file), text.as_bytes(), Source::User, Some(Path::new(HOME)));
    assert!(diagnostics.is_empty(), "{id}: {diagnostics:?}");
    let Some(Declared::Entry(entry)) = declared else { panic!("{id} declares no entry") };
    let mut entry = *entry;
    entry.folder = Some(scratch.work());
    for (name, folder) in [
        ("HOME", "home"),
        ("XDG_CONFIG_HOME", "config"),
        ("XDG_DATA_HOME", "data"),
        ("XDG_STATE_HOME", "state"),
        ("XDG_CACHE_HOME", "cache"),
    ] {
        entry.env.push((name.to_owned(), scratch.0.join(folder).display().to_string()));
    }
    entry.env.push(("SHELL".to_owned(), "/bin/sh".to_owned()));
    // The same words on every machine, so what a test looks for is there whatever language the
    // person's machine speaks.
    entry.env.push(("LANG".to_owned(), "C.UTF-8".to_owned()));
    entry.unset.extend(["LC_ALL", "LC_MESSAGES", "LANGUAGE"].map(str::to_owned));
    entry
}

/// A desktop whose floor holds `entry` alone, and nothing else of this machine.
fn desk(entry: Entry) -> Harness<Desk> {
    let icons = vec![entry.id.clone()];
    let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
    let mut all = load(&folders, Some(Path::new(HOME))).entries;
    all.push(entry);
    let catalog = Catalog::new(support::sealed(all), |entry| !matches!(entry.launch, Launch::Open(_)));
    let desktop =
        Desktop { icons, recents: Vec::new(), welcome_seen: true, resize_hint_seen: true, ..Desktop::default() };
    harness_with(catalog, desktop, SCREEN.0, SCREEN.1)
}

fn front(harness: &Harness<Desk>) -> &Window {
    harness.app().windows().front().expect("a window on the desktop")
}

/// The rows of the window in front, from its left edge on: the pillar that marks the window in
/// front, then what the program drew.
fn window_rows(harness: &Harness<Desk>) -> Vec<String> {
    let rect = front(harness).rect();
    let (left, top) = (usize::try_from(rect.x).unwrap_or(0), usize::try_from(rect.y).unwrap_or(0));
    let height = usize::from(rect.height);
    harness.screen().lines().skip(top).take(height).map(|row| row.chars().skip(left).collect()).collect()
}

/// Renders until `ready` is happy, and answers whether it was before `bound` ran out.
fn within(harness: &mut Harness<Desk>, bound: Duration, ready: impl Fn(&Harness<Desk>) -> bool) -> bool {
    let deadline = Instant::now() + bound;
    loop {
        if ready(harness) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        harness.render();
    }
}

fn ended(harness: &Harness<Desk>) -> bool {
    matches!(front(harness).run(), Some(Run::Ended { .. }))
}

/// One key or typed text that a program answers.
#[derive(Clone, Copy)]
enum Key {
    /// A chord, as the harness names one (`"ctrl+x"`, `"f10"`).
    Press(&'static str),
    /// Text typed one character at a time.
    Type(&'static str),
    /// Text typed one character at a time, then enter.
    Line(&'static str),
}

/// Opens `entry` from its icon, waits for `ready` to find what the program always draws, then
/// sends `keys` one at a time until the window says the program ended.
///
/// A later key is sent only when the one before did not end the program within [`AFTER_A_KEY`]:
/// some programs ask whether they should quit, or put a note of their own in front the first time
/// they run. Every wait is bounded, so a program that never draws or never ends fails the test
/// with the screen it left, rather than hanging it.
fn accept(program: &str, entry: Entry, what: &str, ready: impl Fn(&Harness<Desk>) -> bool, keys: &[Key]) {
    let mut harness = desk(entry);
    // The first icon of the floor; two clicks open it, as on any desktop.
    harness.click(3, 1);
    harness.click(3, 1);
    assert_eq!(harness.app().windows().len(), 1, "{program}: the icon opened a window:\n{}", harness.screen());
    let rect = front(&harness).rect();
    let opened = Instant::now();
    assert!(
        within(&mut harness, BUDGET, &ready),
        "{program}: {what} never came in a window of {}x{}:\n{}",
        rect.width,
        rect.height,
        harness.screen()
    );
    // The whole screen goes out with the line, so whoever runs these by hand sees how it was drawn.
    println!(
        "{program}: {what} drawn in a window of {}x{} after {} ms:\n{}",
        rect.width,
        rect.height,
        opened.elapsed().as_millis(),
        harness.screen()
    );
    for key in keys {
        match key {
            Key::Press(chord) => harness.press(chord),
            Key::Type(text) => harness.type_text(text),
            Key::Line(text) => harness.type_text(text).press("enter"),
        };
        if within(&mut harness, AFTER_A_KEY, ended) {
            break;
        }
    }
    assert!(within(&mut harness, BUDGET, ended), "{program}: it never ended:\n{}", harness.screen());
    let screen = harness.screen();
    assert!(screen.contains("The program ended"), "{program}: the window says the program ended:\n{screen}");
    println!("{program}: ended, {:?}", front(&harness).run());
}

/// Whether `program` is here; says so and answers `None` when it is not.
fn here(program: &str) -> Option<PathBuf> {
    let found = installed(program);
    if found.is_none() {
        println!("{program}: not installed on this machine, skipped");
    }
    found
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|word| (*word).to_owned()).collect()
}

/// A repository in `folder` with three made-up commits by a made-up author, made without reading
/// anything of the person's git settings.
fn repository(folder: &Path) -> [&'static str; 3] {
    let subjects = ["kabul ilk adim", "kabul ikinci adim", "kabul son adim"];
    let git = |arguments: &[&str]| {
        let status = std::process::Command::new("git")
            .args(arguments)
            .current_dir(folder)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_AUTHOR_NAME", "Deneme Kisi")
            .env("GIT_AUTHOR_EMAIL", "deneme@example.invalid")
            .env("GIT_COMMITTER_NAME", "Deneme Kisi")
            .env("GIT_COMMITTER_EMAIL", "deneme@example.invalid")
            .status()
            .expect("git runs");
        assert!(status.success(), "git {arguments:?}");
    };
    git(&["init", "-q", "-b", "ana"]);
    for subject in subjects {
        git(&["commit", "-q", "--allow-empty", "-m", subject]);
    }
    subjects
}

/// The variables a program that reads a repository is given, so the person's git settings stay out.
fn git_env(entry: &mut Entry) {
    entry.env.push(("GIT_CONFIG_GLOBAL".to_owned(), "/dev/null".to_owned()));
    entry.env.push(("GIT_CONFIG_NOSYSTEM".to_owned(), "1".to_owned()));
}

/// A made-up text file of three lines in the work folder.
fn letter(scratch: &Scratch) -> PathBuf {
    let file = scratch.work().join("mektup.txt");
    std::fs::write(&file, "birinci satir\nikinci satir\nucuncu satir\n").expect("the made-up file");
    file
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn htop_draws_its_meters_and_its_process_list_and_ends_on_q() {
    let Some(htop) = here("htop") else { return };
    let scratch = Scratch::new("htop");
    let entry = entry(&scratch, "htop", &[htop.display().to_string()]);
    let ready = |harness: &Harness<Desk>| {
        let screen = harness.screen();
        screen.contains("PID") && screen.contains("CPU%") && screen.contains("Mem[")
    };
    accept("htop", entry, "the PID and CPU% columns and the Mem meter", ready, &[Key::Press("q")]);
}

/// The rows of a vi that shows `file`: the `~` column below its three lines and its name.
fn vi_ready(file: &'static str) -> impl Fn(&Harness<Desk>) -> bool {
    move |harness| {
        let tildes = window_rows(harness)
            .iter()
            .filter(|row| row.trim_start_matches(|c: char| c == '▌' || c.is_whitespace()).starts_with('~'))
            .count();
        tildes >= 5 && harness.screen().contains(file)
    }
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn vim_draws_the_tilde_column_and_the_file_name_and_ends_on_colon_q() {
    let Some(vim) = here("vim") else { return };
    let scratch = Scratch::new("vim");
    let file = letter(&scratch);
    let entry = entry(
        &scratch,
        "vim",
        &[
            vim.display().to_string(),
            "-u".into(),
            "NONE".into(),
            "-i".into(),
            "NONE".into(),
            "-n".into(),
            file.display().to_string(),
        ],
    );
    accept("vim", entry, "the ~ column and the file name", vi_ready("mektup.txt"), &[Key::Line(":q")]);
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn nvim_draws_the_tilde_column_and_the_file_name_and_ends_on_colon_q() {
    let Some(nvim) = here("nvim") else { return };
    let scratch = Scratch::new("nvim");
    let file = letter(&scratch);
    let entry = entry(
        &scratch,
        "nvim",
        &[
            nvim.display().to_string(),
            "--clean".into(),
            "-i".into(),
            "NONE".into(),
            "-n".into(),
            file.display().to_string(),
        ],
    );
    // nvim asks the terminal for its background colour and its cursor at start; a terminal that
    // does not answer leaves an E1568 line on the screen, so a clean screen is part of ready.
    let ready = vi_ready("mektup.txt");
    let answered = move |harness: &Harness<Desk>| ready(harness) && !harness.screen().contains("E15");
    accept("nvim", entry, "the ~ column and the file name, and no error line", answered, &[Key::Line(":q")]);
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn nano_draws_its_title_and_its_key_lines_and_ends_on_ctrl_x() {
    let Some(nano) = here("nano") else { return };
    let scratch = Scratch::new("nano");
    let file = letter(&scratch);
    let entry = entry(&scratch, "nano", &[nano.display().to_string(), "-I".into(), file.display().to_string()]);
    let ready = |harness: &Harness<Desk>| {
        let screen = harness.screen();
        screen.contains("GNU nano") && screen.contains("^X") && screen.contains("birinci satir")
    };
    accept("nano", entry, "the GNU nano title, the ^X key line and the file", ready, &[Key::Press("ctrl+x")]);
}

/// Ends the tmux server of a socket when the test is done with it, however the test ended, so no
/// server of the test's outlives it.
struct Server(PathBuf, PathBuf);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::process::Command::new(&self.0)
            .arg("-S")
            .arg(&self.1)
            .arg("kill-server")
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn tmux_on_a_socket_of_its_own_draws_its_status_line_and_ends_with_its_shell() {
    let Some(tmux) = here("tmux") else { return };
    let scratch = Scratch::new("tmux");
    let socket = scratch.0.join("soket");
    let _server = Server(tmux.clone(), socket.clone());
    let entry = entry(
        &scratch,
        "tmux",
        &words(&[
            &tmux.display().to_string(),
            "-S",
            &socket.display().to_string(),
            "-f",
            "/dev/null",
            "new-session",
            "-s",
            "kabul",
        ]),
    );
    let ready = |harness: &Harness<Desk>| harness.screen().contains("[kabul]");
    accept("tmux", entry, "the status line of the session kabul", ready, &[Key::Line("exit")]);
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn git_log_shows_the_commits_of_a_made_up_repository_and_ends() {
    let Some(git) = here("git") else { return };
    let scratch = Scratch::new("git");
    let subjects = repository(&scratch.work());
    // No pager: whether this machine has one is not what is checked, and a program that prints and
    // ends is a case of its own: the window keeps what it printed above the line that it ended.
    let mut entry = entry(&scratch, "git", &words(&[&git.display().to_string(), "--no-pager", "log", "--oneline"]));
    git_env(&mut entry);
    let ready = move |harness: &Harness<Desk>| subjects.iter().all(|subject| harness.screen().contains(subject));
    accept("git", entry, "the three made-up commits", ready, &[]);
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn btop_draws_its_boxes_and_ends_on_q() {
    let Some(btop) = here("btop") else { return };
    let scratch = Scratch::new("btop");
    let entry = entry(&scratch, "btop", &[btop.display().to_string()]);
    let ready = |harness: &Harness<Desk>| {
        let screen = harness.screen();
        screen.contains("cpu") && screen.contains("mem") && screen.contains("proc")
    };
    accept("btop", entry, "the cpu, mem and proc boxes", ready, &[Key::Press("q"), Key::Press("q")]);
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn lazygit_draws_the_commits_of_a_made_up_repository_and_ends_on_q() {
    let Some(lazygit) = here("lazygit") else { return };
    let scratch = Scratch::new("lazygit");
    let subjects = repository(&scratch.work());
    let mut entry = entry(&scratch, "lazygit", &[lazygit.display().to_string()]);
    git_env(&mut entry);
    let ready = move |harness: &Harness<Desk>| {
        let screen = harness.screen();
        screen.contains("Commits") && screen.contains(subjects[2])
    };
    // The first run may put a note of its own in front; a second q goes past it.
    accept(
        "lazygit",
        entry,
        "the Commits panel and the last commit",
        ready,
        &[Key::Press("q"), Key::Press("esc"), Key::Press("q")],
    );
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn mc_draws_its_two_panels_and_its_key_bar_and_ends_on_f10() {
    let Some(mc) = here("mc") else { return };
    let scratch = Scratch::new("mc");
    letter(&scratch);
    let entry = entry(&scratch, "mc", &words(&[&mc.display().to_string(), "-u"]));
    let ready = |harness: &Harness<Desk>| {
        let screen = harness.screen();
        screen.contains("mektup.txt") && screen.contains("Quit") && screen.contains("Left")
    };
    // mc may ask whether to quit; enter answers yes.
    accept("mc", entry, "the file, the menu bar and the key bar", ready, &[Key::Press("f10"), Key::Press("enter")]);
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn ranger_draws_the_folder_and_ends_on_q() {
    let Some(ranger) = here("ranger") else { return };
    let scratch = Scratch::new("ranger");
    letter(&scratch);
    let entry = entry(&scratch, "ranger", &words(&[&ranger.display().to_string(), "--clean"]));
    let ready = |harness: &Harness<Desk>| harness.screen().contains("mektup.txt");
    accept("ranger", entry, "the made-up file", ready, &[Key::Press("q")]);
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn yazi_draws_the_folder_and_ends_on_q() {
    let Some(yazi) = here("yazi") else { return };
    let scratch = Scratch::new("yazi");
    letter(&scratch);
    let entry = entry(&scratch, "yazi", &[yazi.display().to_string()]);
    let ready = |harness: &Harness<Desk>| harness.screen().contains("mektup.txt");
    accept("yazi", entry, "the made-up file", ready, &[Key::Press("q"), Key::Press("enter")]);
}

#[test]
#[ignore = "runs real programs; run by hand: cargo test --test acceptance -- --ignored"]
fn w3m_draws_a_made_up_page_and_ends_on_shift_q() {
    let Some(w3m) = here("w3m") else { return };
    let scratch = Scratch::new("w3m");
    let page = scratch.work().join("sayfa.html");
    std::fs::write(&page, "<html><body><h1>Kabul sayfasi</h1><p>Bir paragraf.</p></body></html>\n").expect("the page");
    let entry = entry(&scratch, "w3m", &[w3m.display().to_string(), page.display().to_string()]);
    let ready = |harness: &Harness<Desk>| harness.screen().contains("Kabul sayfasi");
    accept("w3m", entry, "the page's heading", ready, &[Key::Type("Q"), Key::Type("y")]);
}
