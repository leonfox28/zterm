//! Bundled SQLite schema, validated row projections, and the single-owner
//! bounded store actor.

use std::fs;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use rusqlite::{Connection, ErrorCode, OpenFlags, OptionalExtension, TransactionBehavior, params};
use zterm_core::{
    AuthGeneration, AuthorizationSnapshot, AuthorizationStatus, DeviceAlias, DeviceDisplayName,
    DeviceId, DomainErrorKind, RelayHint, STATE_SCHEMA_VERSION,
};
use zterm_platform::user_state::{UserPaths, open_append, validate_regular_file};

use crate::error::DaemonError;

/// Raw SQLite status value for an authorized device.
const AUTHORIZED_STATUS: i64 = 1;
/// Raw SQLite status value for a revoked device.
const REVOKED_STATUS: i64 = 2;
/// Bounded capacity of the store actor command mailbox.
pub const STORE_COMMAND_CAPACITY: usize = 64;
/// Default store request deadline.
const DEFAULT_STORE_TIMEOUT: Duration = Duration::from_secs(30);
const COMMAND_QUEUED: u8 = 0;
const COMMAND_STARTED: u8 = 1;
const COMMAND_EXPIRED: u8 = 2;

type RawAuthorizationRow = (String, i64, i64, i64, Option<i64>, Option<i64>);

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

/// Validated projection of one inbound authorization row.
///
/// A `device_auth` row always carries an `Authorized` or `Revoked` status; the
/// `None` status only exists for devices with no row at all.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DeviceAuthorization {
    /// Remote device public identity.
    pub device_id: DeviceId,
    /// Display name supplied by the remote device during pairing.
    pub display_name: DeviceDisplayName,
    /// Current inbound status.
    pub status: AuthorizationStatus,
    /// Current inbound authorization generation.
    pub generation: AuthGeneration,
    /// Unix timestamp of the most recent authorization.
    pub paired_at_unix: i64,
    /// Unix timestamp of the most recent revoke, when revoked.
    pub revoked_at_unix: Option<i64>,
    /// Unix timestamp of the most recent authenticated handshake.
    pub last_seen_at_unix: Option<i64>,
}

/// Validated relay-only route cache for one known device.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RelayRouteCache {
    /// Ordered relay hint URLs retained for future dials.
    pub relay_hints: Vec<RelayHint>,
    /// Unix timestamp when this route was verified by a handshake.
    pub verified_at_unix: i64,
}

/// Non-fatal reason a persisted route cache was ignored.
///
/// Unknown versions remain structurally visible to status/doctor while the
/// known-device row itself stays usable without a route. Malformed data for a
/// version this binary claims to understand remains a corrupt-store error.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RouteCacheDiagnostic {
    /// The row uses a newer or otherwise unsupported cache format.
    UnsupportedVersion {
        /// Raw SQLite version value.
        actual: i64,
    },
}

