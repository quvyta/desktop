//! Whether a program is installed: found on `PATH`, or at the path an entry names.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// Finds `program` the way a shell would, without starting anything.
///
/// A name with a `/` is a path and is checked as it is. A bare name is looked for in each folder
/// of `path`, a `PATH` value, in order. Empty folders in `PATH` are skipped: they would mean the
/// folder qdesk happened to start in. `probe` says whether a path is a program that can run;
/// [`is_executable`] is the real one.
#[must_use]
pub fn find_program(program: &str, path: Option<&OsStr>, probe: impl Fn(&Path) -> bool) -> Option<PathBuf> {
    if program.is_empty() {
        return None;
    }
    if program.contains('/') {
        let direct = PathBuf::from(program);
        return probe(&direct).then_some(direct);
    }
    std::env::split_paths(path?)
        .filter(|folder| !folder.as_os_str().is_empty())
        .map(|folder| folder.join(program))
        .find(|candidate| probe(candidate))
}

/// Whether `path` is a file this user may run: a regular file (or a link to one) with an
/// execute bit set.
#[must_use]
pub fn is_executable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A folder of its own under the system's temporary folder, removed when dropped.
    pub(crate) struct Scratch(pub(crate) PathBuf);

    impl Scratch {
        pub(crate) fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let number = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!("qdesk-apps-{}-{number}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("a scratch folder");
            Self(path)
        }

        /// Writes `text` to `relative`, making its folders.
        pub(crate) fn write(&self, relative: &str, text: impl AsRef<[u8]>) -> PathBuf {
            let path = self.0.join(relative);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).expect("a folder");
            }
            std::fs::write(&path, text).expect("a file");
            path
        }

        /// Writes a program with the given permission bits.
        pub(crate) fn program(&self, relative: &str, mode: u32) -> PathBuf {
            let path = self.write(relative, "#!/bin/sh\n");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).expect("permissions");
            }
            #[cfg(not(unix))]
            let _ = mode;
            path
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn joined(folders: &[&Path]) -> OsString {
        std::env::join_paths(folders).expect("a PATH")
    }

    #[test]
    fn finds_a_program_in_the_first_folder_that_has_it() {
        let scratch = Scratch::new();
        let first = scratch.0.join("a");
        let second = scratch.0.join("b");
        let later = scratch.program("b/htop", 0o755);
        std::fs::create_dir_all(&first).expect("a folder");
        let path = joined(&[&first, &second]);
        assert_eq!(find_program("htop", Some(&path), is_executable), Some(later));
        let earlier = scratch.program("a/htop", 0o700);
        assert_eq!(find_program("htop", Some(&path), is_executable), Some(earlier));
    }

    #[test]
    fn a_missing_program_is_not_found() {
        let scratch = Scratch::new();
        let path = joined(&[&scratch.0]);
        assert_eq!(find_program("htop", Some(&path), is_executable), None);
        assert_eq!(find_program("htop", None, is_executable), None);
        assert_eq!(find_program("", Some(&path), is_executable), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_file_without_an_execute_bit_is_not_a_program() {
        let scratch = Scratch::new();
        scratch.program("notes", 0o644);
        let path = joined(&[&scratch.0]);
        assert_eq!(find_program("notes", Some(&path), is_executable), None);
    }

    #[test]
    fn a_folder_is_not_a_program() {
        let scratch = Scratch::new();
        std::fs::create_dir_all(scratch.0.join("htop")).expect("a folder");
        let path = joined(&[&scratch.0]);
        assert_eq!(find_program("htop", Some(&path), is_executable), None);
    }

    #[test]
    fn a_name_with_a_slash_is_checked_where_it_points() {
        let scratch = Scratch::new();
        let tool = scratch.program("bin/tool", 0o755);
        let tool_text = tool.to_str().expect("a UTF-8 path");
        // The folder is not on PATH: the path alone decides.
        assert_eq!(find_program(tool_text, Some(OsStr::new("/nonexistent")), is_executable), Some(tool.clone()));
        assert_eq!(find_program(&format!("{tool_text}-gone"), None, is_executable), None);
    }

    #[test]
    fn empty_path_folders_are_skipped() {
        let seen = std::cell::RefCell::new(Vec::new());
        let probe = |candidate: &Path| {
            seen.borrow_mut().push(candidate.to_path_buf());
            false
        };
        assert_eq!(find_program("htop", Some(OsStr::new(":/usr/bin::/bin:")), probe), None);
        assert_eq!(seen.into_inner(), vec![PathBuf::from("/usr/bin/htop"), PathBuf::from("/bin/htop")]);
    }
}
