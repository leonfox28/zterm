//! Effective-user paths, permissions, atomic files, and lifecycle locks.

use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::account::EffectiveAccount;

#[cfg(unix)]
const DIRECTORY_MODE: u32 = 0o700;
#[cfg(unix)]
const FILE_MODE: u32 = 0o600;
const MAX_SOCKET_PATH_BYTES: usize = 100;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// All persistent and runtime paths owned by one effective user.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserPaths {
    uid: u32,
    home: PathBuf,
    login_shell: PathBuf,
    state_root: PathBuf,
    config: PathBuf,
    identity: PathBuf,
    database: PathBuf,
    install_metadata: PathBuf,
    logs: PathBuf,
    daemon_log: PathBuf,
    lifecycle_lock: PathBuf,
    daemon_lock: PathBuf,
    runtime_dir: PathBuf,
    socket: PathBuf,
}

impl UserPaths {
    /// Derives product paths from the effective account and safe runtime candidates.
    #[must_use]
    pub fn for_account(account: &EffectiveAccount) -> Self {
        let state_root = account.home().join(".zterm");
        let runtime_dir =
            runtime_candidate(account.uid()).unwrap_or_else(|| runtime_fallback(account.uid()));
        Self::from_roots(
            account.uid(),
            account.home().to_path_buf(),
            account.shell().to_path_buf(),
            state_root,
            runtime_dir,
        )
    }

    /// Constructs isolated paths for tests without consulting product environment variables.
    #[doc(hidden)]
    #[must_use]
    pub fn for_test(uid: u32, home: PathBuf, state_root: PathBuf, runtime_dir: PathBuf) -> Self {
        Self::from_roots(uid, home, PathBuf::from("/bin/sh"), state_root, runtime_dir)
    }

    fn from_roots(
        uid: u32,
        home: PathBuf,
        login_shell: PathBuf,
        state_root: PathBuf,
        runtime_dir: PathBuf,
    ) -> Self {
        Self {
            uid,
            home,
            login_shell,
            config: state_root.join("config.toml"),
            identity: state_root.join("identity.key"),
            database: state_root.join("state.sqlite3"),
            install_metadata: state_root.join("install.json"),
            logs: state_root.join("logs"),
            daemon_log: state_root.join("logs/daemon.log"),
            lifecycle_lock: state_root.join("lifecycle.lock"),
            daemon_lock: state_root.join("daemon.lock"),
            socket: runtime_dir.join("daemon.sock"),
            state_root,
            runtime_dir,
        }
    }

    /// Effective UID which owns these paths.
    #[must_use]
    pub const fn uid(&self) -> u32 {
        self.uid
    }

    /// Effective account home used as detached child cwd.
    #[must_use]
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Login shell from the effective account database.
    #[must_use]
    pub fn login_shell(&self) -> &Path {
        &self.login_shell
    }

    /// Persistent root (`~/.zterm`).
    #[must_use]
    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    /// Versioned TOML configuration.
    #[must_use]
    pub fn config(&self) -> &Path {
        &self.config
    }

    /// Raw 32-byte Iroh identity key.
    #[must_use]
    pub fn identity(&self) -> &Path {
        &self.identity
    }

    /// Bundled SQLite state database.
    #[must_use]
    pub fn database(&self) -> &Path {
        &self.database
    }

    /// Reserved installer-owned metadata path.
    #[must_use]
    pub fn install_metadata(&self) -> &Path {
        &self.install_metadata
    }

    /// Managed log directory.
    #[must_use]
    pub fn logs(&self) -> &Path {
        &self.logs
    }

    /// Daemon lifecycle log.
    #[must_use]
    pub fn daemon_log(&self) -> &Path {
        &self.daemon_log
    }

    /// Short-lived setup/spawn lock.
    #[must_use]
    pub fn lifecycle_lock(&self) -> &Path {
        &self.lifecycle_lock
    }

    /// Daemon lifetime instance lock.
    #[must_use]
    pub fn daemon_lock(&self) -> &Path {
        &self.daemon_lock
    }

    /// Owned runtime directory containing the local socket.
    #[must_use]
    pub fn runtime_dir(&self) -> &Path {
        &self.runtime_dir
    }

