//! Where entries come from, and which one wins when two places have the same id.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use super::builtin;
use super::desktop_file::{DesktopFile, parse_desktop_file};
use super::diagnostic::{Diagnostic, DiagnosticKind};
use super::entry::{Declared, Entry, Launch, Source, parse_entry};
use super::program::{find_program, is_executable};

/// The parts of the process environment the applications depend on. Tests build one by hand, so
/// they never read the machine they run on.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Environment {
    /// The home folder, from `HOME`, when it is an absolute path.
    pub home: Option<PathBuf>,
    /// The user's data folder: `XDG_DATA_HOME`, else `~/.local/share`.
    pub data_home: Option<PathBuf>,
    /// The system's data folders, highest priority first: `XDG_DATA_DIRS`, else
    /// `/usr/local/share` and `/usr/share`.
    pub data_dirs: Vec<PathBuf>,
    /// The `PATH` programs are looked for in.
    pub path: Option<OsString>,
    /// The user's shell, from `SHELL`.
    pub shell: Option<PathBuf>,
}

impl Environment {
    /// The environment of this process.
    #[must_use]
    pub fn from_process() -> Self {
        Self::from_vars(|name| std::env::var_os(name))
    }

    /// The environment described by variables, given as a lookup by name.
    ///
    /// As the XDG specification asks, a folder that is not an absolute path is ignored: it would
    /// depend on the folder qdesk happened to start in.
    #[must_use]
    pub fn from_vars(lookup: impl Fn(&str) -> Option<OsString>) -> Self {
        let absolute = |value: OsString| Some(PathBuf::from(value)).filter(|path| path.is_absolute());
        let home = lookup("HOME").and_then(absolute);
        let data_home = lookup("XDG_DATA_HOME")
            .and_then(absolute)
            .or_else(|| home.as_ref().map(|home| home.join(".local").join("share")));
        let mut data_dirs: Vec<PathBuf> = lookup("XDG_DATA_DIRS")
            .map(|dirs| std::env::split_paths(&dirs).filter(|dir| dir.is_absolute()).collect())
            .unwrap_or_default();
        if data_dirs.is_empty() {
            data_dirs = vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")];
        }
        Self {
            home,
            data_home,
            data_dirs,
            path: lookup("PATH").filter(|path| !path.is_empty()),
            shell: lookup("SHELL").filter(|shell| !shell.is_empty()).map(PathBuf::from),
        }
    }

    /// The folders entries are read from.
    #[must_use]
    pub fn folders(&self) -> Folders {
        let entries = |data: &Path| data.join("quvyta").join("desktop").join("apps");
        Folders {
            user: self.data_home.as_deref().map(entries),
            system: self.data_dirs.iter().map(|dir| entries(dir)).collect(),
            desktop_files: self.data_home.iter().chain(&self.data_dirs).map(|dir| dir.join("applications")).collect(),
        }
    }

    /// Reads every entry this environment names.
    #[must_use]
    pub fn load(&self) -> Loaded {
        load(&self.folders(), self.home.as_deref())
    }

    /// The shell the Terminal screen starts: `SHELL`, else `/bin/sh`.
    #[must_use]
    pub fn terminal_shell(&self) -> PathBuf {
        self.shell.clone().unwrap_or_else(|| PathBuf::from("/bin/sh"))
    }

    /// Whether `entry` can be opened on this machine; see [`Environment::is_installed_with`].
    #[must_use]
    pub fn is_installed(&self, entry: &Entry) -> bool {
        self.is_installed_with(entry, is_executable)
    }

    /// Whether `entry` can be opened, with `probe` saying whether a path is a program.
    ///
    /// A command is installed when its program is found, and for a `.desktop` file also its
    /// `TryExec`. Screens of qdesk are always installed, and so is a folder or file to open: a
    /// missing one is not something to install.
    #[must_use]
    pub fn is_installed_with(&self, entry: &Entry, probe: impl Fn(&Path) -> bool) -> bool {
        let found = |program: &str| find_program(program, self.path.as_deref(), &probe).is_some();
        match &entry.launch {
            Launch::Command(_) => {
                entry.launch.program().is_some_and(found) && entry.try_exec.as_deref().is_none_or(found)
            }
            Launch::Open(_) | Launch::Screen(_) => true,
        }
    }
}

/// The folders entries are read from, and the ones to watch for changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
    /// The user's entry folder, `~/.local/share/quvyta/desktop/apps`.
    pub user: Option<PathBuf>,
    /// The system's entry folders, highest priority first.
    pub system: Vec<PathBuf>,
    /// The `applications` folders `.desktop` files are read from, highest priority first.
    pub desktop_files: Vec<PathBuf>,
}

