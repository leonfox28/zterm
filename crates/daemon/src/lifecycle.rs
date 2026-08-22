//! Per-user daemon singleflight launch, detached entry, and graceful cleanup.

#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::path::Path;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

use zterm_core::DomainErrorKind;
use zterm_platform::account::EffectiveAccount;
use zterm_platform::user_state::UserPaths;
#[cfg(unix)]
use zterm_platform::user_state::{FileLock, validate_regular_file};

use crate::error::DaemonError;
use crate::local_ipc::LocalClient;
use crate::service::DaemonReadiness;
#[cfg(unix)]
use crate::service::DaemonService;

/// Hidden product argument used only to enter the detached daemon child.
pub const INTERNAL_DAEMON_ARGUMENT: &str = "--internal-daemon";

#[cfg(unix)]
const START_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(unix)]
const LOCK_POLL: Duration = Duration::from_millis(20);
const READINESS_PROBE_TIMEOUT: Duration = Duration::from_millis(250);
#[cfg(unix)]
const LOG_ROTATE_BYTES: u64 = 4 * 1024 * 1024;

/// Executable and one hidden argument used by explicit lifecycle operations.
#[derive(Clone, Debug)]
pub struct DaemonLauncher {
    executable: std::path::PathBuf,
    internal_argument: String,
}

impl DaemonLauncher {
    /// Resolves the current zterm executable and product hidden argument.
    pub fn current() -> Result<Self, DaemonError> {
        let executable = std::env::current_exe().map_err(|error| {
            DaemonError::new(
                DomainErrorKind::DaemonStartTimeout,
                format!("unable to locate current zterm executable: {error}"),
            )
        })?;
        Ok(Self {
            executable,
            internal_argument: INTERNAL_DAEMON_ARGUMENT.to_owned(),
        })
    }

    /// Creates an explicit launcher for task-private multi-process tests.
    #[doc(hidden)]
    #[must_use]
    pub fn for_test(executable: std::path::PathBuf, internal_argument: String) -> Self {
        Self {
            executable,
            internal_argument,
        }
    }

    /// Ensures one daemon using this launch target.
    #[cfg(unix)]
    pub async fn ensure(&self, paths: &UserPaths) -> Result<DaemonReadiness, DaemonError> {
        ensure_daemon_with(paths, &self.executable, &self.internal_argument).await
    }

    /// Returns the M3 platform limitation on non-Unix targets.
    #[cfg(not(unix))]
    pub async fn ensure(&self, _paths: &UserPaths) -> Result<DaemonReadiness, DaemonError> {
        let _ = (&self.executable, &self.internal_argument);
        Err(unsupported())
    }
}

/// Resolves product state from the effective account database.
pub fn production_user_paths() -> Result<UserPaths, DaemonError> {
    let account = EffectiveAccount::current().map_err(|error| {
        DaemonError::new(DomainErrorKind::UnsupportedPlatform, error.to_string())
    })?;
    Ok(UserPaths::for_account(&account))
}

/// Returns readiness, starting at most one detached daemon when stopped.
#[cfg(unix)]
pub async fn ensure_current_daemon(paths: &UserPaths) -> Result<DaemonReadiness, DaemonError> {
    DaemonLauncher::current()?.ensure(paths).await
}

/// Singleflight launcher with an explicit executable/argument for isolated harnesses.
#[cfg(unix)]
#[doc(hidden)]
pub async fn ensure_daemon_with(
    paths: &UserPaths,
    executable: &Path,
    internal_argument: &str,
) -> Result<DaemonReadiness, DaemonError> {
    if let Some(readiness) = probe_readiness(paths).await? {
        return Ok(readiness);
    }
    paths
        .prepare_state_directories()
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
    let started = Instant::now();
    let _lifecycle = acquire_lifecycle_lock(paths, started).await?;
    if let Some(readiness) = probe_readiness(paths).await? {
        return Ok(readiness);
    }

    rotate_lifecycle_log(paths)?;
    let mut command =
        zterm_platform::local_unix::detached_command(executable, paths, internal_argument)
            .map_err(platform_error)?;
    let mut child = command.spawn().map_err(|error| {
        DaemonError::new(
            DomainErrorKind::DaemonStartTimeout,
            format!("unable to spawn detached daemon: {error}"),
        )
    })?;

    loop {
        if let Some(readiness) = probe_readiness(paths).await? {
            return Ok(readiness);
        }
        if let Some(status) = child.try_wait().map_err(|error| {
            DaemonError::new(
                DomainErrorKind::DaemonStartTimeout,
                format!("unable to inspect daemon child: {error}"),
            )
        })? {
            return Err(DaemonError::new(
                DomainErrorKind::DaemonStartTimeout,
                format!("daemon child exited before readiness with {status}"),
            ));
        }
        if started.elapsed() >= START_TIMEOUT {
            return Err(DaemonError::new(
                DomainErrorKind::DaemonStartTimeout,
                "detached daemon did not become ready within 5 seconds",
            ));
        }
        tokio::time::sleep(LOCK_POLL).await;
    }
}

#[cfg(not(unix))]
/// Returns the M3 platform limitation on non-Unix targets.
pub async fn ensure_current_daemon(_paths: &UserPaths) -> Result<DaemonReadiness, DaemonError> {
    Err(unsupported())
}

