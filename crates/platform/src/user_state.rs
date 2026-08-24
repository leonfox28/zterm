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
#[cfg(unix)]
const EXECUTABLE_MODE: u32 = 0o700;
const MAX_SOCKET_PATH_BYTES: usize = 100;
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

/// All persistent and runtime paths owned by one effective user.
#[derive(Clone, Eq, PartialEq)]
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

impl fmt::Debug for UserPaths {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UserPaths")
            .field("uid", &self.uid)
            .field("managed_paths", &"[REDACTED]")
            .field("managed_path_count", &13)
            .finish_non_exhaustive()
    }
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
    /// A state-root child is not part of the fixed managed inventory.
    UnexpectedManagedEntry(PathBuf),
    /// Destructive state removal did not receive the exact lifecycle lock.
    LifecycleLockMismatch {
        /// Required lifecycle lock path.
        expected: PathBuf,
        /// Lock path actually held by the caller.
        actual: PathBuf,
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
            Self::UnexpectedManagedEntry(path) => write!(
                formatter,
                "managed state contains an unexpected entry: {}",
                path.display()
            ),
            Self::LifecycleLockMismatch { expected, actual } => write!(
                formatter,
                "managed state removal requires lifecycle lock {}, got {}",
                expected.display(),
                actual.display()
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

/// One activated executable with its exact same-directory rollback owner.
pub struct ExecutableActivation {
    target: PathBuf,
    backup: PathBuf,
}

impl fmt::Debug for ExecutableActivation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecutableActivation")
            .field("paths", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

impl ExecutableActivation {
    /// Removes only the exact retained rollback file after post-activation checks pass.
    pub fn commit(self) -> Result<(), PathError> {
        let parent = self
            .target
            .parent()
            .ok_or_else(|| PathError::NotAbsolute(self.target.clone()))?;
        fs::remove_file(&self.backup).map_err(|error| io_error(&self.backup, error))?;
        sync_directory(parent)
    }

    /// Restores the retained executable and removes the failed activated candidate.
    pub fn rollback(self) -> Result<(), PathError> {
        let parent = self
            .target
            .parent()
            .ok_or_else(|| PathError::NotAbsolute(self.target.clone()))?;
        fs::rename(&self.backup, &self.target).map_err(|error| io_error(&self.target, error))?;
        sync_directory(parent)
    }
}

/// Validates a user-owned direct executable without following a symlink.
pub fn validate_owned_executable(path: &Path, uid: u32) -> Result<(), PathError> {
    ensure_absolute(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| PathError::NotAbsolute(path.to_path_buf()))?;
    validate_executable_parent(parent, uid)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(PathError::Symlink(path.to_path_buf()));
    }
    if !metadata.is_file() {
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
        if actual & 0o100 == 0 || actual & 0o022 != 0 {
            return Err(PathError::WrongMode {
                path: path.to_path_buf(),
                expected: EXECUTABLE_MODE,
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

/// Stages and atomically activates one executable while retaining the old binary.
pub fn activate_executable(
    target: &Path,
    uid: u32,
    writer: impl FnOnce(&mut File) -> io::Result<()>,
) -> Result<ExecutableActivation, PathError> {
    validate_owned_executable(target, uid)?;
    let parent = target
        .parent()
        .ok_or_else(|| PathError::NotAbsolute(target.to_path_buf()))?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let staged = parent.join(format!(".zterm.update-{}-{sequence}", std::process::id()));
    let backup = parent.join(format!(".zterm.rollback-{}-{sequence}", std::process::id()));
    let mut file = create_new_executable(&staged)?;
    let result = writer(&mut file)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error(&staged, error));
    if let Err(error) = result {
        drop(file);
        let _ = fs::remove_file(&staged);
        return Err(error);
    }
    drop(file);

    fs::hard_link(target, &backup).map_err(|error| {
        let _ = fs::remove_file(&staged);
        io_error(target, error)
    })?;
    if let Err(error) = fs::rename(&staged, target) {
        let _ = fs::remove_file(&backup);
        let _ = fs::remove_file(&staged);
        return Err(io_error(target, error));
    }
    let activation = ExecutableActivation {
        target: target.to_path_buf(),
        backup,
    };
    if let Err(error) = sync_directory(parent) {
        return match activation.rollback() {
            Ok(()) => Err(error),
            Err(rollback) => Err(rollback),
        };
    }
    Ok(activation)
}

/// Installs one validated executable with an atomic no-clobber link publication.
pub fn install_executable(source: &Path, target: &Path, uid: u32) -> Result<(), PathError> {
    validate_owned_executable(source, uid)?;
    ensure_absolute(target)?;
    let parent = target
        .parent()
        .ok_or_else(|| PathError::NotAbsolute(target.to_path_buf()))?;
    validate_executable_parent(parent, uid)?;
    if fs::symlink_metadata(target).is_ok() {
        return Err(PathError::Io {
            path: target.to_path_buf(),
            detail: "installation destination already exists".to_owned(),
        });
    }

    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let staged = parent.join(format!(".zterm.install-{}-{sequence}", std::process::id()));
    let mut input = File::open(source).map_err(|error| io_error(source, error))?;
    let mut output = create_new_executable(&staged)?;
    let result = io::copy(&mut input, &mut output)
        .and_then(|_| output.sync_all())
        .map_err(|error| io_error(&staged, error));
    if let Err(error) = result {
        drop(output);
        let _ = fs::remove_file(&staged);
        return Err(error);
    }
    drop(output);
    if let Err(error) = fs::hard_link(&staged, target) {
        let _ = fs::remove_file(&staged);
        return Err(io_error(target, error));
    }
    fs::remove_file(&staged).map_err(|error| io_error(&staged, error))?;
    sync_directory(parent)
}

/// Removes one validated user-owned executable and syncs its directory.
pub fn remove_owned_executable(path: &Path, uid: u32) -> Result<(), PathError> {
    validate_owned_executable(path, uid)?;
    let parent = path
        .parent()
        .ok_or_else(|| PathError::NotAbsolute(path.to_path_buf()))?;
    fs::remove_file(path).map_err(|error| io_error(path, error))?;
    sync_directory(parent)
}

/// Removes the exact managed state root while the caller holds its lifecycle lock.
///
/// The complete tree is validated before the first unlink. Unknown entries,
/// symlinks, unexpected directories, wrong ownership, or unsafe modes therefore
/// fail without deleting any committed state. Each unlink targets either a
/// validated regular file or an empty validated directory and never follows a
/// symbolic link. A missing root is a retry-safe success.
pub fn remove_managed_state_root(
    paths: &UserPaths,
    lifecycle_lock: &FileLock,
) -> Result<(), PathError> {
    remove_managed_state_root_with(paths, lifecycle_lock, remove_validated_file)
}

fn remove_managed_state_root_with(
    paths: &UserPaths,
    lifecycle_lock: &FileLock,
    mut remove_file: impl FnMut(&Path) -> Result<(), PathError>,
) -> Result<(), PathError> {
    if lifecycle_lock.path() != paths.lifecycle_lock() {
        return Err(PathError::LifecycleLockMismatch {
            expected: paths.lifecycle_lock().to_path_buf(),
            actual: lifecycle_lock.path().to_path_buf(),
        });
    }
    match fs::symlink_metadata(paths.state_root()) {
        Ok(_) => validate_directory(paths.state_root(), paths.uid())?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(io_error(paths.state_root(), error)),
    }

    let mut files = Vec::new();
    let mut directories = Vec::new();
    let entries =
        fs::read_dir(paths.state_root()).map_err(|error| io_error(paths.state_root(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_error(paths.state_root(), error))?;
        let path = entry.path();
        if path == paths.logs() {
            validate_directory(&path, paths.uid())?;
            inspect_managed_logs(paths, &mut files)?;
            directories.push(path);
        } else if is_managed_state_file(paths, &path) || is_managed_temporary_file(paths, &path) {
            validate_regular_file(&path, paths.uid())?;
            files.push(path);
        } else {
            return Err(PathError::UnexpectedManagedEntry(path));
        }
    }

    files.sort();
    let lifecycle_position = files
        .iter()
        .position(|path| path == paths.lifecycle_lock())
        .map(|position| files.remove(position));
    for path in files {
        remove_file(&path)?;
    }
    for path in directories {
        fs::remove_dir(&path).map_err(|error| io_error(&path, error))?;
    }
    if let Some(path) = lifecycle_position {
        remove_file(&path)?;
    }
    fs::remove_dir(paths.state_root()).map_err(|error| io_error(paths.state_root(), error))?;
    let parent = paths
        .state_root()
        .parent()
        .ok_or_else(|| PathError::NotAbsolute(paths.state_root().to_path_buf()))?;
    sync_directory(parent)
}

fn inspect_managed_logs(paths: &UserPaths, files: &mut Vec<PathBuf>) -> Result<(), PathError> {
    let archive = paths.logs().join("daemon.log.1");
    let entries = fs::read_dir(paths.logs()).map_err(|error| io_error(paths.logs(), error))?;
    for entry in entries {
        let entry = entry.map_err(|error| io_error(paths.logs(), error))?;
        let path = entry.path();
        if path != paths.daemon_log() && path != archive {
            return Err(PathError::UnexpectedManagedEntry(path));
        }
        validate_regular_file(&path, paths.uid())?;
        files.push(path);
    }
    Ok(())
}

fn is_managed_state_file(paths: &UserPaths, path: &Path) -> bool {
    let journal = database_journal_path(paths.database());
    [
        paths.config(),
        paths.identity(),
        paths.database(),
        paths.install_metadata(),
        paths.lifecycle_lock(),
        paths.daemon_lock(),
        journal.as_path(),
    ]
    .contains(&path)
}

fn database_journal_path(database: &Path) -> PathBuf {
    let mut journal = database.as_os_str().to_os_string();
    journal.push("-journal");
    PathBuf::from(journal)
}

fn is_managed_temporary_file(paths: &UserPaths, path: &Path) -> bool {
    [
        paths.config(),
        paths.identity(),
        paths.database(),
        paths.install_metadata(),
    ]
    .into_iter()
    .any(|managed| temporary_name_matches(managed, path))
}

fn temporary_name_matches(managed: &Path, candidate: &Path) -> bool {
    if managed.parent() != candidate.parent() {
        return false;
    }
    let Some(managed_name) = managed.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let Some(candidate_name) = candidate.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    let prefix = format!(".{managed_name}.tmp-");
    let Some(suffix) = candidate_name.strip_prefix(&prefix) else {
        return false;
    };
    let Some((process, sequence)) = suffix.split_once('-') else {
        return false;
    };
    !process.is_empty()
        && !sequence.is_empty()
        && process.bytes().all(|byte| byte.is_ascii_digit())
        && sequence.bytes().all(|byte| byte.is_ascii_digit())
}

fn remove_validated_file(path: &Path) -> Result<(), PathError> {
    fs::remove_file(path).map_err(|error| io_error(path, error))
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

fn create_new_executable(path: &Path) -> Result<File, PathError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(EXECUTABLE_MODE)
            .custom_flags(nix::libc::O_NOFOLLOW);
    }
    options.open(path).map_err(|error| io_error(path, error))
}

fn validate_executable_parent(path: &Path, uid: u32) -> Result<(), PathError> {
    ensure_absolute(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| io_error(path, error))?;
    if metadata.file_type().is_symlink() {
        return Err(PathError::Symlink(path.to_path_buf()));
    }
    if !metadata.is_dir() {
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
        if actual & 0o022 != 0 {
            return Err(PathError::WrongMode {
                path: path.to_path_buf(),
                expected: 0o755,
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

    #[test]
    fn user_paths_debug_redacts_every_managed_path_without_changing_accessors() {
        const HOME_SENTINEL: &str = "/private/tmp/USER_PATH_HOME_SENTINEL_f31d";
        const STATE_SENTINEL: &str = "/private/tmp/USER_PATH_STATE_SENTINEL_72a9";
        const RUNTIME_SENTINEL: &str = "/private/tmp/USER_PATH_RUNTIME_SENTINEL_c604";
        let paths = UserPaths::for_test(
            1_234,
            HOME_SENTINEL.into(),
            STATE_SENTINEL.into(),
            RUNTIME_SENTINEL.into(),
        );
        let rendered = format!("{paths:?}");

        for sentinel in [HOME_SENTINEL, STATE_SENTINEL, RUNTIME_SENTINEL] {
            assert!(!rendered.contains(sentinel));
        }
        assert!(rendered.contains("uid: 1234"));
        assert!(rendered.contains("managed_paths: \"[REDACTED]\""));
        assert!(rendered.contains("managed_path_count: 13"));
        assert_eq!(paths.home(), Path::new(HOME_SENTINEL));
        assert_eq!(paths.state_root(), Path::new(STATE_SENTINEL));
        assert_eq!(paths.runtime_dir(), Path::new(RUNTIME_SENTINEL));
        assert_eq!(paths, paths.clone());
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

    #[test]
    fn executable_activation_restores_old_binary_after_postcheck_failure_or_commits() {
        use nix::unistd::Uid;
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().expect("temporary executable root");
        let directory = temporary.path().join("bin");
        fs::create_dir(&directory).expect("binary directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))
            .expect("binary directory mode");
        let target = directory.join("zterm");
        fs::write(&target, b"old").expect("old executable");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700))
            .expect("old executable mode");
        let uid = Uid::effective().as_raw();

        let activation = activate_executable(&target, uid, |file| file.write_all(b"new"))
            .expect("activate candidate");
        assert_eq!(fs::read(&target).expect("active candidate"), b"new");
        // The update owner calls rollback when its post-activation self-check fails.
        activation.rollback().expect("rollback candidate");
        assert_eq!(fs::read(&target).expect("restored binary"), b"old");

        let activation = activate_executable(&target, uid, |file| file.write_all(b"final"))
            .expect("activate final candidate");
        activation.commit().expect("commit candidate");
        assert_eq!(fs::read(&target).expect("committed binary"), b"final");
        assert!(
            fs::read_dir(&directory)
                .expect("binary inventory")
                .all(|entry| entry.expect("directory entry").path() == target)
        );
    }

    #[test]
    fn executable_activation_rejects_symlink_and_group_writable_targets() {
        use nix::unistd::Uid;
        use std::os::unix::fs::{PermissionsExt, symlink};

        let temporary = tempfile::tempdir().expect("temporary executable root");
        let directory = temporary.path().join("bin");
        fs::create_dir(&directory).expect("binary directory");
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o755))
            .expect("binary directory mode");
        let target = directory.join("real");
        fs::write(&target, b"old").expect("old executable");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o720))
            .expect("unsafe executable mode");
        let uid = Uid::effective().as_raw();
        assert!(matches!(
            validate_owned_executable(&target, uid),
            Err(PathError::WrongMode { .. })
        ));

        fs::set_permissions(&target, fs::Permissions::from_mode(0o700))
            .expect("safe executable mode");
        let link = directory.join("zterm");
        symlink(&target, &link).expect("executable symlink");
        assert!(matches!(
            validate_owned_executable(&link, uid),
            Err(PathError::Symlink(path)) if path == link
        ));
    }

    #[test]
    fn executable_install_is_atomic_no_clobber() {
        use nix::unistd::Uid;
        use std::os::unix::fs::PermissionsExt;

        let temporary = tempfile::tempdir().expect("temporary executable root");
        let home = temporary.path().join("home");
        let source_directory = temporary.path().join("source");
        let destination_directory = home.join("bin");
        fs::create_dir(&home).expect("test home");
        fs::create_dir(&source_directory).expect("source directory");
        fs::create_dir(&destination_directory).expect("destination directory");
        fs::set_permissions(&source_directory, fs::Permissions::from_mode(0o700))
            .expect("source directory mode");
        fs::set_permissions(&destination_directory, fs::Permissions::from_mode(0o755))
            .expect("destination directory mode");
        let source = source_directory.join("zterm");
        fs::write(&source, b"candidate").expect("candidate bytes");
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700)).expect("candidate mode");
        let target = destination_directory.join("zterm");
        let uid = Uid::effective().as_raw();

        install_executable(&source, &target, uid).expect("first install");
        assert_eq!(fs::read(&target).expect("installed bytes"), b"candidate");
        assert!(
            !home.join(".zterm").exists(),
            "binary install must not create product state"
        );
        assert!(install_executable(&source, &target, uid).is_err());
        assert_eq!(fs::read(&target).expect("unclobbered bytes"), b"candidate");
        assert!(!home.join(".zterm").exists());
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

    #[test]
    fn managed_state_removal_is_inventory_bounded_and_retry_safe() {
        let (_temporary, paths) = isolated_paths();
        paths.prepare_state_directories().expect("state dirs");
        let journal = database_journal_path(paths.database());
        for path in [
            paths.config().to_path_buf(),
            paths.identity().to_path_buf(),
            paths.database().to_path_buf(),
            paths.install_metadata().to_path_buf(),
            journal,
        ] {
            atomic_write(&path, paths.uid(), |file| file.write_all(b"managed"))
                .expect("managed file");
        }
        for managed in [
            paths.config(),
            paths.identity(),
            paths.database(),
            paths.install_metadata(),
        ] {
            let residue = paths.state_root().join(format!(
                ".{}.tmp-123-456",
                managed
                    .file_name()
                    .expect("managed file name")
                    .to_string_lossy()
            ));
            atomic_write(&residue, paths.uid(), |file| file.write_all(b"residue"))
                .expect("managed temporary residue");
        }
        let mut log = open_append(paths.daemon_log(), paths.uid()).expect("managed log");
        log.write_all(b"safe diagnostic\n").expect("log bytes");
        drop(log);
        let mut archive = open_append(&paths.logs().join("daemon.log.1"), paths.uid())
            .expect("managed log archive");
        archive
            .write_all(b"older diagnostic\n")
            .expect("archive bytes");
        drop(archive);
        drop(
            FileLock::try_acquire(paths.daemon_lock(), paths.uid())
                .expect("daemon lock probe")
                .expect("daemon lock"),
        );
        let lifecycle = FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
            .expect("lifecycle lock probe")
            .expect("lifecycle lock");

        let mut removal_order = Vec::new();
        remove_managed_state_root_with(&paths, &lifecycle, |path| {
            removal_order.push(path.to_path_buf());
            remove_validated_file(path)
        })
        .expect("managed state removal");
        assert_eq!(
            removal_order.last().map(PathBuf::as_path),
            Some(paths.lifecycle_lock()),
            "the held lifecycle lock is always the final unlinked file"
        );
        assert!(!paths.state_root().exists());
        remove_managed_state_root(&paths, &lifecycle).expect("retry after complete removal");
    }

    #[test]
    fn managed_state_removal_retries_after_identity_and_config_are_partially_removed() {
        let (_temporary, paths) = isolated_paths();
        paths.prepare_state_directories().expect("state dirs");
        for path in [paths.config(), paths.identity(), paths.database()] {
            atomic_write(path, paths.uid(), |file| file.write_all(b"managed"))
                .expect("managed file");
        }
        let lifecycle = FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
            .expect("lifecycle lock probe")
            .expect("lifecycle lock");
        let mut identity_removed = false;

        let error = remove_managed_state_root_with(&paths, &lifecycle, |path| {
            if identity_removed {
                return Err(PathError::Io {
                    path: path.to_path_buf(),
                    detail: "injected partial-removal failure".to_owned(),
                });
            }
            remove_validated_file(path)?;
            if path == paths.identity() {
                identity_removed = true;
            }
            Ok(())
        })
        .expect_err("direct removal fault");
        assert!(matches!(error, PathError::Io { .. }));
        assert!(!paths.config().exists());
        assert!(!paths.identity().exists());
        assert!(paths.database().exists());
        assert!(paths.state_root().exists());

        remove_managed_state_root(&paths, &lifecycle)
            .expect("retry without intact setup or identity");
        assert!(!paths.state_root().exists());
    }

    #[test]
    fn managed_state_removal_rejects_unknown_type_owner_mode_and_symlink_before_unlink() {
        use std::os::unix::fs::PermissionsExt;
        use std::os::unix::fs::symlink;

        let (_temporary, paths) = isolated_paths();
        paths.prepare_state_directories().expect("state dirs");
        atomic_write(paths.identity(), paths.uid(), |file| {
            file.write_all(b"identity")
        })
        .expect("identity file");
        let unexpected = paths.state_root().join("user-notes");
        atomic_write(&unexpected, paths.uid(), |file| file.write_all(b"keep me"))
            .expect("unexpected regular file");
        let lifecycle = FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
            .expect("lifecycle lock probe")
            .expect("lifecycle lock");
        assert!(matches!(
            remove_managed_state_root(&paths, &lifecycle),
            Err(PathError::UnexpectedManagedEntry(path)) if path == unexpected
        ));
        assert!(
            paths.identity().exists(),
            "validation precedes every unlink"
        );
        assert!(unexpected.exists());
        drop(lifecycle);

        fs::remove_file(&unexpected).expect("remove unexpected fixture");
        let outside = paths.home().join("outside.log");
        fs::write(&outside, b"outside").expect("outside target");
        symlink(&outside, paths.daemon_log()).expect("unsafe log symlink");
        let lifecycle = FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
            .expect("lifecycle lock probe")
            .expect("lifecycle lock");
        assert!(matches!(
            remove_managed_state_root(&paths, &lifecycle),
            Err(PathError::Symlink(path)) if path == paths.daemon_log()
        ));
        assert_eq!(fs::read(&outside).expect("outside bytes"), b"outside");
        assert!(paths.identity().exists(), "unsafe tree remains untouched");
        drop(lifecycle);

        let (_temporary, paths) = isolated_paths();
        paths.prepare_state_directories().expect("state dirs");
        atomic_write(paths.identity(), paths.uid(), |file| {
            file.write_all(b"identity")
        })
        .expect("identity file");
        fs::create_dir(paths.config()).expect("wrong-type managed entry");
        fs::set_permissions(paths.config(), fs::Permissions::from_mode(0o700))
            .expect("wrong-type directory mode");
        let lifecycle = FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
            .expect("lifecycle lock probe")
            .expect("lifecycle lock");
        assert!(matches!(
            remove_managed_state_root(&paths, &lifecycle),
            Err(PathError::WrongType(path)) if path == paths.config()
        ));
        assert!(paths.identity().exists());
        drop(lifecycle);

        let (_temporary, paths) = isolated_paths();
        paths.prepare_state_directories().expect("state dirs");
        atomic_write(paths.identity(), paths.uid(), |file| {
            file.write_all(b"identity")
        })
        .expect("identity file");
        atomic_write(paths.config(), paths.uid(), |file| {
            file.write_all(b"config")
        })
        .expect("config file");
        fs::set_permissions(paths.config(), fs::Permissions::from_mode(0o644))
            .expect("unsafe config mode");
        let lifecycle = FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
            .expect("lifecycle lock probe")
            .expect("lifecycle lock");
        assert!(matches!(
            remove_managed_state_root(&paths, &lifecycle),
            Err(PathError::WrongMode { path, .. }) if path == paths.config()
        ));
        assert!(paths.identity().exists());
        drop(lifecycle);

        let (_temporary, paths) = isolated_paths();
        paths.prepare_state_directories().expect("state dirs");
        atomic_write(paths.identity(), paths.uid(), |file| {
            file.write_all(b"identity")
        })
        .expect("identity file");
        let lifecycle = FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
            .expect("lifecycle lock probe")
            .expect("lifecycle lock");
        let other_uid = paths.uid().checked_add(1).unwrap_or(paths.uid() - 1);
        let wrong_owner = UserPaths::for_test(
            other_uid,
            paths.home().to_path_buf(),
            paths.state_root().to_path_buf(),
            paths.runtime_dir().to_path_buf(),
        );
        assert!(matches!(
            remove_managed_state_root(&wrong_owner, &lifecycle),
            Err(PathError::WrongOwner { path, .. }) if path == paths.state_root()
        ));
        assert!(paths.identity().exists());
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