impl Folders {
    /// Every folder whose files make up the entries: a file added, changed or removed in any of
    /// them means loading again. A folder may not exist yet.
    #[must_use]
    pub fn watched(&self) -> Vec<PathBuf> {
        self.user.iter().chain(&self.system).chain(&self.desktop_files).cloned().collect()
    }
}

/// The entries that won, and every problem found on the way.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Loaded {
    /// One entry per id, the one from the highest source. Hidden ids are left out.
    pub entries: Vec<Entry>,
    /// Problems in the order the files were read. Broken files are left out of `entries`.
    pub diagnostics: Vec<Diagnostic>,
}

/// Reads entries from `folders` and the built-in ones, keeping one entry per id.
///
/// Sources in priority order: the user's folder, the system's folders, the built-in entries,
/// the `.desktop` files. The first source that declares an id decides it: its entry is shown,
/// or with `hidden = true` nothing is. A broken file decides nothing, so an entry below it still
/// shows while its diagnostic says why the one above did not load. Folders that do not exist are
/// normal and say nothing. `home` replaces a leading `~` in paths.
#[must_use]
pub fn load(folders: &Folders, home: Option<&Path>) -> Loaded {
    let mut loaded = Loaded::default();
    let mut claimed: HashSet<String> = HashSet::new();
    let mut decide = |loaded: &mut Loaded, id: &str, entry: Option<Entry>| {
        if claimed.insert(id.to_owned())
            && let Some(entry) = entry
        {
            loaded.entries.push(entry);
        }
    };

    let entry_folders = folders
        .user
        .iter()
        .map(|dir| (dir, Source::User))
        .chain(folders.system.iter().map(|dir| (dir, Source::System)));
    for (dir, source) in entry_folders {
        for (id, file, bytes) in read_folder(dir, "toml", &mut loaded.diagnostics) {
            let (declared, diagnostics) = parse_entry(&id, &file, &bytes, source, home);
            loaded.diagnostics.extend(diagnostics);
            match declared {
                Some(Declared::Entry(entry)) => decide(&mut loaded, &id, Some(*entry)),
                Some(Declared::Hidden) => decide(&mut loaded, &id, None),
                None => {}
            }
        }
    }
    for (id, text) in builtin::ENTRIES {
        let file = PathBuf::from(format!("{id}.toml"));
        let (declared, diagnostics) = parse_entry(id, &file, text.as_bytes(), Source::Builtin, home);
        loaded.diagnostics.extend(diagnostics);
        if let Some(Declared::Entry(entry)) = declared {
            decide(&mut loaded, id, Some(*entry));
        }
    }
    for dir in &folders.desktop_files {
        for (id, file, bytes) in read_folder(dir, "desktop", &mut loaded.diagnostics) {
            let (outcome, diagnostics) = parse_desktop_file(&id, &file, &bytes);
            loaded.diagnostics.extend(diagnostics);
            match outcome {
                DesktopFile::Entry(entry) => decide(&mut loaded, &id, Some(*entry)),
                DesktopFile::Hidden | DesktopFile::NotTerminal => decide(&mut loaded, &id, None),
                DesktopFile::Broken => {}
            }
        }
    }
    loaded
}

/// The files of `dir` with `extension`, in name order, as their id, path and bytes. A file that
/// starts with `.` is left out: editors keep their lock and swap files that way.
fn read_folder(dir: &Path, extension: &str, diagnostics: &mut Vec<Diagnostic>) -> Vec<(String, PathBuf, Vec<u8>)> {
    let listing = match std::fs::read_dir(dir) {
        Ok(listing) => listing,
        Err(error) if error.kind() == ErrorKind::NotFound => return Vec::new(),
        Err(error) => {
            diagnostics.push(Diagnostic::at(dir, None, DiagnosticKind::Unreadable).with_detail(error.to_string()));
            return Vec::new();
        }
    };
    let mut paths: Vec<PathBuf> = listing
        .filter_map(Result::ok)
        .map(|item| item.path())
        .filter(|path| path.extension() == Some(OsStr::new(extension)))
        .collect();
    paths.sort();
    let mut files = Vec::with_capacity(paths.len());
    for path in paths {
        let Some(id) = path.file_stem().and_then(OsStr::to_str) else {
            continue;
        };
        if id.is_empty() || id.starts_with('.') || !path.is_file() {
            continue;
        }
        let id = id.to_owned();
        match std::fs::read(&path) {
            Ok(bytes) => files.push((id, path, bytes)),
            Err(error) => {
                diagnostics
                    .push(Diagnostic::at(&path, None, DiagnosticKind::Unreadable).with_detail(error.to_string()));
            }
        }
    }
    files
}

