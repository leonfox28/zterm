//! Bundled SQLite schema and single-owner store actor.

use std::fs;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags, OptionalExtension, TransactionBehavior, params};
use zterm_core::{DeviceId, DomainErrorKind, STATE_SCHEMA_VERSION};
use zterm_platform::user_state::{UserPaths, open_append, validate_regular_file};

use crate::error::DaemonError;

/// Identity metadata retained in the singleton state row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceMetadata {
    /// Public device ID derived from `identity.key`.
    pub device_id: DeviceId,
    /// User-facing device name.
    pub device_name: String,
    /// Initial setup Unix timestamp.
    pub created_at_unix: i64,
}

/// Authorization status retained as a tombstone after revoke.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(i64)]
pub enum AuthorizationStatus {
    /// Device may authenticate.
    Authorized = 1,
    /// Device was explicitly revoked.
    Revoked = 2,
}

/// Synchronous SQLite owner used during bootstrap and moved into `StoreActor`.
pub struct StateStore {
    connection: Connection,
}

impl StateStore {
    /// Opens, configures, and transactionally migrates the user database.
    pub fn open(paths: &UserPaths) -> Result<Self, DaemonError> {
        if !paths.database().exists() {
            drop(open_append(paths.database(), paths.uid()).map_err(|error| {
                DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string())
            })?);
        }
        validate_regular_file(paths.database(), paths.uid())
            .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let mut connection = open_connection(paths, flags)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(store_error)?;
        reject_too_new_schema(&connection)?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys=ON; PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL;",
            )
            .map_err(store_error)?;
        migrate(&mut connection)?;
        Ok(Self { connection })
    }

    /// Opens committed state without creating files or applying migrations.
    pub fn open_read_only(paths: &UserPaths) -> Result<Self, DaemonError> {
        validate_regular_file(paths.database(), paths.uid())
            .map_err(|error| DaemonError::new(DomainErrorKind::PathUnsafe, error.to_string()))?;
        let flags = OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NOFOLLOW;
        let connection = open_connection(paths, flags)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(store_error)?;
        connection
            .execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(store_error)?;
        require_current_schema(&connection)?;
        Ok(Self { connection })
    }

    /// Returns singleton identity metadata when present.
    pub fn metadata(&self) -> Result<Option<DeviceMetadata>, DaemonError> {
        self.connection
            .query_row(
                "SELECT device_id, device_name, created_at_unix FROM metadata WHERE singleton=1",
                [],
                |row| {
                    let bytes: Vec<u8> = row.get(0)?;
                    let device_id = DeviceId::from_bytes(&bytes).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            bytes.len(),
                            rusqlite::types::Type::Blob,
                            Box::new(error),
                        )
                    })?;
                    Ok(DeviceMetadata {
                        device_id,
                        device_name: row.get(1)?,
                        created_at_unix: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(store_error)
    }

    /// Inserts initial metadata or validates the committed identity/name.
    pub fn ensure_metadata(&mut self, expected: &DeviceMetadata) -> Result<(), DaemonError> {
        match self.metadata()? {
            Some(actual) if actual.device_id != expected.device_id => Err(DaemonError::new(
                DomainErrorKind::IdentityStateMismatch,
                "database device_id does not match identity.key",
            )),
            Some(actual) if actual.device_name != expected.device_name => Err(DaemonError::new(
                DomainErrorKind::AlreadyConfiguredConflict,
                format!(
                    "database device name is {:?}, requested {:?}",
                    actual.device_name, expected.device_name
                ),
            )),
            Some(_) => Ok(()),
            None => {
                self.connection
                    .execute(
                        "INSERT INTO metadata(singleton, device_id, device_name, created_at_unix) VALUES(1, ?1, ?2, ?3)",
                        params![expected.device_id.as_bytes().as_slice(), expected.device_name, expected.created_at_unix],
                    )
                    .map_err(store_error)?;
                Ok(())
            }
        }
    }

    /// Inserts or refreshes an authorized remote endpoint transactionally.
    pub fn authorize_device(
        &mut self,
        endpoint_id: DeviceId,
        display_name: &str,
        now_unix: i64,
    ) -> Result<u64, DaemonError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let generation: Option<i64> = transaction
            .query_row(
                "SELECT generation FROM device_auth WHERE endpoint_id=?1",
                [endpoint_id.as_bytes().as_slice()],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        let generation = generation.unwrap_or(0).saturating_add(1);
        transaction
            .execute(
                "INSERT INTO device_auth(endpoint_id, display_name, status, generation, paired_at_unix, revoked_at_unix, last_seen_at_unix)
                 VALUES(?1, ?2, 1, ?3, ?4, NULL, NULL)
                 ON CONFLICT(endpoint_id) DO UPDATE SET display_name=excluded.display_name, status=1, generation=excluded.generation, revoked_at_unix=NULL",
                params![endpoint_id.as_bytes().as_slice(), display_name, generation, now_unix],
            )
            .map_err(store_error)?;
        transaction.commit().map_err(store_error)?;
        u64::try_from(generation)
            .map_err(|error| DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string()))
    }

    /// Persists a revoked tombstone and advances authorization generation.
    pub fn revoke_device(
        &mut self,
        endpoint_id: DeviceId,
        now_unix: i64,
    ) -> Result<u64, DaemonError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let generation: i64 = transaction
            .query_row(
                "SELECT generation FROM device_auth WHERE endpoint_id=?1",
                [endpoint_id.as_bytes().as_slice()],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        let next = generation.saturating_add(1);
        transaction
            .execute(
                "UPDATE device_auth SET status=2, generation=?2, revoked_at_unix=?3 WHERE endpoint_id=?1",
                params![endpoint_id.as_bytes().as_slice(), next, now_unix],
            )
            .map_err(store_error)?;
        transaction.commit().map_err(store_error)?;
        u64::try_from(next)
            .map_err(|error| DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string()))
    }

    /// Reads the retained authorization status and generation for one endpoint.
    pub fn authorization_status(
        &self,
        endpoint_id: DeviceId,
    ) -> Result<Option<(AuthorizationStatus, u64)>, DaemonError> {
        let row: Option<(i64, i64)> = self
            .connection
            .query_row(
                "SELECT status, generation FROM device_auth WHERE endpoint_id=?1",
                [endpoint_id.as_bytes().as_slice()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(store_error)?;
        row.map(|(status, generation)| {
            let status = match status {
                1 => AuthorizationStatus::Authorized,
                2 => AuthorizationStatus::Revoked,
                value => {
                    return Err(DaemonError::new(
                        DomainErrorKind::StoreUnavailable,
                        format!("unknown authorization status {value}"),
                    ));
                }
            };
            let generation = u64::try_from(generation).map_err(|error| {
                DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string())
            })?;
            Ok((status, generation))
        })
        .transpose()
    }

    /// Inserts or replaces one versioned route-cache entry.
    pub fn upsert_known_device(
        &mut self,
        endpoint_id: DeviceId,
        local_alias: &str,
        remote_name: &str,
        route_cache: Option<(u32, &[u8], i64)>,
    ) -> Result<(), DaemonError> {
        let (version, bytes, verified_at) =
            route_cache.map_or((None, None, None), |(version, bytes, verified_at)| {
                (Some(i64::from(version)), Some(bytes), Some(verified_at))
            });
        self.connection
            .execute(
                "INSERT INTO known_devices(endpoint_id, local_alias, remote_name, route_cache_version, route_cache, routes_verified_at_unix)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(endpoint_id) DO UPDATE SET local_alias=excluded.local_alias, remote_name=excluded.remote_name,
                    route_cache_version=excluded.route_cache_version, route_cache=excluded.route_cache,
                    routes_verified_at_unix=excluded.routes_verified_at_unix",
                params![
                    endpoint_id.as_bytes().as_slice(),
                    local_alias,
                    remote_name,
                    version,
                    bytes,
                    verified_at
                ],
            )
            .map_err(store_error)?;
        Ok(())
    }

    /// Returns all user table names for schema-inventory diagnostics/tests.
    pub fn table_names(&self) -> Result<Vec<String>, DaemonError> {
        let mut statement = self
            .connection
            .prepare("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
            .map_err(store_error)?;
        statement
            .query_map([], |row| row.get(0))
            .map_err(store_error)?
            .collect::<Result<Vec<_>, _>>()
            .map_err(store_error)
    }
}

/// Running daemon's sole SQLite connection owner.
pub struct StoreActor {
    sender: mpsc::Sender<StoreCommand>,
    thread: Option<JoinHandle<()>>,
}

impl StoreActor {
    /// Moves a store into one dedicated owner thread.
    pub fn start(store: StateStore) -> Result<Self, DaemonError> {
        let (sender, receiver) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("zterm-state-store".to_owned())
            .spawn(move || run_store_actor(store, receiver))
            .map_err(|error| {
                DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string())
            })?;
        Ok(Self {
            sender,
            thread: Some(thread),
        })
    }

    /// Reads identity metadata through the owner thread.
    pub fn metadata(&self) -> Result<Option<DeviceMetadata>, DaemonError> {
        let (response, receiver) = mpsc::channel();
        self.sender
            .send(StoreCommand::Metadata(response))
            .map_err(|error| {
                DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string())
            })?;
        receiver.recv().map_err(|error| {
            DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string())
        })?
    }
}