    /// Same-UID local IPC socket.
    #[must_use]
    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Creates and validates the persistent root and log directory.
    pub fn prepare_state_directories(&self) -> Result<(), PathError> {
        create_or_validate_directory(&self.state_root, self.uid)?;
        create_or_validate_directory(&self.logs, self.uid)
    }

    /// Creates and validates the runtime socket directory.
    pub fn prepare_runtime_directory(&self) -> Result<(), PathError> {
        if socket_path_bytes(&self.socket) > MAX_SOCKET_PATH_BYTES {
            return Err(PathError::SocketPathTooLong(self.socket.clone()));
        }
        create_or_validate_directory(&self.runtime_dir, self.uid)
    }
}

/// Managed filesystem trust-boundary failure.
#[derive(Debug)]
pub enum PathError {
    /// Platform does not expose the required Unix ownership/mode boundary.
    UnsupportedPlatform,
    /// Managed path is not absolute.
    NotAbsolute(PathBuf),
    /// Managed path is a symlink.
    Symlink(PathBuf),
    /// Managed node has the wrong file type.
    WrongType(PathBuf),
    /// Managed node is owned by another UID.
    WrongOwner {
        /// Rejected path.
        path: PathBuf,
        /// Required effective UID.
        expected: u32,
        /// Observed owner UID.
        actual: u32,
    },
    /// Managed node is accessible more broadly than allowed.
    WrongMode {
        /// Rejected path.
        path: PathBuf,
        /// Required maximum or exact mode.
        expected: u32,
        /// Observed permission bits.
        actual: u32,
    },
    /// Unix socket path cannot fit the supported sockaddr boundary.
    SocketPathTooLong(PathBuf),
    /// Native filesystem operation failed.
    Io {
        /// Path operated on.
        path: PathBuf,
        /// Stable operation detail.
        detail: String,
    },
}

impl fmt::Display for PathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedPlatform => write!(
                formatter,
                "managed user paths are unsupported on this platform"
            ),
            Self::NotAbsolute(path) => write!(
                formatter,
                "managed path is not absolute: {}",
                path.display()
            ),
            Self::Symlink(path) => write!(
                formatter,
                "managed path must not be a symlink: {}",
                path.display()
            ),
            Self::WrongType(path) => write!(
                formatter,
                "managed path has the wrong file type: {}",
                path.display()
            ),
            Self::WrongOwner {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "managed path {} is owned by UID {actual}, expected {expected}",
                path.display()
            ),
            Self::WrongMode {
                path,
                expected,
                actual,
            } => write!(
                formatter,
                "managed path {} has mode {actual:o}, expected {expected:o}",
                path.display()
            ),
            Self::SocketPathTooLong(path) => write!(
                formatter,
                "Unix socket path is too long: {}",
                path.display()
            ),
            Self::Io { path, detail } => write!(
                formatter,
                "filesystem operation failed for {}: {detail}",
                path.display()
            ),
        }
    }
}

impl std::error::Error for PathError {}

/// Acquired exclusive standard-library file lock.
pub struct FileLock {
    file: File,
    path: PathBuf,
}

/// Non-mutating observation of an existing managed lock file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExistingLockState {
    /// The lock file has not been created yet.
    Missing,
    /// The file exists and no process currently holds its exclusive lock.
    Unlocked,
    /// Another process currently holds the exclusive lock.
    Locked,
}

impl FileLock {
    /// Tries to acquire an exclusive lock, returning `None` when another owner holds it.
    pub fn try_acquire(path: &Path, uid: u32) -> Result<Option<Self>, PathError> {
        let file = open_managed_file(path, uid, true, true)?;
        match file.try_lock() {
            Ok(()) => Ok(Some(Self {
                file,
                path: path.to_path_buf(),
            })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(error)) => Err(io_error(path, error)),
        }
    }

    /// Path whose file description owns this lock.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        let _ = File::unlock(&self.file);
    }
}

