//! Same-UID Unix socket, peer credentials, and detached child primitives.

use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::user_state::{FileLock, PathError, UserPaths, open_append};

/// Daemon lifetime lock required for socket ownership operations.
pub struct DaemonLock(FileLock);

impl DaemonLock {
    /// Tries to acquire the per-user daemon lifetime lock.
    pub fn try_acquire(paths: &UserPaths) -> Result<Option<Self>, LocalPlatformError> {
        Ok(FileLock::try_acquire(paths.daemon_lock(), paths.uid())?.map(Self))
    }

    /// Lock file path retained for diagnostics.
    #[must_use]
    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

/// Unix local-daemon platform failure.
#[derive(Debug)]
pub enum LocalPlatformError {
    /// Managed path boundary rejected a node.
    Path(PathError),
    /// Another live daemon owns the socket.
    AlreadyRunning,
    /// Existing socket candidate is not a current-UID real socket.
    UnsafeSocket(PathBuf),
    /// OS peer credentials identified another user.
    PeerUidMismatch {
        /// Effective daemon UID.
        expected: u32,
        /// Connecting process UID.
        actual: u32,
    },
    /// Required integration is unavailable on this target.
    UnsupportedPlatform,
    /// Native operation failed.
    Io(String),
}

impl fmt::Display for LocalPlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Path(error) => error.fmt(formatter),
            Self::AlreadyRunning => write!(formatter, "another daemon is already listening"),
            Self::UnsafeSocket(path) => write!(
                formatter,
                "refusing unsafe stale socket candidate {}",
                path.display()
            ),
            Self::PeerUidMismatch { expected, actual } => write!(
                formatter,
                "local peer UID {actual} does not match daemon UID {expected}"
            ),
            Self::UnsupportedPlatform => write!(
                formatter,
                "Unix local daemon integration is unsupported on this platform"
            ),
            Self::Io(detail) => write!(formatter, "local platform operation failed: {detail}"),
        }
    }
}

impl std::error::Error for LocalPlatformError {}

impl From<PathError> for LocalPlatformError {
    fn from(error: PathError) -> Self {
        Self::Path(error)
    }
}

/// Applies the same-UID authorization decision to an observed peer.
pub fn authorize_peer(expected: u32, actual: u32) -> Result<(), LocalPlatformError> {
    if expected == actual {
        Ok(())
    } else {
        Err(LocalPlatformError::PeerUidMismatch { expected, actual })
    }
}

/// Returns and authorizes the UID of a connected Unix-domain stream.
#[cfg(unix)]
pub fn authorize_stream_peer<F: std::os::fd::AsFd>(
    stream: &F,
    expected: u32,
) -> Result<u32, LocalPlatformError> {
    let actual = peer_uid(stream)?;
    authorize_peer(expected, actual)?;
    Ok(actual)
}

/// Validates an existing daemon socket without connecting or modifying it.
#[cfg(unix)]
pub fn inspect_daemon_socket(paths: &UserPaths) -> Result<bool, LocalPlatformError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};

    let metadata = match fs::symlink_metadata(paths.socket()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(LocalPlatformError::Io(error.to_string())),
    };
    if metadata.file_type().is_symlink()
        || !metadata.file_type().is_socket()
        || metadata.uid() != paths.uid()
        || metadata.permissions().mode() & 0o177 != 0
    {
        return Err(LocalPlatformError::UnsafeSocket(
            paths.socket().to_path_buf(),
        ));
    }
    Ok(true)
}

/// Binds the daemon socket, removing an owned stale socket only for the lock owner.
#[cfg(unix)]
pub fn bind_daemon_socket(
    paths: &UserPaths,
    _lock: &DaemonLock,
) -> Result<std::os::unix::net::UnixListener, LocalPlatformError> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::{UnixListener, UnixStream};

    paths.prepare_runtime_directory()?;
    match fs::symlink_metadata(paths.socket()) {
        Ok(metadata) => {
            if UnixStream::connect(paths.socket()).is_ok() {
                return Err(LocalPlatformError::AlreadyRunning);
            }
            if metadata.file_type().is_symlink()
                || !metadata.file_type().is_socket()
                || metadata.uid() != paths.uid()
            {
                return Err(LocalPlatformError::UnsafeSocket(
                    paths.socket().to_path_buf(),
                ));
            }
            fs::remove_file(paths.socket())
                .map_err(|error| LocalPlatformError::Io(error.to_string()))?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(LocalPlatformError::Io(error.to_string())),
    }

    let listener = UnixListener::bind(paths.socket())
        .map_err(|error| LocalPlatformError::Io(error.to_string()))?;
    fs::set_permissions(paths.socket(), fs::Permissions::from_mode(0o600))
        .map_err(|error| LocalPlatformError::Io(error.to_string()))?;
    Ok(listener)
}

