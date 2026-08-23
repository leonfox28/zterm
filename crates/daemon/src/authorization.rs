//! In-memory inbound authorization registry with a fair per-device gate.
//!
//! The durable truth lives in SQLite via the [`crate::store::StoreActor`]; this
//! registry preloads and mirrors it so every connection, stream, and sensitive
//! commit can be checked against the current `(status, generation)` without a
//! database round trip. Authorize/revoke acquire the owned write permit here
//! and publish the new snapshot; the later revoke coordinator sequences the
//! durable write, in-memory publish, connection close, and Session detach.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::sync::{OwnedRwLockReadGuard, OwnedRwLockWriteGuard, RwLock, watch};
use zterm_core::{
    AuthGeneration, AuthorizationSnapshot, AuthorizationStatus, DeviceId, DomainErrorKind,
};

use crate::error::DaemonError;
use crate::store::DeviceAuthorization;

/// One per-device fair lock plus the cancellation snapshot channel.
#[derive(Debug)]
struct AuthEntry {
    state: Arc<RwLock<AuthorizationSnapshot>>,
    watch: watch::Sender<AuthorizationSnapshot>,
}

impl AuthEntry {
    fn new(snapshot: AuthorizationSnapshot) -> Self {
        let (watch, _) = watch::channel(snapshot);
        Self {
            state: Arc::new(RwLock::new(snapshot)),
            watch,
        }
    }
}

/// In-memory authorization truth for one daemon process.
#[derive(Clone)]
pub struct AuthorizationRegistry {
    entries: Arc<Mutex<BTreeMap<DeviceId, Arc<AuthEntry>>>>,
}

impl Default for AuthorizationRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl AuthorizationRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Preloads every durable authorization row before the registry is used.
    pub fn preload(&self, rows: Vec<DeviceAuthorization>) -> Result<(), DaemonError> {
        let mut entries = entry_lock(&self.entries);
        for row in rows {
            let snapshot = AuthorizationSnapshot {
                status: row.status,
                generation: row.generation,
            };
            validate_snapshot(snapshot)?;
            if entries
                .insert(row.device_id, Arc::new(AuthEntry::new(snapshot)))
                .is_some()
            {
                return Err(DaemonError::new(
                    DomainErrorKind::StoreUnavailable,
                    "duplicate device authorization while preloading registry",
                ));
            }
        }
        Ok(())
    }

    /// Returns the current authorization snapshot for one device.
    pub fn snapshot(&self, device_id: DeviceId) -> Result<AuthorizationSnapshot, DaemonError> {
        Ok(match self.entry(device_id) {
            Some(entry) => *entry.watch.borrow(),
            None => AuthorizationSnapshot::none(),
        })
    }

    /// Admits an authorized device and subscribes it to cancellation.
    ///
    /// The returned receiver observes the same snapshot and fires when a later
    /// authorize/revoke publishes a new generation, so a connection or stream
    /// can cancel itself once its accepted generation becomes stale. Unknown
    /// and revoked devices both project the same generic unauthorized error, so
    /// a remote peer cannot distinguish them.
    pub fn admit(&self, device_id: DeviceId) -> Result<Admission, DaemonError> {
        let Some(entry) = self.entry(device_id) else {
            return Err(generic_unauthorized());
        };
        let changes = entry.watch.subscribe();
        let snapshot = *changes.borrow();
        if snapshot.status != AuthorizationStatus::Authorized {
            return Err(generic_unauthorized());
        }
        Ok(Admission { snapshot, changes })
    }

    /// Acquires an owned read permit and verifies the expected generation.
    ///
    /// The returned context keeps the read permit until [`AuthorizedCommitContext::run`]
    /// finishes, so a concurrent revoke cannot advance the generation while a
    /// side effect is in flight. Unknown and revoked devices both project the
    /// same generic unauthorized error.
    pub async fn acquire_commit(
        &self,
        device_id: DeviceId,
        expected_generation: AuthGeneration,
    ) -> Result<AuthorizedCommitContext, DaemonError> {
        let Some(entry) = self.entry(device_id) else {
            return Err(generic_unauthorized());
        };
        let guard = entry.state.clone().read_owned().await;
        if guard.status != AuthorizationStatus::Authorized {
            return Err(generic_unauthorized());
        }
        if guard.generation != expected_generation {
            return Err(stale_generation());
        }
        Ok(AuthorizedCommitContext {
            entry,
            guard,
            device_id,
        })
    }

    /// Acquires the owned write permit for a fresh authorization.
    ///
    /// The device entry is created if it does not exist; the caller publishes
    /// the durable generation computed by the store actor.
    pub async fn authorize_guard(
        &self,
        device_id: DeviceId,
    ) -> Result<AuthorizationWriteGuard, DaemonError> {
        let entry = self.entry_or_insert(device_id);
        let guard = entry.state.clone().write_owned().await;
        Ok(AuthorizationWriteGuard { entry, guard })
    }

    /// Acquires the owned write permit for an existing authorization.
    pub async fn revoke_guard(
        &self,
        device_id: DeviceId,
    ) -> Result<AuthorizationWriteGuard, DaemonError> {
        let Some(entry) = self.entry(device_id) else {
            return Err(device_not_found());
        };
        let guard = entry.state.clone().write_owned().await;
        Ok(AuthorizationWriteGuard { entry, guard })
    }

    /// Acquires the revoke writer while exposing one deterministic test-only
    /// notification immediately before the fair write-lock future is polled.
    ///
    /// Entry lookup and state cloning finish before notification. Sending on
    /// an unbounded channel is synchronous and non-blocking, and the next
    /// expression directly awaits `write_owned`, so a notified test can queue
    /// a later reader only after this writer has entered Tokio's fair queue.
    #[doc(hidden)]
    pub async fn revoke_guard_before_wait_for_test(
        &self,
        device_id: DeviceId,
        before_wait: &tokio::sync::mpsc::UnboundedSender<DeviceId>,
    ) -> Result<AuthorizationWriteGuard, DaemonError> {
        let Some(entry) = self.entry(device_id) else {
            return Err(device_not_found());
        };
        let state = entry.state.clone();
        let _ = before_wait.send(device_id);
        let guard = state.write_owned().await;
        Ok(AuthorizationWriteGuard { entry, guard })
    }

    fn entry(&self, device_id: DeviceId) -> Option<Arc<AuthEntry>> {
        entry_lock(&self.entries).get(&device_id).cloned()
    }

    fn entry_or_insert(&self, device_id: DeviceId) -> Arc<AuthEntry> {
        let mut entries = entry_lock(&self.entries);
        entries
            .entry(device_id)
            .or_insert_with(|| Arc::new(AuthEntry::new(AuthorizationSnapshot::none())))
            .clone()
    }
}