/// Atomically replaces a managed regular file after a successful writer closure.
pub fn atomic_write(
    path: &Path,
    uid: u32,
    writer: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<(), PathError> {
    atomic_write_inner(path, uid, true, writer)
}

/// Atomically creates a managed regular file and refuses to replace it.
pub fn atomic_create(
    path: &Path,
    uid: u32,
    writer: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<(), PathError> {
    atomic_write_inner(path, uid, false, writer)
}

/// Opens a managed file for bounded append after validating owner, type and mode.
pub fn open_append(path: &Path, uid: u32) -> Result<File, PathError> {
    open_managed_file(path, uid, true, true)
}

/// Validates an existing managed regular file.
pub fn validate_regular_file(path: &Path, uid: u32) -> Result<(), PathError> {
    validate_node(path, uid, ManagedType::RegularFile)
}

/// Validates an existing managed directory without creating it.
pub fn validate_directory(path: &Path, uid: u32) -> Result<(), PathError> {
    validate_node(path, uid, ManagedType::Directory)
}

/// Observes an existing lock without creating the file or retaining the lock.
pub fn inspect_existing_lock(path: &Path, uid: u32) -> Result<ExistingLockState, PathError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(ExistingLockState::Missing);
        }
        Err(error) => return Err(io_error(path, error)),
    }
    let file = open_managed_file(path, uid, false, false)?;
    match file.try_lock() {
        Ok(()) => {
            File::unlock(&file).map_err(|error| io_error(path, error))?;
            Ok(ExistingLockState::Unlocked)
        }
        Err(std::fs::TryLockError::WouldBlock) => Ok(ExistingLockState::Locked),
        Err(std::fs::TryLockError::Error(error)) => Err(io_error(path, error)),
    }
}

fn atomic_write_inner(
    path: &Path,
    uid: u32,
    replace: bool,
    writer: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<(), PathError> {
    ensure_absolute(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| PathError::NotAbsolute(path.to_path_buf()))?;
    validate_node(parent, uid, ManagedType::Directory)?;
    if !replace && fs::symlink_metadata(path).is_ok() {
        return Err(PathError::Io {
            path: path.to_path_buf(),
            detail: "managed file already exists".to_owned(),
        });
    }
    if replace && fs::symlink_metadata(path).is_ok() {
        validate_regular_file(path, uid)?;
    }

    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .ok_or_else(|| PathError::WrongType(path.to_path_buf()))?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}-{sequence}",
        name.to_string_lossy(),
        std::process::id()
    ));
    let mut file = create_new_file(&temporary)?;
    let result = writer(&mut file)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(&temporary, error));
    if let Err(error) = result {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    drop(file);

    if !replace && fs::symlink_metadata(path).is_ok() {
        let _ = fs::remove_file(&temporary);
        return Err(PathError::Io {
            path: path.to_path_buf(),
            detail: "managed file already exists".to_owned(),
        });
    }
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        io_error(path, error)
    })?;
    sync_directory(parent)
}

fn create_or_validate_directory(path: &Path, uid: u32) -> Result<(), PathError> {
    ensure_absolute(path)?;
    match fs::symlink_metadata(path) {
        Ok(_) => validate_node(path, uid, ManagedType::Directory),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let mut builder = fs::DirBuilder::new();
                builder.mode(DIRECTORY_MODE);
                match builder.create(path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => return Err(io_error(path, error)),
                }
                validate_node(path, uid, ManagedType::Directory)
            }
            #[cfg(not(unix))]
            {
                let _ = uid;
                Err(PathError::UnsupportedPlatform)
            }
        }
        Err(error) => Err(io_error(path, error)),
    }
}

fn open_managed_file(path: &Path, uid: u32, create: bool, append: bool) -> Result<File, PathError> {
    ensure_absolute(path)?;
    if fs::symlink_metadata(path).is_ok() {
        validate_regular_file(path, uid)?;
    }
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(create).append(append);
    configure_secure_open(&mut options);
    let file = options.open(path).map_err(|error| io_error(path, error))?;
    validate_regular_file(path, uid)?;
    Ok(file)
}

fn create_new_file(path: &Path) -> Result<File, PathError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_secure_open(&mut options);
    options.open(path).map_err(|error| io_error(path, error))
}