/// Removes this daemon's own socket during graceful shutdown.
#[cfg(unix)]
pub fn remove_own_socket(paths: &UserPaths, _lock: &DaemonLock) -> Result<(), LocalPlatformError> {
    match fs::remove_file(paths.socket()) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(LocalPlatformError::Io(error.to_string())),
    }
}

/// Builds a detached daemon command with stable cwd and managed log stdio.
pub fn detached_command(
    executable: &Path,
    paths: &UserPaths,
    internal_argument: &str,
) -> Result<Command, LocalPlatformError> {
    paths.prepare_state_directories()?;
    let log = open_append(paths.daemon_log(), paths.uid())?;
    let stderr = log
        .try_clone()
        .map_err(|error| LocalPlatformError::Io(error.to_string()))?;
    let mut command = Command::new(executable);
    command
        .arg(internal_argument)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .current_dir(paths.home());
    Ok(command)
}

/// Starts a new session in the already-spawned internal daemon child.
pub fn detach_current_process() -> Result<(), LocalPlatformError> {
    #[cfg(unix)]
    {
        nix::unistd::setsid().map_err(|error| LocalPlatformError::Io(error.to_string()))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        Err(LocalPlatformError::UnsupportedPlatform)
    }
}

#[cfg(target_os = "linux")]
fn peer_uid<F: std::os::fd::AsFd>(stream: &F) -> Result<u32, LocalPlatformError> {
    nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)
        .map(|credentials| credentials.uid())
        .map_err(|error| LocalPlatformError::Io(error.to_string()))
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "netbsd",
    target_os = "openbsd"
))]
fn peer_uid<F: std::os::fd::AsFd>(stream: &F) -> Result<u32, LocalPlatformError> {
    nix::unistd::getpeereid(stream)
        .map(|(uid, _)| uid.as_raw())
        .map_err(|error| LocalPlatformError::Io(error.to_string()))
}

#[cfg(all(
    unix,
    not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))
))]
fn peer_uid<F: std::os::fd::AsFd>(_stream: &F) -> Result<u32, LocalPlatformError> {
    Err(LocalPlatformError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn isolated_paths() -> (tempfile::TempDir, UserPaths) {
        use nix::unistd::Uid;
        let temporary = tempfile::tempdir().expect("temporary root");
        let home = temporary.path().join("home");
        fs::create_dir(&home).expect("test home");
        let paths = UserPaths::for_test(
            Uid::effective().as_raw(),
            home.clone(),
            home.join(".zterm"),
            temporary.path().join("run"),
        );
        paths.prepare_state_directories().expect("state dirs");
        (temporary, paths)
    }

    #[test]
    fn pure_peer_policy_rejects_other_users() {
        assert!(authorize_peer(1000, 1000).is_ok());
        assert!(matches!(
            authorize_peer(1000, 1001),
            Err(LocalPlatformError::PeerUidMismatch { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn real_unix_stream_reports_the_effective_uid() {
        use nix::unistd::Uid;
        use std::os::unix::net::UnixStream;
        let (left, _right) = UnixStream::pair().expect("Unix stream pair");
        assert_eq!(
            authorize_stream_peer(&left, Uid::effective().as_raw()).expect("same UID accepted"),
            Uid::effective().as_raw()
        );
    }

    #[cfg(unix)]
    #[test]
    fn daemon_lock_owner_preserves_live_socket_and_replaces_only_safe_stale_socket() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        use std::os::unix::net::UnixStream;

        let (_temporary, paths) = isolated_paths();
        let lock = DaemonLock::try_acquire(&paths)
            .expect("daemon lock probe")
            .expect("daemon lock acquired");
        let listener = bind_daemon_socket(&paths, &lock).expect("first bind");
        assert!(UnixStream::connect(paths.socket()).is_ok());
        assert!(
            DaemonLock::try_acquire(&paths)
                .expect("second daemon lock probe")
                .is_none()
        );
        assert_eq!(
            fs::metadata(paths.socket())
                .expect("socket metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        drop(listener);
        let rebound = bind_daemon_socket(&paths, &lock).expect("safe stale socket replaced");
        drop(rebound);
        remove_own_socket(&paths, &lock).expect("own socket removed");

        let symlink_target = paths.runtime_dir().join("socket-target");
        fs::write(&symlink_target, b"target").expect("symlink target");
        symlink(&symlink_target, paths.socket()).expect("socket symlink fixture");
        assert!(matches!(
            bind_daemon_socket(&paths, &lock),
            Err(LocalPlatformError::UnsafeSocket(_))
        ));
        fs::remove_file(paths.socket()).expect("socket symlink cleanup");

        fs::write(paths.socket(), b"not a socket").expect("unsafe candidate fixture");
        assert!(matches!(
            bind_daemon_socket(&paths, &lock),
            Err(LocalPlatformError::UnsafeSocket(_))
        ));
    }
}
