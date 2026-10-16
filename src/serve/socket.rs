//! Where the background desktop listens, and who may talk to it.
//!
//! The socket is the person's alone: it lives in a folder only they can enter, the socket itself
//! can only be opened by them, and every connection is asked who made it, so a program of another
//! user that reached the socket anyway is turned away before it is answered. Whoever can talk to
//! the background can type into every program it holds.

use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};

/// The socket's file name, in its folder.
pub const FILE: &str = "qdesk.sock";

/// Where the socket of this user is: `quvyta-<uid>` in the temporary folder, `$TMPDIR` when it
/// names one and `/tmp` otherwise, a folder of their own made for it.
///
/// Not the runtime folder a login session gives (`$XDG_RUNTIME_DIR`): the system takes that away
/// when the user's last session ends, and on a server reached over SSH the connection that dropped
/// is often the last one. The screen of the next connection would find no socket there, start a
/// second background and never reach the programs the first one holds.
#[must_use]
pub fn place(temporary: Option<&OsStr>, uid: u32) -> PathBuf {
    let base = temporary.filter(|dir| !dir.is_empty()).map(Path::new).filter(|dir| dir.is_absolute());
    base.unwrap_or_else(|| Path::new("/tmp")).join(format!("quvyta-{uid}")).join(FILE)
}

/// The socket's place on this machine, for the user running this process.
#[must_use]
pub fn here() -> PathBuf {
    place(std::env::var_os("TMPDIR").as_deref(), own_uid())
}

/// The user this process runs as.
#[must_use]
pub fn own_uid() -> u32 {
    rustix::process::getuid().as_raw()
}

/// Makes the socket's folder, or checks the one that is there: it must be `uid`'s and closed to
/// everyone else. A folder of another user's, or one others may enter, is refused rather than
/// repaired: under `/tmp` anyone could have made it first, to listen in.
///
/// # Errors
///
/// When the folder cannot be made, or is refused.
pub fn prepare_folder(folder: &Path, uid: u32) -> io::Result<()> {
    match fs::DirBuilder::new().mode(0o700).create(folder) {
        Ok(()) => return Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        // A parent that is missing too, as a test's own temporary folder may be.
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::DirBuilder::new().recursive(true).mode(0o700).create(folder)?;
            return Ok(());
        }
        Err(error) => return Err(error),
    }
    let meta = fs::symlink_metadata(folder)?;
    if !meta.is_dir() {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} is not a folder", folder.display())));
    }
    if meta.uid() != uid {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} belongs to another user", folder.display()),
        ));
    }
    if meta.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} can be entered by other users", folder.display()),
        ));
    }
    Ok(())
}

/// Why the socket could not be listened on.
#[derive(Debug)]
pub enum BindError {
    /// A background desktop is already answering on it.
    Serving,
    /// The folder or the socket could not be made.
    Io(io::Error),
}

impl From<io::Error> for BindError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Listens on `path`, in a folder [`prepare_folder`] made or checked.
///
/// A socket file nobody answers on is what a background that ended without cleaning up leaves:
/// it is taken away and listened on again. One somebody answers on is left alone, since that is a
/// background still holding programs.
///
/// # Errors
///
/// [`BindError::Serving`] when one answers already, [`BindError::Io`] when the folder or the
/// socket cannot be made.
pub fn bind(path: &Path) -> Result<UnixListener, BindError> {
    let folder = path.parent().unwrap_or_else(|| Path::new("/"));
    prepare_folder(folder, own_uid())?;
    if fs::symlink_metadata(path).is_ok() {
        if UnixStream::connect(path).is_ok() {
            return Err(BindError::Serving);
        }
        fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Whether the other end of `stream` runs as `uid`.
///
/// # Errors
///
/// When the system cannot say who the other end is; such a connection is not answered either.
pub fn same_user(stream: &UnixStream, uid: u32) -> io::Result<bool> {
    let peer = rustix::net::sockopt::socket_peercred(stream)?;
    Ok(allowed(peer.uid.as_raw(), uid))
}

/// Whether a connection made by `peer` may talk to the background of `owner`: only the owner may,
/// root included, since root reaching into a person's programs is not something qdesk offers.
#[must_use]
pub fn allowed(peer: u32, owner: u32) -> bool {
    peer == owner
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qdesk-serve-socket-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch folder");
        dir
    }

    #[test]
    fn the_socket_is_in_a_folder_of_the_users_own_in_the_temporary_folder() {
        assert_eq!(place(Some(OsStr::new("/var/tmp")), 1000), Path::new("/var/tmp/quvyta-1000/qdesk.sock"));
        assert_eq!(place(None, 1000), Path::new("/tmp/quvyta-1000/qdesk.sock"));
        assert_eq!(place(Some(OsStr::new("")), 7), Path::new("/tmp/quvyta-7/qdesk.sock"), "empty is none");
        assert_eq!(place(Some(OsStr::new("tmp")), 7), Path::new("/tmp/quvyta-7/qdesk.sock"), "relative is none");
    }

    #[test]
    fn a_new_folder_is_closed_to_everyone_else() {
        let dir = scratch("new");
        let folder = dir.join("tmp/quvyta-1");
        prepare_folder(&folder, own_uid()).expect("made");
        assert_eq!(fs::metadata(&folder).expect("there").mode() & 0o777, 0o700);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_folder_others_may_enter_or_of_another_user_is_refused() {
        let dir = scratch("open");
        let folder = dir.join("quvyta");
        fs::create_dir(&folder).expect("made");
        fs::set_permissions(&folder, fs::Permissions::from_mode(0o755)).expect("opened");
        let error = prepare_folder(&folder, own_uid()).expect_err("an open folder is refused");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::metadata(&folder).expect("there").mode() & 0o777, 0o755, "and left as it was");

        fs::set_permissions(&folder, fs::Permissions::from_mode(0o700)).expect("closed");
        prepare_folder(&folder, own_uid()).expect("a closed folder of one's own is taken");
        let error = prepare_folder(&folder, own_uid() + 1).expect_err("another user's folder is refused");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_socket_opens_only_for_its_user_and_a_dead_one_is_taken_over() {
        let dir = scratch("bind");
        let path = dir.join("quvyta").join(FILE);
        let listener = bind(&path).expect("listening");
        assert_eq!(fs::metadata(&path).expect("there").mode() & 0o777, 0o600);
        assert!(matches!(bind(&path), Err(BindError::Serving)), "one that answers is left alone");
        drop(listener);
        // The file is still there, but nobody answers on it.
        assert!(path.exists());
        let _again = bind(&path).expect("a socket nobody answers on is taken over");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_the_owner_may_talk_to_the_background() {
        let (one, _other) = UnixStream::pair().expect("pair");
        assert!(same_user(&one, own_uid()).expect("the system says who"), "a connection of one's own is answered");
        assert!(!same_user(&one, own_uid() + 1).expect("the system says who"));
        assert!(allowed(1000, 1000));
        assert!(!allowed(0, 1000), "not even root");
        assert!(!allowed(1001, 1000));
    }
}