fn configure_secure_open(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(FILE_MODE).custom_flags(nix::libc::O_NOFOLLOW);
    }
    #[cfg(not(unix))]
    let _ = options;
}

fn sync_directory(path: &Path) -> Result<(), PathError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error(path, error))
}

#[derive(Clone, Copy)]
enum ManagedType {
    Directory,
    RegularFile,
}

fn validate_node(path: &Path, uid: u32, expected_type: ManagedType) -> Result<(), PathError> {
    ensure_absolute(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(PathError::Symlink(path.to_path_buf()));
    }
    let correct_type = match expected_type {
        ManagedType::Directory => metadata.is_dir(),
        ManagedType::RegularFile => metadata.is_file(),
    };
    if !correct_type {
        return Err(PathError::WrongType(path.to_path_buf()));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if metadata.uid() != uid {
            return Err(PathError::WrongOwner {
                path: path.to_path_buf(),
                expected: uid,
                actual: metadata.uid(),
            });
        }
        let actual = metadata.mode() & 0o777;
        let expected = match expected_type {
            ManagedType::Directory => DIRECTORY_MODE,
            ManagedType::RegularFile => FILE_MODE,
        };
        let valid = match expected_type {
            ManagedType::Directory => actual == expected,
            ManagedType::RegularFile => actual & !expected == 0,
        };
        if !valid {
            return Err(PathError::WrongMode {
                path: path.to_path_buf(),
                expected,
                actual,
            });
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = uid;
        Err(PathError::UnsupportedPlatform)
    }
}

fn ensure_absolute(path: &Path) -> Result<(), PathError> {
    if path.is_absolute() {
        Ok(())
    } else {
        Err(PathError::NotAbsolute(path.to_path_buf()))
    }
}

fn runtime_candidate(uid: u32) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    let candidate = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let candidate = std::env::var_os("TMPDIR").map(PathBuf::from);
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    let candidate: Option<PathBuf> = None;

    candidate.and_then(|base| {
        if validate_runtime_base(&base, uid) {
            #[cfg(target_os = "linux")]
            let runtime = base.join("zterm");
            #[cfg(not(target_os = "linux"))]
            let runtime = base.join(format!("zterm-{uid}"));
            let socket = runtime.join("daemon.sock");
            (socket_path_bytes(&socket) <= MAX_SOCKET_PATH_BYTES).then_some(runtime)
        } else {
            None
        }
    })
}

fn runtime_fallback(uid: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/zterm-{uid}"))
}

fn validate_runtime_base(path: &Path, uid: u32) -> bool {
    #[cfg(unix)]
    {
        use nix::fcntl::AtFlags;
        use nix::unistd::{AccessFlags, faccessat};
        use std::os::unix::fs::MetadataExt;
        path.is_absolute()
            && fs::symlink_metadata(path).is_ok_and(|metadata| {
                metadata.is_dir()
                    && !metadata.file_type().is_symlink()
                    && metadata.uid() == uid
                    && metadata.mode() & 0o077 == 0
            })
            && faccessat(
                None,
                path,
                AccessFlags::W_OK | AccessFlags::X_OK,
                AtFlags::AT_EACCESS,
            )
            .is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = (path, uid);
        false
    }
}

fn socket_path_bytes(path: &Path) -> usize {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        path.as_os_str().as_bytes().len()
    }
    #[cfg(not(unix))]
    {
        path.as_os_str().to_string_lossy().len()
    }
}