/// Successful admission plus the cancellation subscription.
#[derive(Clone, Debug)]
pub struct Admission {
    /// Snapshot accepted at admission time.
    pub snapshot: AuthorizationSnapshot,
    /// Receiver which fires when the generation is superseded.
    pub changes: watch::Receiver<AuthorizationSnapshot>,
}

/// Owned read permit bound to one expected authorization generation.
#[derive(Debug)]
pub struct AuthorizedCommitContext {
    entry: Arc<AuthEntry>,
    guard: OwnedRwLockReadGuard<AuthorizationSnapshot>,
    device_id: DeviceId,
}

impl AuthorizedCommitContext {
    /// Device whose authorization is being committed.
    #[must_use]
    pub const fn device_id(&self) -> DeviceId {
        self.device_id
    }

    /// Runs a side effect while holding the read permit on a blocking worker.
    ///
    /// The permit is moved into the blocking closure, so a concurrent revoke
    /// writer waits until the side effect fully returns before it can publish.
    pub async fn run<R>(
        self,
        operation: impl FnOnce() -> Result<R, DaemonError> + Send + 'static,
    ) -> Result<R, DaemonError>
    where
        R: Send + 'static,
    {
        let _entry = self.entry;
        let guard = self.guard;
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            operation()
        })
        .await
        .map_err(|error| {
            DaemonError::new(
                DomainErrorKind::Cancelled,
                format!("authorized commit worker ended unexpectedly: {error}"),
            )
        })?
    }
}

/// Owned write permit held by the revoke coordinator across its ordered steps.
#[derive(Debug)]
pub struct AuthorizationWriteGuard {
    entry: Arc<AuthEntry>,
    guard: OwnedRwLockWriteGuard<AuthorizationSnapshot>,
}

impl AuthorizationWriteGuard {
    /// Current snapshot under the exclusive permit.
    #[must_use]
    pub fn snapshot(&self) -> AuthorizationSnapshot {
        *self.guard
    }

    /// Publishes a new in-memory snapshot and wakes cancellation watchers.
    ///
    /// The locked state is updated before the watch publication, so a reader
    /// which acquires the lock after the permit is released observes the new
    /// snapshot together with its cancellation notification.
    pub fn publish(&mut self, snapshot: AuthorizationSnapshot) -> Result<(), DaemonError> {
        validate_snapshot(snapshot)?;
        *self.guard = snapshot;
        self.entry.watch.send_replace(snapshot);
        Ok(())
    }
}

fn entry_lock(
    map: &Mutex<BTreeMap<DeviceId, Arc<AuthEntry>>>,
) -> MutexGuard<'_, BTreeMap<DeviceId, Arc<AuthEntry>>> {
    map.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn generic_unauthorized() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::Unauthorized,
        "device is not authorized to control this host",
    )
}

fn stale_generation() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::AuthorizationRevoked,
        "authorization generation changed since connection admission",
    )
}

fn device_not_found() -> DaemonError {
    DaemonError::new(
        DomainErrorKind::DeviceNotFound,
        "no inbound authorization record for device",
    )
}

fn validate_snapshot(snapshot: AuthorizationSnapshot) -> Result<(), DaemonError> {
    let valid = match snapshot.status {
        AuthorizationStatus::None => snapshot.generation == AuthGeneration::ZERO,
        AuthorizationStatus::Authorized | AuthorizationStatus::Revoked => {
            snapshot.generation != AuthGeneration::ZERO
        }
    };
    if valid {
        Ok(())
    } else {
        Err(DaemonError::new(
            DomainErrorKind::StoreUnavailable,
            "authorization snapshot status and generation are inconsistent",
        ))
    }
}