#[cfg(test)]
mod tests {
    use super::super::program::tests::Scratch;
    use super::*;
    use crate::apps::Category;

    fn vars(pairs: &[(&str, &str)]) -> Environment {
        Environment::from_vars(|name| {
            pairs.iter().find(|(key, _)| *key == name).map(|(_, value)| OsString::from(value))
        })
    }

    /// An environment whose data folders all live in `scratch`.
    fn scratch_env(scratch: &Scratch) -> Environment {
        Environment {
            home: Some(scratch.0.join("home")),
            data_home: Some(scratch.0.join("home/.local/share")),
            data_dirs: vec![scratch.0.join("usr/local/share"), scratch.0.join("usr/share")],
            path: Some(scratch.0.join("bin").into_os_string()),
            shell: None,
        }
    }

    const USER: &str = "home/.local/share/quvyta/desktop/apps";
    const LOCAL: &str = "usr/local/share/quvyta/desktop/apps";
    const SYSTEM: &str = "usr/share/quvyta/desktop/apps";
    const USER_DESKTOP: &str = "home/.local/share/applications";
    const DESKTOP: &str = "usr/share/applications";

    fn terminal_desktop(name: &str, exec: &str) -> String {
        format!("[Desktop Entry]\nType=Application\nName={name}\nExec={exec}\nTerminal=true\n")
    }