fn io_error(path: &Path, error: impl fmt::Display) -> PathError {
    PathError::Io {
        path: path.to_path_buf(),
        detail: error.to_string(),
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{Read, Write};

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
        (temporary, paths)
    }

    #[cfg(unix)]
    #[test]
    fn directories_files_atomic_failure_and_locks_obey_the_user_boundary() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};

        let (_temporary, paths) = isolated_paths();
        paths.prepare_state_directories().expect("state dirs");
        paths.prepare_runtime_directory().expect("runtime dir");
        assert_eq!(
            fs::metadata(paths.state_root())
                .expect("root metadata")
                .mode()
                & 0o777,
            0o700
        );

        atomic_write(paths.config(), paths.uid(), |file| file.write_all(b"old"))
            .expect("initial file");
        let failure = atomic_write(paths.config(), paths.uid(), |file| {
            file.write_all(b"partial")?;
            Err(io::Error::other("injected writer failure"))
        });
        assert!(failure.is_err());
        let mut contents = String::new();
        File::open(paths.config())
            .expect("config opens")
            .read_to_string(&mut contents)
            .expect("config reads");
        assert_eq!(contents, "old");
        assert_eq!(
            fs::metadata(paths.config()).expect("file metadata").mode() & 0o777,
            0o600
        );

        let lock = FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
            .expect("first lock probe")
            .expect("first lock acquired");
        assert!(
            FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
                .expect("second lock probe")
                .is_none()
        );
        drop(lock);
        assert!(
            FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
                .expect("released lock probe")
                .is_some()
        );
        assert_eq!(
            inspect_existing_lock(paths.lifecycle_lock(), paths.uid())
                .expect("existing lock inspection"),
            ExistingLockState::Unlocked
        );

        atomic_create(paths.identity(), paths.uid(), |file| {
            file.write_all(b"original")
        })
        .expect("create-only file");
        assert!(
            atomic_create(paths.identity(), paths.uid(), |file| {
                file.write_all(b"replacement")
            })
            .is_err()
        );
        assert_eq!(
            fs::read(paths.identity()).expect("create-only contents"),
            b"original"
        );

        fs::set_permissions(paths.config(), fs::Permissions::from_mode(0o644)).expect("widen mode");
        assert!(matches!(
            validate_regular_file(paths.config(), paths.uid()),
            Err(PathError::WrongMode { .. })
        ));
    }

    #[cfg(unix)]
    #[test]
    fn managed_symlinks_are_rejected() {
        use std::os::unix::fs::symlink;
        let (_temporary, paths) = isolated_paths();
        paths.prepare_state_directories().expect("state dirs");
        let target = paths.state_root().join("target");
        File::create(&target).expect("target");
        let link = paths.state_root().join("link");
        symlink(&target, &link).expect("symlink");
        assert!(matches!(
            validate_regular_file(&link, paths.uid()),
            Err(PathError::Symlink(_))
        ));

        let directory_target = paths.home().join("directory-target");
        fs::create_dir(&directory_target).expect("directory target");
        let symlink_root = paths.home().join("symlink-state");
        symlink(&directory_target, &symlink_root).expect("directory symlink");
        let symlink_paths = UserPaths::for_test(
            paths.uid(),
            paths.home().to_path_buf(),
            symlink_root,
            paths.runtime_dir().to_path_buf(),
        );
        assert!(matches!(
            symlink_paths.prepare_state_directories(),
            Err(PathError::Symlink(_))
        ));
    }

    #[cfg(unix)]
    #[test]
    fn runtime_fallback_and_socket_length_are_explicit() {
        use nix::unistd::Uid;
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().expect("temporary root");
        let uid = Uid::effective().as_raw();
        assert_eq!(
            runtime_fallback(uid),
            PathBuf::from(format!("/tmp/zterm-{uid}"))
        );

        let home = temporary.path().join("home");
        fs::create_dir(&home).expect("test home");
        let candidate = temporary.path().join("candidate");
        fs::create_dir(&candidate).expect("runtime candidate");
        fs::set_permissions(&candidate, fs::Permissions::from_mode(0o700))
            .expect("secure runtime candidate mode");
        assert!(validate_runtime_base(&candidate, uid));
        if uid != 0 {
            fs::set_permissions(&candidate, fs::Permissions::from_mode(0o500))
                .expect("unwritable runtime candidate mode");
            assert!(!validate_runtime_base(&candidate, uid));
        }

        let long_runtime = temporary.path().join("r".repeat(MAX_SOCKET_PATH_BYTES));
        let paths = UserPaths::for_test(uid, home.clone(), home.join(".zterm"), long_runtime);
        assert!(matches!(
            paths.prepare_runtime_directory(),
            Err(PathError::SocketPathTooLong(_))
        ));
        assert!(!paths.runtime_dir().exists());
    }
}