/// Product hidden entry: detach before runtime initialization, then serve.
pub fn run_internal_daemon() -> Result<(), DaemonError> {
    #[cfg(unix)]
    {
        zterm_platform::local_unix::detach_current_process().map_err(platform_error)?;
        init_lifecycle_logging();
        let paths = production_user_paths()?;
        run_daemon(&paths)
    }
    #[cfg(not(unix))]
    {
        Err(unsupported())
    }
}

/// Runs one already-detached daemon against explicit paths for test harnesses.
#[cfg(unix)]
#[doc(hidden)]
pub fn run_daemon(paths: &UserPaths) -> Result<(), DaemonError> {
    use std::sync::Arc;

    use crate::bootstrap::validate_committed_setup_with_store;
    use crate::store::{StateStore, StoreActor};

    paths
        .prepare_state_directories()
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
    let daemon_lock = zterm_platform::local_unix::DaemonLock::try_acquire(paths)
        .map_err(platform_error)?
        .ok_or_else(|| {
            DaemonError::new(
                DomainErrorKind::DaemonAlreadyRunning,
                "another daemon owns daemon.lock",
            )
        })?;
    let store = StateStore::open(paths)?;
    let setup = validate_committed_setup_with_store(paths, &store)?;
    let listener = zterm_platform::local_unix::bind_daemon_socket(paths, &daemon_lock)
        .map_err(platform_error)?;
    let _store_actor = StoreActor::start(store)?;
    let service = Arc::new(DaemonService::new(setup));
    tracing::info!(socket = %paths.socket().display(), "local daemon ready");

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| {
            DaemonError::new(
                DomainErrorKind::DaemonStartTimeout,
                format!("unable to build daemon runtime: {error}"),
            )
        })?;
    let server_result = runtime.block_on(crate::local_ipc::serve_local(
        listener,
        paths.uid(),
        service,
    ));
    tracing::info!("local daemon stopping");
    let cleanup_result =
        zterm_platform::local_unix::remove_own_socket(paths, &daemon_lock).map_err(platform_error);
    server_result.and(cleanup_result)
}

pub(crate) async fn probe_readiness(
    paths: &UserPaths,
) -> Result<Option<DaemonReadiness>, DaemonError> {
    let client = LocalClient::new(paths.socket());
    match tokio::time::timeout(READINESS_PROBE_TIMEOUT, client.readiness()).await {
        Ok(Ok(readiness)) => Ok(Some(readiness)),
        Ok(Err(error)) if error.kind() == DomainErrorKind::DaemonStopped => Ok(None),
        Ok(Err(error)) => Err(error),
        Err(_) => Err(DaemonError::new(
            DomainErrorKind::DaemonStartTimeout,
            "existing local socket did not answer readiness",
        )),
    }
}

#[cfg(unix)]
pub(crate) async fn acquire_lifecycle_lock(
    paths: &UserPaths,
    started: Instant,
) -> Result<FileLock, DaemonError> {
    loop {
        if let Some(lock) = FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
            .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?
        {
            return Ok(lock);
        }
        if started.elapsed() >= START_TIMEOUT {
            return Err(DaemonError::new(
                DomainErrorKind::DaemonStartTimeout,
                "timed out waiting for daemon launch singleflight",
            ));
        }
        tokio::time::sleep(LOCK_POLL).await;
    }
}

#[cfg(unix)]
fn rotate_lifecycle_log(paths: &UserPaths) -> Result<(), DaemonError> {
    let metadata = match fs::symlink_metadata(paths.daemon_log()) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(DaemonError::new(
                DomainErrorKind::PathUnsafe,
                error.to_string(),
            ));
        }
    };
    validate_regular_file(paths.daemon_log(), paths.uid())
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
    if metadata.len() < LOG_ROTATE_BYTES {
        return Ok(());
    }
    let archive = paths.logs().join("daemon.log.1");
    match fs::symlink_metadata(&archive) {
        Ok(_) => {
            validate_regular_file(&archive, paths.uid()).map_err(|error| {
                DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string())
            })?;
            fs::remove_file(&archive).map_err(|error| {
                DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string())
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(DaemonError::new(
                DomainErrorKind::PathUnsafe,
                error.to_string(),
            ));
        }
    }
    fs::rename(paths.daemon_log(), archive)
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))
}

#[cfg(unix)]
fn init_lifecycle_logging() {
    let _ = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_target(false)
        .try_init();
}

#[cfg(unix)]
fn platform_error(error: zterm_platform::local_unix::LocalPlatformError) -> DaemonError {
    use zterm_platform::local_unix::LocalPlatformError;
    let kind = match error {
        LocalPlatformError::Path(_) | LocalPlatformError::UnsafeSocket(_) => {
            DomainErrorKind::PathUnsafe
        }
        LocalPlatformError::AlreadyRunning => DomainErrorKind::DaemonAlreadyRunning,
        LocalPlatformError::PeerUidMismatch { .. } => DomainErrorKind::PeerUidMismatch,
        LocalPlatformError::UnsupportedPlatform => DomainErrorKind::UnsupportedPlatform,
        LocalPlatformError::Io(_) => DomainErrorKind::DaemonStopped,
    };
    DaemonError::new(kind, error.to_string())
}

#[cfg(not(unix))]
fn unsupported() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::UnsupportedPlatform,
        "local daemon lifecycle is Unix-only in M3",
    )
}