    fn find<'a>(loaded: &'a Loaded, id: &str) -> Option<&'a Entry> {
        loaded.entries.iter().find(|entry| entry.id == id)
    }

    #[test]
    fn xdg_folders_follow_the_variables() {
        let env =
            vars(&[("HOME", "/home/ada"), ("XDG_DATA_HOME", "/data"), ("XDG_DATA_DIRS", "/opt/share:/usr/share")]);
        let folders = env.folders();
        assert_eq!(folders.user, Some(PathBuf::from("/data/quvyta/desktop/apps")));
        assert_eq!(
            folders.system,
            vec![PathBuf::from("/opt/share/quvyta/desktop/apps"), PathBuf::from("/usr/share/quvyta/desktop/apps")]
        );
        assert_eq!(
            folders.desktop_files,
            vec![
                PathBuf::from("/data/applications"),
                PathBuf::from("/opt/share/applications"),
                PathBuf::from("/usr/share/applications")
            ]
        );
        assert_eq!(folders.watched().len(), 6);
    }

    #[test]
    fn xdg_defaults_fill_in_what_is_missing_or_relative() {
        let env = vars(&[("HOME", "/home/ada"), ("XDG_DATA_HOME", "relative"), ("XDG_DATA_DIRS", "also:relative")]);
        assert_eq!(env.data_home, Some(PathBuf::from("/home/ada/.local/share")));
        assert_eq!(env.data_dirs, vec![PathBuf::from("/usr/local/share"), PathBuf::from("/usr/share")]);
        let homeless = vars(&[("HOME", "ada")]);
        assert_eq!((&homeless.home, &homeless.data_home), (&None, &None));
        assert_eq!(homeless.folders().user, None);
    }

    #[test]
    fn the_terminal_runs_the_users_shell_or_sh() {
        assert_eq!(vars(&[("SHELL", "/usr/bin/fish")]).terminal_shell(), PathBuf::from("/usr/bin/fish"));
        assert_eq!(vars(&[("SHELL", "")]).terminal_shell(), PathBuf::from("/bin/sh"));
        assert_eq!(vars(&[]).terminal_shell(), PathBuf::from("/bin/sh"));
    }

    #[test]
    fn built_in_entries_load_with_no_folders() {
        let folders = Folders { user: None, system: Vec::new(), desktop_files: Vec::new() };
        let loaded = load(&folders, None);
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);
        for id in ["terminal", "settings", "qcode", "qfocus", "qpac", "qtools", "quvyta"] {
            assert!(find(&loaded, id).is_some_and(|entry| entry.source == Source::Builtin), "{id}");
        }
    }

    #[test]
    fn missing_folders_say_nothing() {
        let scratch = Scratch::new();
        let loaded = scratch_env(&scratch).load();
        assert!(loaded.diagnostics.is_empty());
        assert_eq!(loaded.entries.len(), builtin::ENTRIES.len());
    }

    #[test]
    fn higher_sources_win_the_same_id() {
        let scratch = Scratch::new();
        scratch.write(&format!("{USER}/qfocus.toml"), "name = \"My focus\"\ncommand = [\"qfocus\", \"--compact\"]\n");
        scratch.write(&format!("{LOCAL}/htop.toml"), "name = \"htop (local)\"\ncommand = [\"htop\"]\n");
        scratch.write(&format!("{SYSTEM}/htop.toml"), "name = \"htop (system)\"\ncommand = [\"htop\"]\n");
        scratch.write(&format!("{DESKTOP}/htop.desktop"), terminal_desktop("Htop", "htop"));
        scratch.write(&format!("{DESKTOP}/btop.desktop"), terminal_desktop("btop++", "btop"));
        scratch.write(&format!("{DESKTOP}/terminal.desktop"), terminal_desktop("Other terminal", "xterm"));
        let loaded = scratch_env(&scratch).load();
        assert!(loaded.diagnostics.is_empty(), "{:?}", loaded.diagnostics);

        let qfocus = find(&loaded, "qfocus").expect("qfocus");
        assert_eq!((qfocus.name.default.as_str(), qfocus.source), ("My focus", Source::User));
        let htop = find(&loaded, "htop").expect("htop");
        assert_eq!((htop.name.default.as_str(), htop.source), ("htop (local)", Source::System));
        assert_eq!(find(&loaded, "btop").map(|entry| entry.source), Some(Source::DesktopFile));
        assert_eq!(find(&loaded, "terminal").map(|entry| entry.source), Some(Source::Builtin));
        assert_eq!(loaded.entries.iter().filter(|entry| entry.id == "htop").count(), 1);
    }

    #[test]
    fn hidden_hides_the_id_from_every_lower_source() {
        let scratch = Scratch::new();
        scratch.write(&format!("{USER}/htop.toml"), "hidden = true\n");
        scratch.write(&format!("{USER}/qtools.toml"), "hidden = true\n");
        scratch.write(&format!("{SYSTEM}/htop.toml"), "name = \"htop\"\ncommand = [\"htop\"]\n");
        scratch.write(&format!("{DESKTOP}/htop.desktop"), terminal_desktop("Htop", "htop"));
        scratch.write(&format!("{LOCAL}/mc.toml"), "hidden = true\n");
        scratch.write(&format!("{DESKTOP}/mc.desktop"), terminal_desktop("Midnight Commander", "mc"));
        let loaded = scratch_env(&scratch).load();
        assert!(find(&loaded, "htop").is_none());
        assert!(find(&loaded, "qtools").is_none());
        assert!(find(&loaded, "mc").is_none());
        assert!(find(&loaded, "qcode").is_some());
    }

    #[test]
    fn a_hidden_entry_does_not_hide_one_above_it() {
        let scratch = Scratch::new();
        scratch.write(&format!("{USER}/htop.toml"), "name = \"htop\"\ncommand = [\"htop\"]\n");
        scratch.write(&format!("{SYSTEM}/htop.toml"), "hidden = true\n");
        assert!(find(&scratch_env(&scratch).load(), "htop").is_some());
    }

    #[test]
    fn desktop_files_follow_their_own_order() {
        let scratch = Scratch::new();
        // The user's own `.desktop` file wins over the system's, even when it is not a terminal
        // program, as the desktop entry specification says.
        scratch.write(
            &format!("{USER_DESKTOP}/vim.desktop"),
            terminal_desktop("Vim", "vim").replace("Terminal=true", "Terminal=false"),
        );
        scratch.write(&format!("{DESKTOP}/vim.desktop"), terminal_desktop("Vim", "vim"));
        scratch.write(&format!("{USER_DESKTOP}/htop.desktop"), terminal_desktop("Htop", "htop") + "NoDisplay=true\n");
        scratch.write(&format!("{DESKTOP}/htop.desktop"), terminal_desktop("Htop", "htop"));
        scratch.write(&format!("{USER_DESKTOP}/btop.desktop"), terminal_desktop("My btop", "btop"));
        scratch.write(&format!("{DESKTOP}/btop.desktop"), terminal_desktop("btop++", "btop"));
        let loaded = scratch_env(&scratch).load();
        assert!(find(&loaded, "vim").is_none());
        assert!(find(&loaded, "htop").is_none());
        assert_eq!(find(&loaded, "btop").map(|entry| entry.name.default.as_str()), Some("My btop"));
    }

    #[test]
    fn a_broken_file_is_reported_and_the_rest_load() {
        let scratch = Scratch::new();
        scratch.write(&format!("{USER}/broken.toml"), "name = \"x\ncommand = [\n");
        scratch.write(&format!("{USER}/htop.toml"), "name = \"htop\"\ncommand = [\"htop\"]\nfuture = 1\n");
        scratch.write(&format!("{USER}/qcode.toml"), "name = 1\ncommand = [\"qcode\"]\n");
        scratch.write(&format!("{USER}/.htop.toml"), "garbage");
        scratch.write(&format!("{USER}/notes.txt"), "garbage");
        scratch.write(&format!("{DESKTOP}/ranger.desktop"), terminal_desktop("ranger", "\"unclosed"));
        let loaded = scratch_env(&scratch).load();

        assert!(find(&loaded, "broken").is_none());
        assert!(find(&loaded, "htop").is_some());
        assert!(find(&loaded, "ranger").is_none());
        // The user's broken qcode decides nothing: the built-in one shows.
        assert_eq!(find(&loaded, "qcode").map(|entry| entry.source), Some(Source::Builtin));

        let problems: Vec<(String, DiagnosticKind)> = loaded
            .diagnostics
            .iter()
            .map(|d| (d.path.file_name().and_then(OsStr::to_str).unwrap_or_default().to_owned(), d.kind.clone()))
            .collect();
        assert!(problems.contains(&("broken.toml".to_owned(), DiagnosticKind::Syntax)), "{problems:?}");
        assert!(problems.contains(&("htop.toml".to_owned(), DiagnosticKind::UnknownField { key: "future".into() })));
        assert!(
            problems
                .iter()
                .any(|(file, kind)| file == "qcode.toml" && matches!(kind, DiagnosticKind::WrongType { .. }))
        );
        assert!(problems.contains(&("ranger.desktop".to_owned(), DiagnosticKind::BadExec)));
        assert!(problems.iter().all(|(file, _)| file != ".htop.toml" && file != "notes.txt"));
    }

    #[test]
    fn home_is_expanded_in_loaded_entries() {
        let scratch = Scratch::new();
        scratch.write(&format!("{USER}/notes.toml"), "name = \"Notes\"\ncommand = [\"vim\"]\nfolder = \"~/notes\"\n");
        let env = scratch_env(&scratch);
        let loaded = env.load();
        let notes = find(&loaded, "notes").expect("notes");
        assert_eq!(notes.folder, env.home.map(|home| home.join("notes")));
        assert_eq!(notes.category, Category::Other);
    }

    #[cfg(unix)]
    #[test]
    fn an_unreadable_folder_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let scratch = Scratch::new();
        let file = scratch.write(&format!("{USER}/htop.toml"), "name = \"htop\"\ncommand = [\"htop\"]\n");
        let folder = file.parent().expect("a folder").to_path_buf();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o000)).expect("permissions");
        // A user who may read anything, such as root, still reads it; then there is nothing to check.
        let readable = std::fs::read_dir(&folder).is_ok();
        let loaded = scratch_env(&scratch).load();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).expect("permissions");
        if !readable {
            assert_eq!(loaded.diagnostics[0].kind, DiagnosticKind::Unreadable);
            assert_eq!(loaded.diagnostics[0].path, folder);
            assert!(loaded.diagnostics[0].detail.is_some());
        }
    }

    #[test]
    fn installed_means_the_program_is_found() {
        let scratch = Scratch::new();
        scratch.program("bin/htop", 0o755);
        let env = scratch_env(&scratch);
        let entry = |launch: Launch, try_exec: Option<&str>| Entry {
            id: "x".into(),
            name: crate::apps::Localized::plain("x"),
            comment: None,
            icon: None,
            launch,
            folder: None,
            env: Vec::new(),
            category: Category::Other,
            keywords: Vec::new(),
            single: false,
            close_on_exit: false,
            window: crate::apps::WindowPrefs::default(),
            install: crate::apps::Install::default(),
            try_exec: try_exec.map(str::to_owned),
            source: Source::User,
            file: None,
        };
        let command = |program: &str| Launch::Command(vec![program.to_owned()]);
        assert!(env.is_installed(&entry(command("htop"), None)));
        assert!(!env.is_installed(&entry(command("btop"), None)));
        assert!(env.is_installed(&entry(command("htop"), Some("htop"))));
        assert!(!env.is_installed(&entry(command("htop"), Some("htop-helper"))));
        assert!(env.is_installed(&entry(Launch::Screen(crate::apps::Screen::Terminal), None)));
        assert!(env.is_installed(&entry(Launch::Open(PathBuf::from("/gone")), None)));
    }
}
