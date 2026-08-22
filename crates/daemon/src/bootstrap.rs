//! Idempotent setup and partial-state recovery under the lifecycle lock.

use std::fs;
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zterm_core::DomainErrorKind;
use zterm_platform::user_state::{FileLock, UserPaths};

use crate::config::{ValidatedConfig, load_config, validate_setup_input, write_config};
use crate::error::DaemonError;
use crate::identity::DeviceIdentity;
use crate::store::{DeviceMetadata, StateStore};

const LOCK_WAIT: Duration = Duration::from_secs(5);
const LOCK_POLL: Duration = Duration::from_millis(10);

/// Result of a successful setup/validation pass.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BootstrapResult {
    /// Stable public device identity.
    pub device_id: zterm_core::DeviceId,
    /// Iroh canonical public endpoint encoding.
    pub endpoint_id: String,
    /// Committed configuration.
    pub config: ValidatedConfig,
}

/// Creates or validates the complete setup without rotating an existing identity.
pub fn bootstrap(
    paths: &UserPaths,
    requested: &ValidatedConfig,
) -> Result<BootstrapResult, DaemonError> {
    paths
        .prepare_state_directories()
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
    let _lifecycle = acquire_lifecycle_lock(paths)?;
    bootstrap_with_lock_held(paths, requested)
}

/// Runs bootstrap while the caller owns `lifecycle.lock`.
#[doc(hidden)]
pub fn bootstrap_with_lock_held(
    paths: &UserPaths,
    requested: &ValidatedConfig,
) -> Result<BootstrapResult, DaemonError> {
    paths
        .prepare_state_directories()
        .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;

    let key_exists = managed_exists(paths.identity())?;
    let config_exists = managed_exists(paths.config())?;
    let database_exists = managed_exists(paths.database())?;
    if !key_exists && (config_exists || database_exists) {
        return Err(DaemonError::new(
            DomainErrorKind::IdentityInvalid,
            "config/database exists but identity.key is missing; refusing identity rotation",
        ));
    }

    let identity = if key_exists {
        DeviceIdentity::load(paths)?
    } else {
        DeviceIdentity::create(paths)?
    };

    let existing_config = if config_exists {
        let existing = load_config(paths)?;
        if &existing != requested {
            return Err(DaemonError::new(
                DomainErrorKind::AlreadyConfiguredConflict,
                "requested setup differs from committed config.toml",
            ));
        }
        Some(existing)
    } else {
        None
    };

    let mut store = StateStore::open(paths)?;
    let existing_metadata = store.metadata()?;
    if existing_metadata
        .as_ref()
        .is_some_and(|metadata| metadata.device_id != identity.device_id())
    {
        return Err(DaemonError::new(
            DomainErrorKind::IdentityStateMismatch,
            "database device_id does not match identity.key",
        ));
    }
    let committed_config = match (existing_config, existing_metadata.as_ref()) {
        (Some(config), _) => config,
        (None, Some(metadata)) => {
            validate_setup_input(&metadata.device_name, requested.infrastructure.clone())?
        }
        (None, None) => requested.clone(),
    };
    let created_at_unix =
        existing_metadata.map_or_else(now_unix, |metadata| metadata.created_at_unix);
    store.ensure_metadata(&DeviceMetadata {
        device_id: identity.device_id(),
        device_name: committed_config.device_name.clone(),
        created_at_unix,
    })?;

    if !config_exists {
        write_config(paths, &committed_config)?;
    }

    Ok(BootstrapResult {
        device_id: identity.device_id(),
        endpoint_id: identity.endpoint_id(),
        config: committed_config,
    })
}

/// Validates all committed setup state without creating missing nodes.
pub fn validate_committed_setup(paths: &UserPaths) -> Result<BootstrapResult, DaemonError> {
    validate_committed_files_exist(paths)?;
    let store = StateStore::open_read_only(paths)?;
    validate_committed_setup_with_store(paths, &store)
}

/// Validates committed setup against a caller-owned SQLite connection.
///
/// The running daemon uses this after acquiring `daemon.lock`, so no second
/// SQLite connection survives beside its `StoreActor`.
#[doc(hidden)]
pub fn validate_committed_setup_with_store(
    paths: &UserPaths,
    store: &StateStore,
) -> Result<BootstrapResult, DaemonError> {
    validate_committed_files_exist(paths)?;
    let identity = DeviceIdentity::load(paths)?;
    let config = load_config(paths)?;
    let metadata = store.metadata()?.ok_or_else(|| {
        DaemonError::new(
            DomainErrorKind::IdentityStateMismatch,
            "database metadata row is missing",
        )
    })?;
    if metadata.device_id != identity.device_id() {
        return Err(DaemonError::new(
            DomainErrorKind::IdentityStateMismatch,
            "database device_id does not match identity.key",
        ));
    }
    if metadata.device_name != config.device_name {
        return Err(DaemonError::new(
            DomainErrorKind::IdentityStateMismatch,
            "database device name does not match config.toml",
        ));
    }
    Ok(BootstrapResult {
        device_id: identity.device_id(),
        endpoint_id: identity.endpoint_id(),
        config,
    })
}

fn validate_committed_files_exist(paths: &UserPaths) -> Result<(), DaemonError> {
    if !managed_exists(paths.identity())?
        || !managed_exists(paths.config())?
        || !managed_exists(paths.database())?
    {
        return Err(DaemonError::new(
            DomainErrorKind::NotSetup,
            "identity, config, or state database is missing",
        ));
    }
    Ok(())
}

fn acquire_lifecycle_lock(paths: &UserPaths) -> Result<FileLock, DaemonError> {
    let started = std::time::Instant::now();
    loop {
        if let Some(lock) = FileLock::try_acquire(paths.lifecycle_lock(), paths.uid())
            .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?
        {
            return Ok(lock);
        }
        if started.elapsed() >= LOCK_WAIT {
            return Err(DaemonError::new(
                DomainErrorKind::DeadlineExceeded,
                "timed out waiting for setup lifecycle lock",
            ));
        }
        thread::sleep(LOCK_POLL);
    }
}

fn managed_exists(path: &std::path::Path) -> Result<bool, DaemonError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(DaemonError::new(
            DomainErrorKind::PathUnsafe,
            format!("managed path is a symlink: {}", path.display()),
        )),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(DaemonError::new(
            DomainErrorKind::PathUnsafe,
            error.to_string(),
        )),
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_secs()).unwrap_or(i64::MAX)
        })
}