/// Validated projection of one outbound known-device row.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnownDevice {
    /// Remote device public identity.
    pub device_id: DeviceId,
    /// Validated local alias used to select this device.
    pub local_alias: DeviceAlias,
    /// Display name supplied by the remote device.
    pub remote_name: DeviceDisplayName,
    /// Verified relay-only route cache, when one was persisted.
    pub route_cache: Option<RelayRouteCache>,
    /// Non-fatal reason the persisted cache was ignored.
    pub route_cache_diagnostic: Option<RouteCacheDiagnostic>,
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
        let metadata = self
            .connection
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
            .map_err(store_error)?;
        metadata
            .map(|metadata| {
                DeviceDisplayName::new(metadata.device_name.clone()).map_err(|error| {
                    corrupt_store(format!("metadata device name is invalid: {error}"))
                })?;
                validate_timestamp(metadata.created_at_unix, "metadata creation timestamp")?;
                Ok(metadata)
            })
            .transpose()
    }

    /// Inserts initial metadata or validates the committed identity/name.
    pub fn ensure_metadata(&mut self, expected: &DeviceMetadata) -> Result<(), DaemonError> {
        DeviceDisplayName::new(expected.device_name.clone()).map_err(|error| {
            DaemonError::new(
                DomainErrorKind::IdentityInvalid,
                format!("invalid device name: {error}"),
            )
        })?;
        validate_input_timestamp(expected.created_at_unix, "metadata creation timestamp")?;
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

    /// Authorizes a remote endpoint and always advances its generation.
    ///
    /// The generation uses checked arithmetic and refuses to wrap past the
    /// SQLite signed 64-bit ceiling. Re-authorizing an already-revoked device
    /// also clears the tombstone and advances.
    pub fn authorize_device(
        &mut self,
        endpoint_id: DeviceId,
        display_name: &str,
        now_unix: i64,
    ) -> Result<AuthGeneration, DaemonError> {
        let display_name = DeviceDisplayName::new(display_name.to_owned()).map_err(|error| {
            DaemonError::new(
                DomainErrorKind::PairTicketInvalid,
                format!("invalid remote display name: {error}"),
            )
        })?;
        validate_input_timestamp(now_unix, "pairing timestamp")?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let existing: Option<RawAuthorizationRow> = transaction
            .query_row(
                "SELECT display_name, status, generation, paired_at_unix, revoked_at_unix, last_seen_at_unix
                 FROM device_auth WHERE endpoint_id=?1",
                [endpoint_id.as_bytes().as_slice()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()
            .map_err(store_error)?;
        let current = existing
            .map(|(name, status, generation, paired, revoked, last_seen)| {
                project_authorization(
                    endpoint_id,
                    name,
                    status,
                    generation,
                    paired,
                    revoked,
                    last_seen,
                )
                .map(|row| row.generation)
            })
            .transpose()?
            .unwrap_or(AuthGeneration::ZERO);
        let next = checked_next_generation(current)?;
        transaction
            .execute(
                "INSERT INTO device_auth(endpoint_id, display_name, status, generation, paired_at_unix, revoked_at_unix, last_seen_at_unix)
                 VALUES(?1, ?2, 1, ?3, ?4, NULL, NULL)
                 ON CONFLICT(endpoint_id) DO UPDATE SET display_name=excluded.display_name, status=1,
                    generation=excluded.generation, paired_at_unix=excluded.paired_at_unix, revoked_at_unix=NULL",
                params![
                    endpoint_id.as_bytes().as_slice(),
                    display_name.as_str(),
                    next.to_i64(),
                    now_unix
                ],
            )
            .map_err(store_error)?;
        transaction.commit().map_err(store_error)?;
        Ok(next)
    }

    /// Persists a revoked tombstone, advancing the generation on first revoke.
    ///
    /// A repeated revoke is idempotent and returns the current generation
    /// unchanged; a device with no authorization row is `device_not_found`.
    pub fn revoke_device(
        &mut self,
        endpoint_id: DeviceId,
        now_unix: i64,
    ) -> Result<AuthGeneration, DaemonError> {
        validate_input_timestamp(now_unix, "revocation timestamp")?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let row: Option<RawAuthorizationRow> = transaction
            .query_row(
                "SELECT display_name, status, generation, paired_at_unix, revoked_at_unix, last_seen_at_unix
                 FROM device_auth WHERE endpoint_id=?1",
                [endpoint_id.as_bytes().as_slice()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()
            .map_err(store_error)?;
        let Some((name, status, generation, paired, revoked, last_seen)) = row else {
            return Err(DaemonError::new(
                DomainErrorKind::DeviceNotFound,
                "no inbound authorization record for device",
            ));
        };
        let current = project_authorization(
            endpoint_id,
            name,
            status,
            generation,
            paired,
            revoked,
            last_seen,
        )?;
        if current.status == AuthorizationStatus::Revoked {
            return Ok(current.generation);
        }
        let next = checked_next_generation(current.generation)?;
        transaction
            .execute(
                "UPDATE device_auth SET status=2, generation=?2, revoked_at_unix=?3 WHERE endpoint_id=?1",
                params![endpoint_id.as_bytes().as_slice(), next.to_i64(), now_unix],
            )
            .map_err(store_error)?;
        transaction.commit().map_err(store_error)?;
        Ok(next)
    }

    /// Reads the current inbound authorization snapshot for one endpoint.
    pub fn authorization_snapshot(
        &self,
        endpoint_id: DeviceId,
    ) -> Result<AuthorizationSnapshot, DaemonError> {
        let row: Option<RawAuthorizationRow> = self
            .connection
            .query_row(
                "SELECT display_name, status, generation, paired_at_unix, revoked_at_unix, last_seen_at_unix
                 FROM device_auth WHERE endpoint_id=?1",
                [endpoint_id.as_bytes().as_slice()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .optional()
            .map_err(store_error)?;
        match row {
            None => Ok(AuthorizationSnapshot::none()),
            Some((name, status, generation, paired, revoked, last_seen)) => {
                let row = project_authorization(
                    endpoint_id,
                    name,
                    status,
                    generation,
                    paired,
                    revoked,
                    last_seen,
                )?;
                Ok(AuthorizationSnapshot {
                    status: row.status,
                    generation: row.generation,
                })
            }
        }
    }

    /// Lists every inbound authorization row as a validated projection.
    pub fn list_authorizations(&self) -> Result<Vec<DeviceAuthorization>, DaemonError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT endpoint_id, display_name, status, generation, paired_at_unix, revoked_at_unix, last_seen_at_unix
                 FROM device_auth ORDER BY endpoint_id",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([], |row| {
                let bytes: Vec<u8> = row.get(0)?;
                let device_id = device_id_from_sql(&bytes)?;
                Ok((
                    device_id,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, Option<i64>>(6)?,
                ))
            })
            .map_err(store_error)?;
        rows.map(|row| {
            let (
                device_id,
                display_name,
                status,
                generation,
                paired_at_unix,
                revoked_at_unix,
                last_seen_at_unix,
            ) = row.map_err(store_error)?;
            project_authorization(
                device_id,
                display_name,
                status,
                generation,
                paired_at_unix,
                revoked_at_unix,
                last_seen_at_unix,
            )
        })
        .collect()
    }

    /// Records the timestamp of a successful authenticated handshake.
    pub fn set_last_seen(
        &mut self,
        endpoint_id: DeviceId,
        now_unix: i64,
    ) -> Result<(), DaemonError> {
        validate_input_timestamp(now_unix, "last-seen timestamp")?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let changed = transaction
            .execute(
                "UPDATE device_auth SET last_seen_at_unix=?2 WHERE endpoint_id=?1",
                params![endpoint_id.as_bytes().as_slice(), now_unix],
            )
            .map_err(store_error)?;
        if changed == 0 {
            return Err(DaemonError::new(
                DomainErrorKind::DeviceNotFound,
                "no inbound authorization record for device",
            ));
        }
        transaction.commit().map_err(store_error)?;
        Ok(())
    }

    /// Lists every outbound known-device row as a validated projection.
    pub fn list_known_devices(&self) -> Result<Vec<KnownDevice>, DaemonError> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT endpoint_id, local_alias, remote_name, route_cache_version, route_cache, routes_verified_at_unix
                 FROM known_devices ORDER BY endpoint_id",
            )
            .map_err(store_error)?;
        let rows = statement
            .query_map([], |row| {
                let bytes: Vec<u8> = row.get(0)?;
                let device_id = device_id_from_sql(&bytes)?;
                Ok((
                    device_id,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            })
            .map_err(store_error)?;
        rows.map(|row| {
            let (device_id, local_alias, remote_name, cache_version, cache, verified_at) =
                row.map_err(store_error)?;
            project_known_device(
                device_id,
                local_alias,
                remote_name,
                cache_version,
                cache,
                verified_at,
            )
        })
        .collect()
    }

    /// Reads one outbound known-device projection.
    pub fn known_device(&self, endpoint_id: DeviceId) -> Result<Option<KnownDevice>, DaemonError> {
        let row = self
            .connection
            .query_row(
                "SELECT endpoint_id, local_alias, remote_name, route_cache_version, route_cache, routes_verified_at_unix
                 FROM known_devices WHERE endpoint_id=?1",
                [endpoint_id.as_bytes().as_slice()],
                |row| {
                    let bytes: Vec<u8> = row.get(0)?;
                    let device_id = device_id_from_sql(&bytes)?;
                    Ok((
                        device_id,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<Vec<u8>>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(store_error)?;
        row.map(
            |(device_id, local_alias, remote_name, cache_version, cache, verified_at)| {
                project_known_device(
                    device_id,
                    local_alias,
                    remote_name,
                    cache_version,
                    cache,
                    verified_at,
                )
            },
        )
        .transpose()
    }

    /// Inserts or replaces one outbound known-device entry.
    pub fn upsert_known_device(
        &mut self,
        endpoint_id: DeviceId,
        local_alias: &DeviceAlias,
        remote_name: &str,
        route: Option<&RelayRouteCache>,
    ) -> Result<(), DaemonError> {
        let remote_name = DeviceDisplayName::new(remote_name.to_owned()).map_err(|error| {
            DaemonError::new(
                DomainErrorKind::PairTicketInvalid,
                format!("invalid remote display name: {error}"),
            )
        })?;
        let (version, bytes, verified_at) = match route {
            Some(cache) => {
                validate_input_timestamp(cache.verified_at_unix, "route verification timestamp")?;
                (
                    Some(i64::from(zterm_proto::RELAY_ROUTE_CACHE_VERSION)),
                    Some(encode_route_cache(cache)?),
                    Some(cache.verified_at_unix),
                )
            }
            None => (None, None, None),
        };
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        transaction
            .execute(
                "INSERT INTO known_devices(endpoint_id, local_alias, remote_name, route_cache_version, route_cache, routes_verified_at_unix)
                 VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(endpoint_id) DO UPDATE SET local_alias=excluded.local_alias, remote_name=excluded.remote_name,
                    route_cache_version=excluded.route_cache_version, route_cache=excluded.route_cache,
                    routes_verified_at_unix=excluded.routes_verified_at_unix",
                params![
                    endpoint_id.as_bytes().as_slice(),
                    local_alias.as_str(),
                    remote_name.as_str(),
                    version,
                    bytes,
                    verified_at
                ],
            )
            .map_err(map_known_device_error)?;
        transaction.commit().map_err(store_error)?;
        Ok(())
    }

    /// Confirms one outbound known device after a normal authenticated
    /// connection, preserving a previously verified route when this
    /// confirmation observed only a direct path.
    ///
    /// Alias, remote name, and an optional newly verified relay route are
    /// committed in one transaction. `None` permits a new route-less row but
    /// never clears route columns on an existing row.
    pub fn confirm_known_device(
        &mut self,
        endpoint_id: DeviceId,
        local_alias: &DeviceAlias,
        remote_name: &str,
        verified_route: Option<&RelayRouteCache>,
    ) -> Result<(), DaemonError> {
        let remote_name = DeviceDisplayName::new(remote_name.to_owned()).map_err(|error| {
            DaemonError::new(
                DomainErrorKind::PairTicketInvalid,
                format!("invalid remote display name: {error}"),
            )
        })?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        match verified_route {
            Some(route) => {
                validate_input_timestamp(route.verified_at_unix, "route verification timestamp")?;
                let bytes = encode_route_cache(route)?;
                transaction
                    .execute(
                        "INSERT INTO known_devices(endpoint_id, local_alias, remote_name, route_cache_version, route_cache, routes_verified_at_unix)
                         VALUES(?1, ?2, ?3, ?4, ?5, ?6)
                         ON CONFLICT(endpoint_id) DO UPDATE SET local_alias=excluded.local_alias, remote_name=excluded.remote_name,
                            route_cache_version=excluded.route_cache_version, route_cache=excluded.route_cache,
                            routes_verified_at_unix=excluded.routes_verified_at_unix",
                        params![
                            endpoint_id.as_bytes().as_slice(),
                            local_alias.as_str(),
                            remote_name.as_str(),
                            i64::from(zterm_proto::RELAY_ROUTE_CACHE_VERSION),
                            bytes,
                            route.verified_at_unix,
                        ],
                    )
                    .map_err(map_known_device_error)?;
            }
            None => {
                transaction
                    .execute(
                        "INSERT INTO known_devices(endpoint_id, local_alias, remote_name, route_cache_version, route_cache, routes_verified_at_unix)
                         VALUES(?1, ?2, ?3, NULL, NULL, NULL)
                         ON CONFLICT(endpoint_id) DO UPDATE SET local_alias=excluded.local_alias, remote_name=excluded.remote_name",
                        params![
                            endpoint_id.as_bytes().as_slice(),
                            local_alias.as_str(),
                            remote_name.as_str(),
                        ],
                    )
                    .map_err(map_known_device_error)?;
            }
        }
        transaction.commit().map_err(store_error)?;
        Ok(())
    }

    /// Replaces only the verified relay route for an existing known device.
    ///
    /// Alias and remote display name are intentionally left untouched so a
    /// path observation racing a user rename cannot restore stale directory
    /// metadata.
    pub fn set_known_route(
        &mut self,
        endpoint_id: DeviceId,
        route: &RelayRouteCache,
    ) -> Result<(), DaemonError> {
        validate_input_timestamp(route.verified_at_unix, "route verification timestamp")?;
        let bytes = encode_route_cache(route)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let changed = transaction
            .execute(
                "UPDATE known_devices
                 SET route_cache_version=?2, route_cache=?3, routes_verified_at_unix=?4
                 WHERE endpoint_id=?1",
                params![
                    endpoint_id.as_bytes().as_slice(),
                    i64::from(zterm_proto::RELAY_ROUTE_CACHE_VERSION),
                    bytes,
                    route.verified_at_unix,
                ],
            )
            .map_err(store_error)?;
        if changed == 0 {
            return Err(DaemonError::new(
                DomainErrorKind::DeviceNotFound,
                "no outbound known-device record for endpoint",
            ));
        }
        transaction.commit().map_err(store_error)?;
        Ok(())
    }

    /// Renames only the outbound `known_devices.local_alias` for one endpoint.
    pub fn rename_alias(
        &mut self,
        endpoint_id: DeviceId,
        local_alias: &DeviceAlias,
    ) -> Result<(), DaemonError> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(store_error)?;
        let changed = transaction
            .execute(
                "UPDATE known_devices SET local_alias=?2 WHERE endpoint_id=?1",
                params![endpoint_id.as_bytes().as_slice(), local_alias.as_str()],
            )
            .map_err(map_known_device_error)?;
        if changed == 0 {
            return Err(DaemonError::new(
                DomainErrorKind::DeviceNotFound,
                "no outbound known-device record for endpoint",
            ));
        }
        transaction.commit().map_err(store_error)?;
        Ok(())
    }

    /// Returns whether an alias is currently unclaimed in the address book.
    pub fn alias_available(&self, alias: &str) -> Result<bool, DaemonError> {
        let count: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM known_devices WHERE local_alias=?1",
                [alias],
                |row| row.get(0),
            )
            .map_err(store_error)?;
        Ok(count == 0)
    }

    /// Returns the device which durably owns an alias, when one is claimed.
    pub fn alias_owner(&self, alias: &str) -> Result<Option<DeviceId>, DaemonError> {
        let row: Option<Vec<u8>> = self
            .connection
            .query_row(
                "SELECT endpoint_id FROM known_devices WHERE local_alias=?1",
                [alias],
                |row| row.get(0),
            )
            .optional()
            .map_err(store_error)?;
        row.map(|bytes| device_id_from_sql(&bytes))
            .transpose()
            .map_err(store_error)
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

/// Cloneable handle into the running store actor.
///
/// Every call carries one absolute deadline and a started gate; a command whose
/// caller gave up before the actor started it is never executed. Callers on a
/// Tokio runtime wrap these blocking calls in `spawn_blocking`.
#[derive(Clone)]
pub struct StoreHandle {
    sender: SyncSender<StoreCommand>,
}

impl StoreHandle {
    /// Reads identity metadata through the owner thread.
    pub fn metadata(&self, deadline: Instant) -> Result<Option<DeviceMetadata>, DaemonError> {
        self.request(deadline, |meta, reply| StoreCommand::Metadata {
            meta,
            reply,
        })
    }

    /// Reads metadata and signals only after the command occupies a mailbox
    /// slot. This deterministic hook is reserved for bounded-queue tests.
    #[doc(hidden)]
    pub fn metadata_queued_for_test(
        &self,
        deadline: Instant,
        queued: mpsc::Sender<()>,
    ) -> Result<Option<DeviceMetadata>, DaemonError> {
        self.request_observed(deadline, Some(queued), |meta, reply| {
            StoreCommand::Metadata { meta, reply }
        })
    }

    /// Authorizes a remote endpoint, advancing its generation.
    pub fn authorize(
        &self,
        endpoint_id: DeviceId,
        display_name: impl Into<String>,
        now_unix: i64,
        deadline: Instant,
    ) -> Result<AuthGeneration, DaemonError> {
        let display_name = DeviceDisplayName::new(display_name).map_err(|error| {
            DaemonError::new(
                DomainErrorKind::PairTicketInvalid,
                format!("invalid remote display name: {error}"),
            )
        })?;
        self.request(deadline, |meta, reply| StoreCommand::Authorize {
            meta,
            endpoint_id,
            display_name,
            now_unix,
            reply,
        })
    }

    /// Revokes a remote endpoint, advancing its generation on first revoke.
    pub fn revoke(
        &self,
        endpoint_id: DeviceId,
        now_unix: i64,
        deadline: Instant,
    ) -> Result<AuthGeneration, DaemonError> {
        self.request(deadline, |meta, reply| StoreCommand::Revoke {
            meta,
            endpoint_id,
            now_unix,
            reply,
        })
    }

    /// Reads the current inbound authorization snapshot.
    pub fn authorization_snapshot(
        &self,
        endpoint_id: DeviceId,
        deadline: Instant,
    ) -> Result<AuthorizationSnapshot, DaemonError> {
        self.request(deadline, |meta, reply| {
            StoreCommand::AuthorizationSnapshot {
                meta,
                endpoint_id,
                reply,
            }
        })
    }

    /// Lists every inbound authorization row.
    pub fn list_authorizations(
        &self,
        deadline: Instant,
    ) -> Result<Vec<DeviceAuthorization>, DaemonError> {
        self.request(deadline, |meta, reply| StoreCommand::ListAuthorizations {
            meta,
            reply,
        })
    }

    /// Records a successful authenticated handshake timestamp.
    pub fn set_last_seen(
        &self,
        endpoint_id: DeviceId,
        now_unix: i64,
        deadline: Instant,
    ) -> Result<(), DaemonError> {
        self.request(deadline, |meta, reply| StoreCommand::SetLastSeen {
            meta,
            endpoint_id,
            now_unix,
            reply,
        })
    }

    /// Lists every outbound known-device row.
    pub fn list_known_devices(&self, deadline: Instant) -> Result<Vec<KnownDevice>, DaemonError> {
        self.request(deadline, |meta, reply| StoreCommand::ListKnownDevices {
            meta,
            reply,
        })
    }

    /// Reads one outbound known-device projection.
    pub fn known_device(
        &self,
        endpoint_id: DeviceId,
        deadline: Instant,
    ) -> Result<Option<KnownDevice>, DaemonError> {
        self.request(deadline, |meta, reply| StoreCommand::KnownDevice {
            meta,
            endpoint_id,
            reply,
        })
    }

    /// Inserts or replaces one outbound known-device entry.
    pub fn upsert_known_device(
        &self,
        endpoint_id: DeviceId,
        local_alias: DeviceAlias,
        remote_name: impl Into<String>,
        route: Option<RelayRouteCache>,
        deadline: Instant,
    ) -> Result<(), DaemonError> {
        let remote_name = DeviceDisplayName::new(remote_name).map_err(|error| {
            DaemonError::new(
                DomainErrorKind::PairTicketInvalid,
                format!("invalid remote display name: {error}"),
            )
        })?;
        self.request(deadline, |meta, reply| StoreCommand::UpsertKnownDevice {
            meta,
            endpoint_id,
            local_alias,
            remote_name,
            route,
            reply,
        })
    }

    /// Atomically confirms an outbound known device and replaces its route
    /// only when this confirmation carries a newly verified relay route.
    pub fn confirm_known_device(
        &self,
        endpoint_id: DeviceId,
        local_alias: DeviceAlias,
        remote_name: impl Into<String>,
        verified_route: Option<RelayRouteCache>,
        deadline: Instant,
    ) -> Result<(), DaemonError> {
        let remote_name = DeviceDisplayName::new(remote_name).map_err(|error| {
            DaemonError::new(
                DomainErrorKind::PairTicketInvalid,
                format!("invalid remote display name: {error}"),
            )
        })?;
        self.request(deadline, |meta, reply| StoreCommand::ConfirmKnownDevice {
            meta,
            endpoint_id,
            local_alias,
            remote_name,
            verified_route,
            reply,
        })
    }

    /// Replaces only the verified relay route for an existing known device.
    pub fn set_known_route(
        &self,
        endpoint_id: DeviceId,
        route: RelayRouteCache,
        deadline: Instant,
    ) -> Result<(), DaemonError> {
        self.request(deadline, |meta, reply| StoreCommand::SetKnownRoute {
            meta,
            endpoint_id,
            route,
            reply,
        })
    }

    /// Renames only the outbound alias for one endpoint.
    pub fn rename_alias(
        &self,
        endpoint_id: DeviceId,
        local_alias: DeviceAlias,
        deadline: Instant,
    ) -> Result<(), DaemonError> {
        self.request(deadline, |meta, reply| StoreCommand::RenameAlias {
            meta,
            endpoint_id,
            local_alias,
            reply,
        })
    }

    /// Returns whether an alias is currently unclaimed in the address book.
    pub fn alias_available(&self, alias: String, deadline: Instant) -> Result<bool, DaemonError> {
        self.request(deadline, |meta, reply| StoreCommand::AliasAvailable {
            meta,
            alias,
            reply,
        })
    }

    /// Returns the device which durably owns an alias, when one is claimed.
    pub fn alias_owner(
        &self,
        alias: String,
        deadline: Instant,
    ) -> Result<Option<DeviceId>, DaemonError> {
        self.request(deadline, |meta, reply| StoreCommand::AliasOwner {
            meta,
            alias,
            reply,
        })
    }

    /// Blocks the store thread until `release` is signaled, for queue-bounding
    /// tests. Must not be used by production paths.
    #[doc(hidden)]
    pub fn block_for_test(
        &self,
        deadline: Instant,
        entered: SyncSender<()>,
        release: Receiver<()>,
    ) -> Result<(), DaemonError> {
        self.request(deadline, |meta, reply| StoreCommand::BlockForTest {
            meta,
            entered,
            release,
            reply,
        })
    }

    /// Drops an injected response before or after the command-start gate.
    /// This deterministic hook is reserved for outcome-ambiguity tests.
    #[doc(hidden)]
    pub fn disconnect_response_for_test(
        &self,
        deadline: Instant,
        after_start: bool,
    ) -> Result<(), DaemonError> {
        self.request(deadline, |meta, reply| {
            StoreCommand::DisconnectResponseForTest {
                meta,
                after_start,
                reply,
            }
        })
    }

    /// Runs a blocking store operation on the Tokio blocking pool.
    ///
    /// Runtime-facing callers use this instead of invoking a synchronous method
    /// inline, so queue admission and the response wait never block a Tokio
    /// worker thread. The closure carries the single absolute deadline into the
    /// synchronous method it calls.
    pub async fn run_blocking_until<R>(
        &self,
        deadline: Instant,
        operation: impl FnOnce(&StoreHandle, Instant) -> Result<R, DaemonError> + Send + 'static,
    ) -> Result<R, DaemonError>
    where
        R: Send + 'static,
    {
        let handle = self.clone();
        tokio::task::spawn_blocking(move || {
            if Instant::now() >= deadline {
                return Err(deadline_exceeded(
                    "store runtime wait deadline elapsed before dispatch",
                ));
            }
            operation(&handle, deadline)
        })
        .await
        .map_err(|error| {
            DaemonError::new(
                DomainErrorKind::Cancelled,
                format!("store blocking worker ended unexpectedly: {error}"),
            )
        })?
    }

    fn request<R>(
        &self,
        deadline: Instant,
        build: impl FnOnce(CommandMeta, SyncSender<Result<R, DaemonError>>) -> StoreCommand,
    ) -> Result<R, DaemonError> {
        self.request_observed(deadline, None, build)
    }

    fn request_observed<R>(
        &self,
        deadline: Instant,
        queued: Option<mpsc::Sender<()>>,
        build: impl FnOnce(CommandMeta, SyncSender<Result<R, DaemonError>>) -> StoreCommand,
    ) -> Result<R, DaemonError> {
        let gate = Arc::new(CommandGate::default());
        let meta = CommandMeta {
            deadline,
            gate: Arc::clone(&gate),
        };
        let (reply, response) = mpsc::sync_channel(1);
        let mut command = build(meta, reply);
        loop {
            match self.sender.try_send(command) {
                Ok(()) => {
                    if let Some(queued) = queued.as_ref() {
                        let _ = queued.send(());
                    }
                    break;
                }
                Err(TrySendError::Disconnected(_)) => {
                    return Err(store_unavailable("store actor has stopped"));
                }
                Err(TrySendError::Full(returned)) => {
                    command = returned;
                    if Instant::now() >= deadline {
                        let _ = gate.state.compare_exchange(
                            COMMAND_QUEUED,
                            COMMAND_EXPIRED,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        );
                        return Err(deadline_exceeded(
                            "store command queue admission deadline elapsed",
                        ));
                    }
                    thread::sleep(Duration::from_millis(1));
                }
            }
        }
        wait_for_store_response(response, gate, deadline)
    }
}

/// Sole owner of the store actor thread.
///
/// This retains the join handle and is the only component which may stop the
/// actor; `StoreHandle` clones can only enqueue commands. Dropping the owner
/// stops and joins the thread exactly once.
pub struct StoreActor {
    handle: StoreHandle,
    thread: Option<JoinHandle<()>>,
}

impl StoreActor {
    /// Moves a store into one dedicated owner thread.
    pub fn start(store: StateStore) -> Result<Self, DaemonError> {
        let (sender, receiver) = mpsc::sync_channel(STORE_COMMAND_CAPACITY);
        let thread = thread::Builder::new()
            .name("zterm-state-store".to_owned())
            .spawn(move || run_store_actor(store, receiver))
            .map_err(|error| {
                DaemonError::new(DomainErrorKind::StoreUnavailable, error.to_string())
            })?;
        Ok(Self {
            handle: StoreHandle { sender },
            thread: Some(thread),
        })
    }

    /// Returns a cloneable handle for enqueueing store commands.
    #[must_use]
    pub fn handle(&self) -> StoreHandle {
        self.handle.clone()
    }

    /// Stops the actor and joins its thread exactly once.
    pub fn shutdown(mut self) {
        self.stop_and_join();
    }

    fn stop_and_join(&mut self) {
        if self.thread.is_none() {
            return;
        }
        let _ = self.handle.sender.send(StoreCommand::Shutdown);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for StoreActor {
    fn drop(&mut self) {
        self.stop_and_join();
    }
}

/// Per-command deadline and started gate shared by the waiter and the actor.
#[derive(Default)]
struct CommandGate {
    state: AtomicU8,
}

/// Metadata carried by every store command.
struct CommandMeta {
    deadline: Instant,
    gate: Arc<CommandGate>,
}

impl CommandMeta {
    /// Marks the command started if it has not expired; returns whether the
    /// actor may run its side effect.
    fn try_start(&self) -> bool {
        if Instant::now() >= self.deadline {
            let _ = self.gate.state.compare_exchange(
                COMMAND_QUEUED,
                COMMAND_EXPIRED,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            return false;
        }
        self.gate
            .state
            .compare_exchange(
                COMMAND_QUEUED,
                COMMAND_STARTED,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }
}

enum StoreCommand {
    Metadata {
        meta: CommandMeta,
        reply: SyncSender<Result<Option<DeviceMetadata>, DaemonError>>,
    },
    Authorize {
        meta: CommandMeta,
        endpoint_id: DeviceId,
        display_name: DeviceDisplayName,
        now_unix: i64,
        reply: SyncSender<Result<AuthGeneration, DaemonError>>,
    },
    Revoke {
        meta: CommandMeta,
        endpoint_id: DeviceId,
        now_unix: i64,
        reply: SyncSender<Result<AuthGeneration, DaemonError>>,
    },
    AuthorizationSnapshot {
        meta: CommandMeta,
        endpoint_id: DeviceId,
        reply: SyncSender<Result<AuthorizationSnapshot, DaemonError>>,
    },
    ListAuthorizations {
        meta: CommandMeta,
        reply: SyncSender<Result<Vec<DeviceAuthorization>, DaemonError>>,
    },
    SetLastSeen {
        meta: CommandMeta,
        endpoint_id: DeviceId,
        now_unix: i64,
        reply: SyncSender<Result<(), DaemonError>>,
    },
    ListKnownDevices {
        meta: CommandMeta,
        reply: SyncSender<Result<Vec<KnownDevice>, DaemonError>>,
    },
    KnownDevice {
        meta: CommandMeta,
        endpoint_id: DeviceId,
        reply: SyncSender<Result<Option<KnownDevice>, DaemonError>>,
    },
    UpsertKnownDevice {
        meta: CommandMeta,
        endpoint_id: DeviceId,
        local_alias: DeviceAlias,
        remote_name: DeviceDisplayName,
        route: Option<RelayRouteCache>,
        reply: SyncSender<Result<(), DaemonError>>,
    },
    ConfirmKnownDevice {
        meta: CommandMeta,
        endpoint_id: DeviceId,
        local_alias: DeviceAlias,
        remote_name: DeviceDisplayName,
        verified_route: Option<RelayRouteCache>,
        reply: SyncSender<Result<(), DaemonError>>,
    },
    SetKnownRoute {
        meta: CommandMeta,
        endpoint_id: DeviceId,
        route: RelayRouteCache,
        reply: SyncSender<Result<(), DaemonError>>,
    },
    RenameAlias {
        meta: CommandMeta,
        endpoint_id: DeviceId,
        local_alias: DeviceAlias,
        reply: SyncSender<Result<(), DaemonError>>,
    },
    AliasAvailable {
        meta: CommandMeta,
        alias: String,
        reply: SyncSender<Result<bool, DaemonError>>,
    },
    AliasOwner {
        meta: CommandMeta,
        alias: String,
        reply: SyncSender<Result<Option<DeviceId>, DaemonError>>,
    },
    BlockForTest {
        meta: CommandMeta,
        entered: SyncSender<()>,
        release: Receiver<()>,
        reply: SyncSender<Result<(), DaemonError>>,
    },
    DisconnectResponseForTest {
        meta: CommandMeta,
        after_start: bool,
        reply: SyncSender<Result<(), DaemonError>>,
    },
    Shutdown,
}

fn run_store_actor(mut store: StateStore, receiver: Receiver<StoreCommand>) {
    while let Ok(command) = receiver.recv() {
        match command {
            StoreCommand::Shutdown => return,
            StoreCommand::Metadata { meta, reply } => {
                let _ = reply.send(started(meta, || store.metadata()));
            }
            StoreCommand::Authorize {
                meta,
                endpoint_id,
                display_name,
                now_unix,
                reply,
            } => {
                let _ = reply.send(started(meta, || {
                    store.authorize_device(endpoint_id, display_name.as_str(), now_unix)
                }));
            }
            StoreCommand::Revoke {
                meta,
                endpoint_id,
                now_unix,
                reply,
            } => {
                let _ = reply.send(started(meta, || store.revoke_device(endpoint_id, now_unix)));
            }
            StoreCommand::AuthorizationSnapshot {
                meta,
                endpoint_id,
                reply,
            } => {
                let _ = reply.send(started(meta, || store.authorization_snapshot(endpoint_id)));
            }
            StoreCommand::ListAuthorizations { meta, reply } => {
                let _ = reply.send(started(meta, || store.list_authorizations()));
            }
            StoreCommand::SetLastSeen {
                meta,
                endpoint_id,
                now_unix,
                reply,
            } => {
                let _ = reply.send(started(meta, || store.set_last_seen(endpoint_id, now_unix)));
            }
            StoreCommand::ListKnownDevices { meta, reply } => {
                let _ = reply.send(started(meta, || store.list_known_devices()));
            }
            StoreCommand::KnownDevice {
                meta,
                endpoint_id,
                reply,
            } => {
                let _ = reply.send(started(meta, || store.known_device(endpoint_id)));
            }
            StoreCommand::UpsertKnownDevice {
                meta,
                endpoint_id,
                local_alias,
                remote_name,
                route,
                reply,
            } => {
                let _ = reply.send(started(meta, || {
                    store.upsert_known_device(
                        endpoint_id,
                        &local_alias,
                        remote_name.as_str(),
                        route.as_ref(),
                    )
                }));
            }
            StoreCommand::ConfirmKnownDevice {
                meta,
                endpoint_id,
                local_alias,
                remote_name,
                verified_route,
                reply,
            } => {
                let _ = reply.send(started(meta, || {
                    store.confirm_known_device(
                        endpoint_id,
                        &local_alias,
                        remote_name.as_str(),
                        verified_route.as_ref(),
                    )
                }));
            }
            StoreCommand::SetKnownRoute {
                meta,
                endpoint_id,
                route,
                reply,
            } => {
                let _ = reply.send(started(meta, || store.set_known_route(endpoint_id, &route)));
            }
            StoreCommand::RenameAlias {
                meta,
                endpoint_id,
                local_alias,
                reply,
            } => {
                let _ = reply.send(started(meta, || {
                    store.rename_alias(endpoint_id, &local_alias)
                }));
            }
            StoreCommand::AliasAvailable { meta, alias, reply } => {
                let _ = reply.send(started(meta, || store.alias_available(&alias)));
            }
            StoreCommand::AliasOwner { meta, alias, reply } => {
                let _ = reply.send(started(meta, || store.alias_owner(&alias)));
            }
            StoreCommand::BlockForTest {
                meta,
                entered,
                release,
                reply,
            } => {
                if !meta.try_start() {
                    let _ = reply.send(Err(deadline_exceeded(
                        "store command expired before starting",
                    )));
                    continue;
                }
                let _ = entered.send(());
                let _ = release.recv();
                let _ = reply.send(Ok(()));
            }
            StoreCommand::DisconnectResponseForTest {
                meta,
                after_start,
                reply,
            } => {
                if after_start && !meta.try_start() {
                    let _ = reply.send(Err(deadline_exceeded(
                        "store command expired before starting",
                    )));
                }
                // Dropping the sole response sender deterministically exercises
                // the waiter's gate-sensitive disconnect classification.
                drop(reply);
            }
        }
    }
}

/// Runs a store side effect only after its deadline gate reports "started".
fn started<R>(
    meta: CommandMeta,
    operation: impl FnOnce() -> Result<R, DaemonError>,
) -> Result<R, DaemonError> {
    if !meta.try_start() {
        return Err(deadline_exceeded("store command expired before starting"));
    }
    operation()
}

fn wait_for_store_response<R>(
    response: Receiver<Result<R, DaemonError>>,
    gate: Arc<CommandGate>,
    deadline: Instant,
) -> Result<R, DaemonError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    match response.recv_timeout(remaining) {
        Ok(result) => result,
        Err(RecvTimeoutError::Disconnected) => match gate.state.load(Ordering::Acquire) {
            COMMAND_STARTED => Err(DaemonError::new(
                DomainErrorKind::OperationOutcomeUnknown,
                "store command started but its response channel disconnected",
            )),
            COMMAND_EXPIRED => Err(deadline_exceeded("store command expired before starting")),
            _ => Err(store_unavailable(
                "store actor stopped before the command outcome became ambiguous",
            )),
        },
        Err(RecvTimeoutError::Timeout) => {
            if gate
                .state
                .compare_exchange(
                    COMMAND_QUEUED,
                    COMMAND_EXPIRED,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                Err(deadline_exceeded("store command expired before starting"))
            } else {
                Err(DaemonError::new(
                    DomainErrorKind::OperationOutcomeUnknown,
                    "store command started but did not report an outcome before its deadline",
                ))
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn project_authorization(
    device_id: DeviceId,
    display_name: String,
    status: i64,
    generation: i64,
    paired_at_unix: i64,
    revoked_at_unix: Option<i64>,
    last_seen_at_unix: Option<i64>,
) -> Result<DeviceAuthorization, DaemonError> {
    let display_name = DeviceDisplayName::new(display_name)
        .map_err(|error| corrupt_store(format!("authorization display name: {error}")))?;
    let status = authorization_status_from_i64(status, "authorization status")?;
    let generation = auth_generation_from_i64(generation, "authorization generation")?;
    if generation == AuthGeneration::ZERO {
        return Err(corrupt_store(
            "authorization row uses the reserved zero generation",
        ));
    }
    validate_timestamp(paired_at_unix, "authorization pairing timestamp")?;
    if let Some(value) = revoked_at_unix {
        validate_timestamp(value, "authorization revocation timestamp")?;
    }
    if let Some(value) = last_seen_at_unix {
        validate_timestamp(value, "authorization last-seen timestamp")?;
    }
    match (status, revoked_at_unix) {
        (AuthorizationStatus::Authorized, None) | (AuthorizationStatus::Revoked, Some(_)) => {}
        (AuthorizationStatus::Authorized, Some(_)) => {
            return Err(corrupt_store(
                "authorized row must not carry a revocation tombstone",
            ));
        }
        (AuthorizationStatus::Revoked, None) => {
            return Err(corrupt_store(
                "revoked row must carry a revocation tombstone",
            ));
        }
        (AuthorizationStatus::None, _) => {
            return Err(corrupt_store(
                "authorization table row cannot use the absent status",
            ));
        }
    }
    Ok(DeviceAuthorization {
        device_id,
        display_name,
        status,
        generation,
        paired_at_unix,
        revoked_at_unix,
        last_seen_at_unix,
    })
}

fn project_known_device(
    device_id: DeviceId,
    local_alias: String,
    remote_name: String,
    cache_version: Option<i64>,
    cache: Option<Vec<u8>>,
    verified_at: Option<i64>,
) -> Result<KnownDevice, DaemonError> {
    let local_alias = DeviceAlias::new(local_alias)
        .map_err(|error| corrupt_store(format!("known device alias: {error}")))?;
    let remote_name = DeviceDisplayName::new(remote_name)
        .map_err(|error| corrupt_store(format!("known device remote name: {error}")))?;
    let (route_cache, route_cache_diagnostic) = match (cache_version, cache, verified_at) {
        (None, None, None) => (None, None),
        (Some(version), Some(bytes), Some(verified_at_unix)) => {
            validate_timestamp(verified_at_unix, "route verification timestamp")?;
            if version != i64::from(zterm_proto::RELAY_ROUTE_CACHE_VERSION) {
                (
                    None,
                    Some(RouteCacheDiagnostic::UnsupportedVersion { actual: version }),
                )
            } else {
                (
                    Some(RelayRouteCache {
                        relay_hints: decode_relay_hints(&bytes)?,
                        verified_at_unix,
                    }),
                    None,
                )
            }
        }
        _ => {
            return Err(corrupt_store(
                "known device route cache version, blob, and timestamp disagree",
            ));
        }
    };
    Ok(KnownDevice {
        device_id,
        local_alias,
        remote_name,
        route_cache,
        route_cache_diagnostic,
    })
}

fn encode_route_cache(cache: &RelayRouteCache) -> Result<Vec<u8>, DaemonError> {
    zterm_proto::encode_relay_route_cache(&cache.relay_hints)
        .map_err(|error| corrupt_store(format!("route cache: {error}")))
}

/// Decodes the persisted relay route cache; any malformed, oversized, excess,
/// invalid, or unknown-version cache is a typed corrupt-store error.
fn decode_relay_hints(bytes: &[u8]) -> Result<Vec<RelayHint>, DaemonError> {
    zterm_proto::decode_relay_route_cache(bytes)
        .map_err(|error| corrupt_store(format!("route cache: {error}")))
}

fn checked_next_generation(current: AuthGeneration) -> Result<AuthGeneration, DaemonError> {
    current.checked_next().ok_or_else(|| {
        DaemonError::new(
            DomainErrorKind::StoreUnavailable,
            "authorization generation exhausted the SQLite signed 64-bit ceiling",
        )
    })
}

fn authorization_status_from_i64(
    value: i64,
    column: &str,
) -> Result<AuthorizationStatus, DaemonError> {
    match value {
        AUTHORIZED_STATUS => Ok(AuthorizationStatus::Authorized),
        REVOKED_STATUS => Ok(AuthorizationStatus::Revoked),
        _ => Err(corrupt_store(format!("unknown {column} value {value}"))),
    }
}

fn auth_generation_from_i64(value: i64, column: &str) -> Result<AuthGeneration, DaemonError> {
    AuthGeneration::from_i64(value)
        .ok_or_else(|| corrupt_store(format!("{column} {value} is not a valid generation")))
}

fn validate_timestamp(value: i64, column: &str) -> Result<(), DaemonError> {
    if value < 0 {
        Err(corrupt_store(format!(
            "{column} {value} must not be negative"
        )))
    } else {
        Ok(())
    }
}

fn validate_input_timestamp(value: i64, field: &str) -> Result<(), DaemonError> {
    if value < 0 {
        Err(DaemonError::new(
            DomainErrorKind::PairTicketInvalid,
            format!("{field} must not be negative"),
        ))
    } else {
        Ok(())
    }
}

fn device_id_from_sql(bytes: &[u8]) -> Result<DeviceId, rusqlite::Error> {
    DeviceId::from_bytes(bytes).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            bytes.len(),
            rusqlite::types::Type::Blob,
            Box::new(error),
        )
    })
}

fn map_known_device_error(error: rusqlite::Error) -> DaemonError {
    if let rusqlite::Error::SqliteFailure(sqlite_error, _) = &error
        && sqlite_error.code == ErrorCode::ConstraintViolation
    {
        return DaemonError::new(
            DomainErrorKind::DeviceAliasConflict,
            "device alias is already claimed",
        );
    }
    store_error(error)
}

fn corrupt_store(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::StoreUnavailable, detail)
}

fn store_unavailable(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::StoreUnavailable, detail)
}

fn deadline_exceeded(detail: impl Into<String>) -> DaemonError {
    DaemonError::new(DomainErrorKind::DeadlineExceeded, detail)
}

/// Returns a default store request deadline.
#[must_use]
pub fn default_store_deadline() -> Instant {
    Instant::now() + DEFAULT_STORE_TIMEOUT
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