impl Drop for StoreActor {
    fn drop(&mut self) {
        let _ = self.sender.send(StoreCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

enum StoreCommand {
    Metadata(mpsc::Sender<Result<Option<DeviceMetadata>, DaemonError>>),
    Shutdown,
}

fn run_store_actor(store: StateStore, receiver: mpsc::Receiver<StoreCommand>) {
    while let Ok(command) = receiver.recv() {
        match command {
            StoreCommand::Metadata(response) => {
                let _ = response.send(store.metadata());
            }
            StoreCommand::Shutdown => return,
        }
    }
}

fn open_connection(paths: &UserPaths, flags: OpenFlags) -> Result<Connection, DaemonError> {
    // SQLite's `SQLITE_OPEN_NOFOLLOW` rejects a symlink in any path
    // component (for example macOS' `/var` -> `/private/var`). Resolve
    // already-validated components first, then retain `NOFOLLOW` for the
    // actual database open and SQLite-owned sidecar files.
    let database = fs::canonicalize(paths.database()).map_err(|error| {
        DaemonError::new(
            DomainErrorKind::PathUnsafe,
            format!("unable to resolve {}: {error}", paths.database().display()),
        )
    })?;
    Connection::open_with_flags(database, flags).map_err(store_error)
}

fn schema_version(connection: &Connection) -> Result<u32, DaemonError> {
    connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(store_error)
}

fn require_current_schema(connection: &Connection) -> Result<(), DaemonError> {
    let version = schema_version(connection)?;
    reject_schema_version(version)?;
    if version < STATE_SCHEMA_VERSION {
        return Err(DaemonError::new(
            DomainErrorKind::MigrationFailed,
            format!("database schema {version} requires setup migration to {STATE_SCHEMA_VERSION}"),
        ));
    }
    Ok(())
}

fn reject_too_new_schema(connection: &Connection) -> Result<(), DaemonError> {
    reject_schema_version(schema_version(connection)?)
}

fn reject_schema_version(version: u32) -> Result<(), DaemonError> {
    if version > STATE_SCHEMA_VERSION {
        Err(DaemonError::new(
            DomainErrorKind::SchemaTooNew,
            format!("database schema {version} is newer than supported {STATE_SCHEMA_VERSION}"),
        ))
    } else {
        Ok(())
    }
}

fn migrate(connection: &mut Connection) -> Result<(), DaemonError> {
    let version = schema_version(connection)?;
    reject_schema_version(version)?;
    if version == STATE_SCHEMA_VERSION {
        return Ok(());
    }

    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(migration_error)?;
    transaction
        .execute_batch(
            "CREATE TABLE metadata (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                device_id BLOB NOT NULL CHECK (length(device_id) = 32),
                device_name TEXT NOT NULL,
                created_at_unix INTEGER NOT NULL
            );
            CREATE TABLE device_auth (
                endpoint_id BLOB PRIMARY KEY CHECK (length(endpoint_id) = 32),
                display_name TEXT NOT NULL,
                status INTEGER NOT NULL CHECK (status IN (1, 2)),
                generation INTEGER NOT NULL CHECK (generation >= 1),
                paired_at_unix INTEGER NOT NULL,
                revoked_at_unix INTEGER,
                last_seen_at_unix INTEGER
            );
            CREATE TABLE known_devices (
                endpoint_id BLOB PRIMARY KEY CHECK (length(endpoint_id) = 32),
                local_alias TEXT NOT NULL UNIQUE,
                remote_name TEXT NOT NULL,
                route_cache_version INTEGER,
                route_cache BLOB,
                routes_verified_at_unix INTEGER
            );",
        )
        .map_err(migration_error)?;
    transaction
        .pragma_update(None, "user_version", STATE_SCHEMA_VERSION)
        .map_err(migration_error)?;
    transaction.commit().map_err(migration_error)
}

fn store_error(error: impl std::fmt::Display) -> DaemonError {
    DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string())
}

fn migration_error(error: impl std::fmt::Display) -> DaemonError {
    DaemonError::new(DomainErrorKind::MigrationFailed, error.to_string())
}

/// Sets a database's user_version for isolated too-new-schema testing.
#[doc(hidden)]
pub fn set_test_schema_version(path: &std::path::Path, version: u32) -> Result<(), DaemonError> {
    let connection = Connection::open(path).map_err(store_error)?;
    connection
        .pragma_update(None, "user_version", version)
        .map_err(store_error)
}

/// Reads raw database bytes only for secret-scan test ownership.
#[doc(hidden)]
pub fn database_bytes(paths: &UserPaths) -> Result<Vec<u8>, DaemonError> {
    fs::read(paths.database())
        .map_err(|error| DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_authorization_transaction_leaves_no_half_row() {
        let mut connection = Connection::open_in_memory().expect("in-memory SQLite");
        migrate(&mut connection).expect("schema migration");
        {
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .expect("transaction begins");
            transaction
                .execute(
                    "INSERT INTO device_auth(endpoint_id, display_name, status, generation, paired_at_unix) VALUES(?1, 'peer', 1, 1, 1)",
                    [DeviceId::from_array([1; 32]).as_bytes().as_slice()],
                )
                .expect("first statement");
            assert!(transaction
                .execute(
                    "INSERT INTO device_auth(endpoint_id, display_name, status, generation, paired_at_unix) VALUES(?1, 'bad', 99, 1, 1)",
                    [DeviceId::from_array([2; 32]).as_bytes().as_slice()],
                )
                .is_err());
        }
        let rows: i64 = connection
            .query_row("SELECT COUNT(*) FROM device_auth", [], |row| row.get(0))
            .expect("row count");
        assert_eq!(rows, 0);
    }
}
